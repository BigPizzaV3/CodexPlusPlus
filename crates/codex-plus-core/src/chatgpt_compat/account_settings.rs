//! 本地账号的账号级设置。目前只有强调色（`chat_theme`）。
//!
//! 登录 ChatGPT 账号时，Codex 的强调色（推理滑块、开关等用的 `--color-accent`）跟着账号走：
//! 读 `settings/user` 里的 `chat_theme`，在设置里改时发
//! `PATCH /settings/account_user_setting?feature=chat_theme&value=<色>`，并核对应答里回写的值，
//! 对不上就报「Account accent was not saved」并退回原色。
//!
//! 录制里的 `chat_theme` 是录制那个账号自己的（`black`），原样回放会把强调色变成黑色、深色主题下成了白色。
//! 这里改成本机自己记：没改过时是 `default`（和没登录时一样），改了就存下来、原样回写。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// 没改过时的强调色：Codex 自己的默认值。
pub const DEFAULT_CHAT_THEME: &str = "default";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Saved {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    chat_theme: Option<String>,
}

/// 存在状态目录里的账号级设置。
pub struct AccountSettingsStore {
    path: PathBuf,
}

impl AccountSettingsStore {
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    fn load(&self) -> Saved {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// 当前强调色；没改过是 [`DEFAULT_CHAT_THEME`]。
    pub fn chat_theme(&self) -> String {
        self.load()
            .chat_theme
            .filter(|theme| valid_theme(theme))
            .unwrap_or_else(|| DEFAULT_CHAT_THEME.to_string())
    }

    pub fn set_chat_theme(&self, theme: &str) -> anyhow::Result<()> {
        let mut saved = self.load();
        saved.chat_theme = Some(theme.to_string());
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        crate::settings::atomic_write(&self.path, serde_json::to_string_pretty(&saved)?.as_bytes())
    }
}

pub fn store() -> AccountSettingsStore {
    AccountSettingsStore::at(account_settings_path())
}

fn account_settings_path() -> PathBuf {
    crate::paths::default_app_state_dir().join("chatgpt-compat-account-settings.json")
}

/// 强调色名：小写字母、数字、`-` / `_`，不超过 32 个字符（Codex 的取值是 default、blue、purple 这类）。
fn valid_theme(theme: &str) -> bool {
    !theme.is_empty()
        && theme.len() <= 32
        && theme
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' || ch == '_')
}

/// `settings/user` 应答：把 `settings.chat_theme` 换成本机记的强调色。
pub fn with_saved_theme(body: &str, theme: &str) -> String {
    let Ok(mut value) = serde_json::from_str::<Value>(body) else {
        return body.to_string();
    };
    if let Some(settings) = value.get_mut("settings").and_then(Value::as_object_mut) {
        settings.insert("chat_theme".to_string(), json!(theme));
    }
    serde_json::to_string(&value).unwrap_or_else(|_| body.to_string())
}

/// 改账号级设置的请求（`/settings/account_user_setting?feature=…&value=…`）。
///
/// 只认强调色：存下来，应答回写整份设置（Codex 按设置对象的形状解析、核对 `chat_theme`）。
/// 别的设置项照旧回空对象，不记。值不合法时回 400，Codex 会退回原色。
pub fn account_user_setting_reply(
    path: &str,
    settings_user_body: &str,
    store: &AccountSettingsStore,
) -> (u16, String) {
    let query = reqwest::Url::parse(&format!("https://localhost{path}"))
        .ok()
        .map(|url| {
            url.query_pairs()
                .map(|(key, value)| (key.into_owned(), value.into_owned()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let get = |name: &str| {
        query
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    };
    if get("feature") != Some("chat_theme") {
        return (200, "{}".to_string());
    }
    let Some(theme) = get("value").filter(|theme| valid_theme(theme)) else {
        return (400, r#"{"detail":"invalid chat_theme"}"#.to_string());
    };
    if store.set_chat_theme(theme).is_err() {
        return (500, r#"{"detail":"could not save chat_theme"}"#.to_string());
    }
    let settings = serde_json::from_str::<Value>(&with_saved_theme(settings_user_body, theme))
        .ok()
        .and_then(|value| value.get("settings").cloned())
        .unwrap_or_else(|| json!({ "chat_theme": theme }));
    (200, settings.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SETTINGS_USER: &str =
        r#"{"user_id":"user-local","settings":{"chat_theme":"black","contrast_mode":"default"}}"#;

    #[test]
    fn settings_user_reports_default_until_changed_then_the_saved_theme() {
        let store = AccountSettingsStore::at(tempfile::tempdir().unwrap().keep().join("s.json"));
        assert_eq!(store.chat_theme(), "default");
        let body: Value =
            serde_json::from_str(&with_saved_theme(SETTINGS_USER, &store.chat_theme())).unwrap();
        assert_eq!(body["settings"]["chat_theme"], "default", "录制里的 black 不能原样回放");
        assert_eq!(body["settings"]["contrast_mode"], "default", "别的字段不动");

        let (status, reply) = account_user_setting_reply(
            "/backend-api/settings/account_user_setting?feature=chat_theme&value=purple",
            SETTINGS_USER,
            &store,
        );
        assert_eq!(status, 200);
        let reply: Value = serde_json::from_str(&reply).unwrap();
        assert_eq!(reply["chat_theme"], "purple", "Codex 核对回写的值");
        assert_eq!(reply["contrast_mode"], "default");
        assert_eq!(store.chat_theme(), "purple");
    }

    #[test]
    fn rejects_bad_values_and_ignores_other_features() {
        let store = AccountSettingsStore::at(tempfile::tempdir().unwrap().keep().join("s.json"));
        let (status, _) = account_user_setting_reply(
            "/backend-api/settings/account_user_setting?feature=chat_theme&value=%3Cscript%3E",
            SETTINGS_USER,
            &store,
        );
        assert_eq!(status, 400);
        let (status, reply) = account_user_setting_reply(
            "/backend-api/settings/account_user_setting?feature=other&value=x",
            SETTINGS_USER,
            &store,
        );
        assert_eq!((status, reply.as_str()), (200, "{}"));
        assert_eq!(store.chat_theme(), "default");
    }
}
