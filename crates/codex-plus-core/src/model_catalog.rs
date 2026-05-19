use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

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
    let env = std::env::vars().collect::<HashMap<_, _>>();
    let client = match crate::http_client::proxied_client("CodexPlusPlus/1.0") {
        Ok(client) => client,
        Err(error) => {
            return json!({
                "status": "failed",
                "path": home.join("config.toml").to_string_lossy(),
                "message": error.to_string(),
                "model": "",
                "model_provider": "",
                "provider_name": "",
                "default_model": "",
                "models": [],
                "sources": [],
                "responses_api": responses_api_status("unknown", "", "")
            });
        }
    };
    read_codex_model_catalog_from_home(&home, &env, client).await
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
            "model": model,
            "model_provider": model_provider,
            "provider_name": provider_name,
            "default_model": "",
            "models": [],
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
        let probe_model = preferred_probe_model(&model, &source_models);
        let responses_api = if source_status.get("status").and_then(Value::as_str) == Some("ok") {
            probe_responses_api_support(&client, source, &probe_model).await
        } else {
            responses_api_status("unknown", &responses_endpoint(&source.base_url), "")
        };
        source_status["responses_api"] = responses_api;
        models.extend(source_models);
        source_statuses.push(source_status);
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

    json!({
        "status": status,
        "path": config_path.to_string_lossy(),
        "model": model,
        "model_provider": model_provider,
        "provider_name": provider_name,
        "default_model": default_model,
        "models": models,
        "sources": source_statuses,
        "responses_api": responses_api
    })
}

fn codex_home_dir() -> PathBuf {
    std::env::var_os("CODEX_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(crate::relay_config::default_codex_home_dir)
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

async fn fetch_models_from_source(
    client: &reqwest::Client,
    source: &ModelSource,
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

    match request.send().await {
        Ok(response) if response.status().is_success() => match response.json::<Value>().await {
            Ok(payload) => {
                let models = unique_strings(parse_model_payload(&payload));
                safe_source["status"] = json!("ok");
                safe_source["models"] = json!(models.len());
                (models, safe_source)
            }
            Err(error) => failed_source(safe_source, error.to_string()),
        },
        Ok(response) => failed_source(safe_source, format!("HTTP {}", response.status().as_u16())),
        Err(error) => failed_source(safe_source, error.to_string()),
    }
}

fn failed_source(mut source: Value, message: String) -> (Vec<String>, Value) {
    source["status"] = json!("failed");
    source["message"] = json!(message);
    source["models"] = json!(0);
    source["responses_api"] = responses_api_status("unknown", "", "");
    (Vec::new(), source)
}

fn preferred_probe_model(configured_model: &str, source_models: &[String]) -> String {
    if !configured_model.is_empty() && source_models.iter().any(|item| item == configured_model) {
        return configured_model.to_string();
    }
    source_models.first().cloned().unwrap_or_default()
}

async fn probe_responses_api_support(
    client: &reqwest::Client,
    source: &ModelSource,
    model: &str,
) -> Value {
    let endpoint = responses_endpoint(&source.base_url);
    if endpoint.is_empty() || model.is_empty() {
        return responses_api_status("unknown", &endpoint, "");
    }
    let mut request = client
        .post(&endpoint)
        .header(reqwest::header::ACCEPT, "application/json")
        .json(&json!({
            "model": model,
            "input": "ping",
            "max_output_tokens": 1
        }));
    if !source.api_key.is_empty() {
        request = request.bearer_auth(&source.api_key);
    }

    match request.send().await {
        Ok(response) => {
            let status = response.status().as_u16();
            if status < 400 {
                return responses_api_status("supported", &endpoint, "");
            }
            let message = response
                .text()
                .await
                .map(error_message_from_body)
                .unwrap_or_default();
            let detail = http_error_message(status, &message);
            if looks_like_unsupported_responses_api(status, &message) {
                responses_api_status("unsupported", &endpoint, &detail)
            } else {
                responses_api_status("failed", &endpoint, &detail)
            }
        }
        Err(error) => responses_api_status("failed", &endpoint, &error.to_string()),
    }
}

fn responses_api_status(status: &str, endpoint: &str, message: &str) -> Value {
    json!({
        "status": status,
        "endpoint": endpoint,
        "message": message
    })
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
    if cleaned.ends_with("/v1") {
        return format!("{cleaned}/models");
    }
    format!("{cleaned}/v1/models")
}

fn responses_endpoint(base_url: &str) -> String {
    let cleaned = safe_url_for_status(base_url)
        .trim_end_matches('/')
        .to_string();
    if cleaned.is_empty() {
        return String::new();
    }
    if cleaned.ends_with("/responses") {
        return cleaned;
    }
    if cleaned.ends_with("/models") {
        return format!("{}/responses", cleaned.trim_end_matches("/models"));
    }
    if cleaned.ends_with("/v1") {
        return format!("{cleaned}/responses");
    }
    format!("{cleaned}/v1/responses")
}

fn error_message_from_body(body: String) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return value
            .pointer("/error/message")
            .or_else(|| value.get("message"))
            .and_then(Value::as_str)
            .unwrap_or(trimmed)
            .to_string();
    }
    trimmed.to_string()
}

fn http_error_message(status: u16, message: &str) -> String {
    if message.trim().is_empty() {
        format!("HTTP {status}")
    } else {
        format!("HTTP {status}: {}", message.trim())
    }
}

fn looks_like_unsupported_responses_api(status: u16, message: &str) -> bool {
    if matches!(status, 404 | 405 | 410 | 501) {
        return true;
    }
    let message = message.to_ascii_lowercase();
    message.contains("/v1/responses")
        && (message.contains("not found")
            || message.contains("unsupported")
            || message.contains("not support")
            || message.contains("does not support")
            || message.contains("unknown endpoint"))
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
                            .filter_map(|key| object.get(*key).and_then(Value::as_str))
                            .find(|value| !value.trim().is_empty())
                            .map(|value| value.trim().to_string())
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
        .filter_map(|key| object.get(*key).and_then(Value::as_str))
        .find(|value| !value.trim().is_empty())
        .map(|value| vec![value.trim().to_string()])
        .unwrap_or_default()
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
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(value)
        .to_string()
}
