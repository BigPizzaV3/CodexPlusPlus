//! 「对话造拓展」的管理器入口：把拓展创作 MCP 接入 Codex，以及 MCP 进程本身。
//!
//! 接入流程与协作 MCP 共用 [`crate::agent_collaboration_commands::connect_codex`]：
//! 先写 Codex++ 托管上下文（切换供应商时不会丢），再原子写 Codex live 配置，失败只回滚本条。
//! 不造拓展时可以取消接入（条目置为 `enabled = false`），Codex 新对话就不再带上这组工具的说明。

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use codex_plus_core::managed_mcp;
use codex_plus_core::extension_creator::{self, ExtensionCreator};
use codex_plus_core::settings::SettingsStore;
use serde_json::{Value, json};

#[tauri::command]
pub async fn extension_creator_request(action: String) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<Value> {
        let executable = std::env::current_exe().context("无法定位拓展创作 MCP 程序")?;
        let home = codex_plus_core::codex_home::default_codex_home_dir();
        let state_dir = codex_plus_core::paths::default_app_state_dir();
        let settings = SettingsStore::default();
        match action.as_str() {
            "status" => {}
            "connect" => crate::agent_collaboration_commands::connect_codex(
                &extension_creator::SPEC,
                &settings,
                &home,
                &executable,
                &state_dir,
            )?,
            "disconnect" => crate::agent_collaboration_commands::disconnect_codex(
                &extension_creator::SPEC,
                &settings,
                &home,
                &executable,
                &state_dir,
            )?,
            _ => bail!("不支持的拓展创作操作"),
        }
        Ok(registration_view(&settings, &home, &executable, &state_dir))
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("{e:#}"))
}

fn registration_view(settings: &SettingsStore, home: &Path, executable: &Path, state_dir: &Path) -> Value {
    let extensions_directory = crate::commands::default_user_script_manager()
        .user_dir()
        .to_string_lossy()
        .into_owned();
    match managed_mcp::status(&extension_creator::SPEC, home, executable, state_dir) {
        Ok(registration) => json!({
            "status": if registration.registered { "connected" } else if registration.status == "conflict" { "conflict" } else if registration.disabled || disabled_in_managed_context(settings) { "disconnected" } else { "not_connected" },
            "message": registration.message,
            "configPath": registration.config_path,
            "extensionsDirectory": extensions_directory,
        }),
        Err(error) => json!({
            "status": "not_connected",
            "message": error.to_string(),
            "extensionsDirectory": extensions_directory,
        }),
    }
}

/// 切换供应商后停用条目会被移出 live 配置，「已取消接入」以托管上下文为准。
fn disabled_in_managed_context(settings: &SettingsStore) -> bool {
    settings
        .load()
        .is_ok_and(|settings| managed_mcp::disabled_in_context(&extension_creator::SPEC, &settings.relay_context_config_contents))
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
        _ => bail!("拓展创作 MCP 仅接受可选的 --state-dir <绝对目录>"),
    }
}

/// Codex 页面的调试端口：MCP 进程由 Codex 拉起，不继承管理器的环境变量，所以按接入时固化的
/// 状态目录读实例记录，再补常规安装的记录与默认端口。实际操作前仍会核对页面身份。
pub(crate) fn debug_ports(state_dir: &Path) -> Vec<u16> {
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
    let ports = debug_ports(&state_dir);
    // 拓展工具调用后台服务时，经这些端口上的 Codex 页面桥接转给托管后台的进程。
    codex_plus_core::extension_backend::set_remote_bridge_ports(ports.clone());
    let creator = ExtensionCreator {
        manager: codex_plus_core::user_scripts::default_user_script_manager_for_settings(state_dir.join("settings.json")),
        debug_ports: ports,
        state_dir,
    };
    tauri::async_runtime::block_on(extension_creator::serve(creator))
}

/// 多开只重绑定确属本工具的条目；未接入、停用和其他程序的同名条目都保留原意。
pub(crate) fn rebind_multi_instance_creator(
    home: &Path,
    state_dir: &Path,
    executable: &Path,
    context: &str,
) -> anyhow::Result<String> {
    fn rebind(contents: &str, executable: &Path, state_dir: &Path) -> anyhow::Result<String> {
        let document = contents.parse::<toml_edit::DocumentMut>()
            .map_err(|_| anyhow::anyhow!("多开拓展创作配置不是有效 TOML"))?;
        let entries = std::iter::once((extension_creator::SERVER_ID, extension_creator::MCP_FLAG))
            .chain(std::iter::once((extension_creator::LEGACY_SERVER_ID, extension_creator::LEGACY_MCP_FLAG)));
        let owned = entries.filter_map(|(id, flag)| {
            let entry = document.get("mcp_servers")?.get(id)?;
            (entry.get("args")?.as_array()?.get(0)?.as_str()? == flag)
                .then(|| entry.get("enabled").and_then(toml_edit::Item::as_bool).unwrap_or(true))
        }).next();
        let Some(enabled) = owned else { return Ok(contents.to_owned()) };
        let updated = managed_mcp::upsert_managed_context(&extension_creator::SPEC, contents, executable, state_dir)?;
        if enabled { Ok(updated) } else { managed_mcp::disable_managed_context(&extension_creator::SPEC, &updated) }
    }
    let context = rebind(context, executable, state_dir)?;
    let config_path = home.join("config.toml");
    let contents = std::fs::read_to_string(&config_path).context("读取多开拓展创作配置失败")?;
    let updated = rebind(&contents, executable, state_dir)?;
    if updated != contents {
        codex_plus_core::settings::atomic_write(&config_path, updated.as_bytes())?;
    }
    Ok(context)
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
    fn multi_creator_rebinds_only_owned_entries_and_preserves_disabled() {
        let dir = tempfile::tempdir().unwrap();
        let executable = std::env::current_exe().unwrap();
        let worker_state = dir.path().join("worker-state");
        let main_state = dir.path().join("main-state");
        let original = managed_mcp::upsert_managed_context(
            &extension_creator::SPEC, "", &executable, &main_state,
        ).unwrap();
        let disabled = managed_mcp::disable_managed_context(&extension_creator::SPEC, &original).unwrap();
        std::fs::write(dir.path().join("config.toml"), &disabled).unwrap();
        let rebound = rebind_multi_instance_creator(dir.path(), &worker_state, &executable, &disabled).unwrap();
        let document = rebound.parse::<toml_edit::DocumentMut>().unwrap();
        assert_eq!(document["mcp_servers"][extension_creator::SERVER_ID]["args"][2].as_str(), worker_state.to_str());
        assert_eq!(document["mcp_servers"][extension_creator::SERVER_ID]["enabled"].as_bool(), Some(false));
        let foreign = "[mcp_servers.codex_plus_extensions]\ncommand='other'\nargs=['--other']\n";
        std::fs::write(dir.path().join("config.toml"), foreign).unwrap();
        assert_eq!(rebind_multi_instance_creator(dir.path(), &worker_state, &executable, foreign).unwrap(), foreign);
        assert_eq!(std::fs::read_to_string(dir.path().join("config.toml")).unwrap(), foreign);
    }

    #[test]
    fn extension_creator_coexists_with_collaboration_in_managed_context() {
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
            &extension_creator::SPEC,
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
            servers[extension_creator::SERVER_ID]["args"][0].as_str(),
            Some(extension_creator::MCP_FLAG)
        );
        assert_eq!(servers[extension_creator::SERVER_ID]["enabled"].as_bool(), Some(true));
        // 改名前接入的旧条目（归本工具所有）在重新接入时被替换；别人的同名旧条目不动。
        let legacy = format!(
            "[mcp_servers.{id}]\ncommand = 'x'\nargs = ['{flag}', '--state-dir', '/s']\n",
            id = extension_creator::LEGACY_SERVER_ID,
            flag = extension_creator::LEGACY_MCP_FLAG,
        );
        let migrated = managed_mcp::upsert_managed_context(&extension_creator::SPEC, &legacy, &executable, &state_dir).unwrap();
        let migrated: toml_edit::DocumentMut = migrated.parse().unwrap();
        assert!(migrated["mcp_servers"].get(extension_creator::LEGACY_SERVER_ID).is_none());
        assert!(migrated["mcp_servers"].get(extension_creator::SERVER_ID).is_some());
        let foreign_legacy = format!("[mcp_servers.{}]\ncommand = 'other'\nargs = ['--x']\n", extension_creator::LEGACY_SERVER_ID);
        let kept = managed_mcp::upsert_managed_context(&extension_creator::SPEC, &foreign_legacy, &executable, &state_dir).unwrap();
        assert!(kept.contains("command = 'other'"));
        // 同名条目被别的程序占用时不接管。
        let foreign = format!("[mcp_servers.{}]\ncommand = 'other'\nargs = ['--x']\n", extension_creator::SERVER_ID);
        assert!(
            managed_mcp::upsert_managed_context(&extension_creator::SPEC, &foreign, &executable, &state_dir)
                .is_err()
        );
    }

    #[test]
    fn disconnect_disables_entry_in_managed_and_live_configuration() {
        use crate::agent_collaboration_commands::{connect_codex, disconnect_codex};
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("codex-home");
        std::fs::create_dir(&home).unwrap();
        let executable = std::env::current_exe().unwrap();
        let state_dir = dir.path().join("state");
        let settings = SettingsStore::new(dir.path().join("settings.json"));
        let enabled = |contents: &str| -> Option<bool> {
            let document: toml_edit::DocumentMut = contents.parse().unwrap();
            document["mcp_servers"][extension_creator::SERVER_ID]["enabled"].as_bool()
        };
        let live = || std::fs::read_to_string(home.join("config.toml")).unwrap();

        connect_codex(&extension_creator::SPEC, &settings, &home, &executable, &state_dir).unwrap();
        assert_eq!(registration_view(&settings, &home, &executable, &state_dir)["status"], "connected");

        disconnect_codex(&extension_creator::SPEC, &settings, &home, &executable, &state_dir).unwrap();
        assert_eq!(registration_view(&settings, &home, &executable, &state_dir)["status"], "disconnected");
        assert_eq!(enabled(&settings.load().unwrap().relay_context_config_contents), Some(false));
        assert_eq!(enabled(&live()), Some(false));
        // 切换供应商的 apply 流程会把停用条目移出 live；仍应显示「已取消接入」。
        std::fs::write(home.join("config.toml"), "model = 'x'\n").unwrap();
        assert_eq!(registration_view(&settings, &home, &executable, &state_dir)["status"], "disconnected");

        connect_codex(&extension_creator::SPEC, &settings, &home, &executable, &state_dir).unwrap();
        assert_eq!(registration_view(&settings, &home, &executable, &state_dir)["status"], "connected");
        assert_eq!(enabled(&settings.load().unwrap().relay_context_config_contents), Some(true));
        assert_eq!(enabled(&live()), Some(true));
    }
}
