//! 把 ChatGPT 桌面版接到本地兼容服务上，以及还原。
//!
//! 打开时改两处，都先存档：
//! - `config.toml`：只加 `chatgpt_base_url`，让桌面的 ChatGPT 后端请求打到兼容服务。
//!   模型供应商（`model_provider`、`model_providers`）一律不动，模型请求仍走用户原来的供应商。
//! - `auth.json`：写入假的 ChatGPT 登录，保留已有的 API Key。
//!
//! 关掉时按存档还原，只改回动过的键，期间的其它改动保留。

use std::path::Path;

use anyhow::Context;
use serde_json::{Value, json};
use toml_edit::DocumentMut;

use crate::chatgpt_compat::{self, ACCOUNT_ID};
use crate::gateway::restore::{self, FileStash};

const CONFIG_STASH: &str = "chatgpt-compat-config.json";
const AUTH_STASH: &str = "chatgpt-compat-auth.json";

/// 开关当前是否开着（以存档是否存在为准，不看设置里的布尔值）。
pub fn enabled() -> bool {
    stash_path(CONFIG_STASH).exists()
}

/// 打开：写入假登录和指向兼容服务的后端地址。
///
/// 已经开着时也会补写：切换供应商会去掉 `chatgpt_base_url`、改写 `auth.json`，
/// 补写只动缺了的键，存档里接入前的原文保持不变。
pub fn enable(home: &Path) -> anyhow::Result<()> {
    let _guard = crate::official_login::native_auth_write_guard(home)?;
    write_config(home)?;
    write_auth(home)?;
    let _ = crate::diagnostic_log::append_diagnostic_log(
        "chatgpt_compat.enabled",
        json!({ "account": ACCOUNT_ID }),
    );
    Ok(())
}

/// 关掉：按存档还原配置和登录。
pub fn disable(home: &Path) -> anyhow::Result<()> {
    let _guard = crate::official_login::native_auth_write_guard(home)?;
    restore_file(home, "config.toml", CONFIG_STASH, true)?;
    restore_file(home, "auth.json", AUTH_STASH, false)?;
    let _ = crate::diagnostic_log::append_diagnostic_log("chatgpt_compat.disabled", json!({}));
    Ok(())
}

/// 按设置对齐：开着但还没写入时写入，关了但仍有存档时还原。
///
/// 当前生效的是内置 `openai` 供应商时（官方登录或 `auth.json` 里的 API Key），模型请求要用
/// `auth.json` 里的登录，换成假登录就发不出去：这时不写，已写的还原，并返回原因。
pub fn reconcile(home: &Path, enabled_in_settings: bool) -> anyhow::Result<()> {
    if enabled_in_settings && !uses_custom_provider(home)? {
        if enabled() {
            disable(home)?;
        }
        anyhow::bail!(
            "当前用的是内置 OpenAI 供应商（官方登录或 auth.json 里的 API Key），换成本地登录后模型会连不上；先切到 API Key 供应商或本地服务再开"
        );
    }
    if enabled_in_settings {
        enable(home)
    } else if enabled() {
        disable(home)
    } else {
        Ok(())
    }
}

/// 生效的 `model_provider` 是不是自带凭据的自定义供应商（不读 `auth.json` 里的登录发模型请求）。
///
/// `requires_openai_auth = true` 又没有 `experimental_bearer_token` / `env_key` 的供应商会拿
/// `auth.json` 的登录当凭据，换成假登录后同样发不出去，也算不上。
fn uses_custom_provider(home: &Path) -> anyhow::Result<bool> {
    let text = restore::read_optional(&home.join("config.toml"))?;
    let value = restore::toml_value(text.as_deref())?;
    let Some(provider) = value
        .get("model_provider")
        .and_then(Value::as_str)
        .filter(|provider| !provider.trim().is_empty() && *provider != "openai")
    else {
        return Ok(false);
    };
    let table = value.pointer(&format!("/model_providers/{provider}"));
    let needs_login = table
        .and_then(|table| table.get("requires_openai_auth"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let has_own_key = table.is_some_and(|table| {
        ["experimental_bearer_token", "env_key"].iter().any(|key| {
            table
                .get(*key)
                .and_then(Value::as_str)
                .is_some_and(|value| !value.trim().is_empty())
        })
    });
    Ok(!needs_login || has_own_key)
}

fn backend_url() -> String {
    format!("https://localhost:{}/backend-api/", chatgpt_compat::PORT)
}

fn write_config(home: &Path) -> anyhow::Result<()> {
    let path = home.join("config.toml");
    let live = restore::read_optional(&path)?;
    let mut doc = live
        .as_deref()
        .unwrap_or("")
        .parse::<DocumentMut>()
        .unwrap_or_else(|_| DocumentMut::new());
    if doc.get("chatgpt_base_url").and_then(|item| item.as_str()) == Some(backend_url().as_str()) {
        return Ok(());
    }
    // 只改后端地址。模型走哪家供应商是用户的选择，这里不碰。
    doc["chatgpt_base_url"] = toml_edit::value(backend_url());
    let after = doc.to_string();
    let stash_file = stash_path(CONFIG_STASH);
    let mut stash = restore::load_stash::<FileStash>(&stash_file)
        .unwrap_or_else(|| FileStash::new(live.clone()));
    stash.record_write(
        &restore::toml_value(live.as_deref())?,
        &restore::toml_value(Some(&after))?,
    );
    // 先存档再写文件：写到一半退出时，还原仍知道接入前的原文。
    restore::save_stash(&stash_file, &stash)?;
    crate::settings::atomic_write(&path, after.as_bytes())
        .with_context(|| format!("写 {} 失败", path.display()))?;
    Ok(())
}

fn write_auth(home: &Path) -> anyhow::Result<()> {
    let path = home.join("auth.json");
    let live = restore::read_optional(&path)?;
    let live_value = json_or_empty(live.as_deref());
    if live_value.pointer("/tokens/account_id").and_then(Value::as_str) == Some(ACCOUNT_ID)
        && live_value.get("auth_mode").and_then(Value::as_str) == Some("chatgpt")
        && live_value
            .pointer("/tokens/id_token")
            .and_then(Value::as_str)
            .is_some_and(chatgpt_compat::token_matches_profile)
    {
        return Ok(());
    }
    let after = chatgpt_compat::fake_auth_json(live.as_deref());
    let stash_file = stash_path(AUTH_STASH);
    let mut stash = restore::load_stash::<FileStash>(&stash_file)
        .unwrap_or_else(|| FileStash::new(live.clone()));
    stash.record_write(&live_value, &json_or_empty(Some(&after)));
    restore::save_stash(&stash_file, &stash)?;
    crate::settings::atomic_write(&path, after.as_bytes())
        .with_context(|| format!("写 {} 失败", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

fn restore_file(home: &Path, name: &str, stash_name: &str, toml: bool) -> anyhow::Result<()> {
    let stash_file = stash_path(stash_name);
    let Some(stash) = restore::load_stash::<FileStash>(&stash_file) else {
        return Ok(());
    };
    let path = home.join(name);
    let live = restore::read_optional(&path)?;
    let restored = if toml {
        restore::restore_toml(stash.before.as_deref(), live.as_deref(), &stash.paths())?
    } else {
        restore::restore_json(
            stash.before.as_deref(),
            live.as_deref(),
            &auth_keys_to_restore(&stash.changed),
            keep_auth_key,
        )?
    };
    restore::write_or_remove(&path, restored.as_deref())?;
    restore::remove_stash(&stash_file);
    Ok(())
}

/// 登录相关的键按顶层整体还原：存档按两层记了 `tokens.access_token` 这样的子键，
/// 而 Codex 刷新 token 时会整块改写 `tokens`，逐个子键还原会留下半个登录。
fn auth_keys_to_restore(changed: &[restore::ChangedKey]) -> Vec<restore::ChangedKey> {
    let mut out: Vec<restore::ChangedKey> = Vec::new();
    for key in changed {
        let key = match key.path.first().map(String::as_str) {
            Some(top @ ("tokens" | "last_refresh" | "auth_mode")) => restore::ChangedKey {
                path: vec![top.to_string()],
                written: None,
            },
            _ => key.clone(),
        };
        if !out.iter().any(|known| known.path == key.path) {
            out.push(key);
        }
    }
    out
}

/// 还原 `auth.json` 时哪些键保留当前值。
///
/// 登录相关的键（`tokens` / `last_refresh` / `auth_mode`）看当前登录是不是还是本地假账号：
/// 开着期间 Codex 会自己刷新 token、改写这几项，不能当成用户改的；只有换成了真实登录才保留。
/// 其它键（如 API Key）开着期间被改过就保留。
fn keep_auth_key(key: &restore::ChangedKey, _before: &Value, current: &Value) -> bool {
    match key.path.first().map(String::as_str) {
        Some("tokens" | "last_refresh" | "auth_mode") => !login_is_local(current),
        _ => restore::value_at(current, &key.path) != key.written.as_ref(),
    }
}

/// 当前登录是不是本地假账号（或者已经没有登录）。
fn login_is_local(auth: &Value) -> bool {
    let Some(tokens) = auth.get("tokens").filter(|tokens| tokens.is_object()) else {
        return true;
    };
    if tokens.get("account_id").and_then(Value::as_str) == Some(ACCOUNT_ID) {
        return true;
    }
    ["access_token", "id_token"].iter().any(|name| {
        tokens
            .get(*name)
            .and_then(Value::as_str)
            .and_then(jwt_account_id)
            .is_some_and(|account| account == ACCOUNT_ID)
    })
}

fn jwt_account_id(token: &str) -> Option<String> {
    use base64::Engine;
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    let claims: Value = serde_json::from_slice(&bytes).ok()?;
    claims
        .pointer("/https:~1~1api.openai.com~1auth/chatgpt_account_id")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn stash_path(name: &str) -> std::path::PathBuf {
    crate::paths::default_app_state_dir()
        .join("backups")
        .join(name)
}

fn json_or_empty(text: Option<&str>) -> Value {
    text.and_then(|text| serde_json::from_str(text).ok())
        .unwrap_or_else(|| json!({}))
}

/// 假登录里的账号 id，给测试和诊断用。
pub fn account_id() -> &'static str {
    ACCOUNT_ID
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 存档落在状态目录，测试必须换一个临时目录，不能写到用户的真实目录。
    struct StateGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        _dir: tempfile::TempDir,
        previous: Option<std::ffi::OsString>,
    }

    impl StateGuard {
        fn new() -> Self {
            let lock = crate::paths::settings_path_test_guard();
            let dir = tempfile::tempdir().unwrap();
            let previous = std::env::var_os(crate::paths::STATE_DIR_ENV);
            unsafe { std::env::set_var(crate::paths::STATE_DIR_ENV, dir.path()) };
            Self { _lock: lock, _dir: dir, previous }
        }
    }

    impl Drop for StateGuard {
        fn drop(&mut self) {
            unsafe {
                match self.previous.take() {
                    Some(previous) => std::env::set_var(crate::paths::STATE_DIR_ENV, previous),
                    None => std::env::remove_var(crate::paths::STATE_DIR_ENV),
                }
            }
        }
    }

    fn home() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn enable_only_adds_backend_url_and_keeps_the_provider() {
        let _state = StateGuard::new();
        let dir = home();
        let home = dir.path();
        std::fs::write(
            home.join("config.toml"),
            "model = \"gpt-5\"\nmodel_provider = \"magpie\"\n",
        )
        .unwrap();
        std::fs::write(home.join("auth.json"), r#"{"OPENAI_API_KEY":"sk-real"}"#).unwrap();

        enable(home).unwrap();
        let config = std::fs::read_to_string(home.join("config.toml")).unwrap();
        assert!(config.contains("chatgpt_base_url"));
        assert!(config.contains("localhost:8000"));
        assert!(config.contains("model_provider = \"magpie\""));
        assert!(config.contains("model = \"gpt-5\""));
        assert!(!config.contains("model_providers.codex-plus"));
        let auth: Value =
            serde_json::from_str(&std::fs::read_to_string(home.join("auth.json")).unwrap())
                .unwrap();
        assert_eq!(auth["OPENAI_API_KEY"], "sk-real");
        assert_eq!(auth["auth_mode"], "chatgpt");
        assert!(enabled());
    }

    #[test]
    fn enable_then_disable_restores_both_files() {
        let _state = StateGuard::new();
        let dir = home();
        let home = dir.path();
        std::fs::write(home.join("config.toml"), "model = \"gpt-5\"\n").unwrap();
        std::fs::write(home.join("auth.json"), r#"{"OPENAI_API_KEY":"sk-real"}"#).unwrap();

        enable(home).unwrap();
        disable(home).unwrap();
        let config = std::fs::read_to_string(home.join("config.toml")).unwrap();
        assert_eq!(config.trim(), "model = \"gpt-5\"");
        let auth = std::fs::read_to_string(home.join("auth.json")).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&auth).unwrap()["OPENAI_API_KEY"],
            "sk-real"
        );
        assert!(serde_json::from_str::<Value>(&auth).unwrap().get("auth_mode").is_none());
        assert!(!enabled());
    }

    #[test]
    fn disable_keeps_changes_made_while_enabled() {
        let _state = StateGuard::new();
        let dir = home();
        let home = dir.path();
        std::fs::write(home.join("config.toml"), "").unwrap();
        enable(home).unwrap();
        let mut config = std::fs::read_to_string(home.join("config.toml")).unwrap();
        config.push_str("\n[projects.\"/tmp/x\"]\ntrust_level = \"trusted\"\n");
        std::fs::write(home.join("config.toml"), &config).unwrap();

        disable(home).unwrap();
        let config = std::fs::read_to_string(home.join("config.toml")).unwrap();
        assert!(!config.contains("chatgpt_base_url"));
        assert!(config.contains("trust_level"));
    }

    #[test]
    fn reconcile_is_idempotent() {
        let _state = StateGuard::new();
        let dir = home();
        let home = dir.path();
        let custom = "model_provider = \"custom\"\n";
        std::fs::write(home.join("config.toml"), custom).unwrap();
        reconcile(home, true).unwrap();
        reconcile(home, true).unwrap();
        assert!(enabled());
        reconcile(home, false).unwrap();
        assert!(!enabled());
        assert_eq!(std::fs::read_to_string(home.join("config.toml")).unwrap(), custom);
        // 原来没有 auth.json，还原后也不留下。
        assert!(!home.join("auth.json").exists());
    }

    #[test]
    fn reconcile_refuses_with_builtin_openai_provider() {
        let _state = StateGuard::new();
        let dir = home();
        let home = dir.path();
        let real = r#"{"auth_mode":"chatgpt","tokens":{"access_token":"real"}}"#;
        std::fs::write(home.join("auth.json"), real).unwrap();
        // 没写 model_provider 就是内置 openai。
        std::fs::write(home.join("config.toml"), "model = \"gpt-5\"\n").unwrap();
        assert!(reconcile(home, true).is_err());
        assert!(!enabled());
        assert_eq!(std::fs::read_to_string(home.join("auth.json")).unwrap(), real);
    }

    #[test]
    fn reconcile_refuses_provider_that_borrows_the_login() {
        let _state = StateGuard::new();
        let dir = home();
        let home = dir.path();
        let borrowing = "model_provider = \"p\"\n[model_providers.p]\nbase_url = \"http://x/v1\"\nrequires_openai_auth = true\n";
        std::fs::write(home.join("config.toml"), borrowing).unwrap();
        assert!(reconcile(home, true).is_err());
        let with_key = format!("{borrowing}experimental_bearer_token = \"k\"\n");
        std::fs::write(home.join("config.toml"), &with_key).unwrap();
        reconcile(home, true).unwrap();
        assert!(enabled());
    }

    #[test]
    fn enable_reapplies_backend_url_after_a_provider_switch_strips_it() {
        let _state = StateGuard::new();
        let dir = home();
        let home = dir.path();
        std::fs::write(home.join("config.toml"), "model_provider = \"custom\"\n").unwrap();
        enable(home).unwrap();
        // 切供应商重写了整个文件，后端地址没了。
        std::fs::write(home.join("config.toml"), "model_provider = \"other\"\n").unwrap();
        enable(home).unwrap();
        let config = std::fs::read_to_string(home.join("config.toml")).unwrap();
        assert!(config.contains("chatgpt_base_url"));
        assert!(config.contains("model_provider = \"other\""));
        disable(home).unwrap();
        let config = std::fs::read_to_string(home.join("config.toml")).unwrap();
        assert!(!config.contains("chatgpt_base_url"));
        assert!(config.contains("model_provider = \"other\""));
    }

    #[test]
    fn disable_removes_tokens_that_codex_refreshed_while_enabled() {
        let _state = StateGuard::new();
        let dir = home();
        let home = dir.path();
        std::fs::write(home.join("auth.json"), r#"{"OPENAI_API_KEY":"sk"}"#).unwrap();
        enable(home).unwrap();
        // Codex 用本地签发的 token 刷新后改写：没有 account_id，last_refresh 也变了。
        let mut auth: Value =
            serde_json::from_str(&std::fs::read_to_string(home.join("auth.json")).unwrap()).unwrap();
        let token = auth["tokens"]["access_token"].clone();
        auth["tokens"] = json!({ "access_token": token, "id_token": token });
        auth["last_refresh"] = json!("2030-01-01T00:00:00Z");
        std::fs::write(home.join("auth.json"), auth.to_string()).unwrap();

        disable(home).unwrap();
        let auth: Value =
            serde_json::from_str(&std::fs::read_to_string(home.join("auth.json")).unwrap()).unwrap();
        assert_eq!(auth, json!({ "OPENAI_API_KEY": "sk" }));
    }

    #[test]
    fn disable_keeps_a_real_login_made_while_enabled() {
        let _state = StateGuard::new();
        let dir = home();
        let home = dir.path();
        std::fs::write(home.join("auth.json"), r#"{"OPENAI_API_KEY":"sk"}"#).unwrap();
        enable(home).unwrap();
        let real = json!({ "OPENAI_API_KEY": "sk", "auth_mode": "chatgpt",
            "tokens": { "access_token": "a.b.c", "account_id": "acc-real" } });
        std::fs::write(home.join("auth.json"), real.to_string()).unwrap();
        disable(home).unwrap();
        let auth: Value =
            serde_json::from_str(&std::fs::read_to_string(home.join("auth.json")).unwrap()).unwrap();
        assert_eq!(auth["tokens"]["account_id"], "acc-real");
        assert_eq!(auth["auth_mode"], "chatgpt");
    }

    #[test]
    fn disable_keeps_an_api_key_written_while_enabled() {
        let _state = StateGuard::new();
        let dir = home();
        let home = dir.path();
        std::fs::write(home.join("auth.json"), r#"{"OPENAI_API_KEY":"sk-old"}"#).unwrap();
        enable(home).unwrap();
        let mut auth: Value =
            serde_json::from_str(&std::fs::read_to_string(home.join("auth.json")).unwrap()).unwrap();
        auth["OPENAI_API_KEY"] = json!("sk-new");
        std::fs::write(home.join("auth.json"), auth.to_string()).unwrap();
        disable(home).unwrap();
        let auth: Value =
            serde_json::from_str(&std::fs::read_to_string(home.join("auth.json")).unwrap()).unwrap();
        assert_eq!(auth["OPENAI_API_KEY"], "sk-new");
        assert!(auth.get("tokens").is_none(), "{auth}");
    }
}
