//! 「对话造插件」的管理器入口：把插件创作 MCP 接入 Codex，以及 MCP 进程本身。
//!
//! 接入流程与协作 MCP 共用 [`crate::agent_collaboration_commands::connect_codex`]：
//! 先写 Codex++ 托管上下文（切换供应商时不会丢），再原子写 Codex live 配置，失败只回滚本条。

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use codex_plus_core::managed_mcp;
use codex_plus_core::plugin_creator::{self, PluginCreator};
use codex_plus_core::settings::SettingsStore;
use serde_json::{Value, json};

#[tauri::command]
pub async fn plugin_creator_request(action: String) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<Value> {
        let executable = std::env::current_exe().context("无法定位插件创作 MCP 程序")?;
        let home = codex_plus_core::codex_home::default_codex_home_dir();
        let state_dir = codex_plus_core::paths::default_app_state_dir();
        match action.as_str() {
            "status" => {}
            "connect" => crate::agent_collaboration_commands::connect_codex(
                &plugin_creator::SPEC,
                &SettingsStore::default(),
                &home,
                &executable,
                &state_dir,
            )?,
            _ => bail!("不支持的插件创作操作"),
        }
        Ok(registration_view(&home, &executable, &state_dir))
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("{e:#}"))
}

fn registration_view(home: &Path, executable: &Path, state_dir: &Path) -> Value {
    let plugins_directory = crate::commands::default_user_script_manager()
        .user_dir()
        .to_string_lossy()
        .into_owned();
    match managed_mcp::status(&plugin_creator::SPEC, home, executable, state_dir) {
        Ok(registration) => json!({
            "status": if registration.registered { "connected" } else if registration.status == "conflict" { "conflict" } else { "not_connected" },
            "message": registration.message,
            "configPath": registration.config_path,
            "pluginsDirectory": plugins_directory,
        }),
        Err(error) => json!({
            "status": "not_connected",
            "message": error.to_string(),
            "pluginsDirectory": plugins_directory,
        }),
    }
}

fn state_dir_from_args(args: &[OsString]) -> anyhow::Result<PathBuf> {
    match args {
        [] => Ok(codex_plus_core::paths::default_app_state_dir()),
        [flag, directory] if flag == "--state-dir" => {
            let directory = PathBuf::from(directory);
            if !directory.is_absolute() {
                bail!("--state-dir 必须是绝对目录");
            }
            Ok(directory)
        }
        _ => bail!("插件创作 MCP 仅接受可选的 --state-dir <绝对目录>"),
    }
}

/// Codex 页面的调试端口：MCP 进程由 Codex 拉起，不继承管理器的环境变量，所以按接入时固化的
/// 状态目录读实例记录，再补常规安装的记录与默认端口。实际操作前仍会核对页面身份。
fn debug_ports(state_dir: &Path) -> Vec<u16> {
    let mut ports = Vec::new();
    let mut push = |port: Option<u16>| {
        if let Some(port) = port.filter(|port| *port > 0) {
            if !ports.contains(&port) {
                ports.push(port);
            }
        }
    };
    let latest = |path: PathBuf| {
        codex_plus_core::status::StatusStore::new(path)
            .load_latest()
            .ok()
            .flatten()
            .and_then(|status| status.debug_port)
    };
    push(latest(state_dir.join("latest-status.json")));
    if let Some(base) = directories::BaseDirs::new() {
        push(latest(base.home_dir().join(".codex-session-delete/latest-status.json")));
    }
    push(Some(9229));
    ports
}

pub fn run_mcp(args: &[OsString]) -> anyhow::Result<()> {
    let state_dir = state_dir_from_args(args)?;
    let creator = PluginCreator {
        manager: crate::commands::default_user_script_manager(),
        debug_ports: debug_ports(&state_dir),
    };
    tauri::async_runtime::block_on(plugin_creator::serve(creator))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_mode_rejects_unknown_or_relative_arguments() {
        for args in [vec!["--state-dir"], vec!["--state-dir", "relative"], vec!["--write", "x"]] {
            let args = args.into_iter().map(OsString::from).collect::<Vec<_>>();
            assert!(state_dir_from_args(&args).is_err());
        }
        let dir = tempfile::tempdir().unwrap();
        let args = [OsString::from("--state-dir"), dir.path().as_os_str().to_owned()];
        assert_eq!(state_dir_from_args(&args).unwrap(), dir.path());
    }

    #[test]
    fn plugin_creator_coexists_with_collaboration_in_managed_context() {
        let dir = tempfile::tempdir().unwrap();
        let executable = std::env::current_exe().unwrap();
        let state_dir = dir.path().join("state");
        let with_collaboration = codex_plus_core::agent_collaboration::integration::upsert_managed_context(
            "[mcp_servers.other]\ncommand = 'x'\n",
            &executable,
            &state_dir,
        )
        .unwrap();
        let both = managed_mcp::upsert_managed_context(
            &plugin_creator::SPEC,
            &with_collaboration,
            &executable,
            &state_dir,
        )
        .unwrap();
        let document: toml_edit::DocumentMut = both.parse().unwrap();
        let servers = &document["mcp_servers"];
        assert_eq!(servers["other"]["command"].as_str(), Some("x"));
        assert_eq!(
            servers["codex_plus_collaboration"]["args"][0].as_str(),
            Some("--codex-plus-agent-mcp")
        );
        assert_eq!(
            servers[plugin_creator::SERVER_ID]["args"][0].as_str(),
            Some(plugin_creator::MCP_FLAG)
        );
        assert_eq!(servers[plugin_creator::SERVER_ID]["enabled"].as_bool(), Some(true));
        // 同名条目被别的程序占用时不接管。
        let foreign = format!("[mcp_servers.{}]\ncommand = 'other'\nargs = ['--x']\n", plugin_creator::SERVER_ID);
        assert!(
            managed_mcp::upsert_managed_context(&plugin_creator::SPEC, &foreign, &executable, &state_dir)
                .is_err()
        );
    }
}
