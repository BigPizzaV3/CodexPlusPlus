use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::Value;

const RELAY_PROVIDER: &str = "CodexPlusPlus";
const LEGACY_RELAY_PROVIDER: &str = "CodexPP";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatGptAuthStatus {
    pub authenticated: bool,
    pub source: String,
    pub account_label: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayConfigStatus {
    pub configured: bool,
    pub requires_openai_auth: bool,
    pub has_bearer_token: bool,
    pub config_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayStatus {
    pub authenticated: bool,
    pub auth_source: String,
    pub account_label: Option<String>,
    pub config_path: String,
    pub configured: bool,
    pub requires_openai_auth: bool,
    pub has_bearer_token: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayApplyResult {
    pub config_path: String,
    pub backup_path: Option<String>,
    pub configured: bool,
}

pub fn default_codex_home_dir() -> PathBuf {
    directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().join(".codex"))
        .unwrap_or_else(|| PathBuf::from(".codex"))
}

pub fn default_relay_status() -> RelayStatus {
    relay_status_from_home(&default_codex_home_dir())
}

pub fn relay_status_from_home(home: &Path) -> RelayStatus {
    let auth = chatgpt_auth_status_from_home(home);
    let config = relay_config_status_from_home(home);
    RelayStatus {
        authenticated: auth.authenticated,
        auth_source: auth.source,
        account_label: auth.account_label,
        config_path: config.config_path,
        configured: config.configured,
        requires_openai_auth: config.requires_openai_auth,
        has_bearer_token: config.has_bearer_token,
    }
}

pub fn chatgpt_auth_status_from_home(home: &Path) -> ChatGptAuthStatus {
    let auth_path = home.join("auth.json");
    if let Some(account_label) = auth_json_chatgpt_account_label(&auth_path) {
        return ChatGptAuthStatus {
            authenticated: true,
            source: auth_path.to_string_lossy().to_string(),
            account_label,
            message: "已通过 auth.json 和 config.toml 检测到 ChatGPT 登录。".to_string(),
        };
    }

    ChatGptAuthStatus {
        authenticated: false,
        source: String::new(),
        account_label: None,
        message: "未检测到 ChatGPT 登录账号。".to_string(),
    }
}

pub fn relay_config_status_from_home(home: &Path) -> RelayConfigStatus {
    let config_path = home.join("config.toml");
    let contents = std::fs::read_to_string(&config_path).unwrap_or_default();
    let root_provider = root_key_string(&contents, "model_provider")
        .map(|value| value == RELAY_PROVIDER)
        .unwrap_or(false);
    let provider = table_values(&contents, &format!("model_providers.{RELAY_PROVIDER}"));
    let requires_openai_auth = provider
        .as_ref()
        .and_then(|values| values.get("requires_openai_auth"))
        .map(|value| value.trim() == "true")
        .unwrap_or(false);
    let has_bearer_token = provider
        .as_ref()
        .and_then(|values| values.get("experimental_bearer_token"))
        .map(|value| unquote_toml_string(value).trim().to_string())
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    let has_base_url = provider
        .as_ref()
        .and_then(|values| values.get("base_url"))
        .map(|value| !unquote_toml_string(value).trim().is_empty())
        .unwrap_or(false);
    RelayConfigStatus {
        configured: root_provider && requires_openai_auth && has_bearer_token && has_base_url,
        requires_openai_auth,
        has_bearer_token,
        config_path: config_path.to_string_lossy().to_string(),
    }
}

pub fn apply_relay_config_to_home(
    home: &Path,
    base_url: &str,
    bearer_token: &str,
) -> anyhow::Result<RelayApplyResult> {
    let base_url = base_url.trim();
    if base_url.is_empty() {
        anyhow::bail!("中转 Base URL 不能为空");
    }
    let bearer_token = bearer_token.trim();
    if bearer_token.is_empty() {
        anyhow::bail!("中转 Key 不能为空");
    }
    std::fs::create_dir_all(home)?;
    let config_path = home.join("config.toml");
    let existing = std::fs::read_to_string(&config_path).unwrap_or_default();
    let backup_path = if config_path.exists() {
        let path = home.join(format!("config.toml.codex-plus-backup-{}.bak", now_ms()));
        std::fs::write(&path, &existing)?;
        Some(path)
    } else {
        None
    };
    let updated = upsert_model_provider_config(&existing, base_url, bearer_token);
    std::fs::write(&config_path, updated)?;
    let status = relay_config_status_from_home(home);
    Ok(RelayApplyResult {
        config_path: status.config_path,
        backup_path: backup_path.map(|path| path.to_string_lossy().to_string()),
        configured: status.configured,
    })
}

pub fn clear_relay_config_to_home(home: &Path) -> anyhow::Result<RelayApplyResult> {
    std::fs::create_dir_all(home)?;
    let config_path = home.join("config.toml");
    let existing = std::fs::read_to_string(&config_path).unwrap_or_default();
    let backup_path = if config_path.exists() {
        let path = home.join(format!("config.toml.codex-plus-backup-{}.bak", now_ms()));
        std::fs::write(&path, &existing)?;
        Some(path)
    } else {
        None
    };
    let without_relay = remove_root_key(
        &remove_table(
            &remove_table(&existing, &format!("model_providers.{RELAY_PROVIDER}")),
            &format!("model_providers.{LEGACY_RELAY_PROVIDER}"),
        ),
        "OPENAI_API_KEY",
    );
    let updated = remove_root_key(&without_relay, "model_provider");
    std::fs::write(&config_path, updated)?;
    let status = relay_config_status_from_home(home);
    Ok(RelayApplyResult {
        config_path: status.config_path,
        backup_path: backup_path.map(|path| path.to_string_lossy().to_string()),
        configured: status.configured,
    })
}

fn auth_json_chatgpt_account_label(path: &Path) -> Option<Option<String>> {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return None;
    };
    let Ok(value) = serde_json::from_str::<Value>(&contents) else {
        return None;
    };
    let is_chatgpt = value
        .get("auth_mode")
        .and_then(Value::as_str)
        .map(|mode| mode.eq_ignore_ascii_case("chatgpt"))
        .unwrap_or(false);
    let tokens = value.get("tokens")?;
    if !is_chatgpt || !tokens_have_login_secret(tokens) {
        return None;
    }
    Some(account_label_from_tokens(tokens))
}

fn tokens_have_login_secret(tokens: &Value) -> bool {
    ["access_token", "id_token", "refresh_token"]
        .iter()
        .any(|key| {
            tokens
                .get(*key)
                .and_then(Value::as_str)
                .map(|token| !token.trim().is_empty())
                .unwrap_or(false)
        })
}

fn account_label_from_tokens(tokens: &Value) -> Option<String> {
    ["id_token", "access_token"].iter().find_map(|key| {
        tokens
            .get(*key)
            .and_then(Value::as_str)
            .and_then(account_label_from_jwt)
    })
}

fn account_label_from_jwt(token: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    use base64::Engine;
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.as_bytes())
        .ok()
        .or_else(|| {
            base64::engine::general_purpose::URL_SAFE
                .decode(payload.as_bytes())
                .ok()
        })?;
    let value: Value = serde_json::from_slice(&decoded).ok()?;
    value
        .get("email")
        .and_then(Value::as_str)
        .or_else(|| {
            value
                .get("https://api.openai.com/profile")
                .and_then(|profile| profile.get("email"))
                .and_then(Value::as_str)
        })
        .or_else(|| value.get("name").and_then(Value::as_str))
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .map(ToString::to_string)
}

fn root_key_string(contents: &str, key: &str) -> Option<String> {
    root_key_value(contents, key).map(unquote_toml_string)
}

fn root_key_value<'a>(contents: &'a str, key: &str) -> Option<&'a str> {
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            return None;
        }
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }
        let Some((name, value)) = trimmed.split_once('=') else {
            continue;
        };
        if name.trim() == key {
            return Some(value);
        }
    }
    None
}

fn upsert_root_keys(contents: &str, entries: &[(&str, String)]) -> String {
    let mut lines = contents
        .lines()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let root_end = lines
        .iter()
        .position(|line| line.trim_start().starts_with('['))
        .unwrap_or(lines.len());

    for (key, value) in entries {
        if let Some(index) = lines[..root_end]
            .iter()
            .position(|line| root_line_key(line) == Some(*key))
        {
            lines[index] = format!("{key} = {value}");
        } else {
            lines.insert(root_end, format!("{key} = {value}"));
        }
    }

    let mut updated = lines.join("\n");
    if !updated.ends_with('\n') {
        updated.push('\n');
    }
    updated
}

fn upsert_model_provider_config(contents: &str, base_url: &str, bearer_token: &str) -> String {
    let mut updated = upsert_root_keys(
        contents,
        &[(
            "model_provider",
            format!("\"{}\"", toml_escape(RELAY_PROVIDER)),
        )],
    );
    updated = remove_table(&updated, &format!("model_providers.{RELAY_PROVIDER}"));
    updated = remove_table(
        &updated,
        &format!("model_providers.{LEGACY_RELAY_PROVIDER}"),
    );

    let mut lines = updated.lines().map(ToString::to_string).collect::<Vec<_>>();
    let insert_at = first_non_provider_table_index(&lines).unwrap_or(lines.len());
    let provider_lines = vec![
        format!("[model_providers.{RELAY_PROVIDER}]"),
        format!("name = \"{}\"", toml_escape(RELAY_PROVIDER)),
        "wire_api = \"responses\"".to_string(),
        "requires_openai_auth = true".to_string(),
        format!("base_url = \"{}\"", toml_escape(base_url)),
        format!(
            "experimental_bearer_token = \"{}\"",
            toml_escape(bearer_token)
        ),
        String::new(),
    ];
    lines.splice(insert_at..insert_at, provider_lines);
    let mut output = lines.join("\n");
    if !output.ends_with('\n') {
        output.push('\n');
    }
    output
}

fn remove_table(contents: &str, table: &str) -> String {
    let header = format!("[{table}]");
    let mut lines = Vec::new();
    let mut skipping = false;
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if trimmed == header {
                skipping = true;
                continue;
            }
            skipping = false;
        }
        if !skipping {
            lines.push(line.to_string());
        }
    }
    lines.join("\n")
}

fn remove_root_key(contents: &str, key: &str) -> String {
    let mut lines = Vec::new();
    let mut in_root = true;
    for line in contents.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('[') {
            in_root = false;
        }
        if in_root && root_line_key(line) == Some(key) {
            continue;
        }
        lines.push(line.to_string());
    }
    lines.join("\n")
}

fn first_non_provider_table_index(lines: &[String]) -> Option<usize> {
    lines.iter().position(|line| {
        let trimmed = line.trim();
        trimmed.starts_with('[') && !trimmed.starts_with("[model_providers.")
    })
}

fn table_values(contents: &str, table: &str) -> Option<std::collections::HashMap<String, String>> {
    let header = format!("[{table}]");
    let mut in_table = false;
    let mut values = std::collections::HashMap::new();
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if in_table {
                break;
            }
            in_table = trimmed == header;
            continue;
        }
        if !in_table || trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = trimmed.split_once('=') {
            values.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    in_table.then_some(values)
}

fn unquote_toml_string(value: &str) -> String {
    let value = value.trim();
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(value)
        .to_string()
}

fn root_line_key(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    if trimmed.starts_with('#') || trimmed.starts_with('[') {
        return None;
    }
    trimmed.split_once('=').map(|(key, _)| key.trim())
}

fn toml_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
