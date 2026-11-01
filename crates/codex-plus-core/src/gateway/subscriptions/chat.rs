//! 订阅实现共用的 Chat Completions 工具：拼 Chat SSE、把 SSE 拼回完整答复、读上游的 SSE 行。
//!
//! 各家订阅把自己的事件转成 [`ChatSse`] 的几个调用（文本、推理、工具调用、结束），
//! 不用各自拼 `chat.completion.chunk` 的 JSON。

use std::collections::HashMap;

use futures_util::StreamExt;
use serde_json::{Value, json};

use crate::gateway::subscriptions::ChatStream;

/// 用量（Chat 格式的 usage）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cached_tokens: u64,
    pub reasoning_tokens: u64,
}

impl Usage {
    fn to_json(self) -> Value {
        json!({
            "prompt_tokens": self.prompt_tokens,
            "completion_tokens": self.completion_tokens,
            "total_tokens": self.prompt_tokens + self.completion_tokens,
            "prompt_tokens_details": { "cached_tokens": self.cached_tokens },
            "completion_tokens_details": { "reasoning_tokens": self.reasoning_tokens },
        })
    }
}

/// 逐步拼出 Chat Completions SSE。
#[derive(Debug)]
pub struct ChatSse {
    id: String,
    model: String,
    created: u64,
    role_sent: bool,
    /// 工具调用 id → 序号。
    tools: HashMap<String, usize>,
    finished: bool,
}

impl ChatSse {
    pub fn new(model: &str) -> Self {
        Self {
            id: format!("chatcmpl-{}", uuid::Uuid::new_v4().simple()),
            model: model.to_string(),
            created: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_secs())
                .unwrap_or(0),
            role_sent: false,
            tools: HashMap::new(),
            finished: false,
        }
    }

    fn chunk(&mut self, mut delta: Value, finish_reason: Option<&str>) -> Vec<u8> {
        if !self.role_sent {
            delta["role"] = json!("assistant");
            self.role_sent = true;
        }
        let chunk = json!({
            "id": self.id,
            "object": "chat.completion.chunk",
            "created": self.created,
            "model": self.model,
            "choices": [{ "index": 0, "delta": delta, "finish_reason": finish_reason }],
        });
        sse(&chunk)
    }

    pub fn text(&mut self, text: &str) -> Vec<u8> {
        if text.is_empty() {
            return Vec::new();
        }
        self.chunk(json!({ "content": text }), None)
    }

    pub fn reasoning(&mut self, text: &str) -> Vec<u8> {
        if text.is_empty() {
            return Vec::new();
        }
        self.chunk(json!({ "reasoning_content": text }), None)
    }

    /// 工具调用开始（同一个 id 只开一次）；`arguments` 可以先给一部分或空串。
    pub fn tool_call(&mut self, call_id: &str, name: &str, arguments: &str) -> Vec<u8> {
        if let Some(index) = self.tools.get(call_id).copied() {
            return self.tool_arguments_at(index, arguments);
        }
        let index = self.tools.len();
        self.tools.insert(call_id.to_string(), index);
        self.chunk(
            json!({ "tool_calls": [{
                "index": index,
                "id": call_id,
                "type": "function",
                "function": { "name": name, "arguments": arguments },
            }] }),
            None,
        )
    }

    /// 工具调用参数的增量。
    pub fn tool_arguments(&mut self, call_id: &str, delta: &str) -> Vec<u8> {
        match self.tools.get(call_id).copied() {
            Some(index) => self.tool_arguments_at(index, delta),
            None => Vec::new(),
        }
    }

    fn tool_arguments_at(&mut self, index: usize, delta: &str) -> Vec<u8> {
        if delta.is_empty() {
            return Vec::new();
        }
        self.chunk(
            json!({ "tool_calls": [{ "index": index, "function": { "arguments": delta } }] }),
            None,
        )
    }

    pub fn has_tool_calls(&self) -> bool {
        !self.tools.is_empty()
    }

    /// 结束：`reason` 为 None 时按有没有工具调用选 `tool_calls` / `stop`。带 usage 块和 `[DONE]`。
    pub fn finish(&mut self, reason: Option<&str>, usage: Option<Usage>) -> Vec<u8> {
        if self.finished {
            return Vec::new();
        }
        self.finished = true;
        let reason = reason.unwrap_or(if self.has_tool_calls() {
            "tool_calls"
        } else {
            "stop"
        });
        let mut out = self.chunk(json!({}), Some(reason));
        if let Some(usage) = usage {
            out.extend(sse(&json!({
                "id": self.id,
                "object": "chat.completion.chunk",
                "created": self.created,
                "model": self.model,
                "choices": [],
                "usage": usage.to_json(),
            })));
        }
        out.extend_from_slice(b"data: [DONE]\n\n");
        out
    }

    /// 中途失败：发一个错误事件再结束。
    pub fn error(&mut self, message: &str) -> Vec<u8> {
        if self.finished {
            return Vec::new();
        }
        self.finished = true;
        let mut out = sse(&json!({ "error": { "message": message, "type": "upstream_error" } }));
        out.extend_from_slice(b"data: [DONE]\n\n");
        out
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }
}

fn sse(value: &Value) -> Vec<u8> {
    format!("data: {value}\n\n").into_bytes()
}

/// 把一段 Chat SSE 拼回非流式的 `chat.completion`（客户端要非流式时用）。
pub fn aggregate(sse_text: &str, model: &str) -> anyhow::Result<Value> {
    let mut content = String::new();
    let mut reasoning = String::new();
    let mut tools: Vec<Value> = Vec::new();
    let mut finish_reason = Value::Null;
    let mut usage = Value::Null;
    let mut id = String::new();
    for line in sse_text.lines() {
        let Some(data) = line.strip_prefix("data:").map(str::trim) else {
            continue;
        };
        if data == "[DONE]" || data.is_empty() {
            continue;
        }
        let chunk: Value = serde_json::from_str(data)?;
        if let Some(error) = chunk.get("error") {
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("上游返回错误");
            anyhow::bail!("{message}");
        }
        if id.is_empty()
            && let Some(value) = chunk.get("id").and_then(Value::as_str)
        {
            id = value.to_string();
        }
        if let Some(value) = chunk.get("usage").filter(|value| !value.is_null()) {
            usage = value.clone();
        }
        let Some(choice) = chunk.get("choices").and_then(|choices| choices.get(0)) else {
            continue;
        };
        if let Some(reason) = choice.get("finish_reason").filter(|value| !value.is_null()) {
            finish_reason = reason.clone();
        }
        let delta = choice.get("delta").cloned().unwrap_or(Value::Null);
        if let Some(text) = delta.get("content").and_then(Value::as_str) {
            content.push_str(text);
        }
        if let Some(text) = delta.get("reasoning_content").and_then(Value::as_str) {
            reasoning.push_str(text);
        }
        for call in delta
            .get("tool_calls")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let index = call.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
            while tools.len() <= index {
                tools.push(json!({ "id": "", "type": "function", "function": { "name": "", "arguments": "" } }));
            }
            let slot = &mut tools[index];
            if let Some(value) = call.get("id").and_then(Value::as_str) {
                slot["id"] = json!(value);
            }
            if let Some(value) = call.pointer("/function/name").and_then(Value::as_str) {
                slot["function"]["name"] = json!(value);
            }
            if let Some(value) = call.pointer("/function/arguments").and_then(Value::as_str) {
                let joined = format!(
                    "{}{value}",
                    slot["function"]["arguments"].as_str().unwrap_or_default()
                );
                slot["function"]["arguments"] = json!(joined);
            }
        }
    }
    let mut message = json!({ "role": "assistant", "content": content });
    if !reasoning.is_empty() {
        message["reasoning_content"] = json!(reasoning);
    }
    if !tools.is_empty() {
        message["tool_calls"] = json!(tools);
    }
    let mut completion = json!({
        "id": if id.is_empty() { format!("chatcmpl-{}", uuid::Uuid::new_v4().simple()) } else { id },
        "object": "chat.completion",
        "created": 0,
        "model": model,
        "choices": [{ "index": 0, "message": message, "finish_reason": if finish_reason.is_null() { json!("stop") } else { finish_reason } }],
    });
    if !usage.is_null() {
        completion["usage"] = usage;
    }
    Ok(completion)
}

/// 按行切 SSE：喂进字节，吐出完整的 `(event, data)`（`event` 没有时为空串）。
#[derive(Debug, Default)]
pub struct SseReader {
    buffer: Vec<u8>,
    event: String,
    data: Vec<String>,
}

impl SseReader {
    pub fn push(&mut self, bytes: &[u8]) -> Vec<(String, String)> {
        self.buffer.extend_from_slice(bytes);
        let mut out = Vec::new();
        while let Some(position) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=position).collect();
            let line = String::from_utf8_lossy(&line);
            let line = line.trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                if !self.data.is_empty() || !self.event.is_empty() {
                    out.push((std::mem::take(&mut self.event), self.data.join("\n")));
                    self.data.clear();
                }
                continue;
            }
            if let Some(value) = line.strip_prefix("event:") {
                self.event = value.trim().to_string();
            } else if let Some(value) = line.strip_prefix("data:") {
                self.data.push(value.strip_prefix(' ').unwrap_or(value).to_string());
            }
        }
        out
    }

    /// 流结束时还没遇到空行的最后一个事件。
    pub fn finish(&mut self) -> Option<(String, String)> {
        if !self.buffer.is_empty() {
            let rest = std::mem::take(&mut self.buffer);
            let mut extra = self.push(&rest);
            extra.extend(self.push(b"\n\n"));
            return extra.pop();
        }
        if self.data.is_empty() {
            return None;
        }
        Some((std::mem::take(&mut self.event), self.data.join("\n")))
    }
}

/// 上游已经是 OpenAI Chat Completions（拿到 token 后接口兼容）：把它的 SSE 原样当结果流。
pub fn passthrough(response: reqwest::Response) -> ChatStream {
    Box::pin(
        response
            .bytes_stream()
            .map(|chunk| chunk.map(|bytes| bytes.to_vec()).map_err(anyhow::Error::from)),
    )
}

/// 上游回了非 2xx：读出错误信息，按状态码包成 [`crate::gateway::subscriptions::fail`]。
pub async fn upstream_failure(response: reqwest::Response, vendor: &str) -> anyhow::Error {
    let status = response.status().as_u16();
    let body = response.text().await.unwrap_or_default();
    let detail = serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|value| {
            value
                .pointer("/error/message")
                .or_else(|| value.get("message"))
                .or_else(|| value.get("error"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| body.chars().take(300).collect());
    crate::gateway::subscriptions::fail(status, format!("{vendor} 返回 {status}：{detail}"))
}

/// 把 Chat 请求里的消息压成纯文本对话（给只收「一段提示」的上游用）。
pub fn message_text(message: &Value) -> String {
    match message.get("content") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_sse_round_trips_through_aggregate() {
        let mut sse = ChatSse::new("m");
        let mut out = sse.reasoning("想");
        out.extend(sse.text("你"));
        out.extend(sse.text("好"));
        out.extend(sse.tool_call("call_1", "read", "{\"p\":"));
        out.extend(sse.tool_arguments("call_1", "1}"));
        out.extend(sse.finish(
            None,
            Some(Usage {
                prompt_tokens: 3,
                completion_tokens: 2,
                ..Usage::default()
            }),
        ));
        let text = String::from_utf8(out).unwrap();
        assert!(text.ends_with("data: [DONE]\n\n"));
        let completion = aggregate(&text, "m").unwrap();
        let message = &completion["choices"][0]["message"];
        assert_eq!(message["content"], "你好");
        assert_eq!(message["reasoning_content"], "想");
        assert_eq!(message["tool_calls"][0]["function"]["arguments"], "{\"p\":1}");
        assert_eq!(completion["choices"][0]["finish_reason"], "tool_calls");
        assert_eq!(completion["usage"]["total_tokens"], 5);
    }

    #[test]
    fn sse_reader_handles_split_lines() {
        let mut reader = SseReader::default();
        assert!(reader.push(b"event: a\nda").is_empty());
        let events = reader.push(b"ta: {\"x\":1}\n\ndata: 2\n");
        assert_eq!(events, vec![("a".to_string(), "{\"x\":1}".to_string())]);
        assert_eq!(reader.finish(), Some((String::new(), "2".to_string())));
    }

    #[test]
    fn aggregate_reports_error_event() {
        let mut sse = ChatSse::new("m");
        let text = String::from_utf8(sse.error("额度用完")).unwrap();
        assert!(aggregate(&text, "m").unwrap_err().to_string().contains("额度用完"));
    }
}
