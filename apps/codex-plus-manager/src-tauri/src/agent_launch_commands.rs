//! 在新的终端窗口里直接启动命令行 Agent（Claude Code、Grok）。Agent 概览页的「在终端启动」用。

use codex_plus_core::gateway::sessions;

use crate::commands::{CommandResult, failed, ok};

/// 只放行已知的命令行 Agent，命令名写死，不拼接任何外部输入。
fn agent_command(agent: &str) -> Option<&'static str> {
    match agent {
        "claude" => Some("claude"),
        "grok" => Some("grok"),
        _ => None,
    }
}

/// 在主目录打开一个新终端并运行该 Agent。
#[tauri::command]
pub fn open_agent_terminal(agent: String) -> CommandResult<()> {
    let Some(command) = agent_command(&agent) else {
        return failed(&format!("不支持在终端启动：{agent}"), ());
    };
    let scratch = codex_plus_core::paths::default_app_state_dir().join("tmp");
    match sessions::open_in_terminal("", command, &scratch) {
        Ok(()) => ok("已在终端里启动。", ()),
        Err(error) => failed(&format!("打开终端失败：{error:#}"), ()),
    }
}

#[cfg(test)]
mod tests {
    use super::agent_command;

    #[test]
    fn only_known_cli_agents_can_be_launched() {
        assert_eq!(agent_command("claude"), Some("claude"));
        assert_eq!(agent_command("grok"), Some("grok"));
        assert_eq!(agent_command("codex"), None);
        assert_eq!(agent_command("claude; rm"), None);
    }
}
