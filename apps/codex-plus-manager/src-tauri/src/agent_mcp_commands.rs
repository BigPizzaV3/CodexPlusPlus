//! 「全局管理 · MCP」里 Claude Code 与 Grok 两列的读写命令。Codex 一列仍走公共配置（context entries）。

use serde::Serialize;

use codex_plus_core::agent_mcp::{self, AgentMcpConfig, McpAgent};

use crate::commands::{CommandResult, failed, ok};

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentMcpPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<AgentMcpConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_path: Option<String>,
}

fn agent_name(agent: McpAgent) -> &'static str {
    match agent {
        McpAgent::Claude => "Claude Code",
        McpAgent::Grok => "Grok",
    }
}

/// 读一个 Agent 的用户级 MCP 服务器，统一成 Codex 规范的 TOML 表体。
#[tauri::command]
pub fn load_agent_mcp(agent: String) -> CommandResult<AgentMcpPayload> {
    let result = McpAgent::parse(&agent)
        .and_then(|agent| agent_mcp::load(agent).map(|config| (agent, config)));
    match result {
        Ok((agent, config)) => ok(
            &format!("已读取 {} 的 MCP 配置。", agent_name(agent)),
            AgentMcpPayload {
                config: Some(config),
                backup_path: None,
            },
        ),
        Err(error) => failed(
            &format!("读取 MCP 配置失败：{error:#}"),
            AgentMcpPayload::default(),
        ),
    }
}

/// 写入（`toml_body` 有值）或移除（为空）一条 MCP 服务器；写前自动备份。
#[tauri::command]
pub fn save_agent_mcp(
    agent: String,
    id: String,
    toml_body: Option<String>,
) -> CommandResult<AgentMcpPayload> {
    let result = (|| {
        let agent = McpAgent::parse(&agent)?;
        let backup = agent_mcp::save(agent, &id, toml_body.as_deref())?;
        let config = agent_mcp::load(agent)?;
        Ok::<_, anyhow::Error>((agent, backup, config))
    })();
    match result {
        Ok((agent, backup, config)) => {
            let name = agent_name(agent);
            let message = if toml_body.is_some() {
                format!("已把 {id} 写入 {name}。")
            } else {
                format!("已从 {name} 移除 {id}。")
            };
            ok(
                &message,
                AgentMcpPayload {
                    config: Some(config),
                    backup_path: backup.map(|path| path.to_string_lossy().into_owned()),
                },
            )
        }
        Err(error) => failed(
            &format!("保存 MCP 配置失败：{error:#}"),
            AgentMcpPayload::default(),
        ),
    }
}
