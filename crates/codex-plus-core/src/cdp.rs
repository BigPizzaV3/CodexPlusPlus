use anyhow::{Context, bail};
use serde::Deserialize;
use std::time::Duration;

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

    let urls = [
        format!("http://127.0.0.1:{debug_port}/json"),
        format!("http://[::1]:{debug_port}/json"),
    ];
    let mut errors = Vec::new();
    for url in urls {
        match query_targets_url(&client, &url).await {
            Ok(targets) => return Ok(targets),
            Err(error) => errors.push(format!("{url}: {error:#}")),
        }
    }

    bail!(
        "failed to query CDP targets on loopback addresses: {}",
        errors.join("; ")
    )
}

async fn query_targets_url(client: &reqwest::Client, url: &str) -> anyhow::Result<Vec<CdpTarget>> {
    let response = client
        .get(url)
        .send()
        .await
        .context("failed to query CDP targets")?
        .error_for_status()
        .context("CDP target query failed")?;

    response
        .json::<Vec<CdpTarget>>()
        .await
        .context("failed to deserialize CDP targets")
}

pub fn pick_page_target(targets: &[CdpTarget]) -> anyhow::Result<CdpTarget> {
    let mut first_page = None;
    for target in targets
        .iter()
        .filter(|target| is_injectable_page_target(target))
    {
        first_page.get_or_insert(target);
        if is_codex_page_target(target) {
            return Ok(target.clone());
        }
    }

    if let Some(target) = first_page {
        return Ok(target.clone());
    }

    bail!("No injectable page target found")
}

pub fn pick_injectable_codex_page_target(targets: &[CdpTarget]) -> anyhow::Result<CdpTarget> {
    for target in targets
        .iter()
        .filter(|target| is_injectable_page_target(target))
    {
        if is_codex_page_target(target) {
            return Ok(target.clone());
        }
    }

    bail!("No injectable Codex page target found")
}

pub fn is_injectable_page_target(target: &CdpTarget) -> bool {
    target.target_type == "page"
        && target
            .web_socket_debugger_url
            .as_deref()
            .is_some_and(|url| !url.is_empty())
}

pub fn is_codex_page_target(target: &CdpTarget) -> bool {
    if target.target_type != "page" {
        return false;
    }
    let haystack = format!("{} {}", target.title, target.url).to_lowercase();
    haystack.contains("codex")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(target_type: &str, title: &str, url: &str, ws: Option<&str>) -> CdpTarget {
        CdpTarget {
            id: "t".to_string(),
            target_type: target_type.to_string(),
            title: title.to_string(),
            url: url.to_string(),
            web_socket_debugger_url: ws.map(str::to_string),
        }
    }

    // The CSP bypass in `bridge::install_bridge` (Page.setBypassCSP) is per-CDP-session and
    // therefore only affects whichever target `install_bridge` attaches to. That target is
    // exclusively chosen by `pick_injectable_codex_page_target`, so these tests pin down the
    // scope guarantee: injection (and the CSP relaxation riding on it) never reaches a
    // non-Codex or non-page target.

    #[test]
    fn pick_selects_the_codex_page_and_skips_a_non_codex_page() {
        let targets = vec![
            target("page", "Some Website", "https://example.com", Some("ws://x/1")),
            target("page", "Codex", "app://-/index.html", Some("ws://x/2")),
        ];
        let picked = pick_injectable_codex_page_target(&targets).expect("codex page present");
        assert_eq!(picked.web_socket_debugger_url.as_deref(), Some("ws://x/2"));
    }

    #[test]
    fn pick_rejects_when_only_non_codex_pages_exist() {
        let targets = vec![
            target("page", "Gmail", "https://mail.google.com", Some("ws://x/1")),
            target("page", "Docs", "https://docs.example.com", Some("ws://x/2")),
        ];
        assert!(
            pick_injectable_codex_page_target(&targets).is_err(),
            "no Codex page => no injection target => CSP is never bypassed on other pages"
        );
    }

    #[test]
    fn pick_rejects_a_page_without_a_websocket_url() {
        let targets = vec![target("page", "Codex", "app://-/index.html", None)];
        assert!(pick_injectable_codex_page_target(&targets).is_err());
    }

    #[test]
    fn codex_named_but_non_page_targets_are_not_injectable() {
        // A worker/iframe whose title mentions "codex" must not be treated as an injectable page.
        assert!(!is_injectable_page_target(&target(
            "service_worker",
            "codex worker",
            "",
            Some("ws://x/1")
        )));
        assert!(!is_codex_page_target(&target(
            "iframe",
            "Codex helper",
            "app://-/frame",
            Some("ws://x/1")
        )));
    }
}
