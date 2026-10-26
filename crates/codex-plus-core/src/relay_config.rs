use anyhow::Context;
use serde::Serialize;
use serde_json::{Value, json};
use sha2::Digest as _;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use toml_edit::{DocumentMut, Item, Table, TableLike};

use crate::settings::{
    BackendSettings, RelayMode, RelayProfile, RelayProtocol, RelaySessionProvider, SettingsStore,
};

const RELAY_PROVIDER: &str = "custom";
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
    pub protocol_valid: bool,
    pub completion_status: String,
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
    pub skills: Vec<CodexContextEntry>,
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
            .is_some_and(|value| crate::protocol_proxy::is_managed_protocol_proxy_base_url(&value));
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
            && (has_bearer_token || has_auth_api_key)
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
    has_base_url && (has_bearer_token || has_auth_api_key)
}

pub fn responses_proxy_configured_in_home(home: &Path) -> bool {
    let contents = match std::fs::read_to_string(home.join("config.toml")) {
        Ok(contents) => contents,
        Err(_) => return false,
    };
    provider_string_from_config(&contents, "base_url").as_deref()
        == Some(
            crate::protocol_proxy::local_responses_proxy_base_url(
                crate::protocol_proxy::protocol_proxy_port(),
            )
            .as_str(),
        )
}

/// live `config.toml` 是否把 Codex 请求指向本机协议代理。
///
/// 覆盖两种形态：
/// - 普通中转：活动 provider 的 `model_providers.<id>.base_url` 是本机代理地址；
/// - 官方会话身份（Remote Control / 官方 + API 混合）：根键 `openai_base_url`
///   指向本机代理。
///
/// 只认活动 provider，避免配置里残留的其它 provider 造成误判。管理器据此判断
/// 「Codex 即使不经过 Codex++ 启动也会把请求发给本地代理」，从而决定是否常驻
/// 本地 helper。
pub fn live_config_uses_managed_protocol_proxy_in_home(home: &Path) -> bool {
    let contents = match std::fs::read_to_string(home.join("config.toml")) {
        Ok(contents) => contents,
        Err(_) => return false,
    };
    live_config_contents_use_managed_protocol_proxy(&contents)
}

fn live_config_contents_use_managed_protocol_proxy(contents: &str) -> bool {
    if root_key_string(contents, OPENAI_BASE_URL_KEY)
        .is_some_and(|value| crate::protocol_proxy::is_managed_protocol_proxy_base_url(&value))
    {
        return true;
    }
    let Ok(doc) = parse_toml_document(contents) else {
        return false;
    };
    let Some(provider_id) = active_provider_id(&doc) else {
        return false;
    };
    doc.get("model_providers")
        .and_then(Item::as_table)
        .and_then(|providers| providers.get(&provider_id))
        .and_then(Item::as_table)
        .and_then(|provider| provider.get("base_url"))
        .and_then(Item::as_str)
        .is_some_and(|value| crate::protocol_proxy::is_managed_protocol_proxy_base_url(value))
}

pub fn ensure_active_protocol_proxy_config_in_home(
    home: &Path,
    settings: &BackendSettings,
) -> anyhow::Result<bool> {
    // 总开关只约束配置接管；已有代理服务是否继续运行由启动器另行决定。
    if !settings.relay_profiles_enabled {
        return Ok(false);
    }
    let profile = settings.active_relay_profile();
    let known_managed = managed_proxy_base_urls_for_profile(&profile);
    let transport_uses_proxy = settings.active_relay_transport_uses_protocol_proxy();
    let openai_identity_uses_proxy = settings.active_relay_session_provider()
        == RelaySessionProvider::Openai
        || (profile.relay_mode == crate::settings::RelayMode::Official
            && profile.official_mix_api_key);

    let config_path = home.join("config.toml");
    let existing = match std::fs::read_to_string(&config_path) {
        Ok(existing) => existing,
        Err(error)
            if error.kind() == std::io::ErrorKind::NotFound
                && !transport_uses_proxy
                && !openai_identity_uses_proxy =>
        {
            return Ok(false);
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("读取 {} 失败", config_path.display()));
        }
    };
    let mut doc = parse_toml_document(&existing)?;
    let managed = managed_openai_base_url();
    let mut changed = false;
    let session_provider_id = active_session_provider_id(&doc);
    let transport_provider_id = if session_provider_id == "openai" {
        RELAY_PROVIDER.to_string()
    } else {
        active_or_default_provider_id(&doc)
    };

    // Existing configs can retain the built-in OpenAI display name after an
    // upgrade. Codex uses that name to enable remote compaction v2, which
    // third-party relays do not implement, so repair it before launch.
    let provider_name_needs_repair = session_provider_id != "openai"
        && doc
            .get("model_providers")
            .and_then(Item::as_table)
            .and_then(|providers| providers.get(&transport_provider_id))
            .and_then(Item::as_table)
            .and_then(|provider| provider.get("name"))
            .and_then(Item::as_str)
            .map(str::trim)
            == Some("OpenAI");
    if provider_name_needs_repair {
        let provider = doc
            .get_mut("model_providers")
            .and_then(Item::as_table_mut)
            .and_then(|providers| providers.get_mut(&transport_provider_id))
            .and_then(Item::as_table_mut)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "活动 provider 需要修复 model_providers.{transport_provider_id}.name"
                )
            })?;
        provider["name"] = toml_edit::value(transport_provider_id.as_str());
        changed = true;
    }

    if !transport_uses_proxy && !openai_identity_uses_proxy && !changed {
        return Ok(false);
    }

    if transport_uses_proxy {
        let provider = doc
            .get_mut("model_providers")
            .and_then(Item::as_table_mut)
            .and_then(|providers| providers.get_mut(&transport_provider_id))
            .and_then(Item::as_table_mut)
            .ok_or_else(|| {
                anyhow::anyhow!("活动协议代理需要现有 model_providers.{transport_provider_id} 配置")
            })?;
        let current = provider
            .get("base_url")
            .and_then(Item::as_str)
            .map(str::trim);
        if current != Some(managed.as_str()) {
            provider["base_url"] = toml_edit::value(managed.as_str());
            changed = true;
        }
    }

    if openai_identity_uses_proxy {
        let before = doc
            .get(OPENAI_BASE_URL_KEY)
            .and_then(Item::as_str)
            .map(str::trim)
            .map(ToString::to_string);
        update_remote_control_openai_base_url_with_known(&mut doc, true, managed.clone(), &known_managed);
        let after = doc
            .get(OPENAI_BASE_URL_KEY)
            .and_then(Item::as_str)
            .map(str::trim)
            .map(ToString::to_string);
        changed |= before != after;
    }

    if !changed {
        return Ok(false);
    }
    crate::settings::atomic_write(
        &config_path,
        ensure_trailing_newline(doc.to_string()).as_bytes(),
    )?;
    Ok(true)
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
        crate::protocol_proxy::protocol_proxy_port(),
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

/// 解析"要不要把默认成员的官方登录同步进 Codex"，以及要写进去的凭据。
///
/// 返回 `None` 表示本次切换不碰 Token 段（开关关着、不是聚合、或刚升级还没设置过）；
/// 返回 `Some(Some(contents))` 是要同步的 auth.json 片段；`Some(None)` 表示开关开着
/// 但默认成员没有可用的官方登录凭据，此时要顺手清掉 live 里残留的登录态。
fn resolve_synced_chatgpt_tokens(profile: &RelayProfile) -> SyncedChatgptTokens {
    if profile.relay_mode != crate::settings::RelayMode::Aggregate {
        return None;
    }
    let Ok(settings) = crate::settings::SettingsStore::default().load() else {
        return None;
    };
    let aggregate = settings
        .aggregate_relay_profiles
        .iter()
        .find(|aggregate| aggregate.id == profile.id)?;
    if !aggregate.sync_default_member_login {
        return None;
    }
    let default_relay_id = aggregate.default_relay_id.trim();
    if default_relay_id.is_empty() {
        return Some(String::new());
    }
    let Some(member) = settings
        .relay_profiles
        .iter()
        .find(|relay| relay.id == default_relay_id)
    else {
        return Some(String::new());
    };
    // 只有"纯官方登录"成员才有可写的 ChatGPT 凭据；带 URL/Key 的官方混入走 API 通道。
    // 拿不到凭据时返回空串：调用方据此清掉 live 里可能残留的、已经不参与聚合的登录态。
    if !crate::official_login::is_official_login_profile(member) {
        return Some(String::new());
    }
    Some(member.auth_contents.clone())
}

/// 聚合是否会把默认成员的 ChatGPT 登录同步进 Codex（开关开着且成员确有可用凭据）。
fn aggregate_syncs_member_login(profile: &RelayProfile) -> bool {
    resolve_synced_chatgpt_tokens(profile).is_some_and(|contents| {
        !crate::official_login::chatgpt_account_id_from_auth_contents(&contents).is_empty()
    })
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
    // 保留 live auth.json 里已有的其它凭据（尤其是官方 OAuth `tokens`）：
    // 整体覆盖成只含代理 Key 的 JSON 会清掉登录态，重启后 Codex 退回登录页
    // （issue #1604 / PR #1813 的数据完整性风险）。
    let auth_contents = auth_contents_with_proxy_key(home, "", bearer_token, None)?;
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
        crate::protocol_proxy::protocol_proxy_port(),
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
    with_model_catalog_rollback(home, profile, || {
        let config_with_catalog =
            apply_model_catalog_to_config(home, profile, &config_with_limits)?;
        let compatible_config =
            apply_deepseek_responses_compatibility(profile, &config_with_catalog)?;
        // 聚合 profile 的快照里没有凭据，必须现生成一份含代理 token 的合法 JSON，
        // 否则会把 auth.json 写成空文件（issue #1604）。
        let auth_contents = if profile.relay_mode == crate::settings::RelayMode::Aggregate {
            auth_contents_with_proxy_key(
                home,
                &profile.auth_contents,
                &relay_profile_api_key(profile),
                None,
            )?
        } else {
            profile.auth_contents.clone()
        };
        apply_relay_files_to_home(home, &compatible_config, &auth_contents)
    })
}

/// issue #2264：用户未声明默认模型时，重启后 Codex 会把默认 model 回落到
/// model_list 第一条，目标模式任务自动继续时被静默换模型。这里在 apply 时把
/// 未归档目标任务的持久化模型注入 profile.model（config_contents 不动），使
/// 回落链变为 profile.model → 目标任务模型 → model_list 第一条。
/// 显式配置了默认模型的用户不受影响——唯一的例外是值恰好等于 model_list
/// 第一条的存量污染（旧版把第一条写进 config.toml 后被 backfill 固化进
/// profile，并非用户显式意图），此时目标任务对齐优先。
fn align_profile_model_with_active_goal_thread(home: &Path, profile: &RelayProfile) -> RelayProfile {
    if profile.model_list.trim().is_empty() {
        return profile.clone();
    }
    let Some(goal_model) = crate::codex_sqlite::latest_unarchived_goal_thread_model(home)
        .and_then(|model| sanitize_relay_model_name(&model))
    else {
        return profile.clone();
    };
    let declared = relay_profile_model(profile).trim().to_string();
    if !declared.is_empty() && declared != model_list_head(profile) && declared != goal_model {
        return profile.clone();
    }
    let mut next = profile.clone();
    next.model = goal_model;
    // config_contents 里的工具写入 model 行会盖过注入的 profile.model
    // （relay_profile_model 是 config 优先），需要剥掉让注入生效。
    next.config_contents =
        strip_tool_written_model_from_config(home, profile, &next.config_contents);
    next
}

/// model_list 第一条（剥 catalog 后缀），与写入 config.toml 时的隐式默认一致。
fn model_list_head(profile: &RelayProfile) -> String {
    profile
        .model_list
        .split(['\r', '\n', ','])
        .map(str::trim)
        .find(|value| !value.is_empty())
        .map(|first| crate::model_suffix::parse_model_suffix(first).0)
        .unwrap_or_default()
}

/// issue #2264：config.toml 根部的 `model =` 可能是工具写入的隐式默认——
/// model_list 第一条，或目标任务对齐值——而非用户显式意图。backfill 时这类
/// 值不得固化进 profile（config_contents / profile.model），否则会反过来挡住
/// 后续的目标任务对齐，模型漂移重现。值与当前目标任务模型相等时视为工具
/// 写入是安全的：apply 每次都会按最新目标模型重新注入，不需要 backfill 保存。
fn live_model_is_tool_written(home: &Path, profile: &RelayProfile, model: &str) -> bool {
    if profile.relay_mode == RelayMode::Official && !profile.official_mix_api_key {
        return false;
    }
    let model = model.trim();
    if model.is_empty() {
        return false;
    }
    if model == model_list_head(profile) {
        return true;
    }
    crate::codex_sqlite::latest_unarchived_goal_thread_model(home)
        .map(|goal_model| goal_model.trim() == model)
        .unwrap_or(false)
}

/// backfill 场景下剥掉 config 文本里工具写入的隐式默认 `model =` 根键。
fn strip_tool_written_model_from_config(
    home: &Path,
    profile: &RelayProfile,
    config_text: &str,
) -> String {
    let Some(model) = root_key_string(config_text, "model") else {
        return config_text.to_string();
    };
    if !live_model_is_tool_written(home, profile, &model) {
        return config_text.to_string();
    }
    match parse_toml_document(config_text) {
        Ok(mut doc) => {
            doc.as_table_mut().remove("model");
            normalize_optional_toml(doc)
        }
        Err(_) => config_text.to_string(),
    }
}

/// issue #2264：重启链路上的模型漂移修复点。launcher 启动时不重放完整 apply
/// （避免覆盖 Codex 在 UI 选择后回写的 live 值），但 config.toml 里若还是
/// 「工具写入的隐式默认」——为空、等于 profile 模板声明值、或等于 model_list
/// 第一条——且存在未归档目标任务，就把根 model 改写为目标任务模型。返回是否
/// 发生了改写。用户/Codex 回写的 live 值不属于工具写入集合，一律保留。
pub fn align_live_config_model_with_goal_thread(
    home: &Path,
    profile: &RelayProfile,
) -> anyhow::Result<bool> {
    let config_path = home.join("config.toml");
    let Ok(text) = std::fs::read_to_string(&config_path) else {
        return Ok(false);
    };
    let live_model = root_key_string(&text, "model").unwrap_or_default();
    let live = live_model.trim();
    let declared = relay_profile_model(profile).trim().to_string();
    let head = model_list_head(profile);
    if !live.is_empty() && live != declared && live != head {
        return Ok(false);
    }
    let Some(goal_model) = crate::codex_sqlite::latest_unarchived_goal_thread_model(home)
        .and_then(|model| sanitize_relay_model_name(&model))
    else {
        return Ok(false);
    };
    if goal_model == live {
        return Ok(false);
    }
    let mut doc = parse_toml_document(&text)?;
    doc["model"] = toml_edit::value(goal_model.as_str());
    crate::settings::atomic_write(&config_path, doc.to_string().as_bytes())?;
    let _ = crate::diagnostic_log::append_diagnostic_log(
        "launcher.goal_thread_model_aligned",
        serde_json::json!({
            "from": live,
            "to": goal_model,
        }),
    );
    Ok(true)
}

pub fn apply_relay_profile_to_home_with_switch_rules(
    home: &Path,
    profile: &RelayProfile,
    common_config_contents: &str,
) -> anyhow::Result<RelayApplyResult> {
    apply_relay_profile_to_home_with_switch_rules_and_proxy_port(
        home,
        profile,
        common_config_contents,
        crate::protocol_proxy::protocol_proxy_port(),
    )
}

/// 全局保留偏好由调用者显式传入；旧入口保持默认关闭。
pub fn apply_relay_profile_with_preservation(
    home: &Path,
    profile: &RelayProfile,
    common: &str,
    proxy_port: u16,
    preserve_nonprovider: bool,
) -> anyhow::Result<RelayApplyResult> {
    if !preserve_nonprovider {
        return apply_relay_profile_to_home_with_switch_rules_and_proxy_port(home, profile, common, proxy_port);
    }
    let mut effective = profile.clone();
    let config = if profile.use_common_config {
        merge_common_config_into_config(&profile.config_contents, &prepare_common_config_for_apply(common)?)?
    } else { profile.config_contents.clone() };
    effective.config_contents = preserve_live_nonprovider_settings(home, &config)?;
    effective.use_common_config = false;
    apply_relay_profile_to_home_with_switch_rules_and_proxy_port(home, &effective, common, proxy_port)
}

pub fn preserve_live_nonprovider_settings(home: &Path, config: &str) -> anyhow::Result<String> {
    let mut target = parse_toml_document(config)?;
    let live = read_optional_text(&home.join("config.toml"))?;
    if live.trim().is_empty() { return Ok(normalize_optional_toml(target)); }
    // 同上：live 损坏时跳过补缺，不阻断切换。
    let Ok(live) = parse_toml_document(&normalize_duplicate_toml_text(&live)) else {
        return Ok(normalize_optional_toml(target));
    };
    // 按项目补缺，不递归提升一个已有项目的 trust 权限。
    preserve_missing_table_keys(&mut target, &live, "projects");
    if target.get("notify").is_none() {
        if let Some(notify) = live.get("notify") { target["notify"] = notify.clone(); }
    }
    Ok(normalize_optional_toml(target))
}

/// 与 [`apply_relay_profile_to_home_with_switch_rules`] 相同，但允许调用方指定
/// 协议代理端口。多开实例各自运行一个 helper，必须在写入 config.toml 时使用
/// 实例自己的端口，不能先写默认 57321 再依赖启动器补救。
pub fn apply_relay_profile_to_home_with_switch_rules_and_proxy_port(
    home: &Path,
    profile: &RelayProfile,
    common_config_contents: &str,
    proxy_port: u16,
) -> anyhow::Result<RelayApplyResult> {
    let previous_model = live_root_model(home);
    let result =
        apply_relay_profile_to_home_inner(home, profile, common_config_contents, proxy_port)?;
    // issue #2081：默认模型变了就记一笔，旧会话迁移留到下次启动前执行（opt-in）。
    let new_model = live_root_model(home);
    if let Err(error) = crate::codex_sqlite::record_pending_thread_model_migration(
        home,
        &previous_model,
        &new_model,
    ) {
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "relay_config.record_thread_model_migration_failed",
            serde_json::json!({ "message": error.to_string() }),
        );
    }
    Ok(result)
}

fn live_root_model(home: &Path) -> String {
    std::fs::read_to_string(home.join("config.toml"))
        .ok()
        .and_then(|text| root_key_string(&text, "model"))
        .unwrap_or_default()
}

fn apply_relay_profile_to_home_inner(
    home: &Path,
    profile: &RelayProfile,
    common_config_contents: &str,
    proxy_port: u16,
) -> anyhow::Result<RelayApplyResult> {
    if profile.relay_mode == RelayMode::Official && !profile.official_mix_api_key {
        return apply_official_profile_to_home(home, profile, common_config_contents);
    }
    let profile = align_profile_model_with_active_goal_thread(home, profile);
    let profile = &profile;
    let selected_common = if profile.use_common_config {
        prepare_common_config_for_apply(common_config_contents)?
    } else {
        String::new()
    };
    let profile_config = complete_relay_profile_config_with_proxy_port(profile, proxy_port)?;
    let config_with_common = merge_common_config_into_config(&profile_config, &selected_common)?;
    let config_with_common =
        preserve_unmanaged_live_context_entries(home, &config_with_common, common_config_contents)?;
    let config_with_limits = apply_context_limits_to_config(
        &config_with_common,
        &profile.context_window,
        &profile.auto_compact_limit,
    )?;
    with_model_catalog_rollback(home, profile, || {
        let config_with_catalog =
            apply_model_catalog_to_config(home, profile, &config_with_limits)?;
        let compatible_config =
            apply_deepseek_responses_compatibility(profile, &config_with_catalog)?;

        if profile.relay_mode == crate::settings::RelayMode::PureApi {
            // 与 Aggregate / Official 分支对称：先读 live auth.json 再合并，
            // 不能直接把 profile 快照整体写进去（issue #2173）。
            // 纯 API 供应商每切换一次就可能把 live 里的 `tokens` 抹掉，从而静默
            // 摧毁「OpenAI 会话身份」这个唯一能让 CUA 浏览器插件可用的 workaround——
            // 用户表现为「本来能用，某天开始报 unsupported Codex auth method: apikey」，
            // 且没有任何配置报错。
            let auth_contents = pure_api_auth_contents_with_live_login(home, profile)?;
            apply_relay_files_to_home(home, &compatible_config, &auth_contents)
        } else if profile.relay_mode == crate::settings::RelayMode::Aggregate {
            // 聚合模式的实际请求发往本地代理，它需要 API 模式的凭据。
            // 不能走 Official 分支删 OPENAI_API_KEY，否则 auth.json 会被清成空文件/空对象，
            // Codex 判定未登录而弹登录页（issue #1604）。
            //
            // 聚合默认只写代理 Key，Codex 侧看不到官方成员的账号态；用户在聚合里打开
            // 「同步默认成员登录」后，这里再把该成员的 ChatGPT 凭据并进去，账号在 Codex
            // 里就是登录态（`resolve_synced_chatgpt_tokens` 负责判定，默认返回 None）。
            let synced_login = resolve_synced_chatgpt_tokens(profile);
            let mut auth_contents = auth_contents_with_proxy_key(
                home,
                &profile.auth_contents,
                &relay_profile_api_key(profile),
                synced_login.as_deref(),
            )?;
            // 把默认成员的登录态并进 `tokens` 段：同账号用 live 的更新一代，代理 Key 与
            // 用户自己的字段都保留。`synced_login` 为空串表示成员没有可用凭据 → 清掉残留。
            if let Some(member_auth) = synced_login.as_deref()
                && let Some(merged) = crate::official_login::auth_contents_with_member_login(
                    &auth_contents,
                    member_auth,
                    true,
                )
            {
                auth_contents = merged;
            }
            apply_relay_files_to_home(home, &compatible_config, &auth_contents)
        } else {
            let auth_contents = official_profile_auth_for_switch(home, &profile.auth_contents)?;
            apply_relay_files_to_home(home, &compatible_config, &auth_contents)
        }
    })
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
    with_model_catalog_rollback(home, profile, || {
        let config_with_catalog =
            apply_model_catalog_to_config(home, profile, &config_with_limits)?;
        let compatible_config =
            apply_deepseek_responses_compatibility(profile, &config_with_catalog)?;
        apply_relay_config_file_to_home(home, &compatible_config)
    })
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
    // 官方 profile 的 `configContents` 在管理端归一化时会被清空
    // （App.tsx 里 `relayMode === "official"` 分支写空串），于是 `relay_profile_base_url`
    // 返回空串、被下面的空值校验直接拒绝 —— 而官方模式下界面根本没有 base_url 输入框，
    // 用户无处可填（issue #1488）。这里给官方模式兜一个内置地址。
    let mut base_url = relay_profile_base_url(profile);
    if base_url.trim().is_empty()
        && profile.relay_mode == crate::settings::RelayMode::Official
    {
        base_url = crate::official_login::official_base_url();
    }
    let base_url = base_url.trim().trim_end_matches('/');
    if base_url.is_empty() {
        anyhow::bail!("Base URL 不能为空");
    }
    // 纯官方登录 profile 没有 API Key，测试请求同样要带该账号的 OAuth access token。
    let api_key = if crate::official_login::is_official_login_profile(profile)
        && crate::official_login::official_login_available(profile)
    {
        crate::official_login::resolve_official_credential(profile, false)
            .await?
            .access_token
    } else {
        relay_profile_api_key(profile)
    };
    let api_key = api_key.trim();
    if api_key.is_empty() && !profile.uses_no_auth() {
        anyhow::bail!("API Key 不能为空");
    }

    let user_agent = if profile.protocol == RelayProtocol::AnthropicMessages && profile.anthropic_impersonate_claude_code {
        crate::anthropic_bridge::CLAUDE_CODE_USER_AGENT
    } else if !profile.user_agent.trim().is_empty() { profile.user_agent.trim() }
    else { "CodexPlusPlus/RelayTest" };
    let client = crate::http_client::proxied_client_for_profile(user_agent, &profile.id)?;
    let endpoint = match profile.protocol {
        RelayProtocol::Responses => crate::protocol_proxy::responses_url(base_url),
        RelayProtocol::ChatCompletions => crate::protocol_proxy::chat_completions_url(base_url),
        RelayProtocol::AnthropicMessages => {
            crate::anthropic_bridge::anthropic_messages_url(base_url)
        }
    };
    let test_model = model.trim();
    if test_model.is_empty() {
        anyhow::bail!("测试模型不能为空");
    }

    let payload = relay_profile_test_payload(profile.protocol, test_model);
    let response = relay_test_request(&client, &endpoint, profile, api_key, &payload)
        .send()
        .await?;
    let http_status = response.status().as_u16();

    // 如果 404 且 base_url 末尾没有 /v1，尝试自动补 /v1 后再发一次。
    // 许多上游（中转站、自建代理）暴露的路径以 /v1/ 开头，
    // 用户容易遗漏这个前缀，导致 /responses 或 /chat/completions 404。
    let complete_endpoint = ["/responses", "/chat/completions", "/messages"].iter()
        .any(|suffix| base_url.trim_end_matches('#').to_ascii_lowercase().ends_with(suffix));
    if http_status == 404 && !base_url.ends_with("/v1") && !base_url.ends_with('#') && !complete_endpoint {
        let v1_url = format!("{base_url}/v1");
        let v1_endpoint = match profile.protocol {
            RelayProtocol::Responses => crate::protocol_proxy::responses_url(&v1_url),
            RelayProtocol::ChatCompletions => crate::protocol_proxy::chat_completions_url(&v1_url),
            RelayProtocol::AnthropicMessages => {
                crate::anthropic_bridge::anthropic_messages_url(&v1_url)
            }
        };
        let v1_response = relay_test_request(&client, &v1_endpoint, profile, api_key, &payload)
            .send()
            .await?;
        let v1_status = v1_response.status().as_u16();
        if v1_status < 400 {
            let response_text = v1_response.text().await.unwrap_or_default();
            let (protocol_valid, completion_status) = validate_relay_test_response(profile.protocol, v1_status, &response_text);
            if protocol_valid { return Ok(RelayProfileTestResult {
                protocol_valid,
                completion_status,
                http_status: v1_status,
                endpoint: v1_endpoint,
                response_preview: format!(
                    "（Base URL 建议加上 /v1 前缀）{}",
                    response_text.chars().take(280).collect::<String>()
                ),
            }); }
        }
    }

    let response_text = response.text().await.unwrap_or_default();
    let (protocol_valid, completion_status) = validate_relay_test_response(profile.protocol, http_status, &response_text);
    Ok(RelayProfileTestResult {
        http_status,
        endpoint,
        response_preview: response_text.chars().take(320).collect(),
        protocol_valid,
        completion_status,
    })
}

/// 模型协议验收与 HTTP 连通分开；合法但被 token 预算截断的响应显示警告。
pub fn validate_relay_test_response(protocol: RelayProtocol, status: u16, text: &str) -> (bool, String) {
    let failed = || (false, "failed".to_string());
    if !(200..300).contains(&status) { return failed(); }
    let Ok(value) = serde_json::from_str::<Value>(text) else { return failed(); };
    if value.get("error").is_some_and(|v| !v.is_null()) { return failed(); }
    let state = match protocol {
        RelayProtocol::Responses => {
            if value.get("output").and_then(Value::as_array).is_none() { return failed(); }
            // 测试请求自己把输出限制在 16 token，推理模型因此截断属正常，不算告警（#2638）
            let truncated_by_test_limit = value
                .pointer("/incomplete_details/reason")
                .and_then(Value::as_str)
                == Some("max_output_tokens");
            match value.get("status").and_then(Value::as_str) {
                Some("completed") => "completed",
                Some("incomplete") if truncated_by_test_limit => "completed",
                Some("incomplete") => "incomplete",
                _ => return failed(),
            }
        },
        RelayProtocol::ChatCompletions => {
            let Some(choices) = value.get("choices").and_then(Value::as_array).filter(|v| !v.is_empty()) else { return failed(); };
            if choices.iter().any(|c| !c.get("message").is_some_and(Value::is_object)) { return failed(); }
            if choices.iter().all(|c| matches!(c.get("finish_reason").and_then(Value::as_str), Some("stop" | "tool_calls" | "function_call" | "length"))) { "completed" }
            else if choices.iter().all(|c| matches!(c.get("finish_reason").and_then(Value::as_str), Some("stop" | "tool_calls" | "function_call" | "length" | "content_filter"))) { "incomplete" }
            else { return failed(); }
        },
        RelayProtocol::AnthropicMessages => {
            if value.get("type").and_then(Value::as_str) != Some("message") || value.get("content").and_then(Value::as_array).is_none() { return failed(); }
            match value.get("stop_reason").and_then(Value::as_str) {
                Some("end_turn" | "stop_sequence" | "tool_use" | "max_tokens") => "completed",
                Some("pause_turn" | "refusal") => "incomplete",
                _ => return failed(),
            }
        },
    };
    (true, state.to_string())
}

/// 供应商测试请求：Content-Type + 认证 + 供应商自定义请求头。
///
/// 自定义头走 `relay_headers`，与协议代理、模型列表共用同一套优先级与过滤规则，
/// 避免三处行为不一致（issue #1685）。
fn relay_test_request(
    client: &reqwest::Client,
    endpoint: &str,
    profile: &RelayProfile,
    api_key: &str,
    payload: &Value,
) -> reqwest::RequestBuilder {
    let request = client
        .post(endpoint)
        .header(reqwest::header::CONTENT_TYPE, "application/json");
    // 认证字段按协议注入（Anthropic 上游是 x-api-key，其余是 Bearer），
    // 与协议代理转发共用 `relay_headers`，避免两处行为不一致。
    crate::relay_headers::apply_with_api_key(request, profile, api_key).json(payload)
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
        RelayProtocol::AnthropicMessages => serde_json::json!({
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
        // 需要协议转换的上游一律让 Codex 连本机代理，由代理决定真实上游协议。
        RelayProtocol::ChatCompletions | RelayProtocol::AnthropicMessages => {
            crate::protocol_proxy::local_responses_proxy_base_url(proxy_port)
        }
    }
}

const OPENAI_BASE_URL_KEY: &str = "openai_base_url";

fn managed_openai_base_url() -> String {
    crate::protocol_proxy::local_responses_proxy_base_url(
        crate::protocol_proxy::protocol_proxy_port(),
    )
}

fn update_remote_control_openai_base_url(doc: &mut DocumentMut, enabled: bool) {
    update_remote_control_openai_base_url_with(doc, enabled, managed_openai_base_url());
}

fn update_remote_control_openai_base_url_with(doc: &mut DocumentMut, enabled: bool, managed: String) {
    update_remote_control_openai_base_url_with_known(
        doc,
        enabled,
        managed,
        &crate::protocol_proxy::managed_protocol_proxy_base_urls(),
    );
}

fn update_remote_control_openai_base_url_with_known(
    doc: &mut DocumentMut,
    enabled: bool,
    managed: String,
    known_managed: &[String],
) {
    let current = doc
        .get(OPENAI_BASE_URL_KEY)
        .and_then(Item::as_str)
        .map(str::trim)
        .map(ToString::to_string);

    // 迁移两次及以上时，历史受管地址也必须跟进；其它回环地址原样保留。
    let ours = |value: &str| {
        let normalized = normalize_relay_endpoint(value);
        normalized == normalize_relay_endpoint(&managed)
            || known_managed.iter().any(|url| normalized == normalize_relay_endpoint(url))
    };
    if enabled {
        if current.as_deref().is_none_or(ours) {
            doc[OPENAI_BASE_URL_KEY] = toml_edit::value(managed);
        }
    } else if current.as_deref().is_some_and(ours) {
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

/// 卸载 Codex++ 时调用：live `config.toml` 仍指向本机协议代理时恢复为官方配置。
///
/// 卸载后本地代理不再运行，残留的 `base_url = "http://127.0.0.1:57321/v1"`
/// 会让官方 Codex 每次请求都 502（issue #406）。只在确认指向本机代理时才清理
/// （等价于在管理器里「启用官方」，会先写备份）；直连远程中转站、官方登录等
/// 其它配置原样保留。返回是否改动了 live 配置。
pub fn clear_managed_proxy_config_for_uninstall(home: &Path) -> anyhow::Result<bool> {
    if !live_config_uses_managed_protocol_proxy_in_home(home) {
        return Ok(false);
    }
    clear_relay_config_to_home(home)?;
    Ok(true)
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
    // 必须在删除 model_provider 之前读：下面的根键清理会把判断依据删掉。
    // 用它决定要不要连 model 一起清——中转站选的模型名留在官方模式下，
    // 模型选择器仍会显示它（issue #2216）。
    let active_provider = parse_toml_document(&existing)
        .ok()
        .and_then(|doc| active_provider_id(&doc));
    let mut doc = parse_toml_document(&existing)?;
    // 官方 HTTP provider 只改变传输方式，重启同步不能取消它，否则会重新启用 WebSocket。
    // 只认可无额外路由/凭据字段的显式配置，不能仅凭 provider 名字保留第三方连接。
    let preserve_official_http = active_provider
        .as_deref()
        .and_then(|id| doc.get("model_providers")?.get(id))
        .and_then(Item::as_table_like)
        .is_some_and(|provider| {
            provider.get("wire_api").and_then(Item::as_str) == Some("responses")
                && provider.get("requires_openai_auth").and_then(Item::as_bool) == Some(true)
                && provider.get("supports_websockets").and_then(Item::as_bool) == Some(false)
                && provider.iter().all(|(key, _)| {
                    matches!(
                        key,
                        "name" | "wire_api" | "requires_openai_auth" | "supports_websockets"
                    )
                })
        });
    // 除上面确认的官方 HTTP 配置外，中转站与旧版 provider 的整段配置一律移除：
    // 只删认证字段会留下 base_url（切回官方后请求仍发往中转站，issue #2216），
    // 而「未被激活就跳过」会让切走时残留的 token 继续留在文件里。
    // 这里清的是 Codex++ 自己管理的 provider（custom / 旧版名），
    // 用户自定义的其它 provider（如 custom1）不受影响。
    for legacy_provider in LEGACY_RELAY_PROVIDERS {
        if !preserve_official_http || active_provider.as_deref() != Some(*legacy_provider) {
            remove_provider_table(&mut doc, legacy_provider);
        }
    }
    if !preserve_official_http || active_provider.as_deref() != Some(RELAY_PROVIDER) {
        remove_provider_table(&mut doc, RELAY_PROVIDER);
    }
    let mut updated = normalize_optional_toml(doc);
    for key in [
        "OPENAI_API_KEY",
        "model_provider",
        "model_catalog_json",
        "model_context_window",
        "model_auto_compact_token_limit",
        "base_url",
        "experimental_bearer_token",
        "env_key",
        "requires_openai_auth",
    ] {
        if key == "model_provider" && preserve_official_http {
            continue;
        }
        updated = remove_root_key(&updated, key);
    }
    // 只用在中转站 provider 名下时才清 model；用户手写的官方模型名要保留。
    if !preserve_official_http
        && (active_provider.as_deref() == Some(RELAY_PROVIDER)
            || active_provider
                .as_deref()
                .is_some_and(|provider| LEGACY_RELAY_PROVIDERS.contains(&provider)))
    {
        updated = remove_root_key(&updated, "model");
    }
    updated = remove_managed_remote_control_openai_base_url(&updated)?;
    let backup_path = write_codex_live_atomic(home, Some(&updated), auth_bytes.as_deref())?;
    let status = relay_config_status_from_home(home);
    Ok(RelayApplyResult {
        config_path: status.config_path,
        backup_path,
        configured: status.configured,
    })
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
    let live_contents = read_optional_text(&home.join("config.toml"))?;
    profile.config_contents = strip_tool_written_model_from_config(home, profile, &live_contents);
    profile.auth_contents = read_optional_text(&home.join("auth.json"))?;
    let live_config = profile.config_contents.clone();
    sync_context_limits_from_config(profile, &live_config);
    if profile.model.trim().is_empty() {
        if let Some(model) = root_key_string(&live_config, "model")
            .and_then(|model| sanitize_relay_model_name(&model))
        {
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
    backfill_relay_profile_from_home_with_common_and_policy(
        home,
        profile,
        common_config_contents,
        RelayBackfillPolicy::PreserveIdentity,
    )
}

/// 自动回填不猜测外部工具与合法手工编辑的归属；身份改变须由用户显式采纳。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RelayBackfillPolicy {
    #[default]
    PreserveIdentity,
    AdoptLiveIdentity,
}

pub fn backfill_relay_profile_from_home_with_common_and_policy(
    home: &Path,
    profile: &mut RelayProfile,
    common_config_contents: &mut String,
    policy: RelayBackfillPolicy,
) -> anyhow::Result<()> {
    // Normalize before backfilling: the live config may carry a corrupted shape
    // (for example two [mcp_servers.node_repl] headers under one parent), and
    // copying it verbatim into the profile template would freeze that forever.
    let live_config =
        normalize_duplicate_toml_text(&read_optional_text(&home.join("config.toml"))?);
    let live_auth = read_optional_text(&home.join("auth.json"))?;
    let official_login = profile.relay_mode == RelayMode::Official && !profile.official_mix_api_key;
    let no_config_source =
        parse_toml_document(&live_config).is_ok_and(|doc| doc.as_table().is_empty());
    if no_config_source && (policy == RelayBackfillPolicy::PreserveIdentity || official_login) {
        // 新CODEX_HOME/空文件/仅注释不是归属漂移：没有来源就不覆盖已保存的快照。
        // 纯官方仍可以刷新明确的OAuth登录，但不能用空config抹掉专属设置。
        if official_login && auth_contents_looks_like_chatgpt_auth(&live_auth) {
            profile.auth_contents = remove_openai_api_key_from_auth_contents(&live_auth)?;
        }
        return Ok(());
    }
    if policy == RelayBackfillPolicy::PreserveIdentity {
        validate_backfill_endpoint_identity(profile, &live_config)?;
    } else if !(profile.relay_mode == RelayMode::Official && !profile.official_mix_api_key) {
        if profile.relay_mode == RelayMode::Aggregate {
            anyhow::bail!(
                "聚合供应商由成员配置定义，不能从实时代理配置采纳单一上游；请显式导入为普通 API 供应商。"
            );
        }
        let live_endpoint = provider_string_from_config(&live_config, "base_url");
        if !live_endpoint.as_deref().is_some_and(valid_relay_endpoint) {
            anyhow::bail!(
                "实时配置缺少有效的供应商 URL，已保留原快照；请显式导入完整的上游 http(s) URL 与凭据。"
            );
        }
        if recognized_managed_proxy_endpoint(profile, &live_config)
            || live_endpoint.as_deref().is_some_and(|endpoint| {
                crate::protocol_proxy::is_managed_protocol_proxy_base_url(endpoint)
            })
        {
            anyhow::bail!(
                "实时配置指向本地托管代理，不能作为新的上游直接采纳。请导入真实上游的 URL 与凭据。"
            );
        }
    }
    // 所有解析/归属检查先在副本上完成；错误不得留下半份 URL/Key 快照。
    let mut next = profile.clone();
    backfill_profile_from_text(
        &mut next,
        common_config_contents,
        home,
        &live_config,
        &live_auth,
        policy,
    )?;
    if normalize_duplicate_toml_text(&read_optional_text(&home.join("config.toml"))?) != live_config
        || read_optional_text(&home.join("auth.json"))? != live_auth
    {
        anyhow::bail!("实时配置在回填过程中发生变化，已保留原快照；请等待其它配置编辑完成后重试。");
    }
    *profile = next;
    Ok(())
}

fn backfill_profile_from_text(
    profile: &mut RelayProfile,
    common_config_contents: &str,
    home: &Path,
    live_config: &str,
    live_auth: &str,
    policy: RelayBackfillPolicy,
) -> anyhow::Result<()> {
    let template_config = profile.config_contents.clone();
    let template_auth = profile.auth_contents.clone();
    let template_api_key = relay_profile_api_key(profile);
    let managed_proxy = recognized_managed_proxy_endpoint(profile, live_config);
    let mut template_base_url = relay_profile_base_url(profile);
    if managed_proxy
        && is_loopback_responses_endpoint(&template_base_url)
        && !profile.upstream_base_url.trim().is_empty()
        && !is_loopback_responses_endpoint(&profile.upstream_base_url)
    {
        template_base_url = profile.upstream_base_url.trim().to_string();
    }
    profile.config_contents = if profile.use_common_config {
        strip_common_config_from_config(&live_config, common_config_contents)?
    } else {
        ensure_trailing_newline(live_config.to_string())
    };
    profile.config_contents =
        restore_profile_provider_id_for_backfill(&profile.config_contents, &template_config)?;
    profile.config_contents =
        strip_tool_written_model_from_config(home, profile, &profile.config_contents);
    if profile.protocol == RelayProtocol::Responses
        && managed_proxy
        && !template_base_url.trim().is_empty()
    {
        let mut doc = parse_toml_document(&profile.config_contents)?;
        let provider_id = active_or_default_provider_id(&doc);
        ensure_provider_table(&mut doc, &provider_id)?["base_url"] =
            toml_edit::value(template_base_url.trim());
        profile.config_contents =
            move_model_providers_before_profiles(&ensure_trailing_newline(doc.to_string()));
    }
    if policy == RelayBackfillPolicy::AdoptLiveIdentity
        && !(profile.relay_mode == RelayMode::Official && !profile.official_mix_api_key)
    {
        // 显式导入必须从同一套 live 配置取 endpoint 和凭据，不拼接旧 profile Key。
        profile.auth_contents = live_auth.to_string();
        profile.api_key.clear();
        // 任意自定义头都可能承载供应商鉴权，完整身份采纳不继承旧头（默认回填不动）。
        profile.custom_headers.clear();
        profile.base_url = provider_string_from_config(live_config, "base_url").unwrap_or_default();
        profile.upstream_base_url = profile.base_url.clone();
        // 已拒绝托管代理；现在采纳的是provider真实端点，旧Chat marker不能串入旧URL。
        profile.config_contents =
            remove_root_key(&profile.config_contents, CHAT_UPSTREAM_BASE_URL_KEY);
        let live_key = experimental_bearer_token_from_config(live_config)?
            .or_else(|| codex_auth_api_key(live_auth))
            .unwrap_or_default();
        if live_key.trim().is_empty() && !profile.uses_no_auth() {
            anyhow::bail!(
                "实时配置没有可采纳的供应商 API Key；已保留原快照，请显式导入完整 URL 与凭据。"
            );
        }
        profile.api_keys = vec![crate::settings::RelayApiKey {
            id: "live-import".into(), name: "导入".into(), api_key: live_key.clone(),
        }];
        profile.active_api_key_id = "live-import".into();
        restore_profile_credentials_after_backfill(profile, live_auth, &live_key, live_auth)?;
    } else {
        restore_profile_credentials_after_backfill(
            profile,
            &template_auth,
            &template_api_key,
            live_auth,
        )?;
    }
    sync_profile_mode_from_backfilled_live(profile);
    sync_context_limits_from_config(profile, &live_config);
    // 回填源用剥离工具写入 model 后的 config_contents：live_config 里的
    // `model =` 可能是 apply 写入的隐式默认/目标任务对齐值，固化进
    // profile.model 会挡住后续对齐（issue #2264）。
    if profile.model.trim().is_empty() {
        if let Some(model) = root_key_string(&profile.config_contents, "model")
            .and_then(|model| sanitize_relay_model_name(&model))
        {
            profile.model = model;
        }
    }
    Ok(())
}

fn normalize_relay_endpoint(value: &str) -> String {
    reqwest::Url::parse(value.trim())
        .map(|mut url| {
            let path = url.path().trim_end_matches('/').to_string();
            url.set_path(&path);
            url.to_string()
        })
        .unwrap_or_else(|_| value.trim().to_string())
}

fn is_loopback_responses_endpoint(endpoint: &str) -> bool {
    reqwest::Url::parse(endpoint).is_ok_and(|url| {
        url.scheme() == "http"
            && url.host_str() == Some("127.0.0.1")
            && url.port().is_some()
            && url.path().trim_end_matches('/') == "/v1"
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
    })
}

fn valid_relay_endpoint(endpoint: &str) -> bool {
    reqwest::Url::parse(endpoint.trim())
        .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some())
}

fn recognized_managed_proxy_endpoint(profile: &RelayProfile, config: &str) -> bool {
    let Some(live) = provider_string_from_config(config, "base_url") else {
        return false;
    };
    if !is_loopback_responses_endpoint(&live) {
        return false;
    }
    // 存量版本以固定的保留端口识别本工具代理；不能放宽成任意 localhost。
    if crate::protocol_proxy::is_managed_protocol_proxy_base_url(&live) {
        return true;
    }
    let proxy_mode = profile.requires_protocol_proxy_conversion()
        || profile.has_model_routes()
        || profile.uses_no_auth()
        || profile.uses_independent_proxy()
        || profile.web_search_history_compat_enabled()
        || profile.request_compatibility_enabled()
        || profile.rewrites_auto_review_model()
        || profile.relay_mode == RelayMode::Aggregate;
    let saved_endpoint_matches = provider_string_from_config(&profile.config_contents, "base_url")
        .is_some_and(|saved| {
            is_loopback_responses_endpoint(&saved)
                && normalize_relay_endpoint(&saved) == normalize_relay_endpoint(&live)
        });
    let legacy_upstream_matches =
        root_key_string(config, CHAT_UPSTREAM_BASE_URL_KEY).is_some_and(|upstream| {
            normalize_relay_endpoint(&upstream)
                == normalize_relay_endpoint(&relay_profile_base_url(profile))
        });
    let saved_real_upstream = !profile.upstream_base_url.trim().is_empty()
        && !is_loopback_responses_endpoint(&profile.upstream_base_url);
    (proxy_mode && (saved_endpoint_matches || legacy_upstream_matches))
        || (saved_endpoint_matches && saved_real_upstream)
}

fn managed_proxy_base_urls_for_profile(profile: &RelayProfile) -> Vec<String> {
    let mut known = crate::protocol_proxy::managed_protocol_proxy_base_urls();
    // 升级前已经迁移过的版本没有历史端口文件。只从既有归属校验确认的
    // transport 快照补回旧地址，不能单凭 root 上的任意回环地址认领它。
    if recognized_managed_proxy_endpoint(profile, &profile.config_contents)
        && let Some(saved) = provider_string_from_config(&profile.config_contents, "base_url")
    {
        known.push(saved);
    }
    known
}

fn saved_proxy_endpoint_has_real_upstream(profile: &RelayProfile, endpoint: &str) -> bool {
    is_loopback_responses_endpoint(endpoint)
        && valid_relay_endpoint(&profile.upstream_base_url)
        && !is_loopback_responses_endpoint(&profile.upstream_base_url)
}

pub(crate) fn live_endpoint_matches_profile(profile: &RelayProfile, config: &str) -> bool {
    let Some(live) = provider_string_from_config(config, "base_url") else {
        return false;
    };
    normalize_relay_endpoint(&live) == normalize_relay_endpoint(&relay_profile_base_url(profile))
}

/// 自动切换前识别已经归属于其它已保存供应商的 endpoint；不依赖 Key 是否共享。
pub fn live_config_matches_other_profile_in_home(
    home: &Path,
    settings: &BackendSettings,
    previous_profile_id: &str,
) -> anyhow::Result<bool> {
    let live = normalize_duplicate_toml_text(&read_optional_text(&home.join("config.toml"))?);
    let live_doc = parse_toml_document(&live).map_err(|_| {
        anyhow::anyhow!(
            "实时 config.toml 无法解析，已停止归属判定并保留原文件；请检查 TOML 语法后重试。"
        )
    })?;
    if active_provider_id(&live_doc).is_none() {
        return Ok(false);
    }
    let Some(previous) = settings
        .relay_profiles
        .iter()
        .find(|profile| profile.id == previous_profile_id)
    else {
        return Ok(false);
    };
    // 共享本工具的本地端口不是供应商身份，不可误认成另一个聚合profile。
    if recognized_managed_proxy_endpoint(previous, &live) {
        return Ok(false);
    }
    if live_endpoint_matches_profile(previous, &live) {
        let auth = read_optional_text(&home.join("auth.json"))?;
        let live_key =
            experimental_bearer_token_from_config(&live)?.or_else(|| codex_auth_api_key(&auth));
        return Ok(live_key
            .filter(|key| !key.trim().is_empty())
            .is_some_and(|key| {
                key != relay_profile_api_key(previous)
                    && settings.relay_profiles.iter().any(|profile| {
                        profile.id != previous_profile_id
                            && live_endpoint_matches_profile(profile, &live)
                            && relay_profile_api_key(profile) == key
                    })
            }));
    }
    Ok(settings.relay_profiles.iter().any(|profile| {
        profile.id != previous_profile_id && live_endpoint_matches_profile(profile, &live)
    }))
}

/// 回填时发现实时配置的 endpoint 与供应商快照对不上：分不清是外部工具接管还是
/// 用户手工改的，自动回填只能停下（#2666）。单独成类型，切换流程据此让用户
/// 显式选择「导入为新供应商」或「放弃实时改动」，而不是只给一句报错。
#[derive(Debug)]
pub struct LiveEndpointChanged;

impl std::fmt::Display for LiveEndpointChanged {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(
            "实时供应商 endpoint 已改变，无法区分外部接管与手工编辑，已停止回填并保留原配置。请先将实时 config.toml / auth.json 显式导入为新供应商，或在编辑器明确保存新的 URL 与 Key 后再切换。",
        )
    }
}

impl std::error::Error for LiveEndpointChanged {}

fn validate_backfill_endpoint_identity(profile: &RelayProfile, config: &str) -> anyhow::Result<()> {
    if profile.relay_mode == RelayMode::Official && !profile.official_mix_api_key {
        return Ok(());
    }
    // 尚未保存过供应商配置的旧/新 profile 保留首次采集兼容性。
    if profile.config_contents.trim().is_empty()
        && profile.upstream_base_url.trim().is_empty()
        && profile.api_key.trim().is_empty()
        && relay_profile_api_key(profile).trim().is_empty()
        && profile.custom_headers.is_empty()
        && (profile.base_url.trim().is_empty()
            || profile.base_url.trim() == RelayProfile::default().base_url.trim())
    {
        return Ok(());
    }
    if !provider_string_from_config(config, "base_url")
        .as_deref()
        .is_some_and(valid_relay_endpoint)
    {
        anyhow::bail!(
            "实时配置缺少有效的供应商 endpoint，已停止回填并保留原快照。请先显式导入完整上游 URL 与凭据，或确认当前登录模式后重试。"
        );
    }
    if recognized_managed_proxy_endpoint(profile, config)
        || live_endpoint_matches_profile(profile, config)
    {
        return Ok(());
    }
    Err(LiveEndpointChanged.into())
}

/// 纯官方快照保留非认证设置，去掉第三方路由/认证和托管目录指针。
pub fn sanitize_official_profile_config(contents: &str) -> anyhow::Result<String> {
    let normalized = normalize_duplicate_toml_text(contents);
    let mut doc = parse_toml_document(&normalized)
        .map_err(|_| anyhow::anyhow!("纯官方配置快照 TOML 无法解析，已拒绝覆盖原配置。"))?;
    sanitize_official_profile_table(doc.as_table_mut());
    update_remote_control_openai_base_url(&mut doc, false);
    Ok(normalize_optional_toml(doc))
}

fn sanitize_official_profile_table(table: &mut dyn TableLike) {
    let third_party_model = table
        .get("model_provider")
        .and_then(Item::as_str)
        .is_some_and(|provider| provider != "openai");
    if third_party_model {
        table.remove("model");
    }
    let credentials: Vec<String> = table
        .iter()
        .map(|(key, _)| key.to_string())
        .filter(|key| is_provider_credential_root_key(key))
        .collect();
    for key in credentials {
        table.remove(&key);
    }
    for key in [
        "model_provider",
        "model_providers",
        "model_catalog_json",
        "base_url",
        "env_key",
        "requires_openai_auth",
        "OPENAI_API_KEY",
        CHAT_UPSTREAM_BASE_URL_KEY,
    ] {
        table.remove(key);
    }
    if let Some(profiles) = table.get_mut("profiles").and_then(Item::as_table_like_mut) {
        for (_, profile) in profiles.iter_mut() {
            if let Some(profile) = profile.as_table_like_mut() {
                sanitize_official_profile_table(profile);
            }
        }
    }
}

pub(crate) fn apply_official_profile_to_home(
    home: &Path,
    profile: &RelayProfile,
    common_config: &str,
) -> anyhow::Result<RelayApplyResult> {
    normalize_relay_profile_for_storage(&mut profile.clone())?;
    let auth = (!profile.auth_contents.trim().is_empty()).then_some(profile.auth_contents.as_str());
    let has_explicit_model_settings = [
        &profile.model,
        &profile.model_list,
        &profile.model_windows,
        &profile.model_metadata,
        &profile.model_auto_compact,
        &profile.context_window,
        &profile.auto_compact_limit,
    ]
    .iter()
    .any(|value| !value.trim().is_empty());
    if profile.config_contents.trim().is_empty()
        && (!profile.use_common_config || common_config.trim().is_empty())
        && !has_explicit_model_settings
    {
        // 旧版本没有独立快照时沿用仅清路由的行为，不凭空清掉现有非认证设置。
        return clear_relay_config_to_home_with_auth(home, auth);
    }
    let snapshot = if profile.config_contents.trim().is_empty() {
        read_optional_text(&home.join("config.toml"))?
    } else {
        profile.config_contents.clone()
    };
    let mut config = sanitize_official_profile_config(&snapshot)?;
    if root_key_string(&config, "model").is_none()
        && let Some(model) = sanitize_relay_model_name(&profile.model)
    {
        let mut doc = parse_toml_document(&config)?;
        doc["model"] = toml_edit::value(model);
        config = normalize_optional_toml(doc);
    }
    let common = if profile.use_common_config {
        prepare_common_config_for_apply(common_config)?
    } else {
        String::new()
    };
    let config = merge_common_config_into_config(&config, &common)?;
    let config = preserve_unmanaged_live_context_entries(home, &config, common_config)?;
    let config = sanitize_official_profile_config(&config)?;
    let config = apply_context_limits_to_config(
        &config,
        &profile.context_window,
        &profile.auto_compact_limit,
    )?;
    with_model_catalog_rollback(home, profile, || {
        // 旧API目录指针已移除；仍按本官方profile明确声明的窗口/元数据重新生成。
        let config = apply_model_catalog_to_config_with_live_policy(home, profile, &config, false)?;
        let auth = official_profile_auth_for_switch(home, &profile.auth_contents)?;
        // 空配置是官方默认模式的合法状态，不能经过API入口的非空断言。
        let backup_path = write_codex_live_atomic(home, Some(&config), Some(auth.as_bytes()))?;
        let status = relay_config_status_from_home(home);
        Ok(RelayApplyResult {
            config_path: status.config_path,
            backup_path,
            configured: status.configured,
        })
    })
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
    // 合并前先给 profile 侧已有的 mcp_servers 条目留一份备份（#2147）。
    let profile_mcp_servers = target_doc
        .get("mcp_servers")
        .and_then(Item::as_table_like)
        .map(|servers| {
            servers
                .iter()
                .map(|(id, item)| (id.to_string(), item.clone()))
                .collect::<HashMap<_, _>>()
        })
        .unwrap_or_default();
    merge_toml_table_like(target_doc.as_table_mut(), source_doc.as_table());
    if let Some(enabled) = profile_goals_override {
        table_mut_or_insert(&mut target_doc, "features")?["goals"] = toml_edit::value(enabled);
    }
    // 合并后校验 MCP 条目完整性并做一次修复回填（#2147）。
    repair_merged_mcp_servers(&mut target_doc, &profile_mcp_servers);
    Ok(normalize_optional_toml(target_doc))
}

/// 一条 `[mcp_servers.<id>]` 是否是一份 codex 能接受的传输配置。
///
/// codex 的 `RawMcpServerConfig` 由 `command` / `url` 是否存在推断传输方式：
/// 两者都没有（例如只剩一个 `enabled_tools`，或被合并覆盖成空表）时它会报
/// `invalid transport`，并且**不是跳过这一条，而是拒载整份 config.toml**。
/// 用户表现为「编辑通用配置后 MCP 连接配置丢失」，实际是整份配置都没加载
/// （#2147 / #1997 / #2123 同源）。
fn mcp_server_entry_has_transport(table: &dyn TableLike) -> bool {
    ["command", "url"].iter().any(|key| {
        table
            .get(key)
            .and_then(Item::as_str)
            .is_some_and(|value| !value.trim().is_empty())
    })
}

/// 落盘前的 MCP 完整性校验（#2147）。
///
/// 通用配置与 profile 配置是两份独立 TOML，表合并本身按语义递归、正常不丢键；
/// 但通用配置里可能存在历史脚本/手改留下的残缺条目（例如只写 `enabled_tools`
/// 的 `[mcp_servers.codex_app]`），它会作为新表整条并入，让 config.toml 多出一条
/// 永远加载不起来的 MCP 条目。这里做两件事：
///
/// 1. **回填**：残缺条目若在 profile 侧本来有完整定义，就用备份把它补回来——
///    这是「合并把已有连接配置弄丢」的直接修复。补的方式是**只补缺键**，不是整条
///    替换：通用配置新写的键（例如 `.env` 子表）必须留下；
/// 2. **丢弃**：回填后仍无 `command` / `url` 的条目，唯一后果是让整份配置拒载，
///    因此直接移除并记诊断日志，把故障范围从「什么都用不了」收缩到「少一个本来
///    就无法工作的条目」。
fn repair_merged_mcp_servers(doc: &mut DocumentMut, profile_servers: &HashMap<String, Item>) {
    let Some(servers) = doc.get_mut("mcp_servers").and_then(Item::as_table_like_mut) else {
        return;
    };
    let ids: Vec<String> = servers.iter().map(|(id, _)| id.to_string()).collect();
    let mut dropped: Vec<String> = Vec::new();
    let mut restored: Vec<String> = Vec::new();
    for id in ids {
        let is_valid = servers
            .get(id.as_str())
            .and_then(Item::as_table_like)
            .is_some_and(mcp_server_entry_has_transport);
        if is_valid {
            continue;
        }
        // 残缺条目：优先用 profile 侧的同名完整定义补缺（不覆盖合并进来的新键）。
        if let Some(backup) = profile_servers.get(&id)
            && backup
                .as_table_like()
                .is_some_and(mcp_server_entry_has_transport)
            && let Some(existing) = servers.get_mut(id.as_str())
        {
            fill_missing_toml_item(existing, backup);
            if existing
                .as_table_like()
                .is_some_and(mcp_server_entry_has_transport)
            {
                restored.push(id);
                continue;
            }
        }
        servers.remove(id.as_str());
        dropped.push(id);
    }
    if servers.is_empty() {
        doc.as_table_mut().remove("mcp_servers");
    }
    if dropped.is_empty() && restored.is_empty() {
        return;
    }
    // 丢弃/回填是替用户改配置，留一条诊断记录方便排障时回溯。
    let _ = crate::diagnostic_log::append_diagnostic_log(
        "relay_config.mcp_server_entries_repaired",
        serde_json::json!({
            "restored": restored,
            "dropped": dropped,
            "reason": "mcp server entry lacked both `command` and `url`; codex rejects the whole config.toml with `invalid transport`",
        }),
    );
}

pub fn list_context_entries_from_common_config(
    common_config: &str,
) -> anyhow::Result<CodexContextEntries> {
    let normalized = normalize_duplicate_toml_text(common_config);
    let doc = parse_toml_document(&normalized)?;
    Ok(CodexContextEntries {
        mcp_servers: list_context_entries_for_table(&doc, "mcp_servers"),
        skills: list_context_entries_for_table(&doc, "skills"),
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
    // live config.toml 已被上游/人工改坏时，切换供应商不应整次失败（#2478/#2500）：
    // 无法从坏文件里捞用户托管条目，就直接按新配置写出；原文件已在 write_codex_live_atomic
    // 的 create_live_backup 中备份，可手工恢复。
    let Ok(live_doc) = parse_toml_document(&live_config) else {
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "relay_config.live_config_unparsable",
            serde_json::json!({ "home": home.to_string_lossy() }),
        );
        return Ok(ensure_trailing_newline(config_text.to_string()));
    };
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
    for table_name in ["mcp_servers", "skills", "plugins"] {
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
    for table_name in ["mcp_servers", "skills", "plugins"] {
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
    for table_name in ["mcp_servers", "skills", "plugins"] {
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
    for table_name in ["mcp_servers", "skills", "plugins"] {
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
            let config_text = preserve_live_app_settings(home, config_text)?;
            Some(preserve_live_marketplace_configs(home, &config_text)?)
        }
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

fn with_model_catalog_rollback<T>(
    home: &Path,
    profile: &RelayProfile,
    operation: impl FnOnce() -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let catalog_path = home
        .join("model-catalogs")
        .join(format!("{}.json", sanitize_catalog_filename(&profile.id)));
    let previous_catalog = read_optional_bytes(&catalog_path)?;
    match operation() {
        Ok(value) => Ok(value),
        Err(error) => {
            if let Err(restore_error) =
                restore_optional_file(&catalog_path, previous_catalog.as_deref())
            {
                return Err(
                    error.context(format!("应用失败后恢复模型 catalog 失败：{restore_error}"))
                );
            }
            Err(error)
        }
    }
}

fn preserve_live_marketplace_configs(home: &Path, config_text: &str) -> anyhow::Result<String> {
    let live_config = read_optional_text(&home.join("config.toml"))?;
    let mut target = parse_toml_document(config_text)?;
    remove_legacy_official_marketplace_overrides(home, &mut target);
    if live_config.trim().is_empty() {
        return Ok(ensure_trailing_newline(target.to_string()));
    }

    // live 文件损坏时不能让整次切换失败（#2478/#2500）：跳过补回，按新配置写出；
    // 原文件在 write_codex_live_atomic 里已备份。
    let Ok(live) = parse_toml_document(&normalize_duplicate_toml_text(&live_config)) else {
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "relay_config.live_config_unparsable",
            serde_json::json!({ "home": home.to_string_lossy(), "step": "marketplaces" }),
        );
        return Ok(ensure_trailing_newline(target.to_string()));
    };
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

    remove_legacy_official_marketplace_overrides(home, &mut target);
    Ok(ensure_trailing_newline(target.to_string()))
}

/// 旧版把官方目录注册成同名自定义市场，会使原生安装状态被保留名策略过滤。
/// 只清理由本工具历史使用的目录，保留插件启用项与用户自己的市场。
fn remove_legacy_official_marketplace_overrides(home: &Path, doc: &mut DocumentMut) -> bool {
    let Some(markets) = doc.get_mut("marketplaces").and_then(Item::as_table_like_mut) else {
        return false;
    };
    let legacy_root = home.join(".tmp").join("plugins");
    let legacy_root = legacy_root.to_string_lossy().replace('\\', "/");
    let legacy_root = legacy_root.trim_start_matches("//?/");
    let mut changed = false;
    for name in ["openai-curated", "openai-api-curated"] {
        let managed = markets
            .get(name)
            .and_then(Item::as_table_like)
            .is_some_and(|market| {
                let source = market.get("source").and_then(Item::as_str).unwrap_or_default();
                let source = source.replace('\\', "/");
                market.get("source_type").and_then(Item::as_str) == Some("local")
                    && source.trim_start_matches("//?/") == legacy_root
            });
        if managed {
            markets.remove(name);
            changed = true;
        }
    }
    changed
}

pub fn cleanup_legacy_official_marketplace_overrides_in_home(home: &Path) -> anyhow::Result<bool> {
    let config_path = home.join("config.toml");
    let text = read_optional_text(&config_path)?;
    let mut doc = parse_toml_document(&text)
        .map_err(|_| anyhow::anyhow!("Codex 配置格式错误，未修复历史插件市场注册"))?;
    if !remove_legacy_official_marketplace_overrides(home, &mut doc) {
        return Ok(false);
    }
    if read_optional_text(&config_path)? != text {
        anyhow::bail!("Codex 配置已被其他窗口更新，请刷新插件列表重试");
    }
    crate::settings::atomic_write(
        &config_path,
        ensure_trailing_newline(doc.to_string()).as_bytes(),
    )?;
    Ok(true)
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

/// 把可能含重复表头/重复根键的文本，折叠成一份合法、去重的 TOML。
///
/// 历史实现是逐行文本去重：按表头整行字符串匹配，撞见第二次出现的 `[header]`
/// 就把该表体整段丢弃。这在语义上是错的——`[mcp_servers.node_repl]`（带正确的
/// `.env` 子表）和其后一个裸的 `[mcp_servers]`（空表头，来自历史脚本/手改残留）
/// 是两个不同的表头字符串，行级去重完全看不到它们其实是同一棵 TOML 树上的父子
/// 关系；反过来，若两次出现的是同一个表头但各自只写了部分字段，行级去重会把后一
/// 份连同它独有的字段整段丢弃，而不是把两份合并。真实故障（Codex `config.toml`
/// 反复复现 `invalid transport`）就是这条路径把裸 env 变量和 `.env` 子表的残留
/// 一起原样保留进了最终文件。
///
/// 改成按顶层表头切块 + 用已有的 `merge_toml_table_like` 做语义合并：每块单独
/// 解析成 `DocumentMut`（块内容本身必须是合法 TOML），后出现的块合并进先出现的
/// 同名表（标量后写覆盖前写，子表递归合并、不清空），根键重复时也是后写覆盖。
/// 任何一块解析失败就说明输入本身已经坏到不是逐块可解析的程度，退回历史的逐行
/// 丢弃策略，不让这次修复反而让原本能跑的输入报错。
fn normalize_duplicate_toml_text(contents: &str) -> String {
    match merge_duplicate_toml_blocks(contents) {
        Some(merged) => normalize_optional_toml(merged),
        None => normalize_duplicate_toml_text_line_fallback(contents),
    }
}

/// 按不带前导空白的 `[table]` / `[[array_table]]` 行、以及根级重复键，切成若干
/// 独立可解析的 TOML 片段，再逐块 parse 后语义合并。
///
/// 只按表头切块不够：`model = "a"\nmodel = "b"\n` 两行根键都落在同一个块里，
/// 直接喂给 `DocumentMut::parse` 会因为 TOML 语法本身不允许根级重复键而整体
/// 解析失败。所以还要在「当前块的根级部分已经出现过这个 key」时，先切出一个
/// 新块，让后一次赋值单独成块参与合并（合并语义是后写覆盖前写）。
fn merge_duplicate_toml_blocks(contents: &str) -> Option<DocumentMut> {
    let mut blocks: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_in_root = true;
    let mut current_root_keys: HashSet<String> = HashSet::new();
    // 当前块的表头路径。父表头与它的真子表必须留在同一块里一起解析：子表
    // 如果单独成块，toml_edit 会把父级数组表隐式降级成普通表，合并阶段整棵
    // 数组表被覆盖，外层表头随之不再打印。
    let mut current_header_path: Option<String> = None;
    // 当前块内已声明过的标准表（非数组表）路径。同一个块里重复声明同一个
    // 标准表会让整块 TOML 解析失败，随后整体退回逐行去重、丢掉后出现的字段。
    let mut current_declared_headers: HashSet<String> = HashSet::new();

    for line in contents.lines() {
        let trimmed = line.trim();
        let is_new_table_header = trimmed.starts_with('[') && trimmed.ends_with(']');

        if is_new_table_header {
            let header_path = toml_header_path_of_line(trimmed);
            let is_strict_child = match (current_header_path.as_deref(), header_path) {
                (Some(parent), Some(child)) => is_strict_toml_child_path(parent, child),
                _ => false,
            };
            let is_array_table = trimmed.starts_with("[[");
            // 同一块里重复声明同一个标准表会让整块解析失败（真实故障：两个
            // [mcp_servers.node_repl] 各写一部分键），随后整体退回逐行去重，
            // 把后出现的那段字段整段丢掉。数组表允许重复声明，不参与判定。
            let is_repeated_standard_table = match header_path {
                Some(path) if !is_array_table => current_declared_headers.contains(path),
                _ => false,
            };

            if !is_strict_child || is_repeated_standard_table {
                if !current.trim().is_empty() {
                    blocks.push(std::mem::take(&mut current));
                }
                current_in_root = false;
                current_root_keys.clear();
                current_declared_headers.clear();
                current_header_path = header_path.map(str::to_string);
            }
            if let Some(path) = header_path {
                if !is_array_table {
                    current_declared_headers.insert(path.to_string());
                }
            }
        } else if current_in_root && !trimmed.is_empty() && !trimmed.starts_with('#') {
            if let Some((key, _)) = trimmed.split_once('=') {
                let key = key.trim().to_string();
                if !key.is_empty() && !current_root_keys.insert(key) {
                    // 同一个根键在当前块里再次出现：先把当前块切出去，
                    // 让这一行作为新块的第一行，参与合并时以「后写」身份覆盖前值。
                    if !current.trim().is_empty() {
                        blocks.push(std::mem::take(&mut current));
                    }
                    current_root_keys.clear();
                }
            }
        }

        current.push_str(line);
        current.push('\n');
    }
    if !current.trim().is_empty() {
        blocks.push(current);
    }

    let mut merged = DocumentMut::new();
    for block in blocks {
        if let Ok(block_doc) = block.parse::<DocumentMut>() {
            merge_toml_table_like(merged.as_table_mut(), block_doc.as_table());
            continue;
        }
        // 单块解析失败时只降级这一块（块内逐行去重后再试），而不是让整体退回
        // 全局逐行去重，避免一处坏块把其它块里已经解析好的字段一起丢掉。
        let repaired = dedupe_duplicate_headers_within_block(&block);
        let block_doc: DocumentMut = repaired.parse().ok()?;
        merge_toml_table_like(merged.as_table_mut(), block_doc.as_table());
    }
    Some(merged)
}

/// 单块级别的降级：在这一个块内部按整行表头字符串去重，保留首次出现的表头
/// 及其表体，丢弃重复表头与它后面的表体。只在整块解析失败时对该块使用，
/// 不改变其它块已经解析好的结果。
fn dedupe_duplicate_headers_within_block(block: &str) -> String {
    let mut seen_headers = HashSet::new();
    let mut kept = Vec::new();
    let mut skipping_duplicate_table = false;
    for line in block.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
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
        kept.push(line);
    }
    kept.join("\n")
}

/// 取出表头行括号里的路径；不是表头行时返回 None。
fn toml_header_path_of_line(trimmed: &str) -> Option<&str> {
    let inner = if let Some(rest) = trimmed.strip_prefix("[[") {
        rest.strip_suffix("]]")?
    } else if let Some(rest) = trimmed.strip_prefix('[') {
        rest.strip_suffix(']')?
    } else {
        return None;
    };
    let path = inner.trim();
    if path.is_empty() { None } else { Some(path) }
}

/// 子表路径判定必须按点分段多出一级，而不是字符串前缀。
fn is_strict_toml_child_path(parent: &str, child: &str) -> bool {
    match child.strip_prefix(parent) {
        Some(rest) => rest.strip_prefix('.').is_some_and(|tail| !tail.is_empty()),
        None => false,
    }
}

/// 逐块合并失败时的保底路径：原历史实现，逐行文本去重（丢弃后出现的重复表头/
/// 根键），保证至少不比修复前更差。
fn normalize_duplicate_toml_text_line_fallback(contents: &str) -> String {
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

/// 用 live 的键补齐 target 里缺的键，不覆盖 target 已有的值。
fn fill_missing_toml_item(target: &mut Item, source: &Item) {
    if let Some(source_table) = source.as_table_like() {
        if let Some(target_table) = target.as_table_like_mut() {
            for (key, source_item) in source_table.iter() {
                match target_table.get_mut(key) {
                    Some(target_item) => fill_missing_toml_item(target_item, source_item),
                    None => {
                        target_table.insert(key, source_item.clone());
                    }
                }
            }
        }
    }
}

/// mcp_servers 的形状权威在 live。模板里可能留着被压平的残形（启动键缺失、
/// env 键混进父表），这里在写盘前用 live 补齐缺键，并把与 env 子表重复的父表键移除。
fn repair_mcp_servers_from_live(target_doc: &mut DocumentMut, live_doc: &DocumentMut) {
    let Some(live_servers) = live_doc.get("mcp_servers").and_then(Item::as_table_like) else {
        return;
    };
    if target_doc
        .get("mcp_servers")
        .and_then(Item::as_table_like)
        .is_none()
    {
        target_doc["mcp_servers"] = toml_edit::table();
    }
    let Some(target_servers) = target_doc["mcp_servers"].as_table_like_mut() else {
        return;
    };
    for (id, live_item) in live_servers.iter() {
        match target_servers.get_mut(id) {
            Some(existing) => fill_missing_toml_item(existing, live_item),
            None => {
                target_servers.insert(id, live_item.clone());
            }
        }
    }
    let ids: Vec<String> = target_servers
        .iter()
        .map(|(id, _)| id.to_string())
        .collect();
    for id in ids {
        let Some(table) = target_servers
            .get_mut(id.as_str())
            .and_then(Item::as_table_like_mut)
        else {
            continue;
        };
        let env_keys: Vec<String> = table
            .get("env")
            .and_then(Item::as_table_like)
            .map(|env| env.iter().map(|(key, _)| key.to_string()).collect())
            .unwrap_or_default();
        for key in env_keys {
            if key != "env" {
                table.remove(key.as_str());
            }
        }
    }
}

/// 供集成测试直接验证 live 设置保留逻辑。
#[doc(hidden)]
pub fn preserve_live_app_settings_for_test(home: &Path, config_text: &str) -> anyhow::Result<String> {
    preserve_live_app_settings(home, config_text)
}

fn preserve_live_app_settings(home: &Path, config_text: &str) -> anyhow::Result<String> {    let normalized = normalize_config_text_for_write(config_text);
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
    // Windows 沙盒实现属于本机设置，切换模板时保留，避免重启后重新要求设置。
    for key in ["sandbox_mode", "approval_policy", "sandbox_workspace_write", "windows"] {
        if let Some(live_value) = live_doc.get(key).cloned() {
            merge_toml_item(&mut target_doc[key], &live_value);
        }
    }
    // MCP server 条目由用户/Codex 桌面端直接管理：模板与通用配置里已有的
    // 条目优先，live 里多出来的条目原样补回，避免每次重写后 server 逐个
    // 消失（#2263）。整体合并会覆盖通用配置的新值，所以只补缺。
    // 同一个 server 在模板里可能是被压平的残形，这时只补整条不够，
    // 还要用 live 补齐条目内缺失的键，并清掉混进父表的 env 键。
    repair_mcp_servers_from_live(&mut target_doc, &live_doc);
    // Preserve user-managed feature flags such as multi_agent_v2 and memories.
    preserve_missing_table_keys(&mut target_doc, &live_doc, "features");
    // 同上：`[plugins."<id>"]` 由用户与 Codex 桌面端管理，模板里没有时从 live 补回。
    // 这条是兜底——正常切换路径已经在 preserve_unmanaged_live_context_entries 补过，
    // 但 apply_relay_files_to_home / apply_relay_config_file_to_home 等入口不经过那一步，
    // 模板缺 plugins 段时会把用户的插件表整段丢掉（#890 / #597 / #609）。
    preserve_missing_table_keys(&mut target_doc, &live_doc, "plugins");
    // `[memories]`（记忆的提取/整合模型等）同样是用户在 Codex 里的本机设置，
    // 切换供应商或重启时不能被模板冲掉（#2021）。
    preserve_missing_table_keys(&mut target_doc, &live_doc, "memories");
    // hooks 的定义部分（除 state 外的键）同样由用户/桌面端管理，模板里没有时
    // 从 live 补回，否则切换供应商会把定义整段丢掉，只剩 hooks.state。
    preserve_live_hook_definitions(&mut target_doc, &live_doc);
    remove_unsupported_approval_policies(&mut target_doc);
    preserve_live_hook_state(&mut target_doc, &live_doc);
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

fn preserve_missing_table_keys(
    target_doc: &mut DocumentMut,
    live_doc: &DocumentMut,
    table_name: &str,
) {
    let Some(live_table) = live_doc.get(table_name).and_then(Item::as_table_like) else {
        return;
    };
    if target_doc.get(table_name).and_then(Item::as_table_like).is_none() {
        target_doc[table_name] = toml_edit::table();
    }
    let target_table = target_doc[table_name]
        .as_table_like_mut()
        .expect("table was initialized above");
    for (key, value) in live_table.iter() {
        if target_table.get(key).is_none() {
            target_table.insert(key, value.clone());
        }
    }
}

/// Normal-user launches cannot complete the elevated native Windows sandbox
/// setup. Downgrade only that case; an elevated process keeps the user's mode.
pub fn ensure_windows_sandbox_usable_for_current_user(home: &Path) -> anyhow::Result<bool> {
    ensure_windows_sandbox_usable_with(home, windows_process_is_elevated())
}

fn ensure_windows_sandbox_usable_with(home: &Path, process_elevated: bool) -> anyhow::Result<bool> {
    if process_elevated {
        return Ok(false);
    }
    // elevated 只在「首次初始化」时需要管理员权限；初始化完成后普通进程也能用。
    // 过去不看初始化状态一律降级，已经完成初始化的用户每次经 Codex++ 启动都被改回
    // unelevated，反而触发 `CreateProcessSecurityEnvironment failed`（#2615）。
    if windows_elevated_sandbox_setup_present(home) {
        return Ok(false);
    }
    let config_path = home.join("config.toml");
    let existing = match std::fs::read_to_string(&config_path) {
        Ok(existing) => existing,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    let mut doc = parse_toml_document(&existing)?;
    let Some(windows) = doc.get_mut("windows").and_then(Item::as_table_mut) else {
        return Ok(false);
    };
    let elevated = windows
        .get("sandbox")
        .and_then(Item::as_str)
        .is_some_and(|value| value.eq_ignore_ascii_case("elevated"));
    if !elevated {
        return Ok(false);
    }
    windows["sandbox"] = toml_edit::value("unelevated");
    crate::settings::atomic_write(&config_path, normalize_optional_toml(doc).as_bytes())?;
    // 这是替用户改写了他的配置，留一条诊断记录，方便排障时回溯。
    let _ = crate::diagnostic_log::append_diagnostic_log(
        "launcher.windows_sandbox_downgraded",
        serde_json::json!({
            "home": home.to_string_lossy(),
            "from": "elevated",
            "to": "unelevated",
            "reason": "current process is not elevated; native sandbox setup would report updateRequired",
        }),
    );
    Ok(true)
}

/// Codex 的 Windows 沙箱初始化产物是否齐全：`.sandbox/setup_marker.json` 与
/// `.sandbox-secrets/sandbox_users.json` 都存在、都带 `version` 且两者一致
/// （与 Codex `sandbox_setup_is_complete` 的判据一致）。不写死 Codex 的
/// SETUP_VERSION：版本升级后由 Codex 自己提示重新初始化，这里不替用户改配置。
fn windows_elevated_sandbox_setup_present(home: &Path) -> bool {
    let version = |path: PathBuf| -> Option<u64> {
        let text = std::fs::read_to_string(path).ok()?;
        serde_json::from_str::<Value>(&text).ok()?.get("version")?.as_u64()
    };
    match (
        version(home.join(".sandbox").join("setup_marker.json")),
        version(home.join(".sandbox-secrets").join("sandbox_users.json")),
    ) {
        (Some(marker), Some(users)) => marker == users,
        _ => false,
    }
}

#[cfg(windows)]
fn windows_process_is_elevated() -> bool {
    use windows::Win32::UI::Shell::IsUserAnAdmin;

    unsafe { IsUserAnAdmin().as_bool() }
}

#[cfg(not(windows))]
fn windows_process_is_elevated() -> bool {
    true
}

/// hooks 的定义部分（UserPromptSubmit / Stop / SessionEnd 等数组表）属于
/// 用户/桌面端管理的本机设置。模板里没有时从 live 补回，只补缺、不覆盖，
/// 避免切换供应商后 hooks 定义整段消失、只剩 hooks.state。
fn preserve_live_hook_definitions(target_doc: &mut DocumentMut, live_doc: &DocumentMut) {
    let Some(live_hooks) = live_doc.get("hooks").and_then(Item::as_table_like) else {
        return;
    };
    if target_doc.get("hooks").and_then(Item::as_table_like).is_none() {
        target_doc["hooks"] = toml_edit::table();
    }
    let Some(target_hooks) = target_doc["hooks"].as_table_like_mut() else {
        return;
    };
    for (key, value) in live_hooks.iter() {
        if key == "state" {
            continue;
        }
        // 只搬运定义类子表（hooks.X 形式），不搬运 live 独有的标量键，
        // 否则会把本机个性化设置带进 profile 模板。
        if value.as_table_like().is_none() {
            continue;
        }
        if target_hooks.get(key).is_none() {
            target_hooks.insert(key, value.clone());
        }
    }
}

fn preserve_live_hook_state(target_doc: &mut DocumentMut, live_doc: &DocumentMut) {
    let live_state = live_doc
        .get("hooks")
        .and_then(Item::as_table_like)
        .and_then(|hooks| hooks.get("state"))
        .cloned();

    if let Some(live_state) = live_state {
        if target_doc
            .get("hooks")
            .and_then(Item::as_table_like)
            .is_none()
        {
            target_doc["hooks"] = toml_edit::table();
        }
        let hooks = target_doc["hooks"]
            .as_table_like_mut()
            .expect("hooks was initialized as a table");
        hooks.insert("state", live_state);
    } else if let Some(hooks) = target_doc
        .get_mut("hooks")
        .and_then(Item::as_table_like_mut)
    {
        hooks.remove("state");
    }

    let remove_empty_hooks = target_doc
        .get("hooks")
        .and_then(Item::as_table_like)
        .is_some_and(|hooks| hooks.is_empty());
    if remove_empty_hooks {
        target_doc.as_table_mut().remove("hooks");
    }
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
    // 用户常从文档或别处复制带千分位的数字（`1,050,000`）。此前会直接
    // `InvalidDigit` 判整次保存失败，而语义上它就是个合法的 1050000，
    // 静默接受比让整份配置保存不了更合理（issue #2590）。
    let digits: String = trimmed
        .chars()
        .filter(|character| !matches!(character, ',' | '_' | '\u{FF0C}' | '\u{3000}'))
        .collect();
    let digits = digits.trim();
    let parsed = digits
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

/// 外部 `model_catalog_json` 与每模型配置冲突时的降级路径（issue #2203）。
///
/// 保留用户手写的外部指针不动，只往 config.toml 顶层写 `model_context_window` /
/// `model_auto_compact_token_limit` 作兜底。顶层键对全部模型生效，是「每模型窗口
/// 用不上」时的最优可用近似；而过去直接 bail 会让用户**完全切不了供应商**，
/// 代价远大于特性降级。
fn apply_external_catalog_fallback(
    config_text: &str,
    context_window: Option<u64>,
    auto_compact_limit: Option<u64>,
) -> String {
    let Ok(mut doc) = parse_toml_document(config_text) else {
        return config_text.to_string();
    };
    if let Some(value) = context_window {
        doc["model_context_window"] = toml_edit::value(value as i64);
    }
    if let Some(value) = auto_compact_limit {
        doc["model_auto_compact_token_limit"] = toml_edit::value(value as i64);
    }
    normalize_optional_toml(doc)
}

fn apply_model_catalog_to_config(
    home: &Path,
    profile: &RelayProfile,
    config_text: &str,
) -> anyhow::Result<String> {
    apply_model_catalog_to_config_with_live_policy(home, profile, config_text, true)
}

fn wsl_catalog_paths_enabled(home: &Path, config: &str) -> bool {
    let read = |text: &str| -> Option<bool> {
        let doc = parse_toml_document(text).ok()?;
        doc.get("desktop")?.get("runCodexInWindowsSubsystemForLinux")?.as_bool()
    };
    // 与最终写盘的 preserve_live_app_settings 保持相同优先级：桌面运行环境
    // 属于本机设置，不能让供应商快照里过期的 true/false 改变目录解析方式。
    read_optional_text(&home.join("config.toml")).ok().and_then(|text| read(&text))
        .or_else(|| read(config))
        .unwrap_or(false)
}

pub(crate) fn managed_catalog_pointer(home: &Path, relative: &str, config: &str) -> anyhow::Result<String> {
    if wsl_catalog_paths_enabled(home, config) {
        // WSL 的 app-server 不能读取 C:\\...；相对路径由共享 CODEX_HOME 解析。
        return Ok(relative.replace('\\', "/"));
    }
    Ok(std::path::absolute(home.join(relative))
        .context("解析受管模型 catalog 绝对路径失败")?.to_string_lossy().to_string())
}

/// 只迁移当前 home 内的受管目录，不重放供应商配置或改写用户外部目录。
/// 原生 Windows 需要绝对路径（#2529），WSL 则保留共享 home 下的相对路径（#2591）。
pub fn migrate_managed_model_catalog_pointer_in_home(
    home: &Path,
    settings: &BackendSettings,
) -> anyhow::Result<bool> {
    if !settings.relay_profiles_enabled {
        return Ok(false);
    }
    let config_path = home.join("config.toml");
    let config_metadata = match std::fs::symlink_metadata(&config_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error).context("读取 catalog 指针配置状态失败"),
    };
    // 原子写回会替换链接本身，用户自行链接的配置不属于这次窄迁移。
    if config_metadata.file_type().is_symlink() {
        return Ok(false);
    }
    let contents = std::fs::read_to_string(&config_path).context("读取 catalog 指针配置失败")?;
    // 诊断不附带 TOML 原文，避免把用户配置中的凭据写进启动日志。
    let mut doc = parse_toml_document(&contents)
        .map_err(|_| anyhow::anyhow!("实时 config.toml 语法无效，未迁移模型 catalog 指针"))?;
    let Some(raw_path) = doc.get("model_catalog_json").and_then(Item::as_str) else {
        return Ok(false);
    };
    let path = Path::new(raw_path);
    let absolute_home = std::path::absolute(home)?;
    let relative = if path.is_absolute() {
        let Ok(relative) = path.strip_prefix(&absolute_home) else { return Ok(false); };
        relative
    } else { path };
    // 旧产物只有 model-catalogs/<id>.json 两层。拒绝 ..、嵌套目录与其它前缀，
    // 不使用 apply 时用于识别旧管理器目录的宽松启发式。
    let mut components = relative
        .components()
        .filter(|component| *component != std::path::Component::CurDir);
    let Some(std::path::Component::Normal(root)) = components.next() else {
        return Ok(false);
    };
    let Some(std::path::Component::Normal(filename)) = components.next() else {
        return Ok(false);
    };
    if root != "model-catalogs"
        || Path::new(filename).extension() != Some(std::ffi::OsStr::new("json"))
        || components.next().is_some()
    {
        return Ok(false);
    }
    let catalog_dir = home.join(root);
    let catalog_path = catalog_dir.join(filename);
    // 软链或目录联接可能把看似受管的路径指到用户外部目录；保守地保持原状。
    for (candidate, expect_directory) in [(&catalog_dir, true), (&catalog_path, false)] {
        let metadata = match std::fs::symlink_metadata(candidate) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error).context("检查受管模型 catalog 路径失败"),
        };
        if metadata.file_type().is_symlink()
            || (expect_directory && !metadata.is_dir())
            || (!expect_directory && !metadata.is_file())
        {
            return Ok(false);
        }
    }
    let pointer = managed_catalog_pointer(home, &relative.to_string_lossy(), &contents)?;
    if raw_path == pointer { return Ok(false); }
    doc["model_catalog_json"] = toml_edit::value(pointer);
    crate::settings::atomic_write(&config_path, doc.to_string().as_bytes())
        .context("迁移模型 catalog 绝对路径失败")?;
    Ok(true)
}

fn apply_model_catalog_to_config_with_live_policy(
    home: &Path,
    profile: &RelayProfile,
    config_text: &str,
    allow_live_external_catalog: bool,
) -> anyhow::Result<String> {
    let catalog_relative = format!(
        "model-catalogs/{}.json",
        sanitize_catalog_filename(&profile.id)
    );
    // 不用 canonicalize：它在 Windows 会引入 \\?\ 前缀；absolute 也能处理相对
    // CODEX_HOME，同时保持受管文件原来的位置和符号链接语义。
    let catalog_pointer = managed_catalog_pointer(home, &catalog_relative, config_text)?;
    let mut config_text = config_text.to_string();
    if let Some(catalog_json) = crate::aggregate_catalog::build_catalog_for_profile(profile)? {
        let catalog_path = home.join(&catalog_relative);
        if let Some(parent) = catalog_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        validate_managed_model_catalog(&catalog_json)?;
    crate::settings::atomic_write(&catalog_path, catalog_json.as_bytes())?;
        let mut doc = parse_toml_document(&config_text)?;
        if root_key_string(&config_text, "model").is_none()
            && let Ok(catalog) = serde_json::from_str::<Value>(&catalog_json)
            && let Some(default_model) = catalog
                .get("models")
                .and_then(Value::as_array)
                .and_then(|models| models.first())
                .and_then(|model| model.get("slug"))
                .and_then(Value::as_str)
        {
            doc["model"] = toml_edit::value(default_model);
        }
        doc["model_catalog_json"] = toml_edit::value(catalog_pointer);
        return Ok(normalize_optional_toml(doc));
    }
    let mut model_windows = parse_model_string_map(&profile.model_windows, "model_windows")?;
    validate_model_windows(&model_windows)?;
    let model_list = if profile.model_list.contains('[') {
        let (clean_list, migrated_windows) =
            crate::model_suffix::migrate_model_list_with_suffixes(&profile.model_list);
        for (slug, window) in migrated_windows {
            model_windows.entry(slug).or_insert(window);
        }
        clean_list
    } else {
        profile.model_list.clone()
    };
    let model_auto_compact =
        parse_model_string_map(&profile.model_auto_compact, "model_auto_compact")?;
    validate_model_auto_compact(&model_auto_compact)?;
    let model_metadata = parse_model_metadata_map(&profile.model_metadata)?;
    let entries = crate::model_suffix::collect_catalog_entries(
        &model_list,
        &model_windows,
        &model_auto_compact,
        &profile.model,
    );
    let entry_slugs: HashSet<String> = entries.iter().map(|entry| entry.slug.clone()).collect();
    let has_metadata_overrides = model_metadata_has_entries(&model_metadata, &entry_slugs);
    let has_per_model_overrides = has_metadata_overrides
        || entries
            .iter()
            .any(|entry| entry.suffix_window.is_some() || entry.auto_compact_percent.is_some());
    // Codex 侧 wire_api 恒为 Responses，真实上游协议仍以 profile 为准。
    // API 传输的外部目录副本和模型路由不能随会话身份改变。
    let managed_api_mode = matches!(
        profile.relay_mode,
        crate::settings::RelayMode::PureApi | crate::settings::RelayMode::MixedApi
    ) || (profile.relay_mode == crate::settings::RelayMode::Official
        && profile.official_mix_api_key);
    // 保留仅配置 custom provider、未同步模式字段的旧调用方及既有聚合策略。
    let standard_responses = profile.protocol == RelayProtocol::Responses
        && (managed_api_mode
            || active_provider_id(&parse_toml_document(&config_text)?)
                .is_some_and(|provider_id| is_custom_provider_id(&provider_id)));
    // Catalog capabilities must follow the effective config, not stale profile URLs.
    let official_deepseek_responses =
        uses_official_deepseek_responses_for_config(profile, &config_text);
    // Lite 线格式只有 ChatGPT 产品后端支持；官方登录态才可能走该通道，
    // 其余上游（公开 API / 中转）一律标准 Responses。判定基于生成后的
    // 有效 config 而非 profile 陈旧 URL，与 deepseek 判定同一模式。
    let official_login = profile.relay_mode == RelayMode::Official;
    let lite_supported = upstream_supports_responses_lite(
        effective_provider_base_url(&config_text).as_deref(),
        official_login,
    );
    let fallback = parse_optional_positive_u64(&profile.context_window, "上下文大小")?;
    // 外部 catalog 降级时的顶层兜底键（issue #2203）。顶层键对全部模型生效，
    // 是「每模型窗口用不上」时的最优可用近似。
    let auto_compact_fallback =
        parse_optional_positive_u64(&profile.auto_compact_limit, "压缩上下文大小")?;
    // 用户已手写 model_catalog_json 指针时保留，不覆盖（保 preserves_user_model_catalog_json 测试）。
    // Codex++ 管理的 catalog 必须随当前 profile 切换；否则前一个供应商的模型列表会残留。
    // 该固定文件名属于已知的其他管理器投影，不视为用户手写 catalog；
    // 切换到 Codex++ profile 时应接管，否则旧 catalog 会继续覆盖本 profile 的模型元数据。
    if let Some(existing) = root_key_string(&config_text, "model_catalog_json") {
        if existing != catalog_relative && existing != catalog_pointer {
            if is_codex_plus_managed_model_catalog(home, &existing)
                || is_cc_switch_model_catalog(&existing)
            {
                config_text = remove_root_key(&config_text, "model_catalog_json");
            } else if model_catalog_pointer_has_unexpanded_variable(&existing)
                || model_catalog_pointer_file_missing(home, &existing)
            {
                // `%userprofile%\.codex\codex-models.json` 这类指针 codex 不展开变量，
                // 在任何机器上都读不到；「变量已展开但文件其实不存在」的形态（用户
                // 删了 catalog、换机器后路径失效、Windows 上写死别的用户目录）后果
                // 完全相同——留着会让 codex 拒绝加载整份 config.toml（#2123）。
                // 去掉它不会比现在更差——这份 catalog 反正从未生效过。后续流程会照
                // 常生成本 profile 的托管 catalog，每模型窗口得以生效，而不是整份
                // 配置拒载。
                config_text = remove_root_key(&config_text, "model_catalog_json");
            } else {
                if has_per_model_overrides {
                    // 用户手写了外部 catalog 且本 profile 配了每模型窗口/元数据。
                    // 过去直接 bail，代价是用户**完全切不了供应商**——远比「特性降级」严重。
                    // 改为保留外部指针、写顶层兜底键，并让调用方带 warning（issue #2203）。
                    return Ok(apply_external_catalog_fallback(
                        &config_text,
                        fallback,
                        auto_compact_fallback,
                    ));
                }
                if official_deepseek_responses {
                    return Ok(config_text.to_string());
                }
                if standard_responses
                    && copy_standard_responses_catalog(
                        home,
                        &existing,
                        &catalog_relative,
                        &entries,
                        fallback,
                        lite_supported || profile.trust_upstream_catalog_lite,
                    )?
                {
                    let mut doc = parse_toml_document(&config_text)?;
                    doc["model_catalog_json"] = toml_edit::value(catalog_pointer);
                    return Ok(normalize_optional_toml(doc));
                }
                return Ok(config_text);
            }
        }
    }
    if allow_live_external_catalog && !official_deepseek_responses
        && let Some(external_catalog) = live_external_model_catalog(home)
    {
        if has_per_model_overrides {
            // 同上一处：live 里已有外部指针且本 profile 配了每模型覆盖时，
            // 降级为「保留外部指针 + 顶层兜底键」，不再拒绝整次切换（issue #2203）。
            return Ok(apply_external_catalog_fallback(
                &config_text,
                fallback,
                auto_compact_fallback,
            ));
        }
        let mut doc = parse_toml_document(&config_text)?;
        if standard_responses
            && copy_standard_responses_catalog(
                home,
                &external_catalog,
                &catalog_relative,
                &entries,
                fallback,
                lite_supported || profile.trust_upstream_catalog_lite,
            )?
        {
            doc["model_catalog_json"] = toml_edit::value(catalog_pointer);
        } else {
            doc["model_catalog_json"] = toml_edit::value(external_catalog);
        }
        return Ok(normalize_optional_toml(doc));
    }
    // Known bundled metadata entries need a catalog even without a user-supplied window.
    // 托管 Responses 传输走 model_routes 时需要 catalog，与会话身份无关；
    // 纯平铺 model_list 且无窗口/元数据的仍保持"不生成"契约（无后缀不落盘，见既有测试）。
    // 上游不认 tool_search 时必须落 catalog 关掉该能力，否则 Codex 照发、整单 400。
    let catalog_path = home.join(&catalog_relative);
    // 已经存在的托管 catalog 必须随当前模型列表重建：否则用户在供应商渠道里
    // 增删/重命名模型后，Codex 重启读到的仍是旧 catalog，选择器不刷新
    // （#2081）。「从没生成过 catalog 的平铺 model_list」保持不凭空创建。
    // 仅限仍有多模型条目时刷新：模型列表收敛回单个无窗口模型时，平铺列表本身
    // 就够用，继续落 catalog 会架空「关闭每模型覆盖即删除指针」的既有契约
    // （见 tests/model_catalog_paths.rs）。
    // 只数模型列表本身：entries 还会把当前 profile.model 放在最前面，
    // 「列表 a + 默认模型 b」不能算成多模型。
    let listed_models = model_list
        .split(['\r', '\n', ','])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect::<HashSet<_>>()
        .len();
    let existing_managed_catalog = catalog_path.exists()
        && listed_models > 1
        && root_key_string(&config_text, "model_catalog_json").is_some_and(|pointer| {
            pointer == catalog_relative || pointer == catalog_pointer
        });
    if !should_write_managed_model_catalog(
        &entries,
        has_metadata_overrides,
        standard_responses,
        profile.has_model_routes(),
        official_deepseek_responses,
        // 只有用户显式写了模型列表时，供应商级窗口才落 catalog：模型列表留空
        // （典型是官方登录）时 catalog 里只剩 0~1 个模型，会替换 Codex 内置的
        // 官方模型列表，选择器变空；这种情况沿用顶层 model_context_window。
        fallback.is_some() && !model_list.trim().is_empty(),
    ) && !upstream_rejects_tool_search(profile)
        && !existing_managed_catalog
    {
        let mut doc = parse_toml_document(&config_text)?;
        if root_key_string(&config_text, "model_catalog_json")
            .is_some_and(|pointer| pointer == catalog_relative || pointer == catalog_pointer)
        {
            doc.remove("model_catalog_json");
        }
        return Ok(normalize_optional_toml(doc));
    }
    if let Some(parent) = catalog_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Lite 线格式跟随上游能力：仅官方登录态 + ChatGPT 后端保留模板原值，
    // 公开 API / 第三方中转一律关闭（标准 Responses），不再以 provider id 近似判断。
    let catalog_json = crate::model_suffix::build_model_catalog_json_with_capabilities(
        &entries,
        fallback,
        None,
        (!lite_supported).then_some(false),
        official_deepseek_responses,
    );
    let catalog_json = apply_model_metadata_overrides(&catalog_json, &model_metadata)?;
    let catalog_json = if upstream_rejects_tool_search(profile) {
        disable_search_tool_in_catalog(&catalog_json)?
    } else {
        catalog_json
    };
    validate_managed_model_catalog(&catalog_json)?;
    crate::settings::atomic_write(&catalog_path, catalog_json.as_bytes())?;
    let mut doc = parse_toml_document(&config_text)?;
    doc["model_catalog_json"] = toml_edit::value(catalog_pointer);
    Ok(normalize_optional_toml(doc))
}

/// 自定义请求头条数上限：防止设置文件被撑爆以及表单渲染卡顿。
const MAX_RELAY_CUSTOM_HEADERS: usize = 64;

/// 保存前规范化自定义请求头：去掉空行、压缩首尾空白、并做结构校验。
///
/// 校验规则统一在 `relay_headers` 里，测试连接 / 模型列表 / 协议代理三处共用。
fn normalize_relay_headers(
    headers: &mut Vec<crate::settings::RelayHeaderKeyValue>,
) -> anyhow::Result<()> {
    *headers = crate::relay_headers::normalized(headers);
    if headers.len() > MAX_RELAY_CUSTOM_HEADERS {
        anyhow::bail!("自定义请求头最多 {MAX_RELAY_CUSTOM_HEADERS} 条");
    }
    crate::relay_headers::validate(headers)
}

fn parse_model_string_map(
    value: &str,
    field_name: &str,
) -> anyhow::Result<HashMap<String, String>> {
    if value.trim().is_empty() {
        return Ok(HashMap::new());
    }
    let parsed: Value = serde_json::from_str(value)
        .map_err(|error| anyhow::anyhow!("{field_name} JSON 解析失败：{error}"))?;
    let object = parsed
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("{field_name} 必须是 JSON 对象"))?;
    object
        .iter()
        .map(|(key, value)| {
            let value = value
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("{field_name} 的模型 {key} 值必须是字符串"))?;
            Ok((key.clone(), value.to_string()))
        })
        .collect()
}

fn validate_model_windows(model_windows: &HashMap<String, String>) -> anyhow::Result<()> {
    for (slug, value) in model_windows {
        if crate::model_suffix::parse_window_token(value).is_none() {
            anyhow::bail!("model_windows 的模型 {slug} 窗口值无效：{value}");
        }
    }
    Ok(())
}

fn validate_model_auto_compact(model_auto_compact: &HashMap<String, String>) -> anyhow::Result<()> {
    for (slug, value) in model_auto_compact {
        if crate::model_suffix::parse_compact_percent(value).is_none() {
            anyhow::bail!("model_auto_compact 的模型 {slug} 百分比无效：{value}");
        }
    }
    Ok(())
}

pub(crate) fn parse_model_metadata_map(metadata_json: &str) -> anyhow::Result<serde_json::Map<String, Value>> {
    if metadata_json.trim().is_empty() {
        return Ok(serde_json::Map::new());
    }
    let value: Value = serde_json::from_str(metadata_json)
        .map_err(|error| anyhow::anyhow!("model_metadata JSON 解析失败：{error}"))?;
    let map = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("model_metadata 必须是 JSON 对象"))?;
    // 与前端 canonicalizeModelMetadataMap 完全对齐：key 统一剥 [1M] 后缀并做
    // ASCII 小写归一，同名变体（大小写/后缀差异）后写者胜。否则旧数据里
    // "DeepSeek-V4-Pro" 与 "deepseek-v4-pro[512K]" 会在前端显示与 Rust 生成
    // catalog 两侧各选一份覆盖（前端末位胜、这里首位胜），界面与生效值分叉。
    let mut canonical = serde_json::Map::new();
    for (slug, metadata) in map {
        if !metadata.is_object() {
            anyhow::bail!("model_metadata 的模型 {slug} 值必须是对象");
        }
        validate_catalog_model_fields(slug, metadata, false)?;
        let normalized = crate::model_suffix::parse_model_suffix(slug).0;
        if normalized.is_empty() {
            continue;
        }
        canonical.insert(normalized.to_ascii_lowercase(), metadata.clone());
    }
    Ok(canonical)
}

fn model_metadata_has_entries(
    metadata: &serde_json::Map<String, Value>,
    entry_slugs: &HashSet<String>,
) -> bool {
    // metadata 的 key 已由 parse_model_metadata_map 归一化，entry slug 侧做
    // 同样归一后直接求交。
    let normalized_entry_slugs = entry_slugs
        .iter()
        .map(|slug| {
            crate::model_suffix::parse_model_suffix(slug)
                .0
                .to_ascii_lowercase()
        })
        .collect::<HashSet<_>>();
    metadata.keys().any(|slug| normalized_entry_slugs.contains(slug))
}

fn model_metadata_override_for_slug<'a>(
    override_map: &'a serde_json::Map<String, Value>,
    slug: &str,
) -> Option<&'a serde_json::Map<String, Value>> {
    // override_map 的 key 已由 parse_model_metadata_map 归一化（剥后缀 +
    // 小写、变体去重），查询 slug 做同样归一后直接命中。
    let normalized_slug = crate::model_suffix::parse_model_suffix(slug)
        .0
        .to_ascii_lowercase();
    override_map
        .get(&normalized_slug)
        .and_then(Value::as_object)
}

/// 判断当前 profile 是否需要由 CodexPlusPlus 托管写入 model catalog。
///
/// 该谓词只依赖已解析的 catalog 条目和 profile 能力事实，不读取磁盘，也不
/// 改变外部 catalog 的接管、冲突回滚及 per-profile 单值行为。
fn should_write_managed_model_catalog(
    entries: &[crate::model_suffix::ModelCatalogEntry],
    metadata_overrides: bool,
    standard_responses: bool,
    has_model_routes: bool,
    official_deepseek_responses: bool,
    has_profile_window: bool,
) -> bool {
    // profile 级窗口（`context_window`）也必须落 catalog：只配全局窗口、模型列表
    // 不带后缀时，过去不会生成 catalog，Codex 继续用内置默认窗口，用户看到的仍是
    // 272k（issue #2590）。有全局窗口时把每模型条目写成该窗口即可。
    metadata_overrides
        || has_profile_window
        || entries.iter().any(|entry| {
            entry.suffix_window.is_some()
                || entry.auto_compact_percent.is_some()
                || crate::model_suffix::requires_bundled_metadata_catalog(&entry.slug)
                || (official_deepseek_responses && entry.slug.starts_with("deepseek-v4-"))
        })
        || (standard_responses && has_model_routes)
}

#[cfg(test)]
mod managed_catalog_predicate_tests {
    use super::{
        apply_model_metadata_overrides, model_metadata_has_entries,
        model_metadata_override_for_slug, parse_model_metadata_map,
        should_write_managed_model_catalog,
    };
    use crate::model_suffix::ModelCatalogEntry;
    use serde_json::{Value, json};
    use std::collections::HashSet;

    #[test]
    fn fast_metadata_modes_preserve_other_tiers_and_never_leak_the_private_marker() {
        #[derive(serde::Deserialize)]
        struct NativeModelServiceTier { id: String, name: String, description: String }
        let base = json!({"models":[{"slug":"grok-custom", "context_window":128000,
            "service_tiers":[{"id":"flex", "name":"Flex", "description":"Existing tier"}],
            "additional_speed_tiers":["slow"]}]}).to_string();
        for mode in ["supported", "unsupported", "inherit"] {
            let overrides = parse_model_metadata_map(&json!({"grok-custom":{"codex_plus_fast_support":mode}}).to_string()).unwrap();
            let generated: Value = serde_json::from_str(&apply_model_metadata_overrides(&base, &overrides).unwrap()).unwrap();
            let model = &generated["models"][0];
            let tiers: Vec<NativeModelServiceTier> = serde_json::from_value(model["service_tiers"].clone()).unwrap();
            assert_eq!(tiers[0].id, "flex");
            assert_eq!(tiers[0].name, "Flex");
            assert!(!tiers[0].description.is_empty());
            assert!(model.get("codex_plus_fast_support").is_none());
            assert_eq!(model["service_tiers"][0]["id"], "flex");
            assert_eq!(model["additional_speed_tiers"][0], "slow");
            let priority = model["service_tiers"].as_array().unwrap().iter().find(|tier| tier["id"] == "priority");
            assert_eq!(priority.is_some(), mode == "supported");
            if let Some(priority) = priority {
                for field in ["id", "name", "description"] { assert!(priority[field].is_string()); }
            }
        }
    }

    fn entry(slug: &str) -> ModelCatalogEntry {
        ModelCatalogEntry {
            slug: slug.to_string(),
            display_name: slug.to_string(),
            suffix_window: None,
            auto_compact_percent: None,
        }
    }

    #[test]
    fn predicate_keeps_plain_profile_without_routes_unmanaged() {
        assert!(!should_write_managed_model_catalog(
            &[entry("custom-model")],
            false,
            false,
            false,
            false,
            false,
        ));
    }

    #[test]
    fn predicate_writes_for_profile_level_window_without_suffixes() {
        // #2590：只配 profile 级 context_window、模型列表不带 [1M] 后缀时，
        // 也必须生成 catalog，否则 Codex 继续按内置 272k 窗口压缩。
        assert!(should_write_managed_model_catalog(
            &[entry("custom-model")],
            false,
            false,
            false,
            false,
            true,
        ));
    }

    #[test]
    fn predicate_writes_for_per_model_window_compact_or_metadata() {
        let mut window = entry("custom-model");
        window.suffix_window = Some(1_000_000);
        assert!(should_write_managed_model_catalog(
            &[window], false, false, false, false, false
        ));

        let mut compact = entry("custom-model");
        compact.auto_compact_percent = Some(80_000_000);
        assert!(should_write_managed_model_catalog(
            &[compact], false, false, false, false, false
        ));
        assert!(should_write_managed_model_catalog(
            &[entry("custom-model")],
            true,
            false,
            false,
            false,
            false,
        ));
    }

    #[test]
    fn predicate_writes_for_routes_or_official_deepseek() {
        assert!(should_write_managed_model_catalog(
            &[entry("custom-model")],
            false,
            true,
            true,
            false,
            false,
        ));
        assert!(should_write_managed_model_catalog(
            &[entry("deepseek-v4-pro")],
            false,
            false,
            false,
            true,
            false,
        ));
    }

    #[test]
    fn metadata_keys_match_catalog_slugs_case_insensitively_and_without_suffix() {
        // 生产链路里 has_entries 的入参恒来自 parse_model_metadata_map（key
        // 已归一），这里走同一管道构造。
        let metadata = parse_model_metadata_map(r#"{"DeepSeek-V4-Pro[1M]": {"description": "custom"}}"#).unwrap();
        let slugs = HashSet::from(["deepseek-v4-pro".to_string()]);
        assert!(model_metadata_has_entries(&metadata, &slugs));
    }

    #[test]
    fn metadata_map_normalizes_keys_with_last_write_wins() {
        // 同名变体（大小写/后缀）归一到同一 key，后写者胜——与前端
        // canonicalizeModelMetadataMap 一致，否则两侧各选一份覆盖。
        let metadata = parse_model_metadata_map(
            r#"{
                "DeepSeek-V4-Pro": {"description": "first"},
                "deepseek-v4-pro[512K]": {"description": "second"}
            }"#,
        )
        .unwrap();
        assert_eq!(metadata.len(), 1);
        assert_eq!(metadata["deepseek-v4-pro"]["description"], "second");

        // 覆盖查找同样按归一 key 命中：catalog 侧的 slug 带大小写变体也能查到。
        let override_map =
            parse_model_metadata_map(r#"{"GLM-5.3": {"display_name": "GLM"}}"#).unwrap();
        assert_eq!(
            model_metadata_override_for_slug(&override_map, "glm-5.3")
                .unwrap()["display_name"],
            "GLM"
        );
        assert_eq!(
            model_metadata_override_for_slug(&override_map, "GLM-5.3[1M]")
                .unwrap()["display_name"],
            "GLM"
        );
        assert!(model_metadata_override_for_slug(&override_map, "other").is_none());
    }

    #[test]
    fn metadata_overlay_uses_canonical_slug_and_preserves_managed_fields() {
        let metadata = parse_model_metadata_map(
            r#"{"DeepSeek-V4-Pro[1M]": {
                "description": "custom",
                "context_window": 1,
                "max_context_window": 1
            }}"#,
        )
        .unwrap();
        let catalog = json!({"models": [{
            "slug": "deepseek-v4-pro",
            "context_window": 272000,
            "max_context_window": 272000
        }]});
        let result = apply_model_metadata_overrides(&catalog.to_string(), &metadata).unwrap();
        let value: Value = serde_json::from_str(&result).unwrap();
        let model = &value["models"][0];
        assert_eq!(model["description"], "custom");
        assert_eq!(model["context_window"], 272000);
        assert_eq!(model["max_context_window"], 272000);
    }
}

pub(crate) fn apply_model_metadata_overrides(
    catalog_json: &str,
    override_map: &serde_json::Map<String, Value>,
) -> anyhow::Result<String> {
    if override_map.is_empty() {
        return Ok(catalog_json.to_string());
    }
    let mut catalog: Value = serde_json::from_str(catalog_json)
        .map_err(|error| anyhow::anyhow!("catalog JSON 解析失败：{error}"))?;
    let Some(models) = catalog.get_mut("models").and_then(Value::as_array_mut) else {
        return Ok(catalog_json.to_string());
    };
    for model in models {
        let Some(slug) = model.get("slug").and_then(Value::as_str) else {
            continue;
        };
        let Some(user_override) = model_metadata_override_for_slug(override_map, slug) else {
            continue;
        };
        let Some(model_object) = model.as_object_mut() else {
            continue;
        };
        for (key, value) in user_override {
            // 窗口与压缩阈值由 model_windows / model_auto_compact 生成（issue #2191）；
            // max_context_window 是 codex 运行时的 clamp 权威，绝不能被历史 metadata 残留值覆盖。
            if matches!(
                key.as_str(),
                "slug" | "context_window" | "max_context_window" | "auto_compact_token_limit" | "codex_plus_fast_support"
            ) {
                continue;
            }
            model_object.insert(key.clone(), value.clone());
        }
        if user_override.get("codex_plus_reasoning_parameters").and_then(Value::as_str) == Some("omit") {
            model["supported_reasoning_levels"] = json!([]);
            model["default_reasoning_level"] = Value::Null;
            model["supports_reasoning_summaries"] = json!(false);
        }
        crate::model_suffix::apply_model_fast_support(model, user_override.get("codex_plus_fast_support"));
    }
    Ok(serde_json::to_string_pretty(&catalog)?)
}

fn validate_catalog_model_fields(slug: &str, model: &Value, require_fields: bool) -> anyhow::Result<()> {
    for (field, valid) in [
        ("base_instructions", Value::is_string as fn(&Value) -> bool),
        ("supports_parallel_tool_calls", Value::is_boolean as fn(&Value) -> bool),
    ] {
        match model.get(field) {
            Some(value) if valid(value) => {},
            None if !require_fields => {},
            _ => anyhow::bail!("模型 {slug} 的 {field} 类型无效（期望 {}），原目录未修改", if field == "base_instructions" { "string" } else { "boolean" }),
        }
    }
    Ok(())
}

pub(crate) fn validate_managed_model_catalog(catalog: &str) -> anyhow::Result<()> {
    let catalog: Value = serde_json::from_str(catalog)?;
    let models = catalog.get("models").and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("受管模型目录缺少 models 数组"))?;
    for model in models {
        let slug = model.get("slug").and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("受管模型目录缺少模型 slug"))?;
        validate_catalog_model_fields(slug, model, true)?;
    }
    Ok(())
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

/// Kimi 官方（Moonshot 开放平台 / Kimi For Coding）的 Responses 接口不认识 Codex 的
/// `tool_search` 工具类型，请求里带上就整单 400
/// （`tools.N: tool type "tool_search" is not supported`）。
/// 这类上游生成 catalog 时要关掉 `supports_search_tool`，Codex 才不会发该工具，
/// 改为直接列出 MCP 工具。Chat Completions 协议由代理把 tool_search 转成 function
/// 工具，不受影响，因此只判定 Responses。
pub(crate) fn upstream_rejects_tool_search(profile: &RelayProfile) -> bool {
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
    .any(|base_url| kimi_official_base_url(base_url))
}

fn kimi_official_base_url(base_url: &str) -> bool {
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
    ["moonshot.cn", "moonshot.ai", "kimi.com"]
        .iter()
        .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
}

/// 把 catalog 里所有模型的 `supports_search_tool` 置为 false。
/// 须在 `apply_model_metadata_overrides` 之后调用：管理器「保存此模型」会把整份元数据
/// （含模板里的 `supports_search_tool: true`）存成覆盖，覆盖优先会让改任意一项就复发；
/// 上游本来就拒收该工具，允许打开没有意义。
pub(crate) fn disable_search_tool_in_catalog(catalog_json: &str) -> anyhow::Result<String> {
    let mut catalog: Value = serde_json::from_str(catalog_json)
        .map_err(|error| anyhow::anyhow!("catalog JSON 解析失败：{error}"))?;
    let Some(models) = catalog.get_mut("models").and_then(Value::as_array_mut) else {
        return Ok(catalog_json.to_string());
    };
    for model in models.iter_mut().filter(|model| model.is_object()) {
        model["supports_search_tool"] = json!(false);
    }
    Ok(serde_json::to_string_pretty(&catalog)?)
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

/// Responses Lite 线格式（X-OpenAI-Internal-Codex-Responses-Lite 头）只有
/// ChatGPT 产品后端实现；官方公开 API（api.openai.com）与第三方中转均只支持
/// 标准 Responses（判定锚点对齐 sub2api accountCodexToolCapabilities：
/// 认证模式 + 有效 base_url 主机名，而非 provider id）。
///
/// false 是安全默认：declared false 而上游其实支持，损失的只是 Lite 降级优化；
/// declared true 而上游不支持，是运行时不可预期的线格式错配。
/// URL 解析失败一律 false，不猜测。
fn upstream_supports_responses_lite(effective_base_url: Option<&str>, official_login: bool) -> bool {
    if !official_login {
        return false;
    }
    let Some(base_url) = effective_base_url else {
        return false;
    };
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
    host == "chatgpt.com" || host.ends_with(".chatgpt.com")
}

/// 从生成后的有效 config 读取 active transport provider 的 base_url。
/// 注意会话身份（model_provider）可能是保留 id（openai），但 base_url 实际
/// 写在传输表 [model_providers.custom]；故用 active_or_default_provider_id
/// 定位，openai 身份回落 custom 表。仍查不到时回落顶层 base_url。
fn effective_provider_base_url(config_text: &str) -> Option<String> {
    let doc = parse_toml_document(config_text).ok()?;
    let provider_id = active_or_default_provider_id(&doc);
    if let Some(base_url) = doc
        .get("model_providers")
        .and_then(Item::as_table)
        .and_then(|providers| providers.get(&provider_id))
        .and_then(Item::as_table_like)
        .and_then(|provider| provider.get("base_url"))
        .and_then(Item::as_str)
    {
        return Some(base_url.to_string());
    }
    root_key_string(config_text, "base_url")
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

fn copy_standard_responses_catalog(
    home: &Path,
    source: &str,
    target_relative: &str,
    entries: &[crate::model_suffix::ModelCatalogEntry],
    fallback_window: Option<u64>,
    lite_supported: bool,
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
    let configured_windows = entries
        .iter()
        .map(|entry| (entry.slug.clone(), entry.suffix_window.or(fallback_window)))
        .collect::<std::collections::BTreeMap<_, _>>();
    for model in models {
        // Lite 能力跟随上游：官方登录 + ChatGPT 后端保留原值，
        // 其余上游强制关闭（外部目录的 Lite 标记可能照抄官方目录，
        // 与实际上游能力无关，见 upstream_supports_responses_lite）。
        if !lite_supported
            && model.get("use_responses_lite").and_then(Value::as_bool) == Some(true)
        {
            model["use_responses_lite"] = Value::Bool(false);
            changed = true;
        }
        let Some(slug) = model.get("slug").and_then(Value::as_str) else {
            continue;
        };
        // 外部 catalog 的 slug 与本地行的写法/大小写可能不一致，走三级回退查表，
        // 否则配置好的窗口会在外部目录里整条失效、保留上游给的值。
        let Some(Some(window)) = crate::model_suffix::lookup_model_map(&configured_windows, slug)
        else {
            continue;
        };
        if model.get("context_window").and_then(Value::as_u64) != Some(*window) {
            model["context_window"] = json!(window);
            changed = true;
        }
        if model.get("max_context_window").and_then(Value::as_u64) != Some(*window) {
            model["max_context_window"] = json!(window);
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
    let catalog = serde_json::to_string_pretty(&catalog)?;
    validate_managed_model_catalog(&catalog)?;
    crate::settings::atomic_write(&target, catalog.as_bytes())?;
    Ok(true)
}

fn live_external_model_catalog(home: &Path) -> Option<String> {
    let live_text = read_optional_text(&home.join("config.toml")).ok()?;
    let live = parse_toml_document(&live_text).ok()?;
    let path = live.get("model_catalog_json")?.as_str()?.trim();
    (!path.is_empty()
        && !is_codex_plus_managed_model_catalog(home, path)
        && !is_cc_switch_model_catalog(path)
        // 指向不存在的文件时会和显式指针一样让 codex 拒载整份配置（#2123）：
        // 这种「外部指针」没有任何可保留的价值，当作不存在，走正常的托管生成路径。
        && !model_catalog_pointer_file_missing(home, path))
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

pub(crate) fn sanitize_catalog_filename(id: &str) -> String {
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

/// 这条 `model_catalog_json` 指针是否**在任何平台、任何机器上都打不开**。
///
/// codex 核心对读不到的 catalog 不是降级处理，而是直接拒绝加载**整份** config.toml
/// （`os error 3`），用户侧表现为"无法加载 config.toml，因此此对话串无法继续"，
/// 报错信息和真正的故障点毫无关系，极难自诊。所以这种指针绝不能落盘。
///
/// 这里认定**未展开的 shell 变量**这一种形态：codex 自己不做变量展开，所以
/// `%userprofile%\.codex\...` 在任何机器上都不存在，判定是确定的、不会误伤。
/// 「变量已展开但文件确实不存在」另由 `model_catalog_pointer_file_missing`
/// 判定（#2123）——两者后果相同，都是整份 config.toml 拒载。
fn model_catalog_pointer_has_unexpanded_variable(pointer: &str) -> bool {
    let pointer = pointer.trim();
    !pointer.is_empty() && (pointer.contains('%') || pointer.contains('$'))
}

/// 指针指向的 catalog 文件是否**确定不在磁盘上**（#2123）。
///
/// 与 `model_catalog_pointer_has_unexpanded_variable` 是同一族护栏：#2123 报的
/// 是两个形态，「变量没展开」和「变量展开了、但那个路径下什么都没有」。后者常见
/// 于用户换机器/换用户名后指针还写着上一个用户的绝对路径、用户自己清理过
/// `model-catalogs/`，或从别处抄来的配置。codex 遇到读不到的 catalog 直接拒载
/// **整份** config.toml，报错还落在完全无关的行上，所以这种指针同样不能落盘。
///
/// 只按「路径存在性」判定，不做任何猜测：绝对路径直接查；相对路径按 codex 的
/// 语义相对 `CODEX_HOME`（即 `home`）解析。含未展开变量的指针一律视为「无法
/// 判定」，交回上一条护栏处理，避免两次判定互相打架。
fn model_catalog_pointer_file_missing(home: &Path, pointer: &str) -> bool {
    let pointer = pointer.trim();
    if pointer.is_empty() || model_catalog_pointer_has_unexpanded_variable(pointer) {
        return false;
    }
    let path = Path::new(pointer);
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        home.join(path)
    };
    !resolved.is_file()
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
        "skill" | "skills" => Ok("skills"),
        "plugin" | "plugins" => Ok("plugins"),
        other => anyhow::bail!("未知上下文类型：{other}"),
    }
}

fn context_kind_name(table: &str) -> &'static str {
    match table {
        "mcp_servers" => "mcp",
        "skills" => "skill",
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
    if profile.relay_mode == crate::settings::RelayMode::PureApi {
        let provider_token = if profile.uses_no_auth() {
            crate::protocol_proxy::NO_AUTH_PROXY_BEARER_TOKEN
        } else {
            template_api_key
        };
        profile.config_contents =
            set_experimental_bearer_token_in_config(&profile.config_contents, provider_token)?;
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

    // issue #2601：live auth.json 是 Codex 写的 API Key 登录态（如 `auth_mode = "apikey"`）
    // 时，吸收进纯官方快照会被存储校验判为「不支持的官方认证存档格式」，整次切换/保存
    // 失败。这类 live 认证不属于官方快照，保留原快照即可；可存储的格式维持原采纳语义。
    if official_auth_snapshot_is_storable(live_auth) {
        profile.auth_contents = live_auth.to_string();
    } else {
        profile.auth_contents = template_auth.to_string();
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "relay_backfill.official_auth.kept_snapshot",
            json!({
                "profileId": profile.id,
                "reason": "live auth.json 不是可存入官方快照的认证格式，保留原快照",
                "liveAuthMode": serde_json::from_str::<Value>(live_auth).ok().and_then(|value| {
                    value.get("auth_mode").and_then(Value::as_str).map(str::to_string)
                }),
            }),
        );
    }
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
    if profile.uses_no_auth() || profile.relay_mode == RelayMode::Aggregate {
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

/// 生成写入 live 的 auth.json 内容：把本地代理的 bearer token 放进 `OPENAI_API_KEY`，
/// 同时保留已有凭据（官方 OAuth `tokens` 等）。聚合切换与代理注入共用（issue #1604）。
///
/// 来源优先级：**live 优先**，profile 快照只在 live 为空或损坏时回退——Codex 运行中可能
/// 刚刷新过 OAuth token，用旧快照回退会静默覆盖更新的凭据。
/// 因此：1) 不清掉 live 里已有的其它凭据；2) 输出永远是合法 JSON 对象，绝不写空文件
/// （Codex 解析空文件会报 EOF 并退回登录页）；3) live 与快照都不可用时直接报错中止，
/// 不静默覆盖用户凭据。
/// 要同步进 Codex 的登录快照。
///
/// `None`：这次不碰 Token 段（开关关着、不是聚合 —— 与历史行为完全一致）。
/// `Some(contents)`：默认成员的 auth.json 快照（可能为空串，表示成员没有可用登录凭据，
/// 此时要把 live 里残留的 ChatGPT 登录态清掉——否则用户关掉开关、或把默认成员换成
/// API 供应商之后，Codex 里还挂着一个已经不参与聚合的账号）。
type SyncedChatgptTokens = Option<String>;

fn auth_contents_with_proxy_key(
    home: &Path,
    auth_contents: &str,
    bearer_token: &str,
    // synced_login：默认成员的登录快照；`Some("")` 表示"要同步但没有凭据"，
    // 由调用方清理残留（本函数只负责代理 Key，登录态合并见
    // `official_login::auth_contents_with_member_login`）。
    synced_login: Option<&str>,
) -> anyhow::Result<String> {
    let auth_path = home.join("auth.json");
    let live = read_optional_text(&auth_path)?;
    let _ = synced_login;
    if let Some(merged) = merge_proxy_key_into_auth_json(&live, bearer_token) {
        return Ok(merged);
    }
    // live 为空或损坏时才退回已验证的 profile 快照。
    if let Some(merged) = merge_proxy_key_into_auth_json(auth_contents, bearer_token) {
        return Ok(merged);
    }
    if !live.trim().is_empty() {
        anyhow::bail!(
            "{} 不是有效 JSON 对象，已停止切换以避免覆盖当前登录凭据",
            auth_path.display()
        );
    }
    if !auth_contents.trim().is_empty() {
        anyhow::bail!(
            "供应商快照里的 auth.json 不是有效 JSON 对象，已停止切换以避免写入损坏内容"
        );
    }
    // 两边都没有凭据：生成只含代理 token 的合法 JSON，绝不产出空文件。
    Ok(format!(
        "{}\n",
        serde_json::to_string_pretty(&json!({ "OPENAI_API_KEY": bearer_token }))?
    ))
}

/// 纯 API 模式写 auth.json 前的合并（issue #2173）。
///
/// Aggregate / Official 两条分支都会先经 `auth_contents_with_proxy_key` 读 live
/// auth.json 再合并，只有 PureApi 过去是直接写 profile 快照。那把 live 里的
/// `tokens` / `auth_mode` 整体覆盖掉，而 `tokens` 正是「OpenAI 会话身份」
/// （`auth_contents_looks_like_chatgpt_auth` 依赖它）让 CUA 浏览器插件可用的前提，
/// 于是纯 API 供应商每切换一次就可能静默弄坏插件，且不报任何配置错误。
///
/// 策略：以 profile 快照为基底（它承载本供应商需要的 `OPENAI_API_KEY` 等字段），
/// 但把 live 里已有的登录态键原样保留——本 profile 没声明的键也一并继承，
/// 使写入结果与 Aggregate / Official 的行为对齐。
fn pure_api_auth_contents_with_live_login(
    home: &Path,
    profile: &RelayProfile,
) -> anyhow::Result<String> {
    let auth_contents = profile.auth_contents.as_str();
    // 只有会话身份是 openai 的 profile 才保留 live 的 ChatGPT 登录态。
    // 普通纯 API 供应商本就该清掉官方凭据（切换语义要求），无条件保留会破坏它。
    if relay_session_provider_from_config(&profile.config_contents) != RelaySessionProvider::Openai {
        return Ok(auth_contents.to_string());
    }
    let live = read_optional_text(&home.join("auth.json"))?;
    let Some(mut snapshot) = parse_json_object(auth_contents) else {
        // 快照本身不是合法 JSON 对象：沿用旧的「快照为空白则视为无凭据」语义，
        // 非空但损坏时报错，避免写入损坏内容。
        if auth_contents.trim().is_empty() {
            return Ok(auth_contents.to_string());
        }
        anyhow::bail!(
            "供应商快照里的 auth.json 不是有效 JSON 对象，已停止切换以避免写入损坏内容"
        );
    };
    let Some(live_object) = parse_json_object(&live) else {
        return Ok(auth_contents.to_string());
    };

    // 登录态键：live 有就保留，避免把用户的 ChatGPT 登录凭据抹掉。
    const LOGIN_STATE_KEYS: [&str; 2] = ["tokens", "auth_mode"];
    for key in LOGIN_STATE_KEYS {
        if let Some(value) = live_object.get(key) {
            if !value.is_null() {
                snapshot.insert(key.to_string(), value.clone());
            }
        }
    }

    // live 里本 profile 未声明的其它键也继承，保持与 Aggregate / Official 一致。
    for (key, value) in &live_object {
        if value.is_null() || snapshot.contains_key(key) {
            continue;
        }
        snapshot.insert(key.clone(), value.clone());
    }

    Ok(format!(
        "{}\n",
        serde_json::to_string_pretty(&Value::Object(snapshot))?
    ))
}

/// 解析成 JSON 对象；来源为空、非 JSON 或不是对象时返回 None。
fn parse_json_object(source: &str) -> Option<serde_json::Map<String, Value>> {
    if source.trim().is_empty() {
        return None;
    }
    serde_json::from_str::<Value>(source)
        .ok()?
        .as_object()
        .cloned()
}

/// 把代理 token 合进一份 auth.json 文本；来源为空或不是 JSON 对象时返回 None，
/// 由调用方决定是报错还是换一份来源。
fn merge_proxy_key_into_auth_json(source: &str, bearer_token: &str) -> Option<String> {
    if source.trim().is_empty() {
        return None;
    }
    let mut value = serde_json::from_str::<Value>(source).ok()?;
    let object = value.as_object_mut()?;
    object.insert(
        "OPENAI_API_KEY".to_string(),
        Value::String(bearer_token.to_string()),
    );
    serde_json::to_string_pretty(&value)
        .ok()
        .map(|text| format!("{text}\n"))
}

/// 当前 live 配置里实际生效的 API 凭据：`auth.json` 的 `OPENAI_API_KEY` 与
/// 当前 provider 的 `experimental_bearer_token`。读不到或解析失败时返回空列表。
pub fn active_api_credentials_in_home(home: &Path) -> Vec<String> {
    let mut credentials = Vec::new();
    if let Ok(auth) = read_optional_text(&home.join("auth.json"))
        && let Some(key) = codex_auth_api_key(&auth)
    {
        credentials.push(key);
    }
    if let Ok(config) = read_optional_text(&home.join("config.toml"))
        && let Ok(Some(token)) = experimental_bearer_token_from_config(&config)
    {
        let token = token.trim().to_string();
        if !token.is_empty() && !credentials.contains(&token) {
            credentials.push(token);
        }
    }
    credentials
}

fn codex_auth_api_key(auth_contents: &str) -> Option<String> {
    let auth: Value = serde_json::from_str(auth_contents).ok()?;
    auth.get("OPENAI_API_KEY")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(ToString::to_string)
}

/// 模型名只接受合理的单行 slug。issue #1097 问题 2：live config 里的
/// `model =` 可能被污染成整段转义后的供应商配置，写侧转义、读侧
/// （`root_key_string` 不反转义）逐轮叠加导致 settings.json 膨胀到几十 MB、
/// 管理工具白屏。这里统一拒绝换行/控制字符/反斜杠、超长值以及明显混入
/// 供应商配置关键字的值，视为「未声明」。
const MAX_MODEL_NAME_LEN: usize = 256;
const MODEL_NAME_FORBIDDEN_KEYWORDS: [&str; 6] = [
    "model_provider",
    "model_providers",
    "base_url",
    "wire_api",
    "requires_openai_auth",
    "model_catalog_json",
];

fn sanitize_relay_model_name(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed.len() > MAX_MODEL_NAME_LEN
        || trimmed.contains(['\\', '\r', '\n'])
        || trimmed.chars().any(char::is_control)
        || MODEL_NAME_FORBIDDEN_KEYWORDS
            .iter()
            .any(|keyword| trimmed.contains(keyword))
    {
        return None;
    }
    Some(trimmed.to_string())
}

/// 解析 profile 實際使用的模型：優先取 config.toml 裡的 `model =`，
/// 否則退回 profile.model 欄位。供應商測試用它做回退，避免串到別家供應商的模型名。
/// 两侧来源都过 `sanitize_relay_model_name`：被污染的值视为未声明（issue #1097）。
pub fn relay_profile_model(profile: &RelayProfile) -> String {
    root_key_string(&profile.config_contents, "model")
        .and_then(|value| sanitize_relay_model_name(&value))
        .or_else(|| sanitize_relay_model_name(&profile.model))
        .unwrap_or_default()
}

/// 「测试供应商」与 Provider Doctor 使用的模型：供应商配置的默认模型 → 模型列表第一项 → 内置兜底。
/// 不再读取单独的测试模型设置，避免测的模型和实际使用的模型不一致。
pub fn relay_profile_test_model(profile: &RelayProfile) -> String {
    let model = relay_profile_model(profile);
    if !model.is_empty() {
        return model;
    }
    profile
        .model_list
        .lines()
        .map(|line| line.split('[').next().unwrap_or_default())
        .find_map(sanitize_relay_model_name)
        .unwrap_or_else(crate::settings::default_relay_test_model)
}

pub fn relay_profile_base_url(profile: &RelayProfile) -> String {
    if profile.relay_mode == crate::settings::RelayMode::Aggregate {
        return crate::protocol_proxy::local_responses_proxy_base_url(
            crate::protocol_proxy::protocol_proxy_port(),
        );
    }
    let has_managed_proxy_config = provider_string_from_config(&profile.config_contents, "base_url")
        .is_some_and(|base_url| {
            crate::protocol_proxy::is_managed_protocol_proxy_base_url(&base_url)
                || saved_proxy_endpoint_has_real_upstream(profile, &base_url)
        });
    if profile.has_model_routes()
        || profile.uses_no_auth()
        || profile.uses_independent_proxy()
        || profile.web_search_history_compat_enabled()
        || profile.request_compatibility_enabled()
        || profile.rewrites_auto_review_model()
        || has_managed_proxy_config
    {
        if !profile.upstream_base_url.trim().is_empty() {
            return profile.upstream_base_url.trim().to_string();
        }
        if !profile.base_url.trim().is_empty()
            && !crate::protocol_proxy::is_managed_protocol_proxy_base_url(&profile.base_url)
        {
            return profile.base_url.trim().to_string();
        }
    }
    if profile.requires_protocol_proxy_conversion() {
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
    if profile.requires_protocol_proxy_conversion()
        && provider_base_url
            == crate::protocol_proxy::local_responses_proxy_base_url(
                crate::protocol_proxy::protocol_proxy_port(),
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
    // no-auth快照里的codex-plus-no-auth仅供本地客户端访问代理，不是上游凭据。
    if profile.uses_no_auth() {
        return String::new();
    }
    if profile.relay_mode == crate::settings::RelayMode::Aggregate {
        return "codex-plus-aggregate".to_string();
    }
    if let Some(selected) = profile.selected_api_key() {
        return selected.api_key.trim().to_string();
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
    complete_relay_profile_config_with_proxy_port(
        profile,
        crate::protocol_proxy::protocol_proxy_port(),
    )
}

fn complete_relay_profile_config_with_proxy_port(
    profile: &RelayProfile,
    proxy_port: u16,
) -> anyhow::Result<String> {
    let mut doc = parse_toml_document(&profile.config_contents)?;
    let session_provider_id = if profile.relay_mode == RelayMode::Aggregate {
        SettingsStore::default()
            .load()
            .ok()
            .and_then(|settings| {
                settings
                    .active_aggregate_relay_profile()
                    .filter(|aggregate| aggregate.model_aggregation_enabled)
            })
            .map(|aggregate| aggregate.session_provider.as_str().to_string())
            .unwrap_or_else(|| active_session_provider_id(&doc))
    } else {
        active_session_provider_id(&doc)
    };
    let uses_openai_provider = session_provider_id == "openai";
    if uses_openai_provider && profile.protocol != RelayProtocol::Responses {
        anyhow::bail!("OpenAI 会话身份仅支持 Responses API");
    }
    update_remote_control_openai_base_url_with_known(
        &mut doc,
        uses_openai_provider
            || (profile.relay_mode == crate::settings::RelayMode::Official
                && profile.official_mix_api_key),
        managed_openai_base_url(),
        &managed_proxy_base_urls_for_profile(profile),
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
            .and_then(sanitize_relay_model_name)
        {
            model = crate::model_suffix::parse_model_suffix(&first).0;
        }
    }
    // 若用户把后缀语法（如 deepseek-v4-flash[1M]）写在 model 字段，
    // 写入 config.toml 前需剥离后缀；codex 本身不理解后缀，只会按原串匹配 catalog slug。
    let (model, _) = crate::model_suffix::parse_model_suffix(&model);
    if !model.trim().is_empty() {
        doc["model"] = toml_edit::value(model.trim());
    } else {
        // config_contents 里被污染的 `model =`（issue #1097 问题 2）必须剥掉，
        // 否则 normalize 重写快照时脏值会原样保留、逐轮转义放大。
        doc.as_table_mut().remove("model");
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
    // codex 用 `name == "OpenAI"` 的严格匹配判定 provider 是否走 v2 远程压缩
    // （RemoteCompactionSupport::V2，见 openai/codex issue #42313），第三方中转
    // 顶着这个名字会被要求返回 `compaction` 输出项而稳定报错（issue #2217）。
    // 只有真正的 OpenAI 会话身份（官方 OAuth / 混合模式）可以保留该名称，
    // 其余情况一律改写成 provider id，让 codex 回退到本地压缩。
    if !uses_openai_provider
        && provider
            .get("name")
            .and_then(Item::as_str)
            .is_some_and(|name| name.trim().eq_ignore_ascii_case("OpenAI"))
    {
        provider["name"] = toml_edit::value(transport_provider_id.as_str());
    }
    // Codex 26.901 起不再支持 `wire_api = "chat"`（见 openai/codex discussion #7782），
    // 一旦出现会导致整份 config.toml 被判为无效并回退内置默认模型。
    // Chat Completions 上游由本地协议代理（protocol_proxy）负责 responses→chat 转换，
    // 因此对 Codex 暴露的 wire_api 必须恒为 "responses"。
    provider["wire_api"] = toml_edit::value("responses");
    if profile.relay_mode != crate::settings::RelayMode::PureApi
        && provider
            .get("requires_openai_auth")
            .and_then(Item::as_bool)
            .is_none()
    {
        provider["requires_openai_auth"] = toml_edit::value(true);
    }
    if profile.relay_mode == crate::settings::RelayMode::Aggregate {
        // 聚合默认是纯 API 形态。打开「同步默认成员登录」且该成员有可用的 ChatGPT 凭据时，
        // 改成与「官方 + API」混合模式一致的形态：requires_openai_auth = true 让 Codex
        // 读取并显示 auth.json 里的账号登录态，请求仍由 experimental_bearer_token
        // （代理 Key）发往本地聚合代理。为 false 时 Codex 根本不看登录态，界面只显示 provider 名。
        provider["requires_openai_auth"] =
            toml_edit::value(aggregate_syncs_member_login(profile));
    }
    let provider_base_url = if profile.has_model_routes()
        || profile.uses_no_auth()
        || profile.uses_independent_proxy()
        || profile.web_search_history_compat_enabled()
        || profile.request_compatibility_enabled()
        || profile.rewrites_auto_review_model()
    {
        crate::protocol_proxy::local_responses_proxy_base_url(proxy_port)
    } else {
        codex_base_url_for_protocol(base_url.trim(), profile.protocol, proxy_port)
    };
    if !provider_base_url.trim().is_empty() {
        provider["base_url"] = toml_edit::value(provider_base_url.trim());
    }
    if profile.uses_no_auth() {
        provider["experimental_bearer_token"] =
            toml_edit::value(crate::protocol_proxy::NO_AUTH_PROXY_BEARER_TOKEN);
    } else if !api_key.trim().is_empty() {
        // Codex 0.149 起不再为 requires_openai_auth=false 的 provider 借用
        // auth.json 凭据。纯 API 也必须显式携带供应商 Key，不能改成依赖官方登录。
        provider["experimental_bearer_token"] = toml_edit::value(api_key.trim());
    }

    Ok(move_model_providers_before_profiles(
        &ensure_trailing_newline(doc.to_string()),
    ))
}

pub fn normalize_relay_profile_for_storage(profile: &mut RelayProfile) -> anyhow::Result<()> {
    if profile.relay_mode == crate::settings::RelayMode::Official
        && !profile.official_mix_api_key
        && !profile.auth_contents.trim().is_empty()
        && !auth_contents_looks_like_chatgpt_auth(&profile.auth_contents)
    {
        // 保留合法的显式退出/旧 API-only 快照清理语义，其余未知格式不能静默删除。
        let value: Value = serde_json::from_str(&profile.auth_contents)
            .context("认证存档不是有效 JSON，原内容未修改")?;
        let object = value.as_object().ok_or_else(|| anyhow::anyhow!("认证存档必须是 JSON 对象，原内容未修改"))?;
        if object.keys().any(|key| key != "OPENAI_API_KEY") {
            anyhow::bail!("不支持的官方认证存档格式，原内容未修改；请使用受支持的登录流程");
        }
    }
    if profile.api_keys.is_empty()
        && !profile.uses_no_auth()
        && (profile.relay_mode != crate::settings::RelayMode::Official || profile.official_mix_api_key)
    {
        let key = relay_profile_api_key(profile);
        if !key.is_empty() {
            profile.api_keys.push(crate::settings::RelayApiKey {
                id: "default".into(), name: "默认".into(), api_key: key,
            });
        }
    }
    let mut seen_api_key_ids = HashSet::new();
    profile.api_keys = profile
        .api_keys
        .drain(..)
        .filter_map(|mut entry| {
            entry.id = entry.id.trim().to_string();
            entry.name = entry.name.trim().to_string();
            if entry.id.is_empty() || !seen_api_key_ids.insert(entry.id.clone()) {
                return None;
            }
            if entry.name.is_empty() {
                entry.name = "未命名 Key".to_string();
            }
            Some(entry)
        })
        .collect();
    if !profile
        .api_keys
        .iter()
        .any(|entry| entry.id == profile.active_api_key_id)
    {
        profile.active_api_key_id = profile
            .api_keys
            .first()
            .map(|entry| entry.id.clone())
            .unwrap_or_default();
    }
    if let Some(selected) = profile.selected_api_key() {
        profile.api_key = selected.api_key.clone();
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
    normalize_relay_headers(&mut profile.custom_headers)?;
    // Anthropic 上游的认证字段只认两种写法，别名（bearer / x_api_key 等）在这里收敛；
    // 空串表示默认 `x-api-key`。`anthropic_max_tokens` 的 0 表示用内置兜底值，
    // 不强制写盘，避免给既有设置文件引入无意义 diff。
    profile.anthropic_auth_header =
        crate::settings::normalize_anthropic_auth_header(&profile.anthropic_auth_header);

    if profile.model_list.contains('[') {
        let (clean_list, migrated_windows) =
            crate::model_suffix::migrate_model_list_with_suffixes(&profile.model_list);
        let mut windows = parse_model_string_map(&profile.model_windows, "model_windows")?;
        for (slug, window) in migrated_windows {
            windows.entry(slug).or_insert(window);
        }
        profile.model_list = clean_list;
        profile.model_windows = serde_json::to_string(&windows)?;
    }
    let model_windows = parse_model_string_map(&profile.model_windows, "model_windows")?;
    validate_model_windows(&model_windows)?;
    let model_auto_compact =
        parse_model_string_map(&profile.model_auto_compact, "model_auto_compact")?;
    validate_model_auto_compact(&model_auto_compact)?;
    parse_model_metadata_map(&profile.model_metadata)?;
    if profile.relay_mode == crate::settings::RelayMode::Official && !profile.official_mix_api_key {
        profile.config_contents = sanitize_official_profile_config(&profile.config_contents)?;
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
    // PureApi 的 provider token 用于请求鉴权；auth.json 继续保存 API Key，
    // 保持 API 登录态和旧版本兼容，不把它与 live OAuth 凭据混合。
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
    if profile.uses_no_auth() {
        profile.api_key.clear();
    }
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
    // 入参可能来自 official 分支的原始 profile.model 字段（未经 relay_profile_model
    // 出口过滤），污染值并入列表后会在 normalize 逐轮转义放大（issue #1097 问题 2）。
    let model = sanitize_relay_model_name(model).unwrap_or_default();
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

/// 纯官方（非混合）快照能否原样落盘，口径与 `normalize_relay_profile_for_storage`
/// 一致：空（显式退出）、ChatGPT 登录态，或只含 OPENAI_API_KEY 的旧快照。
fn official_auth_snapshot_is_storable(contents: &str) -> bool {
    if contents.trim().is_empty() || auth_contents_looks_like_chatgpt_auth(contents) {
        return true;
    }
    serde_json::from_str::<Value>(contents)
        .ok()
        .and_then(|value| {
            value
                .as_object()
                .map(|object| object.keys().all(|key| key == "OPENAI_API_KEY"))
        })
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
        Err(error) if error.kind() == std::io::ErrorKind::InvalidData => {
            // 上游客户端在中文代码页下可能把 config.toml 写成 GBK/ANSI 字节（#585）。
            // 不能按「没有 live 配置」返回空串：那样切换供应商会把 MCP、项目信任、
            // Windows 沙箱等非供应商设置全部丢掉且毫无提示。这里按 UTF-8 宽松解码——
            // TOML 结构与 ASCII 内容完整保留，只有无法识别的字节变成 U+FFFD；
            // 原始字节先按内容哈希备份一次（同一内容重复读取不再重复备份）。
            let bytes = std::fs::read(path)?;
            let digest = sha2::Sha256::digest(&bytes);
            let tag: String = digest.iter().take(6).map(|byte| format!("{byte:02x}")).collect();
            let backup = path.with_file_name(format!(
                "{}.invalid-utf8-{tag}.bak",
                path.file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| "config.toml".to_string()),
            ));
            let backup_result = if backup.exists() {
                Ok(())
            } else {
                std::fs::write(&backup, &bytes)
            };
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "relay_config.non_utf8_config",
                serde_json::json!({
                    "path": path.to_string_lossy(),
                    "backup": backup.to_string_lossy(),
                    "backup_ok": backup_result.is_ok(),
                    "error": error.to_string(),
                }),
            );
            if backup_result.is_err() {
                return Err(error).context(
                    "config.toml 不是有效 UTF-8，且无法创建备份；原文件未修改",
                );
            }
            Ok(String::from_utf8_lossy(&bytes).into_owned())
        }
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

/// 放弃实时改动前把当前 config.toml / auth.json 原样备份，返回备份目录；
/// 两个文件都不存在时没有可备份的内容，返回 None。
pub fn backup_live_files(home: &Path) -> anyhow::Result<Option<String>> {
    let read = |name: &str| match std::fs::read(home.join(name)) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(anyhow::Error::from(error).context(format!("读取实时 {name} 失败"))),
    };
    let config = read("config.toml")?;
    let auth = read("auth.json")?;
    create_live_backup(home, config.as_deref(), auth.as_deref())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_config_detects_managed_protocol_proxy_for_active_provider_only() {
        let home = tempfile::tempdir().unwrap();
        let config_path = home.path().join("config.toml");
        let managed = managed_openai_base_url();
        let direct = "https://upstream.example/v1";

        // 没有 config.toml：不需要本地代理。
        assert!(!live_config_uses_managed_protocol_proxy_in_home(
            home.path()
        ));

        // 活动 provider 直连上游：不需要。
        std::fs::write(
            &config_path,
            format!("model_provider = 'custom'\n[model_providers.custom]\nbase_url = '{direct}'\n"),
        )
        .unwrap();
        assert!(!live_config_uses_managed_protocol_proxy_in_home(
            home.path()
        ));

        // 活动 provider 指向本地代理：需要常驻。
        std::fs::write(
            &config_path,
            format!(
                "model_provider = 'custom'\n[model_providers.custom]\nbase_url = '{managed}'\n"
            ),
        )
        .unwrap();
        assert!(live_config_uses_managed_protocol_proxy_in_home(home.path()));

        // 只有非活动 provider 指向本地代理时不能误判，否则会拉起用不到的 helper。
        std::fs::write(
            &config_path,
            format!(
                "model_provider = 'custom'\n[model_providers.custom]\nbase_url = '{direct}'\n\
                 [model_providers.other]\nbase_url = '{managed}'\n"
            ),
        )
        .unwrap();
        assert!(!live_config_uses_managed_protocol_proxy_in_home(
            home.path()
        ));

        // 官方会话身份：根键 openai_base_url 指向本地代理。
        std::fs::write(
            &config_path,
            format!(
                "model_provider = 'openai'\nopenai_base_url = '{managed}'\n\
                 [model_providers.custom]\nbase_url = 'https://chatgpt.com/backend-api/codex'\n"
            ),
        )
        .unwrap();
        assert!(live_config_uses_managed_protocol_proxy_in_home(home.path()));
    }

    #[test]
    fn independent_proxy_routes_responses_and_preserves_the_real_upstream() {
        let state = tempfile::tempdir().unwrap();
        let _scope = crate::relay_proxy::ProxyPathTestScope::new(state.path());
        let mut profile = RelayProfile {
            id: "independent-proxy-regression".to_string(),
            relay_mode: RelayMode::PureApi,
            protocol: RelayProtocol::Responses,
            model: "fixture-model".to_string(),
            base_url: "https://upstream.example/v1".to_string(),
            api_key: "fixture-key".to_string(),
            ..RelayProfile::default()
        };
        crate::relay_proxy::set_proxy_for_profile(&profile.id, "http://127.0.0.1:17890").unwrap();
        normalize_relay_profile_for_storage(&mut profile).unwrap();
        assert_eq!(relay_profile_base_url(&profile), "https://upstream.example/v1");
        assert!(profile.config_contents.contains(&format!("base_url = \"{}\"", managed_openai_base_url())));

        let settings = BackendSettings {
            relay_profiles_enabled: true,
            active_relay_id: profile.id.clone(),
            relay_profiles: vec![profile.clone()],
            ..BackendSettings::default()
        };
        assert!(settings.active_relay_transport_uses_protocol_proxy());
        assert!(settings.active_relay_uses_protocol_proxy());
        let home = tempfile::tempdir().unwrap();
        std::fs::write(
            home.path().join("config.toml"),
            "model_provider = 'custom'\n[model_providers.custom]\nbase_url = 'https://upstream.example/v1'\n",
        ).unwrap();
        assert!(ensure_active_protocol_proxy_config_in_home(home.path(), &settings).unwrap());
        let live = std::fs::read_to_string(home.path().join("config.toml")).unwrap();
        assert!(live.contains(&managed_openai_base_url()));

        crate::relay_proxy::set_proxy_for_profile(&profile.id, "").unwrap();
        normalize_relay_profile_for_storage(&mut profile).unwrap();
        assert_eq!(relay_profile_base_url(&profile), "https://upstream.example/v1");
        assert!(profile.config_contents.contains("base_url = \"https://upstream.example/v1\""));
        assert!(!settings.active_relay_transport_uses_protocol_proxy());

        // 同一个 sidecar 配置不能接管纯官方 OAuth。
        crate::relay_proxy::set_proxy_for_profile(&profile.id, "http://127.0.0.1:17890").unwrap();
        profile.relay_mode = RelayMode::Official;
        profile.official_mix_api_key = false;
        assert!(!profile.uses_independent_proxy());
    }

    #[test]
    fn repeated_port_migrations_rewrite_only_known_managed_openai_base_urls() {
        let known = [57321, 58077, 58088].map(crate::protocol_proxy::local_responses_proxy_base_url);
        let current = crate::protocol_proxy::local_responses_proxy_base_url(58099);
        for old in &known {
            let mut doc: DocumentMut = format!("openai_base_url = '{old}'\n").parse().unwrap();
            update_remote_control_openai_base_url_with_known(&mut doc, true, current.clone(), &known);
            assert_eq!(doc[OPENAI_BASE_URL_KEY].as_str(), Some(current.as_str()));
            let mut doc: DocumentMut = format!("openai_base_url = '{old}'\n").parse().unwrap();
            update_remote_control_openai_base_url_with_known(&mut doc, false, current.clone(), &known);
            assert!(doc.get(OPENAI_BASE_URL_KEY).is_none());
            let mut doc: DocumentMut = format!("openai_base_url = '{old}/'\n").parse().unwrap();
            update_remote_control_openai_base_url_with_known(&mut doc, true, current.clone(), &known);
            assert_eq!(doc[OPENAI_BASE_URL_KEY].as_str(), Some(current.as_str()));
        }
        for custom in ["http://127.0.0.1:15721/v1", "http://localhost:58077/v1", "http://127.0.0.1:58077/other"] {
            let mut doc: DocumentMut = format!("openai_base_url = '{custom}'\n").parse().unwrap();
            update_remote_control_openai_base_url_with_known(&mut doc, true, current.clone(), &known);
            assert_eq!(doc[OPENAI_BASE_URL_KEY].as_str(), Some(custom));
            update_remote_control_openai_base_url_with_known(&mut doc, false, current.clone(), &known);
            assert_eq!(doc[OPENAI_BASE_URL_KEY].as_str(), Some(custom));
        }
    }

    #[test]
    fn pre_history_snapshot_repairs_owned_old_port_but_preserves_custom_loopback() {
        let old = "http://127.0.0.1:58113/v1";
        let upstream = "https://upstream.example/v1";
        let mut owned = RelayProfile {
            id: "legacy-managed-port-regression".to_string(),
            relay_mode: RelayMode::PureApi,
            protocol: RelayProtocol::Responses,
            base_url: upstream.to_string(),
            upstream_base_url: upstream.to_string(),
            api_key: "fixture-key".to_string(),
            config_contents: format!(
                "model_provider = 'openai'\nopenai_base_url = '{old}'\n[model_providers.custom]\nbase_url = '{old}'\nwire_api = 'responses'\n"
            ),
            ..RelayProfile::default()
        };
        let home = tempfile::tempdir().unwrap();
        std::fs::write(home.path().join("config.toml"), &owned.config_contents).unwrap();
        let settings = BackendSettings {
            relay_profiles_enabled: true,
            active_relay_id: owned.id.clone(),
            relay_profiles: vec![owned.clone()],
            ..BackendSettings::default()
        };
        assert!(ensure_active_protocol_proxy_config_in_home(home.path(), &settings).unwrap());
        let live = std::fs::read_to_string(home.path().join("config.toml")).unwrap();
        assert_eq!(root_key_string(&live, OPENAI_BASE_URL_KEY).as_deref(), Some(managed_openai_base_url().as_str()));
        normalize_relay_profile_for_storage(&mut owned).unwrap();
        assert_eq!(relay_profile_base_url(&owned), upstream);
        assert_eq!(root_key_string(&owned.config_contents, OPENAI_BASE_URL_KEY).as_deref(), Some(managed_openai_base_url().as_str()));

        // root 自定义回环 URL 与 transport 快照无关，不能借其它字段认领。
        let mut custom_root = owned.clone();
        custom_root.config_contents = format!(
            "model_provider = 'openai'\nopenai_base_url = '{old}'\n[model_providers.custom]\nbase_url = '{upstream}'\nwire_api = 'responses'\n"
        );
        normalize_relay_profile_for_storage(&mut custom_root).unwrap();
        assert_eq!(root_key_string(&custom_root.config_contents, OPENAI_BASE_URL_KEY).as_deref(), Some(old));

        // 真正的自定义本地上游没有分离的真实 upstream，保持其 root 和 provider。
        let mut custom_transport = owned;
        custom_transport.base_url = old.to_string();
        custom_transport.upstream_base_url = old.to_string();
        custom_transport.config_contents = format!(
            "model_provider = 'openai'\nopenai_base_url = '{old}'\n[model_providers.custom]\nbase_url = '{old}'\nwire_api = 'responses'\n"
        );
        normalize_relay_profile_for_storage(&mut custom_transport).unwrap();
        assert_eq!(relay_profile_base_url(&custom_transport), old);
        assert_eq!(root_key_string(&custom_transport.config_contents, OPENAI_BASE_URL_KEY).as_deref(), Some(old));
    }

    #[test]
    fn migrated_proxy_port_rewrites_default_port_openai_base_url_only() {
        // #1163：端口迁移后，根级 openai_base_url 必须跟到新端口；用户自己的本地代理不动。
        let migrated = crate::protocol_proxy::local_responses_proxy_base_url(58077);
        let mut doc: DocumentMut = "openai_base_url = \"http://127.0.0.1:57321/v1\"\n".parse().unwrap();
        update_remote_control_openai_base_url_with(&mut doc, true, migrated.clone());
        assert_eq!(doc[OPENAI_BASE_URL_KEY].as_str(), Some(migrated.as_str()));

        let mut doc: DocumentMut = "openai_base_url = \"http://127.0.0.1:15721/v1\"\n".parse().unwrap();
        update_remote_control_openai_base_url_with(&mut doc, true, migrated.clone());
        assert_eq!(doc[OPENAI_BASE_URL_KEY].as_str(), Some("http://127.0.0.1:15721/v1"));

        // 关闭时旧端口的托管值也要清掉。
        let mut doc: DocumentMut = "openai_base_url = \"http://127.0.0.1:57321/v1\"\n".parse().unwrap();
        update_remote_control_openai_base_url_with(&mut doc, false, migrated);
        assert!(doc.get(OPENAI_BASE_URL_KEY).is_none());
    }

    #[test]
    fn elevated_windows_sandbox_is_kept_once_setup_is_complete() {
        // #2615：已完成 elevated 初始化的用户不能被每次启动降级成 unelevated。
        let home = tempfile::tempdir().unwrap();
        let config = "[windows]\nsandbox = \"elevated\"\n";
        std::fs::write(home.path().join("config.toml"), config).unwrap();

        // 未初始化：普通权限进程照旧降级，避免卡在 updateRequired。
        assert!(ensure_windows_sandbox_usable_with(home.path(), false).unwrap());
        let downgraded = std::fs::read_to_string(home.path().join("config.toml")).unwrap();
        assert!(downgraded.contains("sandbox = \"unelevated\""), "{downgraded}");

        // 初始化产物齐全且版本一致：保留用户的 elevated。
        std::fs::write(home.path().join("config.toml"), config).unwrap();
        std::fs::create_dir_all(home.path().join(".sandbox")).unwrap();
        std::fs::create_dir_all(home.path().join(".sandbox-secrets")).unwrap();
        std::fs::write(
            home.path().join(".sandbox").join("setup_marker.json"),
            r#"{"version":5,"offline_username":"CodexSandboxOffline","online_username":"CodexSandboxOnline"}"#,
        )
        .unwrap();
        std::fs::write(
            home.path().join(".sandbox-secrets").join("sandbox_users.json"),
            r#"{"version":5,"offline":{},"online":{}}"#,
        )
        .unwrap();
        assert!(!ensure_windows_sandbox_usable_with(home.path(), false).unwrap());
        let kept = std::fs::read_to_string(home.path().join("config.toml")).unwrap();
        assert_eq!(kept, config, "初始化完成后不应改写用户配置");

        // 两份产物版本不一致（初始化未完成或被升级打断）：仍按未初始化处理。
        std::fs::write(
            home.path().join(".sandbox-secrets").join("sandbox_users.json"),
            r#"{"version":4}"#,
        )
        .unwrap();
        assert!(ensure_windows_sandbox_usable_with(home.path(), false).unwrap());

        // 管理员进程从不改写。
        std::fs::write(home.path().join("config.toml"), config).unwrap();
        assert!(!ensure_windows_sandbox_usable_with(home.path(), true).unwrap());
    }

    /// 回归 09-23 现场故障：裸的 [mcp_servers] 父表头把切块锚点钉在 mcp_servers，
    /// 之后两个重复的 [mcp_servers.node_repl] 落在同一块里，整块解析失败后退回
    /// 逐行去重，后出现的那段四键被整段丢掉。修复后重复标准表头必须触发切块。
    #[test]
    fn normalize_duplicate_toml_text_keeps_node_repl_keys_under_repeated_header() {
        let contents = r#"[mcp_servers]
[mcp_servers.search]
command = "search"

[mcp_servers.node_repl]
NODE_REPL_NODE_PATH = "n"

[mcp_servers.node_repl.env]
CODEX_HOME = "h"

[mcp_servers.node_repl]
args = []
command = "node_repl"
env_vars = ["CODEX_WINDOWS_REGISTERED_CORE"]
startup_timeout_sec = 120

[mcp_servers.cua_repl]
command = "cua"
"#;

        let normalized = normalize_duplicate_toml_text(contents);
        let doc = normalized
            .parse::<DocumentMut>()
            .expect("normalized output must stay valid TOML");

        let node = &doc["mcp_servers"]["node_repl"];
        assert_eq!(node["command"].as_str(), Some("node_repl"));
        assert_eq!(node["startup_timeout_sec"].as_integer(), Some(120));
        assert_eq!(node["env_vars"].as_array().map(|a| a.len()), Some(1));
        assert_eq!(node["NODE_REPL_NODE_PATH"].as_str(), Some("n"));
        assert_eq!(node["env"]["CODEX_HOME"].as_str(), Some("h"));
        assert_eq!(doc["mcp_servers"]["cua_repl"]["command"].as_str(), Some("cua"));
    }

    /// 回归真实故障：`[mcp_servers.node_repl]`（带正确的 `.env` 子表）之后又混入
    /// 一个裸的空 `[mcp_servers]` 表头。行级去重会把两者当成互不相干的字符串，
    /// 原样保留进输出——空表头在 TOML 语义上会与已有子表冲突，Codex 加载时报
    /// `invalid transport`。语义合并后 `mcp_servers.node_repl` 的字段必须完整
    /// 保留，且不能再出现裸的 `[mcp_servers]` 空表头。
    #[test]
    fn normalize_duplicate_toml_text_merges_child_table_with_later_empty_parent_header() {
        let contents = "\
[mcp_servers.node_repl]
command = \"node\"

[mcp_servers.node_repl.env]
CODEX_HOME = \"/home/user/.codex\"

[mcp_servers]
";

        let normalized = normalize_duplicate_toml_text(contents);
        let doc = normalized.parse::<DocumentMut>().expect("must stay valid TOML");

        assert_eq!(
            doc["mcp_servers"]["node_repl"]["command"].as_str(),
            Some("node")
        );
        assert_eq!(
            doc["mcp_servers"]["node_repl"]["env"]["CODEX_HOME"].as_str(),
            Some("/home/user/.codex")
        );
    }

    /// 同一个表头出现两次、各自只写了部分字段：语义合并要拼成一份完整的表，
    /// 而不是按旧的行级去重丢弃第二次出现的整段内容。
    #[test]
    fn normalize_duplicate_toml_text_merges_fields_from_repeated_same_header() {
        let contents = "\
[mcp_servers.node_repl]
command = \"node\"

[mcp_servers.node_repl]
cwd = \"/tmp\"
";

        let normalized = normalize_duplicate_toml_text(contents);
        let doc = normalized.parse::<DocumentMut>().expect("must stay valid TOML");

        assert_eq!(doc["mcp_servers"]["node_repl"]["command"].as_str(), Some("node"));
        assert_eq!(doc["mcp_servers"]["node_repl"]["cwd"].as_str(), Some("/tmp"));
    }

    /// 重复根键：后写覆盖前写，与 TOML 对同名键重复赋值时的直觉一致。
    #[test]
    fn normalize_duplicate_toml_text_later_root_key_wins() {
        let contents = "model = \"a\"\nmodel = \"b\"\n";
        let normalized = normalize_duplicate_toml_text(contents);
        let doc = normalized.parse::<DocumentMut>().expect("must stay valid TOML");
        assert_eq!(doc["model"].as_str(), Some("b"));
    }

    /// 回归真实故障：数组表父表头与它的子表头被切块逻辑拆开后，子表片段单独解析
    /// 会把父级隐式降级成普通表，合并时整棵数组表被覆盖，外层表头不再打印。
    #[test]
    fn normalize_duplicate_toml_text_keeps_array_table_parent_headers() {
        let contents = "\
[hooks]
[[hooks.UserPromptSubmit]]
[[hooks.UserPromptSubmit.hooks]]
type = \"command\"
command = \"pwsh -File ups.ps1\"

[[hooks.Stop]]
[[hooks.Stop.hooks]]
type = \"command\"
command = \"pwsh -File stop.ps1\"

[[hooks.SessionEnd]]
[[hooks.SessionEnd.hooks]]
type = \"command\"
command = \"pwsh -File end.ps1\"
";

        let normalized = normalize_duplicate_toml_text(contents);
        let doc = normalized
            .parse::<DocumentMut>()
            .expect("normalized output must stay valid TOML");

        let hooks = doc["hooks"].as_table().expect("hooks must stay a table");
        for key in ["UserPromptSubmit", "Stop", "SessionEnd"] {
            let tables = hooks[key]
                .as_array_of_tables()
                .unwrap_or_else(|| panic!("hooks.{key} must stay an array of tables"));
            assert_eq!(tables.len(), 1, "hooks.{key} must have exactly one element");
        }

        for header in ["[[hooks.UserPromptSubmit]]", "[[hooks.Stop]]", "[[hooks.SessionEnd]]"] {
            assert!(normalized.contains(header), "missing outer array-table header {header}");
        }
    }

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

    /// issue #1097 问题 2：live config 的 `model =` 可能被污染成整段转义后的
    /// 供应商配置（含反斜杠/换行逃逸、base_url 等关键字），读回不反转义会在
    /// 保存/切换循环里逐轮放大。这里验证出口统一拒绝污染值。
    #[test]
    fn relay_profile_model_rejects_polluted_model_values() {
        let polluted = "gpt-5.6-sol\\n\\nmodel_provider = \\\"custom\\\"\\nbase_url = \\\"https://relay.example.test/v1\\\"\\n";

        // config 里的污染值视为未声明，退回 profile.model 字段
        let falls_back = RelayProfile {
            config_contents: format!("model = \"{polluted}\"\nmodel_provider = \"custom\"\n"),
            model: "deepseek-v4-pro".to_string(),
            ..RelayProfile::default()
        };
        assert_eq!(relay_profile_model(&falls_back), "deepseek-v4-pro");

        // 两侧都被污染（字段值含反斜杠逃逸）→ 空串
        let rejected = RelayProfile {
            config_contents: format!("model = \"{polluted}\"\nmodel_provider = \"custom\"\n"),
            model: "also\\npolluted".to_string(),
            ..RelayProfile::default()
        };
        assert!(relay_profile_model(&rejected).trim().is_empty());

        // 合法形态不受影响：斜杠命名空间、后缀语法
        for valid in ["gpt-5.6-sol", "openai/gpt-4o", "deepseek-v4-flash[1M]"] {
            assert_eq!(
                relay_profile_model(&RelayProfile {
                    config_contents: format!("model = \"{valid}\"\n"),
                    ..RelayProfile::default()
                }),
                valid
            );
        }
    }

    /// 伪造一个带 threads / automation_runs 表的 Codex 会话库。
    /// 列名与真实 Codex schema 对齐（threads: id+model；automation_runs 至少含
    /// thread_id / updated_at / archived_reason）。
    fn write_goal_thread_fixture_db(home: &Path, threads: &[(&str, &str)], runs: &[(&str, &str, Option<&str>)]) {
        let sqlite_dir = home.join("sqlite");
        std::fs::create_dir_all(&sqlite_dir).unwrap();
        let conn = rusqlite::Connection::open(sqlite_dir.join("codex-dev.db")).unwrap();
        conn.execute(
            "CREATE TABLE IF NOT EXISTS threads (id TEXT PRIMARY KEY, model TEXT)",
            [],
        )
        .unwrap();
        for (id, model) in threads {
            conn.execute(
                "INSERT OR REPLACE INTO threads (id, model) VALUES (?1, ?2)",
                [id, model],
            )
            .unwrap();
        }
        conn.execute(
            "CREATE TABLE IF NOT EXISTS automation_runs (\
                thread_id TEXT, automation_id TEXT, status TEXT, read_at TEXT, \
                thread_title TEXT, source_cwd TEXT, inbox_title TEXT, inbox_summary TEXT, \
                created_at TEXT, updated_at TEXT, archived_user_message TEXT, \
                archived_assistant_message TEXT, archived_reason TEXT\
             )",
            [],
        )
        .unwrap();
        for (thread_id, updated_at, archived_reason) in runs {
            conn.execute(
                "INSERT INTO automation_runs (thread_id, updated_at, archived_reason) VALUES (?1, ?2, ?3)",
                rusqlite::params![thread_id, updated_at, archived_reason],
            )
            .unwrap();
        }
    }

    fn goal_drift_test_profile() -> RelayProfile {
        RelayProfile {
            relay_mode: crate::settings::RelayMode::PureApi,
            protocol: crate::settings::RelayProtocol::Responses,
            config_contents: "model_provider = \"ai\"\n\n[model_providers.ai]\nname = \"ai\"\nwire_api = \"responses\"\nrequires_openai_auth = true\nbase_url = \"https://ahg.codes\"\n"
                .to_string(),
            auth_contents: "{}\n".to_string(),
            model_list: "astra-6-high\ngpt-5.6-luna-max".to_string(),
            ..RelayProfile::default()
        }
    }

    fn apply_and_read_model(home: &Path, profile: &RelayProfile) -> String {
        apply_relay_profile_to_home_with_switch_rules(home, profile, "").unwrap();
        let config = std::fs::read_to_string(home.join("config.toml")).unwrap();
        config
            .lines()
            .find_map(|line| line.strip_prefix("model = "))
            .unwrap_or_default()
            .to_string()
    }

    /// issue #2264：重启后目标模式任务自动继续，但默认 model 回落到
    /// model_list 第一条（astra-6-high），长线任务被静默换模型烧额度。
    /// 修复预期：profile 未声明默认模型时，优先对齐未归档目标任务的持久化模型。
    #[test]
    fn apply_aligns_default_model_with_unarchived_goal_thread() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("auth.json"), "{}\n").unwrap();
        write_goal_thread_fixture_db(
            temp.path(),
            &[("t-goal", "gpt-5.6-luna-max"), ("t-old", "astra-6-high")],
            &[("t-goal", "2026-09-21T02:00:00Z", None)],
        );

        let model = apply_and_read_model(temp.path(), &goal_drift_test_profile());
        assert_eq!(
            model, "\"gpt-5.6-luna-max\"",
            "未归档目标任务的模型应优先于 model_list 第一条"
        );
    }

    /// 对照组：没有未归档目标任务时保持旧行为（model_list 第一条），
    /// 无目标任务的用户不受本次修复影响。
    #[test]
    fn apply_keeps_model_list_head_without_unarchived_goal_runs() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("auth.json"), "{}\n").unwrap();
        write_goal_thread_fixture_db(
            temp.path(),
            &[("t-goal", "gpt-5.6-luna-max")],
            &[("t-goal", "2026-09-21T02:00:00Z", Some("done"))],
        );

        let model = apply_and_read_model(temp.path(), &goal_drift_test_profile());
        assert_eq!(model, "\"astra-6-high\"");
    }

    /// 用户显式声明的默认模型优先级最高，不被目标任务模型覆盖。
    #[test]
    fn apply_keeps_explicit_profile_model_over_goal_thread_model() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("auth.json"), "{}\n").unwrap();
        write_goal_thread_fixture_db(
            temp.path(),
            &[("t-goal", "gpt-5.6-luna-max")],
            &[("t-goal", "2026-09-21T02:00:00Z", None)],
        );

        let mut profile = goal_drift_test_profile();
        profile.model = "my-explicit-model".to_string();
        let model = apply_and_read_model(temp.path(), &profile);
        assert_eq!(model, "\"my-explicit-model\"");
    }

    /// threads 表缺 model 列（schema 漂移）时静默回落旧行为。
    #[test]
    fn apply_falls_back_when_threads_table_lacks_model_column() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("auth.json"), "{}\n").unwrap();
        let sqlite_dir = temp.path().join("sqlite");
        std::fs::create_dir_all(&sqlite_dir).unwrap();
        let conn = rusqlite::Connection::open(sqlite_dir.join("codex-dev.db")).unwrap();
        conn.execute("CREATE TABLE threads (id TEXT PRIMARY KEY)", [])
            .unwrap();
        conn.execute(
            "CREATE TABLE automation_runs (thread_id TEXT, updated_at TEXT, archived_reason TEXT)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO automation_runs VALUES ('t-goal', '2026-09-21T02:00:00Z', NULL)",
            [],
        )
        .unwrap();
        drop(conn);

        let model = apply_and_read_model(temp.path(), &goal_drift_test_profile());
        assert_eq!(model, "\"astra-6-high\"");
    }

    /// 完全没有会话库时回落旧行为。
    #[test]
    fn apply_falls_back_without_session_db() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("auth.json"), "{}\n").unwrap();
        let model = apply_and_read_model(temp.path(), &goal_drift_test_profile());
        assert_eq!(model, "\"astra-6-high\"");
    }

    /// 目标任务的持久化模型带 catalog 后缀语法时，写入 config 前剥掉后缀。
    #[test]
    fn apply_strips_suffix_from_goal_thread_model() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("auth.json"), "{}\n").unwrap();
        write_goal_thread_fixture_db(
            temp.path(),
            &[("t-goal", "gpt-5.6-luna-max[1M]")],
            &[("t-goal", "2026-09-21T02:00:00Z", None)],
        );

        let model = apply_and_read_model(temp.path(), &goal_drift_test_profile());
        assert_eq!(model, "\"gpt-5.6-luna-max\"");
    }

    /// 多条未归档目标任务时取 updated_at 最新的一条。
    #[test]
    fn apply_prefers_most_recent_unarchived_goal_run() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("auth.json"), "{}\n").unwrap();
        write_goal_thread_fixture_db(
            temp.path(),
            &[("t-old", "qwen4-max"), ("t-new", "gpt-5.6-luna-max")],
            &[
                ("t-old", "2026-09-20T01:00:00Z", None),
                ("t-new", "2026-09-21T02:00:00Z", None),
            ],
        );

        let model = apply_and_read_model(temp.path(), &goal_drift_test_profile());
        assert_eq!(model, "\"gpt-5.6-luna-max\"");
    }

    /// 存量污染救援：旧版把 model_list 第一条写进 config.toml 后，切换供应商时
    /// backfill 又把它固化进 profile（profile.model 或 config_contents 的 model =）。
    /// 该值是工具写入的隐式默认而非用户显式意图，目标任务存在且模型不同时
    /// 必须重新对齐，否则修复对存量用户无效。
    #[test]
    fn apply_rescues_profile_model_polluted_by_legacy_model_list_head() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("auth.json"), "{}\n").unwrap();
        write_goal_thread_fixture_db(
            temp.path(),
            &[("t-goal", "gpt-5.6-luna-max")],
            &[("t-goal", "2026-09-21T02:00:00Z", None)],
        );

        // 污染形态一：profile.model 字段被 backfill 固化成 model_list 第一条
        let mut polluted_field = goal_drift_test_profile();
        polluted_field.model = "astra-6-high".to_string();
        let model = apply_and_read_model(temp.path(), &polluted_field);
        assert_eq!(
            model, "\"gpt-5.6-luna-max\"",
            "被固化的 model_list 第一条不应挡住目标任务对齐"
        );

        // 污染形态二：config_contents 里被固化了 model = 第一条
        let mut polluted_config = goal_drift_test_profile();
        polluted_config.config_contents = format!(
            "model = \"astra-6-high\"\n{}",
            polluted_config.config_contents
        );
        let model = apply_and_read_model(temp.path(), &polluted_config);
        assert_eq!(
            model, "\"gpt-5.6-luna-max\"",
            "config_contents 里固化的 model_list 第一条不应挡住目标任务对齐"
        );
    }

    /// 完整时间线（issue #2264 的长期使用形态）：apply 对齐写入目标任务模型 →
    /// 切换供应商触发 backfill → 用户把目标任务换到新模型 → 再次 apply。
    /// backfill 不得把工具写入的目标任务模型固化进 profile，否则第二次 apply
    /// 会被固化值挡住、漂移重现。
    #[test]
    fn apply_realigns_after_backfill_when_goal_model_changed() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("auth.json"), "{}\n").unwrap();
        write_goal_thread_fixture_db(
            temp.path(),
            &[("t-goal", "gpt-5.6-luna-max")],
            &[("t-goal", "2026-09-21T02:00:00Z", None)],
        );

        let mut profile = goal_drift_test_profile();
        // 第一次 apply：对齐写入 gpt-5.6-luna-max
        assert_eq!(
            apply_and_read_model(temp.path(), &profile),
            "\"gpt-5.6-luna-max\""
        );

        // 切换供应商：backfill 把 live config 固化回 profile
        let mut common = String::new();
        backfill_relay_profile_from_home_with_common(temp.path(), &mut profile, &mut common)
            .unwrap();

        // 目标任务换到新模型继续跑
        write_goal_thread_fixture_db(
            temp.path(),
            &[("t-goal", "gpt-5.6-luna-max"), ("t-goal-2", "kimi-k3")],
            &[("t-goal-2", "2026-09-21T23:00:00Z", None)],
        );

        // 第二次 apply：必须对齐到新目标任务模型
        let model = apply_and_read_model(temp.path(), &profile);
        assert_eq!(
            model, "\"kimi-k3\"",
            "backfill 固化的旧目标任务模型不应挡住重新对齐"
        );
    }

    /// launcher 启动时的 live config 对齐：issue #2264 的漂移发生在重启链路上，
    /// 而 manager apply 不在重启路径里（launcher 不重放完整 apply，避免覆盖
    /// Codex 回写的 live 值）。这一步只接管「工具写入的隐式默认」——live model
    /// 为空、等于 profile 模板声明值、或等于 model_list 第一条——且有未归档
    /// 目标任务时，把 config.toml 根 model 改写为目标任务模型。用户/Codex 在
    /// UI 选择后回写的 live 值（不属于以上集合）一律不动。
    #[test]
    fn align_live_config_rewrites_tool_written_default_to_goal_model() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("auth.json"), "{}\n").unwrap();
        std::fs::write(
            temp.path().join("config.toml"),
            "model = \"astra-6-high\"\nmodel_provider = \"ai\"\n",
        )
        .unwrap();
        write_goal_thread_fixture_db(
            temp.path(),
            &[("t-goal", "gpt-5.6-luna-max")],
            &[("t-goal", "2026-09-21T02:00:00Z", None)],
        );

        let profile = goal_drift_test_profile();
        assert!(align_live_config_model_with_goal_thread(temp.path(), &profile).unwrap());
        let model = apply_and_read_model_config_line(temp.path());
        assert_eq!(model, "\"gpt-5.6-luna-max\"");
    }

    /// Codex 在 UI 选择后回写的 live 值（不属于工具写入集合）必须保留。
    #[test]
    fn align_live_config_preserves_user_written_live_model() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("config.toml"),
            "model = \"gpt-5.6-sol\"\nmodel_provider = \"ai\"\n",
        )
        .unwrap();
        write_goal_thread_fixture_db(
            temp.path(),
            &[("t-goal", "gpt-5.6-luna-max")],
            &[("t-goal", "2026-09-21T02:00:00Z", None)],
        );

        let profile = goal_drift_test_profile();
        assert!(!align_live_config_model_with_goal_thread(temp.path(), &profile).unwrap());
        let model = apply_and_read_model_config_line(temp.path());
        assert_eq!(model, "\"gpt-5.6-sol\"");
    }

    /// 无未归档目标任务时不改写；live 值已等于目标任务模型时也不改写。
    #[test]
    fn align_live_config_noop_without_goal_or_when_matching() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("config.toml"),
            "model = \"astra-6-high\"\nmodel_provider = \"ai\"\n",
        )
        .unwrap();
        let profile = goal_drift_test_profile();
        assert!(!align_live_config_model_with_goal_thread(temp.path(), &profile).unwrap());

        write_goal_thread_fixture_db(
            temp.path(),
            &[("t-goal", "astra-6-high")],
            &[("t-goal", "2026-09-21T02:00:00Z", None)],
        );
        assert!(!align_live_config_model_with_goal_thread(temp.path(), &profile).unwrap());
    }

    fn apply_and_read_model_config_line(home: &Path) -> String {
        let config = std::fs::read_to_string(home.join("config.toml")).unwrap();
        config
            .lines()
            .find_map(|line| line.strip_prefix("model = "))
            .unwrap_or_default()
            .to_string()
    }

    #[test]
    fn upstream_supports_responses_lite_matches_only_chatgpt_host() {
        // 官方登录态下，仅 chatgpt.com 及其子域判 true
        let official = true;
        assert!(upstream_supports_responses_lite(
            Some("https://chatgpt.com/backend-api/codex"),
            official
        ));
        assert!(upstream_supports_responses_lite(
            Some("https://ab.chatgpt.com/backend-api/codex"),
            official
        ));
        assert!(upstream_supports_responses_lite(
            Some("HTTPS://ChatGPT.com"),
            official
        ));
        assert!(upstream_supports_responses_lite(
            Some("https://chatgpt.com:443/backend-api"),
            official
        ));
        assert!(!upstream_supports_responses_lite(
            Some("https://api.openai.com/v1"),
            official
        ));
        assert!(!upstream_supports_responses_lite(
            Some("https://chatgpt.com.evil.example/v1"),
            official
        ));
        assert!(!upstream_supports_responses_lite(
            Some("https://notchatgpt.com/v1"),
            official
        ));
        assert!(!upstream_supports_responses_lite(
            Some("https://relay.example/v1"),
            official
        ));
        // 非官方登录态一律 false
        assert!(!upstream_supports_responses_lite(Some("https://chatgpt.com"), false));
        // URL 缺失/无 scheme 一律 false（安全默认，不猜测）
        assert!(!upstream_supports_responses_lite(None, official));
        assert!(!upstream_supports_responses_lite(Some(""), official));
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

#[cfg(test)]
mod window_value_parsing_tests {
    use super::parse_optional_positive_u64;

    #[test]
    fn accepts_plain_digits() {
        assert_eq!(parse_optional_positive_u64("1050000", "上下文大小").unwrap(), Some(1_050_000));
        assert_eq!(parse_optional_positive_u64(" 1050000 ", "上下文大小").unwrap(), Some(1_050_000));
    }

    #[test]
    fn accepts_thousand_separators() {
        // 用户从文档复制 `1,050,000` 时，此前会判整次保存失败（issue #2590）。
        assert_eq!(parse_optional_positive_u64("1,050,000", "上下文大小").unwrap(), Some(1_050_000));
        assert_eq!(parse_optional_positive_u64("1_050_000", "上下文大小").unwrap(), Some(1_050_000));
        assert_eq!(
            parse_optional_positive_u64("1，050，000", "上下文大小").unwrap(),
            Some(1_050_000),
            "全角逗号同样应被接受"
        );
    }

    #[test]
    fn still_rejects_non_numeric_and_zero() {
        assert!(parse_optional_positive_u64("abc", "上下文大小").is_err());
        assert!(parse_optional_positive_u64("0", "上下文大小").is_err());
        assert!(parse_optional_positive_u64("1.05M", "上下文大小").is_err());
        assert_eq!(parse_optional_positive_u64("   ", "上下文大小").unwrap(), None);
    }

    #[test]
    fn separator_position_is_not_validated() {
        // 逗号位置不规范（例如 1,0,5,0）仍按纯数字处理：这个字段的目标是拿到
        // 用户想要的窗口值，不是校验排版。真正的非数字才拒绝。
        assert_eq!(parse_optional_positive_u64("1,0,5,0", "上下文大小").unwrap(), Some(1_050));
    }
}
