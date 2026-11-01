//! Responses 响应（JSON / SSE）→ Gemini GenerateContentResponse。
//!
//! 流式按 `alt=sse` 的格式输出：每个事件一行 `data: {GenerateContentResponse}`，以 `\r\n\r\n` 结尾
//! （与 Gemini 官方一致，Gemini CLI 按这个切分）。
//!
//! - 文字增量 → `parts: [{"text": …}]`；推理增量 → `parts: [{"text": …, "thought": true}]`；
//! - 函数调用的参数要整段解析成对象才能放进 `functionCall.args`，所以参数先缓冲，
//!   该项完成（或终止事件补齐）时才发，绝不发半截 JSON；
//! - `functionCall` 总是带 `id`（Responses 的 call_id），客户端带回 `functionResponse.id` 后
//!   [`super::gemini_inbound`] 能原样配对；
//! - 最后一块带 `finishReason` 与 `usageMetadata`。

use std::collections::{HashMap, HashSet};

use serde_json::{Map, Value, json};

use crate::gateway::chat_outbound::{
    ResponsesSseReader, ResponsesUsage, incomplete_reason, item_key, message_text, output_item_key,
    reasoning_text, responses_usage, tool_call_id, upstream_failure,
};
use crate::request_log::RequestUsage;

#[derive(Debug, Clone, Default)]
struct PendingCall {
    call_id: String,
    name: String,
    arguments: String,
}

/// Responses SSE → Gemini SSE 的增量转换器。
pub struct ResponsesSseToGeminiSse {
    reader: ResponsesSseReader,
    model: String,
    response_id: String,
    finished: bool,
    failed: bool,
    /// 还没发出的函数调用（键是 Responses item 标识），参数在这里缓冲。
    pending_calls: HashMap<String, PendingCall>,
    /// 出现顺序，终止时按这个顺序补发没收到完成事件的调用。
    call_order: Vec<String>,
    /// 已经流式输出过文字 / 推理的 item，完成时不再整段补发。
    streamed_items: HashSet<String>,
    /// 已经处理过完成事件的 item，防止重复。
    completed_items: HashSet<String>,
    /// 各推理 item 当前所在的摘要段（换段时补两个换行）。
    summary_index: HashMap<String, u64>,
    usage: Option<RequestUsage>,
    failure: Option<String>,
}

impl ResponsesSseToGeminiSse {
    /// `model` 是回给客户端的模型名（填进 `modelVersion`）。
    pub fn new(model: &str) -> Self {
        Self {
            reader: ResponsesSseReader::default(),
            model: model.to_string(),
            response_id: new_response_id(),
            finished: false,
            failed: false,
            pending_calls: HashMap::new(),
            call_order: Vec::new(),
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

    /// 流结束：没见到终止事件就补一个错误事件，客户端才知道这一轮没完成。
    pub fn finish(&mut self) -> Vec<u8> {
        let mut output = String::new();
        for (kind, event) in self.reader.drain() {
            self.handle_event(&kind, &event, &mut output);
        }
        if !self.finished {
            self.fail_into(&mut output, 500, "上游在完成事件前结束了响应流");
        }
        output.into_bytes()
    }

    /// 上游读取出错时调用。
    pub fn fail(&mut self, message: &str) -> Vec<u8> {
        let mut output = String::new();
        if !self.finished {
            self.fail_into(&mut output, 500, message);
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
                self.push_parts(output, vec![json!({ "text": delta })]);
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
                self.push_parts(output, vec![json!({ "text": text, "thought": true })]);
            }
            "response.output_item.added" => {
                let Some(item) = event.get("item") else {
                    return;
                };
                if item.get("type").and_then(Value::as_str) != Some("function_call") {
                    return;
                }
                let key = item_key(event);
                if self.completed_items.contains(&key) {
                    return;
                }
                let call = self.pending_call(&key);
                if call.call_id.is_empty() {
                    call.call_id = tool_call_id(item);
                }
                if call.name.is_empty() {
                    call.name = str_field(item, "name");
                }
            }
            "response.function_call_arguments.delta" => {
                let delta = event
                    .get("delta")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let key = item_key(event);
                if delta.is_empty() || self.completed_items.contains(&key) {
                    return;
                }
                self.pending_call(&key).arguments.push_str(delta);
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
                // 只见过增量、没见过完成事件的函数调用：用缓冲的参数收尾。
                for key in self.call_order.clone() {
                    self.item_done(&key, &json!({ "type": "function_call" }), output);
                }
                self.finish_message(&response, output);
            }
            "response.failed" | "error" => {
                let (message, code) = upstream_failure(event);
                let status = if code.contains("overload") || code.contains("capacity") {
                    503
                } else if code.contains("rate_limit") || code.contains("quota") {
                    429
                } else if code.contains("invalid") {
                    400
                } else {
                    500
                };
                self.fail_into(output, status, &message);
            }
            _ => {}
        }
    }

    fn pending_call(&mut self, key: &str) -> &mut PendingCall {
        if !self.pending_calls.contains_key(key) {
            self.call_order.push(key.to_string());
        }
        self.pending_calls.entry(key.to_string()).or_default()
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
                        self.push_parts(output, vec![json!({ "text": text })]);
                    }
                }
            }
            Some("reasoning") => {
                if !self.streamed_items.contains(key) {
                    let text = reasoning_text(item);
                    if !text.is_empty() {
                        self.push_parts(output, vec![json!({ "text": text, "thought": true })]);
                    }
                }
            }
            Some("function_call") => {
                let mut call = self.pending_calls.remove(key).unwrap_or_default();
                if call.call_id.is_empty() {
                    call.call_id = tool_call_id(item);
                }
                if call.name.is_empty() {
                    call.name = str_field(item, "name");
                }
                // 完成事件里的完整参数为准，没有时用缓冲的增量。
                if let Some(arguments) = item
                    .get("arguments")
                    .and_then(Value::as_str)
                    .filter(|arguments| !arguments.is_empty())
                {
                    call.arguments = arguments.to_string();
                }
                self.push_parts(output, vec![function_call_part(&call)]);
            }
            _ => {}
        }
        self.completed_items.insert(key.to_string());
    }

    fn push_parts(&self, output: &mut String, parts: Vec<Value>) {
        let chunk = json!({
            "candidates": [{
                "content": { "role": "model", "parts": parts },
                "index": 0
            }],
            "modelVersion": self.model,
            "responseId": self.response_id
        });
        push_data(output, &chunk);
    }

    fn finish_message(&mut self, response: &Value, output: &mut String) {
        let usage = responses_usage(response.get("usage"));
        self.usage = Some(usage.request_usage());
        // 最后一块不放内容（空 parts 的块 Gemini CLI 只读 finishReason 与 usage，不会当成内容）。
        let chunk = json!({
            "candidates": [{
                "content": { "role": "model", "parts": [] },
                "finishReason": finish_reason(response),
                "index": 0
            }],
            "usageMetadata": usage_metadata(&usage),
            "modelVersion": self.model,
            "responseId": self.response_id
        });
        push_data(output, &chunk);
        self.finished = true;
    }

    fn fail_into(&mut self, output: &mut String, status: u16, message: &str) {
        push_data(output, &gemini_error_body(status, message));
        self.finished = true;
        self.failed = true;
        self.failure = Some(message.to_string());
    }
}

/// 函数调用 → Gemini 的 functionCall 部件；参数解析不成对象（截断等）时给空对象。
fn function_call_part(call: &PendingCall) -> Value {
    let args = serde_json::from_str::<Value>(&call.arguments)
        .ok()
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    let mut function_call = Map::new();
    if !call.call_id.is_empty() {
        function_call.insert("id".to_string(), json!(call.call_id));
    }
    function_call.insert("name".to_string(), json!(call.name));
    function_call.insert("args".to_string(), args);
    json!({ "functionCall": Value::Object(function_call) })
}

fn finish_reason(response: &Value) -> &'static str {
    match incomplete_reason(response) {
        Some("content_filter") => "SAFETY",
        Some(_) => "MAX_TOKENS",
        None => "STOP",
    }
}

/// Responses usage → Gemini usageMetadata：`candidatesTokenCount` 不含思考，思考单列在 `thoughtsTokenCount`。
fn usage_metadata(usage: &ResponsesUsage) -> Value {
    json!({
        "promptTokenCount": usage.input_tokens,
        "candidatesTokenCount": usage.output_tokens.saturating_sub(usage.reasoning_tokens),
        "totalTokenCount": usage.total_tokens,
        "cachedContentTokenCount": usage.cached_tokens,
        "thoughtsTokenCount": usage.reasoning_tokens
    })
}

/// 非流式：Responses JSON → Gemini GenerateContentResponse。
pub fn responses_json_to_gemini(response: &Value, model: &str) -> Value {
    let mut parts = Vec::new();
    for item in response
        .get("output")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        match item.get("type").and_then(Value::as_str) {
            Some("reasoning") => {
                let text = reasoning_text(item);
                if !text.is_empty() {
                    parts.push(json!({ "text": text, "thought": true }));
                }
            }
            Some("message") => {
                let text = message_text(item);
                if !text.is_empty() {
                    parts.push(json!({ "text": text }));
                }
            }
            Some("function_call") => {
                let arguments = match item.get("arguments") {
                    Some(Value::String(arguments)) => arguments.clone(),
                    Some(Value::Null) | None => String::new(),
                    Some(other) => other.to_string(),
                };
                parts.push(function_call_part(&PendingCall {
                    call_id: tool_call_id(item),
                    name: str_field(item, "name"),
                    arguments,
                }));
            }
            _ => {}
        }
    }
    let usage = responses_usage(response.get("usage"));
    json!({
        "candidates": [{
            "content": { "role": "model", "parts": parts },
            "finishReason": finish_reason(response),
            "index": 0
        }],
        "usageMetadata": usage_metadata(&usage),
        "modelVersion": model,
        "responseId": response
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(new_response_id)
    })
}

/// 客户端要流式、上游却回了整段 JSON 时：整段响应作为一个 SSE 事件发出（Gemini 允许一块带完整内容与结束原因）。
pub fn gemini_response_to_sse(response: &Value) -> Vec<u8> {
    let mut output = String::new();
    push_data(&mut output, response);
    output.into_bytes()
}

/// HTTP 状态码 → Google RPC 状态名。
fn gemini_status(status: u16) -> &'static str {
    match status {
        400 | 413 | 422 => "INVALID_ARGUMENT",
        401 => "UNAUTHENTICATED",
        403 => "PERMISSION_DENIED",
        404 => "NOT_FOUND",
        409 => "ABORTED",
        429 => "RESOURCE_EXHAUSTED",
        499 => "CANCELLED",
        501 => "UNIMPLEMENTED",
        503 | 529 => "UNAVAILABLE",
        504 => "DEADLINE_EXCEEDED",
        400..=499 => "FAILED_PRECONDITION",
        500..=599 => "INTERNAL",
        _ => "UNKNOWN",
    }
}

/// Google 风格的错误体。
pub fn gemini_error_body(status: u16, message: &str) -> Value {
    json!({
        "error": { "code": status, "message": message, "status": gemini_status(status) }
    })
}

fn push_data(output: &mut String, data: &Value) {
    output.push_str("data: ");
    output.push_str(&data.to_string());
    output.push_str("\r\n\r\n");
}

fn new_response_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

fn str_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
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
        let text = String::from_utf8(output.to_vec()).unwrap();
        assert!(
            text.is_empty() || text.ends_with("\r\n\r\n"),
            "每个事件以 \\r\\n\\r\\n 结尾：{text:?}"
        );
        text.split("\r\n\r\n")
            .filter(|block| !block.trim().is_empty())
            .map(|block| {
                let data = block.strip_prefix("data: ").expect("每块都是 data 行");
                serde_json::from_str(data).unwrap()
            })
            .collect()
    }

    fn all_parts(chunks: &[Value]) -> Vec<Value> {
        chunks
            .iter()
            .filter_map(|chunk| chunk.pointer("/candidates/0/content/parts"))
            .filter_map(Value::as_array)
            .flatten()
            .cloned()
            .collect()
    }

    fn joined_text(chunks: &[Value], thought: bool) -> String {
        all_parts(chunks)
            .iter()
            .filter(|part| (part.get("thought").and_then(Value::as_bool) == Some(true)) == thought)
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect()
    }

    fn function_calls(chunks: &[Value]) -> Vec<Value> {
        all_parts(chunks)
            .into_iter()
            .filter_map(|part| part.get("functionCall").cloned())
            .collect()
    }

    fn sample_stream() -> String {
        sse(&[
            json!({"type":"response.created","response":{"id":"resp_1"}}),
            json!({"type":"response.output_item.added","output_index":0,"item":{"id":"rs_1","type":"reasoning"}}),
            json!({"type":"response.reasoning_summary_text.delta","item_id":"rs_1","summary_index":0,"delta":"想一想"}),
            json!({"type":"response.reasoning_summary_text.delta","item_id":"rs_1","summary_index":1,"delta":"再想"}),
            json!({"type":"response.output_item.done","output_index":0,"item":{"id":"rs_1","type":"reasoning","summary":[]}}),
            json!({"type":"response.output_item.added","output_index":1,"item":{"id":"msg_1","type":"message"}}),
            json!({"type":"response.output_text.delta","item_id":"msg_1","delta":"好的，"}),
            json!({"type":"response.output_text.delta","item_id":"msg_1","delta":"我来看看"}),
            json!({"type":"response.output_item.done","output_index":1,"item":{"id":"msg_1","type":"message","content":[{"type":"output_text","text":"好的，我来看看"}]}}),
            json!({"type":"response.output_item.added","output_index":2,"item":{"id":"fc_1","type":"function_call","call_id":"call_1","name":"run_shell_command"}}),
            json!({"type":"response.function_call_arguments.delta","item_id":"fc_1","delta":"{\"command\":"}),
            json!({"type":"response.function_call_arguments.delta","item_id":"fc_1","delta":"\"ls\"}"}),
            json!({"type":"response.output_item.done","output_index":2,"item":{"id":"fc_1","type":"function_call","call_id":"call_1","name":"run_shell_command","arguments":"{\"command\":\"ls\"}"}}),
            json!({"type":"response.completed","response":{"id":"resp_1","status":"completed","usage":{"input_tokens":1000,"input_tokens_details":{"cached_tokens":800},"output_tokens":50,"output_tokens_details":{"reasoning_tokens":20}}}}),
        ]) + "data: [DONE]\n\n"
    }

    #[test]
    fn full_stream_translates_to_gemini_chunks() {
        let mut converter = ResponsesSseToGeminiSse::new("gemini-2.5-pro");
        let mut output = converter.push_bytes(sample_stream().as_bytes());
        output.extend(converter.finish());
        let chunks = parse(&output);
        assert!(
            chunks
                .iter()
                .all(|chunk| chunk["modelVersion"] == "gemini-2.5-pro")
        );
        assert!(
            chunks
                .iter()
                .all(|chunk| chunk["responseId"] == chunks[0]["responseId"])
        );
        assert_eq!(chunks[0]["candidates"][0]["content"]["role"], "model");
        assert_eq!(joined_text(&chunks, true), "想一想\n\n再想");
        assert_eq!(joined_text(&chunks, false), "好的，我来看看");
        // 函数调用只在完成时整段发一次，参数是完整对象。
        assert_eq!(
            function_calls(&chunks),
            vec![json!({"id":"call_1","name":"run_shell_command","args":{"command":"ls"}})]
        );
        let last = chunks.last().unwrap();
        assert_eq!(last["candidates"][0]["finishReason"], "STOP");
        assert_eq!(last["candidates"][0]["content"]["parts"], json!([]));
        assert_eq!(
            last["usageMetadata"],
            json!({
                "promptTokenCount": 1000,
                "candidatesTokenCount": 30,
                "totalTokenCount": 1050,
                "cachedContentTokenCount": 800,
                "thoughtsTokenCount": 20
            })
        );
        assert!(
            chunks[..chunks.len() - 1]
                .iter()
                .all(|chunk| chunk["candidates"][0].get("finishReason").is_none())
        );
        assert!(converter.is_finished());
        assert!(!converter.has_failed());
        let usage = converter.usage().unwrap();
        assert_eq!(usage.input_tokens, 1000);
        assert_eq!(usage.cached_tokens, 800);
        assert_eq!(usage.reasoning_tokens, 20);
    }

    #[test]
    fn byte_by_byte_feeding_matches_single_chunk() {
        let stream = sample_stream();
        let mut whole = ResponsesSseToGeminiSse::new("m");
        let mut expected = whole.push_bytes(stream.as_bytes());
        expected.extend(whole.finish());
        let mut split = ResponsesSseToGeminiSse::new("m");
        let mut actual = Vec::new();
        for byte in stream.as_bytes().chunks(1) {
            actual.extend(split.push_bytes(byte));
        }
        actual.extend(split.finish());
        let normalize = |chunks: Vec<Value>| {
            chunks
                .into_iter()
                .map(|mut chunk| {
                    chunk["responseId"] = json!("x");
                    chunk
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(normalize(parse(&actual)), normalize(parse(&expected)));
    }

    #[test]
    fn multibyte_character_split_across_chunks() {
        let stream = sse(&[
            json!({"type":"response.output_text.delta","item_id":"m","delta":"你好世界"}),
            json!({"type":"response.completed","response":{"status":"completed"}}),
        ]);
        let bytes = stream.as_bytes();
        let cut = stream.find("世").unwrap() + 2;
        let mut converter = ResponsesSseToGeminiSse::new("m");
        let mut output = converter.push_bytes(&bytes[..cut]);
        output.extend(converter.push_bytes(&bytes[cut..]));
        assert_eq!(joined_text(&parse(&output), false), "你好世界");
    }

    #[test]
    fn parallel_calls_wait_for_complete_arguments() {
        let stream = sse(&[
            json!({"type":"response.output_item.added","output_index":0,"item":{"id":"fc_a","type":"function_call","call_id":"call_a","name":"read_file"}}),
            json!({"type":"response.output_item.added","output_index":1,"item":{"id":"fc_b","type":"function_call","call_id":"call_b","name":"glob"}}),
            json!({"type":"response.function_call_arguments.delta","item_id":"fc_a","delta":"{\"path\":"}),
            json!({"type":"response.function_call_arguments.delta","item_id":"fc_b","delta":"{\"pattern\":\"*.rs\"}"}),
        ]);
        let mut converter = ResponsesSseToGeminiSse::new("m");
        // 参数还没完整：一个 functionCall 都不能发。
        assert!(function_calls(&parse(&converter.push_bytes(stream.as_bytes()))).is_empty());
        let rest = sse(&[
            json!({"type":"response.output_item.done","output_index":1,"item":{"id":"fc_b","type":"function_call","call_id":"call_b","name":"glob","arguments":"{\"pattern\":\"*.rs\"}"}}),
            json!({"type":"response.function_call_arguments.delta","item_id":"fc_a","delta":"\"a.rs\"}"}),
            // fc_a 没有完成事件：终止时用缓冲的参数收尾。
            json!({"type":"response.completed","response":{"status":"completed"}}),
            // 已结束后的重复事件被忽略。
            json!({"type":"response.output_item.done","output_index":1,"item":{"id":"fc_b","type":"function_call","call_id":"call_b","name":"glob","arguments":"{\"pattern\":\"*.rs\"}"}}),
        ]);
        let chunks = parse(&converter.push_bytes(rest.as_bytes()));
        assert_eq!(
            function_calls(&chunks),
            vec![
                json!({"id":"call_b","name":"glob","args":{"pattern":"*.rs"}}),
                json!({"id":"call_a","name":"read_file","args":{"path":"a.rs"}}),
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
                    {"id":"msg_2","type":"message","content":[{"type":"output_text","text":"整段"}]},
                    {"id":"fc_9","type":"function_call","call_id":"call_9","name":"ls","arguments":"{}"}
                ]
            }}),
        ]);
        let mut converter = ResponsesSseToGeminiSse::new("m");
        let chunks = parse(&converter.push_bytes(stream.as_bytes()));
        assert_eq!(joined_text(&chunks, false), "流式整段");
        assert_eq!(joined_text(&chunks, true), "补发推理");
        assert_eq!(
            function_calls(&chunks),
            vec![json!({"id":"call_9","name":"ls","args":{}})]
        );
    }

    #[test]
    fn incomplete_maps_to_max_tokens() {
        let stream = sse(&[
            json!({"type":"response.output_text.delta","item_id":"m","delta":"x"}),
            json!({"type":"response.incomplete","response":{"status":"incomplete","incomplete_details":{"reason":"max_output_tokens"}}}),
        ]);
        let mut converter = ResponsesSseToGeminiSse::new("m");
        let chunks = parse(&converter.push_bytes(stream.as_bytes()));
        assert_eq!(
            chunks.last().unwrap()["candidates"][0]["finishReason"],
            "MAX_TOKENS"
        );
    }

    #[test]
    fn failed_and_truncated_streams_emit_error_events() {
        let failed = sse(&[
            json!({"type":"response.output_text.delta","item_id":"m","delta":"半句"}),
            json!({"type":"response.failed","response":{"error":{"code":"server_is_overloaded","message":"busy"}}}),
        ]);
        let mut converter = ResponsesSseToGeminiSse::new("m");
        let chunks = parse(&converter.push_bytes(failed.as_bytes()));
        let error = &chunks.last().unwrap()["error"];
        assert_eq!(error["code"], 503);
        assert_eq!(error["status"], "UNAVAILABLE");
        assert_eq!(error["message"], "busy");
        assert!(converter.has_failed());
        assert_eq!(converter.failure(), Some("busy"));
        assert!(converter.finish().is_empty());

        let mut truncated = ResponsesSseToGeminiSse::new("m");
        truncated.push_bytes(
            sse(&[json!({"type":"response.output_text.delta","item_id":"m","delta":"x"})])
                .as_bytes(),
        );
        let chunks = parse(&truncated.finish());
        assert_eq!(chunks.last().unwrap()["error"]["code"], 500);
        assert!(truncated.has_failed());

        let mut broken = ResponsesSseToGeminiSse::new("m");
        let chunks = parse(&broken.fail("连接断开"));
        assert_eq!(chunks[0]["error"]["message"], "连接断开");
    }

    #[test]
    fn non_stream_json_translation() {
        let response = json!({
            "id": "resp_7",
            "status": "completed",
            "output": [
                {"type":"reasoning","summary":[{"type":"summary_text","text":"想"}]},
                {"type":"message","content":[{"type":"output_text","text":"答"}]},
                {"type":"function_call","call_id":"call_9","name":"read_file","arguments":"{\"path\":\"a\"}"}
            ],
            "usage": {"input_tokens": 10, "input_tokens_details": {"cached_tokens": 4}, "output_tokens": 3}
        });
        let converted = responses_json_to_gemini(&response, "gemini-x");
        assert_eq!(converted["modelVersion"], "gemini-x");
        assert_eq!(converted["responseId"], "resp_7");
        let candidate = &converted["candidates"][0];
        assert_eq!(candidate["finishReason"], "STOP");
        assert_eq!(
            candidate["content"]["parts"],
            json!([
                {"text":"想","thought":true},
                {"text":"答"},
                {"functionCall":{"id":"call_9","name":"read_file","args":{"path":"a"}}}
            ])
        );
        assert_eq!(converted["usageMetadata"]["promptTokenCount"], 10);
        assert_eq!(converted["usageMetadata"]["candidatesTokenCount"], 3);
        assert_eq!(converted["usageMetadata"]["totalTokenCount"], 13);
        assert_eq!(converted["usageMetadata"]["cachedContentTokenCount"], 4);

        // 截断的参数不发半截 JSON，退化为空对象。
        let truncated = responses_json_to_gemini(
            &json!({"status":"incomplete","output":[{"type":"function_call","call_id":"c","name":"n","arguments":"{\"pa"}]}),
            "m",
        );
        assert_eq!(truncated["candidates"][0]["finishReason"], "MAX_TOKENS");
        assert_eq!(
            truncated["candidates"][0]["content"]["parts"][0]["functionCall"]["args"],
            json!({})
        );
    }

    #[test]
    fn whole_response_replays_as_single_event() {
        let response = responses_json_to_gemini(
            &json!({"status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"hi"}]}]}),
            "m",
        );
        let chunks = parse(&gemini_response_to_sse(&response));
        assert_eq!(chunks, vec![response]);
    }

    #[test]
    fn error_body_shape() {
        assert_eq!(
            gemini_error_body(429, "slow down"),
            json!({"error":{"code":429,"message":"slow down","status":"RESOURCE_EXHAUSTED"}})
        );
        assert_eq!(
            gemini_error_body(400, "x")["error"]["status"],
            "INVALID_ARGUMENT"
        );
        assert_eq!(gemini_error_body(404, "x")["error"]["status"], "NOT_FOUND");
        assert_eq!(gemini_error_body(502, "x")["error"]["status"], "INTERNAL");
        assert_eq!(
            gemini_error_body(418, "x")["error"]["status"],
            "FAILED_PRECONDITION"
        );
    }
}
