use anyhow::Context;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use toml_edit::{DocumentMut, Item, Table, TableLike};

use crate::settings::{
    BedrockAuthMode, BedrockConfig, RelayProfile, RelayProtocol, RelaySessionProvider,
};

const RELAY_PROVIDER: &str = "custom";
/// 我们代管的 config.toml 上下文表。
///
/// 这里没有 `skills`：codex 的 `skills` 配置键是一个三字段结构体
/// （bundled / include_instructions / max_context_tokens），`[skills.<id>]` 会被
/// serde 当未知字段丢掉。skill 靠 `$CODEX_HOME/skills/<id>/SKILL.md` 目录发现，
/// 由 crate::skills 管理。
const CONTEXT_TABLE_NAMES: [&str; 2] = ["mcp_servers", "plugins"];
const LEGACY_RELAY_PROVIDERS: &[&str] = &["CodexPlusPlus", "CodexPP"];
const CC_SWITCH_MODEL_CATALOG_FILENAME: &str = "cc-switch-model-catalog.json";
const CHAT_UPSTREAM_BASE_URL_KEY: &str = "codex_plus_chat_base_url";
const PROVIDER_SPECIFIC_COMMON_ROOT_KEYS: &[&str] = &[
    "model",
    "model_provider",
    "base_url",
    "openai_base_url",
    "chatgpt_base_url",
    "model_catalog_json",
    "OPENAI_API_KEY",
    CHAT_UPSTREAM_BASE_URL_KEY,
];
const RESERVED_MODEL_PROVIDER_IDS: &[&str] = &[
    "amazon-bedrock",
    "openai",
    "ollama",
    "lmstudio",
    "oss",
    "ollama-chat",
];

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayProfileTestResult {
    pub http_status: u16,
    pub endpoint: String,
    pub response_preview: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexContextEntry {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub summary: String,
    pub toml_body: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexContextEntries {
    pub mcp_servers: Vec<CodexContextEntry>,
    pub plugins: Vec<CodexContextEntry>,
}

pub fn default_codex_home_dir() -> PathBuf {
    crate::codex_home::default_codex_home_dir()
}

pub fn default_relay_status() -> RelayStatus {
    relay_status_from_home(&default_codex_home_dir())
}

pub fn set_codex_goals_feature_in_home(home: &Path, enabled: bool) -> anyhow::Result<()> {
    std::fs::create_dir_all(home)?;
    let config_path = home.join("config.toml");
    let existing = std::fs::read_to_string(&config_path).unwrap_or_default();
    let updated = match parse_toml_document(&existing) {
        Ok(mut doc) => {
            if enabled {
                let features = table_mut_or_insert(&mut doc, "features")?;
                features["goals"] = toml_edit::value(true);
            } else if let Some(features) = table_mut_if_exists(&mut doc, "features") {
                features.remove("goals");
                if features.is_empty() {
                    doc.as_table_mut().remove("features");
                }
            }
            ensure_trailing_newline(doc.to_string())
        }
        Err(_) => set_codex_goals_feature_text_fallback(&existing, enabled),
    };
    crate::settings::atomic_write(&config_path, updated.as_bytes())
}

fn set_codex_goals_feature_text_fallback(existing: &str, enabled: bool) -> String {
    let mut kept = Vec::new();
    let mut skipping_features = false;

    for line in existing.lines() {
        let trimmed = line.trim();
        if trimmed == "[features]" {
            skipping_features = true;
            continue;
        }
        if skipping_features && trimmed.starts_with('[') && trimmed.ends_with(']') {
            skipping_features = false;
        }
        if !skipping_features {
            kept.push(line);
        }
    }

    let mut updated = kept.join("\n").trim_end().to_string();
    if enabled {
        if !updated.is_empty() {
            updated.push_str("\n\n");
        }
        updated.push_str("[features]\ngoals = true");
    }
    ensure_trailing_newline(updated)
}

fn table_mut_or_insert<'a>(doc: &'a mut DocumentMut, key: &str) -> anyhow::Result<&'a mut Table> {
    if !doc.as_table().contains_key(key) {
        doc[key] = toml_edit::table();
    }
    if doc.get(key).and_then(Item::as_table).is_none() {
        doc[key] = toml_edit::table();
    }
    doc.get_mut(key)
        .and_then(Item::as_table_mut)
        .ok_or_else(|| anyhow::anyhow!("{key} 必须是 TOML table"))
}

fn table_mut_if_exists<'a>(doc: &'a mut DocumentMut, key: &str) -> Option<&'a mut Table> {
    doc.get_mut(key).and_then(Item::as_table_mut)
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
    let auth_contents = std::fs::read_to_string(home.join("auth.json")).unwrap_or_default();
    let has_auth_api_key = codex_auth_api_key(&auth_contents).is_some();
    let root_provider = root_key_string(&contents, "model_provider");
    let provider = root_provider.as_ref().and_then(|provider| {
        let active = table_values(&contents, &format!("model_providers.{provider}"));
        if provider != "openai" {
            return active;
        }

        if active
            .as_ref()
            .is_some_and(|values| provider_values_are_configured(values, has_auth_api_key))
        {
            return active;
        }

        let uses_managed_openai_identity = root_key_string(&contents, OPENAI_BASE_URL_KEY)
            .is_some_and(|value| value.trim() == managed_openai_base_url());
        if !uses_managed_openai_identity {
            return active;
        }

        table_values(&contents, &format!("model_providers.{RELAY_PROVIDER}"))
            .filter(|values| provider_values_are_configured(values, has_auth_api_key))
            .or(active)
    });
    let requires_openai_auth = provider
        .as_ref()
        .and_then(|values| values.get("requires_openai_auth"))
        .map(|value| value.trim() == "true")
        .unwrap_or(false);
    let explicitly_disables_auth = provider
        .as_ref()
        .and_then(|values| values.get("requires_openai_auth"))
        .is_some_and(|value| value.trim() == "false");
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
        configured: root_provider.is_some()
            && (has_bearer_token || has_auth_api_key || explicitly_disables_auth)
            && has_base_url,
        requires_openai_auth,
        has_bearer_token,
        config_path: config_path.to_string_lossy().to_string(),
    }
}

fn provider_values_are_configured(
    values: &HashMap<String, String>,
    has_auth_api_key: bool,
) -> bool {
    let has_base_url = values
        .get("base_url")
        .is_some_and(|value| !unquote_toml_string(value).trim().is_empty());
    let has_bearer_token = values
        .get("experimental_bearer_token")
        .is_some_and(|value| !unquote_toml_string(value).trim().is_empty());
    let explicitly_disables_auth = values
        .get("requires_openai_auth")
        .is_some_and(|value| value.trim() == "false");
    has_base_url && (has_bearer_token || has_auth_api_key || explicitly_disables_auth)
}

pub fn responses_proxy_configured_in_home(home: &Path) -> bool {
    let contents = match std::fs::read_to_string(home.join("config.toml")) {
        Ok(contents) => contents,
        Err(_) => return false,
    };
    provider_string_from_config(&contents, "base_url").as_deref()
        == Some(
            crate::protocol_proxy::local_responses_proxy_base_url(
                crate::protocol_proxy::DEFAULT_PROTOCOL_PROXY_PORT,
            )
            .as_str(),
        )
}

pub fn apply_relay_config_to_home(
    home: &Path,
    base_url: &str,
    bearer_token: &str,
) -> anyhow::Result<RelayApplyResult> {
    apply_relay_config_to_home_with_protocol(
        home,
        base_url,
        bearer_token,
        RelayProtocol::Responses,
        crate::protocol_proxy::DEFAULT_PROTOCOL_PROXY_PORT,
    )
}

pub fn apply_relay_config_to_home_with_protocol(
    home: &Path,
    base_url: &str,
    bearer_token: &str,
    protocol: RelayProtocol,
    proxy_port: u16,
) -> anyhow::Result<RelayApplyResult> {
    apply_relay_config_to_home_with_session_provider(
        home,
        base_url,
        bearer_token,
        protocol,
        proxy_port,
        RelaySessionProvider::Custom,
    )
}

pub fn apply_relay_config_to_home_with_session_provider(
    home: &Path,
    base_url: &str,
    bearer_token: &str,
    protocol: RelayProtocol,
    proxy_port: u16,
    session_provider: RelaySessionProvider,
) -> anyhow::Result<RelayApplyResult> {
    let base_url = base_url.trim();
    if base_url.is_empty() {
        anyhow::bail!("中转 Base URL 不能为空");
    }
    let bearer_token = bearer_token.trim();
    if bearer_token.is_empty() {
        anyhow::bail!("中转 Key 不能为空");
    }
    if session_provider == RelaySessionProvider::Openai && protocol != RelayProtocol::Responses {
        anyhow::bail!("OpenAI 会话身份仅支持 Responses API");
    }
    let codex_base_url = codex_base_url_for_protocol(base_url, protocol, proxy_port);
    let updated = upsert_model_provider_config_with_session_provider(
        "",
        &codex_base_url,
        bearer_token,
        true,
        session_provider,
    )?;
    let auth_contents = serde_json::to_string_pretty(&json!({
        "OPENAI_API_KEY": bearer_token
    }))?;
    let backup_path =
        write_codex_live_atomic(home, Some(&updated), Some(auth_contents.as_bytes()))?;
    let status = relay_config_status_from_home(home);
    Ok(RelayApplyResult {
        config_path: status.config_path,
        backup_path,
        configured: status.configured,
    })
}

pub fn apply_pure_api_config_to_home(
    home: &Path,
    base_url: &str,
    bearer_token: &str,
) -> anyhow::Result<RelayApplyResult> {
    apply_pure_api_config_to_home_with_protocol(
        home,
        base_url,
        bearer_token,
        RelayProtocol::Responses,
        crate::protocol_proxy::DEFAULT_PROTOCOL_PROXY_PORT,
    )
}

pub fn apply_relay_files_to_home(
    home: &Path,
    config_contents: &str,
    auth_contents: &str,
) -> anyhow::Result<RelayApplyResult> {
    if config_contents.trim().is_empty() {
        anyhow::bail!("config.toml 内容不能为空");
    }
    std::fs::create_dir_all(home)?;

    let backup_path =
        write_codex_live_atomic(home, Some(config_contents), Some(auth_contents.as_bytes()))?;

    let status = relay_config_status_from_home(home);
    Ok(RelayApplyResult {
        config_path: status.config_path,
        backup_path,
        configured: status.configured,
    })
}

pub fn apply_relay_files_to_home_with_common(
    home: &Path,
    config_contents: &str,
    auth_contents: &str,
    common_config_contents: &str,
) -> anyhow::Result<RelayApplyResult> {
    let config_contents = merge_common_config_into_config(config_contents, common_config_contents)?;
    apply_relay_files_to_home(home, &config_contents, auth_contents)
}

pub fn apply_relay_files_to_home_with_context(
    home: &Path,
    config_contents: &str,
    auth_contents: &str,
    common_config_contents: &str,
    context_window: &str,
    auto_compact_limit: &str,
) -> anyhow::Result<RelayApplyResult> {
    let selected_common = prepare_common_config_for_apply(common_config_contents)?;
    let config_with_common = merge_common_config_into_config(config_contents, &selected_common)?;
    let config_with_common =
        preserve_unmanaged_live_context_entries(home, &config_with_common, common_config_contents)?;
    let config_with_limits =
        apply_context_limits_to_config(&config_with_common, context_window, auto_compact_limit)?;
    apply_relay_files_to_home(home, &config_with_limits, auth_contents)
}

pub fn apply_relay_profile_files_to_home_with_context(
    home: &Path,
    profile: &RelayProfile,
    common_config_contents: &str,
) -> anyhow::Result<RelayApplyResult> {
    let selected_common = if profile.use_common_config {
        prepare_common_config_for_apply(common_config_contents)?
    } else {
        String::new()
    };
    let profile_config = complete_relay_profile_config(profile)?;
    let config_with_common = merge_common_config_into_config(&profile_config, &selected_common)?;
    let config_with_common =
        preserve_unmanaged_live_context_entries(home, &config_with_common, common_config_contents)?;
    let config_with_limits = apply_context_limits_to_config(
        &config_with_common,
        &profile.context_window,
        &profile.auto_compact_limit,
    )?;
    let config_with_catalog = apply_model_catalog_to_config(home, profile, &config_with_limits)?;
    let compatible_config = apply_deepseek_responses_compatibility(profile, &config_with_catalog)?;
    let auth_contents = relay_profile_auth_contents_for_apply(profile)?;
    apply_relay_files_to_home(home, &compatible_config, &auth_contents)
}

pub fn apply_relay_profile_to_home_with_switch_rules(
    home: &Path,
    profile: &RelayProfile,
    common_config_contents: &str,
) -> anyhow::Result<RelayApplyResult> {
    let selected_common = if profile.use_common_config {
        prepare_common_config_for_apply(common_config_contents)?
    } else {
        String::new()
    };
    let profile_config = complete_relay_profile_config(profile)?;
    let config_with_common = merge_common_config_into_config(&profile_config, &selected_common)?;
    let config_with_common =
        preserve_unmanaged_live_context_entries(home, &config_with_common, common_config_contents)?;
    let config_with_limits = apply_context_limits_to_config(
        &config_with_common,
        &profile.context_window,
        &profile.auto_compact_limit,
    )?;
    let config_with_catalog = apply_model_catalog_to_config(home, profile, &config_with_limits)?;
    let compatible_config = apply_deepseek_responses_compatibility(profile, &config_with_catalog)?;

    if profile.relay_mode == crate::settings::RelayMode::PureApi {
        let auth_contents = relay_profile_auth_contents_for_apply(profile)?;
        apply_relay_files_to_home(home, &compatible_config, &auth_contents)
    } else {
        let auth_contents = official_profile_auth_for_switch(home, &profile.auth_contents)?;
        apply_relay_files_to_home(home, &compatible_config, &auth_contents)
    }
}

fn relay_profile_auth_contents_for_apply(profile: &RelayProfile) -> anyhow::Result<String> {
    if profile.uses_no_auth() {
        no_auth_auth_contents(&profile.auth_contents)
    } else {
        Ok(profile.auth_contents.clone())
    }
}

pub fn apply_relay_profile_config_to_home_with_context(
    home: &Path,
    profile: &RelayProfile,
    common_config_contents: &str,
) -> anyhow::Result<RelayApplyResult> {
    let selected_common = if profile.use_common_config {
        prepare_common_config_for_apply(common_config_contents)?
    } else {
        String::new()
    };
    let profile_config = complete_relay_profile_config(profile)?;
    let config_with_common = merge_common_config_into_config(&profile_config, &selected_common)?;
    let config_with_limits = apply_context_limits_to_config(
        &config_with_common,
        &profile.context_window,
        &profile.auto_compact_limit,
    )?;
    let config_with_catalog = apply_model_catalog_to_config(home, profile, &config_with_limits)?;
    let compatible_config = apply_deepseek_responses_compatibility(profile, &config_with_catalog)?;
    apply_relay_config_file_to_home(home, &compatible_config)
}

pub fn apply_relay_config_file_to_home(
    home: &Path,
    config_contents: &str,
) -> anyhow::Result<RelayApplyResult> {
    let config_contents = config_contents
        .strip_prefix('\u{feff}')
        .unwrap_or(config_contents);
    if config_contents.trim().is_empty() {
        anyhow::bail!("config.toml 内容不能为空");
    }
    std::fs::create_dir_all(home)?;

    let backup_path = write_codex_live_atomic(home, Some(config_contents), None)?;

    let status = relay_config_status_from_home(home);
    Ok(RelayApplyResult {
        config_path: status.config_path,
        backup_path,
        configured: status.configured,
    })
}

pub fn apply_pure_api_config_to_home_with_protocol(
    home: &Path,
    base_url: &str,
    bearer_token: &str,
    protocol: RelayProtocol,
    proxy_port: u16,
) -> anyhow::Result<RelayApplyResult> {
    apply_pure_api_config_to_home_with_session_provider(
        home,
        base_url,
        bearer_token,
        protocol,
        proxy_port,
        RelaySessionProvider::Custom,
    )
}

pub fn apply_pure_api_config_to_home_with_session_provider(
    home: &Path,
    base_url: &str,
    bearer_token: &str,
    protocol: RelayProtocol,
    proxy_port: u16,
    session_provider: RelaySessionProvider,
) -> anyhow::Result<RelayApplyResult> {
    let base_url = base_url.trim();
    if base_url.is_empty() {
        anyhow::bail!("中转 Base URL 不能为空");
    }
    let bearer_token = bearer_token.trim();
    if bearer_token.is_empty() {
        anyhow::bail!("中转 Key 不能为空");
    }
    if session_provider == RelaySessionProvider::Openai && protocol != RelayProtocol::Responses {
        anyhow::bail!("OpenAI 会话身份仅支持 Responses API");
    }
    let codex_base_url = codex_base_url_for_protocol(base_url, protocol, proxy_port);
    let updated = upsert_model_provider_config_with_session_provider(
        "",
        &codex_base_url,
        bearer_token,
        false,
        session_provider,
    )?;
    let auth_contents = serde_json::to_string_pretty(&json!({
        "OPENAI_API_KEY": bearer_token
    }))?;
    let backup_path =
        write_codex_live_atomic(home, Some(&updated), Some(auth_contents.as_bytes()))?;
    let status = relay_config_status_from_home(home);
    Ok(RelayApplyResult {
        config_path: status.config_path,
        backup_path,
        configured: status.configured,
    })
}

pub async fn test_relay_profile(
    profile: &RelayProfile,
    model: &str,
) -> anyhow::Result<RelayProfileTestResult> {
    let base_url = relay_profile_base_url(profile);
    let base_url = base_url.trim().trim_end_matches('/');
    if base_url.is_empty() {
        anyhow::bail!("Base URL 不能为空");
    }
    let api_key = relay_profile_api_key(profile);
    let api_key = api_key.trim();
    if api_key.is_empty() && !profile.uses_no_auth() {
        anyhow::bail!("API Key 不能为空");
    }

    let client = crate::http_client::proxied_client("CodexPlusPlus/RelayTest")?;
    let endpoint = match profile.protocol {
        RelayProtocol::Responses => format!("{base_url}/responses"),
        RelayProtocol::ChatCompletions => format!("{base_url}/chat/completions"),
    };
    let test_model = model.trim();
    if test_model.is_empty() {
        anyhow::bail!("测试模型不能为空");
    }

    let payload = relay_profile_test_payload(profile.protocol, test_model);
    let mut request = client
        .post(&endpoint)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .json(&payload);
    if !profile.uses_no_auth() {
        request = request.bearer_auth(api_key);
    }
    let response = request.send().await?;
    let http_status = response.status().as_u16();

    // 如果 404 且 base_url 末尾没有 /v1，尝试自动补 /v1 后再发一次。
    // 许多上游（中转站、自建代理）暴露的路径以 /v1/ 开头，
    // 用户容易遗漏这个前缀，导致 /responses 或 /chat/completions 404。
    if http_status == 404 && !base_url.ends_with("/v1") {
        let v1_url = format!("{base_url}/v1");
        let v1_endpoint = match profile.protocol {
            RelayProtocol::Responses => format!("{v1_url}/responses"),
            RelayProtocol::ChatCompletions => format!("{v1_url}/chat/completions"),
        };
        let mut request = client
            .post(&v1_endpoint)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .json(&payload);
        if !profile.uses_no_auth() {
            request = request.bearer_auth(api_key);
        }
        let v1_response = request.send().await?;
        let v1_status = v1_response.status().as_u16();
        if v1_status < 400 {
            let response_text = v1_response.text().await.unwrap_or_default();
            return Ok(RelayProfileTestResult {
                http_status: v1_status,
                endpoint: v1_endpoint,
                response_preview: format!(
                    "（Base URL 建议加上 /v1 前缀）{}",
                    response_text.chars().take(280).collect::<String>()
                ),
            });
        }
    }

    let response_text = response.text().await.unwrap_or_default();
    Ok(RelayProfileTestResult {
        http_status,
        endpoint,
        response_preview: response_text.chars().take(320).collect(),
    })
}

fn relay_profile_test_payload(protocol: RelayProtocol, model: &str) -> Value {
    match protocol {
        RelayProtocol::Responses => serde_json::json!({
            "model": model,
            "input": "hi",
            "max_output_tokens": 16
        }),
        RelayProtocol::ChatCompletions => serde_json::json!({
            "model": model,
            "messages": [
                { "role": "user", "content": "hi" }
            ],
            "max_tokens": 16
        }),
    }
}

fn codex_base_url_for_protocol(base_url: &str, protocol: RelayProtocol, proxy_port: u16) -> String {
    match protocol {
        RelayProtocol::Responses => base_url.to_string(),
        RelayProtocol::ChatCompletions => {
            crate::protocol_proxy::local_responses_proxy_base_url(proxy_port)
        }
    }
}

const OPENAI_BASE_URL_KEY: &str = "openai_base_url";

fn managed_openai_base_url() -> String {
    crate::protocol_proxy::local_responses_proxy_base_url(
        crate::protocol_proxy::DEFAULT_PROTOCOL_PROXY_PORT,
    )
}

fn update_remote_control_openai_base_url(doc: &mut DocumentMut, enabled: bool) {
    let managed = managed_openai_base_url();
    let current = doc
        .get(OPENAI_BASE_URL_KEY)
        .and_then(Item::as_str)
        .map(str::trim)
        .map(ToString::to_string);

    if enabled {
        if current.as_deref().is_none_or(|value| value == managed) {
            doc[OPENAI_BASE_URL_KEY] = toml_edit::value(managed);
        }
    } else if current.as_deref() == Some(managed.as_str()) {
        doc.as_table_mut().remove(OPENAI_BASE_URL_KEY);
    }
}

fn active_session_provider_id(doc: &DocumentMut) -> String {
    active_provider_id(doc).unwrap_or_else(|| RELAY_PROVIDER.to_string())
}

pub fn relay_session_provider_from_config(contents: &str) -> RelaySessionProvider {
    parse_toml_document(contents)
        .ok()
        .and_then(|doc| active_provider_id(&doc))
        .filter(|provider| provider == "openai")
        .map(|_| RelaySessionProvider::Openai)
        .unwrap_or_default()
}

fn remove_managed_remote_control_openai_base_url(contents: &str) -> anyhow::Result<String> {
    let mut doc = parse_toml_document(contents)?;
    update_remote_control_openai_base_url(&mut doc, false);
    Ok(normalize_optional_toml(doc))
}

pub fn clear_relay_config_to_home(home: &Path) -> anyhow::Result<RelayApplyResult> {
    clear_relay_config_to_home_with_auth(home, None)
}

pub fn clear_relay_config_to_home_with_auth(
    home: &Path,
    auth_contents: Option<&str>,
) -> anyhow::Result<RelayApplyResult> {
    std::fs::create_dir_all(home)?;
    let auth_bytes = match auth_contents {
        Some(contents) if !contents.trim().is_empty() => Some(contents.as_bytes().to_vec()),
        _ => pure_api_auth_json_removed(home)?,
    };
    let config_path = home.join("config.toml");
    let existing = std::fs::read_to_string(&config_path).unwrap_or_default();
    let mut without_tables = existing;
    for legacy_provider in LEGACY_RELAY_PROVIDERS {
        without_tables = remove_table(
            &without_tables,
            &format!("model_providers.{legacy_provider}"),
        );
    }
    let mut updated = without_tables;
    for key in [
        "OPENAI_API_KEY",
        "model_provider",
        "model_catalog_json",
        "base_url",
        "experimental_bearer_token",
        "env_key",
        "requires_openai_auth",
    ] {
        updated = remove_root_key(&updated, key);
    }
    updated = remove_model_provider_auth_fields(&updated, RELAY_PROVIDER)?;
    updated = remove_managed_remote_control_openai_base_url(&updated)?;
    let backup_path = write_codex_live_atomic(home, Some(&updated), auth_bytes.as_deref())?;
    let status = relay_config_status_from_home(home);
    Ok(RelayApplyResult {
        config_path: status.config_path,
        backup_path,
        configured: status.configured,
    })
}

fn remove_model_provider_auth_fields(contents: &str, provider_id: &str) -> anyhow::Result<String> {
    let mut doc = parse_toml_document(contents)?;
    if let Some(provider) = doc
        .get_mut("model_providers")
        .and_then(Item::as_table_mut)
        .and_then(|providers| providers.get_mut(provider_id))
        .and_then(Item::as_table_mut)
    {
        for key in [
            "experimental_bearer_token",
            "env_key",
            "requires_openai_auth",
        ] {
            provider.remove(key);
        }
    }
    Ok(normalize_optional_toml(doc))
}

fn pure_api_auth_json_removed(home: &Path) -> anyhow::Result<Option<Vec<u8>>> {
    let auth_path = home.join("auth.json");
    if !auth_path.exists() {
        return Ok(None);
    }

    let existing = std::fs::read_to_string(&auth_path)?;
    let Ok(mut value) = serde_json::from_str::<Value>(&existing) else {
        return Ok(None);
    };
    let Some(object) = value.as_object_mut() else {
        return Ok(None);
    };
    if object.remove("OPENAI_API_KEY").is_none() {
        return Ok(None);
    }

    Ok(Some(serde_json::to_vec_pretty(&value)?))
}

pub fn backfill_relay_profile_from_home(
    home: &Path,
    profile: &mut RelayProfile,
) -> anyhow::Result<()> {
    profile.config_contents = read_optional_text(&home.join("config.toml"))?;
    profile.auth_contents = read_optional_text(&home.join("auth.json"))?;
    let live_config = profile.config_contents.clone();
    sync_context_limits_from_config(profile, &live_config);
    if profile.model.trim().is_empty() {
        if let Some(model) = root_key_string(&profile.config_contents, "model") {
            profile.model = model;
        }
    }
    Ok(())
}

pub fn backfill_relay_profile_from_home_with_common(
    home: &Path,
    profile: &mut RelayProfile,
    common_config_contents: &mut String,
) -> anyhow::Result<()> {
    let live_config = read_optional_text(&home.join("config.toml"))?;
    let template_config = profile.config_contents.clone();
    let template_auth = profile.auth_contents.clone();
    let template_api_key = relay_profile_api_key(profile);
    let template_base_url = relay_profile_base_url(profile);
    profile.config_contents = if profile.use_common_config {
        strip_common_config_from_config(&live_config, common_config_contents)?
    } else {
        ensure_trailing_newline(live_config.clone())
    };
    profile.config_contents =
        restore_profile_provider_id_for_backfill(&profile.config_contents, &template_config)?;
    if profile.protocol == RelayProtocol::Responses
        && provider_string_from_config(&profile.config_contents, "base_url").as_deref()
            == Some(
                crate::protocol_proxy::local_responses_proxy_base_url(
                    crate::protocol_proxy::DEFAULT_PROTOCOL_PROXY_PORT,
                )
                .as_str(),
            )
        && !template_base_url.trim().is_empty()
    {
        let mut doc = parse_toml_document(&profile.config_contents)?;
        let provider_id = active_or_default_provider_id(&doc);
        ensure_provider_table(&mut doc, &provider_id)?["base_url"] =
            toml_edit::value(template_base_url.trim());
        profile.config_contents =
            move_model_providers_before_profiles(&ensure_trailing_newline(doc.to_string()));
    }
    let live_auth = read_optional_text(&home.join("auth.json"))?;
    restore_profile_credentials_after_backfill(
        profile,
        &template_auth,
        &template_api_key,
        &live_auth,
    )?;
    sync_profile_mode_from_backfilled_live(profile);
    sync_context_limits_from_config(profile, &live_config);
    if profile.model.trim().is_empty() {
        if let Some(model) = root_key_string(&live_config, "model") {
            profile.model = model;
        }
    }
    // 从回填后的 config_contents 识别 Bedrock 配置，避免切走再切回时丢失 bedrock 字段
    profile.bedrock = bedrock_config_from_config_text(&profile.config_contents);
    Ok(())
}

pub fn extract_common_config_from_config(config_text: &str) -> anyhow::Result<String> {
    let mut doc = parse_toml_document(config_text)?;
    remove_provider_specific_common_keys(doc.as_table_mut());
    Ok(normalize_optional_toml(doc))
}

pub fn sanitize_common_config_contents(common_config: &str) -> String {
    match parse_toml_document(common_config) {
        Ok(mut doc) => {
            remove_provider_specific_common_keys(doc.as_table_mut());
            normalize_optional_toml(doc)
        }
        Err(_) => sanitize_common_config_text_fallback(common_config),
    }
}

pub fn strip_common_config_from_config(
    config_text: &str,
    common_config_contents: &str,
) -> anyhow::Result<String> {
    let trimmed = common_config_contents.trim();
    if trimmed.is_empty() {
        return Ok(normalize_duplicate_toml_text(config_text));
    }

    match (
        parse_toml_document(config_text),
        parse_toml_document(trimmed),
    ) {
        (Ok(mut target_doc), Ok(source_doc)) => {
            remove_toml_table_like(target_doc.as_table_mut(), source_doc.as_table());
            Ok(normalize_optional_toml(target_doc))
        }
        _ => Ok(strip_common_config_text_fallback(config_text, trimmed)),
    }
}

pub fn merge_common_config_into_config(
    config_text: &str,
    common_config_contents: &str,
) -> anyhow::Result<String> {
    let sanitized_common = sanitize_common_config_contents(common_config_contents);
    let trimmed = sanitized_common.trim();
    if trimmed.is_empty() {
        return Ok(ensure_trailing_newline(config_text.to_string()));
    }

    let mut target_doc = parse_toml_document(config_text)?;
    let profile_goals_override = target_doc
        .get("features")
        .and_then(Item::as_table_like)
        .and_then(|features| features.get("goals"))
        .and_then(Item::as_bool);
    let source_doc = parse_toml_document(trimmed)?;
    merge_toml_table_like(target_doc.as_table_mut(), source_doc.as_table());
    if let Some(enabled) = profile_goals_override {
        table_mut_or_insert(&mut target_doc, "features")?["goals"] = toml_edit::value(enabled);
    }
    Ok(normalize_optional_toml(target_doc))
}

pub fn list_context_entries_from_common_config(
    common_config: &str,
) -> anyhow::Result<CodexContextEntries> {
    let normalized = normalize_duplicate_toml_text(common_config);
    let doc = parse_toml_document(&normalized)?;
    Ok(CodexContextEntries {
        mcp_servers: list_context_entries_for_table(&doc, "mcp_servers"),
        plugins: list_context_entries_for_table(&doc, "plugins"),
    })
}

pub fn upsert_context_entry_in_common_config(
    common_config: &str,
    kind: &str,
    id: &str,
    toml_body: &str,
) -> anyhow::Result<String> {
    let id = id.trim();
    if id.is_empty() {
        anyhow::bail!("上下文 id 不能为空");
    }
    let table_name = context_table_name(kind)?;
    let body_doc = parse_toml_document(toml_body)?;
    let normalized = normalize_duplicate_toml_text(common_config);
    let mut doc = parse_toml_document(&normalized)?;
    if !doc.as_table().contains_key(table_name) {
        doc[table_name] = toml_edit::table();
    }
    if doc[table_name].as_table().is_none() {
        anyhow::bail!("{table_name} 必须是 TOML 表");
    }
    doc[table_name][id] = Item::Table(body_doc.as_table().clone());
    Ok(normalize_optional_toml(doc))
}

pub fn delete_context_entry_from_common_config(
    common_config: &str,
    kind: &str,
    id: &str,
) -> anyhow::Result<String> {
    let table_name = context_table_name(kind)?;
    let normalized = normalize_duplicate_toml_text(common_config);
    let mut doc = parse_toml_document(&normalized)?;
    if let Some(table) = doc[table_name].as_table_mut() {
        table.remove(id.trim());
        if table.is_empty() {
            doc.as_table_mut().remove(table_name);
        }
    }
    Ok(normalize_optional_toml(doc))
}

/// 剥掉通用配置里供应商各自持有的键，丢掉历史遗留的 `[skills.<id>]` 死表，
/// 再丢掉标记为 `enabled = false` 的上下文条目，得到本次切换真正要合并进
/// config.toml 的那份通用配置。
///
/// 条目启停以条目自身的 `enabled` 为唯一依据——旧版还存在一份「按供应商勾选」的
/// selection，两套机制重叠，空的 selection 会把 live config 里的 MCP 全清空，已移除。
pub fn prepare_common_config_for_apply(common_config: &str) -> anyhow::Result<String> {
    let sanitized_common =
        strip_legacy_skill_tables(&sanitize_common_config_contents(common_config));
    let mut filtered = parse_toml_document(&sanitized_common)?;
    remove_disabled_context_tables(filtered.as_table_mut());
    Ok(normalize_optional_toml(filtered))
}

pub fn sync_live_config_context_entries(
    live_config: &str,
    context_config: &str,
) -> anyhow::Result<String> {
    let normalized_live = normalize_duplicate_toml_text(live_config);
    let normalized_context = normalize_duplicate_toml_text(context_config);
    let mut live_doc = parse_toml_document(&normalized_live)?;
    if normalized_context.trim().is_empty() {
        return Ok(normalize_optional_toml(live_doc));
    }
    let managed_doc = parse_toml_document(&normalized_context)?;
    remove_managed_context_entries(live_doc.as_table_mut(), managed_doc.as_table());
    let mut context_doc = managed_doc;
    remove_disabled_context_tables(context_doc.as_table_mut());
    merge_managed_context_tables(live_doc.as_table_mut(), context_doc.as_table());
    Ok(normalize_optional_toml(live_doc))
}

fn preserve_unmanaged_live_context_entries(
    home: &Path,
    config_text: &str,
    managed_context_config: &str,
) -> anyhow::Result<String> {
    let live_config = read_optional_text(&home.join("config.toml"))?;
    if live_config.trim().is_empty() {
        return Ok(ensure_trailing_newline(config_text.to_string()));
    }
    let mut target_doc = parse_toml_document(config_text)?;
    let live_doc = parse_toml_document(&live_config)?;
    let managed_doc =
        parse_toml_document(&sanitize_common_config_contents(managed_context_config))?;
    preserve_unmanaged_context_tables(
        target_doc.as_table_mut(),
        live_doc.as_table(),
        managed_doc.as_table(),
    );
    Ok(normalize_optional_toml(target_doc))
}

fn merge_managed_context_tables(target: &mut toml_edit::Table, managed: &toml_edit::Table) {
    for table_name in CONTEXT_TABLE_NAMES {
        merge_managed_context_table(target, managed, table_name);
    }
}

fn merge_managed_context_table(
    target: &mut toml_edit::Table,
    managed: &toml_edit::Table,
    table_name: &str,
) {
    let Some(managed_item) = managed.get(table_name) else {
        return;
    };
    let Some(managed_table) = managed_item.as_table_like() else {
        return;
    };
    if target.get(table_name).is_none() {
        target[table_name] = toml_edit::table();
    }
    let Some(target_table) = target.get_mut(table_name).and_then(Item::as_table_like_mut) else {
        target[table_name] = managed_item.clone();
        return;
    };
    for (id, item) in managed_table.iter() {
        target_table.insert(id, item.clone());
    }
}

fn remove_managed_context_entries(target: &mut toml_edit::Table, managed: &toml_edit::Table) {
    for table_name in CONTEXT_TABLE_NAMES {
        remove_managed_context_entry_table(target, managed, table_name);
    }
}

fn remove_managed_context_entry_table(
    target: &mut toml_edit::Table,
    managed: &toml_edit::Table,
    table_name: &str,
) {
    let Some(managed_item) = managed.get(table_name) else {
        return;
    };
    let Some(managed_table) = managed_item.as_table_like() else {
        return;
    };
    let Some(target_table) = target.get_mut(table_name).and_then(Item::as_table_like_mut) else {
        return;
    };
    for (id, _) in managed_table.iter() {
        target_table.remove(id);
    }
}

fn preserve_unmanaged_context_tables(
    target: &mut toml_edit::Table,
    live: &toml_edit::Table,
    managed: &toml_edit::Table,
) {
    for table_name in CONTEXT_TABLE_NAMES {
        preserve_unmanaged_context_table(target, live, managed, table_name);
    }
}

fn preserve_unmanaged_context_table(
    target: &mut toml_edit::Table,
    live: &toml_edit::Table,
    managed: &toml_edit::Table,
    table_name: &str,
) {
    let Some(live_item) = live.get(table_name) else {
        return;
    };
    let Some(live_table) = live_item.as_table_like() else {
        return;
    };
    if target.get(table_name).is_none() {
        target[table_name] = toml_edit::table();
    }
    let Some(target_table) = target.get_mut(table_name).and_then(Item::as_table_like_mut) else {
        return;
    };
    let managed_ids = managed
        .get(table_name)
        .and_then(Item::as_table_like)
        .map(|table| {
            table
                .iter()
                .map(|(id, _)| id.to_string())
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default();
    for (id, item) in live_table.iter() {
        if !managed_ids.contains(id) && target_table.get(id).is_none() {
            target_table.insert(id, item.clone());
        }
    }
}

fn remove_disabled_context_tables(table: &mut toml_edit::Table) {
    for table_name in CONTEXT_TABLE_NAMES {
        let Some(item) = table.get_mut(table_name) else {
            continue;
        };
        let Some(context_table) = item.as_table_mut() else {
            continue;
        };
        let disabled_ids: Vec<String> = context_table
            .iter()
            .filter_map(|(id, item)| {
                let enabled = item.as_table().map(context_entry_enabled).unwrap_or(true);
                (!enabled).then_some(id.to_string())
            })
            .collect();
        for id in disabled_ids {
            context_table.remove(&id);
        }
    }
}

fn write_codex_live_atomic(
    home: &Path,
    config_text: Option<&str>,
    auth_bytes: Option<&[u8]>,
) -> anyhow::Result<Option<String>> {
    std::fs::create_dir_all(home)?;
    let config_path = home.join("config.toml");
    let auth_path = home.join("auth.json");
    #[cfg(windows)]
    let normalized_config_text = config_text.map(normalize_config_text_for_write);
    #[cfg(windows)]
    let config_text = normalized_config_text.as_deref();

    let config_text = match config_text {
        Some(config_text) => {
            let config_text = preserve_live_desktop_settings(home, config_text)?;
            Some(preserve_live_marketplace_configs(home, &config_text)?)
        }
        None => None,
    };
    let config_text = config_text.as_deref();

    let config_text = match config_text {
        Some(config_text) => Some(
            crate::plugin_marketplace::preserve_openai_curated_remote_marketplace_config(
                home,
                config_text,
            )?,
        ),
        None => None,
    };
    let config_text = config_text.as_deref();

    if let Some(config_text) = config_text {
        validate_toml_config(config_text, &config_path)?;
    }
    if let Some(auth_bytes) = auth_bytes {
        validate_auth_json(auth_bytes, &auth_path)?;
    }

    let old_config = read_optional_bytes(&config_path)?;
    let old_auth = read_optional_bytes(&auth_path)?;
    let backup_path = create_live_backup(home, old_config.as_deref(), old_auth.as_deref())?;
    let mut auth_written = false;

    if let Some(auth_bytes) = auth_bytes {
        if let Err(error) = crate::settings::atomic_write(&auth_path, auth_bytes) {
            return Err(error.context("写入 auth.json 失败"));
        }
        auth_written = true;
    }

    if let Some(config_text) = config_text {
        if let Err(error) = crate::settings::atomic_write(&config_path, config_text.as_bytes()) {
            if auth_written {
                let _ = restore_optional_file(&auth_path, old_auth.as_deref());
            }
            let _ = restore_optional_file(&config_path, old_config.as_deref());
            return Err(error.context("写入 config.toml 失败"));
        }
    }

    Ok(backup_path)
}

fn preserve_live_marketplace_configs(home: &Path, config_text: &str) -> anyhow::Result<String> {
    let live_config = read_optional_text(&home.join("config.toml"))?;
    if live_config.trim().is_empty() {
        return Ok(config_text.to_string());
    }

    let mut target = parse_toml_document(config_text)?;
    let live = parse_toml_document(&live_config)?;
    let Some(live_marketplaces) = live.get("marketplaces").and_then(Item::as_table_like) else {
        return Ok(ensure_trailing_newline(target.to_string()));
    };
    if live_marketplaces.is_empty() {
        return Ok(ensure_trailing_newline(target.to_string()));
    }

    if target.get("marketplaces").is_none() {
        target["marketplaces"] = toml_edit::table();
    }
    if target
        .get("marketplaces")
        .and_then(Item::as_table_like)
        .is_none()
    {
        target["marketplaces"] = toml_edit::table();
    }
    let Some(target_marketplaces) = target
        .get_mut("marketplaces")
        .and_then(Item::as_table_like_mut)
    else {
        return Ok(ensure_trailing_newline(target.to_string()));
    };

    for (name, marketplace) in live_marketplaces.iter() {
        if target_marketplaces.get(name).is_none() {
            target_marketplaces.insert(name, marketplace.clone());
        }
    }

    Ok(ensure_trailing_newline(target.to_string()))
}

fn active_provider_id(doc: &DocumentMut) -> Option<String> {
    doc.get("model_provider")
        .and_then(Item::as_str)
        .map(str::trim)
        .filter(|provider| !provider.is_empty())
        .map(ToString::to_string)
}

fn active_or_default_provider_id(doc: &DocumentMut) -> String {
    active_provider_id(doc)
        .filter(|provider| {
            is_custom_provider_id(provider) && !LEGACY_RELAY_PROVIDERS.contains(&provider.as_str())
        })
        .unwrap_or_else(|| RELAY_PROVIDER.to_string())
}

fn is_custom_provider_id(provider: &str) -> bool {
    !provider.is_empty() && !RESERVED_MODEL_PROVIDER_IDS.contains(&provider)
}

fn provider_table_exists(doc: &DocumentMut, provider_id: &str) -> bool {
    doc.get("model_providers")
        .and_then(Item::as_table)
        .and_then(|table| table.get(provider_id))
        .is_some()
}

fn parse_toml_document(contents: &str) -> anyhow::Result<DocumentMut> {
    let contents = contents.trim_start_matches('\u{feff}');
    if contents.trim().is_empty() {
        Ok(DocumentMut::new())
    } else {
        contents
            .parse::<DocumentMut>()
            .map_err(|error| anyhow::anyhow!("config.toml TOML 解析失败：{error}"))
    }
}

fn remove_provider_specific_common_keys(table: &mut dyn TableLike) {
    for key in PROVIDER_SPECIFIC_COMMON_ROOT_KEYS {
        table.remove(key);
    }
    let sensitive_keys: Vec<String> = table
        .iter()
        .map(|(key, _)| key.to_string())
        .filter(|key| is_provider_credential_root_key(key))
        .collect();
    for key in sensitive_keys {
        table.remove(&key);
    }
    table.remove("model_providers");
}

fn is_provider_specific_common_root_key(key: &str) -> bool {
    let key = key.trim().trim_matches(['\"', '\'']);
    PROVIDER_SPECIFIC_COMMON_ROOT_KEYS.contains(&key) || is_provider_credential_root_key(key)
}

fn is_provider_credential_root_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    matches!(
        key.as_str(),
        "api_key" | "access_token" | "bearer_token" | "experimental_bearer_token"
    ) || key.ends_with("_api_key")
        || key.ends_with("_access_token")
        || key.ends_with("_bearer_token")
}

fn sanitize_common_config_text_fallback(common_config: &str) -> String {
    let mut kept = Vec::new();
    let mut in_root = true;
    let mut skipping_model_providers = false;

    for line in common_config.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_root = false;
            skipping_model_providers =
                trimmed == "[model_providers]" || trimmed.starts_with("[model_providers.");
            if skipping_model_providers {
                continue;
            }
        } else if skipping_model_providers {
            continue;
        }

        if in_root {
            if let Some((key, _)) = trimmed.split_once('=') {
                if is_provider_specific_common_root_key(key) {
                    continue;
                }
            }
        }

        kept.push(line);
    }

    normalize_text_toml(kept.join("\n"))
}

fn normalize_text_toml(contents: String) -> String {
    let trimmed = contents.trim();
    if trimmed.is_empty() {
        String::new()
    } else {
        ensure_trailing_newline(trimmed.to_string())
    }
}

/// 丢掉历史遗留的 `[skills.<id>]` 表。
///
/// 这些条目是早期把 skill 当 config.toml 注册表管留下来的，codex 从来没读过它们
/// （`skills` 是个三字段结构体，`[skills.<id>]` 会被当未知字段丢掉）。skill 现在由
/// `$CODEX_HOME/skills/` 目录管，这里顺手把死数据从用户配置里清掉。
///
/// 注意只删 `[skills.<id>]` 子表，`[skills]` 本身是合法配置（bundled /
/// include_instructions / max_context_tokens），得留着。
pub fn strip_legacy_skill_tables(contents: &str) -> String {
    let Ok(mut doc) = parse_toml_document(contents) else {
        return contents.to_string();
    };
    let Some(skills) = doc.as_table_mut().get_mut("skills") else {
        return contents.to_string();
    };
    let Some(table) = skills.as_table_like_mut() else {
        return contents.to_string();
    };
    let legacy_ids: Vec<String> = table
        .iter()
        .filter(|(_, item)| item.is_table_like())
        .map(|(id, _)| id.to_string())
        .collect();
    if legacy_ids.is_empty() {
        return contents.to_string();
    }
    for id in legacy_ids {
        table.remove(&id);
    }
    if table.is_empty() {
        doc.as_table_mut().remove("skills");
    }
    normalize_optional_toml(doc)
}

pub fn normalize_config_text(contents: &str) -> String {
    normalize_duplicate_toml_text(contents)
}

fn normalize_duplicate_toml_text(contents: &str) -> String {
    let mut seen_root_keys = HashSet::new();
    let mut seen_headers = HashSet::new();
    let mut kept = Vec::new();
    let mut skipping_duplicate_table = false;
    let mut in_root = true;

    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_root = false;
            skipping_duplicate_table = !seen_headers.insert(trimmed.to_string());
            if skipping_duplicate_table {
                continue;
            }
            kept.push(line);
            continue;
        }

        if skipping_duplicate_table {
            continue;
        }

        if in_root && !trimmed.is_empty() && !trimmed.starts_with('#') {
            if let Some((key, _)) = trimmed.split_once('=') {
                let key = key.trim();
                if !key.is_empty() && !key.contains('.') && !seen_root_keys.insert(key.to_string())
                {
                    continue;
                }
            }
        }

        kept.push(line);
    }

    normalize_text_toml(kept.join("\n"))
}

fn strip_common_config_text_fallback(config_text: &str, common_config: &str) -> String {
    let normalized = normalize_duplicate_toml_text(config_text);
    let anchors = common_config_anchors(common_config);
    if anchors.root_keys.is_empty() && anchors.table_headers.is_empty() {
        return normalized;
    }

    let mut kept = Vec::new();
    let mut skipping_table = false;

    for line in normalized.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            skipping_table = anchors.table_headers.contains(trimmed);
            if skipping_table {
                continue;
            }
            kept.push(line);
            continue;
        }

        if skipping_table {
            continue;
        }

        if !trimmed.is_empty() && !trimmed.starts_with('#') {
            if let Some((key, _)) = trimmed.split_once('=') {
                if anchors.root_keys.contains(key.trim()) {
                    continue;
                }
            }
        }

        kept.push(line);
    }

    normalize_text_toml(kept.join("\n"))
}

struct CommonConfigAnchors {
    root_keys: HashSet<String>,
    table_headers: HashSet<String>,
}

fn common_config_anchors(common_config: &str) -> CommonConfigAnchors {
    let mut root_keys = HashSet::new();
    let mut table_headers = HashSet::new();
    let mut in_root = true;

    for line in common_config.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_root = false;
            table_headers.insert(trimmed.to_string());
            continue;
        }

        if in_root && !trimmed.is_empty() && !trimmed.starts_with('#') {
            if let Some((key, _)) = trimmed.split_once('=') {
                let key = key.trim();
                if !key.is_empty() {
                    root_keys.insert(key.to_string());
                }
            }
        }
    }

    CommonConfigAnchors {
        root_keys,
        table_headers,
    }
}

fn validate_toml_config(config_text: &str, path: &Path) -> anyhow::Result<()> {
    let config_text = config_text.trim_start_matches('\u{feff}');
    if config_text.trim().is_empty() {
        return Ok(());
    }
    config_text
        .parse::<toml::Table>()
        .with_context(|| format!("{} 不是有效 TOML", path.display()))?;
    Ok(())
}

fn normalize_config_text_for_write(config_text: &str) -> String {
    config_text.trim_start_matches('\u{feff}').to_string()
}

fn preserve_live_desktop_settings(home: &Path, config_text: &str) -> anyhow::Result<String> {
    let normalized = normalize_config_text_for_write(config_text);
    let mut target_doc = parse_toml_document(&normalized)?;
    remove_unsupported_approval_policies(&mut target_doc);
    let live_text = read_optional_text(&home.join("config.toml"))?;
    if live_text.trim().is_empty() {
        return Ok(normalize_optional_toml(target_doc));
    }
    let Ok(live_doc) = parse_toml_document(&live_text) else {
        return Ok(normalize_optional_toml(target_doc));
    };
    if let Some(live_desktop) = live_doc.get("desktop").cloned() {
        if !live_desktop.is_none() {
            merge_toml_item(&mut target_doc["desktop"], &live_desktop);
        }
    }
    for key in ["sandbox_mode", "approval_policy", "sandbox_workspace_write"] {
        if let Some(live_value) = live_doc.get(key).cloned() {
            merge_toml_item(&mut target_doc[key], &live_value);
        }
    }
    remove_unsupported_approval_policies(&mut target_doc);
    let context_usage_configured = target_doc
        .get("desktop")
        .and_then(Item::as_table)
        .and_then(|desktop| desktop.get("show-context-window-usage"))
        .is_some();
    if !context_usage_configured {
        if target_doc.get("desktop").is_none() {
            target_doc["desktop"] = toml_edit::table();
        }
        if let Some(desktop) = target_doc["desktop"].as_table_mut() {
            desktop["show-context-window-usage"] = toml_edit::value(true);
        }
    }
    Ok(normalize_optional_toml(target_doc))
}

fn remove_unsupported_approval_policies(doc: &mut DocumentMut) -> bool {
    let mut changed = false;
    if doc.get("approval_policy").and_then(Item::as_str) == Some("untrusted") {
        doc.as_table_mut().remove("approval_policy");
        changed = true;
    }
    if let Some(profiles) = doc.get_mut("profiles").and_then(Item::as_table_mut) {
        for (_, profile) in profiles.iter_mut() {
            let Some(profile) = profile.as_table_mut() else {
                continue;
            };
            if profile.get("approval_policy").and_then(Item::as_str) == Some("untrusted") {
                profile.remove("approval_policy");
                changed = true;
            }
        }
    }
    changed
}

pub fn cleanup_unsupported_approval_policies_in_home(home: &Path) -> anyhow::Result<bool> {
    let config_path = home.join("config.toml");
    let existing = match std::fs::read_to_string(&config_path) {
        Ok(existing) => existing,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    let mut doc = parse_toml_document(&existing)?;
    if !remove_unsupported_approval_policies(&mut doc) {
        return Ok(false);
    }
    crate::settings::atomic_write(&config_path, normalize_optional_toml(doc).as_bytes())?;
    Ok(true)
}

fn validate_auth_json(auth_bytes: &[u8], path: &Path) -> anyhow::Result<()> {
    if auth_bytes.iter().all(|byte| byte.is_ascii_whitespace()) {
        return Ok(());
    }
    serde_json::from_slice::<Value>(auth_bytes)
        .with_context(|| format!("{} 不是有效 JSON", path.display()))?;
    Ok(())
}

fn parse_optional_positive_u64(value: &str, label: &str) -> anyhow::Result<Option<u64>> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let parsed = trimmed
        .parse::<u64>()
        .with_context(|| format!("{label}必须是正整数"))?;
    if parsed == 0 {
        anyhow::bail!("{label}必须大于 0");
    }
    Ok(Some(parsed))
}

fn apply_context_limits_to_config(
    config_text: &str,
    context_window: &str,
    auto_compact_limit: &str,
) -> anyhow::Result<String> {
    let mut doc = parse_toml_document(config_text)?;
    if let Some(value) = parse_optional_positive_u64(context_window, "上下文大小")? {
        doc["model_context_window"] = toml_edit::value(value as i64);
    }
    if let Some(value) = parse_optional_positive_u64(auto_compact_limit, "压缩上下文大小")? {
        doc["model_auto_compact_token_limit"] = toml_edit::value(value as i64);
    }
    Ok(normalize_optional_toml(doc))
}

fn apply_model_catalog_to_config(
    home: &Path,
    profile: &RelayProfile,
    config_text: &str,
) -> anyhow::Result<String> {
    let catalog_relative = format!(
        "model-catalogs/{}.json",
        sanitize_catalog_filename(&profile.id)
    );
    let mut config_text = config_text.to_string();
    let custom_responses = custom_responses_provider(&config_text);
    // Catalog capabilities must follow the effective config, not stale profile URLs.
    let official_deepseek_responses =
        uses_official_deepseek_responses_for_config(profile, &config_text);
    // 用户已手写 model_catalog_json 指针时保留，不覆盖（保 preserves_user_model_catalog_json 测试）
    // 仅当现有指针指向本 profile 自己生成的 catalog 时才重新生成。
    // cc-switch 的固定文件名属于已知的其他管理器投影，不视为用户手写 catalog；
    // 切换到 Codex++ profile 时应接管，否则旧 catalog 会继续覆盖本 profile 的模型元数据。
    if let Some(existing) = root_key_string(&config_text, "model_catalog_json") {
        if existing != catalog_relative {
            if is_cc_switch_model_catalog(&existing) {
                config_text = remove_root_key(&config_text, "model_catalog_json");
            } else if official_deepseek_responses {
                return Ok(config_text.to_string());
            } else if custom_responses
                && copy_standard_responses_catalog(home, &existing, &catalog_relative)?
            {
                let mut doc = parse_toml_document(&config_text)?;
                doc["model_catalog_json"] = toml_edit::value(catalog_relative);
                return Ok(normalize_optional_toml(doc));
            } else {
                return Ok(config_text);
            }
        }
    }
    if !official_deepseek_responses
        && let Some(external_catalog) = live_external_model_catalog(home)
    {
        let mut doc = parse_toml_document(&config_text)?;
        if custom_responses
            && copy_standard_responses_catalog(home, &external_catalog, &catalog_relative)?
        {
            doc["model_catalog_json"] = toml_edit::value(catalog_relative);
        } else {
            doc["model_catalog_json"] = toml_edit::value(external_catalog);
        }
        return Ok(normalize_optional_toml(doc));
    }
    let (model_list, model_windows): (String, std::collections::HashMap<String, String>) =
        if profile.model_windows.trim().is_empty() && profile.model_list.contains('[') {
            crate::model_suffix::migrate_model_list_with_suffixes(&profile.model_list)
        } else {
            (
                profile.model_list.clone(),
                serde_json::from_str(&profile.model_windows).unwrap_or_default(),
            )
        };
    let entries =
        crate::model_suffix::collect_catalog_entries(&model_list, &model_windows, &profile.model);
    // Known bundled metadata entries need a catalog even without a user-supplied window.
    if !entries.iter().any(|entry| {
        entry.suffix_window.is_some()
            || crate::model_suffix::requires_bundled_metadata_catalog(&entry.slug)
            || (official_deepseek_responses && entry.slug.starts_with("deepseek-v4-"))
    }) {
        return Ok(config_text);
    }
    let fallback = parse_optional_positive_u64(&profile.context_window, "上下文大小")?;
    let catalog_path = home.join(&catalog_relative);
    if let Some(parent) = catalog_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Only custom Responses providers need the standard Responses tool wire format. Official
    // profiles and custom Chat Completions retain the model template's original Lite behavior.
    let catalog_json = crate::model_suffix::build_model_catalog_json_with_capabilities(
        &entries,
        fallback,
        None,
        custom_responses.then_some(false),
        official_deepseek_responses,
    );
    std::fs::write(&catalog_path, catalog_json)?;
    let mut doc = parse_toml_document(&config_text)?;
    doc["model_catalog_json"] = toml_edit::value(catalog_relative);
    Ok(normalize_optional_toml(doc))
}

pub(crate) fn uses_official_deepseek_responses(profile: &RelayProfile) -> bool {
    if profile.protocol != RelayProtocol::Responses {
        return false;
    }
    let resolved_base_url = relay_profile_base_url(profile);
    [
        profile.base_url.as_str(),
        profile.upstream_base_url.as_str(),
        resolved_base_url.as_str(),
    ]
    .iter()
    .any(|base_url| deepseek_api_base_url(base_url))
}

fn deepseek_api_base_url(base_url: &str) -> bool {
    let host = base_url
        .trim()
        .split("://")
        .nth(1)
        .unwrap_or(base_url)
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .split(':')
        .next()
        .unwrap_or_default()
        .trim_end_matches('.')
        .to_ascii_lowercase();
    host == "deepseek.com" || host.ends_with(".deepseek.com")
}

pub fn apply_deepseek_responses_compatibility(
    profile: &RelayProfile,
    config_text: &str,
) -> anyhow::Result<String> {
    if !uses_official_deepseek_responses_for_config(profile, config_text) {
        return Ok(config_text.to_string());
    }

    let mut doc = parse_toml_document(config_text)?;
    if doc.get("features").and_then(Item::as_table_like).is_none() {
        doc["features"] = toml_edit::table();
    }
    // DeepSeek Responses rejects Code Mode's custom `exec` tool. Unified Exec uses the
    // supported function tools `exec_command` and `write_stdin`, so preserve that setting.
    let features = doc
        .get_mut("features")
        .and_then(Item::as_table_like_mut)
        .expect("features table-like item was created above");
    features.insert("code_mode_only", toml_edit::value(false));
    if features
        .get("code_mode")
        .and_then(Item::as_table_like)
        .is_none()
    {
        features.insert("code_mode", toml_edit::table());
    }
    features
        .get_mut("code_mode")
        .and_then(Item::as_table_like_mut)
        .expect("code_mode table-like item was created above")
        .insert("enabled", toml_edit::value(false));
    Ok(normalize_optional_toml(doc))
}

fn uses_official_deepseek_responses_for_config(profile: &RelayProfile, config_text: &str) -> bool {
    if let Ok(doc) = parse_toml_document(config_text) {
        if let Some(provider_id) = active_provider_id(&doc) {
            if let Some(provider) = doc
                .get("model_providers")
                .and_then(Item::as_table)
                .and_then(|providers| providers.get(&provider_id))
                .and_then(Item::as_table_like)
            {
                let uses_responses = provider
                    .get("wire_api")
                    .and_then(Item::as_str)
                    .map(|wire_api| wire_api.trim().eq_ignore_ascii_case("responses"))
                    .unwrap_or(profile.protocol == RelayProtocol::Responses);
                if !uses_responses {
                    return false;
                }
                if let Some(base_url) = provider.get("base_url").and_then(Item::as_str) {
                    return deepseek_api_base_url(base_url);
                }
            }
        }

        if profile.protocol == RelayProtocol::Responses
            && let Some(base_url) = root_key_string(config_text, "base_url")
        {
            return deepseek_api_base_url(&base_url);
        }
    }

    uses_official_deepseek_responses(profile)
}

fn custom_responses_provider(config_text: &str) -> bool {
    let Ok(doc) = parse_toml_document(config_text) else {
        return false;
    };
    let Some(provider_id) = active_provider_id(&doc) else {
        return false;
    };
    if !is_custom_provider_id(&provider_id) {
        return false;
    }
    doc.get("model_providers")
        .and_then(Item::as_table)
        .and_then(|providers| providers.get(&provider_id))
        .and_then(Item::as_table_like)
        .and_then(|provider| provider.get("wire_api"))
        .and_then(Item::as_str)
        .is_some_and(|wire_api| wire_api.trim().eq_ignore_ascii_case("responses"))
}

fn copy_standard_responses_catalog(
    home: &Path,
    source: &str,
    target_relative: &str,
) -> anyhow::Result<bool> {
    let source_path = {
        let path = Path::new(source);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            home.join(path)
        }
    };
    let Ok(contents) = std::fs::read_to_string(source_path) else {
        return Ok(false);
    };
    let Ok(mut catalog) = serde_json::from_str::<Value>(&contents) else {
        return Ok(false);
    };
    let Some(models) = catalog.get_mut("models").and_then(Value::as_array_mut) else {
        return Ok(false);
    };
    let mut changed = false;
    for model in models {
        if model.get("use_responses_lite").and_then(Value::as_bool) == Some(true) {
            model["use_responses_lite"] = Value::Bool(false);
            changed = true;
        }
    }
    if !changed {
        return Ok(false);
    }

    let target = home.join(target_relative);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(target, serde_json::to_string_pretty(&catalog)?)?;
    Ok(true)
}

fn live_external_model_catalog(home: &Path) -> Option<String> {
    let live_text = read_optional_text(&home.join("config.toml")).ok()?;
    let live = parse_toml_document(&live_text).ok()?;
    let path = live.get("model_catalog_json")?.as_str()?.trim();
    (!path.is_empty()
        && !is_codex_plus_managed_model_catalog(home, path)
        && !is_cc_switch_model_catalog(path))
    .then(|| path.to_string())
}

fn is_cc_switch_model_catalog(path: &str) -> bool {
    path.trim()
        .replace('\\', "/")
        .rsplit('/')
        .next()
        .is_some_and(|name| name.eq_ignore_ascii_case(CC_SWITCH_MODEL_CATALOG_FILENAME))
}

fn is_codex_plus_managed_model_catalog(home: &Path, path: &str) -> bool {
    let normalized = path.trim().replace('\\', "/");
    let relative = normalized.trim_start_matches("./");
    if relative.to_ascii_lowercase().starts_with("model-catalogs/") {
        return true;
    }
    let normalized_lower = normalized.to_ascii_lowercase();
    if normalized_lower.contains("/model-catalogs/")
        || normalized_lower.ends_with("/model-catalogs")
    {
        return true;
    }
    let managed_root = home
        .join("model-catalogs")
        .to_string_lossy()
        .replace('\\', "/");
    let managed_root = managed_root.trim_end_matches('/');
    normalized.eq_ignore_ascii_case(managed_root)
        || normalized
            .get(..managed_root.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(managed_root))
            && normalized
                .as_bytes()
                .get(managed_root.len())
                .is_some_and(|byte| *byte == b'/')
}

fn sanitize_catalog_filename(id: &str) -> String {
    id.chars()
        .map(|char| {
            if char.is_ascii_alphanumeric() || char == '-' || char == '_' {
                char
            } else {
                '-'
            }
        })
        .collect()
}

fn sync_context_limits_from_config(profile: &mut RelayProfile, config_text: &str) {
    if let Some(value) = root_positive_int_string(config_text, "model_context_window") {
        profile.context_window = value;
    }
    if let Some(value) = root_positive_int_string(config_text, "model_auto_compact_token_limit") {
        profile.auto_compact_limit = value;
    }
}

fn root_positive_int_string(config_text: &str, key: &str) -> Option<String> {
    if let Ok(doc) = parse_toml_document(config_text) {
        if let Some(value) = doc
            .get(key)
            .and_then(Item::as_value)
            .and_then(toml_edit::Value::as_integer)
            .filter(|value| *value > 0)
        {
            return Some(value.to_string());
        }
    }

    root_key_value(config_text, key)
        .and_then(|value| value.split('#').next())
        .map(str::trim)
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .map(|value| value.to_string())
}

fn toml_value_is_subset(target: &toml_edit::Value, source: &toml_edit::Value) -> bool {
    match (target, source) {
        (toml_edit::Value::String(target), toml_edit::Value::String(source)) => {
            target.value() == source.value()
        }
        (toml_edit::Value::Integer(target), toml_edit::Value::Integer(source)) => {
            target.value() == source.value()
        }
        (toml_edit::Value::Float(target), toml_edit::Value::Float(source)) => {
            target.value() == source.value()
        }
        (toml_edit::Value::Boolean(target), toml_edit::Value::Boolean(source)) => {
            target.value() == source.value()
        }
        (toml_edit::Value::Datetime(target), toml_edit::Value::Datetime(source)) => {
            target.value() == source.value()
        }
        (toml_edit::Value::Array(target), toml_edit::Value::Array(source)) => {
            toml_array_contains_subset(target, source)
        }
        (toml_edit::Value::InlineTable(target), toml_edit::Value::InlineTable(source)) => {
            source.iter().all(|(key, source_item)| {
                target
                    .get(key)
                    .is_some_and(|target_item| toml_value_is_subset(target_item, source_item))
            })
        }
        _ => false,
    }
}

fn toml_array_contains_subset(target: &toml_edit::Array, source: &toml_edit::Array) -> bool {
    let mut matched = vec![false; target.len()];
    let target_items: Vec<&toml_edit::Value> = target.iter().collect();

    source.iter().all(|source_item| {
        if let Some((index, _)) = target_items
            .iter()
            .enumerate()
            .find(|(index, target_item)| {
                !matched[*index] && toml_value_is_subset(target_item, source_item)
            })
        {
            matched[index] = true;
            true
        } else {
            false
        }
    })
}

fn toml_remove_array_items(target: &mut toml_edit::Array, source: &toml_edit::Array) {
    for source_item in source.iter() {
        let index = {
            let target_items: Vec<&toml_edit::Value> = target.iter().collect();
            target_items
                .iter()
                .enumerate()
                .find(|(_, target_item)| toml_value_is_subset(target_item, source_item))
                .map(|(index, _)| index)
        };

        if let Some(index) = index {
            target.remove(index);
        }
    }
}

fn merge_toml_item(target: &mut Item, source: &Item) {
    if let Some(source_table) = source.as_table_like() {
        if let Some(target_table) = target.as_table_like_mut() {
            merge_toml_table_like(target_table, source_table);
            return;
        }
    }

    *target = source.clone();
}

fn merge_toml_table_like(target: &mut dyn TableLike, source: &dyn TableLike) {
    for (key, source_item) in source.iter() {
        match target.get_mut(key) {
            Some(target_item) => merge_toml_item(target_item, source_item),
            None => {
                target.insert(key, source_item.clone());
            }
        }
    }
}

fn remove_toml_item(target: &mut Item, source: &Item) {
    if let Some(source_table) = source.as_table_like() {
        if let Some(target_table) = target.as_table_like_mut() {
            remove_toml_table_like(target_table, source_table);
            if target_table.is_empty() {
                *target = Item::None;
            }
            return;
        }
    }

    if let Some(source_value) = source.as_value() {
        let mut remove_item = false;

        if let Some(target_value) = target.as_value_mut() {
            match (target_value, source_value) {
                (toml_edit::Value::Array(target_arr), toml_edit::Value::Array(source_arr)) => {
                    toml_remove_array_items(target_arr, source_arr);
                    remove_item = target_arr.is_empty();
                }
                (target_value, source_value)
                    if toml_value_is_subset(target_value, source_value) =>
                {
                    remove_item = true;
                }
                _ => {}
            }
        }

        if remove_item {
            *target = Item::None;
        }
    }
}

fn remove_toml_table_like(target: &mut dyn TableLike, source: &dyn TableLike) {
    let keys: Vec<String> = source.iter().map(|(key, _)| key.to_string()).collect();

    for key in keys {
        let mut remove_key = false;
        if let (Some(target_item), Some(source_item)) = (target.get_mut(&key), source.get(&key)) {
            remove_toml_item(target_item, source_item);
            remove_key = target_item.is_none()
                || target_item
                    .as_table_like()
                    .is_some_and(|table_like| table_like.is_empty());
        }

        if remove_key {
            target.remove(&key);
        }
    }
}

fn normalize_optional_toml(doc: DocumentMut) -> String {
    let contents = doc.to_string();
    if contents.trim().is_empty() {
        String::new()
    } else {
        ensure_trailing_newline(contents)
    }
}

fn list_context_entries_for_table(doc: &DocumentMut, table_name: &str) -> Vec<CodexContextEntry> {
    let Some(table) = doc.get(table_name).and_then(Item::as_table) else {
        return Vec::new();
    };
    table
        .iter()
        .filter_map(|(id, item)| {
            let table = item.as_table()?;
            let body = table_body_to_string(table);
            Some(CodexContextEntry {
                id: id.to_string(),
                kind: context_kind_name(table_name).to_string(),
                title: id.to_string(),
                summary: context_entry_summary(&body),
                toml_body: body,
                enabled: context_entry_enabled(table),
            })
        })
        .collect()
}

fn table_body_to_string(table: &Table) -> String {
    let mut doc = DocumentMut::new();
    merge_toml_table_like(doc.as_table_mut(), table);
    normalize_optional_toml(doc)
}

fn context_table_name(kind: &str) -> anyhow::Result<&'static str> {
    match kind {
        "mcp" | "mcpServer" | "mcpServers" => Ok("mcp_servers"),
        "plugin" | "plugins" => Ok("plugins"),
        other => anyhow::bail!("未知上下文类型：{other}"),
    }
}

fn context_kind_name(table: &str) -> &'static str {
    match table {
        "mcp_servers" => "mcp",
        "plugins" => "plugin",
        _ => "unknown",
    }
}

fn context_entry_summary(body: &str) -> String {
    body.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .unwrap_or("")
        .chars()
        .take(96)
        .collect()
}

fn context_entry_enabled(table: &Table) -> bool {
    if table
        .get("enabled")
        .and_then(|value| value.as_bool())
        .is_some_and(|enabled| !enabled)
    {
        return false;
    }
    if table
        .get("disabled")
        .and_then(|value| value.as_bool())
        .is_some_and(|disabled| disabled)
    {
        return false;
    }
    true
}

fn set_provider_id(doc: &mut DocumentMut, provider_id: &str) {
    doc["model_provider"] = toml_edit::value(provider_id);
}

fn restore_profile_provider_id_for_backfill(
    live_config: &str,
    template_config: &str,
) -> anyhow::Result<String> {
    let Some(template_provider_id) = provider_id_with_table_from_config(template_config)? else {
        return Ok(ensure_trailing_newline(live_config.to_string()));
    };
    if live_config.trim().is_empty() {
        return Ok(ensure_trailing_newline(live_config.to_string()));
    }

    let mut doc = parse_toml_document(live_config)?;
    let Some(live_provider_id) = active_provider_id(&doc) else {
        return Ok(ensure_trailing_newline(doc.to_string()));
    };
    if live_provider_id == template_provider_id {
        return Ok(ensure_trailing_newline(doc.to_string()));
    }
    if live_provider_id != RELAY_PROVIDER || template_provider_id == RELAY_PROVIDER {
        return Ok(ensure_trailing_newline(doc.to_string()));
    }
    if !provider_table_exists(&doc, &live_provider_id) {
        return Ok(ensure_trailing_newline(doc.to_string()));
    }

    rename_provider_table(&mut doc, &live_provider_id, &template_provider_id);
    rewrite_profile_provider_refs(&mut doc, &live_provider_id, &template_provider_id);
    set_provider_id(&mut doc, &template_provider_id);
    Ok(ensure_trailing_newline(doc.to_string()))
}

fn provider_id_with_table_from_config(config_text: &str) -> anyhow::Result<Option<String>> {
    if config_text.trim().is_empty() {
        return Ok(None);
    }
    let doc = parse_toml_document(config_text)?;
    let Some(provider_id) = active_provider_id(&doc) else {
        return Ok(None);
    };
    Ok(provider_table_exists(&doc, &provider_id).then_some(provider_id))
}

fn restore_profile_credentials_after_backfill(
    profile: &mut RelayProfile,
    template_auth: &str,
    template_api_key: &str,
    live_auth: &str,
) -> anyhow::Result<()> {
    if profile.uses_no_auth() {
        profile.config_contents =
            remove_experimental_bearer_token_from_config(&profile.config_contents)?;
        profile.auth_contents = no_auth_auth_contents(template_auth)?;
        profile.api_key.clear();
        return Ok(());
    }
    if profile.relay_mode == crate::settings::RelayMode::PureApi {
        profile.config_contents =
            remove_experimental_bearer_token_from_config(&profile.config_contents)?;
        profile.auth_contents =
            set_openai_api_key_in_auth_contents(template_auth, template_api_key)?;
        profile.api_key = template_api_key.trim().to_string();
        return Ok(());
    }

    if profile.relay_mode == crate::settings::RelayMode::Official && profile.official_mix_api_key {
        profile.auth_contents = remove_openai_api_key_from_auth_contents(live_auth)?;
        profile.config_contents =
            set_experimental_bearer_token_in_config(&profile.config_contents, template_api_key)?;
        profile.api_key = template_api_key.trim().to_string();
        return Ok(());
    }

    profile.auth_contents = live_auth.to_string();
    let Some(token) = experimental_bearer_token_from_config(&profile.config_contents)? else {
        return Ok(());
    };
    profile.api_key = token.clone();

    if !profile.auth_contents.trim().is_empty() {
        if codex_auth_api_key(&profile.auth_contents).is_none() {
            return Ok(());
        }
        profile.config_contents =
            remove_experimental_bearer_token_from_config(&profile.config_contents)?;
        return Ok(());
    }

    profile.config_contents =
        remove_experimental_bearer_token_from_config(&profile.config_contents)?;
    profile.auth_contents = set_openai_api_key_in_auth_contents(template_auth, &token)?;
    Ok(())
}

fn set_openai_api_key_in_auth_contents(
    auth_contents: &str,
    api_key: &str,
) -> anyhow::Result<String> {
    let mut auth = if auth_contents.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str::<Value>(auth_contents).with_context(|| "auth.json JSON 解析失败")?
    };
    if !auth.is_object() {
        auth = json!({});
    }
    if let Some(auth_object) = auth.as_object_mut() {
        if api_key.trim().is_empty() {
            auth_object.remove("OPENAI_API_KEY");
        } else {
            auth_object.insert(
                "OPENAI_API_KEY".to_string(),
                Value::String(api_key.trim().to_string()),
            );
        }
    } else {
        anyhow::bail!("auth.json 必须是 JSON 对象");
    }
    Ok(serde_json::to_string_pretty(&auth)?)
}

fn set_experimental_bearer_token_in_config(
    config_contents: &str,
    api_key: &str,
) -> anyhow::Result<String> {
    let mut doc = parse_toml_document(config_contents)?;
    let session_provider_id = active_provider_id(&doc);
    let provider_id = if session_provider_id.as_deref() == Some("openai") {
        RELAY_PROVIDER.to_string()
    } else {
        active_or_default_provider_id(&doc)
    };
    let provider = ensure_provider_table(&mut doc, &provider_id)?;
    if api_key.trim().is_empty() {
        provider.remove("experimental_bearer_token");
    } else {
        provider["experimental_bearer_token"] = toml_edit::value(api_key.trim());
    }
    Ok(move_model_providers_before_profiles(
        &ensure_trailing_newline(doc.to_string()),
    ))
}

fn sync_profile_mode_from_backfilled_live(profile: &mut RelayProfile) {
    if profile.uses_no_auth() {
        profile.relay_mode = crate::settings::RelayMode::PureApi;
        profile.official_mix_api_key = false;
        return;
    }
    if profile.relay_mode == crate::settings::RelayMode::Official && !profile.official_mix_api_key {
        return;
    }

    if codex_auth_api_key(&profile.auth_contents)
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
    {
        profile.relay_mode = crate::settings::RelayMode::PureApi;
        profile.official_mix_api_key = false;
        return;
    }

    let has_provider_endpoint = provider_string_from_config(&profile.config_contents, "base_url")
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty());
    if has_provider_endpoint || !profile.api_key.trim().is_empty() {
        profile.relay_mode = crate::settings::RelayMode::Official;
        profile.official_mix_api_key = true;
    }
}

fn official_profile_auth_for_switch(home: &Path, auth_contents: &str) -> anyhow::Result<String> {
    let source = if auth_contents.trim().is_empty() {
        read_optional_text(&home.join("auth.json"))?
    } else {
        auth_contents.to_string()
    };
    remove_openai_api_key_from_auth_contents(&source)
}

fn codex_auth_api_key(auth_contents: &str) -> Option<String> {
    let auth: Value = serde_json::from_str(auth_contents).ok()?;
    auth.get("OPENAI_API_KEY")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(ToString::to_string)
}

/// 解析 profile 實際使用的模型：優先取 config.toml 裡的 `model =`，
/// 否則退回 profile.model 欄位。供應商測試用它做回退，避免串到別家供應商的模型名。
pub fn relay_profile_model(profile: &RelayProfile) -> String {
    root_key_string(&profile.config_contents, "model")
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| profile.model.trim().to_string())
}

pub fn relay_profile_base_url(profile: &RelayProfile) -> String {
    if profile.relay_mode == crate::settings::RelayMode::Aggregate {
        return crate::protocol_proxy::local_responses_proxy_base_url(
            crate::protocol_proxy::DEFAULT_PROTOCOL_PROXY_PORT,
        );
    }
    if profile.has_model_routes() {
        if !profile.upstream_base_url.trim().is_empty() {
            return profile.upstream_base_url.trim().to_string();
        }
        if !profile.base_url.trim().is_empty()
            && profile.base_url.trim()
                != crate::protocol_proxy::local_responses_proxy_base_url(
                    crate::protocol_proxy::DEFAULT_PROTOCOL_PROXY_PORT,
                )
        {
            return profile.base_url.trim().to_string();
        }
    }
    if profile.protocol == RelayProtocol::ChatCompletions {
        if !profile.upstream_base_url.trim().is_empty() {
            return profile.upstream_base_url.trim().to_string();
        }
        if let Some(value) = root_key_string(&profile.config_contents, CHAT_UPSTREAM_BASE_URL_KEY)
            .filter(|value| !value.trim().is_empty())
        {
            return value;
        }
        if !profile.base_url.trim().is_empty() {
            return profile.base_url.trim().to_string();
        }
    }
    let provider_base_url = provider_string_from_config(&profile.config_contents, "base_url")
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_default();
    if profile.protocol == RelayProtocol::ChatCompletions
        && provider_base_url
            == crate::protocol_proxy::local_responses_proxy_base_url(
                crate::protocol_proxy::DEFAULT_PROTOCOL_PROXY_PORT,
            )
    {
        String::new()
    } else if !provider_base_url.is_empty() {
        provider_base_url
    } else {
        profile.base_url.trim().to_string()
    }
}

pub fn relay_profile_api_key(profile: &RelayProfile) -> String {
    if profile.uses_no_auth() {
        return String::new();
    }
    if profile.relay_mode == crate::settings::RelayMode::Aggregate {
        return "codex-plus-aggregate".to_string();
    }
    if profile.relay_mode == crate::settings::RelayMode::Official {
        return experimental_bearer_token_from_config(&profile.config_contents)
            .ok()
            .flatten()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| profile.api_key.trim().to_string());
    }
    codex_auth_api_key(&profile.auth_contents)
        .or_else(|| {
            experimental_bearer_token_from_config(&profile.config_contents)
                .ok()
                .flatten()
        })
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| profile.api_key.trim().to_string())
}

fn complete_relay_profile_config(profile: &RelayProfile) -> anyhow::Result<String> {
    if let Some(bedrock) = &profile.bedrock {
        return match bedrock.auth_mode {
            BedrockAuthMode::BearerToken => complete_bedrock_bearer_token_config(profile, bedrock),
            BedrockAuthMode::AwsProfile => complete_bedrock_aws_profile_config(profile, bedrock),
        };
    }

    let mut doc = parse_toml_document(&profile.config_contents)?;
    let session_provider_id = active_session_provider_id(&doc);
    let uses_openai_provider = session_provider_id == "openai";
    if uses_openai_provider && profile.protocol != RelayProtocol::Responses {
        anyhow::bail!("OpenAI 会话身份仅支持 Responses API");
    }
    update_remote_control_openai_base_url(
        &mut doc,
        uses_openai_provider
            || (profile.relay_mode == crate::settings::RelayMode::Official
                && profile.official_mix_api_key),
    );
    // `openai` is the Remote session identity, while the actual relay
    // transport remains in `model_providers.custom`.
    let provider_id = active_or_default_provider_id(&doc);
    let transport_provider_id = if uses_openai_provider {
        RELAY_PROVIDER.to_string()
    } else {
        provider_id.clone()
    };
    set_provider_id(
        &mut doc,
        if uses_openai_provider {
            "openai"
        } else {
            &provider_id
        },
    );

    let mut model = relay_profile_model(profile);
    // 若用户未填写默认模型，但 model_list 有内容，则取第一条作为默认 model，
    // 避免 codex 启动时回退到历史会话中带后缀的模型名。
    if model.trim().is_empty() && !profile.model_list.trim().is_empty() {
        if let Some(first) = profile
            .model_list
            .split(['\r', '\n', ','])
            .map(str::trim)
            .find(|value| !value.is_empty())
        {
            model = crate::model_suffix::parse_model_suffix(first).0;
        }
    }
    // 若用户把后缀语法（如 deepseek-v4-flash[1M]）写在 model 字段，
    // 写入 config.toml 前需剥离后缀；codex 本身不理解后缀，只会按原串匹配 catalog slug。
    let (model, _) = crate::model_suffix::parse_model_suffix(&model);
    if !model.trim().is_empty() {
        doc["model"] = toml_edit::value(model.trim());
    }

    let base_url = relay_profile_base_url(profile);
    let api_key = relay_profile_api_key(profile);
    doc.as_table_mut().remove(CHAT_UPSTREAM_BASE_URL_KEY);
    retain_only_provider_table(&mut doc, &transport_provider_id);
    for legacy_provider in LEGACY_RELAY_PROVIDERS {
        if transport_provider_id != *legacy_provider {
            remove_provider_table(&mut doc, legacy_provider);
        }
    }
    let provider = ensure_provider_table(&mut doc, &transport_provider_id)?;
    if provider
        .get("name")
        .and_then(Item::as_str)
        .map(str::trim)
        .is_none_or(str::is_empty)
    {
        provider["name"] = toml_edit::value(transport_provider_id.as_str());
    }
    if provider
        .get("wire_api")
        .and_then(Item::as_str)
        .map(str::trim)
        .is_none_or(str::is_empty)
    {
        provider["wire_api"] = toml_edit::value("responses");
    }
    if profile.uses_no_auth() {
        provider["requires_openai_auth"] = toml_edit::value(true);
    } else if profile.relay_mode != crate::settings::RelayMode::PureApi
        && provider
            .get("requires_openai_auth")
            .and_then(Item::as_bool)
            .is_none()
    {
        provider["requires_openai_auth"] = toml_edit::value(true);
    }
    let provider_base_url = if profile.has_model_routes() || profile.uses_no_auth() {
        crate::protocol_proxy::local_responses_proxy_base_url(
            crate::protocol_proxy::DEFAULT_PROTOCOL_PROXY_PORT,
        )
    } else {
        codex_base_url_for_protocol(
            base_url.trim(),
            profile.protocol,
            crate::protocol_proxy::DEFAULT_PROTOCOL_PROXY_PORT,
        )
    };
    if !provider_base_url.trim().is_empty() {
        provider["base_url"] = toml_edit::value(provider_base_url.trim());
    }
    if profile.uses_no_auth() {
        provider["experimental_bearer_token"] =
            toml_edit::value(crate::protocol_proxy::NO_AUTH_PROXY_BEARER_TOKEN);
    } else if profile.relay_mode == crate::settings::RelayMode::PureApi {
        provider.remove("experimental_bearer_token");
    } else if !api_key.trim().is_empty() {
        provider["experimental_bearer_token"] = toml_edit::value(api_key.trim());
    }

    Ok(move_model_providers_before_profiles(
        &ensure_trailing_newline(doc.to_string()),
    ))
}

pub fn normalize_relay_profile_for_storage(profile: &mut RelayProfile) -> anyhow::Result<()> {
    // Secondary validation for Bedrock BearerToken path — last line of defense before persisting.
    if let Some(bedrock) = &profile.bedrock {
        if bedrock.auth_mode == BedrockAuthMode::BearerToken {
            validate_custom_provider_id(&bedrock.provider_id).map_err(|e| anyhow::anyhow!(e))?;
            require_non_empty(&bedrock.region, "AWS 区域")?;
            require_non_empty(&relay_profile_api_key(profile), "Bedrock API Key")?;
        }
    }

    profile.no_auth = profile.relay_mode == crate::settings::RelayMode::PureApi && profile.no_auth;
    if profile.no_auth {
        profile.api_key.clear();
        profile.sub2api_enabled = false;
        profile.sub2api_multiplier.clear();
    }
    let mut seen_models = HashSet::new();
    profile.model_routes = profile
        .model_routes
        .drain(..)
        .filter_map(|mut route| {
            route.model = route.model.trim().to_string();
            route.target_relay_id = route.target_relay_id.trim().to_string();
            route.target_model = route.target_model.trim().to_string();
            if route.model.is_empty()
                || route.target_relay_id.is_empty()
                || !seen_models.insert(route.model.clone())
            {
                None
            } else {
                Some(route)
            }
        })
        .collect();
    if profile.model_windows.trim().is_empty() && profile.model_list.contains('[') {
        let (clean_list, windows) =
            crate::model_suffix::migrate_model_list_with_suffixes(&profile.model_list);
        profile.model_list = clean_list;
        profile.model_windows = serde_json::to_string(&windows).unwrap_or_default();
    }
    if profile.relay_mode == crate::settings::RelayMode::Official && !profile.official_mix_api_key {
        let has_api_config = !profile.base_url.trim().is_empty()
            || !profile.api_key.trim().is_empty()
            || codex_auth_api_key(&profile.auth_contents).is_some()
            || config_has_model_provider(profile.config_contents.as_str());
        if has_api_config {
            profile.config_contents.clear();
        }
        if !profile.model_list.trim().is_empty() {
            profile.model_list = merge_model_into_model_list(&profile.model, &profile.model_list);
        }
        profile.model.clear();
        profile.base_url.clear();
        profile.upstream_base_url.clear();
        profile.api_key.clear();
        profile.model_routes.clear();
        if auth_contents_looks_like_chatgpt_auth(&profile.auth_contents) {
            profile.auth_contents =
                remove_openai_api_key_from_auth_contents(&profile.auth_contents)?;
        } else {
            profile.auth_contents.clear();
        }
        return Ok(());
    }
    let source_base_url = relay_profile_base_url(profile);
    let source_api_key = relay_profile_api_key(profile);
    if !profile.config_contents.trim().is_empty()
        || profile.relay_mode == crate::settings::RelayMode::PureApi
        || profile.official_mix_api_key
    {
        profile.config_contents = complete_relay_profile_config(profile)?;
    }
    // PureApi 模式下 `complete_relay_profile_config` 会移除 config.toml 里的
    // `experimental_bearer_token`，auth.json 是 key 唯一的落点。
    // 这里过去还要求 auth_contents 为空，于是「非空但不含 OPENAI_API_KEY」的 auth.json
    // （例如退出 ChatGPT 登录后残留的 tokens/last_refresh）会把写入挡掉，
    // key 两边都没有，Codex CLI 只能回退到 OPENAI_API_KEY 环境变量，上游返回 401（issue #1965）。
    if profile.relay_mode == crate::settings::RelayMode::PureApi
        && !source_api_key.trim().is_empty()
    {
        profile.auth_contents =
            set_openai_api_key_in_auth_contents(&profile.auth_contents, &source_api_key)
                // auth.json 本身已损坏时保不住原内容，但 key 必须有落点，直接重建。
                .or_else(|_| set_openai_api_key_in_auth_contents("", &source_api_key))?;
    }
    if profile.uses_no_auth() {
        profile.auth_contents = no_auth_auth_contents(&profile.auth_contents)?;
    }
    if profile.relay_mode == crate::settings::RelayMode::Official {
        profile.auth_contents = remove_openai_api_key_from_auth_contents(&profile.auth_contents)?;
    }
    profile.model = relay_profile_model(profile);
    profile.model_list = merge_model_into_model_list(&profile.model, &profile.model_list);
    profile.upstream_base_url = source_base_url.clone();
    profile.base_url = source_base_url;
    profile.api_key = relay_profile_api_key(profile);
    Ok(())
}

fn remove_openai_api_key_from_auth_contents(auth_contents: &str) -> anyhow::Result<String> {
    if auth_contents.trim().is_empty() {
        return Ok(String::new());
    }
    let mut value =
        serde_json::from_str::<Value>(auth_contents).with_context(|| "auth.json JSON 解析失败")?;
    let Some(object) = value.as_object_mut() else {
        anyhow::bail!("auth.json 必须是 JSON 对象");
    };
    object.remove("OPENAI_API_KEY");
    if object.is_empty() {
        return Ok(String::new());
    }
    Ok(format!("{}\n", serde_json::to_string_pretty(&value)?))
}

fn no_auth_auth_contents(auth_contents: &str) -> anyhow::Result<String> {
    let mut value = if auth_contents.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str::<Value>(auth_contents).with_context(|| "auth.json JSON 解析失败")?
    };
    let Some(object) = value.as_object_mut() else {
        anyhow::bail!("auth.json 必须是 JSON 对象");
    };
    object.remove("OPENAI_API_KEY");
    Ok(format!("{}\n", serde_json::to_string_pretty(&value)?))
}

fn merge_model_into_model_list(model: &str, model_list: &str) -> String {
    let model = model.trim();
    let mut models = Vec::new();
    if !model.is_empty() {
        models.push(model.to_string());
    }
    for item in model_list.split(['\r', '\n', ',']).map(str::trim) {
        if !item.is_empty() && !models.iter().any(|existing| existing == item) {
            models.push(item.to_string());
        }
    }
    models.join("\n")
}

fn config_has_model_provider(config_contents: &str) -> bool {
    parse_toml_document(config_contents)
        .ok()
        .and_then(|doc| {
            doc.get("model_provider")
                .and_then(Item::as_str)
                .map(str::to_string)
        })
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
}

fn auth_contents_looks_like_chatgpt_auth(contents: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(contents) else {
        return false;
    };
    let is_chatgpt = value
        .get("auth_mode")
        .and_then(Value::as_str)
        .map(|mode| mode.eq_ignore_ascii_case("chatgpt"))
        .unwrap_or(false);
    is_chatgpt
        && value
            .get("tokens")
            .map(tokens_have_login_secret)
            .unwrap_or(false)
}

fn provider_string_from_config(config_contents: &str, key: &str) -> Option<String> {
    let doc = parse_toml_document(config_contents).ok()?;
    let active = active_provider_id(&doc);
    if let Some(provider_id) = active.as_deref() {
        if let Some(value) = doc
            .get("model_providers")
            .and_then(Item::as_table)
            .and_then(|providers| providers.get(provider_id))
            .and_then(Item::as_table)
            .and_then(|provider| provider.get(key))
            .and_then(Item::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(value.to_string());
        }
    }

    for provider in provider_tables(&doc) {
        if let Some(value) = provider
            .get(key)
            .and_then(Item::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(value.to_string());
        }
    }
    None
}

fn experimental_bearer_token_from_config(config_contents: &str) -> anyhow::Result<Option<String>> {
    let doc = parse_toml_document(config_contents)?;
    if let Some(provider_id) = active_provider_id(&doc) {
        if let Some(token) = provider_token_from_table(&doc, &provider_id) {
            return Ok(Some(token));
        }
        // OpenAI is the session identity, while Codex++ keeps relay
        // credentials in the custom transport table.
        if provider_id == "openai" {
            if let Some(token) = provider_token_from_table(&doc, RELAY_PROVIDER) {
                return Ok(Some(token));
            }
        }
    }
    Ok(None)
}

fn provider_token_from_table(doc: &DocumentMut, provider_id: &str) -> Option<String> {
    doc.get("model_providers")
        .and_then(Item::as_table)
        .and_then(|providers| providers.get(provider_id))
        .and_then(Item::as_table)
        .and_then(|provider| provider.get("experimental_bearer_token"))
        .and_then(Item::as_str)
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(ToString::to_string)
}

fn remove_experimental_bearer_token_from_config(config_contents: &str) -> anyhow::Result<String> {
    let mut doc = parse_toml_document(config_contents)?;
    if let Some(providers) = doc.get_mut("model_providers").and_then(Item::as_table_mut) {
        for (_, item) in providers.iter_mut() {
            if let Some(provider) = item.as_table_like_mut() {
                provider.remove("experimental_bearer_token");
            }
        }
    }
    Ok(ensure_trailing_newline(doc.to_string()))
}

fn provider_tables(doc: &DocumentMut) -> Vec<&dyn TableLike> {
    let mut tables: Vec<&dyn TableLike> = Vec::new();
    if let Some(providers) = doc.get("model_providers").and_then(Item::as_table) {
        for (_, item) in providers.iter() {
            if let Some(provider) = item.as_table_like() {
                tables.push(provider);
            }
        }
    }
    tables
}

fn ensure_provider_table<'a>(
    doc: &'a mut DocumentMut,
    provider_id: &str,
) -> anyhow::Result<&'a mut Table> {
    let providers = table_mut_or_insert(doc, "model_providers")?;
    if !providers.contains_key(provider_id)
        || providers
            .get(provider_id)
            .and_then(Item::as_table)
            .is_none()
    {
        providers.insert(provider_id, toml_edit::table());
    }
    providers
        .get_mut(provider_id)
        .and_then(Item::as_table_mut)
        .ok_or_else(|| anyhow::anyhow!("model_providers.{provider_id} 必须是 TOML table"))
}

fn remove_provider_table(doc: &mut DocumentMut, provider_id: &str) {
    if let Some(providers) = doc.get_mut("model_providers").and_then(Item::as_table_mut) {
        providers.remove(provider_id);
        if providers.is_empty() {
            doc.as_table_mut().remove("model_providers");
        }
    }
}

fn retain_only_provider_table(doc: &mut DocumentMut, provider_id: &str) {
    if let Some(providers) = doc.get_mut("model_providers").and_then(Item::as_table_mut) {
        let provider = providers
            .remove(provider_id)
            .unwrap_or_else(toml_edit::table);
        providers.clear();
        providers.insert(provider_id, provider);
    }
}

fn rename_provider_table(doc: &mut DocumentMut, from: &str, to: &str) {
    if from == to {
        return;
    }
    if let Some(providers) = doc.get_mut("model_providers").and_then(Item::as_table_mut) {
        let moved = providers.remove(from).unwrap_or_else(toml_edit::table);
        providers.insert(to, moved);
    }
}

fn rewrite_profile_provider_refs(doc: &mut DocumentMut, from: &str, to: &str) {
    let Some(profiles) = doc.get_mut("profiles").and_then(Item::as_table_mut) else {
        return;
    };
    for (_, item) in profiles.iter_mut() {
        let Some(profile) = item.as_table_mut() else {
            continue;
        };
        if profile
            .get("model_provider")
            .and_then(Item::as_str)
            .is_some_and(|provider| provider == from)
        {
            profile.insert("model_provider", toml_edit::value(to));
        }
    }
}

fn read_optional_text(path: &Path) -> anyhow::Result<String> {
    match std::fs::read_to_string(path) {
        Ok(contents) => Ok(contents),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error.into()),
    }
}

fn read_optional_bytes(path: &Path) -> anyhow::Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn restore_optional_file(path: &Path, contents: Option<&[u8]>) -> anyhow::Result<()> {
    match contents {
        Some(contents) => crate::settings::atomic_write(path, contents),
        None => match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        },
    }
}

fn create_live_backup(
    home: &Path,
    config: Option<&[u8]>,
    auth: Option<&[u8]>,
) -> anyhow::Result<Option<String>> {
    if config.is_none() && auth.is_none() {
        return Ok(None);
    }

    let backup_dir = home
        .join("backups")
        .join(format!("codex-plus-live-{}", timestamp_millis()));
    std::fs::create_dir_all(&backup_dir)?;
    if let Some(config) = config {
        std::fs::write(backup_dir.join("config.toml"), config)?;
    }
    if let Some(auth) = auth {
        std::fs::write(backup_dir.join("auth.json"), auth)?;
    }
    Ok(Some(backup_dir.to_string_lossy().to_string()))
}

fn timestamp_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn ensure_trailing_newline(mut contents: String) -> String {
    if !contents.ends_with('\n') {
        contents.push('\n');
    }
    contents
}

fn move_model_providers_before_profiles(contents: &str) -> String {
    let lines = contents.lines().collect::<Vec<_>>();
    let Some(provider_start) = lines
        .iter()
        .position(|line| line.trim_start().starts_with("[model_providers."))
    else {
        return ensure_trailing_newline(contents.to_string());
    };
    let provider_end = lines[provider_start + 1..]
        .iter()
        .position(|line| line.trim_start().starts_with('['))
        .map(|offset| provider_start + 1 + offset)
        .unwrap_or(lines.len());
    let Some(profile_start) = lines
        .iter()
        .position(|line| line.trim_start().starts_with("[profiles."))
    else {
        return ensure_trailing_newline(contents.to_string());
    };
    if provider_start < profile_start {
        return ensure_trailing_newline(contents.to_string());
    }

    let mut output = Vec::with_capacity(lines.len());
    output.extend_from_slice(&lines[..profile_start]);
    output.extend_from_slice(&lines[provider_start..provider_end]);
    if output.last().is_some_and(|line| !line.trim().is_empty()) {
        output.push("");
    }
    output.extend_from_slice(&lines[profile_start..provider_start]);
    output.extend_from_slice(&lines[provider_end..]);
    ensure_trailing_newline(output.join("\n"))
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

// ---------------------------------------------------------------------------
// Bedrock Mantle URL 模板常量
//
// 与 TS 端 `apps/codex-plus-manager/src/bedrock-config.ts` 的
// `BEDROCK_MANTLE_URL_PREFIX` / `BEDROCK_MANTLE_URL_SUFFIX` 保持一致。
// 修改这里必须同步修改 TS 常量（前后端都在生成与识别端使用）。
// ---------------------------------------------------------------------------
pub(crate) const BEDROCK_MANTLE_URL_PREFIX: &str = "https://bedrock-mantle.";
pub(crate) const BEDROCK_MANTLE_URL_SUFFIX: &str = ".api.aws/openai/v1";

/// 从 `config_text` 识别这是否是一个 Bedrock 供应商配置，以及走哪条鉴权路径。
/// 返回 `None` 表示这不是 Bedrock 配置（保持现状交给既有非 Bedrock 回填逻辑处理）。
///
/// 逻辑：
/// 1. `model_provider == "amazon-bedrock"` → AWS Profile 路径，回填 region / aws_profile
/// 2. `requires_openai_auth == true` 且 `base_url` 匹配 bedrock-mantle 正则 → Bearer Token 路径
/// 3. 其余 → None
pub(crate) fn bedrock_config_from_config_text(config_text: &str) -> Option<BedrockConfig> {
    use crate::settings::default_bedrock_iam_key_validity_days;

    let root_provider = root_key_string(config_text, "model_provider");

    // 路径 1：AWS Profile 路径识别
    if root_provider.as_deref() == Some("amazon-bedrock") {
        let region = table_values(config_text, "model_providers.amazon-bedrock.aws")
            .and_then(|values| values.get("region").cloned())
            .map(|v| unquote_toml_string(&v))
            .unwrap_or_default();
        let aws_profile = table_values(config_text, "model_providers.amazon-bedrock.aws")
            .and_then(|values| values.get("profile").cloned())
            .map(|v| unquote_toml_string(&v))
            .unwrap_or_default();

        return Some(BedrockConfig {
            auth_mode: BedrockAuthMode::AwsProfile,
            provider_id: String::new(),
            region,
            aws_profile,
            iam_user_name: String::new(),
            iam_key_validity_days: default_bedrock_iam_key_validity_days(),
        });
    }

    // 路径 2：Bearer Token 路径识别
    // 需要当前活跃 provider 的 requires_openai_auth == true 且 base_url 匹配 bedrock-mantle 模式
    let active_provider = root_provider.as_deref()?;
    let provider_values = table_values(config_text, &format!("model_providers.{active_provider}"))?;

    let requires_auth = provider_values
        .get("requires_openai_auth")
        .map(|v| v.trim() == "true")
        .unwrap_or(false);
    if !requires_auth {
        return None;
    }

    let base_url = provider_values
        .get("base_url")
        .map(|v| unquote_toml_string(v))
        .unwrap_or_default();

    // 匹配 ^<PREFIX><region><SUFFIX>$，其中 region 不含 '.' 或 '/'（与 TS 正则 [^./]+ 等价）。
    if base_url.starts_with(BEDROCK_MANTLE_URL_PREFIX)
        && base_url.ends_with(BEDROCK_MANTLE_URL_SUFFIX)
    {
        let region_start = BEDROCK_MANTLE_URL_PREFIX.len();
        let region_end = base_url.len() - BEDROCK_MANTLE_URL_SUFFIX.len();
        if region_start < region_end {
            let region = &base_url[region_start..region_end];
            if !region.is_empty() && !region.contains('.') && !region.contains('/') {
                return Some(BedrockConfig {
                    auth_mode: BedrockAuthMode::BearerToken,
                    provider_id: active_provider.to_string(),
                    region: region.to_string(),
                    aws_profile: String::new(),
                    iam_user_name: String::new(),
                    iam_key_validity_days: default_bedrock_iam_key_validity_days(),
                });
            }
        }
    }

    None
}

/// 校验用户填写的自定义 provider 标识符：不能为空，不能与保留标识符冲突。
pub fn validate_custom_provider_id(provider_id: &str) -> Result<String, String> {
    let trimmed = provider_id.trim();
    if trimmed.is_empty() {
        return Err("provider 标识符不能为空".to_string());
    }
    if RESERVED_MODEL_PROVIDER_IDS.contains(&trimmed) {
        return Err(format!(
            "「{trimmed}」与保留供应商标识符冲突，请换一个名称"
        ));
    }
    Ok(trimmed.to_string())
}

/// 校验字符串非空（trim 后），为空时返回带 label 的错误信息。
pub(crate) fn require_non_empty(value: &str, label: &str) -> anyhow::Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(anyhow::anyhow!("{label} 不能为空"));
    }
    Ok(trimmed.to_string())
}

/// 确保已知的根级标量赋值行（model、model_provider 等）出现在任何 `[...]` 表头之前。
///
/// 扫描第一个 `[...]` 表头之前的所有行，将匹配已知 key 的赋值行提升到文件最前面，
/// 保持它们之间的相对顺序，然后跟上其余行；表头之后的所有内容按原样保留，
/// 不再"提升"任何看起来像根级标量的行——这是因为 TOML 语义下，表段之后的
/// `key = value` 属于所在 `[section]` 的子字段，与根级同名 key 不是同一个东西。
///
/// 该函数用于修补 `toml_edit` 在某些写入顺序下会把新加的根级标量写到表段之后
/// 的问题；调用方只在自己完全掌控的 config（Codex config.toml）上使用它，
/// 不承诺对任意 TOML 文本语义正确。
///
/// 注释行（`#` 开头）不会被认作根级标量赋值行，避免把 `# model = "foo"` 之类的
/// 说明性注释误当作真正的赋值抢到最前。
pub(crate) fn ensure_root_scalars_precede_tables(contents: &str) -> String {
    const ROOT_SCALAR_KEYS: &[&str] = &[
        "model",
        "model_provider",
        "web_search",
        "model_context_window",
        "model_auto_compact_token_limit",
        "model_catalog_json",
    ];

    let mut scalar_lines: Vec<&str> = Vec::new();
    let mut other_lines: Vec<&str> = Vec::new();
    let mut seen_table = false;

    for line in contents.lines() {
        let trimmed = line.trim();

        // 一旦看到 `[...]` 表头，就把当前及之后所有行都归入 other_lines，
        // 避免把表段内的同名 key（例如 `[foo] \n model = "x"`）误认为是根级标量赋值。
        if !seen_table && trimmed.starts_with('[') && trimmed.ends_with(']') {
            seen_table = true;
        }

        let is_root_scalar = if seen_table || trimmed.starts_with('#') {
            false
        } else if let Some((key, _)) = trimmed.split_once('=') {
            ROOT_SCALAR_KEYS.contains(&key.trim())
        } else {
            false
        };

        if is_root_scalar {
            scalar_lines.push(line);
        } else {
            other_lines.push(line);
        }
    }

    let mut result = scalar_lines.join("\n");
    if !scalar_lines.is_empty() && !other_lines.is_empty() {
        result.push('\n');
    }
    result.push_str(&other_lines.join("\n"));
    ensure_trailing_newline(result)
}

/// Bearer Token 鉴权路径的 config.toml 生成。
///
/// 校验 provider_id / region / API Key 后，生成包含自定义 provider 表的配置文本。
/// 任一校验失败即通过 `?` 提前返回 `Err`，不产出任何字符串。
///
/// `model_providers.<provider_id>` 采用**整表替换**语义（与 AWS Profile 分支一致）：
/// 即便先前已存在同名 provider 表并携带 `env_key` / `http_headers` 等杂字段，
/// 也会被完全丢弃，避免旧字段残留影响 Codex 客户端的行为。
pub(crate) fn complete_bedrock_bearer_token_config(
    profile: &RelayProfile,
    bedrock: &BedrockConfig,
) -> anyhow::Result<String> {
    let provider_id = validate_custom_provider_id(&bedrock.provider_id)
        .map_err(|e| anyhow::anyhow!(e))?;
    let region = require_non_empty(&bedrock.region, "AWS 区域")?;
    let api_key = require_non_empty(&relay_profile_api_key(profile), "Bedrock API Key")?;

    let mut doc = parse_toml_document(&profile.config_contents)?;
    set_provider_id(&mut doc, &provider_id);

    // 写入模型（如果非空）
    let model = relay_profile_model(profile);
    let (model, _) = crate::model_suffix::parse_model_suffix(&model);
    if !model.trim().is_empty() {
        doc["model"] = toml_edit::value(model.trim());
    }

    // Requirement 3.3：无条件写入 web_search = "disabled"
    doc["web_search"] = toml_edit::value("disabled");

    // 构建全新的 provider 表——不复用 `retain_only_provider_table` + 字段覆盖的写法，
    // 因为那样会保留同名旧表下的杂字段（P2 卫生问题）。
    let base_url = format!("{BEDROCK_MANTLE_URL_PREFIX}{region}{BEDROCK_MANTLE_URL_SUFFIX}");
    let mut provider_table = toml_edit::Table::new();
    provider_table.insert("name", toml_edit::value(provider_id.as_str()));
    provider_table.insert("wire_api", toml_edit::value("responses"));
    provider_table.insert("requires_openai_auth", toml_edit::value(true));
    provider_table.insert("base_url", toml_edit::value(base_url.as_str()));
    provider_table.insert(
        "experimental_bearer_token",
        toml_edit::value(api_key.as_str()),
    );

    // 整表替换 model_providers.<provider_id>，并清空同级其它 provider 兄弟表，
    // 与 AWS Profile 分支保持一致。
    let providers = table_mut_or_insert(&mut doc, "model_providers")?;
    providers.clear();
    providers.insert(&provider_id, toml_edit::Item::Table(provider_table));

    let rendered = ensure_trailing_newline(doc.to_string());
    let reordered = move_model_providers_before_profiles(&rendered);
    Ok(ensure_root_scalars_precede_tables(&reordered))
}

/// AWS Profile 鉴权路径的配置生成分支。
/// 使用 Codex 保留的 `amazon-bedrock` provider，配置 `[model_providers.amazon-bedrock.aws]`
/// 子表（region 必填，profile 可选），不需要 API Key 或 base_url。
pub(crate) fn complete_bedrock_aws_profile_config(
    profile: &RelayProfile,
    bedrock: &BedrockConfig,
) -> anyhow::Result<String> {
    const BEDROCK_PROVIDER_ID: &str = "amazon-bedrock";

    let region = require_non_empty(&bedrock.region, "AWS 区域")?;

    let mut doc = parse_toml_document(&profile.config_contents)?;
    set_provider_id(&mut doc, BEDROCK_PROVIDER_ID);

    // 写入模型（如果非空）
    let model = relay_profile_model(profile);
    let (model, _) = crate::model_suffix::parse_model_suffix(&model);
    if !model.trim().is_empty() {
        doc["model"] = toml_edit::value(model.trim());
    }

    // 只保留当前 provider 表，清除其他
    retain_only_provider_table(&mut doc, BEDROCK_PROVIDER_ID);

    // 构建 model_providers.amazon-bedrock 表，仅包含 aws 子表
    let aws_profile = bedrock.aws_profile.trim();

    let mut aws_table = toml_edit::Table::new();
    aws_table.insert("region", toml_edit::value(region.as_str()));
    if !aws_profile.is_empty() {
        aws_table.insert("profile", toml_edit::value(aws_profile));
    }

    let mut provider_table = toml_edit::Table::new();
    provider_table.insert("aws", toml_edit::Item::Table(aws_table));

    // 整表替换 model_providers.amazon-bedrock
    // 使用 table_mut_or_insert 确保 model_providers 作为 TOML 表存在
    let providers = table_mut_or_insert(&mut doc, "model_providers")?;
    providers.insert(BEDROCK_PROVIDER_ID, toml_edit::Item::Table(provider_table));

    let rendered = ensure_trailing_newline(doc.to_string());
    let reordered = move_model_providers_before_profiles(&rendered);
    Ok(ensure_root_scalars_precede_tables(&reordered))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_common_config_preserves_explicit_profile_goals_override() {
        let disabled = merge_common_config_into_config(
            "[features]\ngoals = false\n",
            "[features]\ngoals = true\nfast_mode = true\n",
        )
        .unwrap();
        let disabled_doc = disabled.parse::<DocumentMut>().unwrap();
        assert_eq!(disabled_doc["features"]["goals"].as_bool(), Some(false));
        assert_eq!(disabled_doc["features"]["fast_mode"].as_bool(), Some(true));

        let enabled = merge_common_config_into_config(
            "[features]\ngoals = true\n",
            "[features]\ngoals = false\nfast_mode = true\n",
        )
        .unwrap();
        let enabled_doc = enabled.parse::<DocumentMut>().unwrap();
        assert_eq!(enabled_doc["features"]["goals"].as_bool(), Some(true));
        assert_eq!(enabled_doc["features"]["fast_mode"].as_bool(), Some(true));
    }

    #[test]
    fn merge_common_config_uses_common_goals_without_profile_override() {
        let merged =
            merge_common_config_into_config("model = \"gpt-5\"\n", "[features]\ngoals = true\n")
                .unwrap();
        let doc = merged.parse::<DocumentMut>().unwrap();
        assert_eq!(doc["features"]["goals"].as_bool(), Some(true));
    }

    #[test]
    fn backfill_relay_profile_from_home_with_common_restores_template_provider_id() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("config.toml"),
            "model_provider = \"custom\"\nmodel = \"gpt-image-2\"\n\n[model_providers.custom]\nname = \"custom\"\nwire_api = \"responses\"\nrequires_openai_auth = true\nbase_url = \"https://ahg.codes\"\n",
        )
        .unwrap();
        std::fs::write(temp.path().join("auth.json"), "{}\n").unwrap();

        let mut profile = RelayProfile {
            relay_mode: crate::settings::RelayMode::PureApi,
            protocol: crate::settings::RelayProtocol::Responses,
            config_contents: "model_provider = \"ai\"\nmodel = \"gpt-image-2\"\n\n[model_providers.ai]\nname = \"ai\"\nwire_api = \"responses\"\nrequires_openai_auth = true\nbase_url = \"https://ahg.codes\"\n"
                .to_string(),
            auth_contents: "{}\n".to_string(),
            ..RelayProfile::default()
        };
        let mut common = String::new();

        backfill_relay_profile_from_home_with_common(temp.path(), &mut profile, &mut common)
            .unwrap();

        assert!(profile.config_contents.contains("model_provider = \"ai\""));
        assert!(profile.config_contents.contains("[model_providers.ai]"));
        assert!(!profile.config_contents.contains("[model_providers.custom]"));
    }

    #[test]
    fn relay_profile_model_prefers_config_then_field_then_empty() {
        // 1. 供應商測試的回退第一級：config.toml 的 model = 優先
        let from_config = RelayProfile {
            config_contents: "model = \"deepseek-v4-flash\"\nmodel_provider = \"custom\"\n"
                .to_string(),
            model: "should-not-be-used".to_string(),
            ..RelayProfile::default()
        };
        assert_eq!(relay_profile_model(&from_config), "deepseek-v4-flash");

        // 2. config 沒寫 model 時退回 profile.model 欄位
        let from_field = RelayProfile {
            config_contents: "model_provider = \"custom\"\n".to_string(),
            model: "deepseek-v4-pro".to_string(),
            ..RelayProfile::default()
        };
        assert_eq!(relay_profile_model(&from_field), "deepseek-v4-pro");

        // 3. 兩者皆空 → 空字串；呼叫端據此才回退到全域 relayTestModel
        let empty = RelayProfile {
            config_contents: String::new(),
            model: String::new(),
            ..RelayProfile::default()
        };
        assert!(relay_profile_model(&empty).trim().is_empty());
    }

    #[test]
    fn ensure_root_scalars_precede_tables_does_not_steal_in_table_scalars() {
        // Regression: 表段之内的 `model = "..."` 属于所在 section 的子字段，
        // 不应该被误当作根级标量抢走到最前面（会破坏 TOML 语义）。
        let input = "\
[custom_section]
model = \"foo\"
name = \"bar\"

[another_section]
model_provider = \"baz\"
";
        let output = ensure_root_scalars_precede_tables(input);
        // 输出的第一行不能是表段内的 `model = "foo"`
        let first_line = output.lines().next().unwrap_or("");
        assert!(
            first_line.trim_start().starts_with('['),
            "First line should still be a table header, but got: {first_line:?}\nFull output:\n{output}"
        );
        // model = "foo" 依然出现在 [custom_section] 之后（未被上提）
        let model_line_idx = output
            .lines()
            .position(|line| line.trim() == "model = \"foo\"")
            .expect("expected `model = \"foo\"` line in output");
        let custom_section_idx = output
            .lines()
            .position(|line| line.trim() == "[custom_section]")
            .expect("expected [custom_section] line in output");
        assert!(
            model_line_idx > custom_section_idx,
            "model in [custom_section] must remain after its section header; \
             model at line {model_line_idx}, section at line {custom_section_idx}\nOutput:\n{output}"
        );
    }

    #[test]
    fn ensure_root_scalars_precede_tables_ignores_commented_scalars() {
        // Regression: `# model = "foo"` 是注释，不是根级赋值，不应被上提。
        let input = "\
# model = \"commented-out\"

[custom_section]
name = \"bar\"

model_provider = \"real\"
";
        let output = ensure_root_scalars_precede_tables(input);
        // 注释行不会被识别为根级 scalar，因此原地保留
        // 而 `model_provider = "real"` 出现在 [custom_section] 之后，
        // 按新语义视作表内字段，不被上提（也不会破坏 TOML 语义）。
        let first_line = output.lines().next().unwrap_or("");
        assert!(
            first_line.starts_with("# model"),
            "Comment line should remain at the top, but got: {first_line:?}\nOutput:\n{output}"
        );
    }

    #[test]
    fn complete_bedrock_bearer_token_config_drops_stale_provider_fields() {
        // Regression: Bearer Token 分支采用整表替换语义，
        // 同名旧 provider 表下的杂字段（如 env_key / http_headers）不应残留。
        let profile = RelayProfile {
            config_contents: "\
model_provider = \"my-bedrock\"

[model_providers.my-bedrock]
name = \"leftover-name\"
env_key = \"OLD_KEY\"
env_http_headers = { X-Trace = \"leftover\" }
http_headers = { Authorization = \"Bearer stale\" }

[model_providers.other-leftover]
name = \"should-be-cleared\"
"
                .to_string(),
            api_key: "brk-test-key".to_string(),
            bedrock: Some(BedrockConfig {
                auth_mode: BedrockAuthMode::BearerToken,
                provider_id: "my-bedrock".to_string(),
                region: "us-east-2".to_string(),
                aws_profile: String::new(),
                iam_user_name: String::new(),
                iam_key_validity_days: crate::settings::default_bedrock_iam_key_validity_days(),
            }),
            ..RelayProfile::default()
        };
        let bedrock = profile.bedrock.as_ref().unwrap();
        let output = complete_bedrock_bearer_token_config(&profile, bedrock)
            .expect("bearer token config should succeed");

        // 5 个 Bedrock 必需字段
        assert!(output.contains("name = \"my-bedrock\""), "expected fresh name, got:\n{output}");
        assert!(output.contains("wire_api = \"responses\""), "expected wire_api, got:\n{output}");
        assert!(output.contains("requires_openai_auth = true"), "expected requires_openai_auth, got:\n{output}");
        assert!(
            output.contains("base_url = \"https://bedrock-mantle.us-east-2.api.aws/openai/v1\""),
            "expected bedrock-mantle base_url, got:\n{output}"
        );
        assert!(
            output.contains("experimental_bearer_token = \"brk-test-key\""),
            "expected experimental_bearer_token, got:\n{output}"
        );

        // 旧脏字段不应残留
        assert!(!output.contains("leftover-name"), "stale name should be dropped, got:\n{output}");
        assert!(!output.contains("OLD_KEY"), "stale env_key value should be dropped, got:\n{output}");
        assert!(!output.contains("env_key"), "stale env_key field should be dropped, got:\n{output}");
        assert!(!output.contains("env_http_headers"), "stale env_http_headers should be dropped, got:\n{output}");
        assert!(!output.contains("http_headers"), "stale http_headers should be dropped, got:\n{output}");
        assert!(!output.contains("leftover"), "stale header values should be dropped, got:\n{output}");
        assert!(!output.contains("should-be-cleared"), "sibling provider should be cleared, got:\n{output}");
        assert!(!output.contains("[model_providers.other-leftover]"), "sibling provider table should be dropped, got:\n{output}");
    }

    #[test]
    fn bedrock_config_from_config_text_returns_none_for_non_bedrock_configs() {
        // 1. 标准自定义供应商配置
        let custom_config = "model_provider = \"custom\"\nmodel = \"some-model\"\n\n[model_providers.custom]\nname = \"custom\"\nbase_url = \"https://example.com\"\nwire_api = \"responses\"\nrequires_openai_auth = true\n";
        assert_eq!(
            bedrock_config_from_config_text(custom_config),
            None,
            "Custom provider config should return None"
        );

        // 2. Official/OpenAI 风格配置
        let openai_config = "model_provider = \"openai\"\nmodel = \"gpt-4\"\n\n[model_providers.openai]\nname = \"openai\"\nbase_url = \"https://api.openai.com/v1\"\nwire_api = \"responses\"\nrequires_openai_auth = true\n";
        assert_eq!(
            bedrock_config_from_config_text(openai_config),
            None,
            "OpenAI-style config should return None"
        );

        // 3. 不含 model_provider 的配置
        let no_provider_config = "model = \"some-model\"\n\n[model_providers.foo]\nname = \"foo\"\nbase_url = \"https://foo.example.com\"\n";
        assert_eq!(
            bedrock_config_from_config_text(no_provider_config),
            None,
            "Config without model_provider should return None"
        );

        // 4. requires_openai_auth = true 但 base_url 不匹配 bedrock-mantle 模式
        let non_bedrock_url_config = "model_provider = \"myapi\"\nmodel = \"deepseek-v4\"\n\n[model_providers.myapi]\nname = \"myapi\"\nbase_url = \"https://api.myservice.com/v1\"\nwire_api = \"responses\"\nrequires_openai_auth = true\n";
        assert_eq!(
            bedrock_config_from_config_text(non_bedrock_url_config),
            None,
            "Non-bedrock-mantle base_url config should return None"
        );

        // 5. 空字符串
        assert_eq!(
            bedrock_config_from_config_text(""),
            None,
            "Empty config should return None"
        );
    }

    #[test]
    fn backfill_does_not_set_bedrock_for_non_bedrock_config() {
        let temp = tempfile::tempdir().unwrap();
        // 写入一个标准自定义供应商的 config.toml（非 Bedrock 形状）
        std::fs::write(
            temp.path().join("config.toml"),
            "model_provider = \"custom\"\nmodel = \"gpt-4\"\n\n[model_providers.custom]\nname = \"custom\"\nwire_api = \"responses\"\nrequires_openai_auth = true\nbase_url = \"https://example.com\"\n",
        )
        .unwrap();
        std::fs::write(temp.path().join("auth.json"), "{}\n").unwrap();

        let mut profile = RelayProfile {
            relay_mode: crate::settings::RelayMode::PureApi,
            protocol: crate::settings::RelayProtocol::Responses,
            config_contents: "model_provider = \"custom\"\nmodel = \"gpt-4\"\n\n[model_providers.custom]\nname = \"custom\"\nwire_api = \"responses\"\nrequires_openai_auth = true\nbase_url = \"https://example.com\"\n"
                .to_string(),
            auth_contents: "{}\n".to_string(),
            bedrock: None,
            ..RelayProfile::default()
        };
        let mut common = String::new();

        backfill_relay_profile_from_home_with_common(temp.path(), &mut profile, &mut common)
            .unwrap();

        // 回填后 bedrock 字段应保持 None
        assert!(
            profile.bedrock.is_none(),
            "bedrock should remain None for non-Bedrock config after backfill, got: {:?}",
            profile.bedrock
        );
    }

    mod proptest_property_tests {
        use super::*;
        use proptest::prelude::*;

        // Feature: amazon-bedrock-provider, Property 10: 保留字冲突校验（通用，不限于 Bedrock）
        //
        // *For any* 字符串，`validate_custom_provider_id` 返回 `Err` 当且仅当该字符串（trim 后）为空，
        // 或该字符串（trim 后）属于 `RESERVED_MODEL_PROVIDER_IDS` 集合；其余情况返回 `Ok`。
        //
        // **Validates: Requirements 2.2**
        proptest! {
            #[test]
            fn property_10_reserved_id_conflict_validation(input in ".*") {
                let trimmed = input.trim();
                let result = validate_custom_provider_id(&input);

                if trimmed.is_empty() {
                    // trim 后为空 → 必须返回 Err
                    prop_assert!(result.is_err(), "Expected Err for empty-after-trim input {:?}, got Ok", input);
                } else if RESERVED_MODEL_PROVIDER_IDS.contains(&trimmed) {
                    // trim 后命中保留字 → 必须返回 Err
                    prop_assert!(result.is_err(), "Expected Err for reserved id {:?}, got Ok", trimmed);
                } else {
                    // 其余情况 → 必须返回 Ok(trimmed)
                    prop_assert!(result.is_ok(), "Expected Ok for non-reserved non-empty id {:?}, got Err: {:?}", trimmed, result);
                    prop_assert_eq!(result.unwrap(), trimmed.to_string());
                }
            }
        }

        // 补充：显式使用保留字作为输入，确保每一个保留字都被拒绝
        proptest! {
            #[test]
            fn property_10_reserved_ids_explicitly_rejected(
                idx in 0..RESERVED_MODEL_PROVIDER_IDS.len(),
                prefix_spaces in "\\s{0,5}",
                suffix_spaces in "\\s{0,5}",
            ) {
                let reserved = RESERVED_MODEL_PROVIDER_IDS[idx];
                let input = format!("{}{}{}", prefix_spaces, reserved, suffix_spaces);
                let result = validate_custom_provider_id(&input);
                prop_assert!(result.is_err(), "Expected Err for reserved id {:?} (with whitespace), got Ok", input);
            }
        }

        // Feature: amazon-bedrock-provider, Property 3: 根级标量顺序不变式
        //
        // *For any* 任意预先存在的 `config_contents`（用 `proptest` 生成随机根级
        // `key = "value"` 赋值行 + 随机 `[table.name]` 表头 + 表内字段的 TOML 文本），
        // 对其调用 `ensure_root_scalars_precede_tables` 后，输出文本中
        // `model_provider`/`model` 所在行的行号都小于第一个 `[...]` 表头所在行的行号。
        //
        // 说明：TOML 语义下，第一个表头之后的 `key = value` 属于所在 `[section]`
        // 的子字段，与根级同名 key 不是同一个东西。生成器把"根级 scalar 行"限制
        // 在第一个表头之前，反映这一 TOML 语义约束——`ensure_root_scalars_precede_tables`
        // 不再"暴力抢走"表段内的同名字段。
        //
        // **Validates: Requirements 4.2**

        /// 策略：先生成 0..N 行"表头之前的行"（scalar / 注释 / 空行），再拼接
        /// 0..M 行"表头及之后的行"（表头 / 表内字段 / 注释 / 空行），保证根级
        /// scalar 行不会出现在任何 `[...]` 表头之后。
        fn arb_toml_like_text() -> impl Strategy<Value = String> {
            // 已知根级标量 key 列表
            const SCALAR_KEYS: &[&str] = &[
                "model",
                "model_provider",
                "web_search",
                "model_context_window",
                "model_auto_compact_token_limit",
                "model_catalog_json",
            ];

            let scalar_line = prop::sample::select(SCALAR_KEYS)
                .prop_flat_map(|key| {
                    "[a-zA-Z0-9_\\-]{1,20}".prop_map(move |val| {
                        format!("{} = \"{}\"", key, val)
                    })
                });

            let table_header = "[a-zA-Z_][a-zA-Z0-9_]{0,10}(\\.[a-zA-Z_][a-zA-Z0-9_]{0,10}){0,2}"
                .prop_map(|name| format!("[{}]", name));

            let non_root_scalar_line = prop_oneof![
                Just(String::new()),                         // 空行
                Just("# comment line".to_string()),          // 注释
                "[a-z_]{1,10} = \"[a-z]{1,10}\""             // 非根级 key 的赋值行
                    .prop_filter("must not be a root scalar key", |line| {
                        if let Some((key, _)) = line.split_once('=') {
                            let key = key.trim();
                            !SCALAR_KEYS.contains(&key)
                        } else {
                            true
                        }
                    }),
            ];

            // 表头之前：允许 scalar 与非 scalar 的普通行
            let pre_header_line = prop_oneof![
                3 => scalar_line,
                3 => non_root_scalar_line.clone(),
            ];

            // 表头及之后：只允许表头与非 scalar 行（表内字段视作 non-scalar）
            let post_header_line = prop_oneof![
                3 => table_header,
                3 => non_root_scalar_line,
            ];

            (
                prop::collection::vec(pre_header_line, 0..15),
                prop::collection::vec(post_header_line, 0..15),
            )
                .prop_map(|(pre, post)| {
                    let mut lines = pre;
                    lines.extend(post);
                    lines.join("\n")
                })
        }

        proptest! {
            #[test]
            fn property_3_root_scalars_precede_tables(input in arb_toml_like_text()) {
                let output = ensure_root_scalars_precede_tables(&input);

                // 已知根级标量 key 集合
                const SCALAR_KEYS: &[&str] = &[
                    "model",
                    "model_provider",
                    "web_search",
                    "model_context_window",
                    "model_auto_compact_token_limit",
                    "model_catalog_json",
                ];

                // 找到输出中第一个表头行的行号（0-based）
                let first_table_header_line: Option<usize> = output.lines().enumerate()
                    .find(|(_, line)| {
                        let trimmed = line.trim();
                        trimmed.starts_with('[') && trimmed.ends_with(']') && !trimmed.starts_with("[[")
                    })
                    .map(|(idx, _)| idx);

                // 检查每个根级标量行是否都出现在第一个表头之前
                for (line_idx, line) in output.lines().enumerate() {
                    let trimmed = line.trim();
                    let is_root_scalar = if let Some((key, _)) = trimmed.split_once('=') {
                        let key = key.trim();
                        SCALAR_KEYS.contains(&key)
                    } else {
                        false
                    };

                    if is_root_scalar {
                        if let Some(first_table) = first_table_header_line {
                            prop_assert!(
                                line_idx < first_table,
                                "Root scalar at line {} ({:?}) is NOT before first table header at line {}.\nOutput:\n{}",
                                line_idx, trimmed, first_table, output
                            );
                        }
                        // 如果没有表头行，则标量行可以出现在任何位置（属性自动满足）
                    }
                }
            }
        }

        // Feature: amazon-bedrock-provider, Property 1: Bearer Token 配置生成正确性
        //
        // *For any* 非保留的 provider 标识符字符串、任意非空 region 字符串、任意非空
        // API Key 字符串，生成结果中顶层 `model_provider` 等于该标识符；`base_url` 等于
        // `https://bedrock-mantle.<region>.api.aws/openai/v1`；`requires_openai_auth`
        // 为 `true`；`experimental_bearer_token` 等于该 API Key；顶层 `web_search`
        // 等于 `"disabled"`。
        //
        // **Validates: Requirements 2.1, 2.3, 3.1, 3.3**
        proptest! {
            #[test]
            fn property_1_bearer_token_config_correctness(
                provider_id in "[a-z][a-z0-9]{1,12}",
                region in "[a-z][a-z0-9\\-]{2,15}",
                api_key in "[a-zA-Z0-9]{5,30}",
            ) {
                // 构造 RelayProfile，设置 api_key 和 bedrock bearer token 配置
                let profile = RelayProfile {
                    api_key: api_key.clone(),
                    bedrock: Some(crate::settings::BedrockConfig {
                        auth_mode: crate::settings::BedrockAuthMode::BearerToken,
                        provider_id: provider_id.clone(),
                        region: region.clone(),
                        aws_profile: String::new(),
                        iam_user_name: String::new(),
                        iam_key_validity_days: crate::settings::default_bedrock_iam_key_validity_days(),
                    }),
                    ..RelayProfile::default()
                };

                let bedrock = profile.bedrock.as_ref().unwrap();
                let result = complete_bedrock_bearer_token_config(&profile, bedrock);

                // 结果必须为 Ok
                prop_assert!(result.is_ok(), "Expected Ok, got Err: {:?}", result);
                let output = result.unwrap();

                // 验证 model_provider = "<provider_id>"
                let expected_provider = format!("model_provider = \"{}\"", provider_id);
                prop_assert!(
                    output.contains(&expected_provider),
                    "Output should contain {:?}, but got:\n{}",
                    expected_provider, output
                );

                // 验证 base_url = "https://bedrock-mantle.<region>.api.aws/openai/v1"
                let expected_base_url = format!(
                    "base_url = \"https://bedrock-mantle.{}.api.aws/openai/v1\"",
                    region
                );
                prop_assert!(
                    output.contains(&expected_base_url),
                    "Output should contain {:?}, but got:\n{}",
                    expected_base_url, output
                );

                // 验证 requires_openai_auth = true
                prop_assert!(
                    output.contains("requires_openai_auth = true"),
                    "Output should contain 'requires_openai_auth = true', but got:\n{}",
                    output
                );

                // 验证 experimental_bearer_token = "<api_key>"
                let expected_token = format!("experimental_bearer_token = \"{}\"", api_key);
                prop_assert!(
                    output.contains(&expected_token),
                    "Output should contain {:?}, but got:\n{}",
                    expected_token, output
                );

                // 验证 web_search = "disabled"
                prop_assert!(
                    output.contains("web_search = \"disabled\""),
                    "Output should contain 'web_search = \"disabled\"', but got:\n{}",
                    output
                );
            }
        }

        // Feature: amazon-bedrock-provider, Property 2: AWS Profile 配置生成正确性
        //
        // *For any* 任意非空 region 字符串、任意（可为空）AWS profile 名称字符串，
        // 生成结果中顶层 `model_provider` 恒等于 `"amazon-bedrock"`；
        // `model_providers.amazon-bedrock` 表含 `[model_providers.amazon-bedrock.aws]` section
        // 带 `region = "<region>"`；aws_profile 非空时含 `profile = "<aws_profile>"`，
        // 为空时不含 `profile =`；不含 `requires_openai_auth`/`base_url`/`experimental_bearer_token`。
        //
        // **Validates: Requirements 4.1, 4.3, 5.1, 5.2, 5.3**
        proptest! {
            #[test]
            fn property_2_aws_profile_config_correctness(
                region in "[a-z][a-z0-9\\-]{2,15}",
                aws_profile in "(|[a-z][a-z0-9\\-]{1,15})",
            ) {
                let profile = RelayProfile {
                    bedrock: Some(crate::settings::BedrockConfig {
                        auth_mode: crate::settings::BedrockAuthMode::AwsProfile,
                        region: region.clone(),
                        aws_profile: aws_profile.clone(),
                        provider_id: String::new(),
                        iam_user_name: String::new(),
                        iam_key_validity_days: crate::settings::default_bedrock_iam_key_validity_days(),
                    }),
                    ..RelayProfile::default()
                };

                let bedrock = profile.bedrock.as_ref().unwrap();
                let result = complete_bedrock_aws_profile_config(&profile, bedrock);

                // 结果必须为 Ok
                prop_assert!(result.is_ok(), "Expected Ok, got Err: {:?}", result);
                let output = result.unwrap();

                // 验证 model_provider = "amazon-bedrock"
                prop_assert!(
                    output.contains("model_provider = \"amazon-bedrock\""),
                    "Output should contain 'model_provider = \"amazon-bedrock\"', but got:\n{}",
                    output
                );

                // 验证 [model_providers.amazon-bedrock.aws] section 含 region = "<region>"
                let expected_region = format!("region = \"{}\"", region);
                prop_assert!(
                    output.contains(&expected_region),
                    "Output should contain {:?}, but got:\n{}",
                    expected_region, output
                );

                // 验证 aws_profile 非空时含 profile = "<aws_profile>"，为空时不含 profile =
                let trimmed_profile = aws_profile.trim();
                if !trimmed_profile.is_empty() {
                    let expected_profile = format!("profile = \"{}\"", trimmed_profile);
                    prop_assert!(
                        output.contains(&expected_profile),
                        "Output should contain {:?} for non-empty aws_profile, but got:\n{}",
                        expected_profile, output
                    );
                } else {
                    // 在 [model_providers.amazon-bedrock.aws] section 内不应有 profile =
                    prop_assert!(
                        !output.contains("profile ="),
                        "Output should NOT contain 'profile =' for empty aws_profile, but got:\n{}",
                        output
                    );
                }

                // 验证不含 requires_openai_auth / base_url / experimental_bearer_token
                prop_assert!(
                    !output.contains("requires_openai_auth"),
                    "Output should NOT contain 'requires_openai_auth', but got:\n{}",
                    output
                );
                prop_assert!(
                    !output.contains("base_url"),
                    "Output should NOT contain 'base_url', but got:\n{}",
                    output
                );
                prop_assert!(
                    !output.contains("experimental_bearer_token"),
                    "Output should NOT contain 'experimental_bearer_token', but got:\n{}",
                    output
                );
            }
        }

        // Feature: amazon-bedrock-provider, Property 4: Region 必填校验
        //
        // *For any* 空字符串或全空白字符串作为 region，`require_non_empty` 返回 `Err`；
        // *for any* 非空非空白字符串返回 `Ok`。
        // 另外验证 `complete_bedrock_bearer_token_config` 与 `complete_bedrock_aws_profile_config`
        // 在 region 为空/空白时返回 `Err`，在 region 非空时返回 `Ok`。
        //
        // **Validates: Requirements 2.4, 5.4**

        proptest! {
            #[test]
            fn property_4_region_required_validation(
                empty_region in "\\s{0,20}",
                non_empty_region in "[a-z][a-z0-9\\-]{2,15}",
            ) {
                // Part 1: require_non_empty 对空/空白字符串返回 Err
                let err_result = require_non_empty(&empty_region, "region");
                prop_assert!(
                    err_result.is_err(),
                    "Expected Err for empty/whitespace region {:?}, got Ok",
                    empty_region
                );

                // Part 2: require_non_empty 对非空非空白字符串返回 Ok
                let ok_result = require_non_empty(&non_empty_region, "region");
                prop_assert!(
                    ok_result.is_ok(),
                    "Expected Ok for non-empty region {:?}, got Err: {:?}",
                    non_empty_region, ok_result
                );
                prop_assert_eq!(ok_result.unwrap(), non_empty_region.trim().to_string());

                // Part 3: complete_bedrock_bearer_token_config 在 region 为空/空白时返回 Err
                let bearer_profile_empty_region = RelayProfile {
                    api_key: "valid-api-key-12345".to_string(),
                    bedrock: Some(crate::settings::BedrockConfig {
                        auth_mode: crate::settings::BedrockAuthMode::BearerToken,
                        provider_id: "my-bedrock-provider".to_string(),
                        region: empty_region.clone(),
                        aws_profile: String::new(),
                        iam_user_name: String::new(),
                        iam_key_validity_days: crate::settings::default_bedrock_iam_key_validity_days(),
                    }),
                    ..RelayProfile::default()
                };
                let bedrock_bearer = bearer_profile_empty_region.bedrock.as_ref().unwrap();
                let bearer_result = complete_bedrock_bearer_token_config(&bearer_profile_empty_region, bedrock_bearer);
                prop_assert!(
                    bearer_result.is_err(),
                    "Expected Err for bearer token config with empty region {:?}, got Ok",
                    empty_region
                );

                // Part 4: complete_bedrock_aws_profile_config 在 region 为空/空白时返回 Err
                let aws_profile_empty_region = RelayProfile {
                    bedrock: Some(crate::settings::BedrockConfig {
                        auth_mode: crate::settings::BedrockAuthMode::AwsProfile,
                        provider_id: String::new(),
                        region: empty_region.clone(),
                        aws_profile: "default".to_string(),
                        iam_user_name: String::new(),
                        iam_key_validity_days: crate::settings::default_bedrock_iam_key_validity_days(),
                    }),
                    ..RelayProfile::default()
                };
                let bedrock_aws = aws_profile_empty_region.bedrock.as_ref().unwrap();
                let aws_result = complete_bedrock_aws_profile_config(&aws_profile_empty_region, bedrock_aws);
                prop_assert!(
                    aws_result.is_err(),
                    "Expected Err for AWS profile config with empty region {:?}, got Ok",
                    empty_region
                );

                // Part 5: 两条路径在 region 非空时返回 Ok
                let bearer_profile_valid_region = RelayProfile {
                    api_key: "valid-api-key-12345".to_string(),
                    bedrock: Some(crate::settings::BedrockConfig {
                        auth_mode: crate::settings::BedrockAuthMode::BearerToken,
                        provider_id: "my-bedrock-provider".to_string(),
                        region: non_empty_region.clone(),
                        aws_profile: String::new(),
                        iam_user_name: String::new(),
                        iam_key_validity_days: crate::settings::default_bedrock_iam_key_validity_days(),
                    }),
                    ..RelayProfile::default()
                };
                let bedrock_bearer_valid = bearer_profile_valid_region.bedrock.as_ref().unwrap();
                let bearer_ok_result = complete_bedrock_bearer_token_config(&bearer_profile_valid_region, bedrock_bearer_valid);
                prop_assert!(
                    bearer_ok_result.is_ok(),
                    "Expected Ok for bearer token config with valid region {:?}, got Err: {:?}",
                    non_empty_region, bearer_ok_result
                );

                let aws_profile_valid_region = RelayProfile {
                    bedrock: Some(crate::settings::BedrockConfig {
                        auth_mode: crate::settings::BedrockAuthMode::AwsProfile,
                        provider_id: String::new(),
                        region: non_empty_region.clone(),
                        aws_profile: "default".to_string(),
                        iam_user_name: String::new(),
                        iam_key_validity_days: crate::settings::default_bedrock_iam_key_validity_days(),
                    }),
                    ..RelayProfile::default()
                };
                let bedrock_aws_valid = aws_profile_valid_region.bedrock.as_ref().unwrap();
                let aws_ok_result = complete_bedrock_aws_profile_config(&aws_profile_valid_region, bedrock_aws_valid);
                prop_assert!(
                    aws_ok_result.is_ok(),
                    "Expected Ok for AWS profile config with valid region {:?}, got Err: {:?}",
                    non_empty_region, aws_ok_result
                );
            }
        }

        // Feature: amazon-bedrock-provider, Property 11: Bearer Token 路径生成的 Base URL 与既有测试机制一致
        //
        // *For any* 非保留的 provider 标识符字符串与任意非空 region 字符串，对生成后的
        // `RelayProfile`（`config_contents` 设为 `complete_bedrock_bearer_token_config` 的
        // 生成结果）调用既有 `relay_profile_base_url`，其返回值恒等于
        // `https://bedrock-mantle.<region>.api.aws/openai/v1`。
        //
        // **Validates: Requirements 9.3**
        proptest! {
            #[test]
            fn property_11_bearer_token_base_url_consistency(
                provider_id in "[a-z][a-z0-9]{1,12}",
                region in "[a-z][a-z0-9\\-]{2,15}",
            ) {
                // 构造带 BedrockConfig (BearerToken mode) 的 RelayProfile
                let api_key = "test-api-key-for-property-11".to_string();
                let profile = RelayProfile {
                    api_key: api_key.clone(),
                    bedrock: Some(crate::settings::BedrockConfig {
                        auth_mode: crate::settings::BedrockAuthMode::BearerToken,
                        provider_id: provider_id.clone(),
                        region: region.clone(),
                        aws_profile: String::new(),
                        iam_user_name: String::new(),
                        iam_key_validity_days: crate::settings::default_bedrock_iam_key_validity_days(),
                    }),
                    ..RelayProfile::default()
                };

                // 调用 complete_bedrock_bearer_token_config 生成配置
                let bedrock = profile.bedrock.as_ref().unwrap();
                let config_result = complete_bedrock_bearer_token_config(&profile, bedrock);
                prop_assert!(config_result.is_ok(), "Expected Ok from config generation, got Err: {:?}", config_result);
                let config_contents = config_result.unwrap();

                // 构造一个新的 RelayProfile，将生成的 config 设为 config_contents
                let profile_with_config = RelayProfile {
                    config_contents: config_contents,
                    ..RelayProfile::default()
                };

                // 调用既有 relay_profile_base_url 函数
                let base_url = relay_profile_base_url(&profile_with_config);

                // 断言返回的 base_url 等于期望值
                let expected_base_url = format!("https://bedrock-mantle.{}.api.aws/openai/v1", region);
                prop_assert_eq!(
                    base_url,
                    expected_base_url,
                    "relay_profile_base_url should return the Bedrock Mantle URL for region {:?}",
                    region
                );
            }
        }

        // Feature: amazon-bedrock-provider, Property 9: AWS Profile 生成与识别往返
        //
        // *For any* 任意非空 region 字符串与任意（可为空）AWS profile 名称字符串，将
        // `complete_bedrock_aws_profile_config` 生成的 config.toml 文本交给
        // `bedrock_config_from_config_text` 解析，得到的 `auth_mode` 恒为 `AwsProfile`，
        // `region` 等于原始 region，`aws_profile` 等于原始名称（未填写时为空字符串）。
        //
        // **Validates: Requirements 10.1**
        proptest! {
            #[test]
            fn property_9_aws_profile_roundtrip(
                region in "[a-z][a-z0-9\\-]{2,15}",
                aws_profile in "[a-zA-Z0-9_\\-]{0,20}",
            ) {
                // 构造带 BedrockConfig (AwsProfile mode) 的 RelayProfile
                let profile = RelayProfile {
                    bedrock: Some(crate::settings::BedrockConfig {
                        auth_mode: crate::settings::BedrockAuthMode::AwsProfile,
                        provider_id: String::new(),
                        region: region.clone(),
                        aws_profile: aws_profile.clone(),
                        iam_user_name: String::new(),
                        iam_key_validity_days: crate::settings::default_bedrock_iam_key_validity_days(),
                    }),
                    ..RelayProfile::default()
                };

                let bedrock = profile.bedrock.as_ref().unwrap();

                // 调用 complete_bedrock_aws_profile_config 生成配置文本
                let config_result = complete_bedrock_aws_profile_config(&profile, bedrock);
                prop_assert!(config_result.is_ok(), "Expected Ok from config generation, got Err: {:?}", config_result);
                let config_text = config_result.unwrap();

                // 将生成的文本交给 bedrock_config_from_config_text 解析
                let parsed = bedrock_config_from_config_text(&config_text);
                prop_assert!(parsed.is_some(), "Expected Some(BedrockConfig) from parsing, got None for config:\n{}", config_text);
                let parsed = parsed.unwrap();

                // 断言 auth_mode 为 AwsProfile
                prop_assert_eq!(
                    parsed.auth_mode,
                    crate::settings::BedrockAuthMode::AwsProfile,
                    "auth_mode should be AwsProfile"
                );

                // 断言 region 等于原始 region（trimmed）
                let expected_region = region.trim().to_string();
                prop_assert_eq!(
                    parsed.region,
                    expected_region,
                );

                // 断言 aws_profile 等于原始名称（trim 后，空则为空字符串）
                let expected_profile = if aws_profile.trim().is_empty() {
                    String::new()
                } else {
                    aws_profile.trim().to_string()
                };
                prop_assert_eq!(
                    parsed.aws_profile,
                    expected_profile,
                );
            }
        }

        // Feature: amazon-bedrock-provider, Property 8: Bearer Token 生成与识别往返
        //
        // *For any* 非保留的 provider 标识符字符串与任意非空 region 字符串（API Key 任取一个
        // 非空字符串），将 complete_bedrock_bearer_token_config 生成的 config.toml 文本交给
        // `bedrock_config_from_config_text` 解析，得到的 `auth_mode` 恒为 `BearerToken`，
        // `provider_id` 等于原始标识符，`region` 等于原始 region。
        //
        // **Validates: Requirements 10.2**
        proptest! {
            #[test]
            fn property_8_bearer_token_roundtrip(
                provider_id in "[a-z][a-z0-9]{1,12}",
                region in "[a-z][a-z0-9\\-]{2,15}",
            ) {
                // 使用固定非空 API Key
                let api_key = "test-api-key-for-roundtrip".to_string();

                // 构造带 BedrockConfig (BearerToken mode) 的 RelayProfile
                let profile = RelayProfile {
                    api_key: api_key.clone(),
                    bedrock: Some(crate::settings::BedrockConfig {
                        auth_mode: crate::settings::BedrockAuthMode::BearerToken,
                        provider_id: provider_id.clone(),
                        region: region.clone(),
                        aws_profile: String::new(),
                        iam_user_name: String::new(),
                        iam_key_validity_days: crate::settings::default_bedrock_iam_key_validity_days(),
                    }),
                    ..RelayProfile::default()
                };

                let bedrock = profile.bedrock.as_ref().unwrap();

                // 调用 complete_bedrock_bearer_token_config 生成配置文本
                let config_result = complete_bedrock_bearer_token_config(&profile, bedrock);
                prop_assert!(config_result.is_ok(), "Expected Ok from config generation, got Err: {:?}", config_result);
                let config_text = config_result.unwrap();

                // 将生成的文本交给 bedrock_config_from_config_text 解析
                let parsed = bedrock_config_from_config_text(&config_text);
                prop_assert!(parsed.is_some(), "Expected Some(BedrockConfig) from parsing, got None for config:\n{}", config_text);
                let parsed = parsed.unwrap();

                // 断言 auth_mode 为 BearerToken
                prop_assert_eq!(
                    parsed.auth_mode,
                    crate::settings::BedrockAuthMode::BearerToken,
                    "auth_mode should be BearerToken"
                );

                // 断言 provider_id 等于原始标识符
                prop_assert_eq!(
                    parsed.provider_id,
                    provider_id.clone(),
                    "provider_id should match the original identifier"
                );

                // 断言 region 等于原始 region
                prop_assert_eq!(
                    parsed.region,
                    region.clone(),
                    "region should match the original region"
                );
            }
        }
    }
}

pub fn root_key_string(contents: &str, key: &str) -> Option<String> {
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

fn upsert_model_provider_config_with_session_provider(
    contents: &str,
    base_url: &str,
    bearer_token: &str,
    requires_openai_auth: bool,
    session_provider: RelaySessionProvider,
) -> anyhow::Result<String> {
    let mut doc = parse_toml_document(contents)?;
    let provider_id = if session_provider == RelaySessionProvider::Openai {
        RELAY_PROVIDER.to_string()
    } else {
        active_or_default_provider_id(&doc)
    };
    set_provider_id(
        &mut doc,
        if session_provider == RelaySessionProvider::Openai {
            session_provider.as_str()
        } else {
            &provider_id
        },
    );
    update_remote_control_openai_base_url(
        &mut doc,
        session_provider == RelaySessionProvider::Openai,
    );
    for legacy_provider in LEGACY_RELAY_PROVIDERS {
        remove_provider_table(&mut doc, legacy_provider);
    }
    if provider_id != RELAY_PROVIDER {
        remove_provider_table(&mut doc, RELAY_PROVIDER);
    }

    let provider = ensure_provider_table(&mut doc, &provider_id)?;
    provider["name"] = toml_edit::value(provider_id.as_str());
    provider["wire_api"] = toml_edit::value("responses");
    if requires_openai_auth {
        provider["requires_openai_auth"] = toml_edit::value(true);
    }
    provider["base_url"] = toml_edit::value(base_url);
    provider["experimental_bearer_token"] = toml_edit::value(bearer_token);

    Ok(move_model_providers_before_profiles(
        &ensure_trailing_newline(doc.to_string()),
    ))
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
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
        })
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
