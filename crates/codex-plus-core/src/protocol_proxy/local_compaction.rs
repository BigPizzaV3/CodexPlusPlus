use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::ProxySessionHeaders;
use crate::settings::RelayProfile;

const MAX_TOOLS_BYTES: usize = 8 * 1024 * 1024;
const MAX_CACHE_BYTES: usize = 32 * 1024 * 1024;
const MAX_CACHE_ENTRIES: usize = 128;

pub(super) struct ToolContext {
    tools: Vec<Value>,
    bytes: usize,
}

impl ToolContext {
    pub fn from_request(body: &Value) -> Option<Self> {
        let tools = body.get("tools")?.as_array()?;
        if tools.is_empty() {
            return None;
        }
        let bytes = serde_json::to_vec(tools).ok()?.len();
        (bytes <= MAX_TOOLS_BYTES).then(|| Self {
            tools: tools.clone(),
            bytes,
        })
    }
}

struct Entry {
    context: ToolContext,
    exec_adapted: bool,
    updated: Instant,
}

pub(super) struct Candidate {
    pub body: Value,
    pub exec_adapted: bool,
}

#[derive(Default)]
struct ToolCache {
    entries: BTreeMap<String, Entry>,
}

fn cache() -> &'static Mutex<ToolCache> {
    static CACHE: OnceLock<Mutex<ToolCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(ToolCache::default()))
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
            body.get("model")
        ])
        .to_string(),
    )
}

impl ToolCache {
    fn remember(&mut self, key: String, context: Option<ToolContext>, exec_adapted: bool) {
        self.entries.remove(&key);
        let Some(context) = context else {
            return;
        };
        self.entries.insert(
            key,
            Entry {
                context,
                exec_adapted,
                updated: Instant::now(),
            },
        );
        while self.entries.len() > MAX_CACHE_ENTRIES
            || self
                .entries
                .values()
                .map(|entry| entry.context.bytes)
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

    fn candidate(&mut self, key: &str, body: &Value) -> Option<Candidate> {
        let entry = self.entries.get_mut(key)?;
        // 只恢复声明；历史、系统提示和全部生成参数始终来自本次请求。
        let mut candidate = body.clone();
        let object = candidate.as_object_mut()?;
        object.insert("tools".into(), Value::Array(entry.context.tools.clone()));
        object.insert("tool_choice".into(), json!("none"));
        object.insert("parallel_tool_calls".into(), json!(false));
        entry.updated = Instant::now();
        Some(Candidate {
            body: candidate,
            exec_adapted: entry.exec_adapted,
        })
    }
}

pub(super) fn remember(key: String, context: Option<ToolContext>, exec_adapted: bool) {
    if let Ok(mut cache) = cache().lock() {
        cache.remember(key, context, exec_adapted);
    }
}

pub(super) fn candidate(key: &str, body: &Value) -> Option<Candidate> {
    cache().lock().ok()?.candidate(key, body)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn tool_identity_isolates_provider_credentials_protocol_model_and_session() {
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
            let changed_key = key(&changed, &body, headers).unwrap();
            assert_ne!(changed_key, original);
            assert!(!changed_key.contains("different-private-key"));
            assert!(!changed_key.contains("Bearer changed"));
        }
        assert_ne!(
            key(&relay, &json!({"model":"other"}), headers).unwrap(),
            original
        );
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
        assert_eq!(
            key(
                &relay,
                &json!({"model":"model","prompt_cache_key":"new"}),
                headers
            )
            .unwrap(),
            original
        );
        assert_eq!(
            key(&relay, &json!({"model":"model"}), headers).unwrap(),
            original
        );
    }

    #[test]
    fn candidate_restores_only_tools_without_matching_or_reusing_old_context() {
        let mut cache = ToolCache::default();
        let old = json!({"input":[{"content":"old private history"}],"instructions":"old system",
            "previous_response_id":"resp_old","prompt_cache_key":"old",
            "tools":[{"type":"function","name":"run"}],
            "tool_choice":"required","parallel_tool_calls":true,"temperature":1});
        cache.remember("session".into(), ToolContext::from_request(&old), true);
        let current = json!({"input":[{"content":"trimmed current history"},{"content":"summary"}],
            "instructions":"new system","previous_response_id":"resp_new","prompt_cache_key":"new",
            "tools":[],"tool_choice":"required","parallel_tool_calls":true,
            "reasoning":{"effort":"low"},"max_output_tokens":128,
            "text":{"format":{"type":"json_schema","name":"checkpoint","schema":{"type":"object"}}},
            "metadata":{"new":true},"client_metadata":{"turn":"new"},"stream":true,
            "temperature":0,"store":false,"truncation":"disabled","service_tier":"priority"});
        let candidate = cache.candidate("session", &current).unwrap();
        let mut expected = current.clone();
        expected["tools"] = old["tools"].clone();
        expected["tool_choice"] = json!("none");
        expected["parallel_tool_calls"] = json!(false);
        assert_eq!(candidate.body, expected);
        assert!(candidate.exec_adapted);
        assert!(cache.candidate("other-session", &current).is_none());
        let mut absent_controls = current.clone();
        for name in ["reasoning", "metadata", "temperature", "stream"] {
            absent_controls.as_object_mut().unwrap().remove(name);
        }
        let candidate = cache.candidate("session", &absent_controls).unwrap();
        for name in ["reasoning", "metadata", "temperature", "stream"] {
            assert!(candidate.body.get(name).is_none());
        }
    }

    #[test]
    fn cache_counts_only_tools_and_replaces_or_clears_the_latest_declarations() {
        let mut cache = ToolCache::default();
        let mut body = json!({"tools":[{"type":"function","name":"old"}],
            "input":[{"content":"x".repeat(MAX_TOOLS_BYTES + 1)}]});
        let context = ToolContext::from_request(&body).unwrap();
        assert!(context.bytes < 100);
        cache.remember("session".into(), Some(context), false);
        body["tools"][0]["name"] = json!("new");
        cache.remember("session".into(), ToolContext::from_request(&body), true);
        let candidate = cache.candidate("session", &json!({})).unwrap();
        assert_eq!(candidate.body["tools"], body["tools"]);
        assert!(candidate.exec_adapted);
        for tools in [
            json!([]),
            Value::Null,
            json!([{"description":"x".repeat(MAX_TOOLS_BYTES + 1)}]),
        ] {
            body["tools"] = json!([{"type":"function","name":"current"}]);
            cache.remember("session".into(), ToolContext::from_request(&body), false);
            body["tools"] = tools;
            cache.remember("session".into(), ToolContext::from_request(&body), false);
            assert!(cache.candidate("session", &json!({})).is_none());
        }
    }

    #[test]
    fn cache_evicts_least_recently_used_tools_at_capacity() {
        let mut cache = ToolCache::default();
        let body = json!({"tools":[{"type":"function","name":"run"}]});
        for index in 0..MAX_CACHE_ENTRIES {
            cache.remember(index.to_string(), ToolContext::from_request(&body), false);
        }
        assert!(cache.candidate("0", &json!({})).is_some());
        cache.remember("new".into(), ToolContext::from_request(&body), false);
        assert_eq!(cache.entries.len(), MAX_CACHE_ENTRIES);
        assert!(cache.candidate("0", &json!({})).is_some());
        assert!(cache.candidate("1", &json!({})).is_none());
        let context = ToolContext {
            tools: vec![json!({"type":"function","name":"large"})],
            bytes: MAX_CACHE_BYTES + 1,
        };
        cache.remember("large".into(), Some(context), false);
        assert!(cache.entries.is_empty());
    }
}
