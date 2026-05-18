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
    let url = format!("http://127.0.0.1:{debug_port}/json");
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(CDP_HTTP_TIMEOUT)
        .build()
        .context("failed to build CDP HTTP client")?;
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
    let pages = targets.iter().filter(|target| {
        target.target_type == "page"
            && target
                .web_socket_debugger_url
                .as_deref()
                .is_some_and(|url| !url.is_empty())
    });

    let mut first_page = None;
    for target in pages {
        first_page.get_or_insert(target);
        let haystack = format!("{} {}", target.title, target.url).to_lowercase();
        if haystack.contains("codex") {
            return Ok(target.clone());
        }
    }

    if let Some(target) = first_page {
        return Ok(target.clone());
    }

    bail!("No injectable Codex page target found")
}
