use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;

use anyhow::Context;
use serde::{Deserialize, Serialize};

const STATE_FILE: &str = "weixin-connect-state.json";
const MAX_PROCESSED_IDS: usize = 512;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConnectState {
    #[serde(default)]
    pub get_updates_buf: String,
    #[serde(default)]
    pub thread_ids: BTreeMap<String, String>,
    #[serde(default)]
    pub context_tokens: BTreeMap<String, String>,
    #[serde(default)]
    processed_message_keys: VecDeque<String>,
}

impl ConnectState {
    pub fn is_processed(&self, message_key: &str) -> bool {
        !message_key.is_empty()
            && self
                .processed_message_keys
                .iter()
                .any(|key| key == message_key)
    }

    pub fn mark_processed(&mut self, message_key: impl Into<String>) {
        let message_key = message_key.into();
        if message_key.is_empty() || self.is_processed(&message_key) {
            return;
        }
        self.processed_message_keys.push_back(message_key);
        while self.processed_message_keys.len() > MAX_PROCESSED_IDS {
            self.processed_message_keys.pop_front();
        }
    }
}

#[derive(Debug, Clone)]
pub struct ConnectSessionStore {
    path: PathBuf,
}

impl ConnectSessionStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn default_for_account(account_id: &str) -> Self {
        let account = sanitize_account_id(account_id);
        let file_name = if account == "default" {
            STATE_FILE.to_string()
        } else {
            format!("weixin-connect-state-{account}.json")
        };
        Self::new(crate::paths::default_app_state_dir().join(file_name))
    }

    pub fn load(&self) -> anyhow::Result<ConnectState> {
        match std::fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .with_context(|| format!("无法解析微信连接状态 {}", self.path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(ConnectState::default())
            }
            Err(error) => {
                Err(error).with_context(|| format!("无法读取微信连接状态 {}", self.path.display()))
            }
        }
    }

    pub fn save(&self, state: &ConnectState) -> anyhow::Result<()> {
        let bytes = serde_json::to_vec_pretty(state)?;
        crate::settings::atomic_write(&self.path, &bytes)
    }
}

fn sanitize_account_id(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        return "default".to_string();
    }
    value
        .chars()
        .map(|character| match character {
            '/' | '\\' | ':' | '\0' => '_',
            other => other,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_store_round_trips_and_limits_dedup_history() {
        let temp = tempfile::tempdir().unwrap();
        let store = ConnectSessionStore::new(temp.path().join("state.json"));
        let mut state = ConnectState {
            get_updates_buf: "cursor".to_string(),
            ..ConnectState::default()
        };
        state
            .thread_ids
            .insert("peer".to_string(), "thread".to_string());
        for id in 1..=600 {
            state.mark_processed(format!("message-{id}"));
        }

        store.save(&state).unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(loaded.get_updates_buf, "cursor");
        assert_eq!(
            loaded.thread_ids.get("peer").map(String::as_str),
            Some("thread")
        );
        assert!(!loaded.is_processed("message-1"));
        assert!(loaded.is_processed("message-600"));
    }
}
