//! Agent 接入的只读诊断：磁盘上的当前配置、helper 状态，以及网关实际见到的流量。
//!
//! 没有流量只能说明尚未验证，不能据此断言 Agent 绕过了网关。配置值只输出脱敏后的地址，
//! 凭据只返回「相符 / 不相符」的证据，错误只返回固定分类与 HTTP 状态码。

use std::collections::BTreeSet;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use reqwest::Url;
use serde::Serialize;
use serde_json::Value;

use crate::gateway::activity::{ActivitySnapshot, RouteRecord};
use crate::gateway::agents::files::{self, AgentStash, FileFormat, FileSet};
use crate::gateway::agents::{self, AgentSpec};
use crate::gateway::apply::{ApplyContext, CODEX_GATEWAY_RELAY_ID, gateway_base_url};
use crate::gateway::restore;
use crate::gateway::store::{AgentBinding, GatewaySettings};
use crate::tools::ToolId;

const RECENT_TRAFFIC_MS: u64 = 5 * 60 * 1000;
const MARKER_HOST: &str = "codex-plus-diagnostics.invalid";
const MARKER_TOKEN: &str = "cpx-diagnostics-placeholder";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigEvidence {
    pub path: String,
    pub field: String,
    pub code: String,
    pub endpoint: Option<String>,
    pub expected_endpoint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentError {
    pub kind: String,
    pub status: Option<u16>,
    pub at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDiagnostics {
    /// disabled / configMissing / configUnreadable / configUnverified / configChanged /
    /// gatewayStopped / awaitingTraffic / active / upstreamError。
    pub state: String,
    pub endpoint: Option<String>,
    pub expected_endpoint: String,
    pub config_evidence: Vec<ConfigEvidence>,
    pub requests: u64,
    pub errors: u64,
    pub in_flight: u64,
    pub last_seen_ms: Option<u64>,
    pub recent_error: Option<RecentError>,
}

struct Inspection<'a> {
    ctx: &'a ApplyContext,
    valid: bool,
    endpoint: Option<String>,
    expected_endpoint: String,
    evidence: Vec<ConfigEvidence>,
    modified_at_ms: u64,
}

impl<'a> Inspection<'a> {
    fn new(ctx: &'a ApplyContext, agent: &ToolId) -> Self {
        Self {
            ctx,
            valid: true,
            endpoint: None,
            expected_endpoint: gateway_base_url(ctx.port, agent),
            evidence: Vec::new(),
            modified_at_ms: 0,
        }
    }

    fn watch(&mut self, path: &Path) {
        if let Some(time) = std::fs::metadata(path)
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .and_then(unix_ms)
        {
            self.modified_at_ms = self.modified_at_ms.max(time);
        }
    }

    fn add(
        &mut self,
        path: &Path,
        field: &str,
        code: &str,
        endpoint: Option<String>,
        expected_endpoint: Option<String>,
    ) {
        self.watch(path);
        if self.endpoint.is_none() && endpoint.is_some() {
            self.endpoint = endpoint.clone();
        }
        if !matches!(
            code,
            "endpoint_match" | "credential_match" | "selection_match" | "managed_config_match"
        ) {
            self.valid = false;
        }
        self.evidence.push(ConfigEvidence {
            path: self.ctx.agent_env.display(path),
            field: safe_field(field),
            code: code.to_string(),
            endpoint,
            expected_endpoint,
        });
    }

    fn endpoint(&mut self, path: &Path, field: &str, actual: Option<&str>, expected: &str) {
        let code = endpoint_code(actual, expected);
        self.add(
            path,
            field,
            code,
            actual.and_then(safe_endpoint),
            safe_endpoint(expected),
        );
    }

    fn credential(&mut self, path: &Path, field: &str, matches: bool) {
        self.add(
            path,
            field,
            if matches {
                "credential_match"
            } else {
                "credential_changed"
            },
            None,
            None,
        );
    }

    fn read(&mut self, path: &Path) -> Option<String> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                self.watch(path);
                Some(text)
            }
            Err(error) => {
                self.add(
                    path,
                    "",
                    if error.kind() == std::io::ErrorKind::NotFound {
                        "config_missing"
                    } else {
                        "config_unreadable"
                    },
                    None,
                    None,
                );
                None
            }
        }
    }
}

fn unix_ms(time: SystemTime) -> Option<u64> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64)
}

/// 不写配置、不触发模型探测。`helper_running` 必须来自常驻 helper 的实际状态。
pub fn inspect(
    ctx: &ApplyContext,
    agent: &ToolId,
    gateway: &GatewaySettings,
    helper_running: bool,
    activity: &ActivitySnapshot,
) -> AgentDiagnostics {
    let now = unix_ms(SystemTime::now()).unwrap_or_default();
    let current = activity
        .agents
        .iter()
        .find(|current| current.agent == agent.as_str());
    let last = current.and_then(|current| current.last.as_ref());
    let in_flight = current.map_or(0, |current| current.in_flight.len() as u64);
    let binding = gateway.binding(agent);
    let enabled = binding.is_some_and(|binding| binding.enabled);
    let mut config = Inspection::new(ctx, agent);
    if let Some(binding) = binding.filter(|_| enabled) {
        match agent {
            ToolId::Codex => codex(&mut config, binding),
            ToolId::Claude => claude(&mut config, binding),
            ToolId::Grok => grok(&mut config, binding),
            _ => match agents::spec(agent.as_str()) {
                Some(spec) => generic(&mut config, spec, gateway, binding),
                None => config.add(Path::new(""), "", "unsupported_agent", None, None),
            },
        }
    }

    let recent = last.filter(|last| {
        last.finished_at_ms >= config.modified_at_ms
            && last.finished_at_ms <= now.saturating_add(5_000)
            && now.saturating_sub(last.finished_at_ms) <= RECENT_TRAFFIC_MS
    });
    let state = if !enabled {
        "disabled"
    } else if config
        .evidence
        .iter()
        .any(|item| item.code == "config_missing")
    {
        "configMissing"
    } else if config
        .evidence
        .iter()
        .any(|item| matches!(item.code.as_str(), "config_unreadable" | "config_invalid"))
    {
        "configUnreadable"
    } else if config.evidence.iter().any(|item| {
        matches!(
            item.code.as_str(),
            "config_unverified" | "unsupported_agent"
        )
    }) {
        "configUnverified"
    } else if !config.valid {
        "configChanged"
    } else if !helper_running {
        "gatewayStopped"
    } else if in_flight > 0 {
        "active"
    } else if recent.is_some_and(|last| last.result != "answered") {
        "upstreamError"
    } else if recent.is_some() {
        "active"
    } else {
        "awaitingTraffic"
    };
    AgentDiagnostics {
        state: state.to_string(),
        endpoint: config.endpoint,
        expected_endpoint: config.expected_endpoint,
        config_evidence: config.evidence,
        requests: current.map_or(0, |current| current.requests),
        errors: current.map_or(0, |current| current.errors),
        in_flight,
        last_seen_ms: last.map(|last| last.finished_at_ms),
        recent_error: recent
            .filter(|last| last.result != "answered")
            .map(recent_error),
    }
}

fn codex(inspection: &mut Inspection<'_>, binding: &AgentBinding) {
    let path = inspection.ctx.codex_home.join("config.toml");
    let Some(text) = inspection.read(&path) else {
        return;
    };
    let Ok(value) = restore::toml_value(Some(&text)) else {
        inspection.add(&path, "", "config_invalid", None, None);
        return;
    };
    let provider = value
        .get("model_provider")
        .and_then(Value::as_str)
        .unwrap_or("openai");
    if provider == "openai" {
        let expected = crate::protocol_proxy::local_responses_proxy_base_url(inspection.ctx.port);
        inspection.expected_endpoint = expected.clone();
        inspection.endpoint(
            &path,
            "openai_base_url",
            value.get("openai_base_url").and_then(Value::as_str),
            &expected,
        );
        // /v1 本身不能证明经网关：协议代理按当前供应商决定下一跳。
        let selected = match inspection.ctx.settings.load() {
            Ok(settings) => Some(settings).filter(|settings| {
                settings.relay_profiles_enabled
                    && settings.active_relay_id == CODEX_GATEWAY_RELAY_ID
            }),
            Err(_) => {
                inspection.add(&path, "gateway.upstream", "config_unverified", None, None);
                return;
            }
        };
        inspection.add(
            &path,
            "model_provider",
            if selected.is_some() {
                "selection_match"
            } else {
                "selection_changed"
            },
            None,
            None,
        );
        if let Some(settings) = selected {
            let profile = settings.active_relay_profile();
            let expected = format!("{}/v1", codex_expected_root(inspection.ctx, &text));
            let upstream = crate::relay_config::relay_profile_base_url(&profile);
            inspection.endpoint(&path, "gateway.upstream", Some(&upstream), &expected);
        }
    } else {
        let table = value
            .get("model_providers")
            .and_then(|providers| providers.get(provider));
        let base = codex_expected_root(inspection.ctx, &text);
        let expected = format!("{base}/v1");
        inspection.expected_endpoint = expected.clone();
        inspection.endpoint(
            &path,
            "model_providers.<selected>.base_url",
            table
                .and_then(|table| table.get("base_url"))
                .and_then(Value::as_str),
            &expected,
        );
        inspection.credential(
            &path,
            "model_providers.<selected>.experimental_bearer_token",
            table
                .and_then(|table| table.get("experimental_bearer_token"))
                .and_then(Value::as_str)
                == Some(binding.token.as_str()),
        );
    }
}

fn codex_expected_root(ctx: &ApplyContext, config: &str) -> String {
    let host = crate::wsl_network::codex_runs_in_wsl_config(config)
        .then(crate::wsl_network::wsl_listener_address)
        .flatten()
        .filter(|(_, port)| *port == ctx.port)
        .map(|(address, _)| address.to_string())
        .unwrap_or_else(|| crate::wsl_network::LOOPBACK_HOST.to_string());
    crate::gateway::apply::gateway_base_url_on(&host, ctx.port, &ToolId::Codex)
}

fn claude(inspection: &mut Inspection<'_>, binding: &AgentBinding) {
    let path = inspection.ctx.claude.claude_home.join("settings.json");
    let Some(text) = inspection.read(&path) else {
        return;
    };
    let Ok(value) = files::parse_jsonc(&text) else {
        inspection.add(&path, "", "config_invalid", None, None);
        return;
    };
    let expected = gateway_base_url(inspection.ctx.port, &ToolId::Claude);
    inspection.endpoint(
        &path,
        "env.ANTHROPIC_BASE_URL",
        value
            .pointer("/env/ANTHROPIC_BASE_URL")
            .and_then(Value::as_str),
        &expected,
    );
    inspection.credential(
        &path,
        "env.ANTHROPIC_AUTH_TOKEN",
        value
            .pointer("/env/ANTHROPIC_AUTH_TOKEN")
            .and_then(Value::as_str)
            == Some(binding.token.as_str()),
    );
}

fn grok(inspection: &mut Inspection<'_>, binding: &AgentBinding) {
    let path = inspection.ctx.grok_home.join("config.toml");
    let Some(text) = inspection.read(&path) else {
        return;
    };
    let Ok(value) = restore::toml_value(Some(&text)) else {
        inspection.add(&path, "", "config_invalid", None, None);
        return;
    };
    let selected = value
        .pointer("/models/default")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let table = value.get("model").and_then(|models| models.get(selected));
    let expected = format!(
        "{}/v1",
        gateway_base_url(inspection.ctx.port, &ToolId::Grok)
    );
    inspection.expected_endpoint = expected.clone();
    inspection.endpoint(
        &path,
        "model.<selected>.base_url",
        table
            .and_then(|table| table.get("base_url"))
            .and_then(Value::as_str),
        &expected,
    );
    inspection.credential(
        &path,
        "model.<selected>.api_key",
        table
            .and_then(|table| table.get("api_key"))
            .and_then(Value::as_str)
            == Some(binding.token.as_str()),
    );
}

fn generic(
    inspection: &mut Inspection<'_>,
    spec: &AgentSpec,
    gateway: &GatewaySettings,
    binding: &AgentBinding,
) {
    let main = (spec.config_path)(&inspection.ctx.agent_env);
    inspection.watch(&main);
    if let Some(saved) =
        restore::load_stash::<AgentStash>(&agents::connect::stash_path(inspection.ctx, spec.id))
    {
        for path in saved.files.keys() {
            inspection.watch(path);
        }
    }
    let mut wiring = agents::connect::wiring(gateway, binding, spec.id, inspection.ctx.port);
    let expected_root = wiring.root.clone();
    wiring.root = format!("http://{MARKER_HOST}/gw/{}", spec.id);
    wiring.token = MARKER_TOKEN.to_string();
    // OpenPencil 的写入规格禁止应用运行时修改配置；这里只读投影，不受这个写入保护限制。
    let env = if spec.id == "openpencil" {
        inspection.ctx.agent_env.clone().with_var("PATH", "")
    } else {
        inspection.ctx.agent_env.clone()
    };
    let mut projected = FileSet::new();
    let projection = (spec.write)(&env, &wiring, &mut projected).and_then(|_| projected.changes());
    let Ok(changes) = projection else {
        inspection.add(&main, "", "config_invalid", None, None);
        return;
    };
    let mut fallback = Vec::new();
    for change in changes {
        let Some(current) = inspection.read(&change.path) else {
            continue;
        };
        let projected = change.after.as_deref().unwrap_or_default();
        let values = match change.format {
            FileFormat::Json => files::parse_jsonc(projected)
                .ok()
                .zip(files::parse_jsonc(&current).ok()),
            FileFormat::Toml => restore::toml_value(Some(projected))
                .ok()
                .zip(restore::toml_value(Some(&current)).ok()),
            FileFormat::Raw => files::parse_jsonc(projected)
                .ok()
                .zip(files::parse_jsonc(&current).ok()),
            _ => None,
        };
        if let Some((projected, current)) = values {
            inspect_values(
                inspection,
                &change.path,
                &projected,
                Some(&current),
                &mut Vec::new(),
                &wiring.root,
                &expected_root,
                &binding.token,
            );
        } else if change.format == FileFormat::Env {
            let expected = files::content_value(FileFormat::Env, Some(projected)).ok();
            let live = files::EnvDoc::parse(&current);
            if let Some(Value::Object(expected)) = expected {
                for (key, _) in expected {
                    let projected = files::EnvDoc::parse(projected)
                        .get(&key)
                        .unwrap_or_default();
                    let actual = live.get(&key);
                    inspect_scalar(
                        inspection,
                        &change.path,
                        &key,
                        &projected,
                        actual.as_deref(),
                        &wiring.root,
                        &expected_root,
                        &binding.token,
                    );
                }
            }
        } else {
            fallback.push(change.path);
        }
    }
    // YAML / 自管文本沿用规格自己的结构化读写器作只读投影比较，不扫描注释里的 URL。
    if !fallback.is_empty() {
        let wiring = agents::connect::wiring(gateway, binding, spec.id, inspection.ctx.port);
        let mut set = FileSet::new();
        let changed = (spec.write)(&env, &wiring, &mut set)
            .and_then(|_| set.changes())
            .map(|changes| {
                changes
                    .into_iter()
                    .map(|change| change.path)
                    .collect::<BTreeSet<_>>()
            });
        for path in fallback {
            let matches = changed
                .as_ref()
                .is_ok_and(|changed| !changed.contains(&path));
            inspection.add(
                &path,
                "",
                if matches {
                    "managed_config_match"
                } else {
                    "managed_config_changed"
                },
                None,
                Some(expected_root.clone()),
            );
        }
    }
    if inspection.evidence.is_empty() {
        inspection.add(&main, "", "config_unverified", None, None);
    }
}

#[allow(clippy::too_many_arguments)]
fn inspect_values(
    inspection: &mut Inspection<'_>,
    path: &Path,
    projected: &Value,
    current: Option<&Value>,
    field: &mut Vec<String>,
    marker_root: &str,
    expected_root: &str,
    token: &str,
) {
    match projected {
        Value::Object(entries) => {
            for (key, value) in entries {
                field.push(key.clone());
                inspect_values(
                    inspection,
                    path,
                    value,
                    current.and_then(|current| current.get(key)),
                    field,
                    marker_root,
                    expected_root,
                    token,
                );
                field.pop();
            }
        }
        Value::Array(entries) => {
            for (index, value) in entries.iter().enumerate() {
                let current = current.and_then(Value::as_array).and_then(|current| {
                    let identity = ["id", "name", "provider_source_id", "vendor", "providerId"]
                        .iter()
                        .find_map(|key| {
                            value
                                .get(*key)
                                .and_then(Value::as_str)
                                .map(|value| (*key, value))
                        });
                    match identity {
                        Some((key, id)) => current
                            .iter()
                            .find(|item| item.get(key).and_then(Value::as_str) == Some(id)),
                        None => current.get(index),
                    }
                });
                field.push(index.to_string());
                inspect_values(
                    inspection,
                    path,
                    value,
                    current,
                    field,
                    marker_root,
                    expected_root,
                    token,
                );
                field.pop();
            }
        }
        Value::String(projected) => inspect_scalar(
            inspection,
            path,
            &field.join("."),
            projected,
            current.and_then(Value::as_str),
            marker_root,
            expected_root,
            token,
        ),
        _ => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn inspect_scalar(
    inspection: &mut Inspection<'_>,
    path: &Path,
    field: &str,
    projected: &str,
    current: Option<&str>,
    marker_root: &str,
    expected_root: &str,
    token: &str,
) {
    if projected.starts_with(marker_root) {
        let expected = projected.replacen(marker_root, expected_root, 1);
        inspection.endpoint(path, field, current, &expected);
    } else if projected.contains(MARKER_TOKEN) {
        let expected = projected.replace(MARKER_TOKEN, token);
        inspection.credential(path, field, current == Some(expected.as_str()));
    } else if provider_reference(projected)
        && (selection_field(field.rsplit('.').next().unwrap_or_default())
            || field.contains("enabled_providers."))
    {
        inspection.add(
            path,
            field,
            if current.is_some_and(|value| same_provider_reference(projected, value)) {
                "selection_match"
            } else {
                "selection_changed"
            },
            None,
            None,
        );
    } else if strict_selection_field(field.rsplit('.').next().unwrap_or_default()) {
        inspection.add(
            path,
            field,
            if current == Some(projected) {
                "selection_match"
            } else {
                "selection_changed"
            },
            None,
            None,
        );
    }
}

fn provider_reference(value: &str) -> bool {
    value == agents::PROVIDER_ID
        || value.starts_with("codexplus/")
        || value == "codexplus-openchamber"
        || value.starts_with("codexplus-openchamber/")
        || value.starts_with("custom_provider:codexplus/")
}

fn same_provider_reference(expected: &str, actual: &str) -> bool {
    if let Some((provider, _)) = expected.split_once('/') {
        actual
            .strip_prefix(provider)
            .is_some_and(|rest| rest.starts_with('/'))
    } else {
        actual == expected
    }
}

fn strict_selection_field(field: &str) -> bool {
    matches!(
        field.to_ascii_lowercase().as_str(),
        "selectedtype" | "authmode" | "authtype" | "apiprovider" | "lastusedprovider"
    )
}

fn selection_field(field: &str) -> bool {
    matches!(
        field.to_ascii_lowercase().as_str(),
        "model"
            | "modelprovider"
            | "defaultmodel"
            | "defaultprovider"
            | "default_model"
            | "default_provider"
            | "provider"
            | "provider_id"
            | "providerid"
            | "activeprovider"
            | "active_provider"
            | "activeprofile"
            | "default_provider_id"
            | "lastusedprovider"
            | "small_model"
            | "primary"
    )
}

fn safe_field(field: &str) -> String {
    field
        .split('.')
        .map(|part| {
            let known = matches!(
                part.to_ascii_lowercase().as_str(),
                "" | "env"
                    | "model"
                    | "models"
                    | "model_providers"
                    | "gateway"
                    | "upstream"
                    | "provider"
                    | "providers"
                    | "provider_sources"
                    | "provider_settings"
                    | "provider_accounts"
                    | "options"
                    | "access"
                    | "api"
                    | "requestheaders"
                    | "http_headers"
                    | "headers"
                    | "authorization"
                    | "x-api-key"
                    | "name"
                    | "value"
                    | "settings"
                    | "authentication"
                    | "auth"
                    | "selectedtype"
                    | "authmode"
                    | "authtype"
                    | "apiprovider"
                    | "security"
                    | "root"
                    | "codexplus"
                    | "codex-plus-gateway"
                    | "custom_provider"
                    | "custommodels"
                    | "custom_models"
                    | "connections"
                    | "accounts"
                    | "config"
                    | "base_url"
                    | "baseurl"
                    | "endpoint"
                    | "url"
                    | "api_url"
                    | "api_base"
                    | "modelsapi"
                    | "inferencegatewaybaseurl"
                    | "openaibaseurl"
                    | "modelprovider"
                    | "defaultmodel"
                    | "defaultprovider"
                    | "default_model"
                    | "default_provider"
                    | "provider_id"
                    | "providerid"
                    | "activeprovider"
                    | "active_provider"
                    | "activeprofile"
                    | "default_provider_id"
                    | "lastusedprovider"
                    | "small_model"
                    | "primary"
                    | "api_key"
                    | "apikey"
                    | "experimental_bearer_token"
                    | "anthropic_base_url"
                    | "anthropic_auth_token"
                    | "google_gemini_base_url"
                    | "gemini_api_key"
                    | "openai_base_url"
                    | "openai_api_key"
                    | "auth_token"
                    | "inferencegatewayapikey"
                    | "openaapikey"
                    | "token"
                    | "key"
            );
            if known || part.bytes().all(|byte| byte.is_ascii_digit()) {
                part
            } else {
                "*"
            }
        })
        .collect::<Vec<_>>()
        .join(".")
}

fn endpoint_code(actual: Option<&str>, expected: &str) -> &'static str {
    let Some(actual) = actual.filter(|actual| !actual.trim().is_empty()) else {
        return "endpoint_missing";
    };
    let Ok(actual) = Url::parse(actual.trim()) else {
        return "endpoint_invalid";
    };
    let Ok(expected) = Url::parse(expected) else {
        return "endpoint_invalid";
    };
    if !actual.username().is_empty()
        || actual.password().is_some()
        || actual.query().is_some()
        || actual.fragment().is_some()
    {
        return "endpoint_changed";
    }
    if actual.scheme() != expected.scheme() || actual.host() != expected.host() {
        return "endpoint_host_mismatch";
    }
    if actual.port_or_known_default() != expected.port_or_known_default() {
        return "endpoint_port_mismatch";
    }
    if actual.path().trim_end_matches('/') != expected.path().trim_end_matches('/') {
        return "endpoint_path_mismatch";
    }
    "endpoint_match"
}

/// 仅供界面展示的地址：去除用户身份、密码、查询参数和片段，非标准路径段替换为占位符。
///
/// 无法解析或不是 HTTP(S) 时返回 None，调用方不能回退为原始字符串（其中可能有凭据）。
pub fn safe_endpoint(value: &str) -> Option<String> {
    let mut url = Url::parse(value.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return None;
    }
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);
    let mut gateway_id = false;
    let path = url
        .path_segments()?
        .map(|segment| {
            let allowed = matches!(
                segment,
                "" | "api"
                    | "v1"
                    | "v1beta"
                    | "openai"
                    | "anthropic"
                    | "responses"
                    | "messages"
                    | "chat"
                    | "completions"
                    | "models"
                    | "gw"
            ) || gateway_id
                && (matches!(segment, "codex" | "claude" | "grok")
                    || agents::spec(segment).is_some());
            gateway_id = segment == "gw";
            if allowed { segment } else { "[redacted]" }
        })
        .collect::<Vec<_>>()
        .join("/");
    url.set_path(&path);
    Some(url.to_string().trim_end_matches('/').to_string())
}

fn recent_error(record: &RouteRecord) -> RecentError {
    let step = record
        .steps
        .iter()
        .rev()
        .find(|step| step.outcome != "answered");
    let status = step.and_then(|step| step.status);
    let kind = match step.and_then(|step| step.failure.as_deref()) {
        Some("auth_invalid") => "auth_invalid",
        Some("rate_limited") => "rate_limited",
        Some("quota_exhausted") => "quota_exhausted",
        Some("server_error") => "server_error",
        Some("client_error") => "client_error",
        _ if record.result == "cooling" => "cooling",
        _ if step.is_some_and(|step| step.outcome == "transport") => "transport",
        _ if matches!(status, Some(401 | 403)) => "auth_invalid",
        _ if status == Some(429) => "rate_limited",
        _ if status.is_some_and(|status| status >= 500) => "server_error",
        _ => "upstream_error",
    };
    RecentError {
        kind: kind.to_string(),
        status,
        at_ms: record.finished_at_ms,
    }
}
