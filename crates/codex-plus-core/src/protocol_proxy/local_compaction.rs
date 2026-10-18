use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use futures_util::{Stream, StreamExt};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{ProxySessionHeaders, UpstreamWireApi};
use crate::settings::RelayProfile;

const MAX_REQUEST_BYTES: usize = 8 * 1024 * 1024;
const MAX_CACHE_BYTES: usize = 32 * 1024 * 1024;
const MAX_CACHE_ENTRIES: usize = 16;
const CACHE_TTL: Duration = Duration::from_secs(15 * 60);
const MAX_SUMMARY_BYTES: usize = 4 * 1024 * 1024;
const SUMMARY_IDLE_TIMEOUT: Duration = Duration::from_secs(180);

struct Entry {
    body: Value,
    exec_adapted: bool,
    bytes: usize,
    updated: Instant,
}

pub(super) struct Candidate {
    pub body: Value,
    pub exec_adapted: bool,
}

#[derive(Default)]
struct RequestCache {
    entries: BTreeMap<String, Entry>,
}

fn cache() -> &'static Mutex<RequestCache> {
    static CACHE: OnceLock<Mutex<RequestCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(RequestCache::default()))
}

pub(super) fn is_local_request(body: &Value, summary_prompt: &str) -> bool {
    if !matches!(body.get("tools"), None | Some(Value::Null))
        && !body
            .get("tools")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty)
    {
        return false;
    }
    let Some(last) = body
        .get("input")
        .and_then(Value::as_array)
        .and_then(|items| items.last())
    else {
        return false;
    };
    if last["role"] != "user"
        || !matches!(
            last.get("type").and_then(Value::as_str),
            None | Some("message")
        )
    {
        return false;
    }
    let text = match last.get("content") {
        Some(Value::String(text)) => Some(text.as_str()),
        Some(Value::Array(parts)) if parts.len() == 1 && parts[0]["type"] == "input_text" => {
            parts[0]["text"].as_str()
        }
        _ => None,
    };
    let Some(text) = text.filter(|text| !text.trim().is_empty()) else {
        return false;
    };
    let native_metadata = body
        .pointer("/client_metadata/x-codex-turn-metadata")
        .and_then(Value::as_str)
        .and_then(|metadata| serde_json::from_str::<Value>(metadata).ok());
    if let Some(metadata) = native_metadata
        && metadata["request_kind"] == "compaction"
    {
        return metadata.pointer("/compaction/implementation") == Some(&json!("responses"));
    }
    text.replace("\r\n", "\n").trim() == summary_prompt.replace("\r\n", "\n").trim()
}

pub(super) fn key(
    relay: &RelayProfile,
    body: &Value,
    headers: ProxySessionHeaders<'_>,
) -> Option<String> {
    let (session_kind, session) = [
        ("thread-id", headers.thread_id),
        ("session-id", headers.session_id),
        ("x-opencode-session", headers.opencode_session),
        (
            "conversation_id",
            body.get("conversation_id").and_then(Value::as_str),
        ),
        (
            "conversation",
            body.get("conversation").and_then(Value::as_str),
        ),
    ]
    .into_iter()
    .filter_map(|(kind, value)| value.map(|value| (kind, value)))
    .find(|(_, value)| !value.trim().is_empty())?;
    let headers_digest = Sha256::digest(serde_json::to_vec(&relay.custom_headers).ok()?);
    Some(
        json!([
            relay.id,
            relay.base_url,
            relay.protocol,
            relay.standard_openai_protocol,
            format!("{:x}", Sha256::digest(relay.api_key.as_bytes())),
            format!("{headers_digest:x}"),
            session_kind,
            session,
            body.get("model"),
            body.get("prompt_cache_key")
        ])
        .to_string(),
    )
}

impl RequestCache {
    fn remember(&mut self, key: String, body: &Value, exec_adapted: bool) {
        self.entries.remove(&key);
        let bytes = body.to_string().len();
        if bytes > MAX_REQUEST_BYTES {
            return;
        }
        self.entries
            .retain(|_, entry| entry.updated.elapsed() < CACHE_TTL);
        self.entries.insert(
            key,
            Entry {
                body: body.clone(),
                exec_adapted,
                bytes,
                updated: Instant::now(),
            },
        );
        while self.entries.len() > MAX_CACHE_ENTRIES
            || self
                .entries
                .values()
                .map(|entry| entry.bytes)
                .sum::<usize>()
                > MAX_CACHE_BYTES
        {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.updated)
                .map(|(key, _)| key.clone());
            if let Some(oldest) = oldest {
                self.entries.remove(&oldest);
            }
        }
    }

    fn candidate(&self, key: &str, body: &Value, wire_api: UpstreamWireApi) -> Option<Candidate> {
        let entry = self.entries.get(key)?;
        if entry.updated.elapsed() >= CACHE_TTL {
            return None;
        }
        let history_key = match wire_api {
            UpstreamWireApi::Responses => "input",
            UpstreamWireApi::ChatCompletions => "messages",
            _ => return None,
        };
        let prefix = entry.body.get(history_key)?.as_array()?;
        let history = body.get(history_key)?.as_array()?;
        // Compare logical history before either request receives exec wire adaptation.
        if prefix.is_empty()
            || history.len() <= prefix.len()
            || !history.starts_with(prefix)
            || entry.body.get("instructions") != body.get("instructions")
            || entry.body.get("previous_response_id") != body.get("previous_response_id")
        {
            return None;
        }
        let tools = entry
            .body
            .get("tools")?
            .as_array()
            .filter(|tools| !tools.is_empty())?;
        let mut candidate = body.clone();
        let object = candidate.as_object_mut()?;
        object.insert("tools".into(), Value::Array(tools.clone()));
        object.insert("tool_choice".into(), json!("none"));
        object.insert("parallel_tool_calls".into(), json!(false));
        Some(Candidate {
            body: candidate,
            exec_adapted: entry.exec_adapted,
        })
    }
}

pub(super) fn remember(key: String, body: &Value, exec_adapted: bool) {
    if let Ok(mut cache) = cache().lock() {
        cache.remember(key, body, exec_adapted);
    }
}

pub(super) fn candidate(key: &str, body: &Value, wire_api: UpstreamWireApi) -> Option<Candidate> {
    cache().lock().ok()?.candidate(key, body, wire_api)
}

fn text_only_item(item: &Value) -> Option<bool> {
    match item["type"].as_str()? {
        "reasoning" => Some(false),
        "message" if item["role"] == "assistant" => {
            let mut has_text = false;
            for part in item.get("content")?.as_array()? {
                if part["type"] != "output_text" {
                    return None;
                }
                has_text |= !part["text"].as_str()?.trim().is_empty();
            }
            Some(has_text)
        }
        _ => None,
    }
}

fn valid_response(response: &Value) -> bool {
    response["id"]
        .as_str()
        .is_some_and(|id| !id.trim().is_empty())
        && response["status"] == "completed"
        && response.get("error").is_none_or(Value::is_null)
        && response
            .get("incomplete_details")
            .is_none_or(Value::is_null)
        && response
            .get("output")
            .and_then(Value::as_array)
            .and_then(|items| {
                items.iter().try_fold(false, |has_text, item| {
                    if item
                        .get("status")
                        .is_some_and(|status| status != "completed")
                    {
                        return None;
                    }
                    Some(has_text | text_only_item(item)?)
                })
            })
            == Some(true)
}

fn no_chat_tools(message: &Value) -> bool {
    message
        .get("tool_calls")
        .is_none_or(|calls| calls.is_null() || calls.as_array().is_some_and(Vec::is_empty))
        && message.get("function_call").is_none_or(Value::is_null)
}

fn valid_chat_response(response: &Value) -> bool {
    response.get("error").is_none_or(Value::is_null)
        && response
            .get("choices")
            .and_then(Value::as_array)
            .is_some_and(|choices| {
                choices.len() == 1
                    && choices[0]["finish_reason"] == "stop"
                    && no_chat_tools(&choices[0]["message"])
                    && choices[0]["message"]
                        .get("role")
                        .is_none_or(|role| role == "assistant")
                    && choices[0]["message"]["content"]
                        .as_str()
                        .is_some_and(|text| !text.trim().is_empty())
            })
}

#[derive(Default)]
struct SummaryEvents {
    completed: Option<Value>,
    response_id: Option<String>,
    deltas: BTreeMap<(usize, usize), String>,
    done_text: BTreeMap<(usize, usize), String>,
    done_items: BTreeMap<usize, Value>,
    chat_completed: bool,
    chat_has_text: bool,
    done_sentinel: bool,
}

impl SummaryEvents {
    fn push(&mut self, block: &[u8], wire_api: UpstreamWireApi) -> Result<(), &'static str> {
        let text = std::str::from_utf8(block).map_err(|_| "invalid_summary_or_tool_call")?;
        let mut event_name = None;
        let mut parts = Vec::new();
        for line in text.lines() {
            if line.is_empty() || line.starts_with(':') {
                continue;
            }
            let (field, value) = line.split_once(':').unwrap_or((line, ""));
            let value = value.strip_prefix(' ').unwrap_or(value);
            match field {
                "event" => event_name = Some(value),
                "data" => parts.push(value),
                "id" | "retry" => {}
                _ => return Err("invalid_summary_or_tool_call"),
            }
        }
        if parts.is_empty() {
            return Ok(());
        }
        let data = parts.join("\n");
        if data == "[DONE]" {
            if self.done_sentinel || !(self.completed.is_some() || self.chat_completed) {
                return Err("invalid_summary_or_tool_call");
            }
            self.done_sentinel = true;
            return Ok(());
        }
        if self.done_sentinel || self.completed.is_some() {
            return Err("invalid_summary_or_tool_call");
        }
        let event: Value =
            serde_json::from_str(&data).map_err(|_| "invalid_summary_or_tool_call")?;
        if !event.is_object()
            || event.get("error").is_some_and(|error| !error.is_null())
            || event
                .pointer("/response/error")
                .is_some_and(|error| !error.is_null())
        {
            return Err("invalid_summary_or_tool_call");
        }
        match wire_api {
            UpstreamWireApi::Responses => {
                let kind = event
                    .get("type")
                    .and_then(Value::as_str)
                    .or(event_name)
                    .ok_or("invalid_summary_or_tool_call")?;
                if event_name.is_some_and(|name| name != kind) {
                    return Err("invalid_summary_or_tool_call");
                }
                self.push_responses(&event, kind)
            }
            UpstreamWireApi::ChatCompletions => self.push_chat(&event),
            _ => Err("invalid_summary_or_tool_call"),
        }
    }

    fn push_responses(&mut self, event: &Value, kind: &str) -> Result<(), &'static str> {
        let invalid = "invalid_summary_or_tool_call";
        if let Some(response) = event.get("response") {
            if response.get("status").is_some_and(|status| {
                status != "completed" && status != "in_progress" && status != "queued"
            }) {
                return Err(invalid);
            }
            let id = response["id"]
                .as_str()
                .filter(|id| !id.trim().is_empty())
                .ok_or(invalid)?;
            if self
                .response_id
                .as_deref()
                .is_some_and(|previous| previous != id)
            {
                return Err(invalid);
            }
            self.response_id = Some(id.to_string());
            if let Some(items) = response.get("output").and_then(Value::as_array)
                && items.iter().any(|item| text_only_item(item).is_none())
            {
                return Err(invalid);
            }
        }
        match kind {
            "response.created" | "response.in_progress" | "response.queued" => {
                if event.get("response").is_none() {
                    return Err(invalid);
                }
            }
            "response.output_item.added" | "response.output_item.done" => {
                let item = event.get("item").ok_or(invalid)?;
                text_only_item(item).ok_or(invalid)?;
                let index = event["output_index"].as_u64().ok_or(invalid)? as usize;
                if kind == "response.output_item.done"
                    && self.done_items.insert(index, item.clone()).is_some()
                {
                    return Err(invalid);
                }
            }
            "response.content_part.added" | "response.content_part.done" => {
                let part = event.get("part").ok_or(invalid)?;
                if part["type"] != "output_text" || !part["text"].is_string() {
                    return Err(invalid);
                }
                if kind == "response.content_part.done" {
                    self.capture_done_text(event, &part["text"])?;
                }
            }
            "response.output_text.delta" => {
                let index = text_index(event).ok_or(invalid)?;
                self.deltas
                    .entry(index)
                    .or_default()
                    .push_str(event["delta"].as_str().ok_or(invalid)?);
            }
            "response.output_text.done" => self.capture_done_text(event, &event["text"])?,
            "response.reasoning_summary_part.added"
            | "response.reasoning_summary_part.done"
            | "response.reasoning_summary_text.delta"
            | "response.reasoning_summary_text.done"
            | "response.reasoning_text.delta"
            | "response.reasoning_text.done" => {}
            "response.completed" => {
                let response = event.get("response").ok_or(invalid)?;
                if !valid_response(response) || !self.matches_output(response) {
                    return Err(invalid);
                }
                self.completed = Some(response.clone());
            }
            _ => return Err(invalid),
        }
        Ok(())
    }

    fn capture_done_text(&mut self, event: &Value, text: &Value) -> Result<(), &'static str> {
        let invalid = "invalid_summary_or_tool_call";
        let index = text_index(event).ok_or(invalid)?;
        let text = text.as_str().ok_or(invalid)?;
        if self
            .done_text
            .get(&index)
            .is_some_and(|previous| previous != text)
        {
            return Err(invalid);
        }
        self.done_text.insert(index, text.to_string());
        Ok(())
    }

    fn matches_output(&self, response: &Value) -> bool {
        let Some(output) = response["output"].as_array() else {
            return false;
        };
        let final_text = |&(item_index, content_index): &(usize, usize)| {
            let item = output.get(item_index)?;
            if item["type"] != "message" || item["role"] != "assistant" {
                return None;
            }
            item["content"].as_array()?.get(content_index)?["text"].as_str()
        };
        self.deltas
            .iter()
            .all(|(index, text)| final_text(index) == Some(text.as_str()))
            && self
                .done_text
                .iter()
                .all(|(index, text)| final_text(index) == Some(text.as_str()))
            && self
                .done_items
                .iter()
                .all(|(index, item)| output.get(*index) == Some(item))
    }

    fn push_chat(&mut self, event: &Value) -> Result<(), &'static str> {
        let invalid = "invalid_summary_or_tool_call";
        let choices = event["choices"].as_array().ok_or(invalid)?;
        // Empty choices are the usual usage-only chunk after the stop chunk.
        if choices.is_empty() {
            return Ok(());
        }
        if choices.len() != 1 || self.chat_completed {
            return Err(invalid);
        }
        let choice = &choices[0];
        if choice.get("index").is_some_and(|index| index != 0)
            || !no_chat_tools(&choice["delta"])
            || choice
                .pointer("/delta/role")
                .is_some_and(|role| role != "assistant")
        {
            return Err(invalid);
        }
        if let Some(content) = choice
            .pointer("/delta/content")
            .filter(|value| !value.is_null())
        {
            self.chat_has_text |= !content.as_str().ok_or(invalid)?.trim().is_empty();
        }
        if let Some(reason) = choice.get("finish_reason").filter(|value| !value.is_null()) {
            if reason != "stop" {
                return Err(invalid);
            }
            self.chat_completed = true;
        }
        Ok(())
    }

    fn finish(self, original: Vec<u8>, wire_api: UpstreamWireApi) -> Result<Vec<u8>, &'static str> {
        let invalid = "invalid_summary_or_tool_call";
        match wire_api {
            UpstreamWireApi::Responses => {
                let response = self.completed.ok_or(invalid)?;
                let mut output = String::new();
                // Native Codex collects items from output_item.done, including final-only providers.
                for (index, item) in response["output"]
                    .as_array()
                    .ok_or(invalid)?
                    .iter()
                    .enumerate()
                {
                    super::push_sse(
                        &mut output,
                        "response.output_item.done",
                        json!({"type":"response.output_item.done","sequence_number":index,
                            "output_index":index,"item":item}),
                    );
                }
                super::push_sse(
                    &mut output,
                    "response.completed",
                    json!({"type":"response.completed",
                        "sequence_number":response["output"].as_array().ok_or(invalid)?.len(),
                        "response":response}),
                );
                output.push_str("data: [DONE]\n\n");
                if output.len() > MAX_SUMMARY_BYTES {
                    return Err("summary_size");
                }
                Ok(output.into_bytes())
            }
            UpstreamWireApi::ChatCompletions if self.chat_completed && self.chat_has_text => {
                Ok(original)
            }
            _ => Err(invalid),
        }
    }
}

fn text_index(event: &Value) -> Option<(usize, usize)> {
    Some((
        usize::try_from(event["output_index"].as_u64()?).ok()?,
        usize::try_from(event["content_index"].as_u64()?).ok()?,
    ))
}

#[derive(Default)]
struct SseBuffer {
    pending: Vec<u8>,
    scan_from: usize,
    events: SummaryEvents,
}

impl SseBuffer {
    fn push(&mut self, bytes: &[u8], wire_api: UpstreamWireApi) -> Result<(), &'static str> {
        self.pending.extend_from_slice(bytes);
        loop {
            let boundary = (self.scan_from..self.pending.len()).find_map(|index| {
                if self.pending[index..].starts_with(b"\n\n") {
                    Some((index, 2))
                } else if self.pending[index..].starts_with(b"\r\n\r\n") {
                    Some((index, 4))
                } else {
                    None
                }
            });
            let Some((index, size)) = boundary else {
                self.scan_from = self.pending.len().saturating_sub(3);
                return Ok(());
            };
            self.events.push(&self.pending[..index], wire_api)?;
            self.pending.drain(..index + size);
            self.scan_from = 0;
        }
    }
}

async fn read_summary<S, B, E>(
    stream: S,
    wire_api: UpstreamWireApi,
    streaming: bool,
    idle_timeout: Duration,
) -> Result<Vec<u8>, &'static str>
where
    S: Stream<Item = Result<B, E>>,
    B: AsRef<[u8]>,
{
    futures_util::pin_mut!(stream);
    let mut bytes = Vec::new();
    let mut sse = SseBuffer::default();
    let mut last_progress = tokio::time::Instant::now();
    loop {
        let chunk = tokio::time::timeout_at(last_progress + idle_timeout, stream.next())
            .await
            .map_err(|_| "summary_timeout")?;
        let Some(chunk) = chunk else {
            break;
        };
        let chunk = chunk.map_err(|_| "stream_read")?;
        let chunk = chunk.as_ref();
        if chunk.len() > MAX_SUMMARY_BYTES.saturating_sub(bytes.len()) {
            return Err("summary_size");
        }
        if !chunk.is_empty() {
            last_progress = tokio::time::Instant::now();
            if streaming {
                sse.push(chunk, wire_api)?;
            }
            bytes.extend_from_slice(chunk);
        }
    }
    if streaming {
        if !sse.pending.is_empty() {
            return Err("invalid_summary_or_tool_call");
        }
        sse.events.finish(bytes, wire_api)
    } else {
        let response: Value =
            serde_json::from_slice(&bytes).map_err(|_| "invalid_summary_or_tool_call")?;
        let valid = match wire_api {
            UpstreamWireApi::Responses => valid_response(&response),
            UpstreamWireApi::ChatCompletions => valid_chat_response(&response),
            _ => false,
        };
        if valid {
            Ok(bytes)
        } else {
            Err("invalid_summary_or_tool_call")
        }
    }
}

pub(super) async fn buffer_summary(
    upstream: reqwest::Response,
    wire_api: UpstreamWireApi,
    streaming: bool,
) -> Result<reqwest::Response, &'static str> {
    if !upstream.status().is_success() {
        return Err("upstream_status");
    }
    let status = upstream.status();
    let version = upstream.version();
    let mut headers = upstream.headers().clone();
    let streaming = streaming
        || headers
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.contains("text/event-stream"));
    let bytes = read_summary(
        upstream.bytes_stream(),
        wire_api,
        streaming,
        SUMMARY_IDLE_TIMEOUT,
    )
    .await?;
    // The normalized body may differ in size and no longer uses upstream transfer encoding.
    headers.remove(reqwest::header::CONTENT_LENGTH);
    headers.remove(reqwest::header::TRANSFER_ENCODING);
    if streaming {
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            reqwest::header::HeaderValue::from_static("text/event-stream"),
        );
    }
    let mut buffered = http::Response::new(bytes);
    *buffered.status_mut() = status;
    *buffered.version_mut() = version;
    *buffered.headers_mut() = headers;
    Ok(reqwest::Response::from(buffered))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(text: &str) -> Value {
        json!({"id":"msg_summary","type":"message","role":"assistant","status":"completed",
            "content":[{"type":"output_text","text":text,"annotations":[]}]})
    }

    fn response() -> Value {
        json!({"id":"resp_summary","status":"completed","output":[message("summary")],
            "usage":{"input_tokens":100,"output_tokens":2,"total_tokens":102}})
    }

    fn frame(event: Value) -> Vec<u8> {
        format!("data: {event}\n\n").into_bytes()
    }

    async fn validate(
        bytes: Vec<u8>,
        wire_api: UpstreamWireApi,
        streaming: bool,
    ) -> Result<Vec<u8>, &'static str> {
        read_summary(
            futures_util::stream::iter([Ok::<_, ()>(bytes)]),
            wire_api,
            streaming,
            Duration::from_secs(1),
        )
        .await
    }

    #[test]
    fn native_detection_preserves_windows_prompt_bytes_and_excludes_remote_metadata() {
        let canonical = super::super::COMPACTION_SUMMARY_INSTRUCTION;
        let mut body = json!({"tools":[],"input":[{"role":"user","content":[
            {"type":"input_text","text":canonical.replace('\n', "\r\n")}
        ]}]});
        assert!(is_local_request(&body, canonical));
        body["input"][0]["content"][0]["text"] = json!("Custom checkpoint");
        assert!(!is_local_request(&body, canonical));
        body["client_metadata"] = json!({"x-codex-turn-metadata":json!({
            "request_kind":"compaction","compaction":{"implementation":"responses"}
        }).to_string()});
        assert!(is_local_request(&body, canonical));
        body["input"][0]["content"][0]["text"] = json!(canonical);
        body["client_metadata"]["x-codex-turn-metadata"] = json!(
            json!({"request_kind":"compaction","compaction":{"implementation":"remote"}})
                .to_string()
        );
        assert!(!is_local_request(&body, canonical));
        body["tools"] = json!([{"type":"function","name":"run"}]);
        assert!(!is_local_request(&body, canonical));
    }

    #[test]
    fn cache_identity_isolates_credentials_protocol_model_cache_key_and_session() {
        use crate::settings::RelayHeaderKeyValue;
        let body = json!({"model":"model","prompt_cache_key":"thread"});
        let headers = ProxySessionHeaders {
            thread_id: Some("thread"),
            ..ProxySessionHeaders::default()
        };
        let relay = RelayProfile::default();
        let original = key(&relay, &body, headers).unwrap();
        let mut variants = Vec::new();
        let mut changed = relay.clone();
        changed.standard_openai_protocol = !changed.standard_openai_protocol;
        variants.push(changed);
        let mut changed = relay.clone();
        changed.protocol = crate::settings::RelayProtocol::ChatCompletions;
        variants.push(changed);
        let mut changed = relay.clone();
        changed.api_key = "different-private-key".into();
        variants.push(changed);
        let mut changed = relay.clone();
        changed.custom_headers.push(RelayHeaderKeyValue {
            key: "Authorization".into(),
            value: "Bearer changed".into(),
        });
        variants.push(changed);
        let mut changed = relay.clone();
        changed.base_url = "https://other.example".into();
        variants.push(changed);
        let mut changed = relay.clone();
        changed.id = "other-relay".into();
        variants.push(changed);
        for changed in variants {
            assert_ne!(key(&changed, &body, headers).unwrap(), original);
        }
        assert!(!original.contains("different-private-key"));
        for changed in [
            json!({"model":"other","prompt_cache_key":"thread"}),
            json!({"model":"model","prompt_cache_key":"other"}),
        ] {
            assert_ne!(key(&relay, &changed, headers).unwrap(), original);
        }
        for headers in [
            ProxySessionHeaders {
                thread_id: Some("other"),
                ..headers
            },
            ProxySessionHeaders {
                thread_id: None,
                session_id: Some("thread"),
                ..headers
            },
            ProxySessionHeaders {
                thread_id: None,
                opencode_session: Some("thread"),
                ..ProxySessionHeaders::default()
            },
        ] {
            assert_ne!(key(&relay, &body, headers).unwrap(), original);
        }
        assert!(key(&relay, &body, ProxySessionHeaders::default()).is_none());
    }

    #[test]
    fn candidate_restores_only_declarations_and_forces_no_tools_preserving_current_controls() {
        let mut cache = RequestCache::default();
        let original = json!({"input":[{"content":"old"}],"instructions":"system",
            "previous_response_id":"resp_previous","tools":[{"type":"function","name":"run"}],
            "tool_choice":{"type":"function","name":"run"},"parallel_tool_calls":true,
            "reasoning":{"effort":"high"},"metadata":{"old":true},"temperature":1});
        cache.remember("session".into(), &original, true);
        let current = json!({"input":[{"content":"old"},{"content":"summary"}],"instructions":"system",
            "previous_response_id":"resp_previous","tools":[],"tool_choice":"required",
            "parallel_tool_calls":true,"reasoning":{"effort":"low"},"max_output_tokens":128,
            "text":{"format":{"type":"json_schema","name":"checkpoint","schema":{"type":"object"}}},
            "metadata":{"new":true},"client_metadata":{"turn":"new"},"stream":true,
            "temperature":0,"store":false,"truncation":"disabled","service_tier":"priority"});
        let candidate = cache
            .candidate("session", &current, UpstreamWireApi::Responses)
            .unwrap();
        let mut expected = current.clone();
        expected["tools"] = original["tools"].clone();
        expected["tool_choice"] = json!("none");
        expected["parallel_tool_calls"] = json!(false);
        assert_eq!(candidate.body, expected);
        assert!(candidate.exec_adapted);
        let mut absent_controls = current.clone();
        for name in ["reasoning", "metadata", "temperature", "stream"] {
            absent_controls.as_object_mut().unwrap().remove(name);
        }
        let candidate = cache
            .candidate("session", &absent_controls, UpstreamWireApi::Responses)
            .unwrap();
        for name in ["reasoning", "metadata", "temperature", "stream"] {
            assert!(candidate.body.get(name).is_none());
        }
    }

    #[test]
    fn candidate_requires_exact_logical_history_instructions_and_previous_response_id() {
        let mut cache = RequestCache::default();
        let original = json!({"input":[{"type":"custom_tool_call","name":"exec_command",
            "input":"Get-Location"}],"tools":[{"type":"custom","name":"exec_command"}],
            "instructions":"system","previous_response_id":"resp_old"});
        cache.remember("session".into(), &original, true);
        let mut current = original.clone();
        current["tools"] = json!([]);
        current["input"]
            .as_array_mut()
            .unwrap()
            .push(json!({"role":"user","content":"summary"}));
        assert!(
            cache
                .candidate("session", &current, UpstreamWireApi::Responses)
                .is_some()
        );
        let mut changed = current.clone();
        changed["input"][0]["type"] = json!("function_call");
        assert!(
            cache
                .candidate("session", &changed, UpstreamWireApi::Responses)
                .is_none()
        );
        for field in ["instructions", "previous_response_id"] {
            let mut changed = current.clone();
            changed[field] = json!("different");
            assert!(
                cache
                    .candidate("session", &changed, UpstreamWireApi::Responses)
                    .is_none()
            );
            changed.as_object_mut().unwrap().remove(field);
            assert!(
                cache
                    .candidate("session", &changed, UpstreamWireApi::Responses)
                    .is_none()
            );
        }
        current["input"].as_array_mut().unwrap().remove(0);
        assert!(
            cache
                .candidate("session", &current, UpstreamWireApi::Responses)
                .is_none()
        );
        assert!(
            cache
                .candidate("other", &current, UpstreamWireApi::Responses)
                .is_none()
        );
        assert!(
            cache
                .candidate("session", &original, UpstreamWireApi::Responses)
                .is_none()
        );
    }

    #[test]
    fn chat_candidate_retains_current_messages_and_exec_flag_is_not_inferred() {
        let mut cache = RequestCache::default();
        let original = json!({"messages":[{"role":"user","content":"old"}],
            "tools":[{"type":"function","function":{"name":"run"}}]});
        cache.remember("chat".into(), &original, false);
        let current = json!({"messages":[{"role":"user","content":"old"},
            {"role":"user","content":"summary"}],"max_tokens":123,"response_format":{"type":"json_object"}});
        let candidate = cache
            .candidate("chat", &current, UpstreamWireApi::ChatCompletions)
            .unwrap();
        assert_eq!(candidate.body["messages"], current["messages"]);
        assert_eq!(
            candidate.body["response_format"],
            current["response_format"]
        );
        assert_eq!(candidate.body["max_tokens"], 123);
        assert_eq!(candidate.body["tool_choice"], "none");
        assert!(!candidate.exec_adapted);
        assert!(
            cache
                .candidate("chat", &current, UpstreamWireApi::AnthropicMessages)
                .is_none()
        );
    }

    #[test]
    fn cache_expires_and_evicts_by_entries_and_total_bytes_and_rejects_oversized_replacement() {
        let mut cache = RequestCache::default();
        let original = json!({"input":[{"content":"old"}],"tools":[{"type":"function"}]});
        let current = json!({"input":[{"content":"old"},{"content":"summary"}]});
        cache.remember("session".into(), &original, false);
        cache.entries.get_mut("session").unwrap().updated = Instant::now() - CACHE_TTL;
        assert!(
            cache
                .candidate("session", &current, UpstreamWireApi::Responses)
                .is_none()
        );
        for index in 0..30 {
            cache.remember(index.to_string(), &original, false);
        }
        assert_eq!(cache.entries.len(), MAX_CACHE_ENTRIES);
        assert!(!cache.entries.contains_key("0"));
        assert!(!cache.entries.contains_key("session"));
        let near_limit = json!({"input":"x".repeat(MAX_REQUEST_BYTES - 128)});
        for index in 0..5 {
            cache.remember(format!("large-{index}"), &near_limit, false);
        }
        assert!(
            cache
                .entries
                .values()
                .map(|entry| entry.bytes)
                .sum::<usize>()
                <= MAX_CACHE_BYTES
        );
        assert!(!cache.entries.contains_key("large-0"));
        cache.remember(
            "large-4".into(),
            &json!({"input":"x".repeat(MAX_REQUEST_BYTES)}),
            false,
        );
        assert!(!cache.entries.contains_key("large-4"));
    }

    #[tokio::test]
    async fn final_only_responses_stream_delivers_items_before_completed_and_preserves_usage() {
        let final_response = response();
        let bytes = frame(json!({"type":"response.completed","response":final_response}));
        let normalized = validate(bytes, UpstreamWireApi::Responses, true)
            .await
            .unwrap();
        let text = String::from_utf8(normalized).unwrap();
        let events: Vec<Value> = text
            .lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .filter(|data| *data != "[DONE]")
            .map(|data| serde_json::from_str(data).unwrap())
            .collect();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["type"], "response.output_item.done");
        assert_eq!(events[0]["output_index"], 0);
        assert_eq!(events[0]["item"], final_response["output"][0]);
        assert_eq!(events[1]["type"], "response.completed");
        assert_eq!(events[1]["response"], final_response);
        let consumed: Vec<_> = events
            .iter()
            .filter(|event| event["type"] == "response.output_item.done")
            .map(|event| event["item"].clone())
            .collect();
        assert_eq!(
            consumed,
            final_response["output"].as_array().unwrap().clone()
        );
    }

    #[tokio::test]
    async fn complete_responses_stream_validates_deltas_and_normalizes_chunked_crlf() {
        let events = [
            json!({"type":"response.created","response":{"id":"resp_summary","status":"in_progress","output":[]}}),
            json!({"type":"response.output_item.added","output_index":0,
                "item":{"id":"msg_summary","type":"message","role":"assistant","content":[]}}),
            json!({"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":"sum"}),
            json!({"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":"mary"}),
            json!({"type":"response.output_text.done","output_index":0,"content_index":0,"text":"summary"}),
            json!({"type":"response.content_part.done","output_index":0,"content_index":0,
                "part":{"type":"output_text","text":"summary"}}),
            json!({"type":"response.output_item.done","output_index":0,"item":message("summary")}),
            json!({"type":"response.completed","response":response()}),
        ];
        let mut bytes = events.into_iter().flat_map(frame).collect::<Vec<_>>();
        bytes.extend_from_slice(b"data: [DONE]\n\n");
        let crlf = String::from_utf8(bytes)
            .unwrap()
            .replace('\n', "\r\n")
            .into_bytes();
        let chunks = crlf.chunks(7).map(|chunk| Ok::<_, ()>(chunk.to_vec()));
        let normalized = read_summary(
            futures_util::stream::iter(chunks),
            UpstreamWireApi::Responses,
            true,
            Duration::from_secs(1),
        )
        .await
        .unwrap();
        assert!(
            String::from_utf8(normalized)
                .unwrap()
                .contains("response.output_item.done")
        );
    }

    #[tokio::test]
    async fn summary_rejects_missing_ids_empty_nontext_tools_errors_and_truncation() {
        let mut missing_id = response();
        missing_id.as_object_mut().unwrap().remove("id");
        let mut incomplete = response();
        incomplete["status"] = json!("incomplete");
        let mut empty = response();
        empty["output"] = json!([message("  ")]);
        let mut tool = response();
        tool["output"]
            .as_array_mut()
            .unwrap()
            .push(json!({"type":"function_call","name":"run"}));
        let mut refusal = response();
        refusal["output"][0]["content"] = json!([{"type":"refusal","refusal":"no"}]);
        let mut wrong_role = response();
        wrong_role["output"][0]["role"] = json!("user");
        let mut error = response();
        error["error"] = json!({"message":"failed"});
        let mut incomplete_item = response();
        incomplete_item["output"][0]["status"] = json!("incomplete");
        for invalid in [
            missing_id,
            incomplete,
            empty,
            tool,
            refusal,
            wrong_role,
            error,
            incomplete_item,
        ] {
            assert!(
                validate(
                    invalid.to_string().into_bytes(),
                    UpstreamWireApi::Responses,
                    false
                )
                .await
                .is_err()
            );
            assert!(
                validate(
                    frame(json!({"type":"response.completed","response":invalid})),
                    UpstreamWireApi::Responses,
                    true
                )
                .await
                .is_err()
            );
        }
        let completed = frame(json!({"type":"response.completed","response":response()}));
        let mut truncated = completed.clone();
        truncated.pop();
        assert!(
            validate(truncated, UpstreamWireApi::Responses, true)
                .await
                .is_err()
        );
        for bytes in [
            Vec::new(),
            b"data: {invalid}\n\n".to_vec(),
            b"data: [DONE]\n\n".to_vec(),
            frame(
                json!({"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":"summary"}),
            ),
        ] {
            assert!(
                validate(bytes, UpstreamWireApi::Responses, true)
                    .await
                    .is_err()
            );
        }
    }

    #[tokio::test]
    async fn summary_rejects_conflicting_events_tools_and_terminal_states() {
        let completed = frame(json!({"type":"response.completed","response":response()}));
        for event in [
            json!({"type":"response.output_item.added","output_index":0,"item":{"type":"web_search_call"}}),
            json!({"type":"response.function_call_arguments.delta","delta":"{}"}),
            json!({"type":"response.failed","response":{"id":"resp_summary","status":"failed"}}),
            json!({"type":"response.incomplete","response":{"id":"resp_summary","status":"incomplete"}}),
            json!({"type":"error","error":{"message":"failed"}}),
            json!({"type":"response.created","response":{"id":"other","status":"in_progress","output":[]}}),
            json!({"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":"different"}),
            json!({"type":"response.output_item.done","output_index":0,"item":message("different")}),
            json!({"type":"response.content_part.done","output_index":0,"content_index":0,
                "part":{"type":"output_text","text":"different"}}),
        ] {
            let mut bytes = frame(event);
            bytes.extend_from_slice(&completed);
            assert!(
                validate(bytes, UpstreamWireApi::Responses, true)
                    .await
                    .is_err()
            );
        }
        for suffix in [
            completed.clone(),
            frame(
                json!({"type":"response.failed","response":{"id":"resp_summary","status":"failed"}}),
            ),
            frame(
                json!({"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":"late"}),
            ),
        ] {
            let mut bytes = completed.clone();
            bytes.extend_from_slice(&suffix);
            assert!(
                validate(bytes, UpstreamWireApi::Responses, true)
                    .await
                    .is_err()
            );
        }
    }

    #[tokio::test]
    async fn chat_summary_requires_single_text_stop_without_tools_or_conflicting_finish() {
        let chat = json!({"choices":[{"finish_reason":"stop","message":{"role":"assistant","content":"summary"}}]});
        assert!(
            validate(
                chat.to_string().into_bytes(),
                UpstreamWireApi::ChatCompletions,
                false
            )
            .await
            .is_ok()
        );
        for invalid in [
            json!({"choices":[{"finish_reason":"length","message":{"content":"summary"}}]}),
            json!({"choices":[{"finish_reason":"stop","message":{"content":"summary","tool_calls":[{}]}}]}),
            json!({"choices":[{"finish_reason":"stop","message":{"content":"summary","function_call":{}}}]}),
            json!({"choices":[{"finish_reason":"stop","message":{"content":" "}}]}),
        ] {
            assert!(
                validate(
                    invalid.to_string().into_bytes(),
                    UpstreamWireApi::ChatCompletions,
                    false
                )
                .await
                .is_err()
            );
        }
        let mut bytes = frame(
            json!({"choices":[{"index":0,"delta":{"role":"assistant","content":"summary"},"finish_reason":null}]}),
        );
        bytes.extend(frame(
            json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}),
        ));
        bytes.extend(frame(json!({"choices":[],"usage":{"total_tokens":100}})));
        bytes.extend_from_slice(b"data: [DONE]\n\n");
        assert_eq!(
            validate(bytes.clone(), UpstreamWireApi::ChatCompletions, true)
                .await
                .unwrap(),
            bytes
        );
        for event in [
            json!({"choices":[{"index":0,"delta":{"tool_calls":[{}]},"finish_reason":null}]}),
            json!({"choices":[{"index":0,"delta":{"function_call":{}},"finish_reason":null}]}),
            json!({"choices":[{"index":0,"delta":{"content":"summary"},"finish_reason":"length"}]}),
        ] {
            assert!(
                validate(frame(event), UpstreamWireApi::ChatCompletions, true)
                    .await
                    .is_err()
            );
        }
    }

    #[tokio::test]
    async fn progressing_chunks_can_exceed_idle_timeout_in_total_but_stalls_fail() {
        let bytes = frame(json!({"type":"response.completed","response":response()}));
        let chunks: Vec<_> = bytes.chunks(bytes.len() / 4).map(Vec::from).collect();
        let stream = futures_util::stream::iter(chunks).then(|chunk| async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            Ok::<_, ()>(chunk)
        });
        let started = tokio::time::Instant::now();
        assert!(
            read_summary(
                stream,
                UpstreamWireApi::Responses,
                true,
                Duration::from_millis(100)
            )
            .await
            .is_ok()
        );
        assert!(started.elapsed() >= Duration::from_millis(120));
        let stalled = futures_util::stream::pending::<Result<Vec<u8>, ()>>();
        assert_eq!(
            read_summary(
                stalled,
                UpstreamWireApi::Responses,
                true,
                Duration::from_millis(10)
            )
            .await
            .unwrap_err(),
            "summary_timeout"
        );
        let empty = futures_util::stream::repeat_with(|| Ok::<_, ()>(Vec::<u8>::new())).then(
            |chunk| async move {
                tokio::time::sleep(Duration::from_millis(5)).await;
                chunk
            },
        );
        assert_eq!(
            read_summary(
                empty,
                UpstreamWireApi::Responses,
                true,
                Duration::from_millis(20)
            )
            .await
            .unwrap_err(),
            "summary_timeout"
        );
    }

    #[tokio::test]
    async fn summary_has_exact_byte_bound_and_rejects_invalid_frames_before_waiting_for_eof() {
        let mut padded = response().to_string().into_bytes();
        padded.resize(MAX_SUMMARY_BYTES, b' ');
        assert!(
            validate(padded.clone(), UpstreamWireApi::Responses, false)
                .await
                .is_ok()
        );
        padded.push(b' ');
        assert_eq!(
            validate(padded, UpstreamWireApi::Responses, false)
                .await
                .unwrap_err(),
            "summary_size"
        );
        let invalid = futures_util::stream::iter([Ok::<_, ()>(b"data: {broken}\n\n".to_vec())])
            .chain(futures_util::stream::pending());
        assert_eq!(
            read_summary(
                invalid,
                UpstreamWireApi::Responses,
                true,
                Duration::from_millis(50)
            )
            .await
            .unwrap_err(),
            "invalid_summary_or_tool_call"
        );
    }

    #[tokio::test]
    async fn buffer_rejects_http_errors_and_updates_normalized_body_headers() {
        for status in [400, 422] {
            let mut upstream = http::Response::new(b"upstream error".to_vec());
            *upstream.status_mut() = http::StatusCode::from_u16(status).unwrap();
            assert_eq!(
                buffer_summary(
                    reqwest::Response::from(upstream),
                    UpstreamWireApi::Responses,
                    true
                )
                .await
                .err(),
                Some("upstream_status")
            );
        }
        let bytes = frame(json!({"type":"response.completed","response":response()}));
        let upstream = http::Response::builder()
            .status(200)
            .header("content-type", "text/event-stream")
            .header("content-length", bytes.len())
            .header("x-request-id", "upstream-id")
            .body(bytes)
            .unwrap();
        let buffered = buffer_summary(
            reqwest::Response::from(upstream),
            UpstreamWireApi::Responses,
            false,
        )
        .await
        .unwrap();
        assert!(buffered.headers().get("content-length").is_none());
        assert_eq!(buffered.headers()["x-request-id"], "upstream-id");
        assert!(
            buffered
                .text()
                .await
                .unwrap()
                .contains("response.output_item.done")
        );
    }
}
