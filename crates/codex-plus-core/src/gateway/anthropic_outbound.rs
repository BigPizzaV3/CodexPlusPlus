//! Responses 响应（JSON / SSE）→ Anthropic Messages 响应。
//!
//! 现有代理已经能把任意上游（Chat / Anthropic / 原生 Responses）统一成 Responses SSE；
//! 这里只做最后一跳，把它翻成 Claude Code 认识的 Anthropic 事件流。
//!
//! Anthropic 的内容块必须一个接一个（start → delta… → stop），不能交错。Responses 里
//! 多个工具调用可能交错输出参数，所以只有「当前打开」的块实时转发，其余先缓冲，
//! 等该项完成时整块补发。

use std::collections::HashMap;

use serde_json::{Value, json};

use crate::gateway::anthropic_inbound::encode_signature;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockKind {
    Text,
    Thinking,
    Tool,
}

#[derive(Debug, Clone)]
struct OpenBlock {
    index: u32,
    kind: BlockKind,
    item_key: String,
    /// 工具块是否已经发过参数增量。
    emitted_arguments: bool,
    /// 思考块当前所在的摘要段（换段时补两个换行）。
    summary_index: Option<u64>,
}

#[derive(Debug, Clone, Default)]
struct PendingTool {
    call_id: String,
    name: String,
    arguments: String,
}

/// Responses SSE → Anthropic SSE 的增量转换器。
pub struct ResponsesSseToAnthropicSse {
    buffer: String,
    utf8_remainder: Vec<u8>,
    model: String,
    message_id: String,
    input_tokens_estimate: u64,
    started: bool,
    finished: bool,
    failed: bool,
    next_index: u32,
    open: Option<OpenBlock>,
    /// 没能实时转发、等完成时补发的工具调用（键是 Responses item 标识）。
    pending_tools: HashMap<String, PendingTool>,
    /// 已经整块发出过的 item，防止重复。
    completed_items: Vec<String>,
    emitted_tool_use: bool,
    usage: Option<crate::request_log::RequestUsage>,
    failure: Option<String>,
}

impl ResponsesSseToAnthropicSse {
    /// `model` 是回给客户端的模型名（用客户端请求里的原名，避免 Claude Code 认不出来）。
    pub fn new(model: &str, input_tokens_estimate: u64) -> Self {
        Self {
            buffer: String::new(),
            utf8_remainder: Vec::new(),
            model: model.to_string(),
            message_id: format!("msg_{}", uuid::Uuid::new_v4().simple()),
            input_tokens_estimate,
            started: false,
            finished: false,
            failed: false,
            next_index: 0,
            open: None,
            pending_tools: HashMap::new(),
            completed_items: Vec::new(),
            emitted_tool_use: false,
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

    pub fn usage(&self) -> Option<crate::request_log::RequestUsage> {
        self.usage.clone()
    }

    pub fn push_bytes(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut data = std::mem::take(&mut self.utf8_remainder);
        data.extend_from_slice(bytes);
        let text = match std::str::from_utf8(&data) {
            Ok(text) => text.to_string(),
            Err(error) => {
                let valid = error.valid_up_to();
                self.utf8_remainder = data[valid..].to_vec();
                String::from_utf8_lossy(&data[..valid]).to_string()
            }
        };
        self.buffer.push_str(&text);
        let mut output = String::new();
        while let Some(block) = take_sse_block(&mut self.buffer) {
            self.handle_block(&block, &mut output);
        }
        output.into_bytes()
    }

    /// 流结束：没见到终止事件就补一个错误事件，客户端才知道这一轮没完成。
    pub fn finish(&mut self) -> Vec<u8> {
        let mut output = String::new();
        if !self.buffer.trim().is_empty() {
            let rest = std::mem::take(&mut self.buffer);
            self.handle_block(&rest, &mut output);
        }
        if !self.finished {
            self.fail_into(&mut output, "api_error", "上游在完成事件前结束了响应流");
        }
        output.into_bytes()
    }

    /// 上游读取出错时调用。
    pub fn fail(&mut self, message: &str) -> Vec<u8> {
        let mut output = String::new();
        if !self.finished {
            self.fail_into(&mut output, "api_error", message);
        }
        output.into_bytes()
    }

    fn handle_block(&mut self, block: &str, output: &mut String) {
        if self.finished {
            return;
        }
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
            return;
        }
        let Ok(event) = serde_json::from_str::<Value>(&data) else {
            return;
        };
        let kind = event
            .get("type")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or(event_name)
            .unwrap_or_default();
        self.ensure_started(output);
        match kind.as_str() {
            "response.output_text.delta" | "response.refusal.delta" => {
                let delta = event
                    .get("delta")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if delta.is_empty() {
                    return;
                }
                let key = item_key(&event);
                self.open_block(BlockKind::Text, &key, None, output);
                push_event(
                    output,
                    "content_block_delta",
                    json!({
                        "type": "content_block_delta",
                        "index": self.open.as_ref().map(|block| block.index).unwrap_or(0),
                        "delta": { "type": "text_delta", "text": delta }
                    }),
                );
            }
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                let delta = event
                    .get("delta")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if delta.is_empty() {
                    return;
                }
                let key = item_key(&event);
                let summary_index = event.get("summary_index").and_then(Value::as_u64);
                self.open_block(BlockKind::Thinking, &key, None, output);
                let mut text = delta.to_string();
                if let Some(open) = self.open.as_mut() {
                    if let (Some(previous), Some(current)) = (open.summary_index, summary_index)
                        && previous != current
                    {
                        text = format!("\n\n{text}");
                    }
                    if summary_index.is_some() {
                        open.summary_index = summary_index;
                    }
                }
                push_event(
                    output,
                    "content_block_delta",
                    json!({
                        "type": "content_block_delta",
                        "index": self.open.as_ref().map(|block| block.index).unwrap_or(0),
                        "delta": { "type": "thinking_delta", "thinking": text }
                    }),
                );
            }
            "response.output_item.added" => {
                let item = event.get("item").cloned().unwrap_or(Value::Null);
                if item.get("type").and_then(Value::as_str) == Some("function_call") {
                    let key = item_key(&event);
                    let tool = PendingTool {
                        call_id: tool_call_id(&item),
                        name: item
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        arguments: String::new(),
                    };
                    // 没有其它块在实时转发时立即开始这个工具块；否则先缓冲，完成时补发。
                    if self.open.is_none() {
                        self.start_tool_block(&key, &tool, output);
                    } else {
                        self.pending_tools.insert(key, tool);
                    }
                }
            }
            "response.function_call_arguments.delta" => {
                let delta = event
                    .get("delta")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if delta.is_empty() {
                    return;
                }
                let key = item_key(&event);
                if let Some(open) = self.open.as_mut()
                    && open.kind == BlockKind::Tool
                    && open.item_key == key
                {
                    open.emitted_arguments = true;
                    let index = open.index;
                    push_event(
                        output,
                        "content_block_delta",
                        json!({
                            "type": "content_block_delta",
                            "index": index,
                            "delta": { "type": "input_json_delta", "partial_json": delta }
                        }),
                    );
                } else {
                    self.pending_tools
                        .entry(key)
                        .or_default()
                        .arguments
                        .push_str(delta);
                }
            }
            "response.output_item.done" => {
                let item = event.get("item").cloned().unwrap_or(Value::Null);
                let key = item_key(&event);
                self.item_done(&key, &item, output);
            }
            "response.completed" | "response.incomplete" => {
                let response = event.get("response").cloned().unwrap_or(Value::Null);
                // 有的上游只在终止事件里给完整 output：补发还没见过的项。
                if let Some(items) = response.get("output").and_then(Value::as_array) {
                    for (index, item) in items.iter().enumerate() {
                        let key = item
                            .get("id")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                            .unwrap_or_else(|| format!("#{index}"));
                        if !self.completed_items.contains(&key) && !self.has_seen(&key) {
                            self.item_done(&key, item, output);
                        }
                    }
                }
                self.close_open(output);
                self.finish_message(&response, output);
            }
            "response.failed" | "error" => {
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
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                let error_type = if code.contains("overload") || code.contains("capacity") {
                    "overloaded_error"
                } else if code.contains("rate_limit") {
                    "rate_limit_error"
                } else {
                    "api_error"
                };
                self.close_open(output);
                self.fail_into(output, error_type, &message);
            }
            _ => {}
        }
    }

    fn has_seen(&self, key: &str) -> bool {
        self.open.as_ref().is_some_and(|open| open.item_key == key)
            || self.pending_tools.contains_key(key)
    }

    fn item_done(&mut self, key: &str, item: &Value, output: &mut String) {
        if self.completed_items.iter().any(|done| done == key) {
            return;
        }
        match item.get("type").and_then(Value::as_str) {
            Some("reasoning") => {
                let encrypted = item
                    .get("encrypted_content")
                    .and_then(Value::as_str)
                    .filter(|value| !value.trim().is_empty());
                let is_open = self
                    .open
                    .as_ref()
                    .is_some_and(|open| open.kind == BlockKind::Thinking && open.item_key == key);
                if !is_open {
                    // 没有流式增量（或已经被别的块打断）：有内容才整块补发。
                    let text = reasoning_text(item);
                    if text.is_empty() && encrypted.is_none() {
                        self.completed_items.push(key.to_string());
                        return;
                    }
                    self.open_block(BlockKind::Thinking, key, None, output);
                    if !text.is_empty() {
                        let index = self.open.as_ref().map(|block| block.index).unwrap_or(0);
                        push_event(
                            output,
                            "content_block_delta",
                            json!({
                                "type": "content_block_delta",
                                "index": index,
                                "delta": { "type": "thinking_delta", "thinking": text }
                            }),
                        );
                    }
                }
                let index = self.open.as_ref().map(|block| block.index).unwrap_or(0);
                push_event(
                    output,
                    "content_block_delta",
                    json!({
                        "type": "content_block_delta",
                        "index": index,
                        "delta": { "type": "signature_delta", "signature": encode_signature(encrypted) }
                    }),
                );
                self.close_open(output);
            }
            Some("message") => {
                let is_open = self
                    .open
                    .as_ref()
                    .is_some_and(|open| open.kind == BlockKind::Text && open.item_key == key);
                if is_open {
                    self.close_open(output);
                } else {
                    let text = message_text(item);
                    if !text.is_empty() {
                        self.open_block(BlockKind::Text, key, None, output);
                        let index = self.open.as_ref().map(|block| block.index).unwrap_or(0);
                        push_event(
                            output,
                            "content_block_delta",
                            json!({
                                "type": "content_block_delta",
                                "index": index,
                                "delta": { "type": "text_delta", "text": text }
                            }),
                        );
                        self.close_open(output);
                    }
                }
            }
            Some("function_call") => {
                let arguments = item
                    .get("arguments")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let is_open = self
                    .open
                    .as_ref()
                    .is_some_and(|open| open.kind == BlockKind::Tool && open.item_key == key);
                if is_open {
                    let open = self.open.as_ref().expect("open block");
                    if !open.emitted_arguments && !arguments.is_empty() {
                        let index = open.index;
                        push_event(
                            output,
                            "content_block_delta",
                            json!({
                                "type": "content_block_delta",
                                "index": index,
                                "delta": { "type": "input_json_delta", "partial_json": arguments }
                            }),
                        );
                    }
                    self.close_open(output);
                } else {
                    let mut tool = self.pending_tools.remove(key).unwrap_or_default();
                    if tool.call_id.is_empty() {
                        tool.call_id = tool_call_id(item);
                    }
                    if tool.name.is_empty() {
                        tool.name = item
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                    }
                    if !arguments.is_empty() {
                        tool.arguments = arguments;
                    }
                    self.close_open(output);
                    self.start_tool_block(key, &tool, output);
                    if !tool.arguments.is_empty() {
                        let index = self.open.as_ref().map(|block| block.index).unwrap_or(0);
                        push_event(
                            output,
                            "content_block_delta",
                            json!({
                                "type": "content_block_delta",
                                "index": index,
                                "delta": { "type": "input_json_delta", "partial_json": tool.arguments }
                            }),
                        );
                    }
                    self.close_open(output);
                }
            }
            _ => {}
        }
        self.completed_items.push(key.to_string());
    }

    fn ensure_started(&mut self, output: &mut String) {
        if self.started {
            return;
        }
        self.started = true;
        push_event(
            output,
            "message_start",
            json!({
                "type": "message_start",
                "message": {
                    "id": self.message_id,
                    "type": "message",
                    "role": "assistant",
                    "model": self.model,
                    "content": [],
                    "stop_reason": null,
                    "stop_sequence": null,
                    "usage": {
                        "input_tokens": self.input_tokens_estimate,
                        "cache_creation_input_tokens": 0,
                        "cache_read_input_tokens": 0,
                        "output_tokens": 1
                    }
                }
            }),
        );
    }

    /// 让 `kind`/`key` 成为当前打开的块：已经是就不动，否则关掉旧块再开新块。
    fn open_block(
        &mut self,
        kind: BlockKind,
        key: &str,
        tool: Option<&PendingTool>,
        output: &mut String,
    ) {
        if self
            .open
            .as_ref()
            .is_some_and(|open| open.kind == kind && open.item_key == key)
        {
            return;
        }
        self.close_open(output);
        let index = self.next_index;
        self.next_index += 1;
        let content_block = match kind {
            BlockKind::Text => json!({ "type": "text", "text": "" }),
            BlockKind::Thinking => json!({ "type": "thinking", "thinking": "", "signature": "" }),
            BlockKind::Tool => {
                let tool = tool.cloned().unwrap_or_default();
                self.emitted_tool_use = true;
                json!({
                    "type": "tool_use",
                    "id": if tool.call_id.is_empty() { format!("toolu_{index}") } else { tool.call_id },
                    "name": tool.name,
                    "input": {}
                })
            }
        };
        push_event(
            output,
            "content_block_start",
            json!({
                "type": "content_block_start",
                "index": index,
                "content_block": content_block
            }),
        );
        self.open = Some(OpenBlock {
            index,
            kind,
            item_key: key.to_string(),
            emitted_arguments: false,
            summary_index: None,
        });
    }

    fn start_tool_block(&mut self, key: &str, tool: &PendingTool, output: &mut String) {
        self.open_block(BlockKind::Tool, key, Some(tool), output);
    }

    fn close_open(&mut self, output: &mut String) {
        if let Some(open) = self.open.take() {
            push_event(
                output,
                "content_block_stop",
                json!({ "type": "content_block_stop", "index": open.index }),
            );
        }
    }

    fn finish_message(&mut self, response: &Value, output: &mut String) {
        let usage = anthropic_usage(response.get("usage"));
        self.usage = Some(crate::request_log::RequestUsage {
            input_tokens: usage.input_tokens + usage.cache_read_input_tokens,
            output_tokens: usage.output_tokens,
            cached_tokens: usage.cache_read_input_tokens,
            reasoning_tokens: usage.reasoning_tokens,
        });
        let stop_reason = stop_reason(response, self.emitted_tool_use);
        push_event(
            output,
            "message_delta",
            json!({
                "type": "message_delta",
                "delta": { "stop_reason": stop_reason, "stop_sequence": null },
                "usage": {
                    "input_tokens": usage.input_tokens,
                    "cache_creation_input_tokens": 0,
                    "cache_read_input_tokens": usage.cache_read_input_tokens,
                    "output_tokens": usage.output_tokens
                }
            }),
        );
        push_event(output, "message_stop", json!({ "type": "message_stop" }));
        self.finished = true;
    }

    fn fail_into(&mut self, output: &mut String, error_type: &str, message: &str) {
        self.close_open(output);
        push_event(
            output,
            "error",
            json!({
                "type": "error",
                "error": { "type": error_type, "message": message }
            }),
        );
        self.finished = true;
        self.failed = true;
        self.failure = Some(message.to_string());
    }
}

struct AnthropicUsage {
    input_tokens: u64,
    cache_read_input_tokens: u64,
    output_tokens: u64,
    reasoning_tokens: u64,
}

/// Responses usage → Anthropic usage：Anthropic 的 `input_tokens` 不含缓存命中部分。
fn anthropic_usage(usage: Option<&Value>) -> AnthropicUsage {
    let number = |pointer: &str| {
        usage
            .and_then(|usage| usage.pointer(pointer))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    let total_input = number("/input_tokens");
    let cached = number("/input_tokens_details/cached_tokens");
    AnthropicUsage {
        input_tokens: total_input.saturating_sub(cached),
        cache_read_input_tokens: cached,
        output_tokens: number("/output_tokens"),
        reasoning_tokens: number("/output_tokens_details/reasoning_tokens"),
    }
}

fn stop_reason(response: &Value, has_tool_use: bool) -> &'static str {
    if has_tool_use {
        return "tool_use";
    }
    let incomplete_reason = response
        .pointer("/incomplete_details/reason")
        .and_then(Value::as_str);
    if response.get("status").and_then(Value::as_str) == Some("incomplete")
        && incomplete_reason.is_none_or(|reason| reason == "max_output_tokens")
    {
        return "max_tokens";
    }
    "end_turn"
}

fn item_key(event: &Value) -> String {
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

fn tool_call_id(item: &Value) -> String {
    item.get("call_id")
        .or_else(|| item.get("id"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn reasoning_text(item: &Value) -> String {
    let summary = item
        .get("summary")
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n\n")
        })
        .unwrap_or_default();
    if !summary.is_empty() {
        return summary;
    }
    item.get("content")
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

fn message_text(item: &Value) -> String {
    item.get("content")
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter(|part| {
                    matches!(
                        part.get("type").and_then(Value::as_str),
                        Some("output_text" | "refusal" | "text")
                    )
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

/// 非流式：Responses JSON → Anthropic message JSON。
pub fn responses_json_to_anthropic(response: &Value, model: &str) -> Value {
    let mut content = Vec::new();
    let mut has_tool_use = false;
    for item in response
        .get("output")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        match item.get("type").and_then(Value::as_str) {
            Some("reasoning") => {
                let text = reasoning_text(item);
                let encrypted = item
                    .get("encrypted_content")
                    .and_then(Value::as_str)
                    .filter(|value| !value.trim().is_empty());
                if text.is_empty() && encrypted.is_none() {
                    continue;
                }
                content.push(json!({
                    "type": "thinking",
                    "thinking": text,
                    "signature": encode_signature(encrypted)
                }));
            }
            Some("message") => {
                let text = message_text(item);
                if !text.is_empty() {
                    content.push(json!({ "type": "text", "text": text }));
                }
            }
            Some("function_call") => {
                has_tool_use = true;
                let arguments = item
                    .get("arguments")
                    .and_then(Value::as_str)
                    .unwrap_or("{}");
                let input = serde_json::from_str::<Value>(arguments)
                    .ok()
                    .filter(Value::is_object)
                    .unwrap_or_else(|| json!({}));
                content.push(json!({
                    "type": "tool_use",
                    "id": tool_call_id(item),
                    "name": item.get("name").and_then(Value::as_str).unwrap_or_default(),
                    "input": input
                }));
            }
            _ => {}
        }
    }
    let usage = anthropic_usage(response.get("usage"));
    json!({
        "id": format!("msg_{}", uuid::Uuid::new_v4().simple()),
        "type": "message",
        "role": "assistant",
        "model": model,
        "content": content,
        "stop_reason": stop_reason(response, has_tool_use),
        "stop_sequence": null,
        "usage": {
            "input_tokens": usage.input_tokens,
            "cache_creation_input_tokens": 0,
            "cache_read_input_tokens": usage.cache_read_input_tokens,
            "output_tokens": usage.output_tokens
        }
    })
}

/// 客户端要流式、上游却回了整段 JSON 时：把完整的 Anthropic message 拆成事件流。
pub fn anthropic_message_to_sse(message: &Value) -> Vec<u8> {
    let mut output = String::new();
    let mut start = message.clone();
    start["content"] = json!([]);
    start["stop_reason"] = Value::Null;
    push_event(
        &mut output,
        "message_start",
        json!({ "type": "message_start", "message": start }),
    );
    for (index, block) in message
        .get("content")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .enumerate()
    {
        let (empty, delta) = match block.get("type").and_then(Value::as_str) {
            Some("thinking") => (
                json!({ "type": "thinking", "thinking": "", "signature": "" }),
                vec![
                    json!({ "type": "thinking_delta", "thinking": block.get("thinking").cloned().unwrap_or(json!("")) }),
                    json!({ "type": "signature_delta", "signature": block.get("signature").cloned().unwrap_or(json!("")) }),
                ],
            ),
            Some("tool_use") => (
                json!({ "type": "tool_use", "id": block.get("id").cloned().unwrap_or(json!("")), "name": block.get("name").cloned().unwrap_or(json!("")), "input": {} }),
                vec![
                    json!({ "type": "input_json_delta", "partial_json": block.get("input").map(Value::to_string).unwrap_or_else(|| "{}".to_string()) }),
                ],
            ),
            _ => (
                json!({ "type": "text", "text": "" }),
                vec![
                    json!({ "type": "text_delta", "text": block.get("text").cloned().unwrap_or(json!("")) }),
                ],
            ),
        };
        push_event(
            &mut output,
            "content_block_start",
            json!({ "type": "content_block_start", "index": index, "content_block": empty }),
        );
        for delta in delta {
            push_event(
                &mut output,
                "content_block_delta",
                json!({ "type": "content_block_delta", "index": index, "delta": delta }),
            );
        }
        push_event(
            &mut output,
            "content_block_stop",
            json!({ "type": "content_block_stop", "index": index }),
        );
    }
    push_event(
        &mut output,
        "message_delta",
        json!({
            "type": "message_delta",
            "delta": { "stop_reason": message.get("stop_reason").cloned().unwrap_or(json!("end_turn")), "stop_sequence": null },
            "usage": message.get("usage").cloned().unwrap_or(json!({}))
        }),
    );
    push_event(
        &mut output,
        "message_stop",
        json!({ "type": "message_stop" }),
    );
    output.into_bytes()
}

/// HTTP 状态码 → Anthropic 错误类型。
pub fn anthropic_error_type(status: u16) -> &'static str {
    match status {
        400 | 422 => "invalid_request_error",
        401 => "authentication_error",
        402 => "billing_error",
        403 => "permission_error",
        404 => "not_found_error",
        413 => "request_too_large",
        429 => "rate_limit_error",
        503 | 529 => "overloaded_error",
        _ => "api_error",
    }
}

/// Anthropic 格式的错误体。
pub fn anthropic_error_body(status: u16, message: &str) -> Value {
    json!({
        "type": "error",
        "error": { "type": anthropic_error_type(status), "message": message }
    })
}

fn push_event(output: &mut String, event: &str, data: Value) {
    output.push_str("event: ");
    output.push_str(event);
    output.push_str("\ndata: ");
    output.push_str(&data.to_string());
    output.push_str("\n\n");
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

    fn parse(output: &[u8]) -> Vec<Value> {
        String::from_utf8(output.to_vec())
            .unwrap()
            .split("\n\n")
            .filter(|block| !block.trim().is_empty())
            .map(|block| {
                let data = block
                    .lines()
                    .find_map(|line| line.strip_prefix("data: "))
                    .unwrap();
                serde_json::from_str(data).unwrap()
            })
            .collect()
    }

    fn sample_stream() -> String {
        sse(&[
            json!({"type":"response.created","response":{"id":"resp_1"}}),
            json!({"type":"response.output_item.added","output_index":0,"item":{"id":"rs_1","type":"reasoning"}}),
            json!({"type":"response.reasoning_summary_text.delta","item_id":"rs_1","summary_index":0,"delta":"想一想"}),
            json!({"type":"response.reasoning_summary_text.delta","item_id":"rs_1","summary_index":1,"delta":"再想"}),
            json!({"type":"response.output_item.done","output_index":0,"item":{"id":"rs_1","type":"reasoning","encrypted_content":"enc-1","summary":[]}}),
            json!({"type":"response.output_item.added","output_index":1,"item":{"id":"msg_1","type":"message"}}),
            json!({"type":"response.output_text.delta","item_id":"msg_1","delta":"好的，"}),
            json!({"type":"response.output_text.delta","item_id":"msg_1","delta":"我来看看"}),
            json!({"type":"response.output_item.done","output_index":1,"item":{"id":"msg_1","type":"message","content":[{"type":"output_text","text":"好的，我来看看"}]}}),
            json!({"type":"response.output_item.added","output_index":2,"item":{"id":"fc_1","type":"function_call","call_id":"call_1","name":"Bash"}}),
            json!({"type":"response.function_call_arguments.delta","item_id":"fc_1","delta":"{\"command\":"}),
            json!({"type":"response.function_call_arguments.delta","item_id":"fc_1","delta":"\"ls\"}"}),
            json!({"type":"response.output_item.done","output_index":2,"item":{"id":"fc_1","type":"function_call","call_id":"call_1","name":"Bash","arguments":"{\"command\":\"ls\"}"}}),
            json!({"type":"response.completed","response":{"id":"resp_1","status":"completed","usage":{"input_tokens":1000,"input_tokens_details":{"cached_tokens":800},"output_tokens":50}}}),
        ]) + "data: [DONE]\n\n"
    }

    #[test]
    fn full_stream_translates_to_ordered_anthropic_events() {
        let mut converter = ResponsesSseToAnthropicSse::new("claude-sonnet-5", 42);
        let mut output = converter.push_bytes(sample_stream().as_bytes());
        output.extend(converter.finish());
        let events = parse(&output);
        let kinds = events
            .iter()
            .map(|event| event["type"].as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            vec![
                "message_start",
                "content_block_start",
                "content_block_delta",
                "content_block_delta",
                "content_block_delta",
                "content_block_stop",
                "content_block_start",
                "content_block_delta",
                "content_block_delta",
                "content_block_stop",
                "content_block_start",
                "content_block_delta",
                "content_block_delta",
                "content_block_stop",
                "message_delta",
                "message_stop"
            ]
        );
        assert_eq!(events[0]["message"]["model"], "claude-sonnet-5");
        assert_eq!(events[0]["message"]["usage"]["input_tokens"], 42);
        assert_eq!(events[1]["content_block"]["type"], "thinking");
        assert_eq!(events[3]["delta"]["thinking"], "\n\n再想");
        let signature = events[4]["delta"]["signature"].as_str().unwrap();
        assert_eq!(
            crate::gateway::anthropic_inbound::decode_signature(signature),
            Some(Some("enc-1".to_string()))
        );
        assert_eq!(events[10]["content_block"]["type"], "tool_use");
        assert_eq!(events[10]["content_block"]["id"], "call_1");
        assert_eq!(events[10]["content_block"]["name"], "Bash");
        assert_eq!(events[10]["index"], 2);
        assert_eq!(events[14]["delta"]["stop_reason"], "tool_use");
        assert_eq!(events[14]["usage"]["input_tokens"], 200);
        assert_eq!(events[14]["usage"]["cache_read_input_tokens"], 800);
        assert_eq!(events[14]["usage"]["output_tokens"], 50);
        assert!(converter.is_finished());
        assert!(!converter.has_failed());
        let usage = converter.usage().unwrap();
        assert_eq!(usage.input_tokens, 1000);
        assert_eq!(usage.cached_tokens, 800);
    }

    #[test]
    fn byte_by_byte_feeding_matches_single_chunk() {
        let stream = sample_stream();
        let mut whole = ResponsesSseToAnthropicSse::new("m", 0);
        let mut expected = whole.push_bytes(stream.as_bytes());
        expected.extend(whole.finish());
        let mut split = ResponsesSseToAnthropicSse::new("m", 0);
        let mut actual = Vec::new();
        for byte in stream.as_bytes().chunks(1) {
            actual.extend(split.push_bytes(byte));
        }
        actual.extend(split.finish());
        let strip_ids = |events: Vec<Value>| {
            events
                .into_iter()
                .map(|mut event| {
                    if let Some(message) = event.get_mut("message") {
                        message["id"] = json!("x");
                    }
                    event
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(strip_ids(parse(&actual)), strip_ids(parse(&expected)));
    }

    #[test]
    fn interleaved_parallel_tool_calls_are_serialized() {
        let stream = sse(&[
            json!({"type":"response.output_item.added","output_index":0,"item":{"id":"fc_a","type":"function_call","call_id":"call_a","name":"Read"}}),
            json!({"type":"response.output_item.added","output_index":1,"item":{"id":"fc_b","type":"function_call","call_id":"call_b","name":"Grep"}}),
            json!({"type":"response.function_call_arguments.delta","item_id":"fc_b","delta":"{\"p\":1}"}),
            json!({"type":"response.function_call_arguments.delta","item_id":"fc_a","delta":"{\"f\":2}"}),
            json!({"type":"response.output_item.done","output_index":0,"item":{"id":"fc_a","type":"function_call","call_id":"call_a","name":"Read","arguments":"{\"f\":2}"}}),
            json!({"type":"response.output_item.done","output_index":1,"item":{"id":"fc_b","type":"function_call","call_id":"call_b","name":"Grep","arguments":"{\"p\":1}"}}),
            json!({"type":"response.completed","response":{"status":"completed"}}),
        ]);
        let mut converter = ResponsesSseToAnthropicSse::new("m", 0);
        let events = parse(&converter.push_bytes(stream.as_bytes()));
        let summary = events
            .iter()
            .filter(|event| event["type"] != "message_start")
            .map(|event| match event["type"].as_str().unwrap() {
                "content_block_start" => {
                    format!("start:{}", event["content_block"]["id"].as_str().unwrap())
                }
                "content_block_delta" => {
                    format!("delta:{}", event["delta"]["partial_json"].as_str().unwrap())
                }
                other => other.to_string(),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            summary,
            vec![
                "start:call_a",
                "delta:{\"f\":2}",
                "content_block_stop",
                "start:call_b",
                "delta:{\"p\":1}",
                "content_block_stop",
                "message_delta",
                "message_stop"
            ]
        );
    }

    #[test]
    fn failed_and_truncated_streams_emit_error_events() {
        let failed = sse(&[
            json!({"type":"response.output_text.delta","item_id":"m","delta":"半句"}),
            json!({"type":"response.failed","response":{"error":{"code":"server_is_overloaded","message":"busy"}}}),
        ]);
        let mut converter = ResponsesSseToAnthropicSse::new("m", 0);
        let events = parse(&converter.push_bytes(failed.as_bytes()));
        let last = events.last().unwrap();
        assert_eq!(last["type"], "error");
        assert_eq!(last["error"]["type"], "overloaded_error");
        assert_eq!(events[events.len() - 2]["type"], "content_block_stop");
        assert!(converter.has_failed());

        let mut truncated = ResponsesSseToAnthropicSse::new("m", 0);
        truncated.push_bytes(
            sse(&[json!({"type":"response.output_text.delta","item_id":"m","delta":"x"})])
                .as_bytes(),
        );
        let events = parse(&truncated.finish());
        assert_eq!(events.last().unwrap()["type"], "error");
    }

    #[test]
    fn incomplete_maps_to_max_tokens() {
        let stream = sse(&[
            json!({"type":"response.output_text.delta","item_id":"m","delta":"x"}),
            json!({"type":"response.incomplete","response":{"status":"incomplete","incomplete_details":{"reason":"max_output_tokens"}}}),
        ]);
        let mut converter = ResponsesSseToAnthropicSse::new("m", 0);
        let events = parse(&converter.push_bytes(stream.as_bytes()));
        let delta = events
            .iter()
            .find(|event| event["type"] == "message_delta")
            .unwrap();
        assert_eq!(delta["delta"]["stop_reason"], "max_tokens");
    }

    #[test]
    fn completed_only_output_is_backfilled() {
        let stream = sse(&[json!({"type":"response.completed","response":{
            "status":"completed",
            "output":[{"id":"msg_x","type":"message","content":[{"type":"output_text","text":"整段"}]}]
        }})]);
        let mut converter = ResponsesSseToAnthropicSse::new("m", 0);
        let events = parse(&converter.push_bytes(stream.as_bytes()));
        let text = events
            .iter()
            .find(|event| event["delta"]["type"] == "text_delta")
            .unwrap();
        assert_eq!(text["delta"]["text"], "整段");
        assert_eq!(
            events
                .iter()
                .find(|event| event["type"] == "message_delta")
                .unwrap()["delta"]["stop_reason"],
            "end_turn"
        );
    }

    #[test]
    fn non_stream_json_translation() {
        let response = json!({
            "status": "completed",
            "output": [
                {"type":"reasoning","summary":[{"type":"summary_text","text":"想"}],"encrypted_content":"e"},
                {"type":"message","content":[{"type":"output_text","text":"答"}]},
                {"type":"function_call","call_id":"call_9","name":"Read","arguments":"{\"file\":\"a\"}"}
            ],
            "usage": {"input_tokens": 10, "input_tokens_details": {"cached_tokens": 4}, "output_tokens": 3}
        });
        let message = responses_json_to_anthropic(&response, "claude-x");
        assert_eq!(message["model"], "claude-x");
        assert_eq!(message["content"][0]["type"], "thinking");
        assert_eq!(message["content"][1]["text"], "答");
        assert_eq!(message["content"][2]["input"]["file"], "a");
        assert_eq!(message["stop_reason"], "tool_use");
        assert_eq!(message["usage"]["input_tokens"], 6);
        assert_eq!(message["usage"]["cache_read_input_tokens"], 4);
    }

    #[test]
    fn whole_message_replays_as_event_stream() {
        let message = responses_json_to_anthropic(
            &json!({
                "status": "completed",
                "output": [
                    {"type":"message","content":[{"type":"output_text","text":"hi"}]},
                    {"type":"function_call","call_id":"c1","name":"Read","arguments":"{\"f\":1}"}
                ]
            }),
            "m",
        );
        let events = parse(&anthropic_message_to_sse(&message));
        let kinds = events
            .iter()
            .map(|event| event["type"].as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        assert_eq!(kinds.first().map(String::as_str), Some("message_start"));
        assert_eq!(kinds.last().map(String::as_str), Some("message_stop"));
        assert_eq!(events[2]["delta"]["text"], "hi");
        assert_eq!(events[5]["delta"]["partial_json"], "{\"f\":1}");
        assert_eq!(events[7]["delta"]["stop_reason"], "tool_use");
    }

    #[test]
    fn error_type_mapping() {
        assert_eq!(anthropic_error_type(429), "rate_limit_error");
        assert_eq!(anthropic_error_type(529), "overloaded_error");
        assert_eq!(anthropic_error_type(401), "authentication_error");
        assert_eq!(anthropic_error_type(502), "api_error");
        assert_eq!(
            anthropic_error_body(404, "x")["error"]["type"],
            "not_found_error"
        );
    }
}
