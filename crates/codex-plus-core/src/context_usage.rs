use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_ENTRIES: usize = 128;

#[derive(Debug, Clone)]
struct RequestBoundary {
    model: String,
    message_count: usize,
    boundary_message_index: usize,
    boundary_hash: String,
    created_at_ms: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct ContextUsageAnchor {
    pub(crate) model: String,
    pub(crate) context_used: u64,
    pub(crate) covered_message_count: usize,
    pub(crate) boundary_message_index: usize,
    pub(crate) boundary_hash: String,
    pub(crate) observed_at_ms: u64,
}

#[derive(Debug)]
enum UsageEntry {
    Pending(RequestBoundary),
    Anchor(ContextUsageAnchor),
}

static ENTRIES: LazyLock<Mutex<HashMap<String, UsageEntry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// 记录一次待发送请求的边界。上游 usage 到达后，这份边界能把
/// `total_tokens` 精确关联到“请求历史 + assistant 输出”。
pub(crate) fn remember_request_boundary(thread_id: Option<&str>, model: &str, messages: &[Value]) {
    let Some(thread_id) = thread_id.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    let model = model.trim();
    if model.is_empty() {
        return;
    }

    let boundary_message_index = messages.len().saturating_sub(1);
    let boundary_hash = messages
        .last()
        .map(message_hash)
        .unwrap_or_else(|| "empty".to_string());
    let entry = UsageEntry::Pending(RequestBoundary {
        model: model.to_string(),
        message_count: messages.len(),
        boundary_message_index,
        boundary_hash,
        created_at_ms: now_ms(),
    });
    insert_entry(normalized_thread_id(thread_id), entry);
}

/// 将上游 usage 合并到最近一次请求边界，供下一轮 VLM 预处理使用。
pub(crate) fn record_usage(thread_id: Option<&str>, model: &str, usage: &Value) {
    let Some(thread_id) = thread_id.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    let model = model.trim();
    if model.is_empty() {
        return;
    }
    let Some(context_used) = usage
        .get("total_tokens")
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
    else {
        return;
    };

    let mut entries = ENTRIES.lock().unwrap_or_else(|error| error.into_inner());
    let key = normalized_thread_id(thread_id);
    let Some(UsageEntry::Pending(boundary)) = entries.get_mut(&key) else {
        return;
    };
    if boundary.model != model {
        return;
    }

    let anchor = ContextUsageAnchor {
        model: boundary.model.clone(),
        context_used,
        // Chat/Responses 的下一轮历史通常追加一条 assistant 消息。
        covered_message_count: boundary.message_count.saturating_add(1),
        boundary_message_index: boundary.boundary_message_index,
        boundary_hash: boundary.boundary_hash.clone(),
        observed_at_ms: now_ms(),
    };
    entries.insert(key, UsageEntry::Anchor(anchor));
}

pub(crate) fn anchor_for(thread_id: Option<&str>, model: &str) -> Option<ContextUsageAnchor> {
    let thread_id = thread_id.map(str::trim).filter(|value| !value.is_empty())?;
    let model = model.trim();
    if model.is_empty() {
        return None;
    }
    let entries = ENTRIES.lock().unwrap_or_else(|error| error.into_inner());
    match entries.get(&normalized_thread_id(thread_id)) {
        Some(UsageEntry::Anchor(anchor)) if anchor.model == model => Some(anchor.clone()),
        _ => None,
    }
}

pub(crate) fn message_matches_anchor(message: &Value, anchor: &ContextUsageAnchor) -> bool {
    message_hash(message) == anchor.boundary_hash
}

fn insert_entry(key: String, entry: UsageEntry) {
    let mut entries = ENTRIES.lock().unwrap_or_else(|error| error.into_inner());
    entries.insert(key, entry);
    while entries.len() > MAX_ENTRIES {
        let oldest = entries
            .iter()
            .min_by_key(|(_, entry)| entry.observed_at_ms())
            .map(|(key, _)| key.clone());
        let Some(oldest) = oldest else {
            break;
        };
        entries.remove(&oldest);
    }
}

impl UsageEntry {
    fn observed_at_ms(&self) -> u64 {
        match self {
            Self::Pending(boundary) => boundary.created_at_ms,
            Self::Anchor(anchor) => anchor.observed_at_ms,
        }
    }
}

fn message_hash(message: &Value) -> String {
    let bytes = serde_json::to_vec(message).unwrap_or_default();
    Sha256::digest(&bytes)
        .iter()
        .take(16)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn normalized_thread_id(thread_id: &str) -> String {
    thread_id
        .trim()
        .trim_start_matches("local:")
        .trim()
        .to_ascii_lowercase()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn usage_attaches_to_the_latest_request_boundary() {
        let thread_id = "usage-anchor-test";
        let messages = vec![json!({"role": "user", "content": "hello"})];
        remember_request_boundary(Some(thread_id), "fixture-model", &messages);
        record_usage(
            Some(thread_id),
            "fixture-model",
            &json!({"total_tokens": 1234}),
        );

        let anchor = anchor_for(Some(thread_id), "fixture-model").unwrap();
        assert_eq!(anchor.context_used, 1234);
        assert_eq!(anchor.covered_message_count, 2);
        assert_eq!(anchor.boundary_message_index, 0);
        assert!(message_matches_anchor(&messages[0], &anchor));
    }

    #[test]
    fn model_mismatch_or_missing_usage_does_not_create_an_anchor() {
        let thread_id = "usage-model-mismatch-test";
        let messages = vec![json!({"role": "user", "content": "hello"})];
        remember_request_boundary(Some(thread_id), "model-a", &messages);
        record_usage(Some(thread_id), "model-b", &json!({"total_tokens": 100}));
        assert!(anchor_for(Some(thread_id), "model-b").is_none());

        record_usage(Some(thread_id), "model-a", &json!({"total_tokens": 0}));
        assert!(anchor_for(Some(thread_id), "model-a").is_none());
    }
}
