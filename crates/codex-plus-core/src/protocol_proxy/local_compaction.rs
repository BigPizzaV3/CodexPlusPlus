use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use super::ProxySessionHeaders;
use crate::settings::RelayProfile;

const MAX_CONTEXT_BYTES: usize = 8 * 1024 * 1024;
const MAX_CACHE_BYTES: usize = 32 * 1024 * 1024;
const MAX_CACHE_ENTRIES: usize = 128;

// 不保存旧历史、响应状态、输出上限或本轮追踪信息。
const CONTEXT_FIELDS: &[&str] = &[
    "instructions",
    "tools",
    "reasoning",
    "include",
    "text",
    "prompt_cache_key",
    "prompt_cache_retention",
    "store",
    "truncation",
    "service_tier",
    "temperature",
    "top_p",
];

pub(super) struct RequestContext {
    fields: Map<String, Value>,
    input_prefix: Option<(usize, [u8; 32])>,
    bytes: usize,
}

impl RequestContext {
    pub fn from_request(body: &Value) -> Option<Self> {
        let fields: Map<String, Value> = CONTEXT_FIELDS
            .iter()
            .filter_map(|&name| body.get(name).map(|value| (name.into(), value.clone())))
            .collect();
        if fields.is_empty() {
            return None;
        }
        let bytes = serde_json::to_vec(&fields).ok()?.len();
        (bytes <= MAX_CONTEXT_BYTES).then(|| Self {
            fields,
            input_prefix: body
                .get("input")
                .and_then(Value::as_array)
                .and_then(|items| input_digest(items).map(|hash| (items.len(), hash))),
            bytes,
        })
    }
}

fn input_digest(items: &[Value]) -> Option<[u8; 32]> {
    Some(Sha256::digest(serde_json::to_vec(items).ok()?).into())
}

struct Entry {
    context: RequestContext,
    exec_adapted: bool,
    updated: Instant,
}

pub(super) struct Candidate {
    pub body: Value,
    pub exec_adapted: bool,
    pub context_fields_match: bool,
    pub history_prefix_matches: Option<bool>,
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
            relay.uses_no_auth(),
            format!("{:x}", Sha256::digest(relay.api_key.as_bytes())),
            format!("{headers_digest:x}"),
            session_kind,
            session,
            body.get("model")
        ])
        .to_string(),
    )
}

impl RequestCache {
    fn remember(&mut self, key: String, context: Option<RequestContext>, exec_adapted: bool) {
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
        // 显式的新指令/参数优先；仅工具声明已知是本地摘要主动清空的字段。
        let mut candidate = body.clone();
        let object = candidate.as_object_mut()?;
        let mut restored_tools = false;
        for (name, value) in &entry.context.fields {
            let cleared_tools = name == "tools"
                && (matches!(object.get(name), None | Some(Value::Null))
                    || object
                        .get(name)
                        .and_then(Value::as_array)
                        .is_some_and(Vec::is_empty));
            if cleared_tools || !object.contains_key(name) {
                object.insert(name.clone(), value.clone());
                restored_tools |= name == "tools"
                    && value.as_array().is_some_and(|tools| !tools.is_empty());
            }
        }
        if restored_tools {
            object.insert("tool_choice".into(), json!("none"));
            object.insert("parallel_tool_calls".into(), json!(false));
        }
        let context_fields_match = entry
            .context
            .fields
            .iter()
            .all(|(name, value)| object.get(name) == Some(value));
        let history_prefix_matches = entry.context.input_prefix.map(|(len, hash)| {
            object
                .get("input")
                .and_then(Value::as_array)
                .and_then(|items| items.get(..len))
                .and_then(input_digest)
                == Some(hash)
        });
        entry.updated = Instant::now();
        Some(Candidate {
            body: candidate,
            exec_adapted: entry.exec_adapted && restored_tools,
            context_fields_match,
            history_prefix_matches,
        })
    }
}

pub(super) fn remember(key: String, context: Option<RequestContext>, exec_adapted: bool) {
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
        body["client_metadata"]["x-codex-turn-metadata"] = json!(
            json!({"request_kind":"compaction","compaction":{"implementation":"responses"}})
                .to_string()
        );
        assert!(!is_local_request(&body, canonical));
        body.as_object_mut().unwrap().remove("client_metadata");
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
        let mut changed = relay.clone();
        changed.relay_mode = crate::settings::RelayMode::PureApi;
        changed.no_auth = true;
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
    fn candidate_keeps_explicit_current_fields_and_never_reuses_old_history_or_response_state() {
        let mut cache = RequestCache::default();
        let old = json!({"input":[{"content":"old private history"}],"instructions":"old system",
            "previous_response_id":"resp_old","prompt_cache_key":"old",
            "tools":[{"type":"function","name":"run"}],
            "tool_choice":"required","parallel_tool_calls":true,"temperature":1});
        cache.remember("session".into(), RequestContext::from_request(&old), true);
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
        assert!(!candidate.context_fields_match);
        assert_eq!(candidate.history_prefix_matches, Some(false));
        assert!(cache.candidate("other-session", &current).is_none());
        let mut absent_controls = current.clone();
        for name in ["reasoning", "metadata", "temperature", "stream"] {
            absent_controls.as_object_mut().unwrap().remove(name);
        }
        let candidate = cache.candidate("session", &absent_controls).unwrap();
        assert_eq!(candidate.body["temperature"], 1);
        for name in ["reasoning", "metadata", "stream"] {
            assert!(candidate.body.get(name).is_none());
        }
    }

    #[test]
    fn candidate_fills_missing_prefix_fields_but_preserves_explicit_empty_values_and_latest_tools()
    {
        let mut cache = RequestCache::default();
        let old = json!({
            "instructions":"Ordinary rules.", "tools":[{"type":"function","name":"old"}],
            "input":[{"role":"user","content":"Task"}],
            "reasoning":{"effort":"high"}, "prompt_cache_key":"ordinary-key",
            "include":["reasoning.encrypted_content"],
            "text":{"format":{"type":"json_schema","name":"old","schema":{"type":"object"}}},
            "stream":false, "max_output_tokens":9999, "previous_response_id":"old",
            "metadata":{"old":true}, "client_metadata":{"old":true},
            "conversation_id":"old", "conversation":"old"
        });
        cache.remember("session".into(), RequestContext::from_request(&old), true);
        let current = json!({
            "input":[{"role":"user","content":"Task"},{"role":"assistant","content":"Fresh result"},
                {"role":"user","content":"Summary"}],
            "tools":[], "stream":true
        });
        let candidate = cache.candidate("session", &current).unwrap();
        for name in CONTEXT_FIELDS {
            assert_eq!(candidate.body.get(*name), old.get(*name));
        }
        assert_eq!(candidate.body["input"], current["input"]);
        assert_eq!(candidate.body["stream"], true);
        assert!(candidate.context_fields_match);
        assert_eq!(candidate.history_prefix_matches, Some(true));
        for name in [
            "metadata",
            "client_metadata",
            "previous_response_id",
            "conversation",
            "conversation_id",
            "max_output_tokens",
        ] {
            assert!(candidate.body.get(name).is_none());
        }
        for value in [json!(""), Value::Null, json!({})] {
            let mut explicit = current.clone();
            for name in ["instructions", "reasoning", "text", "prompt_cache_key"] {
                explicit[name] = value.clone();
            }
            explicit["tools"] = json!([{"type":"function","name":"current"}]);
            let candidate = cache.candidate("session", &explicit).unwrap();
            for name in [
                "instructions",
                "reasoning",
                "text",
                "prompt_cache_key",
                "tools",
            ] {
                assert_eq!(candidate.body[name], explicit[name]);
            }
            assert!(!candidate.context_fields_match);
            assert!(
                !candidate.exec_adapted,
                "current tools cannot inherit old exec state"
            );
        }
    }

    #[test]
    fn candidate_preserves_current_tool_controls_unless_nonempty_tools_are_restored() {
        for old_tools in [None, Some(Value::Null), Some(json!([]))] {
            let mut cache = RequestCache::default();
            let mut old = json!({"instructions":"Ordinary rules."});
            if let Some(tools) = old_tools {
                old["tools"] = tools;
            }
            cache.remember("session".into(), RequestContext::from_request(&old), true);
            for current_tools in [None, Some(Value::Null), Some(json!([]))] {
                for controls in [json!({}), json!({
                    "tool_choice":"required", "parallel_tool_calls":true
                })] {
                    let mut current = controls;
                    current["instructions"] = old["instructions"].clone();
                    current["input"] = json!([{"role":"user","content":"Summary"}]);
                    if let Some(tools) = &current_tools {
                        current["tools"] = tools.clone();
                    }
                    let candidate = cache.candidate("session", &current).unwrap();
                    for name in ["tool_choice", "parallel_tool_calls", "input"] {
                        assert_eq!(candidate.body.get(name), current.get(name));
                    }
                    assert!(!candidate.exec_adapted);
                }
            }
        }
        let mut cache = RequestCache::default();
        cache.remember(
            "session".into(),
            RequestContext::from_request(&json!({
                "tools":[{"type":"function","name":"old"}]
            })),
            true,
        );
        let current = json!({
            "tools":[{"type":"function","name":"current"}],
            "tool_choice":"required", "parallel_tool_calls":true
        });
        let candidate = cache.candidate("session", &current).unwrap();
        assert_eq!(candidate.body, current);
        assert!(!candidate.exec_adapted);
    }

    #[test]
    fn cache_counts_context_not_history_and_replaces_or_clears_the_latest_snapshot() {
        let mut cache = RequestCache::default();
        let mut body = json!({"tools":[{"type":"function","name":"old"}],
            "input":[{"content":"x".repeat(MAX_CONTEXT_BYTES + 1)}]});
        let context = RequestContext::from_request(&body).unwrap();
        assert!(context.bytes < 100);
        cache.remember("session".into(), Some(context), false);
        body["tools"][0]["name"] = json!("new");
        cache.remember("session".into(), RequestContext::from_request(&body), true);
        let candidate = cache.candidate("session", &json!({})).unwrap();
        assert_eq!(candidate.body["tools"], body["tools"]);
        assert!(candidate.exec_adapted);
        for tools in [json!([]), Value::Null] {
            cache.remember(
                "session".into(),
                RequestContext::from_request(&json!({"tools":tools})),
                false,
            );
            assert_eq!(
                cache.candidate("session", &json!({})).unwrap().body["tools"],
                tools
            );
        }
        for replacement in [
            json!({}),
            json!({"instructions":"x".repeat(MAX_CONTEXT_BYTES + 1)}),
        ] {
            cache.remember("session".into(), RequestContext::from_request(&body), false);
            cache.remember(
                "session".into(),
                RequestContext::from_request(&replacement),
                false,
            );
            assert!(cache.candidate("session", &json!({})).is_none());
        }
    }

    #[test]
    fn cache_evicts_least_recently_used_context_at_capacity() {
        let mut cache = RequestCache::default();
        let body = json!({"tools":[{"type":"function","name":"run"}]});
        for index in 0..MAX_CACHE_ENTRIES {
            cache.remember(
                index.to_string(),
                RequestContext::from_request(&body),
                false,
            );
        }
        assert!(cache.candidate("0", &json!({})).is_some());
        cache.remember("new".into(), RequestContext::from_request(&body), false);
        assert_eq!(cache.entries.len(), MAX_CACHE_ENTRIES);
        assert!(cache.candidate("0", &json!({})).is_some());
        assert!(cache.candidate("1", &json!({})).is_none());
        let context = RequestContext {
            fields: Map::new(),
            input_prefix: None,
            bytes: MAX_CACHE_BYTES + 1,
        };
        cache.remember("large".into(), Some(context), false);
        assert!(cache.entries.is_empty());
    }
}
