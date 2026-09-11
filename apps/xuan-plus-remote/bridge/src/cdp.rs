use std::net::IpAddr;
use std::time::Duration;

use anyhow::{Context, bail};
use serde::Deserialize;

const CDP_HTTP_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct CdpTarget {
    pub id: String,
    #[serde(rename = "type")]
    pub target_type: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub url: String,
    #[serde(default, rename = "webSocketDebuggerUrl")]
    pub web_socket_debugger_url: Option<String>,
}

pub async fn list_targets(debug_port: u16) -> anyhow::Result<Vec<CdpTarget>> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(CDP_HTTP_TIMEOUT)
        .build()
        .context("failed to build CDP HTTP client")?;
    let mut errors = Vec::new();
    for url in [
        format!("http://127.0.0.1:{debug_port}/json"),
        format!("http://[::1]:{debug_port}/json"),
    ] {
        let result = async {
            let targets = client
                .get(&url)
                .send()
                .await?
                .error_for_status()?
                .json::<Vec<CdpTarget>>()
                .await?;
            for target in &targets {
                if let Some(websocket) = target.web_socket_debugger_url.as_deref() {
                    validate_cdp_websocket_url(websocket, debug_port)?;
                }
            }
            anyhow::Ok(targets)
        }
        .await;
        match result {
            Ok(targets) => return Ok(targets),
            Err(error) => errors.push(format!("{url}: {error:#}")),
        }
    }
    bail!("failed to query CDP targets: {}", errors.join("; "))
}

pub fn validate_cdp_websocket_url(url: &str, expected_port: u16) -> anyhow::Result<()> {
    let parsed = url::Url::parse(url).context("invalid CDP WebSocket URL")?;
    if parsed.scheme() != "ws" || !parsed.username().is_empty() || parsed.password().is_some() {
        bail!("CDP WebSocket URL must be an unauthenticated ws URL");
    }
    let host = parsed.host_str().context("CDP WebSocket URL has no host")?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let address: IpAddr = host
        .parse()
        .context("CDP WebSocket host must be an IP address")?;
    if !address.is_loopback() || parsed.port_or_known_default() != Some(expected_port) {
        bail!("CDP WebSocket URL must use the expected loopback port");
    }
    if !parsed.path().starts_with("/devtools/page/") {
        bail!("CDP WebSocket URL is not a page target");
    }
    Ok(())
}

pub fn is_primary_codex_page_target(target: &CdpTarget) -> bool {
    target.target_type == "page"
        && target
            .url
            .trim()
            .to_ascii_lowercase()
            .starts_with("app://-/")
        && !target.url.to_ascii_lowercase().contains("avatar-overlay")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_loopback_or_wrong_port() {
        assert!(validate_cdp_websocket_url("ws://127.0.0.1:9222/devtools/page/main", 9222).is_ok());
        assert!(
            validate_cdp_websocket_url("ws://192.168.1.2:9222/devtools/page/main", 9222).is_err()
        );
        assert!(
            validate_cdp_websocket_url("ws://127.0.0.1:9223/devtools/page/main", 9222).is_err()
        );
    }
}
