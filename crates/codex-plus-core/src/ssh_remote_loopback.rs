//! SSH 远端连接与本机回环中转地址的冲突提示（issue #1195）。
//!
//! Codex 的「设置 → 连接 → SSH 服务器」由 Codex 自己建立和维护，Codex++ 不往远端写任何配置。
//! 但本机 config.toml 里的中转地址常是 `http://127.0.0.1:<端口>`（Codex++ 本地代理 / 网关），
//! 这类地址一旦被带到远端 Codex 的配置里，就指向服务器自己，请求会静默失败。
//! 这里只做只读检测，并给出 `ssh -R` 反向端口转发的做法，不改任何配置。

use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeSet;
use std::net::IpAddr;
use std::path::Path;
use toml_edit::{DocumentMut, Item};

/// 主机解析不出来时命令里的占位符。
const REMOTE_PLACEHOLDER: &str = "user@server";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshRemoteLoopbackNotice {
    /// 本机 config.toml 里指向回环地址的配置项。
    pub loopback_endpoints: Vec<LoopbackEndpoint>,
    /// 需要反向转发的本机端口（去重、升序）。
    pub ports: Vec<u16>,
    /// Codex 里登记的 SSH 远端，以及各自可复制的转发命令。
    pub remotes: Vec<SshRemoteForwardHint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoopbackEndpoint {
    /// 配置项位置，如 `openai_base_url`、`model_providers.custom.base_url`。
    pub key: String,
    pub url: String,
    pub port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshRemoteForwardHint {
    pub label: String,
    /// 连接时手动带上的反向转发命令，例如 `ssh -R 57321:127.0.0.1:57321 user@host`。
    pub forward_command: String,
    /// 想让 Codex 自己发起的 SSH 连接也带上转发时，写进 `~/.ssh/config` 对应 Host 下的行。
    pub ssh_config_lines: Vec<String>,
}

/// 读取 `<codex_home>/config.toml` 与 Codex 的全局状态，判断是否需要提示。
/// 任一文件缺失或无法解析都视为无需提示。
pub fn inspect_ssh_remote_loopback_for_home(codex_home: &Path) -> Option<SshRemoteLoopbackNotice> {
    let config = std::fs::read_to_string(codex_home.join("config.toml")).ok()?;
    let state = std::fs::read_to_string(codex_home.join(".codex-global-state.json")).ok()?;
    let state: Value = serde_json::from_str(&state).ok()?;
    ssh_remote_loopback_notice(&config, &state)
}

/// 本机配置里有回环中转地址、且 Codex 登记了 SSH 远端时返回提示；否则返回 None。
pub fn ssh_remote_loopback_notice(
    config: &str,
    global_state: &Value,
) -> Option<SshRemoteLoopbackNotice> {
    let loopback_endpoints = loopback_endpoints_from_config(config);
    if loopback_endpoints.is_empty() {
        return None;
    }
    let remotes = ssh_remotes_from_global_state(global_state);
    if remotes.is_empty() {
        return None;
    }
    let ports = loopback_endpoints
        .iter()
        .map(|endpoint| endpoint.port)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    // 每个端口转发到它在本机配置里实际写的回环主机（127.0.0.1 / localhost / [::1]）。
    let forwards = ports
        .iter()
        .map(|port| {
            let host = loopback_endpoints
                .iter()
                .find(|endpoint| endpoint.port == *port)
                .map(|endpoint| forward_host(&endpoint.url))
                .unwrap_or_else(|| "127.0.0.1".to_string());
            (*port, host)
        })
        .collect::<Vec<_>>();
    let remotes = remotes
        .into_iter()
        .map(|remote| forward_hint(remote, &forwards))
        .collect();
    Some(SshRemoteLoopbackNotice {
        loopback_endpoints,
        ports,
        remotes,
    })
}

/// URL 的主机是否是本机回环地址：`localhost`（含 `*.localhost`）、`127.0.0.0/8`、`[::1]`。
pub fn is_loopback_base_url(url: &str) -> bool {
    loopback_port(url).is_some()
}

fn loopback_port(url: &str) -> Option<u16> {
    let parsed = reqwest::Url::parse(url.trim()).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    let host = parsed.host_str()?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let loopback = host.eq_ignore_ascii_case("localhost")
        || host.to_ascii_lowercase().ends_with(".localhost")
        || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback());
    loopback.then(|| parsed.port_or_known_default()).flatten()
}

fn forward_host(url: &str) -> String {
    reqwest::Url::parse(url.trim())
        .ok()
        .and_then(|parsed| {
            let host = parsed.host_str()?.to_string();
            Some(if host.contains(':') && !host.starts_with('[') {
                format!("[{host}]")
            } else {
                host
            })
        })
        .unwrap_or_else(|| "127.0.0.1".to_string())
}

/// 当前生效的中转地址：根上的 `openai_base_url`，以及活动 provider（openai 会话身份时是
/// 承载传输的 `custom`）的 `base_url`。
fn loopback_endpoints_from_config(config: &str) -> Vec<LoopbackEndpoint> {
    let Ok(doc) = config.trim_start_matches('\u{feff}').parse::<DocumentMut>() else {
        return Vec::new();
    };
    let mut candidates = Vec::new();
    if let Some(url) = doc.get("openai_base_url").and_then(Item::as_str) {
        candidates.push(("openai_base_url".to_string(), url.to_string()));
    }
    let active = doc
        .get("model_provider")
        .and_then(Item::as_str)
        .map(str::trim)
        .filter(|provider| !provider.is_empty())
        .unwrap_or("custom");
    let mut provider_ids = vec![active];
    if active == "openai" {
        provider_ids.push("custom");
    }
    for provider_id in provider_ids {
        if let Some(url) = doc
            .get("model_providers")
            .and_then(|providers| providers.get(provider_id))
            .and_then(|provider| provider.get("base_url"))
            .and_then(Item::as_str)
        {
            candidates.push((
                format!("model_providers.{provider_id}.base_url"),
                url.to_string(),
            ));
        }
    }
    candidates
        .into_iter()
        .filter_map(|(key, url)| {
            let port = loopback_port(&url)?;
            Some(LoopbackEndpoint {
                key,
                url: url.trim().to_string(),
                port,
            })
        })
        .collect()
}

struct SshRemote {
    label: String,
    /// 可直接交给 ssh 的目标（`user@host` 或 ssh config 里的别名），解析不出来时为 None。
    destination: Option<String>,
    port: Option<u16>,
}

fn string_field(connection: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| match connection.get(*key) {
        Some(Value::String(value)) => Some(value.trim().to_string()).filter(|v| !v.is_empty()),
        Some(Value::Number(value)) => Some(value.to_string()),
        _ => None,
    })
}

/// 只接受普通的主机名 / 别名字符，避免把奇怪的值拼进可复制的命令里。
fn safe_ssh_token(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('-')
        && value.chars().all(|ch| {
            ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_' | ':' | '[' | ']' | '@')
        })
}

fn ssh_destination(connection: &serde_json::Map<String, Value>) -> (Option<String>, Option<u16>) {
    let port =
        string_field(connection, &["sshPort", "port"]).and_then(|port| port.parse::<u16>().ok());
    // ssh config 里的别名本身就能连（用户、端口都在别名里），优先用它。
    if let Some(alias) =
        string_field(connection, &["sshAlias", "alias"]).filter(|alias| safe_ssh_token(alias))
    {
        return (Some(alias), port);
    }
    let Some(host) =
        string_field(connection, &["sshHost", "hostname"]).filter(|host| safe_ssh_token(host))
    else {
        return (None, port);
    };
    if host.contains('@') {
        return (Some(host), port);
    }
    let destination =
        match string_field(connection, &["sshUser", "user"]).filter(|user| safe_ssh_token(user)) {
            Some(user) => format!("{user}@{host}"),
            None => host,
        };
    (Some(destination), port)
}

fn ssh_remotes_from_global_state(state: &Value) -> Vec<SshRemote> {
    let mut remotes: Vec<SshRemote> = Vec::new();
    let connections = state
        .get("codex-managed-remote-connections")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    for connection in connections.iter().filter_map(Value::as_object) {
        let (destination, port) = ssh_destination(connection);
        let label = string_field(
            connection,
            &[
                "displayName",
                "label",
                "name",
                "sshAlias",
                "alias",
                "sshHost",
                "hostname",
                "hostId",
            ],
        )
        .unwrap_or_else(|| "SSH".to_string());
        if remotes.iter().any(|remote| remote.label == label) {
            continue;
        }
        remotes.push(SshRemote {
            label,
            destination,
            port,
        });
    }
    // 只有远端项目、没有连接记录时（旧版状态），仍要提示，只是主机用占位符。
    let has_remote_projects = state
        .get("remote-projects")
        .and_then(Value::as_array)
        .is_some_and(|projects| !projects.is_empty());
    if remotes.is_empty() && has_remote_projects {
        remotes.push(SshRemote {
            label: "SSH".to_string(),
            destination: None,
            port: None,
        });
    }
    remotes
}

fn forward_hint(remote: SshRemote, forwards: &[(u16, String)]) -> SshRemoteForwardHint {
    let mut command = vec!["ssh".to_string()];
    for (port, host) in forwards {
        command.push("-R".to_string());
        command.push(format!("{port}:{host}:{port}"));
    }
    if let Some(port) = remote.port {
        command.push("-p".to_string());
        command.push(port.to_string());
    }
    command.push(
        remote
            .destination
            .clone()
            .unwrap_or_else(|| REMOTE_PLACEHOLDER.to_string()),
    );
    let ssh_config_lines = forwards
        .iter()
        .map(|(port, host)| format!("RemoteForward {port} {host}:{port}"))
        .collect();
    SshRemoteForwardHint {
        label: remote.label,
        forward_command: command.join(" "),
        ssh_config_lines,
    }
}
