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
    pub completed_resets: BTreeMap<String, String>,
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
    published_path: Option<PathBuf>,
}

impl ConnectSessionStore {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            published_path: None,
        }
    }

    pub fn default_for_account(account_id: &str) -> Self {
        Self::in_directory(&crate::paths::default_app_state_dir(), account_id)
    }

    fn in_directory(directory: &std::path::Path, account_id: &str) -> Self {
        let account = account_id.trim();
        let file_name = if account.is_empty() {
            STATE_FILE.to_string()
        } else {
            format!("weixin-connect-state-{account}.json")
        };
        Self {
            path: directory
                .join("weixin-connect-state")
                .join(format!("{}.json", account_key(account))),
            published_path: (account.is_empty()
                || (sanitize_account_id(account) == account && account != "default"))
                .then(|| directory.join(file_name)),
        }
    }

    pub fn load(&self) -> anyhow::Result<ConnectState> {
        match std::fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .with_context(|| format!("无法解析微信连接状态 {}", self.path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if let Some(path) = &self.published_path {
                    match std::fs::read_dir(path.parent().context("状态目录不存在")?) {
                        Ok(entries) => {
                            for entry in entries {
                                let entry = entry?;
                                if Some(entry.file_name().as_os_str()) == path.file_name() {
                                    return serde_json::from_slice(&std::fs::read(entry.path())?)
                                        .context("无法解析已发布的微信连接状态");
                                }
                            }
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) => return Err(error.into()),
                    }
                }
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

pub(super) fn account_key(account: &str) -> String {
    let encoded: String = account
        .trim()
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("account-{encoded}")
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
    fn account_paths_preserve_at_dot_and_uppercase_without_migrating_state() {
        let temp = tempfile::tempdir().unwrap();
        for account in ["Bot.ABC123@im.bot", "Sender.XYZ@im.wechat"] {
            let default = ConnectSessionStore::default_for_account(account);
            let filename = format!("weixin-connect-state-{account}.json");
            assert_eq!(
                default
                    .published_path
                    .as_ref()
                    .unwrap()
                    .file_name()
                    .unwrap()
                    .to_str(),
                Some(filename.as_str())
            );
            assert_eq!(sanitize_account_id(account), account);
            let store = ConnectSessionStore::new(temp.path().join(filename));
            let mut state = ConnectState::default();
            state.get_updates_buf = format!("cursor-{account}");
            state
                .thread_ids
                .insert("peer".into(), format!("thread-{account}"));
            state
                .context_tokens
                .insert("peer".into(), "saved-context".into());
            state.mark_processed("inbound-message");
            store.save(&state).unwrap();
            assert_eq!(store.load().unwrap(), state);
        }
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 2);
        assert_eq!(sanitize_account_id("  "), "default");
        assert_eq!(sanitize_account_id("  Bot/A\\B:C\0  "), "Bot_A_B_C_");
    }

    #[test]
    fn empty_account_preserves_published_thread_and_prefers_new_state_after_save() {
        let temp = tempfile::tempdir().unwrap();
        let published = temp.path().join(STATE_FILE);
        let old_bytes = br#"{"threadIds":{"peer":"original"},"getUpdatesBuf":"cursor"}"#;
        std::fs::write(&published, old_bytes).unwrap();
        let store = ConnectSessionStore::in_directory(temp.path(), "  ");
        let mut state = store.load().unwrap();
        assert_eq!(state.thread_ids["peer"], "original");
        assert_eq!(state.get_updates_buf, "cursor");
        state.get_updates_buf = "next-cursor".into();
        store.save(&state).unwrap();
        assert_eq!(store.load().unwrap(), state);
        assert_eq!(std::fs::read(&published).unwrap(), old_bytes);
        assert!(
            ConnectSessionStore::in_directory(temp.path(), "default")
                .load()
                .unwrap()
                .thread_ids
                .is_empty()
        );
    }

    #[test]
    fn malformed_published_empty_account_state_is_not_silently_replaced() {
        let temp = tempfile::tempdir().unwrap();
        let published = temp.path().join(STATE_FILE);
        std::fs::write(&published, b"not-json").unwrap();
        let store = ConnectSessionStore::in_directory(temp.path(), "");
        assert!(store.load().is_err());
        assert!(!store.path.exists());
        assert_eq!(std::fs::read(&published).unwrap(), b"not-json");
    }

    #[test]
    fn real_account_paths_are_case_sensitive_and_published_threads_remain_readable() {
        let temp = tempfile::tempdir().unwrap();
        let account = "Bot.1@wx.example";
        let published = temp
            .path()
            .join(format!("weixin-connect-state-{account}.json"));
        std::fs::write(&published, br#"{"threadIds":{"peer":"original"}}"#).unwrap();
        let store = ConnectSessionStore::in_directory(temp.path(), account);
        assert_eq!(store.load().unwrap().thread_ids["peer"], "original");
        let lower = ConnectSessionStore::in_directory(temp.path(), "bot.1@wx.example");
        assert_ne!(
            store.path.to_string_lossy().to_lowercase(),
            lower.path.to_string_lossy().to_lowercase()
        );
        assert!(lower.load().unwrap().thread_ids.is_empty());
        let state = store.load().unwrap();
        store.save(&state).unwrap();
        assert_eq!(store.load().unwrap().thread_ids["peer"], "original");
        assert!(published.exists());
        assert_ne!(
            ConnectSessionStore::in_directory(temp.path(), "a/b").path,
            ConnectSessionStore::in_directory(temp.path(), "a_b").path
        );
    }

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
