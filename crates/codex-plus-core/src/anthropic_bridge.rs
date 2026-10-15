//! Codex Responses ↔ Anthropic Messages 协议桥。
//!
//! 形态选择：以 Chat Completions 作中间态复用既有实现 ——
//! 请求方向 `Responses → Chat → Anthropic Messages`，
//! 响应方向 `Anthropic Messages → Chat → Responses`（非流式 JSON 与 SSE 同构）。
//!
//! 这样工具调用配对、命名空间工具、图片、思考内容、用量统计等映射全部沿用
//! `protocol_proxy` 里已有测试覆盖的那一套，本模块只负责 Chat ↔ Anthropic 这一段，
//! 出问题时可以单独对拍，不必重新验证整条链路。
//!
//! 已知边界：
//! - 不做提示缓存标记注入（`cache_control`），因此长对话不会自动命中 5 分钟缓存；
//! - 不发送 `thinking` 配置，历史里的思考内容不回放（Anthropic 要求带签名，
//!   缺签名整轮 400，宁可不发）。

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::protocol_proxy::ChatSseToResponsesConverter;

/// Anthropic Messages API 版本头，与官方 SDK 一致。
pub const ANTHROPIC_VERSION: &str = "2023-06-01";
/// Anthropic 的 `max_tokens` 是必填项；Codex 请求没带上限时用它兜底。
pub const ANTHROPIC_DEFAULT_MAX_TOKENS: u64 = 8192;
/// 认证字段：`x-api-key`（Anthropic 原生）或 `authorization`（多数中转网关）。
pub const ANTHROPIC_AUTH_HEADER_API_KEY: &str = "x-api-key";
pub const ANTHROPIC_AUTH_HEADER_BEARER: &str = "authorization";

/// 上游 Messages 端点。Base URL 允许写成服务根地址、`/v1`、或完整端点：
/// 末尾带 `#` 表示「原样使用，不要补版本前缀」（与 `chat_completions_url` 同一约定）。
pub fn anthropic_messages_url(base_url: &str) -> String {
    let skip_version_prefix = base_url.trim().ends_with('#');
    let base = base_url.trim().trim_end_matches('#').trim_end_matches('/');
    if base.to_ascii_lowercase().ends_with("/messages") {
        return base.to_string();
    }
    if skip_version_prefix || base.to_ascii_lowercase().ends_with("/v1") {
        return format!("{base}/messages");
    }
    format!("{base}/v1/messages")
}

/// Responses 请求体 → Anthropic Messages 请求体。
pub fn responses_to_anthropic_messages(
    body: Value,
    max_tokens_default: u64,
) -> anyhow::Result<Value> {
    let chat = crate::protocol_proxy::responses_to_chat_completions(body)?;
    chat_completions_to_anthropic_messages(chat, max_tokens_default)
}

/// Chat Completions 请求体 → Anthropic Messages 请求体。
pub fn chat_completions_to_anthropic_messages(
    chat: Value,
    max_tokens_default: u64,
) -> anyhow::Result<Value> {
    let mut out = json!({});

    if let Some(model) = chat.get("model") {
        out["model"] = model.clone();
    }

    // Anthropic 必填项：调用方没有输出上限时用兜底值，避免整轮 400。
    let max_tokens = chat
        .get("max_tokens")
        .or_else(|| chat.get("max_completion_tokens"))
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .unwrap_or(max_tokens_default.max(1));
    out["max_tokens"] = json!(max_tokens);

    if chat.get("stream").and_then(Value::as_bool).unwrap_or(false) {
        out["stream"] = json!(true);
    }
    if let Some(temperature) = chat.get("temperature").and_then(Value::as_f64) {
        // Anthropic 的 temperature 上限是 1.0（OpenAI 允许到 2.0）。
        out["temperature"] = json!(temperature.clamp(0.0, 1.0));
    }
    if let Some(top_p) = chat.get("top_p") {
        out["top_p"] = top_p.clone();
    }
    if let Some(stop) = chat.get("stop") {
        let sequences: Vec<Value> = match stop {
            Value::String(text) => vec![json!(text)],
            Value::Array(items) => items.clone(),
            _ => Vec::new(),
        };
        if !sequences.is_empty() {
            out["stop_sequences"] = json!(sequences);
        }
    }

    let mut system_parts: Vec<String> = Vec::new();
    let mut messages: Vec<Value> = Vec::new();
    let chat_messages = chat
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for message in &chat_messages {
        match message.get("role").and_then(Value::as_str).unwrap_or("") {
            // chat 侧已把 system 收敛到队首，这里再合并一次以防手写请求分散在多处。
            "system" | "developer" => {
                let text = content_to_text(message.get("content"));
                if !text.is_empty() {
                    system_parts.push(text);
                }
            }
            "assistant" => {
                if let Some(converted) = chat_assistant_to_anthropic(message) {
                    push_anthropic_message(&mut messages, converted);
                }
            }
            "tool" => {
                if let Some(converted) = chat_tool_message_to_anthropic(message) {
                    push_anthropic_message(&mut messages, converted);
                }
            }
            _ => {
                let blocks = chat_content_to_anthropic_blocks(message.get("content"));
                if !blocks.is_empty() {
                    push_anthropic_message(
                        &mut messages,
                        json!({ "role": "user", "content": blocks }),
                    );
                }
            }
        }
    }

    if !system_parts.is_empty() {
        out["system"] = json!(system_parts.join("\n\n"));
    }

    // Anthropic 要求 messages 非空且首条为 user。
    if messages.is_empty() {
        messages.push(json!({
            "role": "user",
            "content": [{ "type": "text", "text": "(continuing the conversation)" }]
        }));
    } else if messages[0].get("role").and_then(Value::as_str) == Some("assistant") {
        messages.insert(
            0,
            json!({
                "role": "user",
                "content": [{ "type": "text", "text": "(continuing the conversation)" }]
            }),
        );
    }
    out["messages"] = json!(messages);

    let mut has_tools = false;
    if let Some(tools) = chat.get("tools").and_then(Value::as_array) {
        let converted: Vec<Value> = tools.iter().filter_map(chat_tool_to_anthropic).collect();
        if !converted.is_empty() {
            has_tools = true;
            out["tools"] = json!(converted);
        }
    }
    // Anthropic 在 tools 为空时拒绝 tool_choice。
    if has_tools {
        if let Some(choice) = chat
            .get("tool_choice")
            .and_then(chat_tool_choice_to_anthropic)
        {
            out["tool_choice"] = choice;
        }
        if chat.get("parallel_tool_calls").and_then(Value::as_bool) == Some(false) {
            if let Some(choice) = out.get_mut("tool_choice").and_then(Value::as_object_mut) {
                choice.insert("disable_parallel_tool_use".to_string(), json!(true));
            }
        }
    }

    Ok(out)
}

/// Anthropic Messages 响应体 → Chat Completions 响应体。
///
/// 转换结果继续交给 `protocol_proxy::chat_completion_to_response_with_request`，
/// 工具调用 / 思考内容 / 用量的 Responses 形态映射全部复用那条链路。
pub fn anthropic_message_to_chat_completion(message: Value) -> anyhow::Result<Value> {
    let id = message
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("msg_anthropic")
        .to_string();
    let model = message
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let created = message
        .get("created")
        .and_then(Value::as_u64)
        .unwrap_or_else(now_unix_seconds);

    let mut text = String::new();
    let mut reasoning = String::new();
    let mut tool_calls: Vec<Value> = Vec::new();
    for block in message
        .get("content")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        match block.get("type").and_then(Value::as_str).unwrap_or("") {
            "text" => {
                if let Some(part) = block.get("text").and_then(Value::as_str) {
                    text.push_str(part);
                }
            }
            "thinking" | "redacted_thinking" => {
                if let Some(part) = block
                    .get("thinking")
                    .or_else(|| block.get("data"))
                    .and_then(Value::as_str)
                {
                    reasoning.push_str(part);
                }
            }
            "tool_use" => {
                let name = block
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if name.is_empty() {
                    continue;
                }
                let call_id = block
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| format!("toolu_{}", tool_calls.len()));
                let arguments = block
                    .get("input")
                    .map(|input| serde_json::to_string(input).unwrap_or_else(|_| "{}".to_string()))
                    .unwrap_or_else(|| "{}".to_string());
                tool_calls.push(json!({
                    "id": call_id,
                    "type": "function",
                    "function": { "name": name, "arguments": arguments }
                }));
            }
            _ => {}
        }
    }

    let mut message_object = json!({ "role": "assistant" });
    if text.is_empty() {
        message_object["content"] = Value::Null;
    } else {
        message_object["content"] = json!(text);
    }
    if !reasoning.is_empty() {
        message_object["reasoning_content"] = json!(reasoning);
    }
    if !tool_calls.is_empty() {
        message_object["tool_calls"] = json!(tool_calls);
    }

    let stop_reason = message.get("stop_reason").and_then(Value::as_str);
    Ok(json!({
        "id": id,
        "object": "chat.completion",
        "created": created,
        "model": model,
        "choices": [{
            "index": 0,
            "message": message_object,
            "finish_reason": anthropic_stop_reason_to_finish_reason(stop_reason),
        }],
        "usage": anthropic_usage_to_chat_usage(message.get("usage")),
    }))
}

fn now_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn anthropic_stop_reason_to_finish_reason(stop_reason: Option<&str>) -> &'static str {
    match stop_reason.unwrap_or("") {
        "max_tokens" => "length",
        "tool_use" => "tool_calls",
        "refusal" => "content_filter",
        _ => "stop",
    }
}

/// Anthropic usage → chat usage。
///
/// `chat_usage_to_responses_usage` 本身就认识 Anthropic 的
/// `input_tokens` / `output_tokens` / `cache_read_input_tokens`，这里只把
/// `cache_creation` 明细摊平并补上 `total_tokens`，让用量看板能正确计价。
fn anthropic_usage_to_chat_usage(usage: Option<&Value>) -> Value {
    let Some(usage) = usage.filter(|value| value.is_object()) else {
        return Value::Null;
    };
    let mut out = usage.clone();
    if let Some(creation) = usage.get("cache_creation") {
        if let Some(value) = creation.get("ephemeral_5m_input_tokens") {
            out["cache_creation_5m_input_tokens"] = value.clone();
        }
        if let Some(value) = creation.get("ephemeral_1h_input_tokens") {
            out["cache_creation_1h_input_tokens"] = value.clone();
        }
    }
    if out.get("total_tokens").is_none() {
        let number = |key: &str| usage.get(key).and_then(Value::as_u64).unwrap_or(0);
        // Anthropic 的 `cache_creation_input_tokens` 是缓存写入总量，
        // `cache_creation` 里的 5m/1h 只是它的拆分；缺总量时用拆分求和兜底，
        // 两者都存在时不能相加，否则缓存 token 会被算两遍。
        let cache_creation = usage
            .get("cache_creation_input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or_else(|| {
                let ephemeral = |key: &str| {
                    usage
                        .pointer(&format!("/cache_creation/{key}"))
                        .and_then(Value::as_u64)
                        .unwrap_or(0)
                };
                ephemeral("ephemeral_5m_input_tokens") + ephemeral("ephemeral_1h_input_tokens")
            });
        out["total_tokens"] = json!(
            number("input_tokens")
                + number("output_tokens")
                + number("cache_read_input_tokens")
                + cache_creation
        );
    }
    out
}

fn content_to_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| match part {
                Value::String(text) => Some(text.clone()),
                Value::Object(_) => part.get("text").and_then(Value::as_str).map(str::to_string),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn chat_content_to_anthropic_blocks(content: Option<&Value>) -> Vec<Value> {
    match content {
        Some(Value::String(text)) => {
            if text.is_empty() {
                Vec::new()
            } else {
                vec![json!({ "type": "text", "text": text })]
            }
        }
        Some(Value::Array(parts)) => parts.iter().filter_map(chat_part_to_anthropic_block).collect(),
        _ => Vec::new(),
    }
}

fn chat_part_to_anthropic_block(part: &Value) -> Option<Value> {
    let part_type = part.get("type").and_then(Value::as_str).unwrap_or("");
    match part_type {
        "text" | "input_text" | "output_text" => part
            .get("text")
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .map(|text| json!({ "type": "text", "text": text })),
        "image_url" | "input_image" => {
            let raw = part.get("image_url")?;
            let url = raw
                .as_str()
                .or_else(|| raw.get("url").and_then(Value::as_str))?;
            anthropic_image_block(url)
        }
        // 上游自定义部件（如 VLM 分析结果）统一降级成文本，不能整轮丢弃。
        _ => part
            .get("text")
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .map(|text| json!({ "type": "text", "text": text })),
    }
}

/// data URL → base64 源；http(s) URL → url 源。其余形态无法表达，直接丢弃。
fn anthropic_image_block(url: &str) -> Option<Value> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    if let Some(rest) = url.strip_prefix("data:") {
        let (meta, data) = rest.split_once(',')?;
        let lowered = meta.to_ascii_lowercase();
        if !lowered.contains("base64") {
            return None;
        }
        let media_type = meta.split(';').next().unwrap_or("").trim();
        let media_type = if media_type.is_empty() {
            "image/png"
        } else {
            media_type
        };
        return Some(json!({
            "type": "image",
            "source": { "type": "base64", "media_type": media_type, "data": data }
        }));
    }
    if url.starts_with("http://") || url.starts_with("https://") {
        return Some(json!({
            "type": "image",
            "source": { "type": "url", "url": url }
        }));
    }
    None
}

fn chat_assistant_to_anthropic(message: &Value) -> Option<Value> {
    let mut blocks = chat_content_to_anthropic_blocks(message.get("content"));
    if let Some(tool_calls) = message.get("tool_calls").and_then(Value::as_array) {
        for (index, tool_call) in tool_calls.iter().enumerate() {
            let function = tool_call.get("function").unwrap_or(tool_call);
            let name = function
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim();
            if name.is_empty() {
                continue;
            }
            let call_id = tool_call
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_string)
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| format!("toolu_{index}"));
            let input = tool_call_arguments_to_value(function.get("arguments"));
            blocks.push(json!({
                "type": "tool_use",
                "id": call_id,
                "name": name,
                "input": input
            }));
        }
    }
    if blocks.is_empty() {
        return None;
    }
    Some(json!({ "role": "assistant", "content": blocks }))
}

fn tool_call_arguments_to_value(arguments: Option<&Value>) -> Value {
    match arguments {
        Some(Value::String(raw)) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                json!({})
            } else {
                serde_json::from_str::<Value>(trimmed).unwrap_or_else(|_| json!({}))
            }
        }
        Some(value) if value.is_object() => value.clone(),
        _ => json!({}),
    }
}

fn chat_tool_message_to_anthropic(message: &Value) -> Option<Value> {
    let tool_use_id = message
        .get("tool_call_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if tool_use_id.is_empty() {
        return None;
    }
    let content = match message.get("content") {
        Some(Value::String(text)) => Value::String(text.clone()),
        Some(Value::Array(parts)) => {
            let blocks: Vec<Value> = parts.iter().filter_map(chat_part_to_anthropic_block).collect();
            if blocks.is_empty() {
                Value::String(String::new())
            } else {
                Value::Array(blocks)
            }
        }
        _ => Value::String(String::new()),
    };
    Some(json!({
        "role": "user",
        "content": [{
            "type": "tool_result",
            "tool_use_id": tool_use_id,
            "content": content
        }]
    }))
}

/// Anthropic 要求同角色消息连续出现时合并（tool_result 也必须和相邻 user 消息同组）。
fn push_anthropic_message(messages: &mut Vec<Value>, message: Value) {
    let role = message
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or("user")
        .to_string();
    if let Some(last) = messages.last_mut() {
        if last.get("role").and_then(Value::as_str) == Some(role.as_str()) {
            let incoming = message
                .get("content")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if let Some(content) = last.get_mut("content").and_then(Value::as_array_mut) {
                content.extend(incoming);
                return;
            }
        }
    }
    messages.push(message);
}

fn chat_tool_to_anthropic(tool: &Value) -> Option<Value> {
    let function = tool.get("function").unwrap_or(tool);
    let name = function.get("name").and_then(Value::as_str)?.trim();
    if name.is_empty() {
        return None;
    }
    let mut out = json!({ "name": name });
    if let Some(description) = function.get("description").and_then(Value::as_str) {
        if !description.trim().is_empty() {
            out["description"] = json!(description);
        }
    }
    let mut schema = function
        .get("parameters")
        .or_else(|| function.get("input_schema"))
        .cloned()
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({ "type": "object", "properties": {} }));
    if schema.get("type").is_none() {
        schema["type"] = json!("object");
    }
    out["input_schema"] = schema;
    Some(out)
}

fn chat_tool_choice_to_anthropic(choice: &Value) -> Option<Value> {
    match choice {
        Value::String(value) => match value.trim() {
            "auto" => Some(json!({ "type": "auto" })),
            "none" => Some(json!({ "type": "none" })),
            "required" | "any" => Some(json!({ "type": "any" })),
            _ => None,
        },
        Value::Object(_) => {
            let name = choice
                .pointer("/function/name")
                .or_else(|| choice.get("name"))
                .and_then(Value::as_str)?
                .trim();
            (!name.is_empty()).then(|| json!({ "type": "tool", "name": name }))
        }
        _ => None,
    }
}

// ============================================================================
// SSE：Anthropic 事件流 → Chat Completions 事件流 → Responses 事件流
// ============================================================================

/// 有状态地把 Anthropic SSE 事件流翻译成 Chat Completions SSE 分片。
///
/// 只做「事件 → chat chunk」的机械映射，终止语义（finish_reason / [DONE]）保持
/// 与 chat 上游一致，后续交给 `ChatSseToResponsesConverter` 产出 Responses 事件。
pub struct AnthropicSseToChatSse {
    buffer: String,
    utf8_remainder: Vec<u8>,
    message_id: String,
    model: String,
    created: u64,
    /// Anthropic 的 content block 下标 → chat 侧的 tool_calls 下标。
    tool_index_by_block: BTreeMap<u64, u64>,
    next_tool_index: u64,
    stopped: bool,
    failed: bool,
}

impl Default for AnthropicSseToChatSse {
    fn default() -> Self {
        Self {
            buffer: String::new(),
            utf8_remainder: Vec::new(),
            message_id: "msg_anthropic".to_string(),
            model: String::new(),
            created: now_unix_seconds(),
            tool_index_by_block: BTreeMap::new(),
            next_tool_index: 0,
            stopped: false,
            failed: false,
        }
    }
}

impl AnthropicSseToChatSse {
    pub fn has_failed(&self) -> bool {
        self.failed
    }

    /// 只有收到 `message_stop`（或错误事件）才算正常终止，用于识别被掐断的流。
    pub fn has_terminal_event(&self) -> bool {
        self.stopped || self.failed
    }

    pub fn push_bytes(&mut self, bytes: &[u8]) -> Vec<u8> {
        append_utf8_safe(&mut self.buffer, &mut self.utf8_remainder, bytes);
        let mut output = String::new();
        while let Some(block) = take_sse_block(&mut self.buffer) {
            if block.trim().is_empty() {
                continue;
            }
            self.handle_block(&block, &mut output);
            if self.failed {
                break;
            }
        }
        output.into_bytes()
    }

    /// 上游流结束时调用。未见到 `message_stop` 时不补 `[DONE]`，
    /// 让上层按「流被掐断」处理，避免把不完整回答伪装成成功。
    pub fn finish(&mut self) -> Vec<u8> {
        if !self.utf8_remainder.is_empty() {
            self.buffer
                .push_str(&String::from_utf8_lossy(&self.utf8_remainder));
            self.utf8_remainder.clear();
        }
        Vec::new()
    }

    fn handle_block(&mut self, block: &str, output: &mut String) {
        let mut event_name: Option<String> = None;
        let mut data_parts: Vec<String> = Vec::new();
        for line in block.lines() {
            if let Some(event) = strip_sse_field(line, "event") {
                event_name = Some(event.trim().to_string());
            }
            if let Some(data) = strip_sse_field(line, "data") {
                data_parts.push(data.to_string());
            }
        }
        if data_parts.is_empty() {
            return;
        }
        let data_text = data_parts.join("\n");
        if data_text.trim() == "[DONE]" {
            self.emit_done(output);
            return;
        }
        let Ok(data) = serde_json::from_str::<Value>(&data_text) else {
            return;
        };
        // 部分网关不写 `event:` 行，只把类型放在 JSON 的 `type` 字段里。
        let event = event_name
            .or_else(|| data.get("type").and_then(Value::as_str).map(str::to_string))
            .unwrap_or_default();
        self.handle_event(event.trim(), &data, output);
    }

    fn handle_event(&mut self, event: &str, data: &Value, output: &mut String) {
        match event {
            "message_start" => {
                if let Some(message) = data.get("message") {
                    if let Some(id) = message.get("id").and_then(Value::as_str) {
                        if !id.trim().is_empty() {
                            self.message_id = id.to_string();
                        }
                    }
                    if let Some(model) = message.get("model").and_then(Value::as_str) {
                        if !model.trim().is_empty() {
                            self.model = model.to_string();
                        }
                    }
                    let usage = message.get("usage").map(|usage| anthropic_usage_to_chat_usage(Some(usage)));
                    let chunk = self.base_chunk(json!({ "role": "assistant" }), None, usage);
                    emit_chat_chunk(output, &chunk);
                }
            }
            "content_block_start" => {
                let block = data.get("content_block").cloned().unwrap_or(Value::Null);
                if block.get("type").and_then(Value::as_str) != Some("tool_use") {
                    return;
                }
                let block_index = data.get("index").and_then(Value::as_u64).unwrap_or(0);
                let tool_index = self.next_tool_index;
                self.next_tool_index += 1;
                self.tool_index_by_block.insert(block_index, tool_index);
                let call_id = block
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let name = block
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let chunk = self.base_chunk(
                    json!({ "tool_calls": [{
                        "index": tool_index,
                        "id": call_id,
                        "type": "function",
                        "function": { "name": name, "arguments": "" }
                    }] }),
                    None,
                    None,
                );
                emit_chat_chunk(output, &chunk);
            }
            "content_block_delta" => {
                let block_index = data.get("index").and_then(Value::as_u64).unwrap_or(0);
                let delta = data.get("delta").cloned().unwrap_or(Value::Null);
                match delta.get("type").and_then(Value::as_str).unwrap_or("") {
                    "text_delta" => {
                        if let Some(text) = delta.get("text").and_then(Value::as_str) {
                            if !text.is_empty() {
                                let chunk = self.base_chunk(json!({ "content": text }), None, None);
                                emit_chat_chunk(output, &chunk);
                            }
                        }
                    }
                    "thinking_delta" => {
                        if let Some(text) = delta.get("thinking").and_then(Value::as_str) {
                            if !text.is_empty() {
                                let chunk = self
                                    .base_chunk(json!({ "reasoning_content": text }), None, None);
                                emit_chat_chunk(output, &chunk);
                            }
                        }
                    }
                    "input_json_delta" => {
                        let partial = delta
                            .get("partial_json")
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        if partial.is_empty() {
                            return;
                        }
                        let tool_index = self
                            .tool_index_by_block
                            .get(&block_index)
                            .copied()
                            .unwrap_or(block_index);
                        let chunk = self.base_chunk(
                            json!({ "tool_calls": [{
                                "index": tool_index,
                                "function": { "arguments": partial }
                            }] }),
                            None,
                            None,
                        );
                        emit_chat_chunk(output, &chunk);
                    }
                    _ => {}
                }
            }
            "message_delta" => {
                let stop_reason = data.pointer("/delta/stop_reason").and_then(Value::as_str);
                let finish_reason = anthropic_stop_reason_to_finish_reason(stop_reason);
                let usage = data
                    .get("usage")
                    .map(|usage| anthropic_usage_to_chat_usage(Some(usage)));
                let chunk = self.base_chunk(json!({}), Some(finish_reason), usage);
                emit_chat_chunk(output, &chunk);
            }
            "message_stop" => self.emit_done(output),
            "error" => {
                let error = data
                    .get("error")
                    .cloned()
                    .unwrap_or_else(|| json!({ "message": "上游返回错误事件" }));
                emit_chat_chunk(output, &json!({ "error": error }));
                self.failed = true;
            }
            // ping / content_block_stop / signature_delta 等无对应 chat 语义。
            _ => {}
        }
    }

    fn emit_done(&mut self, output: &mut String) {
        if self.stopped {
            return;
        }
        self.stopped = true;
        output.push_str("data: [DONE]\n\n");
    }

    fn base_chunk(
        &self,
        delta: Value,
        finish_reason: Option<&str>,
        usage: Option<Value>,
    ) -> Value {
        let mut chunk = json!({
            "id": self.message_id,
            "object": "chat.completion.chunk",
            "created": self.created,
            "model": self.model,
            "choices": [{
                "index": 0,
                "delta": delta,
                "finish_reason": finish_reason,
            }],
        });
        if let Some(usage) = usage.filter(|value| !value.is_null()) {
            chunk["usage"] = usage;
        }
        chunk
    }
}

/// Anthropic SSE → Responses SSE 的有状态转换器。
pub struct AnthropicSseToResponsesConverter {
    bridge: AnthropicSseToChatSse,
    chat: ChatSseToResponsesConverter,
}

impl AnthropicSseToResponsesConverter {
    pub fn with_request(request: &Value) -> Self {
        Self {
            bridge: AnthropicSseToChatSse::default(),
            chat: ChatSseToResponsesConverter::with_request(request),
        }
    }

    pub fn has_failed(&self) -> bool {
        self.chat.has_failed()
    }

    pub fn has_terminal_event(&self) -> bool {
        self.chat.has_terminal_event()
    }

    pub fn push_bytes(&mut self, bytes: &[u8]) -> Vec<u8> {
        let chat_bytes = self.bridge.push_bytes(bytes);
        if chat_bytes.is_empty() {
            return Vec::new();
        }
        self.chat.push_bytes(&chat_bytes)
    }

    pub fn finish(&mut self) -> Vec<u8> {
        let tail = self.bridge.finish();
        let mut output = if tail.is_empty() {
            Vec::new()
        } else {
            self.chat.push_bytes(&tail)
        };
        output.extend(self.chat.finish());
        output
    }

    pub fn fail(&mut self, message: String, error_type: Option<String>) -> Vec<u8> {
        self.chat.fail(message, error_type)
    }
}

impl Default for AnthropicSseToResponsesConverter {
    fn default() -> Self {
        Self {
            bridge: AnthropicSseToChatSse::default(),
            chat: ChatSseToResponsesConverter::default(),
        }
    }
}

/// 一次性把整段 Anthropic SSE 转成 Responses SSE（非 launcher 路径使用）。
pub fn anthropic_sse_to_responses_sse(input: &str, request: &Value) -> Vec<u8> {
    let mut converter = AnthropicSseToResponsesConverter::with_request(request);
    let mut output = converter.push_bytes(input.as_bytes());
    output.extend(converter.finish());
    output
}

/// 测试 / 诊断用：一次性转成 chat SSE。
pub fn anthropic_sse_to_chat_sse(input: &str) -> String {
    let mut converter = AnthropicSseToChatSse::default();
    let mut output = converter.push_bytes(input.as_bytes());
    output.extend(converter.finish());
    String::from_utf8_lossy(&output).to_string()
}

fn emit_chat_chunk(output: &mut String, chunk: &Value) {
    output.push_str("data: ");
    output.push_str(&chunk.to_string());
    output.push_str("\n\n");
}

fn take_sse_block(buffer: &mut String) -> Option<String> {
    let lf = buffer.find("\n\n").map(|index| (index, 2));
    let crlf = buffer.find("\r\n\r\n").map(|index| (index, 4));
    let (index, delimiter_len) = match (lf, crlf) {
        (Some(left), Some(right)) => {
            if left.0 <= right.0 {
                left
            } else {
                right
            }
        }
        (Some(value), None) | (None, Some(value)) => value,
        (None, None) => return None,
    };
    let block = buffer[..index].to_string();
    buffer.drain(..index + delimiter_len);
    Some(block)
}

fn append_utf8_safe(buffer: &mut String, remainder: &mut Vec<u8>, bytes: &[u8]) {
    if bytes.is_empty() {
        return;
    }
    let mut combined = Vec::new();
    if !remainder.is_empty() {
        combined.extend_from_slice(remainder);
        remainder.clear();
    }
    combined.extend_from_slice(bytes);
    match std::str::from_utf8(&combined) {
        Ok(text) => buffer.push_str(text),
        Err(error) => {
            let valid = error.valid_up_to();
            if valid > 0 {
                buffer.push_str(std::str::from_utf8(&combined[..valid]).unwrap_or_default());
            }
            if error.error_len().is_none() {
                remainder.extend_from_slice(&combined[valid..]);
            } else {
                buffer.push_str(&String::from_utf8_lossy(&combined[valid..]));
            }
        }
    }
}

fn strip_sse_field<'a>(line: &'a str, field: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(field)?.strip_prefix(':')?;
    Some(rest.strip_prefix(' ').unwrap_or(rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chat_request() -> Value {
        json!({
            "model": "claude-sonnet-5",
            "max_tokens": 512,
            "temperature": 1.7,
            "stream": true,
            "messages": [
                { "role": "system", "content": "你是助手" },
                { "role": "user", "content": [
                    { "type": "text", "text": "看图" },
                    { "type": "image_url", "image_url": { "url": "data:image/png;base64,AAAA" } }
                ]},
                { "role": "assistant", "content": null, "tool_calls": [
                    { "id": "call_1", "type": "function",
                      "function": { "name": "lookup", "arguments": "{\"q\":\"x\"}" } }
                ]},
                { "role": "tool", "tool_call_id": "call_1", "content": "结果" }
            ],
            "tools": [
                { "type": "function", "function": {
                    "name": "lookup", "description": "查一下",
                    "parameters": { "type": "object", "properties": { "q": { "type": "string" } } }
                }}
            ],
            "tool_choice": "auto"
        })
    }

    #[test]
    fn messages_url_normalizes_common_shapes() {
        assert_eq!(
            anthropic_messages_url("https://api.example.com"),
            "https://api.example.com/v1/messages"
        );
        assert_eq!(
            anthropic_messages_url("https://api.example.com/"),
            "https://api.example.com/v1/messages"
        );
        assert_eq!(
            anthropic_messages_url("https://api.example.com/v1"),
            "https://api.example.com/v1/messages"
        );
        assert_eq!(
            anthropic_messages_url("https://api.example.com/v1/messages"),
            "https://api.example.com/v1/messages"
        );
        assert_eq!(
            anthropic_messages_url("https://gw.example.com/openai#"),
            "https://gw.example.com/openai/messages"
        );
    }

    #[test]
    fn chat_request_maps_to_anthropic_shape() {
        let converted = chat_completions_to_anthropic_messages(chat_request(), 8192).unwrap();
        assert_eq!(converted["model"], "claude-sonnet-5");
        assert_eq!(converted["max_tokens"], 512);
        assert_eq!(converted["stream"], true);
        // OpenAI 的 temperature 上限是 2.0，Anthropic 只到 1.0。
        assert_eq!(converted["temperature"], 1.0);
        assert_eq!(converted["system"], "你是助手");
        assert_eq!(converted["messages"][0]["role"], "user");
        let blocks = converted["messages"][0]["content"].as_array().unwrap();
        assert_eq!(blocks[0], json!({ "type": "text", "text": "看图" }));
        assert_eq!(
            blocks[1],
            json!({ "type": "image", "source": {
                "type": "base64", "media_type": "image/png", "data": "AAAA" } })
        );
        // 助手工具调用 → tool_use
        assert_eq!(converted["messages"][1]["role"], "assistant");
        assert_eq!(
            converted["messages"][1]["content"][0],
            json!({ "type": "tool_use", "id": "call_1", "name": "lookup", "input": { "q": "x" } })
        );
        // 工具结果 → 紧随其后的 user + tool_result
        assert_eq!(converted["messages"][2]["role"], "user");
        assert_eq!(
            converted["messages"][2]["content"][0],
            json!({ "type": "tool_result", "tool_use_id": "call_1", "content": "结果" })
        );
        assert_eq!(converted["tools"][0]["name"], "lookup");
        assert_eq!(converted["tools"][0]["description"], "查一下");
        assert_eq!(converted["tools"][0]["input_schema"]["type"], "object");
        assert_eq!(converted["tool_choice"], json!({ "type": "auto" }));
    }

    #[test]
    fn missing_max_tokens_falls_back_and_first_message_is_user() {
        let request = json!({
            "model": "claude-sonnet-5",
            "messages": [
                { "role": "assistant", "content": "上一条回答" },
                { "role": "user", "content": "继续" }
            ]
        });
        let converted = chat_completions_to_anthropic_messages(request, 4096).unwrap();
        assert_eq!(converted["max_tokens"], 4096);
        let messages = converted["messages"].as_array().unwrap();
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"][0]["text"], "(continuing the conversation)");
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[2]["role"], "user");
    }

    #[test]
    fn empty_tools_drop_tool_choice() {
        let request = json!({
            "model": "m",
            "messages": [{ "role": "user", "content": "hi" }],
            "tools": [],
            "tool_choice": "auto"
        });
        let converted = chat_completions_to_anthropic_messages(request, 8192).unwrap();
        assert!(converted.get("tools").is_none());
        assert!(converted.get("tool_choice").is_none());
    }

    #[test]
    fn anthropic_message_maps_back_to_chat_completion() {
        let message = json!({
            "id": "msg_1",
            "model": "claude-sonnet-5",
            "stop_reason": "tool_use",
            "content": [
                { "type": "thinking", "thinking": "先想一下" },
                { "type": "text", "text": "我来查" },
                { "type": "tool_use", "id": "toolu_1", "name": "lookup", "input": { "q": "x" } }
            ],
            "usage": {
                "input_tokens": 10,
                "output_tokens": 4,
                "cache_read_input_tokens": 2,
                "cache_creation": { "ephemeral_5m_input_tokens": 3 }
            }
        });
        let converted = anthropic_message_to_chat_completion(message).unwrap();
        assert_eq!(converted["choices"][0]["finish_reason"], "tool_calls");
        assert_eq!(converted["choices"][0]["message"]["content"], "我来查");
        assert_eq!(
            converted["choices"][0]["message"]["reasoning_content"],
            "先想一下"
        );
        assert_eq!(
            converted["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"],
            "{\"q\":\"x\"}"
        );
        assert_eq!(converted["usage"]["cache_creation_5m_input_tokens"], 3);
        assert_eq!(converted["usage"]["total_tokens"], 19);
    }

    #[test]
    fn anthropic_sse_maps_text_thinking_and_tool_calls() {
        let stream = concat!(
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_9\",\"model\":\"claude-sonnet-5\",\"usage\":{\"input_tokens\":7}}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"想\"}}\n\n",
            "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"text\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"text_delta\",\"text\":\"你好\"}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":2,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_2\",\"name\":\"lookup\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":2,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"q\\\":\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":2,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"\\\"x\\\"}\"}}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":5}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"
        );
        let chat = anthropic_sse_to_chat_sse(stream);
        assert!(chat.contains("\"reasoning_content\":\"想\""), "{chat}");
        assert!(chat.contains("\"content\":\"你好\""), "{chat}");
        assert!(chat.contains("\"name\":\"lookup\""), "{chat}");
        assert!(chat.contains("\"arguments\":\"{\\\"q\\\":\""), "{chat}");
        assert!(chat.contains("\"finish_reason\":\"tool_calls\""), "{chat}");
        assert!(chat.trim_end().ends_with("data: [DONE]"), "{chat}");
    }

    #[test]
    fn anthropic_sse_converts_to_responses_events() {
        let stream = concat!(
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_9\",\"model\":\"claude-sonnet-5\"}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"你好\"}}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":2}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"
        );
        let request = json!({ "model": "claude-sonnet-5" });
        let responses = String::from_utf8(anthropic_sse_to_responses_sse(stream, &request)).unwrap();
        assert!(responses.contains("response.created"), "{responses}");
        assert!(responses.contains("response.output_text.delta"), "{responses}");
        assert!(responses.contains("你好"), "{responses}");
        assert!(responses.contains("response.completed"), "{responses}");
    }

    #[test]
    fn truncated_anthropic_stream_has_no_terminal_event() {
        let mut converter = AnthropicSseToChatSse::default();
        let _ = converter.push_bytes(
            b"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"m\"}}\n\n",
        );
        assert!(!converter.has_terminal_event());
        let _ = converter.finish();
        assert!(
            !converter.has_terminal_event(),
            "缺少 message_stop 不能被当成正常结束"
        );
    }

    #[test]
    fn error_event_surfaces_as_failure() {
        let mut converter = AnthropicSseToResponsesConverter::with_request(&json!({}));
        let _ = converter.push_bytes(
            "event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"忙\"}}\n\n"
                .as_bytes(),
        );
        assert!(converter.has_failed());
        assert!(converter.has_terminal_event());
    }
}
