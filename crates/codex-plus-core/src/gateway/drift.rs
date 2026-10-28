//! 已接入网关的 Agent，配置是否还指向网关。
//!
//! 别的工具（例如 CC Switch）接管了 Agent 的配置后，Agent 的请求就不再经过 Codex++，
//! 网关这边却还以为「已接入」。这里读 Agent 的配置文件做判断，界面据此提示并给「重新接入」。

use std::path::Path;

use serde::Serialize;
use serde_json::Value;

use crate::gateway::apply::ApplyContext;
use crate::tools::ToolId;

/// 配置被改掉了：现在连到哪、像是哪个工具改的。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Drift {
    /// 认得出的工具名（如 `CC Switch`）；认不出为 None。
    pub by: Option<String>,
    /// Agent 现在实际连的地址；读不到为空。
    pub endpoint: String,
}

/// 按地址或文件名认出常见的配置管理工具。
fn known_tool(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    if lower.contains(":15721") || lower.contains("cc-switch") || lower.contains("ccs-") {
        return Some("CC Switch".to_string());
    }
    None
}

fn drift(endpoint: String, evidence: &str) -> Option<Drift> {
    Some(Drift {
        by: known_tool(&endpoint).or_else(|| known_tool(evidence)),
        endpoint,
    })
}

/// 开着网关的 Agent，配置是否还指向网关；指向网关（或没开）时返回 None。
pub fn check(ctx: &ApplyContext, agent: &ToolId) -> Option<Drift> {
    match agent {
        ToolId::Codex => codex(&ctx.codex_home, ctx.port),
        ToolId::Claude => claude(&ctx.claude.claude_home),
        ToolId::Grok => grok(&ctx.grok_home),
        _ => None,
    }
}

fn codex(home: &Path, port: u16) -> Option<Drift> {
    let text = std::fs::read_to_string(home.join("config.toml")).unwrap_or_default();
    let Ok(doc) = text.parse::<toml_edit::DocumentMut>() else {
        return drift(String::new(), &text);
    };
    let provider = doc
        .get("model_provider")
        .and_then(toml_edit::Item::as_str)
        .unwrap_or("openai");
    let endpoint = if provider == "openai" {
        doc.get("openai_base_url")
            .and_then(toml_edit::Item::as_str)
            .unwrap_or_default()
    } else {
        doc.get("model_providers")
            .and_then(|providers| providers.get(provider))
            .and_then(|table| table.get("base_url"))
            .and_then(toml_edit::Item::as_str)
            .unwrap_or_default()
    }
    .trim()
    .to_string();
    // 官方身份：openai_base_url 指向本地代理，由它转给网关；自定义供应商：直接指向网关。
    let ours = endpoint.contains("/gw/codex")
        || (provider == "openai"
            && (crate::protocol_proxy::is_managed_protocol_proxy_base_url(&endpoint)
                || endpoint.trim_end_matches('/')
                    == crate::protocol_proxy::local_responses_proxy_base_url(port)
                        .trim_end_matches('/')));
    if ours {
        return None;
    }
    let catalog = doc
        .get("model_catalog_json")
        .and_then(toml_edit::Item::as_str)
        .unwrap_or_default();
    drift(endpoint, catalog)
}

fn claude(claude_home: &Path) -> Option<Drift> {
    let text = std::fs::read_to_string(claude_home.join("settings.json")).unwrap_or_default();
    let endpoint = serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|value| {
            value
                .pointer("/env/ANTHROPIC_BASE_URL")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_default();
    if endpoint.contains("/gw/claude") {
        return None;
    }
    drift(endpoint, &text)
}

fn grok(grok_home: &Path) -> Option<Drift> {
    let text = std::fs::read_to_string(grok_home.join("config.toml")).unwrap_or_default();
    if text.contains("/gw/grok") {
        return None;
    }
    let endpoint = text
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once('=')?;
            key.trim()
                .ends_with("base_url")
                .then(|| value.trim().trim_matches('"').to_string())
        })
        .unwrap_or_default();
    drift(endpoint, &text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_on_the_gateway_is_not_drifted() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        // 官方身份：openai_base_url 指向本地代理。
        std::fs::write(
            home.join("config.toml"),
            "model_provider = \"openai\"\nopenai_base_url = \"http://127.0.0.1:57321/v1\"\n",
        )
        .unwrap();
        assert_eq!(codex(home, 57321), None);
        // 自定义供应商：直接指向网关。
        std::fs::write(
            home.join("config.toml"),
            "model_provider = \"CodexPlusPlus\"\n[model_providers.CodexPlusPlus]\nbase_url = \"http://127.0.0.1:57321/gw/codex/v1\"\n",
        )
        .unwrap();
        assert_eq!(codex(home, 57321), None);
    }

    #[test]
    fn codex_taken_over_by_another_tool_is_reported() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        std::fs::write(
            home.join("config.toml"),
            "model_provider = \"custom\"\nmodel_catalog_json = \"cc-switch-model-catalog.json\"\n[model_providers.custom]\nbase_url = \"http://127.0.0.1:15721/v1\"\n",
        )
        .unwrap();
        let found = codex(home, 57321).unwrap();
        assert_eq!(found.by.as_deref(), Some("CC Switch"));
        assert_eq!(found.endpoint, "http://127.0.0.1:15721/v1");

        std::fs::write(
            home.join("config.toml"),
            "model_provider = \"other\"\n[model_providers.other]\nbase_url = \"https://relay.example/v1\"\n",
        )
        .unwrap();
        let found = codex(home, 57321).unwrap();
        assert_eq!(found.by, None);
        assert_eq!(found.endpoint, "https://relay.example/v1");
    }

    #[test]
    fn claude_and_grok_are_checked_by_their_base_url() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("settings.json"),
            r#"{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:57321/gw/claude"}}"#,
        )
        .unwrap();
        assert_eq!(claude(temp.path()), None);
        std::fs::write(
            temp.path().join("settings.json"),
            r#"{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:15721"}}"#,
        )
        .unwrap();
        assert_eq!(
            claude(temp.path()).unwrap().by.as_deref(),
            Some("CC Switch")
        );

        std::fs::write(
            temp.path().join("config.toml"),
            "[model.a]\nbase_url = \"http://127.0.0.1:57321/gw/grok/v1\"\n",
        )
        .unwrap();
        assert_eq!(grok(temp.path()), None);
        std::fs::write(
            temp.path().join("config.toml"),
            "[model.a]\nbase_url = \"https://api.x.ai/v1\"\n",
        )
        .unwrap();
        assert_eq!(grok(temp.path()).unwrap().endpoint, "https://api.x.ai/v1");
    }
}
