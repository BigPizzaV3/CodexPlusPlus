use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

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
}

#[derive(Debug, Default)]
struct CodexConfig {
    root: HashMap<String, String>,
    profiles: HashMap<String, HashMap<String, String>>,
    model_providers: HashMap<String, HashMap<String, String>>,
}

pub async fn read_codex_model_catalog() -> Value {
    let home = codex_home_dir();
    let settings_path = crate::paths::default_settings_path();
    if settings_path.exists() {
        if let Ok(settings) = SettingsStore::new(settings_path).load() {
            let profile = settings.active_relay_profile();
            let catalog = relay_profile_model_catalog_value(&home, &profile);
            if catalog
                .get("models")
                .and_then(Value::as_array)
                .map_or(false, |m| !m.is_empty())
            {
                return catalog;
            }
        }
    }
    let env = std::env::vars().collect::<HashMap<_, _>>();
    let client = match crate::http_client::proxied_client("CodexPlusPlus/1.0") {
        Ok(client) => client,
        Err(error) => {
            return json!({
                "status": "failed",
                "path": home.join("config.toml").to_string_lossy(),
                "message": error.to_string(),
                "service_tier": config_service_tier_value(&home),
                "model": "",
                "model_provider": "",
                "provider_name": "",
                "default_model": "",
                "models": [],
                "modelMetadata": {},
                "sources": [],
                "responses_api": responses_api_status("unknown", "", "")
            });
        }
    };
    read_codex_model_catalog_from_home(&home, &env, client).await
}

fn relay_profile_model_catalog_value(home: &Path, profile: &RelayProfile) -> Value {
    let models = relay_profile_model_ids(profile);
    let model = profile.model.trim().to_string();
    let codex_model_provider = codex_model_provider_for_relay_profile(home, profile);
    let default_model = models.first().cloned().unwrap_or_default();
    let provider_name = if profile.name.trim().is_empty() {
        profile.id.trim()
    } else {
        profile.name.trim()
    };
    let model_count = models.len();
    let model_metadata = model_ui_metadata_map(&models);
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

fn model_ui_metadata_map(models: &[String]) -> Value {
    let mut metadata = Map::new();
    for model in models {
        if let Some(value) = crate::model_suffix::model_ui_metadata(model) {
            metadata.insert(model.clone(), value);
        }
    }
    Value::Object(metadata)
}

pub async fn read_codex_model_catalog_from_home(
    home: &Path,
    env: &HashMap<String, String>,
    client: reqwest::Client,
) -> Value {
    let config_path = home.join("config.toml");
    let auth_api_key = read_codex_auth_api_key(&home.join("auth.json"));
    let (config, effective, error) = load_codex_config(&config_path);
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

    let mut sources = model_sources_from_environment(env, &auth_api_key);
    if error.is_none() {
        if let Some(source) = model_source_from_config(&config, &effective, env, &auth_api_key) {
            if sources
                .iter()
                .all(|existing| trim_url(&existing.base_url) != trim_url(&source.base_url))
            {
                sources.push(source);
            }
        }
    }

    let mut source_statuses = Vec::new();
    let mut models = Vec::new();
    for source in sources.iter() {
        let (source_models, mut source_status) = fetch_models_from_source(&client, source).await;
        source_status["responses_api"] = responses_api_status("unknown", "", "");
        models.extend(source_models);
        source_statuses.push(source_status);
    }
    let (catalog_models, catalog_status) = models_from_config_model_catalog_json(home, &effective);
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
    if !source.api_key.is_empty() {
        request = request.bearer_auth(&source.api_key);
    }

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
    let models = unique_strings(parse_model_payload(&payload));
    if !models.is_empty() {
        safe_source["status"] = json!("ok");
        safe_source["models"] = json!(models.len());
        return (models, safe_source);
    }
    if let Some(message) = business_error_message(&payload) {
        return failed_source(safe_source, message);
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
        api_key: profile.api_key.trim().to_string(),
    };
    if source.base_url.is_empty() {
        anyhow::bail!("Base URL 不能为空");
    }
    let endpoint = models_endpoint(&source.base_url);
    let client = crate::http_client::proxied_client(&profile.user_agent)?;
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
    let cleaned = safe_url_for_status(base_url)
        .trim_end_matches('/')
        .to_string();
    if cleaned.is_empty() {
        return String::new();
    }
    if cleaned.ends_with("/models") {
        return cleaned;
    }
    // Only append the default `/v1` version prefix when the base URL does not
    // already carry a version segment. Providers such as Volcano Engine ARK use
    // a versioned base (e.g. `.../api/coding/v3`), so blindly appending
    // `/v1/models` produced `.../api/coding/v3/v1/models` and 404'd. This mirrors
    // the version handling already used by the protocol proxy. See issue #1349.
    if crate::protocol_proxy::has_version_suffix(&cleaned) {
        return format!("{cleaned}/models");
    }
    format!("{cleaned}/v1/models")
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
}
