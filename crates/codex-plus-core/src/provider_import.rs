use crate::gateway::apply::ProviderInput;
use crate::gateway::store::GatewayStore;
use crate::settings::{BackendSettings, RelayMode, RelayProfile, RelayProtocol, SettingsStore};
use anyhow::Context;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderImportRequest {
    pub name: String,
    pub base_url: String,
    pub api_key: String,
    #[serde(default = "default_wire_api")]
    pub wire_api: String,
    #[serde(default = "default_relay_mode")]
    pub relay_mode: String,
    #[serde(default)]
    pub config_contents: String,
    #[serde(default)]
    pub auth_contents: String,
    /// 导入到哪里：空为直连模式供应商（原有行为），`pool` 为网关供应商池。
    #[serde(default)]
    pub target: String,
}

/// 导入目标：网关供应商池。
pub const IMPORT_TARGET_POOL: &str = "pool";

/// 唯一接受的导入链接地址（查询参数之前的部分）。
pub const PROVIDER_IMPORT_URL: &str = "codexplusplus://v1/import/provider";

/// 待确认导入和它的指纹。指纹是 pending 文件内容的 SHA-256：弹窗展示时拿到，确认时带回，
/// 后端比对一致才写入，免得用户确认的是一条、实际导入的是随后被新链接覆盖的另一条。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingProviderImport {
    pub request: ProviderImportRequest,
    pub fingerprint: String,
}

/// 确认导入的结果；`to_pool` 与实际导入取自同一次读取。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmedProviderImport {
    pub result: ProviderImportResult,
    pub to_pool: bool,
}

/// 确认时 pending 已被新链接覆盖。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingProviderImportChanged;

impl std::fmt::Display for PendingProviderImportChanged {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("导入内容已变化，请重新确认")
    }
}

impl std::error::Error for PendingProviderImportChanged {}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderImportResult {
    pub imported: bool,
    pub profile_id: String,
    pub profile_name: String,
}

/// 将本机配置保存为普通供应商；后续读取不覆盖用户对该供应商的修改。
pub fn initialize_local_config_provider(
    store: &SettingsStore,
    home: &Path,
) -> anyhow::Result<BackendSettings> {
    let mut settings = store.load()?;
    if settings.local_config_provider_imported {
        return Ok(settings);
    }
    if !settings
        .relay_profiles
        .iter()
        .any(|profile| matches!(profile.name.as_str(), "系统默认" | "系统默认配置"))
    {
        let mut profile = RelayProfile {
            use_common_config: false,
            ..RelayProfile::default()
        };
        let read_local = |name: &str| -> anyhow::Result<String> {
            match std::fs::read_to_string(home.join(name)) {
                Ok(contents) => Ok(contents),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
                Err(error) => Err(error).with_context(|| format!("无法读取本机 {name}")),
            }
        };
        profile.config_contents = read_local("config.toml")?;
        profile.auth_contents = read_local("auth.json")?;
        let config: toml::Value = profile
            .config_contents
            .parse()
            .context("本机 config.toml 格式无效")?;
        let auth: serde_json::Value = if profile.auth_contents.trim().is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_str(&profile.auth_contents).context("本机 auth.json 格式无效")?
        };
        let provider_id = config
            .get("model_provider")
            .and_then(toml::Value::as_str)
            .unwrap_or("openai");
        let provider = config
            .get("model_providers")
            .and_then(|providers| providers.get(provider_id));
        let wire_api = provider
            .and_then(|value| value.get("wire_api"))
            .and_then(toml::Value::as_str);
        if matches!(wire_api, Some("chat" | "chat_completions"))
            || config
                .get("codex_plus_chat_base_url")
                .and_then(toml::Value::as_str)
                .is_some_and(|value| !value.trim().is_empty())
        {
            profile.protocol = RelayProtocol::ChatCompletions;
        }
        profile.base_url = crate::relay_config::relay_profile_base_url(&profile);
        profile.upstream_base_url = profile.base_url.clone();
        profile.relay_mode = RelayMode::PureApi;
        profile.api_key = crate::relay_config::relay_profile_api_key(&profile);
        let api_auth = auth
            .get("OPENAI_API_KEY")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|key| !key.trim().is_empty());
        profile.no_auth = !api_auth
            && profile.api_key.is_empty()
            && !profile.base_url.is_empty()
            && provider
                .and_then(|value| value.get("requires_openai_auth"))
                .and_then(toml::Value::as_bool)
                == Some(false);
        if !api_auth && profile.api_key.is_empty() && !profile.no_auth {
            profile.relay_mode = RelayMode::Official;
            profile.official_mix_api_key = !profile.base_url.is_empty();
        }

        // 本机文件指向工具的协议代理时，复用当前供应商保存的真实上游和凭证。
        // 直接把代理地址当上游保存，会在再次切换后形成回环。
        if settings.relay_profiles_enabled
            && crate::relay_config::responses_proxy_configured_in_home(home)
        {
            profile = settings.active_relay_profile();
            profile.use_common_config = false;
            crate::relay_config::backfill_relay_profile_from_home_with_common(
                home,
                &mut profile,
                &mut String::new(),
            )?;
        } else {
            crate::relay_config::backfill_relay_profile_from_home_with_common_and_policy(
                home,
                &mut profile,
                &mut String::new(),
                crate::relay_config::RelayBackfillPolicy::AdoptLiveIdentity,
            )?;
        }
        let aggregate = if profile.relay_mode == RelayMode::Aggregate {
            Some(
                settings
                    .aggregate_relay_profiles
                    .iter()
                    .find(|aggregate| aggregate.id == profile.id)
                    .cloned()
                    .context("当前聚合供应商缺少成员定义，无法导入本机配置")?,
            )
        } else {
            None
        };
        profile.id = format!("relay-{}", uuid::Uuid::new_v4());
        profile.name = "系统默认".to_string();
        profile.model_list = profile.model.clone();
        if let Some(mut aggregate) = aggregate {
            aggregate.id = profile.id.clone();
            aggregate.name = profile.name.clone();
            settings.aggregate_relay_profiles.push(aggregate);
        }

        if settings.relay_profiles == crate::settings::default_relay_profiles() {
            let legacy = settings.active_relay_profile();
            if legacy == RelayProfile::default() {
                settings.relay_profiles.clear();
            } else {
                settings.relay_profiles[0] = legacy;
            }
        }
        if settings.relay_profiles.is_empty() {
            settings.active_relay_id = profile.id.clone();
        }
        settings.relay_profiles.insert(0, profile);
    }
    settings.local_config_provider_imported = true;
    store.save(&settings)?;
    store.load()
}

pub fn import_provider_from_url(url: &str) -> anyhow::Result<ProviderImportResult> {
    let request = request_from_url(url)?;
    import_provider(request)
}

pub fn save_pending_provider_import_from_url(url: &str) -> anyhow::Result<ProviderImportRequest> {
    let request = request_from_url(url)?;
    save_pending_provider_import(&request)?;
    Ok(request)
}

pub fn save_pending_provider_import(request: &ProviderImportRequest) -> anyhow::Result<()> {
    save_pending_provider_import_at(
        &crate::paths::default_pending_provider_import_path(),
        request,
    )
}

pub fn load_pending_provider_import() -> anyhow::Result<Option<ProviderImportRequest>> {
    load_pending_provider_import_at(&crate::paths::default_pending_provider_import_path())
}

pub fn clear_pending_provider_import() -> anyhow::Result<()> {
    clear_pending_provider_import_at(&crate::paths::default_pending_provider_import_path())
}

pub fn load_pending_provider_import_with_fingerprint()
-> anyhow::Result<Option<PendingProviderImport>> {
    load_pending_provider_import_with_fingerprint_at(
        &crate::paths::default_pending_provider_import_path(),
    )
}

/// 读一次 pending 文件，同时给出内容和指纹。
pub fn load_pending_provider_import_with_fingerprint_at(
    path: &Path,
) -> anyhow::Result<Option<PendingProviderImport>> {
    let contents = match std::fs::read(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| {
                format!("读取待确认供应商导入失败：{}", path.to_string_lossy())
            });
        }
    };
    let request = serde_json::from_slice(&contents).context("待确认供应商导入内容无效")?;
    Ok(Some(PendingProviderImport {
        request,
        fingerprint: pending_fingerprint(&contents),
    }))
}

fn pending_fingerprint(contents: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(contents))
}

/// 确认导入：只有 pending 的指纹与弹窗展示时一致才写入；导入目标与内容来自同一次读取。
pub fn confirm_pending_provider_import_matching(
    fingerprint: &str,
) -> anyhow::Result<Option<ConfirmedProviderImport>> {
    confirm_pending_provider_import_matching_at(
        &crate::paths::default_pending_provider_import_path(),
        fingerprint,
        SettingsStore::default(),
        &GatewayStore::default(),
    )
}

pub fn confirm_pending_provider_import_matching_at(
    path: &Path,
    fingerprint: &str,
    settings: SettingsStore,
    gateway: &GatewayStore,
) -> anyhow::Result<Option<ConfirmedProviderImport>> {
    let Some(pending) = load_pending_provider_import_with_fingerprint_at(path)? else {
        return Ok(None);
    };
    if pending.fingerprint != fingerprint.trim() {
        return Err(PendingProviderImportChanged.into());
    }
    let to_pool = is_pool_import(&pending.request);
    let result = if to_pool {
        import_provider_to_pool(pending.request, gateway)?
    } else {
        import_provider_with_store(pending.request, settings)?
    };
    // 导入期间又来了新链接时留着它，等界面轮询到后再让用户确认。
    clear_pending_provider_import_if_unchanged_at(path, &pending.fingerprint)?;
    Ok(Some(ConfirmedProviderImport { result, to_pool }))
}

/// 取消导入：只删用户看到的那一条；已被新链接覆盖时保留并返回新的那条。
pub fn dismiss_pending_provider_import_matching(
    fingerprint: Option<&str>,
) -> anyhow::Result<Option<PendingProviderImport>> {
    dismiss_pending_provider_import_matching_at(
        &crate::paths::default_pending_provider_import_path(),
        fingerprint,
    )
}

pub fn dismiss_pending_provider_import_matching_at(
    path: &Path,
    fingerprint: Option<&str>,
) -> anyhow::Result<Option<PendingProviderImport>> {
    let Some(fingerprint) = fingerprint.map(str::trim).filter(|value| !value.is_empty()) else {
        clear_pending_provider_import_at(path)?;
        return Ok(None);
    };
    clear_pending_provider_import_if_unchanged_at(path, fingerprint)?;
    load_pending_provider_import_with_fingerprint_at(path)
}

fn clear_pending_provider_import_if_unchanged_at(
    path: &Path,
    fingerprint: &str,
) -> anyhow::Result<()> {
    let current = match std::fs::read(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error).with_context(|| {
                format!("读取待确认供应商导入失败：{}", path.to_string_lossy())
            });
        }
    };
    if pending_fingerprint(&current) == fingerprint {
        clear_pending_provider_import_at(path)?;
    }
    Ok(())
}

/// 导入链接带 `target=pool` 时，确认后加进网关供应商池而不是直连模式。
pub fn is_pool_import(request: &ProviderImportRequest) -> bool {
    request.target == IMPORT_TARGET_POOL
}

pub fn confirm_pending_provider_import_to_pool_at(
    path: &Path,
    store: &GatewayStore,
) -> anyhow::Result<ProviderImportResult> {
    let request = load_pending_provider_import_at(path)?.context("没有待确认的供应商导入")?;
    let result = import_provider_to_pool(request, store)?;
    clear_pending_provider_import_at(path)?;
    Ok(result)
}

/// 把导入请求加进网关供应商池。同一地址下已有相同 Key 的供应商时不重复添加。
/// 模型列表留空，由调用方随后从上游获取。
pub fn import_provider_to_pool(
    request: ProviderImportRequest,
    store: &GatewayStore,
) -> anyhow::Result<ProviderImportResult> {
    let request = normalize_request(request)?;
    let gateway = store.load()?;
    if let Some(existing) = gateway.providers.iter().find(|provider| {
        let relay = &provider.relay;
        relay.upstream_base_url.trim_end_matches('/') == request.base_url
            && (relay.api_key == request.api_key
                || relay.api_keys.iter().any(|key| key.api_key == request.api_key))
    }) {
        return Ok(ProviderImportResult {
            imported: false,
            profile_id: existing.relay.id.clone(),
            profile_name: existing.relay.name.clone(),
        });
    }
    let input = ProviderInput {
        name: request.name.clone(),
        base_url: request.base_url.clone(),
        protocol: relay_protocol(&request.wire_api),
        keys: vec![request.api_key.clone()],
        ..ProviderInput::default()
    };
    let (_, id) = crate::gateway::apply::create_provider(store, &input, "import:url", None)?;
    Ok(ProviderImportResult {
        imported: true,
        profile_id: id,
        profile_name: request.name,
    })
}

pub fn save_pending_provider_import_at(
    path: &Path,
    request: &ProviderImportRequest,
) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut pending = request.clone();
    pending.config_contents.clear();
    pending.auth_contents.clear();
    let contents = serde_json::to_string_pretty(&pending)?;
    std::fs::write(path, format!("{contents}\n"))?;
    Ok(())
}

pub fn load_pending_provider_import_at(
    path: &Path,
) -> anyhow::Result<Option<ProviderImportRequest>> {
    if !path.exists() {
        return Ok(None);
    }
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("读取待确认供应商导入失败：{}", path.to_string_lossy()))?;
    let request = serde_json::from_str(&contents).context("待确认供应商导入内容无效")?;
    Ok(Some(request))
}

pub fn clear_pending_provider_import_at(path: &Path) -> anyhow::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error)
            .with_context(|| format!("清理待确认供应商导入失败：{}", path.to_string_lossy())),
    }
}

pub fn confirm_pending_provider_import_at(
    path: &Path,
    store: SettingsStore,
) -> anyhow::Result<ProviderImportResult> {
    let request = load_pending_provider_import_at(path)?.context("没有待确认的供应商导入")?;
    let result = import_provider_with_store(request, store)?;
    clear_pending_provider_import_at(path)?;
    Ok(result)
}

pub fn import_provider(request: ProviderImportRequest) -> anyhow::Result<ProviderImportResult> {
    import_provider_with_store(request, SettingsStore::default())
}

pub fn import_provider_with_store(
    request: ProviderImportRequest,
    store: SettingsStore,
) -> anyhow::Result<ProviderImportResult> {
    let request = normalize_request(request)?;
    let mut settings = store.load().unwrap_or_default();
    let identity = provider_identity(&request.name, &request.base_url);
    if let Some(existing) = settings
        .relay_profiles
        .iter()
        .find(|profile| provider_identity(&profile.name, &profile.upstream_base_url) == identity)
    {
        return Ok(ProviderImportResult {
            imported: false,
            profile_id: existing.id.clone(),
            profile_name: existing.name.clone(),
        });
    }

    let existing_ids = settings
        .relay_profiles
        .iter()
        .map(|profile| profile.id.clone())
        .collect::<Vec<_>>();
    let profile = relay_profile_from_request(&request, &existing_ids);
    let result = ProviderImportResult {
        imported: true,
        profile_id: profile.id.clone(),
        profile_name: profile.name.clone(),
    };
    settings.relay_profiles.push(profile);
    settings.active_relay_id = result.profile_id.clone();
    store.save(&settings)?;
    Ok(result)
}

pub fn request_from_url(url: &str) -> anyhow::Result<ProviderImportRequest> {
    let (location, query) = url.trim().split_once('?').context("导入链接缺少查询参数")?;
    if location.trim_end_matches('/') != PROVIDER_IMPORT_URL {
        anyhow::bail!("只支持 {PROVIDER_IMPORT_URL} 导入链接");
    }
    let mut values = std::collections::BTreeMap::<String, String>::new();
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        values.insert(percent_decode(key), percent_decode(value));
    }
    let base_url = required_value(&values, "baseUrl")?;
    validate_import_base_url(&base_url)?;
    Ok(ProviderImportRequest {
        name: required_value(&values, "name")?,
        base_url,
        api_key: required_value(&values, "apiKey")?,
        wire_api: values
            .get("wireApi")
            .cloned()
            .unwrap_or_else(default_wire_api),
        relay_mode: values
            .get("relayMode")
            .cloned()
            .unwrap_or_else(default_relay_mode),
        config_contents: String::new(),
        auth_contents: String::new(),
        target: normalize_target(values.get("target").map(String::as_str).unwrap_or_default()),
    })
}

/// 导入链接里的 Base URL 只接受 http / https，其它协议（file、javascript 等）一律拒绝。
fn validate_import_base_url(value: &str) -> anyhow::Result<()> {
    let parsed = reqwest::Url::parse(value.trim())
        .map_err(|_| anyhow::anyhow!("导入链接的 Base URL 不是有效的网址：{value}"))?;
    match parsed.scheme() {
        "http" | "https" => {}
        other => anyhow::bail!(
            "导入链接的 Base URL 只支持 http:// 或 https:// 地址，不支持 {other}://"
        ),
    }
    if parsed.host_str().is_none_or(str::is_empty) {
        anyhow::bail!("导入链接的 Base URL 缺少主机名");
    }
    Ok(())
}

/// 导入目标只认识网关供应商池，其余（含空）都按直连模式处理。
fn normalize_target(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "pool" | "gateway" | "gateway-pool" => IMPORT_TARGET_POOL.to_string(),
        _ => String::new(),
    }
}

fn relay_profile_from_request(
    request: &ProviderImportRequest,
    existing_ids: &[String],
) -> RelayProfile {
    RelayProfile {
        id: unique_profile_id(
            &format!("import-{}", sanitize_id(&request.name)),
            existing_ids,
        ),
        name: request.name.clone(),
        model: String::new(),
        base_url: request.base_url.clone(),
        upstream_base_url: request.base_url.clone(),
        api_key: request.api_key.clone(),
        api_keys: vec![crate::settings::RelayApiKey {
            id: "default".to_string(),
            name: "默认".to_string(),
            api_key: request.api_key.clone(),
        }],
        active_api_key_id: "default".to_string(),
        protocol: relay_protocol(&request.wire_api),
        relay_mode: relay_mode(&request.relay_mode),
        official_mix_api_key: false,
        no_auth: false,
        hide_official_usage_alert: false,
        test_model: String::new(),
        config_contents: request.config_contents.clone(),
        auth_contents: request.auth_contents.clone(),
        use_common_config: true,
        context_window: String::new(),
        auto_compact_limit: String::new(),
        model_insert_mode: Default::default(),
        model_list: String::new(),
        model_windows: String::new(),
        model_auto_compact: String::new(),
        model_metadata: String::new(),
        model_vlm: String::new(),
        vlm_api_key: String::new(),
        vlm_model: String::new(),
        vlm_base_url: String::new(),
        user_agent: String::new(),
        sub2api_enabled: false,
        sub2api_multiplier: String::new(),
        model_routes: Vec::new(),
        custom_headers: Vec::new(),
        standard_openai_protocol: false,
        web_search_history_compat: false,
        local_compaction_prompt_at_end: false,
        trust_upstream_catalog_lite: false,
        strict_responses_reasoning: false,
        strip_image_generation_tool: false,
        auto_review_model: String::new(),
        multi_agent: String::new(),
        anthropic_auth_header: String::new(),
        anthropic_max_tokens: 0,
        anthropic_prompt_cache: true,
        anthropic_impersonate_claude_code: false,
        anthropic_thinking_mode: String::new(),
        gateway_proxy: None,
    }
}

/// 网关「导入到供应商池」用：按地址、Key、协议拼出与导入链接同形的供应商
/// （config.toml / auth.json 快照齐全，切到它时 Codex 也能直接用）。
pub fn pool_relay_profile(
    name: &str,
    base_url: &str,
    api_key: &str,
    protocol: RelayProtocol,
    default_model: &str,
    model_list: &str,
    existing_ids: &[String],
) -> anyhow::Result<RelayProfile> {
    let wire_api = match protocol {
        RelayProtocol::Responses => "responses",
        RelayProtocol::ChatCompletions => "chat",
        RelayProtocol::AnthropicMessages => "anthropic",
    };
    let request = normalize_request(ProviderImportRequest {
        name: name.to_string(),
        base_url: base_url.to_string(),
        api_key: api_key.to_string(),
        wire_api: wire_api.to_string(),
        relay_mode: default_relay_mode(),
        config_contents: String::new(),
        auth_contents: String::new(),
        target: String::new(),
    })?;
    let mut profile = relay_profile_from_request(&request, existing_ids);
    let default_model = default_model.trim();
    if !default_model.is_empty() {
        profile.config_contents = format!(
            "model = \"{}\"\n{}",
            toml_string(default_model),
            profile.config_contents
        );
        profile.model = default_model.to_string();
    }
    profile.model_list = model_list.trim().to_string();
    Ok(profile)
}

fn normalize_request(mut request: ProviderImportRequest) -> anyhow::Result<ProviderImportRequest> {
    request.name = request.name.trim().to_string();
    request.base_url = request.base_url.trim().trim_end_matches('/').to_string();
    request.api_key = request.api_key.trim().to_string();
    request.wire_api = request.wire_api.trim().to_ascii_lowercase();
    request.relay_mode = request.relay_mode.trim().to_ascii_lowercase();
    if request.name.is_empty() {
        anyhow::bail!("供应商名称为空");
    }
    if request.base_url.is_empty() {
        anyhow::bail!("Base URL 为空");
    }
    if request.api_key.is_empty() {
        anyhow::bail!("API Key 为空");
    }
    request.config_contents = build_config_toml(
        &request.base_url,
        &request.api_key,
        relay_protocol(&request.wire_api),
    );
    request.auth_contents = build_auth_json(&request.api_key);
    Ok(request)
}

fn relay_protocol(value: &str) -> RelayProtocol {
    match value.trim().to_ascii_lowercase().as_str() {
        "chat" | "chat_completions" | "chat-completions" => RelayProtocol::ChatCompletions,
        "anthropic" | "anthropic_messages" | "anthropic-messages" | "messages" => {
            RelayProtocol::AnthropicMessages
        }
        _ => RelayProtocol::Responses,
    }
}

fn relay_mode(value: &str) -> RelayMode {
    match value.trim().to_ascii_lowercase().as_str() {
        "official" => RelayMode::Official,
        "mixedapi" | "mixed-api" | "mixed_api" => RelayMode::MixedApi,
        "aggregate" => RelayMode::Aggregate,
        _ => RelayMode::PureApi,
    }
}

fn build_config_toml(base_url: &str, api_key: &str, protocol: RelayProtocol) -> String {
    // Codex 26.901+ 不再支持 wire_api = "chat"（openai/codex discussion #7782），
    // 写出 "chat" 会导致整份 config.toml 无效并回退默认。Chat Completions 上游
    // 一律经由本地协议代理转换，对 Codex 始终暴露 "responses"。
    let _ = protocol;
    let wire_api = "responses";
    [
        "model_provider = \"CodexPlusPlus\"".to_string(),
        String::new(),
        "[model_providers.CodexPlusPlus]".to_string(),
        "name = \"CodexPlusPlus\"".to_string(),
        format!("wire_api = \"{wire_api}\""),
        "requires_openai_auth = true".to_string(),
        format!("base_url = \"{}\"", toml_string(base_url)),
        format!("experimental_bearer_token = \"{}\"", toml_string(api_key)),
        String::new(),
    ]
    .join("\n")
}

fn build_auth_json(api_key: &str) -> String {
    format!(
        "{}\n",
        serde_json::to_string_pretty(&serde_json::json!({ "OPENAI_API_KEY": api_key }))
            .unwrap_or_else(|_| "{\"OPENAI_API_KEY\":\"\"}".to_string())
    )
}

fn required_value(
    values: &std::collections::BTreeMap<String, String>,
    key: &str,
) -> anyhow::Result<String> {
    values
        .get(key)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .with_context(|| format!("导入链接缺少 {key}"))
}

fn percent_decode(value: &str) -> String {
    let value = value.replace('+', " ");
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let Ok(hex) = u8::from_str_radix(&value[index + 1..index + 3], 16) {
                output.push(hex);
                index += 3;
                continue;
            }
        }
        output.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&output).to_string()
}

fn provider_identity(name: &str, base_url: &str) -> String {
    format!(
        "{}\n{}",
        name.trim().to_ascii_lowercase(),
        base_url.trim().trim_end_matches('/').to_ascii_lowercase()
    )
}

fn sanitize_id(value: &str) -> String {
    let mut result = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            result.push(ch.to_ascii_lowercase());
        } else if !result.ends_with('-') {
            result.push('-');
        }
    }
    let result = result.trim_matches('-').to_string();
    if result.is_empty() {
        "provider".to_string()
    } else {
        result
    }
}

fn unique_profile_id(base: &str, existing_ids: &[String]) -> String {
    if !existing_ids.iter().any(|id| id == base) {
        return base.to_string();
    }
    let mut index = 2;
    loop {
        let candidate = format!("{base}-{index}");
        if !existing_ids.iter().any(|id| id == &candidate) {
            return candidate;
        }
        index += 1;
    }
}

fn toml_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn default_wire_api() -> String {
    "responses".to_string()
}

fn default_relay_mode() -> String {
    "pureApi".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_codexplusplus_provider_url() {
        let url = "codexplusplus://v1/import/provider?resource=provider&name=JOJO%20Code&baseUrl=https%3A%2F%2Fjojocode.com%2Fv1&apiKey=sk-test&wireApi=responses&relayMode=pureApi&configContents=bW9kZWxfcHJvdmlkZXIgPSAiQ29kZXhQbHVzUGx1cyIK&authContents=eyJPUEVOQUlfQVBJX0tFWSI6InNrLXRlc3QifQo%3D";

        let request = request_from_url(url).unwrap();

        assert_eq!(request.name, "JOJO Code");
        assert_eq!(request.base_url, "https://jojocode.com/v1");
        assert_eq!(request.api_key, "sk-test");
        assert_eq!(request.wire_api, "responses");
        assert_eq!(request.relay_mode, "pureApi");
        assert!(request.config_contents.is_empty());
        assert!(request.auth_contents.is_empty());
    }

    #[test]
    fn parses_pool_target_from_url() {
        let pool = request_from_url(
            "codexplusplus://v1/import/provider?name=JOJO%20Code&baseUrl=https%3A%2F%2Fmax2.jojocode.com&apiKey=sk-test&wireApi=anthropic&target=pool",
        )
        .unwrap();
        let unknown = request_from_url(
            "codexplusplus://v1/import/provider?name=X&baseUrl=https%3A%2F%2Fa.example%2Fv1&apiKey=sk-test&target=whatever",
        )
        .unwrap();
        let legacy = request_from_url(
            "codexplusplus://v1/import/provider?name=X&baseUrl=https%3A%2F%2Fa.example%2Fv1&apiKey=sk-test",
        )
        .unwrap();

        assert!(is_pool_import(&pool));
        assert_eq!(pool.wire_api, "anthropic");
        assert!(!is_pool_import(&unknown));
        assert!(!is_pool_import(&legacy));
    }

    #[test]
    fn imports_provider_to_gateway_pool_once() {
        let dir = tempfile::tempdir().unwrap();
        let store = GatewayStore::at(dir.path().join("gateway.json"));
        let request = request_from_url(
            "codexplusplus://v1/import/provider?name=JOJO%20Code%20Claude&baseUrl=https%3A%2F%2Fmax2.jojocode.com%2F&apiKey=sk-claude&wireApi=anthropic&target=pool",
        )
        .unwrap();

        let first = import_provider_to_pool(request.clone(), &store).unwrap();
        let second = import_provider_to_pool(request, &store).unwrap();
        let gateway = store.load().unwrap();

        assert!(first.imported);
        assert!(!second.imported);
        assert_eq!(first.profile_id, second.profile_id);
        assert_eq!(gateway.providers.len(), 1);
        let relay = &gateway.providers[0].relay;
        assert_eq!(relay.protocol, RelayProtocol::AnthropicMessages);
        assert_eq!(relay.upstream_base_url, "https://max2.jojocode.com");
        assert_eq!(relay.api_key, "sk-claude");
        assert_eq!(gateway.providers[0].source, "import:url");
    }

    #[test]
    fn confirms_pending_pool_import_and_keeps_direct_profiles_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let pending_path = dir.path().join("pending-provider-import.json");
        let gateway = GatewayStore::at(dir.path().join("gateway.json"));
        let mut request = request_from_url(
            "codexplusplus://v1/import/provider?name=JOJO%20Code&baseUrl=https%3A%2F%2Fmax2.jojocode.com%2Fv1&apiKey=sk-test&target=pool",
        )
        .unwrap();
        request.config_contents = "notify = [\"calc\"]\n".to_string();
        save_pending_provider_import_at(&pending_path, &request).unwrap();

        let result = confirm_pending_provider_import_to_pool_at(&pending_path, &gateway).unwrap();

        assert!(result.imported);
        assert_eq!(gateway.load().unwrap().providers.len(), 1);
        assert!(!pending_path.exists());
    }

    #[test]
    fn rejects_other_import_paths_and_non_http_base_urls() {
        let path_error = request_from_url(
            "codexplusplus://v1/import/skin?name=X&baseUrl=https%3A%2F%2Fa.example&apiKey=sk-test",
        )
        .unwrap_err()
        .to_string();
        assert!(path_error.contains("codexplusplus://v1/import/provider"), "{path_error}");
        assert!(
            request_from_url(
                "codexplusplus://v2/import/provider?name=X&baseUrl=https%3A%2F%2Fa.example&apiKey=sk-test"
            )
            .is_err()
        );
        for base_url in ["file%3A%2F%2F%2Fetc%2Fpasswd", "javascript%3Aalert(1)", "ftp%3A%2F%2Fa.example", "not-a-url"] {
            let error = request_from_url(&format!(
                "codexplusplus://v1/import/provider?name=X&baseUrl={base_url}&apiKey=sk-test"
            ))
            .unwrap_err()
            .to_string();
            assert!(error.contains("Base URL"), "{base_url}: {error}");
        }
        // 末尾带斜杠、http 地址照常接受。
        let request = request_from_url(
            "codexplusplus://v1/import/provider/?name=X&baseUrl=http%3A%2F%2F127.0.0.1%3A3000%2Fv1&apiKey=sk-test",
        )
        .unwrap();
        assert_eq!(request.base_url, "http://127.0.0.1:3000/v1");
    }

    fn save_pending_url(path: &Path, url: &str) -> String {
        save_pending_provider_import_at(path, &request_from_url(url).unwrap()).unwrap();
        load_pending_provider_import_with_fingerprint_at(path)
            .unwrap()
            .unwrap()
            .fingerprint
    }

    #[test]
    fn confirm_rejects_pending_import_replaced_after_it_was_shown() {
        let dir = tempfile::tempdir().unwrap();
        let pending_path = dir.path().join("pending-provider-import.json");
        let settings = SettingsStore::new(dir.path().join("settings.json"));
        let gateway = GatewayStore::at(dir.path().join("gateway.json"));
        let shown = save_pending_url(
            &pending_path,
            "codexplusplus://v1/import/provider?name=Shown&baseUrl=https%3A%2F%2Fshown.example&apiKey=sk-shown",
        );
        // 弹窗展示后又来了一条新链接（导入到供应商池），覆盖了 pending。
        let replaced = save_pending_url(
            &pending_path,
            "codexplusplus://v1/import/provider?name=Other&baseUrl=https%3A%2F%2Fother.example&apiKey=sk-other&target=pool",
        );
        assert_ne!(shown, replaced);

        let error = confirm_pending_provider_import_matching_at(
            &pending_path,
            &shown,
            settings.clone(),
            &gateway,
        )
        .unwrap_err();

        assert!(error.downcast_ref::<PendingProviderImportChanged>().is_some());
        assert_eq!(error.to_string(), "导入内容已变化，请重新确认");
        // 什么都没写，新的那条仍待确认。
        assert!(gateway.load().unwrap().providers.is_empty());
        assert!(
            !settings
                .load()
                .unwrap_or_default()
                .relay_profiles
                .iter()
                .any(|profile| profile.name == "Shown" || profile.name == "Other")
        );
        assert!(pending_path.exists());

        // 带上新指纹再确认：按同一次读取的 target=pool 导入到供应商池。
        let confirmed = confirm_pending_provider_import_matching_at(
            &pending_path,
            &replaced,
            settings,
            &gateway,
        )
        .unwrap()
        .unwrap();
        assert!(confirmed.to_pool);
        assert!(confirmed.result.imported);
        assert_eq!(gateway.load().unwrap().providers.len(), 1);
        assert!(!pending_path.exists());
    }

    #[test]
    fn confirm_matching_fingerprint_imports_direct_profile() {
        let dir = tempfile::tempdir().unwrap();
        let pending_path = dir.path().join("pending-provider-import.json");
        let settings = SettingsStore::new(dir.path().join("settings.json"));
        let gateway = GatewayStore::at(dir.path().join("gateway.json"));
        let fingerprint = save_pending_url(
            &pending_path,
            "codexplusplus://v1/import/provider?name=Direct&baseUrl=https%3A%2F%2Fdirect.example%2Fv1&apiKey=sk-direct",
        );

        let confirmed = confirm_pending_provider_import_matching_at(
            &pending_path,
            &fingerprint,
            settings.clone(),
            &gateway,
        )
        .unwrap()
        .unwrap();

        assert!(!confirmed.to_pool);
        assert!(confirmed.result.imported);
        assert!(settings.load().unwrap().relay_profiles.iter().any(|profile| profile.name == "Direct"));
        assert!(gateway.load().unwrap_or_default().providers.is_empty());
        assert!(!pending_path.exists());
        // 已经没有待确认导入时返回 None。
        assert!(
            confirm_pending_provider_import_matching_at(&pending_path, &fingerprint, settings, &gateway)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn dismiss_keeps_pending_import_replaced_after_it_was_shown() {
        let dir = tempfile::tempdir().unwrap();
        let pending_path = dir.path().join("pending-provider-import.json");
        let shown = save_pending_url(
            &pending_path,
            "codexplusplus://v1/import/provider?name=Shown&baseUrl=https%3A%2F%2Fshown.example&apiKey=sk-shown",
        );
        let replaced = save_pending_url(
            &pending_path,
            "codexplusplus://v1/import/provider?name=Other&baseUrl=https%3A%2F%2Fother.example&apiKey=sk-other",
        );

        let remaining = dismiss_pending_provider_import_matching_at(&pending_path, Some(&shown))
            .unwrap()
            .unwrap();
        assert_eq!(remaining.fingerprint, replaced);
        assert_eq!(remaining.request.name, "Other");

        assert!(
            dismiss_pending_provider_import_matching_at(&pending_path, Some(&replaced))
                .unwrap()
                .is_none()
        );
        assert!(!pending_path.exists());
    }

    #[test]
    fn url_import_discards_embedded_config_and_auth_payloads() {
        use base64::Engine as _;

        let dangerous_config = "notify = [\"powershell\", \"-Command\", \"calc\"]\n[mcp_servers.evil]\ncommand = \"cmd\"\n";
        let dangerous_auth = r#"{"OPENAI_API_KEY":"sk-test","exec":"calc"}"#;
        let config = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(dangerous_config);
        let auth = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(dangerous_auth);
        let url = format!(
            "codexplusplus://v1/import/provider?name=Unsafe&baseUrl=https%3A%2F%2Frelay.example%2Fv1&apiKey=sk-test&configContents={config}&authContents={auth}"
        );

        let request = request_from_url(&url).unwrap();

        assert!(request.config_contents.is_empty());
        assert!(request.auth_contents.is_empty());
    }

    #[test]
    fn imports_provider_once_and_selects_it() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(dir.path().join("settings.json"));
        let request = ProviderImportRequest {
            name: "JOJO Code".to_string(),
            base_url: "https://jojocode.com/v1/".to_string(),
            api_key: "sk-test".to_string(),
            wire_api: "responses".to_string(),
            relay_mode: "pureApi".to_string(),
            config_contents:
                "notify = [\"powershell\", \"-Command\", \"calc\"]\n[mcp_servers.evil]\ncommand = \"cmd\"\n"
                    .to_string(),
            auth_contents: r#"{"OPENAI_API_KEY":"sk-test","exec":"calc"}"#.to_string(),
            target: String::new(),
        };

        let first = import_provider_with_store(request.clone(), store.clone()).unwrap();
        let second = import_provider_with_store(request, store.clone()).unwrap();
        let settings = store.load().unwrap();

        assert!(first.imported);
        assert!(!second.imported);
        assert_eq!(first.profile_id, second.profile_id);
        assert_eq!(settings.active_relay_id, first.profile_id);
        assert_eq!(settings.relay_profiles.len(), 2);
        assert_eq!(
            settings.relay_profiles[1].protocol,
            RelayProtocol::Responses
        );
        assert_eq!(settings.relay_profiles[1].relay_mode, RelayMode::PureApi);
        assert_eq!(
            settings.relay_profiles[1].upstream_base_url,
            "https://jojocode.com/v1"
        );
        assert!(
            !settings.relay_profiles[1]
                .config_contents
                .contains("notify")
        );
        assert!(
            !settings.relay_profiles[1]
                .config_contents
                .contains("mcp_servers")
        );
        assert!(!settings.relay_profiles[1].auth_contents.contains("exec"));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&settings.relay_profiles[1].auth_contents)
                .unwrap(),
            serde_json::json!({ "OPENAI_API_KEY": "sk-test" })
        );
    }

    #[test]
    fn pending_provider_import_round_trips_and_clears() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pending-provider-import.json");
        let request = ProviderImportRequest {
            name: "JOJO Code".to_string(),
            base_url: "https://jojocode.com/v1".to_string(),
            api_key: "sk-test".to_string(),
            wire_api: "responses".to_string(),
            relay_mode: "pureApi".to_string(),
            config_contents: "notify = [\"calc\"]\n".to_string(),
            auth_contents: r#"{"OPENAI_API_KEY":"sk-test","exec":"calc"}"#.to_string(),
            target: String::new(),
        };

        save_pending_provider_import_at(&path, &request).unwrap();
        let pending = load_pending_provider_import_at(&path).unwrap().unwrap();
        let pending_file = std::fs::read_to_string(&path).unwrap();
        clear_pending_provider_import_at(&path).unwrap();

        assert_eq!(pending.name, "JOJO Code");
        assert_eq!(pending.base_url, "https://jojocode.com/v1");
        assert!(pending.config_contents.is_empty());
        assert!(pending.auth_contents.is_empty());
        assert!(!pending_file.contains("notify"));
        assert!(!pending_file.contains("exec"));
        assert!(load_pending_provider_import_at(&path).unwrap().is_none());
    }

    #[test]
    fn confirms_pending_provider_import_and_removes_pending_file() {
        let dir = tempfile::tempdir().unwrap();
        let pending_path = dir.path().join("pending-provider-import.json");
        let store = SettingsStore::new(dir.path().join("settings.json"));
        save_pending_provider_import_at(
            &pending_path,
            &ProviderImportRequest {
                name: "JOJO Code".to_string(),
                base_url: "https://jojocode.com/v1".to_string(),
                api_key: "sk-test".to_string(),
                wire_api: "responses".to_string(),
                relay_mode: "pureApi".to_string(),
                config_contents: String::new(),
                auth_contents: String::new(),
                target: String::new(),
            },
        )
        .unwrap();

        let result = confirm_pending_provider_import_at(&pending_path, store.clone()).unwrap();
        let settings = store.load().unwrap();

        assert!(result.imported);
        assert_eq!(settings.relay_profiles.len(), 2);
        assert!(
            load_pending_provider_import_at(&pending_path)
                .unwrap()
                .is_none()
        );
    }
}
