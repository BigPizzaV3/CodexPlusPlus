//! Responses 响应（JSON / SSE）→ OpenAI Chat Completions 响应。
//!
//! 与 [`super::anthropic_outbound`] 同一套思路：代理已经把上游统一成 Responses SSE，这里只做
//! 最后一跳，翻成 `chat.completion.chunk` 流。
//!
//! Chat 的增量块没有「块开始 / 块结束」的约束：文字、推理、多个工具调用的参数都可以交错输出，
//! 工具调用只靠 `index` 区分，所以不需要像 Anthropic 那样暂存重放。要防的是：
//! - 同一个工具调用只发一次头（id、name），参数只发一次，完成事件只补发还没发出的后缀；
//! - 已经流式发过的文字 / 推理，完成事件里不再整段补发。
//!
//! 文件末尾的 [`ResponsesSseReader`] 等读取工具也供 [`super::gemini_outbound`] 复用。

use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

use crate::request_log::RequestUsage;

#[derive(Debug, Clone, Default)]
struct ToolState {
    /// 发出头之后才分配，保证客户端看到的 index 从 0 起按出现顺序递增。
    index: Option<u32>,
    call_id: String,
    name: String,
    /// 已收到的参数；发出头之后它与已发出的内容一致，之前是缓冲。
    arguments: String,
    done: bool,
}

/// Responses SSE → Chat Completions SSE 的增量转换器。
pub struct ResponsesSseToChatSse {
    reader: ResponsesSseReader,
    model: String,
    id: String,
    created: u64,
    include_usage: bool,
    started: bool,
    finished: bool,
    failed: bool,
    tools: HashMap<String, ToolState>,
    next_tool_index: u32,
    /// 已经流式输出过文字 / 推理的 item，完成时不再整段补发。
    streamed_items: HashSet<String>,
    /// 已经处理过完成事件的 item，防止重复。
    completed_items: HashSet<String>,
    /// 各推理 item 当前所在的摘要段（换段时补两个换行）。
    summary_index: HashMap<String, u64>,
    usage: Option<RequestUsage>,
    failure: Option<String>,
}

impl ResponsesSseToChatSse {
    /// `model` 是回给客户端的模型名；`include_usage` 对应请求里的 `stream_options.include_usage`。
    pub fn new(model: &str, include_usage: bool) -> Self {
        Self {
            reader: ResponsesSseReader::default(),
            model: model.to_string(),
            id: new_completion_id(),
            created: unix_now(),
            include_usage,
            started: false,
            finished: false,
            failed: false,
            tools: HashMap::new(),
            next_tool_index: 0,
            streamed_items: HashSet::new(),
            completed_items: HashSet::new(),
            summary_index: HashMap::new(),
            usage: None,
            failure: None,
        }
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }

    pub fn has_failed(&self) -> bool {
        self.failed
    }

    pub fn failure(&self) -> Option<&str> {
        self.failure.as_deref()
    }

    pub fn usage(&self) -> Option<RequestUsage> {
        self.usage.clone()
    }

    pub fn push_bytes(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut output = String::new();
        for (kind, event) in self.reader.push(bytes) {
            self.handle_event(&kind, &event, &mut output);
        }
        output.into_bytes()
    }

    /// 流结束：没见到终止事件就补一个错误块，客户端才知道这一轮没完成。
    pub fn finish(&mut self) -> Vec<u8> {
        let mut output = String::new();
        for (kind, event) in self.reader.drain() {
            self.handle_event(&kind, &event, &mut output);
        }
        if !self.finished {
            self.fail_into(
                &mut output,
                "server_error",
                None,
                "上游在完成事件前结束了响应流",
            );
        }
        output.into_bytes()
    }

    /// 上游读取出错时调用。
    pub fn fail(&mut self, message: &str) -> Vec<u8> {
        let mut output = String::new();
        if !self.finished {
            self.fail_into(&mut output, "server_error", None, message);
        }
        output.into_bytes()
    }

    fn handle_event(&mut self, kind: &str, event: &Value, output: &mut String) {
        if self.finished {
            return;
        }
        match kind {
            "response.output_text.delta" | "response.refusal.delta" => {
                let delta = event
                    .get("delta")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let key = item_key(event);
                if delta.is_empty() || self.completed_items.contains(&key) {
                    return;
                }
                self.streamed_items.insert(key);
                let field = if kind == "response.refusal.delta" {
                    "refusal"
                } else {
                    "content"
                };
                self.push_delta(output, json!({ field: delta }));
            }
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                let delta = event
                    .get("delta")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let key = item_key(event);
                if delta.is_empty() || self.completed_items.contains(&key) {
                    return;
                }
                let mut text = delta.to_string();
                if let Some(current) = event.get("summary_index").and_then(Value::as_u64)
                    && let Some(previous) = self.summary_index.insert(key.clone(), current)
                    && previous != current
                {
                    text = format!("\n\n{text}");
                }
                self.streamed_items.insert(key);
                self.push_delta(output, json!({ "reasoning_content": text }));
            }
            "response.output_item.added" => {
                let Some(item) = event.get("item") else {
                    return;
                };
                if item.get("type").and_then(Value::as_str) != Some("function_call") {
                    return;
                }
                let key = item_key(event);
                if self.tools.contains_key(&key) {
                    // 重复的 added：同一个工具调用只认第一次。
                    return;
                }
                let tool = ToolState {
                    call_id: tool_call_id(item),
                    name: str_field(item, "name"),
                    ..ToolState::default()
                };
                self.tools.insert(key.clone(), tool);
                self.send_tool_header(&key, output);
            }
            "response.function_call_arguments.delta" => {
                let delta = event
                    .get("delta")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if delta.is_empty() {
                    return;
                }
                let key = item_key(event);
                let tool = self.tools.entry(key).or_default();
                if tool.done {
                    // 已经收尾的工具调用，后到的增量丢弃，免得参数重复。
                    return;
                }
                tool.arguments.push_str(delta);
                if let Some(index) = tool.index {
                    let chunk = json!({ "tool_calls": [{
                        "index": index,
                        "function": { "arguments": delta }
                    }]});
                    self.push_delta(output, chunk);
                }
            }
            "response.output_item.done" => {
                if let Some(item) = event.get("item") {
                    let key = item_key(event);
                    self.item_done(&key, item, output);
                }
            }
            "response.completed" | "response.incomplete" => {
                let response = event.get("response").cloned().unwrap_or(Value::Null);
                // 有的上游只在终止事件里给完整 output：补发还没处理过的项。
                if let Some(items) = response.get("output").and_then(Value::as_array) {
                    for (index, item) in items.iter().enumerate() {
                        let key = output_item_key(item, index);
                        self.item_done(&key, item, output);
                    }
                }
                // 只见过增量、没见过完成事件的工具调用：用已收到的参数收尾。
                let unfinished = self
                    .tools
                    .iter()
                    .filter(|(_, tool)| !tool.done)
                    .map(|(key, _)| key.clone())
                    .collect::<Vec<_>>();
                for key in unfinished {
                    self.item_done(&key, &json!({ "type": "function_call" }), output);
                }
                self.finish_message(&response, output);
            }
            "response.failed" | "error" => {
                let (message, code) = upstream_failure(event);
                let error_type = if code.contains("rate_limit") {
                    "rate_limit_error"
                } else if code.contains("invalid") {
                    "invalid_request_error"
                } else {
                    "server_error"
                };
                let code = (!code.is_empty()).then_some(code);
                self.fail_into(output, error_type, code.as_deref(), &message);
            }
            _ => {}
        }
    }

    fn item_done(&mut self, key: &str, item: &Value, output: &mut String) {
        if self.completed_items.contains(key) {
            return;
        }
        match item.get("type").and_then(Value::as_str) {
            Some("message") => {
                if !self.streamed_items.contains(key) {
                    let text = message_text(item);
                    if !text.is_empty() {
                        self.push_delta(output, json!({ "content": text }));
                    }
                }
            }
            Some("reasoning") => {
                if !self.streamed_items.contains(key) {
                    let text = reasoning_text(item);
                    if !text.is_empty() {
                        self.push_delta(output, json!({ "reasoning_content": text }));
                    }
                }
            }
            Some("function_call") => {
                let tool = self.tools.entry(key.to_string()).or_default();
                if tool.done {
                    return;
                }
                if tool.call_id.is_empty() {
                    tool.call_id = tool_call_id(item);
                }
                if tool.name.is_empty() {
                    tool.name = str_field(item, "name");
                }
                let full = item
                    .get("arguments")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                if tool.index.is_none() {
                    // 头还没发：完整参数优先，随头一起整段发出。
                    if !full.is_empty() {
                        tool.arguments = full;
                    }
                    self.send_tool_header(key, output);
                } else {
                    // 实时转发过的只补发多出来的后缀；对不上（上游改写了参数）时无法撤回，保持已发内容。
                    let remainder = full
                        .strip_prefix(tool.arguments.as_str())
                        .unwrap_or_default()
                        .to_string();
                    if !remainder.is_empty() {
                        tool.arguments.push_str(&remainder);
                        let index = tool.index.unwrap_or_default();
                        let chunk = json!({ "tool_calls": [{
                            "index": index,
                            "function": { "arguments": remainder }
                        }]});
                        self.push_delta(output, chunk);
                    }
                }
                if let Some(tool) = self.tools.get_mut(key) {
                    tool.done = true;
                }
            }
            _ => {}
        }
        self.completed_items.insert(key.to_string());
    }

    /// 发工具调用的头（index、id、name 与已缓冲的参数）；没有名字时先不发，等完成事件补齐。
    fn send_tool_header(&mut self, key: &str, output: &mut String) {
        let Some(tool) = self.tools.get_mut(key) else {
            return;
        };
        if tool.index.is_some() || tool.name.is_empty() {
            return;
        }
        let index = self.next_tool_index;
        self.next_tool_index += 1;
        tool.index = Some(index);
        if tool.call_id.is_empty() {
            tool.call_id = format!("call_{}", uuid::Uuid::new_v4().simple());
        }
        let chunk = json!({ "tool_calls": [{
            "index": index,
            "id": tool.call_id,
            "type": "function",
            "function": { "name": tool.name, "arguments": tool.arguments }
        }]});
        self.push_delta(output, chunk);
    }

    fn ensure_started(&mut self, output: &mut String) {
        if self.started {
            return;
        }
        self.started = true;
        let chunk = self.chunk(json!({ "role": "assistant", "content": "" }), None);
        push_data(output, &chunk);
    }

    fn push_delta(&mut self, output: &mut String, delta: Value) {
        self.ensure_started(output);
        let chunk = self.chunk(delta, None);
        push_data(output, &chunk);
    }

    fn chunk(&self, delta: Value, finish_reason: Option<&str>) -> Value {
        json!({
            "id": self.id,
            "object": "chat.completion.chunk",
            "created": self.created,
            "model": self.model,
            "choices": [{
                "index": 0,
                "delta": delta,
                "logprobs": null,
                "finish_reason": finish_reason
            }]
        })
    }

    fn finish_message(&mut self, response: &Value, output: &mut String) {
        let usage = responses_usage(response.get("usage"));
        self.usage = Some(usage.request_usage());
        let reason = finish_reason(response, self.next_tool_index > 0);
        self.ensure_started(output);
        let chunk = self.chunk(json!({}), Some(reason));
        push_data(output, &chunk);
        if self.include_usage {
            push_data(
                output,
                &json!({
                    "id": self.id,
                    "object": "chat.completion.chunk",
                    "created": self.created,
                    "model": self.model,
                    "choices": [],
                    "usage": usage.chat_usage()
                }),
            );
        }
        output.push_str("data: [DONE]\n\n");
        self.finished = true;
    }

    fn fail_into(
        &mut self,
        output: &mut String,
        error_type: &str,
        code: Option<&str>,
        message: &str,
    ) {
        push_data(
            output,
            &json!({ "error": { "message": message, "type": error_type, "param": null, "code": code } }),
        );
        output.push_str("data: [DONE]\n\n");
        self.finished = true;
        self.failed = true;
        self.failure = Some(message.to_string());
    }
}

/// Responses 的 status / incomplete_details → Chat 的 finish_reason。
fn finish_reason(response: &Value, has_tool_calls: bool) -> &'static str {
    match incomplete_reason(response) {
        Some("content_filter") => "content_filter",
        Some(_) => "length",
        None if has_tool_calls => "tool_calls",
        None => "stop",
    }
}

/// 非流式：Responses JSON → `chat.completion`。
pub fn responses_json_to_chat(response: &Value, model: &str) -> Value {
    let mut text = String::new();
    let mut refusal = String::new();
    let mut reasoning = Vec::new();
    let mut tool_calls = Vec::new();
    for item in response
        .get("output")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        match item.get("type").and_then(Value::as_str) {
            Some("reasoning") => {
                let piece = reasoning_text(item);
                if !piece.is_empty() {
                    reasoning.push(piece);
                }
            }
            Some("message") => {
                text.push_str(&message_text_of(item, &["output_text", "text"]));
                refusal.push_str(&message_text_of(item, &["refusal"]));
            }
            Some("function_call") => {
                let arguments = match item.get("arguments") {
                    Some(Value::String(arguments)) => arguments.clone(),
                    Some(Value::Null) | None => "{}".to_string(),
                    Some(other) => other.to_string(),
                };
                tool_calls.push(json!({
                    "id": tool_call_id(item),
                    "type": "function",
                    "function": { "name": str_field(item, "name"), "arguments": arguments }
                }));
            }
            _ => {}
        }
    }
    let mut message = Map::new();
    message.insert("role".to_string(), json!("assistant"));
    // 只有工具调用、没有文字时按 OpenAI 的习惯给 null。
    message.insert(
        "content".to_string(),
        if text.is_empty() && !tool_calls.is_empty() {
            Value::Null
        } else {
            json!(text)
        },
    );
    message.insert(
        "refusal".to_string(),
        if refusal.is_empty() {
            Value::Null
        } else {
            json!(refusal)
        },
    );
    if !reasoning.is_empty() {
        message.insert(
            "reasoning_content".to_string(),
            json!(reasoning.join("\n\n")),
        );
    }
    let has_tool_calls = !tool_calls.is_empty();
    if has_tool_calls {
        message.insert("tool_calls".to_string(), Value::Array(tool_calls));
    }
    json!({
        "id": new_completion_id(),
        "object": "chat.completion",
        "created": response.get("created_at").and_then(Value::as_u64).unwrap_or_else(unix_now),
        "model": model,
        "choices": [{
            "index": 0,
            "message": Value::Object(message),
            "logprobs": null,
            "finish_reason": finish_reason(response, has_tool_calls)
        }],
        "usage": responses_usage(response.get("usage")).chat_usage()
    })
}

/// 客户端要流式、上游却回了整段 JSON 时：把完整的 `chat.completion` 拆成 chunk 流。
///
/// 拿不到客户端的 `stream_options`，usage 挂在带 finish_reason 的最后一块上
/// （OpenAI SDK 从任意块读 usage；不要 usage 的客户端会忽略它），不另发 `choices: []` 的块，
/// 免得只认 `choices[0]` 的客户端出错。
pub fn chat_completion_to_sse(completion: &Value) -> Vec<u8> {
    let mut output = String::new();
    let id = completion
        .get("id")
        .cloned()
        .unwrap_or_else(|| json!(new_completion_id()));
    let created = completion
        .get("created")
        .cloned()
        .unwrap_or_else(|| json!(unix_now()));
    let model = completion.get("model").cloned().unwrap_or(json!(""));
    let chunk = |index: u64, delta: Value, finish_reason: Value| {
        json!({
            "id": id,
            "object": "chat.completion.chunk",
            "created": created,
            "model": model,
            "choices": [{
                "index": index,
                "delta": delta,
                "logprobs": null,
                "finish_reason": finish_reason
            }]
        })
    };
    let choices = completion
        .get("choices")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    for (position, choice) in choices.iter().enumerate() {
        let index = choice
            .get("index")
            .and_then(Value::as_u64)
            .unwrap_or(position as u64);
        let message = choice.get("message").cloned().unwrap_or(Value::Null);
        push_data(
            &mut output,
            &chunk(
                index,
                json!({ "role": "assistant", "content": "" }),
                Value::Null,
            ),
        );
        for field in ["reasoning_content", "content", "refusal"] {
            if let Some(text) = message.get(field).and_then(Value::as_str)
                && !text.is_empty()
            {
                push_data(
                    &mut output,
                    &chunk(index, json!({ field: text }), Value::Null),
                );
            }
        }
        if let Some(calls) = message.get("tool_calls").and_then(Value::as_array)
            && !calls.is_empty()
        {
            let calls = calls
                .iter()
                .enumerate()
                .map(|(call_index, call)| {
                    let mut call = call.clone();
                    call["index"] = json!(call_index);
                    call
                })
                .collect::<Vec<_>>();
            push_data(
                &mut output,
                &chunk(index, json!({ "tool_calls": calls }), Value::Null),
            );
        }
        let mut last = chunk(
            index,
            json!({}),
            choice
                .get("finish_reason")
                .cloned()
                .unwrap_or(json!("stop")),
        );
        if position + 1 == choices.len()
            && let Some(usage) = completion.get("usage")
        {
            last["usage"] = usage.clone();
        }
        push_data(&mut output, &last);
    }
    output.push_str("data: [DONE]\n\n");
    output.into_bytes()
}

/// HTTP 状态码 → OpenAI 错误的（type, code）。
fn chat_error_kind(status: u16) -> (&'static str, Option<&'static str>) {
    match status {
        400 | 413 | 422 => ("invalid_request_error", None),
        401 => ("authentication_error", Some("invalid_api_key")),
        402 => ("insufficient_quota", Some("insufficient_quota")),
        403 => ("permission_error", None),
        404 => ("not_found_error", Some("model_not_found")),
        429 => ("rate_limit_error", Some("rate_limit_exceeded")),
        503 | 529 => ("server_error", Some("server_overloaded")),
        _ => ("server_error", None),
    }
}

/// OpenAI 风格的错误体。
pub fn chat_error_body(status: u16, message: &str) -> Value {
    let (error_type, code) = chat_error_kind(status);
    json!({
        "error": { "message": message, "type": error_type, "param": null, "code": code }
    })
}

fn push_data(output: &mut String, data: &Value) {
    output.push_str("data: ");
    output.push_str(&data.to_string());
    output.push_str("\n\n");
}

fn new_completion_id() -> String {
    format!("chatcmpl-{}", uuid::Uuid::new_v4().simple())
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

fn str_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

// ---------------------------------------------------------------------------
// 以下是读取 Responses SSE 的公共工具，gemini_outbound 也用。
// ---------------------------------------------------------------------------

/// Responses SSE 的增量读取：处理 UTF-8 多字节字符被切开、事件被切成多段，产出（类型，事件）。
#[derive(Debug, Default)]
pub(crate) struct ResponsesSseReader {
    buffer: String,
    utf8_remainder: Vec<u8>,
}

impl ResponsesSseReader {
    pub(crate) fn push(&mut self, bytes: &[u8]) -> Vec<(String, Value)> {
        let mut data = std::mem::take(&mut self.utf8_remainder);
        data.extend_from_slice(bytes);
        let text = match std::str::from_utf8(&data) {
            Ok(text) => text.to_string(),
            // 只有结尾那个字符还没收完（error_len 为 None）才留到下一块；中间夹着坏字节时按替换字符处理，
            // 不然之后每一块都在第 0 个字节解码失败，整个流就卡住不再输出。
            Err(error) if error.error_len().is_none() => {
                let valid = error.valid_up_to();
                self.utf8_remainder = data[valid..].to_vec();
                String::from_utf8_lossy(&data[..valid]).to_string()
            }
            Err(_) => String::from_utf8_lossy(&data).to_string(),
        };
        self.buffer.push_str(&text);
        let mut events = Vec::new();
        while let Some(block) = take_sse_block(&mut self.buffer) {
            events.extend(parse_sse_block(&block));
        }
        events
    }

    /// 流结束：没有结尾空行的最后一块也解析掉。
    pub(crate) fn drain(&mut self) -> Vec<(String, Value)> {
        let remainder = std::mem::take(&mut self.utf8_remainder);
        if !remainder.is_empty() {
            self.buffer.push_str(&String::from_utf8_lossy(&remainder));
        }
        let rest = std::mem::take(&mut self.buffer);
        if rest.trim().is_empty() {
            return Vec::new();
        }
        parse_sse_block(&rest).into_iter().collect()
    }
}

fn parse_sse_block(block: &str) -> Option<(String, Value)> {
    let mut event_name = None;
    let mut data_lines = Vec::new();
    for line in block.lines() {
        if let Some(value) = line.strip_prefix("event:") {
            event_name = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("data:") {
            data_lines.push(value.strip_prefix(' ').unwrap_or(value).to_string());
        }
    }
    let data = data_lines.join("\n");
    if data.trim().is_empty() || data.trim() == "[DONE]" {
        return None;
    }
    let event = serde_json::from_str::<Value>(&data).ok()?;
    let kind = event
        .get("type")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or(event_name)
        .unwrap_or_default();
    Some((kind, event))
}

fn take_sse_block(buffer: &mut String) -> Option<String> {
    let lf = buffer.find("\n\n").map(|index| (index, 2));
    let crlf = buffer.find("\r\n\r\n").map(|index| (index, 4));
    let (index, delimiter) = match (lf, crlf) {
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
    buffer.drain(..index + delimiter);
    Some(block)
}

/// 流式事件所属 item 的标识：优先 item_id，其次 item.id，再次 output_index。
pub(crate) fn item_key(event: &Value) -> String {
    event
        .get("item_id")
        .and_then(Value::as_str)
        .or_else(|| event.pointer("/item/id").and_then(Value::as_str))
        .map(str::to_string)
        .or_else(|| {
            event
                .get("output_index")
                .and_then(Value::as_u64)
                .map(|index| format!("#{index}"))
        })
        .unwrap_or_default()
}

/// 终止事件 output 数组里第 `index` 项的标识，与流式事件的 item 键保持同一套规则。
pub(crate) fn output_item_key(item: &Value, index: usize) -> String {
    item.get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| format!("#{index}"))
}

pub(crate) fn tool_call_id(item: &Value) -> String {
    item.get("call_id")
        .or_else(|| item.get("id"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// reasoning 项的可读文本：优先摘要，其次原始推理文本。
pub(crate) fn reasoning_text(item: &Value) -> String {
    let join = |key: &str, separator: &str| {
        item.get(key)
            .and_then(Value::as_array)
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(|part| part.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join(separator)
            })
            .unwrap_or_default()
    };
    let summary = join("summary", "\n\n");
    if !summary.is_empty() {
        return summary;
    }
    join("content", "")
}

/// message 项的全部文字（拒答也算进去）。
pub(crate) fn message_text(item: &Value) -> String {
    message_text_of(item, &["output_text", "refusal", "text"])
}

fn message_text_of(item: &Value, kinds: &[&str]) -> String {
    item.get("content")
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter(|part| {
                    part.get("type")
                        .and_then(Value::as_str)
                        .is_some_and(|kind| kinds.contains(&kind))
                })
                .filter_map(|part| {
                    part.get("text")
                        .or_else(|| part.get("refusal"))
                        .and_then(Value::as_str)
                })
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

/// 未完成时的原因：`status` 为 incomplete 时给 `incomplete_details.reason`（缺省按输出上限算）。
pub(crate) fn incomplete_reason(response: &Value) -> Option<&str> {
    if response.get("status").and_then(Value::as_str) != Some("incomplete") {
        return None;
    }
    Some(
        response
            .pointer("/incomplete_details/reason")
            .and_then(Value::as_str)
            .unwrap_or("max_output_tokens"),
    )
}

/// 失败事件的（消息，小写错误码）。
pub(crate) fn upstream_failure(event: &Value) -> (String, String) {
    let message = event
        .pointer("/response/error/message")
        .or_else(|| event.pointer("/error/message"))
        .or_else(|| event.get("message"))
        .and_then(Value::as_str)
        .unwrap_or("上游返回失败")
        .to_string();
    let code = event
        .pointer("/response/error/code")
        .or_else(|| event.pointer("/response/error/type"))
        .or_else(|| event.pointer("/error/code"))
        .or_else(|| event.pointer("/error/type"))
        .or_else(|| event.get("code"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    (message, code)
}

/// Responses 的 usage（`input_tokens` 含缓存命中，`output_tokens` 含推理）。
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ResponsesUsage {
    pub(crate) input_tokens: u64,
    pub(crate) cached_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) reasoning_tokens: u64,
    pub(crate) total_tokens: u64,
}

pub(crate) fn responses_usage(usage: Option<&Value>) -> ResponsesUsage {
    let number = |pointer: &str| {
        usage
            .and_then(|usage| usage.pointer(pointer))
            .and_then(Value::as_u64)
    };
    let input_tokens = number("/input_tokens").unwrap_or(0);
    let output_tokens = number("/output_tokens").unwrap_or(0);
    ResponsesUsage {
        input_tokens,
        cached_tokens: number("/input_tokens_details/cached_tokens").unwrap_or(0),
        output_tokens,
        reasoning_tokens: number("/output_tokens_details/reasoning_tokens").unwrap_or(0),
        total_tokens: number("/total_tokens").unwrap_or(input_tokens + output_tokens),
    }
}

impl ResponsesUsage {
    /// 请求日志用的 usage：与 Anthropic 那边一致，input 含缓存命中。
    pub(crate) fn request_usage(&self) -> RequestUsage {
        RequestUsage {
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            cached_tokens: self.cached_tokens,
            reasoning_tokens: self.reasoning_tokens,
        }
    }

    /// Chat 的 usage：`prompt_tokens` 同样含缓存命中，`completion_tokens` 含推理。
    fn chat_usage(&self) -> Value {
        json!({
            "prompt_tokens": self.input_tokens,
            "completion_tokens": self.output_tokens,
            "total_tokens": self.total_tokens,
            "prompt_tokens_details": { "cached_tokens": self.cached_tokens },
            "completion_tokens_details": { "reasoning_tokens": self.reasoning_tokens }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sse(events: &[Value]) -> String {
        events
            .iter()
            .map(|event| {
                format!(
                    "event: {}\ndata: {}\n\n",
                    event["type"].as_str().unwrap(),
                    event
                )
            })
            .collect()
    }

    /// 解析输出：返回所有 JSON 块，以及是否以 `[DONE]` 结尾。
    fn parse(output: &[u8]) -> (Vec<Value>, bool) {
        let text = String::from_utf8(output.to_vec()).unwrap();
        let mut done = false;
        let mut chunks = Vec::new();
        for block in text.split("\n\n").filter(|block| !block.trim().is_empty()) {
            assert!(!done, "[DONE] 之后不应再有内容：{text}");
            let data = block.strip_prefix("data: ").expect("每块都是 data 行");
            if data == "[DONE]" {
                done = true;
            } else {
                chunks.push(serde_json::from_str(data).unwrap());
            }
        }
        (chunks, done)
    }

    fn deltas(chunks: &[Value]) -> Vec<&Value> {
        chunks
            .iter()
            .filter_map(|chunk| chunk.pointer("/choices/0/delta"))
            .collect()
    }

    fn joined(chunks: &[Value], field: &str) -> String {
        deltas(chunks)
            .iter()
            .filter_map(|delta| delta.get(field).and_then(Value::as_str))
            .collect()
    }

    /// 按 index 拼出每个工具调用的（id，name，arguments）。
    fn tool_calls(chunks: &[Value]) -> Vec<(String, String, String)> {
        let mut calls: Vec<(String, String, String)> = Vec::new();
        for delta in deltas(chunks) {
            for call in delta
                .get("tool_calls")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let index = call["index"].as_u64().unwrap() as usize;
                if index == calls.len() {
                    assert!(call.get("id").is_some(), "首个分片必须带 id：{call}");
                    calls.push((
                        call["id"].as_str().unwrap().to_string(),
                        call["function"]["name"].as_str().unwrap().to_string(),
                        String::new(),
                    ));
                } else {
                    assert!(index < calls.len(), "index 必须按出现顺序递增：{call}");
                    assert!(
                        call.get("id").is_none(),
                        "同一个工具调用不能重复发头：{call}"
                    );
                }
                if let Some(arguments) = call["function"]["arguments"].as_str() {
                    calls[index].2.push_str(arguments);
                }
            }
        }
        calls
    }

    fn sample_stream() -> String {
        sse(&[
            json!({"type":"response.created","response":{"id":"resp_1"}}),
            json!({"type":"response.output_item.added","output_index":0,"item":{"id":"rs_1","type":"reasoning"}}),
            json!({"type":"response.reasoning_summary_text.delta","item_id":"rs_1","summary_index":0,"delta":"想一想"}),
            json!({"type":"response.reasoning_summary_text.delta","item_id":"rs_1","summary_index":1,"delta":"再想"}),
            json!({"type":"response.output_item.done","output_index":0,"item":{"id":"rs_1","type":"reasoning","summary":[{"type":"summary_text","text":"想一想"},{"type":"summary_text","text":"再想"}]}}),
            json!({"type":"response.output_item.added","output_index":1,"item":{"id":"msg_1","type":"message"}}),
            json!({"type":"response.output_text.delta","item_id":"msg_1","delta":"好的，"}),
            json!({"type":"response.output_text.delta","item_id":"msg_1","delta":"我来看看"}),
            json!({"type":"response.output_item.done","output_index":1,"item":{"id":"msg_1","type":"message","content":[{"type":"output_text","text":"好的，我来看看"}]}}),
            json!({"type":"response.output_item.added","output_index":2,"item":{"id":"fc_1","type":"function_call","call_id":"call_1","name":"Bash"}}),
            json!({"type":"response.function_call_arguments.delta","item_id":"fc_1","delta":"{\"command\":"}),
            json!({"type":"response.function_call_arguments.delta","item_id":"fc_1","delta":"\"ls\"}"}),
            json!({"type":"response.output_item.done","output_index":2,"item":{"id":"fc_1","type":"function_call","call_id":"call_1","name":"Bash","arguments":"{\"command\":\"ls\"}"}}),
            json!({"type":"response.completed","response":{"id":"resp_1","status":"completed","usage":{"input_tokens":1000,"input_tokens_details":{"cached_tokens":800},"output_tokens":50,"output_tokens_details":{"reasoning_tokens":20},"total_tokens":1050}}}),
        ]) + "data: [DONE]\n\n"
    }

    #[test]
    fn full_stream_translates_to_chat_chunks() {
        let mut converter = ResponsesSseToChatSse::new("gpt-x", true);
        let mut output = converter.push_bytes(sample_stream().as_bytes());
        output.extend(converter.finish());
        let (chunks, done) = parse(&output);
        assert!(done);
        assert_eq!(chunks[0]["object"], "chat.completion.chunk");
        assert_eq!(chunks[0]["model"], "gpt-x");
        assert_eq!(chunks[0]["choices"][0]["delta"]["role"], "assistant");
        assert!(chunks.iter().all(|chunk| chunk["id"] == chunks[0]["id"]));
        assert_eq!(joined(&chunks, "reasoning_content"), "想一想\n\n再想");
        assert_eq!(joined(&chunks, "content"), "好的，我来看看");
        assert_eq!(
            tool_calls(&chunks),
            vec![(
                "call_1".to_string(),
                "Bash".to_string(),
                "{\"command\":\"ls\"}".to_string()
            )]
        );
        let finish = &chunks[chunks.len() - 2];
        assert_eq!(finish["choices"][0]["finish_reason"], "tool_calls");
        let usage_chunk = chunks.last().unwrap();
        assert_eq!(usage_chunk["choices"], json!([]));
        assert_eq!(usage_chunk["usage"]["prompt_tokens"], 1000);
        assert_eq!(usage_chunk["usage"]["completion_tokens"], 50);
        assert_eq!(usage_chunk["usage"]["total_tokens"], 1050);
        assert_eq!(
            usage_chunk["usage"]["prompt_tokens_details"]["cached_tokens"],
            800
        );
        assert_eq!(
            usage_chunk["usage"]["completion_tokens_details"]["reasoning_tokens"],
            20
        );
        assert!(converter.is_finished());
        assert!(!converter.has_failed());
        let usage = converter.usage().unwrap();
        assert_eq!(usage.input_tokens, 1000);
        assert_eq!(usage.cached_tokens, 800);
        assert_eq!(usage.reasoning_tokens, 20);
    }

    #[test]
    fn usage_chunk_only_when_requested() {
        let mut converter = ResponsesSseToChatSse::new("m", false);
        let (chunks, done) = parse(&converter.push_bytes(sample_stream().as_bytes()));
        assert!(done);
        assert!(
            chunks
                .iter()
                .all(|chunk| chunk["choices"].as_array().is_some_and(|c| !c.is_empty()))
        );
        assert_eq!(
            chunks.last().unwrap()["choices"][0]["finish_reason"],
            "tool_calls"
        );
        // 不发 usage 块，但请求日志照样拿得到用量。
        assert_eq!(converter.usage().unwrap().output_tokens, 50);
    }

    #[test]
    fn byte_by_byte_feeding_matches_single_chunk() {
        let stream = sample_stream();
        let mut whole = ResponsesSseToChatSse::new("m", true);
        let mut expected = whole.push_bytes(stream.as_bytes());
        expected.extend(whole.finish());
        let mut split = ResponsesSseToChatSse::new("m", true);
        let mut actual = Vec::new();
        for byte in stream.as_bytes().chunks(1) {
            actual.extend(split.push_bytes(byte));
        }
        actual.extend(split.finish());
        let normalize = |chunks: Vec<Value>| {
            chunks
                .into_iter()
                .map(|mut chunk| {
                    chunk["id"] = json!("x");
                    chunk["created"] = json!(0);
                    chunk
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(normalize(parse(&actual).0), normalize(parse(&expected).0));
    }

    #[test]
    fn multibyte_character_split_across_chunks() {
        let stream = sse(&[
            json!({"type":"response.output_text.delta","item_id":"m","delta":"你好世界"}),
            json!({"type":"response.completed","response":{"status":"completed"}}),
        ]);
        let bytes = stream.as_bytes();
        // 切在「你」的三个字节中间。
        let cut = stream.find("你").unwrap() + 1;
        let mut converter = ResponsesSseToChatSse::new("m", false);
        let mut output = converter.push_bytes(&bytes[..cut]);
        output.extend(converter.push_bytes(&bytes[cut..]));
        let (chunks, done) = parse(&output);
        assert!(done);
        assert_eq!(joined(&chunks, "content"), "你好世界");
    }

    #[test]
    fn invalid_utf8_byte_does_not_stall_the_stream() {
        let mut converter = ResponsesSseToChatSse::new("m", false);
        let first = converter.push_bytes(b"event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"r\"}}\n\n\xff");
        let second = converter.push_bytes(b"\n\nevent: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"output_index\":0,\"delta\":\"hi\"}\n\n");
        let text = String::from_utf8_lossy(&[first, second].concat()).to_string();
        assert!(text.contains("hi"), "坏字节之后的事件照常输出：{text}");
    }

    #[test]
    fn interleaved_parallel_tool_calls_keep_separate_indexes() {
        let stream = sse(&[
            json!({"type":"response.output_item.added","output_index":0,"item":{"id":"fc_a","type":"function_call","call_id":"call_a","name":"Read"}}),
            json!({"type":"response.output_item.added","output_index":1,"item":{"id":"fc_b","type":"function_call","call_id":"call_b","name":"Grep"}}),
            json!({"type":"response.function_call_arguments.delta","item_id":"fc_b","delta":"{\"p\":"}),
            json!({"type":"response.function_call_arguments.delta","item_id":"fc_a","delta":"{\"f\":2}"}),
            json!({"type":"response.function_call_arguments.delta","item_id":"fc_b","delta":"1}"}),
            json!({"type":"response.output_item.done","output_index":1,"item":{"id":"fc_b","type":"function_call","call_id":"call_b","name":"Grep","arguments":"{\"p\":1}"}}),
            json!({"type":"response.output_item.done","output_index":0,"item":{"id":"fc_a","type":"function_call","call_id":"call_a","name":"Read","arguments":"{\"f\":2}"}}),
            // 重复的完成事件不能再发一遍参数。
            json!({"type":"response.output_item.done","output_index":0,"item":{"id":"fc_a","type":"function_call","call_id":"call_a","name":"Read","arguments":"{\"f\":2}"}}),
            json!({"type":"response.completed","response":{"status":"completed","output":[
                {"id":"fc_a","type":"function_call","call_id":"call_a","name":"Read","arguments":"{\"f\":2}"},
                {"id":"fc_b","type":"function_call","call_id":"call_b","name":"Grep","arguments":"{\"p\":1}"}
            ]}}),
        ]);
        let mut converter = ResponsesSseToChatSse::new("m", false);
        let (chunks, _) = parse(&converter.push_bytes(stream.as_bytes()));
        assert_eq!(
            tool_calls(&chunks),
            vec![
                (
                    "call_a".to_string(),
                    "Read".to_string(),
                    "{\"f\":2}".to_string()
                ),
                (
                    "call_b".to_string(),
                    "Grep".to_string(),
                    "{\"p\":1}".to_string()
                ),
            ]
        );
    }

    #[test]
    fn streamed_tool_done_only_sends_missing_suffix_and_late_header_buffers() {
        let stream = sse(&[
            json!({"type":"response.output_item.added","output_index":0,"item":{"id":"fc_1","type":"function_call","call_id":"call_1","name":"Bash"}}),
            json!({"type":"response.function_call_arguments.delta","item_id":"fc_1","delta":"{\"command\":"}),
            json!({"type":"response.output_item.done","output_index":0,"item":{"id":"fc_1","type":"function_call","call_id":"call_1","name":"Bash","arguments":"{\"command\":\"pwd\"}"}}),
            // 没有 added、先来参数增量的工具：缓冲到完成事件再整段发出。
            json!({"type":"response.function_call_arguments.delta","item_id":"fc_2","delta":"{\"x\":"}),
            json!({"type":"response.output_item.done","output_index":1,"item":{"id":"fc_2","type":"function_call","call_id":"call_2","name":"Read","arguments":"{\"x\":1}"}}),
            json!({"type":"response.completed","response":{"status":"completed"}}),
        ]);
        let mut converter = ResponsesSseToChatSse::new("m", false);
        let (chunks, _) = parse(&converter.push_bytes(stream.as_bytes()));
        assert_eq!(
            tool_calls(&chunks),
            vec![
                (
                    "call_1".to_string(),
                    "Bash".to_string(),
                    "{\"command\":\"pwd\"}".to_string()
                ),
                (
                    "call_2".to_string(),
                    "Read".to_string(),
                    "{\"x\":1}".to_string()
                ),
            ]
        );
    }

    #[test]
    fn completed_only_output_is_backfilled_without_duplicates() {
        let stream = sse(&[
            json!({"type":"response.output_text.delta","item_id":"msg_1","delta":"流式"}),
            json!({"type":"response.completed","response":{
                "status":"completed",
                "output":[
                    {"id":"rs_x","type":"reasoning","summary":[{"type":"summary_text","text":"补发推理"}]},
                    {"id":"msg_1","type":"message","content":[{"type":"output_text","text":"流式"}]},
                    {"id":"msg_2","type":"message","content":[{"type":"output_text","text":"整段"}]}
                ]
            }}),
        ]);
        let mut converter = ResponsesSseToChatSse::new("m", false);
        let (chunks, _) = parse(&converter.push_bytes(stream.as_bytes()));
        assert_eq!(joined(&chunks, "content"), "流式整段");
        assert_eq!(joined(&chunks, "reasoning_content"), "补发推理");
        assert_eq!(
            chunks.last().unwrap()["choices"][0]["finish_reason"],
            "stop"
        );
    }

    #[test]
    fn incomplete_maps_to_length() {
        let stream = sse(&[
            json!({"type":"response.output_text.delta","item_id":"m","delta":"x"}),
            json!({"type":"response.incomplete","response":{"status":"incomplete","incomplete_details":{"reason":"max_output_tokens"}}}),
        ]);
        let mut converter = ResponsesSseToChatSse::new("m", false);
        let (chunks, done) = parse(&converter.push_bytes(stream.as_bytes()));
        assert!(done);
        assert_eq!(
            chunks.last().unwrap()["choices"][0]["finish_reason"],
            "length"
        );
    }

    #[test]
    fn failed_and_truncated_streams_emit_error_then_done() {
        let failed = sse(&[
            json!({"type":"response.output_text.delta","item_id":"m","delta":"半句"}),
            json!({"type":"response.failed","response":{"error":{"code":"rate_limit_exceeded","message":"slow down"}}}),
            json!({"type":"response.output_text.delta","item_id":"m","delta":"不该出现"}),
        ]);
        let mut converter = ResponsesSseToChatSse::new("m", false);
        let (chunks, done) = parse(&converter.push_bytes(failed.as_bytes()));
        assert!(done);
        let error = &chunks.last().unwrap()["error"];
        assert_eq!(error["message"], "slow down");
        assert_eq!(error["type"], "rate_limit_error");
        assert_eq!(error["code"], "rate_limit_exceeded");
        assert_eq!(joined(&chunks, "content"), "半句");
        assert!(converter.has_failed());
        assert_eq!(converter.failure(), Some("slow down"));
        assert!(converter.finish().is_empty(), "已结束的流不再追加内容");

        let mut truncated = ResponsesSseToChatSse::new("m", false);
        truncated.push_bytes(
            sse(&[json!({"type":"response.output_text.delta","item_id":"m","delta":"x"})])
                .as_bytes(),
        );
        let (chunks, done) = parse(&truncated.finish());
        assert!(done);
        assert!(chunks.last().unwrap().get("error").is_some());
        assert!(truncated.has_failed());

        let mut broken = ResponsesSseToChatSse::new("m", false);
        let (chunks, done) = parse(&broken.fail("连接断开"));
        assert!(done);
        assert_eq!(chunks[0]["error"]["message"], "连接断开");
    }

    #[test]
    fn final_block_without_trailing_blank_line_is_parsed_on_finish() {
        let stream =
            sse(&[json!({"type":"response.output_text.delta","item_id":"m","delta":"hi"})])
                + "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\"}}";
        let mut converter = ResponsesSseToChatSse::new("m", false);
        let mut output = converter.push_bytes(stream.as_bytes());
        output.extend(converter.finish());
        let (chunks, done) = parse(&output);
        assert!(done);
        assert!(!converter.has_failed());
        assert_eq!(
            chunks.last().unwrap()["choices"][0]["finish_reason"],
            "stop"
        );
    }

    #[test]
    fn non_stream_json_translation() {
        let response = json!({
            "status": "completed",
            "output": [
                {"type":"reasoning","summary":[{"type":"summary_text","text":"想"}]},
                {"type":"message","content":[{"type":"output_text","text":"答"}]},
                {"type":"function_call","call_id":"call_9","name":"Read","arguments":"{\"file\":\"a\"}"}
            ],
            "usage": {"input_tokens": 10, "input_tokens_details": {"cached_tokens": 4}, "output_tokens": 3}
        });
        let completion = responses_json_to_chat(&response, "gpt-x");
        assert_eq!(completion["object"], "chat.completion");
        assert_eq!(completion["model"], "gpt-x");
        let message = &completion["choices"][0]["message"];
        assert_eq!(message["role"], "assistant");
        assert_eq!(message["content"], "答");
        assert_eq!(message["reasoning_content"], "想");
        assert_eq!(message["tool_calls"][0]["id"], "call_9");
        assert_eq!(message["tool_calls"][0]["type"], "function");
        assert_eq!(message["tool_calls"][0]["function"]["name"], "Read");
        assert_eq!(
            message["tool_calls"][0]["function"]["arguments"],
            "{\"file\":\"a\"}"
        );
        assert_eq!(completion["choices"][0]["finish_reason"], "tool_calls");
        assert_eq!(completion["usage"]["prompt_tokens"], 10);
        assert_eq!(completion["usage"]["completion_tokens"], 3);
        assert_eq!(completion["usage"]["total_tokens"], 13);
        assert_eq!(
            completion["usage"]["prompt_tokens_details"]["cached_tokens"],
            4
        );

        let tool_only = responses_json_to_chat(
            &json!({"status":"completed","output":[{"type":"function_call","call_id":"c","name":"n","arguments":"{}"}]}),
            "m",
        );
        assert!(tool_only["choices"][0]["message"]["content"].is_null());
        let truncated = responses_json_to_chat(
            &json!({"status":"incomplete","incomplete_details":{"reason":"max_output_tokens"},"output":[]}),
            "m",
        );
        assert_eq!(truncated["choices"][0]["finish_reason"], "length");
        assert_eq!(truncated["choices"][0]["message"]["content"], "");
    }

    #[test]
    fn whole_completion_replays_as_chunk_stream() {
        let completion = responses_json_to_chat(
            &json!({
                "status": "completed",
                "output": [
                    {"type":"reasoning","summary":[{"type":"summary_text","text":"想"}]},
                    {"type":"message","content":[{"type":"output_text","text":"hi"}]},
                    {"type":"function_call","call_id":"c1","name":"Read","arguments":"{\"f\":1}"}
                ],
                "usage": {"input_tokens": 5, "output_tokens": 2}
            }),
            "m",
        );
        let (chunks, done) = parse(&chat_completion_to_sse(&completion));
        assert!(done);
        assert!(chunks.iter().all(|chunk| chunk["id"] == completion["id"]));
        assert_eq!(chunks[0]["choices"][0]["delta"]["role"], "assistant");
        assert_eq!(joined(&chunks, "reasoning_content"), "想");
        assert_eq!(joined(&chunks, "content"), "hi");
        assert_eq!(
            tool_calls(&chunks),
            vec![(
                "c1".to_string(),
                "Read".to_string(),
                "{\"f\":1}".to_string()
            )]
        );
        let last = chunks.last().unwrap();
        assert_eq!(last["choices"][0]["finish_reason"], "tool_calls");
        assert_eq!(last["usage"]["prompt_tokens"], 5);
    }

    #[test]
    fn error_body_shape() {
        let body = chat_error_body(429, "too many");
        assert_eq!(body["error"]["message"], "too many");
        assert_eq!(body["error"]["type"], "rate_limit_error");
        assert_eq!(body["error"]["code"], "rate_limit_exceeded");
        assert_eq!(
            chat_error_body(400, "bad")["error"]["type"],
            "invalid_request_error"
        );
        assert!(chat_error_body(502, "x")["error"]["code"].is_null());
        assert_eq!(
            chat_error_body(401, "x")["error"]["code"],
            "invalid_api_key"
        );
    }
}
