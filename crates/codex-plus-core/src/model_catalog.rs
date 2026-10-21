use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::settings::{RelayProfile, SettingsStore};
use serde_json::{Map, Value, json};

const BASE_URL_ENV_KEYS: &[&str] = &[
    "CODEX_PLUS_OPENAI_BASE_URL",
    "CODEX_PLUS_BASE_URL",
    "OPENAI_BASE_URL",
    "OPENAI_API_BASE_URL",
    "OPENAI_API_BASE",
    "OPENAI_API_URL",
];
const API_KEY_ENV_KEYS: &[&str] = &[
    "CODEX_PLUS_OPENAI_API_KEY",
    "CODEX_PLUS_API_KEY",
    "OPENAI_API_KEY",
];

#[derive(Debug, Clone)]
struct ModelSource {
    source_id: String,
    source_type: String,
    name: String,
    base_url: String,
    api_key: String,
    /// 供应商自定义请求头（issue #1685）。非供应商来源为空。
    headers: Vec<crate::settings::RelayHeaderKeyValue>,
}

#[derive(Debug, Default)]
struct CodexConfig {
    root: HashMap<String, String>,
    profiles: HashMap<String, HashMap<String, String>>,
    model_providers: HashMap<String, HashMap<String, String>>,
}

// sources 含凭据，不派生 Debug，避免断言或诊断意外输出密钥。
struct CodexCatalogSnapshot {
    config_path: PathBuf,
    config: CodexConfig,
    effective: HashMap<String, String>,
    error: Option<String>,
    sources: Vec<ModelSource>,
    catalog_models: Vec<String>,
    catalog_status: Option<Value>,
}

enum CatalogReadSnapshot {
    Ready(Value),
    Fallback(CodexCatalogSnapshot),
}

pub async fn read_codex_model_catalog() -> Value {
    let home = codex_home_dir();
    let env = std::env::vars_os()
        .filter_map(|(name, value)| {
            let name = name.into_string().ok()?;
            Some((name, value.to_string_lossy().into_owned()))
        })
        .collect::<HashMap<_, _>>();
    // 空目录的 fallback 也必须在同一次协调读中捕获，不能释放锁后重读 live。
    let snapshot = SettingsStore::default().with_snapshot(|settings| {
        Ok(capture_catalog_read_snapshot(&home, &env, &settings))
    });
    let snapshot = match snapshot {
        Ok(CatalogReadSnapshot::Ready(catalog)) => return catalog,
        Ok(CatalogReadSnapshot::Fallback(snapshot)) => snapshot,
        Err(error) => return catalog_read_failure(&home.join("config.toml"), Value::Null, &error),
    };
    let client = match crate::http_client::proxied_client("CodexPlusPlus/1.0") {
        Ok(client) => client,
        Err(error) => {
            return catalog_read_failure(
                &snapshot.config_path,
                service_tier_value(&snapshot.effective),
                &error,
            );
        }
    };
    read_codex_model_catalog_from_snapshot(snapshot, client).await
}

fn capture_catalog_read_snapshot(
    home: &Path,
    env: &HashMap<String, String>,
    settings: &crate::settings::BackendSettings,
) -> CatalogReadSnapshot {
    let profile = settings.active_relay_profile();
    let catalog = if let Some(aggregate) = settings.active_aggregate_relay_profile() {
        let catalog = crate::aggregate_catalog::catalog_summary(home, settings, &aggregate)
            .unwrap_or_else(|error| catalog_read_failure(&home.join("config.toml"), Value::Null, &error));
        // 聚合成员目录是权威来源，错误/空目录也不能读回旧的 live 或上游目录。
        return CatalogReadSnapshot::Ready(catalog);
    } else {
        relay_profile_model_catalog_value(home, &profile)
    };
    if catalog
        .get("models")
        .and_then(Value::as_array)
        .is_some_and(|models| !models.is_empty())
    {
        CatalogReadSnapshot::Ready(catalog)
    } else {
        CatalogReadSnapshot::Fallback(CodexCatalogSnapshot::capture(home, env))
    }
}

fn catalog_read_failure(path: &Path, service_tier: Value, error: &anyhow::Error) -> Value {
    json!({
        "status": "failed",
        "path": path.to_string_lossy(),
        "message": error.to_string(),
        "service_tier": service_tier,
        "model": "",
        "model_provider": "",
        "provider_name": "",
        "default_model": "",
        "models": [],
        "modelMetadata": {},
        "sources": [],
        "responses_api": responses_api_status("unknown", "", "")
    })
}

fn relay_profile_model_catalog_value(home: &Path, profile: &RelayProfile) -> Value {
    let models = relay_profile_model_ids(profile);
    let model = crate::relay_config::relay_profile_model(profile);
    let codex_model_provider = codex_model_provider_for_relay_profile(home, profile);
    let default_model = if models.iter().any(|item| item == &model) {
        model.clone()
    } else {
        models.first().cloned().unwrap_or_default()
    };
    let provider_name = if profile.name.trim().is_empty() {
        profile.id.trim()
    } else {
        profile.name.trim()
    };
    let model_count = models.len();
    let model_metadata = relay_profile_model_ui_metadata_map(profile, &models);
    json!({
        "status": if models.is_empty() { "not_configured" } else { "ok" },
        "path": home.join("config.toml").to_string_lossy(),
        "service_tier": config_service_tier_value(home),
        "model": model,
        "model_provider": profile.id.trim(),
        "codex_model_provider": codex_model_provider,
        "provider_name": provider_name,
        "default_model": default_model,
        "models": models,
        "modelMetadata": model_metadata,
        "sources": [
            {
                "id": format!("relay-profile:{}", profile.id),
                "type": "relay_profile_model_list",
                "name": provider_name,
                "base_url": profile.base_url.trim(),
                "status": "ok",
                "models": model_count,
                "responses_api": responses_api_status("unknown", "", "")
            }
        ],
        "responses_api": responses_api_status("unknown", "", "")
    })
}

pub fn codex_model_provider_for_relay_profile(home: &Path, profile: &RelayProfile) -> String {
    let profile_config = parse_codex_config(&profile.config_contents);
    let profile_provider = string_value(profile_config.root.get("model_provider"));
    if !profile_provider.is_empty() {
        return profile_provider;
    }

    let (live_config, _, error) = load_codex_config(&home.join("config.toml"));
    if error.is_some() {
        return String::new();
    }
    string_value(live_config.root.get("model_provider"))
}

fn relay_profile_model_ids(profile: &RelayProfile) -> Vec<String> {
    unique_strings(
        profile
            .model_list
            .split(['\r', '\n', ','])
            .chain(std::iter::once(profile.model.as_str()))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .collect(),
    )
}

/// 当前活动供应商在 Codex 模型列表里公布的模型 ID 集合，用于清理历史残留引用
/// （例如换目录后模型选择器里还留着的 `ccs-xxx/model`）。
///
/// - 模型聚合：目录里的全部 `cpx-*`——目录由 Codex++ 生成，这份集合是权威的；
///   成员未就绪导致目录算不出来时返回空集，调用方据此跳过清理，绝不误删。
/// - 官方登录且没混用 API Key：模型列表由 Codex 自己决定，返回空集表示「不要清理」。
/// - 普通 API 供应商：它的 model_list + 当前 model + 模型路由的源模型名，
///   并同时收下剥掉 `[1M]` 这类窗口后缀的形式，宁可多留不可误删。
pub fn active_published_model_ids(settings: &crate::settings::BackendSettings) -> HashSet<String> {
    if let Some(aggregate) = crate::aggregate_catalog::active_model_aggregation(settings) {
        // 配了默认供应商时，目录外的模型名仍有兜底转发，清「最近使用」会误删可用条目。
        if crate::aggregate_catalog::default_backs_unbound_models(&aggregate) {
            return HashSet::new();
        }
        return crate::aggregate_catalog::active_model_bindings(settings)
            .map(|bindings| {
                bindings
                    .into_iter()
                    .map(|binding| binding.catalog_model)
                    .collect()
            })
            .unwrap_or_default();
    }
    let profile = settings.active_relay_profile();
    if profile.relay_mode == crate::settings::RelayMode::Official && !profile.official_mix_api_key {
        return HashSet::new();
    }
    let mut ids = HashSet::new();
    for model in relay_profile_model_ids(&profile) {
        let (stripped, _) = crate::model_suffix::parse_model_suffix(&model);
        if !stripped.is_empty() {
            ids.insert(stripped);
        }
        ids.insert(model);
    }
    for route in &profile.model_routes {
        let model = route.model.trim();
        if !model.is_empty() {
            ids.insert(model.to_string());
        }
    }
    ids
}

fn model_ui_metadata_map(models: &[String]) -> Value {
    model_ui_metadata_map_with_overrides(models, "")
}

fn model_ui_metadata_map_with_overrides(models: &[String], user_json: &str) -> Value {
    let user: Value = serde_json::from_str(user_json).unwrap_or(Value::Null);
    let mut metadata = Map::new();
    for model in models {
        let slug = crate::model_suffix::parse_model_suffix(model).0;
        let user_override = user.as_object().and_then(|entries| entries.iter()
            .find(|(key, value)| value.is_object() && crate::model_suffix::parse_model_suffix(key).0.eq_ignore_ascii_case(&slug))
            .map(|(_, value)| value));
        if let Some(value) = crate::model_suffix::model_ui_metadata_with_override(model, user_override) {
            metadata.insert(model.clone(), value);
        }
    }
    Value::Object(metadata)
}

fn relay_profile_model_ui_metadata_map(profile: &RelayProfile, models: &[String]) -> Value {
    let mut metadata = model_ui_metadata_map_with_overrides(models, &profile.model_metadata)
        .as_object().cloned().unwrap_or_default();
    let windows: std::collections::BTreeMap<String, String> =
        serde_json::from_str::<Value>(&profile.model_windows)
            .ok()
            .and_then(|value| value.as_object().cloned())
            .map(|object| object.into_iter()
                .filter_map(|(key, value)| value.as_str().map(|text| (key, text.to_string())))
                .collect())
            .unwrap_or_default();
    let fallback = crate::model_suffix::parse_window_token(&profile.context_window);
    for model in models {
        let window = crate::model_suffix::lookup_model_map(&windows, model)
            .and_then(|token| crate::model_suffix::parse_window_token(token))
            .or(fallback);
        if let Some(window) = window {
            let entry = metadata.entry(model.clone()).or_insert_with(|| json!({}));
            for field in ["contextWindow", "maxContextWindow", "context_window", "max_context_window"] {
                entry[field] = json!(window);
            }
        }
    }
    Value::Object(metadata)
}

#[cfg(test)]
mod window_override_tests {
    use super::*;

    #[test]
    fn manual_window_precedes_profile_and_builtin_metadata() {
        let models = vec!["gpt-5.5".to_string(), "vendor-model".to_string()];
        let profile = RelayProfile {
            model_windows: r#"{"GPT-5.5[1M]":"1M"}"#.into(),
            context_window: "128K".into(),
            ..Default::default()
        };
        let metadata = relay_profile_model_ui_metadata_map(&profile, &models);
        for field in ["contextWindow", "maxContextWindow", "context_window", "max_context_window"] {
            assert_eq!(metadata["gpt-5.5"][field], 1_000_000);
            assert_eq!(metadata["vendor-model"][field], 128_000);
        }
        let metadata = relay_profile_model_ui_metadata_map(&RelayProfile::default(), &models);
        assert!(metadata["gpt-5.5"]["contextWindow"].as_u64().is_some());
        assert!(metadata.get("vendor-model").is_none());
    }
}

pub async fn read_codex_model_catalog_from_home(
    home: &Path,
    env: &HashMap<String, String>,
    client: reqwest::Client,
) -> Value {
    // 显式 home 的原始接口不访问默认 SettingsStore，也不创建真实 state。
    read_codex_model_catalog_from_snapshot(CodexCatalogSnapshot::capture(home, env), client).await
}

impl CodexCatalogSnapshot {
    fn capture(home: &Path, env: &HashMap<String, String>) -> Self {
        let config_path = home.join("config.toml");
        let auth_api_key = read_codex_auth_api_key(&home.join("auth.json"));
        let (config, effective, error) = load_codex_config(&config_path);
        let mut sources = model_sources_from_environment(env, &auth_api_key);
        if error.is_none()
            && let Some(source) = model_source_from_config(&config, &effective, env, &auth_api_key)
            && sources
                .iter()
                .all(|existing| trim_url(&existing.base_url) != trim_url(&source.base_url))
        {
            sources.push(source);
        }
        let (catalog_models, catalog_status) =
            models_from_config_model_catalog_json(home, &effective);
        Self {
            config_path,
            config,
            effective,
            error,
            sources,
            catalog_models,
            catalog_status,
        }
    }
}

async fn read_codex_model_catalog_from_snapshot(
    snapshot: CodexCatalogSnapshot,
    client: reqwest::Client,
) -> Value {
    let CodexCatalogSnapshot {
        config_path,
        config,
        effective,
        error,
        sources,
        catalog_models,
        catalog_status,
    } = snapshot;
    let mut model = string_value(effective.get("model"));
    let mut model_provider = string_value(effective.get("model_provider"));
    let (resolved_provider, provider_config) =
        provider_config_for_model_provider(&config, &model_provider);
    if model_provider.is_empty() && !resolved_provider.is_empty() {
        model_provider = resolved_provider;
    }
    let provider_name = provider_config
        .as_ref()
        .and_then(|provider| provider.get("name"))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| model_provider.clone());

    if let Some(error) = error.as_ref().filter(|error| *error != "missing") {
        return json!({
            "status": "failed",
            "path": config_path.to_string_lossy(),
            "message": error,
            "service_tier": service_tier_value(&effective),
            "model": model,
            "model_provider": model_provider,
            "provider_name": provider_name,
            "default_model": "",
            "models": [],
            "modelMetadata": {},
            "sources": [],
            "responses_api": responses_api_status("unknown", "", "")
        });
    }

    let mut source_statuses = Vec::new();
    let mut models = Vec::new();
    for source in sources.iter() {
        let (source_models, mut source_status) = fetch_models_from_source(&client, source).await;
        source_status["responses_api"] = responses_api_status("unknown", "", "");
        models.extend(source_models);
        source_statuses.push(source_status);
    }
    models.extend(catalog_models);
    if let Some(status) = catalog_status {
        source_statuses.push(status);
    }

    models = unique_strings(models);
    if model.is_empty() {
        model = string_value(effective.get("default_model"));
    }
    let default_model = if models.iter().any(|item| item == &model) {
        model.clone()
    } else {
        models.first().cloned().unwrap_or_default()
    };
    let status = if !models.is_empty() {
        "ok"
    } else if !source_statuses.is_empty()
        && source_statuses
            .iter()
            .any(|source| source.get("status").and_then(Value::as_str) == Some("failed"))
    {
        "failed"
    } else if error.as_deref() == Some("missing") {
        "missing"
    } else {
        "not_configured"
    };
    let responses_api = preferred_responses_api_status(&source_statuses);
    let model_metadata = model_ui_metadata_map(&models);

    json!({
        "status": status,
        "path": config_path.to_string_lossy(),
        "service_tier": service_tier_value(&effective),
        "model": model,
        "model_provider": model_provider,
        "provider_name": provider_name,
        "default_model": default_model,
        "models": models,
        "modelMetadata": model_metadata,
        "sources": source_statuses,
        "responses_api": responses_api
    })
}

fn codex_home_dir() -> PathBuf {
    crate::codex_home::default_codex_home_dir()
}

// 读取 config.toml（含 profile 覆盖）里生效的 service_tier；未配置时返回 null。
fn service_tier_value(effective: &HashMap<String, String>) -> Value {
    let tier = string_value(effective.get("service_tier"));
    if tier.is_empty() {
        Value::Null
    } else {
        Value::String(tier)
    }
}

fn config_service_tier_value(home: &Path) -> Value {
    let (_, effective, _) = load_codex_config(&home.join("config.toml"));
    service_tier_value(&effective)
}

fn load_codex_config(path: &Path) -> (CodexConfig, HashMap<String, String>, Option<String>) {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return (
                CodexConfig::default(),
                HashMap::new(),
                Some("missing".to_string()),
            );
        }
        Err(error) => {
            return (
                CodexConfig::default(),
                HashMap::new(),
                Some(error.to_string()),
            );
        }
    };
    let config = parse_codex_config(&contents);
    let mut effective = config.root.clone();
    if let Some(profile) = config.root.get("profile") {
        if let Some(profile_values) = config.profiles.get(profile) {
            effective.extend(profile_values.clone());
        }
    }
    (config, effective, None)
}

fn parse_codex_config(contents: &str) -> CodexConfig {
    let mut config = CodexConfig::default();
    let mut section = ConfigSection::Root;
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            section = ConfigSection::from_header(trimmed.trim_matches(&['[', ']'][..]));
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        let key = key.trim().to_string();
        let value = unquote_toml_string(value);
        match &section {
            ConfigSection::Root => {
                config.root.insert(key, value);
            }
            ConfigSection::Profile(name) => {
                config
                    .profiles
                    .entry(name.clone())
                    .or_default()
                    .insert(key, value);
            }
            ConfigSection::ModelProvider(name) => {
                config
                    .model_providers
                    .entry(name.clone())
                    .or_default()
                    .insert(key, value);
            }
            ConfigSection::Other => {}
        }
    }
    config
}

#[derive(Debug, Clone)]
enum ConfigSection {
    Root,
    Profile(String),
    ModelProvider(String),
    Other,
}

impl ConfigSection {
    fn from_header(header: &str) -> Self {
        if let Some(name) = header.strip_prefix("profiles.") {
            return Self::Profile(name.trim_matches('"').to_string());
        }
        if let Some(name) = header.strip_prefix("model_providers.") {
            return Self::ModelProvider(name.trim_matches('"').to_string());
        }
        Self::Other
    }
}

fn read_codex_auth_api_key(path: &Path) -> String {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return String::new();
    };
    let Ok(payload) = serde_json::from_str::<Value>(&contents) else {
        return String::new();
    };
    for key in [
        "OPENAI_API_KEY",
        "api_key",
        "apikey",
        "access_token",
        "token",
    ] {
        let value = payload
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        if !value.is_empty() {
            return value.to_string();
        }
    }
    String::new()
}

fn provider_config_for_model_provider(
    config: &CodexConfig,
    model_provider: &str,
) -> (String, Option<HashMap<String, String>>) {
    if !model_provider.is_empty() {
        return (
            model_provider.to_string(),
            config.model_providers.get(model_provider).cloned(),
        );
    }
    if config.model_providers.len() == 1 {
        if let Some((name, provider)) = config.model_providers.iter().next() {
            return (name.clone(), Some(provider.clone()));
        }
    }
    (model_provider.to_string(), None)
}

fn model_sources_from_environment(
    env: &HashMap<String, String>,
    auth_api_key: &str,
) -> Vec<ModelSource> {
    let base_url = first_env_value(env, BASE_URL_ENV_KEYS);
    if base_url.is_empty() {
        return Vec::new();
    }
    let api_key = first_env_value(env, API_KEY_ENV_KEYS);
    vec![ModelSource {
        source_id: "env:openai-compatible".to_string(),
        source_type: "environment".to_string(),
        name: "Environment".to_string(),
        base_url,
        api_key: if api_key.is_empty() {
            auth_api_key.to_string()
        } else {
            api_key
        },
        headers: Vec::new(),
    }]
}

fn model_source_from_config(
    config: &CodexConfig,
    effective: &HashMap<String, String>,
    env: &HashMap<String, String>,
    auth_api_key: &str,
) -> Option<ModelSource> {
    let model_provider = string_value(effective.get("model_provider"));
    let (resolved_provider, provider_config) =
        provider_config_for_model_provider(config, &model_provider);
    let provider_config = provider_config?;
    let base_url = string_value(provider_config.get("base_url"));
    if base_url.is_empty() {
        return None;
    }
    let name = string_value(provider_config.get("name"));
    let api_key = provider_api_key(&provider_config, env, auth_api_key);
    Some(ModelSource {
        source_id: format!(
            "config:{}",
            if resolved_provider.is_empty() {
                &name
            } else {
                &resolved_provider
            }
        ),
        source_type: "config".to_string(),
        name: if name.is_empty() {
            resolved_provider
        } else {
            name
        },
        base_url,
        api_key,
        headers: Vec::new(),
    })
}

fn provider_api_key(
    provider_config: &HashMap<String, String>,
    env: &HashMap<String, String>,
    auth_api_key: &str,
) -> String {
    for key in [
        "experimental_bearer_token",
        "api_key",
        "apikey",
        "bearer_token",
        "token",
    ] {
        let value = string_value(provider_config.get(key));
        if !value.is_empty() {
            return value;
        }
    }
    for key in [
        "env_key",
        "api_key_env",
        "api_key_env_var",
        "key_env",
        "bearer_token_env",
    ] {
        let env_name = string_value(provider_config.get(key));
        if !env_name.is_empty() {
            let value = first_env_value(env, &[&env_name]);
            if !value.is_empty() {
                return value;
            }
        }
    }
    let env_key = first_env_value(env, API_KEY_ENV_KEYS);
    if env_key.is_empty() {
        auth_api_key.to_string()
    } else {
        env_key
    }
}

/// 单个模型列表请求的整体预算（发请求 + 读响应体）。模型列表是小体量非流式响应，
/// 30 秒足够；此前请求没有任何超时，上游不响应时管理器的「获取模型」会永久挂起。
const MODELS_FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

async fn fetch_models_from_source(
    client: &reqwest::Client,
    source: &ModelSource,
) -> (Vec<String>, Value) {
    fetch_models_from_source_with_timeout(client, source, MODELS_FETCH_TIMEOUT).await
}

async fn fetch_models_from_source_with_timeout(
    client: &reqwest::Client,
    source: &ModelSource,
    timeout: std::time::Duration,
) -> (Vec<String>, Value) {
    let endpoint = models_endpoint(&source.base_url);
    let mut safe_source = json!({
        "id": source.source_id,
        "type": source.source_type,
        "name": source.name,
        "base_url": safe_url_for_status(&source.base_url),
        "endpoint": safe_url_for_status(&endpoint),
        "auth": if source.api_key.is_empty() { "missing" } else { "present" },
    });
    if endpoint.is_empty() {
        safe_source["status"] = json!("failed");
        safe_source["message"] = json!("Missing base URL");
        safe_source["models"] = json!(0);
        return (Vec::new(), safe_source);
    }

    let mut request = client
        .get(&endpoint)
        .header(reqwest::header::ACCEPT, "application/json");
    // 与协议代理、供应商测试共用 relay_headers：显式 Authorization 优先于 API Key。
    if !source.api_key.is_empty() && !crate::relay_headers::has_authorization(&source.headers) {
        request = request.bearer_auth(&source.api_key);
    }
    request = crate::relay_headers::apply_headers(request, &source.headers);

    match tokio::time::timeout(timeout, async move {
        let response = request.send().await?;
        let status = response.status().as_u16();
        let body = response.text().await?;
        Ok::<_, reqwest::Error>((status, body))
    })
    .await
    {
        Err(_elapsed) => failed_source(
            safe_source,
            format!(
                "上游 {} 秒内未返回模型列表（已超时中断）",
                timeout.as_secs()
            ),
        ),
        Ok(Err(error)) => failed_source(safe_source, error.to_string()),
        Ok(Ok((status_code, body))) => interpret_models_response(status_code, &body, safe_source),
    }
}

fn interpret_models_response(
    status_code: u16,
    body: &str,
    mut safe_source: Value,
) -> (Vec<String>, Value) {
    let payload = serde_json::from_str::<Value>(body.trim()).ok();
    if !(200..300).contains(&status_code) {
        return failed_source(
            safe_source,
            format!("HTTP {status_code}{}", upstream_error_detail(body)),
        );
    }
    let Some(payload) = payload else {
        return failed_source(
            safe_source,
            format!(
                "HTTP {status_code} 响应不是有效 JSON{}",
                upstream_error_detail(body)
            ),
        );
    };
    // 业务错误信封优先于模型列表判定：网关在错误态下可能回一个占位或部分
    // 列表（issue 复现：{"code":401,"success":false,"data":[{"id":"..."}]}），
    // 先看列表会把它当成「获取成功」，用户看到成功但实际令牌已失效。
    // 反过来不会误伤正常响应——business_error_message 只认 success=false /
    // code∉{0,200} / error 非空，标准 OpenAI 响应三者都没有。
    if let Some(message) = business_error_message(&payload) {
        return failed_source(safe_source, message);
    }
    let models = unique_strings(parse_model_payload(&payload));
    if !models.is_empty() {
        safe_source["status"] = json!("ok");
        safe_source["models"] = json!(models.len());
        return (models, safe_source);
    }
    // HTTP 200 且无业务错误信封：维持既有语义，按“网关可达但 0 个模型”处理
    safe_source["status"] = json!("ok");
    safe_source["models"] = json!(0);
    (Vec::new(), safe_source)
}

fn failed_source(mut source: Value, message: String) -> (Vec<String>, Value) {
    source["status"] = json!("failed");
    source["message"] = json!(message);
    source["models"] = json!(0);
    source["responses_api"] = responses_api_status("unknown", "", "");
    (Vec::new(), source)
}

/// 部分网关用 HTTP 200 + 业务信封承载失败。智谱 Codex 专属端点（/api/v1）在 key
/// 缺失或无效时返回 `{"code":401,"msg":"令牌已过期或验证不正确","success":false}`，
/// HTTP 状态仍是 200，此前只认状态码，这类失败被解析成“0 个模型”，报错完全不
/// 指向真因（#2190）。只在模型列表解析为空时才判信封——能取到数据就优先信数据，
/// 避免误伤非标成功响应。
fn business_error_message(payload: &Value) -> Option<String> {
    let object = payload.as_object()?;
    let envelope_failed = match object.get("success").and_then(Value::as_bool) {
        Some(success) => !success,
        None => {
            numeric_business_code(object.get("code")).is_some_and(|failed| failed)
                || object.get("error").is_some_and(|error| !error.is_null())
        }
    };
    if !envelope_failed {
        return None;
    }
    message_from_payload(object)
}

/// `code` 字段是否落在失败档：数字（含字符串数字）且不等于 0/200 视为失败，
/// 0 是常见的“成功”码，200 是 OpenAI 系成功码。非数字 code 不参与判定。
fn numeric_business_code(code: Option<&Value>) -> Option<bool> {
    let code = code?;
    let code = code.as_i64().or_else(|| {
        code.as_str()
            .and_then(|code| code.trim().parse::<i64>().ok())
    })?;
    Some(code != 0 && code != 200)
}

/// 从错误体对象里提取人话原因，兼容 OpenAI 系 `{"error":{"message":...}}`、
/// 智谱 `{"msg":...}`、xAI `{"error":"..."}` 等形态。
fn message_from_payload(object: &Map<String, Value>) -> Option<String> {
    for key in ["error", "msg", "message"] {
        let Some(value) = object.get(key).filter(|value| !value.is_null()) else {
            continue;
        };
        let text = match value {
            Value::String(text) => Some(text.trim().to_string()),
            Value::Object(nested) => nested
                .get("message")
                .and_then(Value::as_str)
                .map(|text| text.trim().to_string()),
            _ => None,
        };
        if let Some(text) = text.filter(|text| !text.is_empty()) {
            return Some(text);
        }
    }
    None
}

/// 非 2xx（或非 JSON）时把上游错误体里的原因带进报错（如
/// `HTTP 401：Incorrect API key provided`）。此前只报 "HTTP 401"，key 失效、
/// 地址填错、套餐未开通全靠用户猜。
fn upstream_error_detail(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let detail = serde_json::from_str::<Value>(trimmed)
        .ok()
        .and_then(|payload| message_from_payload(payload.as_object()?))
        .unwrap_or_else(|| truncate_for_message(trimmed));
    format!("：{detail}")
}

/// 报错信息里附带的响应体上限，避免上游回一整页 HTML 时把 UI 提示撑爆。
fn truncate_for_message(text: &str) -> String {
    const MAX_CHARS: usize = 200;
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= MAX_CHARS {
        return text;
    }
    let mut truncated: String = text.chars().take(MAX_CHARS).collect();
    truncated.push('…');
    truncated
}

fn responses_api_status(status: &str, endpoint: &str, message: &str) -> Value {
    json!({
        "status": status,
        "endpoint": endpoint,
        "message": message
    })
}

pub async fn fetch_relay_profile_model_ids(
    profile: &RelayProfile,
) -> anyhow::Result<(Vec<String>, String)> {
    // 官方登录 profile 没有 Base URL：带账号 OAuth 凭据读官方模型目录，
    // 「从上游获取」按钮对官方账号同样可用（issue 讨论场景：聚合成员要填模型列表）。
    if crate::official_login::member_uses_official_login_transport(profile) {
        return fetch_official_login_model_ids(profile).await;
    }
    if crate::official_login::is_official_login_profile(profile)
        && profile.base_url.trim().is_empty()
    {
        anyhow::bail!(
            "官方登录供应商「{}」缺少 ChatGPT 登录凭据，请先导入或登录官方账号",
            profile.name
        );
    }
    let source = ModelSource {
        source_id: format!("relay-profile:{}", profile.id),
        source_type: "relay_profile".to_string(),
        name: if profile.name.trim().is_empty() {
            profile.id.clone()
        } else {
            profile.name.trim().to_string()
        },
        base_url: if profile.upstream_base_url.trim().is_empty() {
            profile.base_url.trim().to_string()
        } else {
            profile.upstream_base_url.trim().to_string()
        },
        api_key: crate::relay_config::relay_profile_api_key(profile),
        headers: profile.custom_headers.clone(),
    };
    if source.base_url.is_empty() {
        anyhow::bail!("Base URL 不能为空");
    }
    let endpoint = models_endpoint(&source.base_url);
    let client = crate::http_client::proxied_client_for_profile(&profile.user_agent, &profile.id)?;
    let (models, status) = fetch_models_from_source(&client, &source).await;
    if models.is_empty() {
        let message = status
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("上游没有返回可用模型");
        anyhow::bail!("{message}");
    }
    Ok((models, endpoint))
}

/// 官方模型目录的客户端版本参数；与 Codex CLI 的取值保持一致。
const OFFICIAL_MODELS_CLIENT_VERSION: &str = "0.159.0";

/// 官方登录 profile 的模型列表：带账号 OAuth 凭据请求 chatgpt.com 官方目录。
async fn fetch_official_login_model_ids(
    profile: &RelayProfile,
) -> anyhow::Result<(Vec<String>, String)> {
    let credential = crate::official_login::resolve_official_credential(profile, false).await?;
    let base = crate::official_login::official_base_url();
    let endpoint = format!("{}/models", base.trim().trim_end_matches('/'));
    let client = crate::http_client::proxied_client_for_profile(&profile.user_agent, &profile.id)?;
    let mut request = client
        .get(&endpoint)
        .query(&[("client_version", OFFICIAL_MODELS_CLIENT_VERSION)])
        .header(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {}", credential.access_token.trim()),
        )
        .header("originator", "codex_cli_rs")
        .header("version", OFFICIAL_MODELS_CLIENT_VERSION)
        .timeout(MODELS_FETCH_TIMEOUT);
    if !credential.account_id.trim().is_empty() {
        request = request.header("chatgpt-account-id", credential.account_id.trim());
    }
    let response = request.send().await.context("官方模型列表请求失败")?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!(
            "获取官方模型列表失败：HTTP {status} {}",
            truncate_for_message(&text)
        );
    }
    let value: Value = serde_json::from_str(&text).context("官方模型列表解析失败")?;
    let models = parse_official_model_ids(&value);
    if models.is_empty() {
        anyhow::bail!("官方账号没有返回可见模型");
    }
    Ok((models, endpoint))
}

/// 官方目录条目：slug/id/name 均可作为模型 id；`visibility=hide` 与
/// `visible_in_picker=false` 是官方内部预留位，不进入供应商模型列表。
pub(crate) fn parse_official_model_ids(value: &Value) -> Vec<String> {
    let entries = value
        .get("data")
        .and_then(Value::as_array)
        .or_else(|| value.get("models").and_then(Value::as_array))
        .or_else(|| value.get("items").and_then(Value::as_array))
        .or_else(|| value.as_array());
    let Some(entries) = entries else {
        return Vec::new();
    };
    let mut ids = Vec::new();
    let mut seen = HashSet::new();
    for entry in entries {
        let (id, hidden) = match entry {
            Value::String(id) => (id.trim().to_string(), false),
            Value::Object(map) => {
                let hidden = map
                    .get("visibility")
                    .and_then(Value::as_str)
                    .is_some_and(|visibility| visibility.eq_ignore_ascii_case("hide"))
                    || map.get("visible_in_picker").and_then(Value::as_bool) == Some(false);
                let id = ["slug", "id", "name"]
                    .iter()
                    .find_map(|key| map.get(*key).and_then(Value::as_str))
                    .map(str::trim)
                    .unwrap_or_default()
                    .to_string();
                (id, hidden)
            }
            _ => (String::new(), false),
        };
        if id.is_empty() || hidden || !seen.insert(id.clone()) {
            continue;
        }
        ids.push(id);
    }
    ids
}

fn preferred_responses_api_status(sources: &[Value]) -> Value {
    let statuses = sources
        .iter()
        .filter_map(|source| source.get("responses_api"))
        .collect::<Vec<_>>();
    for wanted in ["unsupported", "supported", "failed"] {
        if let Some(status) = statuses
            .iter()
            .find(|status| status.get("status").and_then(Value::as_str) == Some(wanted))
        {
            return (*status).clone();
        }
    }
    responses_api_status("unknown", "", "")
}

fn models_endpoint(base_url: &str) -> String {
    let cleaned = safe_url_for_status(base_url).trim_end_matches('/').to_string();
    if cleaned.is_empty() {
        return String::new();
    }
    // 与实际代理共用规则，保留 /v1beta/openai 等已有路径；清理后的 URL 不含凭据。
    crate::protocol_proxy::models_url(&cleaned)
}

fn parse_model_payload(payload: &Value) -> Vec<String> {
    if let Some(array) = payload.as_array() {
        return array
            .iter()
            .filter_map(|item| {
                item.as_str().map(str::to_string).or_else(|| {
                    item.as_object().and_then(|object| {
                        ["id", "model", "name"]
                            .iter()
                            .filter_map(|key| {
                                object
                                    .get(*key)
                                    .and_then(Value::as_str)
                                    .map(|value| model_id_from_field(key, value))
                                    .filter(|value| !value.is_empty())
                            })
                            .next()
                            .map(ToString::to_string)
                    })
                })
            })
            .collect();
    }
    let Some(object) = payload.as_object() else {
        return Vec::new();
    };
    for key in ["data", "models", "items"] {
        if let Some(value) = object.get(key) {
            let nested = parse_model_payload(value);
            if !nested.is_empty() {
                return nested;
            }
        }
    }
    ["id", "model", "name"]
        .iter()
        .filter_map(|key| {
            object
                .get(*key)
                .and_then(Value::as_str)
                .map(|value| model_id_from_field(key, value))
                .filter(|value| !value.is_empty())
        })
        .next()
        .map(|value| vec![value.to_string()])
        .unwrap_or_default()
}

/// 模型 ID 字段取值。Gemini 的列表把 ID 放在 `name` 字段且带 `models/` 前缀
/// （如 `models/gemini-2.5-pro`），剥掉前缀让 ID 与请求体里的 `model` 值一致。
fn model_id_from_field<'a>(key: &str, value: &'a str) -> &'a str {
    let value = value.trim();
    if key == "name" {
        value.strip_prefix("models/").unwrap_or(value)
    } else {
        value
    }
}

fn models_from_config_model_catalog_json(
    home: &Path,
    effective: &HashMap<String, String>,
) -> (Vec<String>, Option<Value>) {
    let raw_path = string_value(effective.get("model_catalog_json"));
    if raw_path.is_empty() {
        return (Vec::new(), None);
    }
    let path = resolve_config_path(home, &raw_path);
    let safe_path = path.to_string_lossy().to_string();
    let contents = match std::fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) => {
            return (
                Vec::new(),
                Some(json!({
                    "id": "config:model_catalog_json",
                    "type": "model_catalog_json",
                    "name": "Codex model catalog",
                    "path": safe_path,
                    "status": "failed",
                    "message": error.to_string(),
                    "models": 0,
                    "responses_api": responses_api_status("unknown", "", "")
                })),
            );
        }
    };
    let payload = match serde_json::from_str::<Value>(&contents) {
        Ok(payload) => payload,
        Err(error) => {
            return (
                Vec::new(),
                Some(json!({
                    "id": "config:model_catalog_json",
                    "type": "model_catalog_json",
                    "name": "Codex model catalog",
                    "path": safe_path,
                    "status": "failed",
                    "message": error.to_string(),
                    "models": 0,
                    "responses_api": responses_api_status("unknown", "", "")
                })),
            );
        }
    };
    let models = unique_strings(parse_model_catalog_json_models(&payload));
    let count = models.len();
    (
        models,
        Some(json!({
            "id": "config:model_catalog_json",
            "type": "model_catalog_json",
            "name": "Codex model catalog",
            "path": safe_path,
            "status": "ok",
            "models": count,
            "responses_api": responses_api_status("unknown", "", "")
        })),
    )
}

fn resolve_config_path(home: &Path, value: &str) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        home.join(path)
    }
}

fn parse_model_catalog_json_models(payload: &Value) -> Vec<String> {
    let Some(models) = payload.get("models").and_then(Value::as_array) else {
        return Vec::new();
    };
    models
        .iter()
        .filter(|model| catalog_model_visible_in_api(model))
        .filter_map(|model| model.get("slug").and_then(Value::as_str))
        .map(str::trim)
        .filter(|slug| !slug.is_empty())
        .map(str::to_string)
        .collect()
}

fn catalog_model_visible_in_api(model: &Value) -> bool {
    let supported_in_api = model
        .get("supported_in_api")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    if !supported_in_api {
        return false;
    }
    let visibility = model
        .get("visibility")
        .and_then(Value::as_str)
        .unwrap_or("list")
        .trim();
    visibility.eq_ignore_ascii_case("list")
}

fn unique_strings(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for value in values {
        let value = value.trim();
        if value.is_empty() || !seen.insert(value.to_string()) {
            continue;
        }
        result.push(value.to_string());
    }
    result
}

fn first_env_value(env: &HashMap<String, String>, names: &[&str]) -> String {
    names
        .iter()
        .filter_map(|name| env.get(*name))
        .map(|value| value.trim())
        .find(|value| !value.is_empty())
        .unwrap_or_default()
        .to_string()
}

fn safe_url_for_status(url: &str) -> String {
    let mut cleaned = url
        .split('?')
        .next()
        .unwrap_or_default()
        .split('#')
        .next()
        .unwrap_or_default()
        .to_string();
    if let Ok(parsed) = reqwest::Url::parse(&cleaned) {
        let host = parsed.host_str().unwrap_or_default();
        let authority = parsed
            .port()
            .map(|port| format!("{host}:{port}"))
            .unwrap_or_else(|| host.to_string());
        cleaned = format!("{}://{}{}", parsed.scheme(), authority, parsed.path());
    }
    cleaned
}

fn trim_url(url: &str) -> String {
    url.trim_end_matches('/').to_string()
}

fn string_value(value: Option<&String>) -> String {
    value
        .map(|value| value.trim().to_string())
        .unwrap_or_default()
}

fn unquote_toml_string(value: &str) -> String {
    let value = value.trim();
    if let Ok(parsed) = toml::from_str::<toml::Value>(&format!("value = {value}")) {
        if let Some(value) = parsed.get("value").and_then(toml::Value::as_str) {
            return value.to_string();
        }
    }
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

#[cfg(test)]
mod model_fetch_tests {
    use super::*;

    #[test]
    fn invalid_pool_catalog_does_not_resurrect_live_catalog_or_compat_model() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("config.toml"), "model = \"unconfigured-compat\"\n").unwrap();
        let settings = crate::settings::BackendSettings {
            active_relay_id: "pool".into(),
            active_aggregate_relay_id: "pool".into(),
            relay_profiles: vec![RelayProfile { id: "pool".into(), relay_mode: crate::settings::RelayMode::Aggregate,
                ..RelayProfile::default() }],
            aggregate_relay_profiles: serde_json::from_value(json!([{"id":"pool","name":"Pool","members":[]}])).unwrap(),
            ..crate::settings::BackendSettings::default()
        };
        let CatalogReadSnapshot::Ready(catalog) = capture_catalog_read_snapshot(temp.path(), &HashMap::new(), &settings) else {
            panic!("pool catalog must remain authoritative");
        };
        assert_eq!(catalog["status"], "failed");
        assert_eq!(catalog["models"], json!([]));
    }

    #[tokio::test]
    async fn empty_profile_fallback_waits_for_failed_publication_rollback() {
        use fs2::FileExt;
        use std::sync::mpsc;
        use std::time::Duration;

        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let settings_path = home.join("settings.json");
        let store = SettingsStore::new(settings_path.clone());
        let initial = crate::settings::BackendSettings {
            active_relay_id: "empty-profile".into(),
            relay_profiles: vec![RelayProfile {
                id: "empty-profile".into(),
                model: String::new(),
                model_list: String::new(),
                ..RelayProfile::default()
            }],
            ..crate::settings::BackendSettings::default()
        };
        store.save(&initial).unwrap();
        let mut pending = store.load().unwrap();
        pending.relay_profiles[0].name = "pending".into();
        pending.relay_profiles[0].model_list = "pending-profile".into();
        let original_config =
            b"model = \"old-local\"\nmodel_catalog_json = \"catalog.json\"\n";
        let original_auth = br#"{"OPENAI_API_KEY":"synthetic-old-auth"}"#;
        let original_catalog = br#"{"models":[{"slug":"old-local"}]}"#;
        std::fs::write(home.join("config.toml"), original_config).unwrap();
        std::fs::write(home.join("auth.json"), original_auth).unwrap();
        std::fs::write(home.join("catalog.json"), original_catalog).unwrap();
        let (published_tx, published_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let writer_home = home.to_path_buf();
        let writer_store = store.clone();
        let writer = std::thread::spawn(move || {
            writer_store.save_with_commit(&pending, |_| {
                std::fs::write(
                    writer_home.join("config.toml"),
                    "model = \"pending-local\"\nmodel_catalog_json = \"catalog.json\"\n",
                )?;
                std::fs::write(
                    writer_home.join("auth.json"), br#"{"OPENAI_API_KEY":"synthetic-pending-auth"}"#,
                )?;
                std::fs::write(
                    writer_home.join("catalog.json"), br#"{"models":[{"slug":"pending-local"}]}"#,
                )?;
                published_tx.send(()).unwrap();
                let released = release_rx.recv_timeout(Duration::from_secs(5));
                // 关联文件先在锁内恢复，再由 SettingsStore 恢复 settings 原始字节。
                std::fs::write(writer_home.join("config.toml"), original_config)?;
                std::fs::write(writer_home.join("auth.json"), original_auth)?;
                std::fs::write(writer_home.join("catalog.json"), original_catalog)?;
                released.expect("publication release timed out");
                anyhow::bail!("injected catalog publication failure");
            })
        });
        published_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let (contended_tx, contended_rx) = mpsc::channel();
        let reader_store = store.clone();
        let reader_home = home.to_path_buf();
        let reader = std::thread::spawn(move || {
            let file = std::fs::OpenOptions::new().read(true).write(true)
                .open(settings_path.with_extension("json.lock")).unwrap();
            let contended = match FileExt::try_lock_exclusive(&file) {
                Ok(()) => {
                    FileExt::unlock(&file).unwrap();
                    false
                }
                Err(error) => error.raw_os_error() == fs2::lock_contended_error().raw_os_error(),
            };
            contended_tx.send(contended).unwrap();
            reader_store.with_snapshot(|settings| {
                Ok(capture_catalog_read_snapshot(&reader_home, &HashMap::new(), &settings))
            })
        });
        let contended = contended_rx.recv_timeout(Duration::from_secs(5));
        // 断言前释放并 join，避免失败路径留下等待发布的线程。
        let _ = release_tx.send(());
        let write_result = writer.join().unwrap();
        let read_result = reader.join().unwrap();
        assert!(contended.unwrap(), "catalog reader must encounter the publication lock");
        assert!(write_result.unwrap_err().to_string().contains("injected catalog publication failure"));
        let CatalogReadSnapshot::Fallback(snapshot) = read_result.unwrap() else {
            panic!("empty profile must still use fallback after rollback");
        };
        assert!(snapshot.sources.is_empty());
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let catalog = tokio::time::timeout(
            Duration::from_secs(5), read_codex_model_catalog_from_snapshot(snapshot, client),
        ).await.expect("rolled back catalog read timed out");
        assert_eq!(catalog["model"], "old-local");
        assert_eq!(catalog["models"], json!(["old-local"]));
        assert_eq!(catalog["sources"][0]["status"], "ok");
    }

    #[tokio::test]
    async fn empty_profile_fallback_uses_captured_config_auth_catalog_and_sources() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let old_config = format!(
            "model = \"old-local\"\nmodel_provider = \"old-provider\"\nservice_tier = \"fast\"\nmodel_catalog_json = \"old-catalog.json\"\n[model_providers.old-provider]\nname = \"Old Provider\"\nbase_url = \"http://{address}/config/v1\"\n"
        );
        std::fs::write(home.join("config.toml"), old_config).unwrap();
        std::fs::write(
            home.join("auth.json"), r#"{"OPENAI_API_KEY":"synthetic-old-auth"}"#,
        ).unwrap();
        std::fs::write(
            home.join("old-catalog.json"), r#"{"models":[{"slug":"old-local"}]}"#,
        ).unwrap();
        let mut env = HashMap::from([(
            "OPENAI_BASE_URL".to_string(), format!("http://{address}/env/v1"),
        )]);
        let store = SettingsStore::new(home.join("settings.json"));
        let settings = crate::settings::BackendSettings {
            active_relay_id: "empty-profile".into(),
            relay_profiles: vec![RelayProfile {
                id: "empty-profile".into(),
                model: String::new(),
                model_list: String::new(),
                ..RelayProfile::default()
            }],
            ..crate::settings::BackendSettings::default()
        };
        store.save(&settings).unwrap();
        let captured = store.with_snapshot(|settings| {
            Ok(capture_catalog_read_snapshot(home, &env, &settings))
        }).unwrap();
        let CatalogReadSnapshot::Fallback(snapshot) = captured else {
            panic!("empty profile must capture fallback in the coordinated read");
        };

        // 同时改指针和旧指针的内容，异步阶段不能重读任意一个本地文件。
        std::fs::write(
            home.join("config.toml"),
            "model = \"new-local\"\nmodel_provider = \"new-provider\"\nservice_tier = \"flex\"\nmodel_catalog_json = \"new-catalog.json\"\n[model_providers.new-provider]\nname = \"New Provider\"\nbase_url = \"http://127.0.0.1:9/new/v1\"\n",
        ).unwrap();
        std::fs::write(
            home.join("auth.json"), r#"{"OPENAI_API_KEY":"synthetic-new-auth"}"#,
        ).unwrap();
        for filename in ["old-catalog.json", "new-catalog.json"] {
            std::fs::write(
                home.join(filename), r#"{"models":[{"slug":"new-local"}]}"#,
            ).unwrap();
        }
        env.insert("OPENAI_BASE_URL".into(), "http://127.0.0.1:9/new-env/v1".into());
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let serve = async {
            for expected_path in ["/env/v1/models", "/config/v1/models"] {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let mut chunk = [0_u8; 1024];
                while !bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                    let read = stream.read(&mut chunk).await.unwrap();
                    assert!(read > 0, "mock request closed before headers");
                    bytes.extend_from_slice(&chunk[..read]);
                }
                let request = String::from_utf8(bytes).unwrap();
                // 不在失败信息中打印含凭据的完整请求。
                assert!(request.starts_with(&format!("GET {expected_path} HTTP/1.1\r\n")));
                assert!(request.lines().any(|line| {
                    line.eq_ignore_ascii_case("authorization: Bearer synthetic-old-auth")
                }), "request must use captured synthetic credentials");
                let body = r#"{"data":[{"id":"old-remote"}]}"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len(),
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        };
        // 不 spawn：超时或断言失败会直接销毁两个 future 和所有 mock 连接。
        let (catalog, ()) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            tokio::join!(read_codex_model_catalog_from_snapshot(snapshot, client), serve)
        }).await.expect("snapshot regression timed out");
        assert_eq!(catalog["status"], "ok");
        assert_eq!(catalog["model"], "old-local");
        assert_eq!(catalog["model_provider"], "old-provider");
        assert_eq!(catalog["provider_name"], "Old Provider");
        assert_eq!(catalog["service_tier"], "fast");
        assert_eq!(catalog["default_model"], "old-local");
        assert_eq!(catalog["models"], json!(["old-remote", "old-local"]));
        assert_eq!(catalog["sources"][2]["path"], home.join("old-catalog.json").to_string_lossy().as_ref());
        assert_eq!(catalog["sources"][2]["status"], "ok");
    }

    #[test]
    fn relay_profile_model_catalog_keeps_configured_default_after_model_list_sorting() {
        let profile = RelayProfile {
            model: "qwen3-coder".into(),
            model_list: "deepseek-coder\nqwen3-coder\nclaude-compatible".into(),
            config_contents: "model = \"qwen3-coder\"\n".into(),
            ..RelayProfile::default()
        };
        let home = tempfile::tempdir().unwrap();
        let catalog = relay_profile_model_catalog_value(home.path(), &profile);

        assert_eq!(catalog["default_model"], "qwen3-coder");
    }

    #[test]
    fn relay_profile_catalog_model_and_default_use_the_same_configured_value() {
        let home = tempfile::tempdir().unwrap();
        let profile = RelayProfile {
            model: "old-model".into(),
            model_list: "old-model\nconfigured-model".into(),
            config_contents: "model = \"configured-model\"\n".into(),
            ..RelayProfile::default()
        };
        let catalog = relay_profile_model_catalog_value(home.path(), &profile);
        assert_eq!(catalog["model"], "configured-model");
        assert_eq!(catalog["default_model"], catalog["model"]);
    }

    #[test]
    fn model_list_endpoint_preserves_custom_prefixes_and_removes_credentials() {
        for (base, expected) in [
            ("https://example.test/v1beta/openai/", "https://example.test/v1beta/openai/models"),
            ("https://example.test", "https://example.test/v1/models"),
            ("https://example.test/api/coding/v3", "https://example.test/api/coding/v3/models"),
            ("https://user:secret@example.test/v1/models?token=secret", "https://example.test/v1/models"),
        ] { assert_eq!(models_endpoint(base), expected); }
    }

    #[test]
    fn relay_model_priority_declaration_reaches_ui_without_enabling_other_models() {
        let profile = RelayProfile {
            model: "grok-custom".into(), model_list: "grok-custom\ngrok-unknown\ngpt-5.4".into(),
            model_metadata: json!({
                "GROK-CUSTOM[1M]": {"service_tiers":[{"id":"flex"}], "additional_speed_tiers":["slow"], "codex_plus_fast_support":"supported"},
                "gpt-5.4": {"codex_plus_fast_support":"unsupported"}
            }).to_string(),
            ..RelayProfile::default()
        };
        let home = tempfile::tempdir().unwrap();
        let catalog = relay_profile_model_catalog_value(home.path(), &profile);
        let metadata = &catalog["modelMetadata"];
        assert_eq!(metadata["grok-custom"]["prioritySupportOverride"], true);
        assert_eq!(metadata["gpt-5.4"]["prioritySupportOverride"], false);
        assert_eq!(metadata["grok-custom"]["serviceTiers"][0]["id"], "flex");
        assert_eq!(metadata["grok-custom"]["serviceTiers"][1]["id"], "priority");
        assert!(metadata.get("grok-unknown").is_none());
        assert!(metadata["grok-custom"].get("codex_plus_fast_support").is_none());
    }

    #[test]
    fn business_error_message_detects_zhipu_style_envelope() {
        // 智谱 /api/v1 Codex 专属端点在 key 缺失或无效时返回 HTTP 200 + 业务信封（#2190）
        let payload = json!({"code": 401, "msg": "令牌已过期或验证不正确", "success": false});
        assert_eq!(
            business_error_message(&payload).as_deref(),
            Some("令牌已过期或验证不正确")
        );
    }

    #[test]
    fn business_error_message_detects_failed_code_without_success_field() {
        let numeric = json!({"code": 1001, "msg": "鉴权失败"});
        assert_eq!(
            business_error_message(&numeric).as_deref(),
            Some("鉴权失败")
        );

        let string_code = json!({"code": "401", "message": "invalid token"});
        assert_eq!(
            business_error_message(&string_code).as_deref(),
            Some("invalid token")
        );

        let error_object = json!({"error": {"code": "1001", "message": "未收到 Authorization"}});
        assert_eq!(
            business_error_message(&error_object).as_deref(),
            Some("未收到 Authorization")
        );
    }

    #[test]
    fn business_error_message_ignores_success_envelopes() {
        for payload in [
            json!({"code": 200, "success": true, "data": []}),
            json!({"code": 0, "msg": "ok", "data": []}),
            json!({"object": "list", "data": []}),
            json!({"code": "200", "data": []}),
            json!([{"id": "glm-5.3"}]),
        ] {
            assert_eq!(business_error_message(&payload), None, "{payload}");
        }
    }

    #[test]
    fn interpret_models_response_keeps_ok_semantics_when_200_has_no_models() {
        let (models, status) =
            interpret_models_response(200, r#"{"object":"list","data":[]}"#, json!({"id": "t"}));
        assert!(models.is_empty());
        assert_eq!(status["status"], "ok");
        assert_eq!(status["models"], 0);
    }

    #[test]
    fn interpret_models_response_surfaces_business_error_on_http_200() {
        let (models, status) = interpret_models_response(
            200,
            r#"{"code":401,"msg":"令牌已过期或验证不正确","success":false}"#,
            json!({"id": "t"}),
        );
        assert!(models.is_empty());
        assert_eq!(status["status"], "failed");
        assert_eq!(status["message"], "令牌已过期或验证不正确");
    }

    #[test]
    fn interpret_models_response_prefers_business_error_over_a_returned_model_list() {
        // 网关在错误态下回了一个列表时，错误必须优先——否则用户看到「获取成功」
        // 但令牌其实已失效，排查会往错误方向走。
        let (models, status) = interpret_models_response(
            200,
            r#"{"code":401,"msg":"令牌已过期","success":false,"data":[{"id":"glm-5.3"}]}"#,
            json!({"id": "t"}),
        );
        assert!(models.is_empty(), "错误态下的列表不应被当作可用模型");
        assert_eq!(status["status"], "failed");
        assert_eq!(status["message"], "令牌已过期");
        assert_eq!(status["models"], 0);
    }

    #[test]
    fn interpret_models_response_keeps_normal_lists_working_without_an_envelope() {
        // 反向保护：标准 OpenAI 响应没有 success/code/error 字段，
        // 提前检查业务错误不能把它误判成失败。
        for body in [
            r#"{"object":"list","data":[{"id":"gpt-5.6-sol"},{"id":"gpt-5.6-terra"}]}"#,
            r#"{"code":200,"success":true,"data":[{"id":"glm-5.3"}]}"#,
            r#"{"code":0,"msg":"ok","data":[{"id":"deepseek-v4"}]}"#,
        ] {
            let (models, status) = interpret_models_response(200, body, json!({"id": "t"}));
            assert!(!models.is_empty(), "{body}");
            assert_eq!(status["status"], "ok", "{body}");
        }
    }

    #[test]
    fn interpret_models_response_appends_upstream_reason_on_http_error() {
        let (_, status) = interpret_models_response(
            401,
            r#"{"error":{"code":"1001","message":"Header中未收到Authorization参数，无法进行身份验证。"}}"#,
            json!({"id": "t"}),
        );
        assert_eq!(status["status"], "failed");
        let message = status["message"].as_str().unwrap();
        assert!(message.starts_with("HTTP 401"), "{message}");
        assert!(
            message.contains("Header中未收到Authorization参数"),
            "{message}"
        );
    }

    #[test]
    fn interpret_models_response_reports_invalid_json_body() {
        let (_, status) = interpret_models_response(200, "<html>502</html>", json!({"id": "t"}));
        assert_eq!(status["status"], "failed");
        let message = status["message"].as_str().unwrap();
        assert!(message.contains("不是有效 JSON"), "{message}");
        assert!(message.contains("<html>502</html>"), "{message}");
    }

    #[test]
    fn upstream_error_detail_truncates_plain_text_bodies() {
        assert_eq!(
            upstream_error_detail("Authentication Fails (governor)"),
            "：Authentication Fails (governor)"
        );
        assert_eq!(upstream_error_detail("   "), "");
        let long_body = "x".repeat(500);
        let detail = upstream_error_detail(&long_body);
        assert!(detail.chars().count() < 260, "{}", detail.chars().count());
    }

    #[test]
    fn parse_model_payload_strips_gemini_models_prefix() {
        let models = parse_model_payload(&json!({
            "models": [
                {"name": "models/gemini-2.5-pro", "supportedGenerationMethods": ["generateContent"]},
                {"name": "models/gemini-2.5-flash"}
            ]
        }));
        assert_eq!(models, vec!["gemini-2.5-pro", "gemini-2.5-flash"]);
    }

    #[test]
    fn parse_model_payload_keeps_plain_name_fields() {
        let models = parse_model_payload(&json!({
            "models": [{"name": "moonshot-v1"}, {"id": "kimi-k2.6"}]
        }));
        assert_eq!(models, vec!["moonshot-v1", "kimi-k2.6"]);
    }

    #[tokio::test]
    async fn models_fetch_times_out_when_upstream_never_responds() {
        // 只 bind 不 accept：TCP 握手由内核完成，send 能成功，读响应体会一直挂起
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let source = ModelSource {
            source_id: "test".to_string(),
            source_type: "test".to_string(),
            name: "Test".to_string(),
            base_url: format!("http://{address}"),
            api_key: "key".to_string(),
            headers: Vec::new(),
        };
        let (_, status) = fetch_models_from_source_with_timeout(
            &client,
            &source,
            std::time::Duration::from_millis(300),
        )
        .await;
        assert_eq!(status["status"], "failed");
        assert!(
            status["message"]
                .as_str()
                .unwrap()
                .contains("未返回模型列表"),
            "{}",
            status["message"]
        );
        drop(listener);
    }

    #[test]
    fn official_model_ids_skip_hidden_entries_and_dedupe() {
        let value = json!({
            "data": [
                {"slug": "gpt-6.1-sol"},
                {"slug": "gpt-reserve", "visibility": "hide"},
                {"id": "gpt-6-sol"},
                {"name": "gpt-6-luna", "visible_in_picker": false},
                "gpt-6.1-sol",
                {"slug": "   "}
            ]
        });
        assert_eq!(
            parse_official_model_ids(&value),
            vec!["gpt-6.1-sol", "gpt-6-sol"]
        );
    }

    #[test]
    fn official_model_ids_accept_models_and_items_shapes() {
        assert_eq!(
            parse_official_model_ids(&json!({"models": [{"slug": "a"}]})),
            vec!["a"]
        );
        assert_eq!(
            parse_official_model_ids(&json!({"items": ["b"]})),
            vec!["b"]
        );
        assert!(parse_official_model_ids(&json!({"other": []})).is_empty());
    }

    /// 官方登录 profile 的模型拉取：带 OAuth 凭据请求官方 models 端点，跳过隐藏位。
    struct OfficialBaseUrlEnvGuard {
        previous: Option<std::ffi::OsString>,
    }

    impl OfficialBaseUrlEnvGuard {
        fn set(value: &str) -> Self {
            let previous = std::env::var_os("CODEX_PLUS_OFFICIAL_BASE_URL");
            unsafe {
                std::env::set_var("CODEX_PLUS_OFFICIAL_BASE_URL", value);
            }
            Self { previous }
        }
    }

    impl Drop for OfficialBaseUrlEnvGuard {
        fn drop(&mut self) {
            unsafe {
                match &self.previous {
                    Some(value) => std::env::set_var("CODEX_PLUS_OFFICIAL_BASE_URL", value),
                    None => std::env::remove_var("CODEX_PLUS_OFFICIAL_BASE_URL"),
                }
            }
        }
    }

    #[tokio::test]
    async fn official_login_profile_fetches_models_with_oauth_headers() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buffer = [0_u8; 4096];
            let read = stream.read(&mut buffer).await.unwrap();
            let request = String::from_utf8_lossy(&buffer[..read]).to_string();
            let body = r#"{"data":[{"slug":"gpt-6.1-sol"},{"slug":"gpt-reserve","visibility":"hide"}]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).await.unwrap();
            request
        });
        let _guard = OfficialBaseUrlEnvGuard::set(&format!("http://{address}/backend-api/codex"));

        let profile = RelayProfile {
            id: "official".to_string(),
            name: "官方登录".to_string(),
            relay_mode: crate::settings::RelayMode::Official,
            auth_contents: r#"{"auth_mode":"chatgpt","tokens":{"access_token":"official-access","refresh_token":"r","account_id":"acct-1"}}"#.to_string(),
            ..RelayProfile::default()
        };
        let (models, endpoint) = fetch_relay_profile_model_ids(&profile).await.unwrap();
        assert_eq!(models, vec!["gpt-6.1-sol"]);
        assert!(endpoint.ends_with("/backend-api/codex/models"), "{endpoint}");

        let request = server.await.unwrap();
        let lower = request.to_ascii_lowercase();
        assert!(
            request.starts_with("GET /backend-api/codex/models?client_version="),
            "{request}"
        );
        assert!(
            lower.contains("authorization: bearer official-access"),
            "{request}"
        );
        assert!(lower.contains("chatgpt-account-id: acct-1"), "{request}");
        assert!(lower.contains("originator: codex_cli_rs"), "{request}");
    }
}
