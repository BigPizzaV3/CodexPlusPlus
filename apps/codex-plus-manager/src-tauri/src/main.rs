#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    let args: Vec<_> = std::env::args_os().collect();
    // MCP 是独立 stdio 进程，必须在 GUI、单实例锁和启动日志之前处理。
    if args.get(1).is_some_and(|arg| arg == "--codex-plus-cu-mcp") {
        if let Err(error) = codex_plus_manager_lib::run_computer_use_mcp() {
            eprintln!("Computer Use MCP 退出：{error:#}");
            std::process::exit(1);
        }
        return;
    }
    if args.get(1).is_some_and(|arg| arg == "--codex-plus-browser-mcp") {
        if let Err(error) = codex_plus_manager_lib::run_browser_use_mcp() {
            eprintln!("浏览器控制 MCP 退出：{error:#}");
            std::process::exit(1);
        }
        return;
    }
    if args.get(1).is_some_and(|arg| arg == codex_plus_core::extension_claude::MCP_FLAG) {
        if let Err(error) = run_claude_extension_mcp(&args[2..]) {
            eprintln!("Claude 拓展 MCP 退出：{error:#}");
            std::process::exit(1);
        }
        return;
    }
    // Claude Code 的 Stop / SubagentStop 钩子调用：只往 spool 里记一行，不启动 GUI。
    // 第二个参数是安装时固化的 state 目录（钩子不继承 CODEX_PLUS_STATE_DIR）。
    if args.get(1).is_some_and(|arg| arg == "--codex-plus-browser-turn-end") {
        codex_plus_core::claude_hooks::record_turn_end(args.get(2).map(std::ffi::OsString::as_os_str));
        return;
    }
    if args.get(1).is_some_and(|arg| arg == "--codex-plus-cu-cli") {
        if let Err(error) = codex_plus_manager_lib::run_computer_use_cli(&args[2..]) {
            eprintln!("Computer Use 诊断失败：{error:#}");
            std::process::exit(1);
        }
        return;
    }
    // 卸载器在删除主程序前调用：config.toml 仍指向本机代理时恢复官方配置（issue #406）。
    // 结果只写诊断日志，失败也以 0 退出，不能让卸载器卡住。
    if args.get(1).is_some_and(|arg| arg == "--codex-plus-uninstall-cleanup") {
        // 先把走网关的 Agent（Codex / Claude Code / Grok）恢复成直连：卸载后网关不在了。
        // 失败只记日志，万一 panic 也接住，不影响下面的清理。
        let _ = std::panic::catch_unwind(codex_plus_manager_lib::restore_gateway_for_uninstall);
        let home = codex_plus_core::codex_home::default_codex_home_dir();
        let detail = match codex_plus_core::relay_config::clear_managed_proxy_config_for_uninstall(
            &home,
        ) {
            Ok(changed) => serde_json::json!({
                "home": home.to_string_lossy(),
                "changed": changed,
            }),
            Err(error) => serde_json::json!({
                "home": home.to_string_lossy(),
                "error": format!("{error:#}"),
            }),
        };
        let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
            "uninstall.proxy_config_cleanup",
            detail,
        );
        return;
    }
    if args.get(1).is_some_and(|arg| arg == "--codex-plus-agent-mcp") {
        if let Err(error) = codex_plus_manager_lib::run_agent_collaboration_mcp(&args[2..]) {
            eprintln!("Agent 协作 MCP 启动失败：{error:#}");
            std::process::exit(1);
        }
        return;
    }
    if args
        .get(1)
        .is_some_and(|arg| arg == codex_plus_core::dsh_plugins::MCP_FLAG)
    {
        match codex_plus_core::dsh_plugins::run_mcp(&args[2..]) {
            Ok(code) => std::process::exit(code),
            Err(error) => {
                eprintln!("dsh 插件 MCP 启动失败：{error:#}");
                std::process::exit(1);
            }
        }
    }
    // Codex 钩子分发：拓展在 codexpp.json 里声明的 hooks 都经这一条命令运行，stdout 写合并后的决定。
    if args.get(1).is_some_and(|arg| arg == codex_plus_core::extension_hooks::HOOK_FLAG) {
        let event = args.get(2).and_then(|arg| arg.to_str()).unwrap_or_default().to_string();
        std::process::exit(codex_plus_manager_lib::run_extension_hook(&event));
    }
    if args.get(1).is_some_and(|arg| arg == codex_plus_core::extension_backend::HOST_CALL_FLAG) {
        std::process::exit(codex_plus_manager_lib::run_extension_host_call());
    }
    // 旧参数 --codex-plus-plugin-mcp 是改名前接入写下的，重新接入前仍要能启动。
    if args.get(1).is_some_and(|arg| {
        arg == codex_plus_core::extension_creator::MCP_FLAG
            || arg == codex_plus_core::extension_creator::LEGACY_MCP_FLAG
    }) {
        if let Err(error) = codex_plus_manager_lib::run_extension_creator_mcp(&args[2..]) {
            eprintln!("拓展创作 MCP 启动失败：{error:#}");
            std::process::exit(1);
        }
        return;
    }
    if let Some(options_path) = args
        .windows(2)
        .find(|window| window[0] == "--codex-plus-multi-worker")
        .and_then(|window| window.get(1))
    {
        if let Err(error) = codex_plus_manager_lib::commands::run_codex_multi_worker(
            std::path::Path::new(options_path),
        ) {
            eprintln!("Codex 多开 worker 失败：{error:#}");
            std::process::exit(1);
        }
        return;
    }
    #[cfg(target_os = "macos")]
    if launched_from_legacy_manager_compat_bundle() {
        // 旧版更新器要求 DMG 中有第二个 manager bundle，并会在替换后打开它。
        // 兼容壳只满足身份校验；让真正的统一 Codex++ app 接管界面。
        return;
    }
    // 开机启动也只打开界面；禁用开机项不影响用户主动双击。
    if should_skip_autostart(
        std::env::args(),
        codex_plus_core::watcher::default_watcher_disabled_flag().exists(),
    ) {
        return;
    }
    #[cfg(target_os = "macos")]
    {
        let args: Vec<_> = std::env::args_os().collect();
        if args
            .get(1)
            .is_some_and(|arg| arg == "--apply-codex-plus-update")
        {
            let result = if args.len() == 3 {
                codex_plus_core::update::macos::run_update_helper(std::path::Path::new(&args[2]))
            } else {
                Err(anyhow::anyhow!(
                    "更新 helper 需要且仅接受一个计划文件路径。"
                ))
            };
            if let Err(error) = result {
                eprintln!("更新 helper 失败：{error:#}");
                let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                    "update.macos.helper_failed",
                    serde_json::json!({ "message": error.to_string() }),
                );
                std::process::exit(1);
            }
            return;
        }
    }
    for arg in std::env::args() {
        if arg.starts_with("dreamskin://") {
            if codex_plus_manager_lib::handle_dream_skin_url(&arg) {
                codex_plus_manager_lib::focus_existing_manager_window();
            }
        } else if codex_plus_manager_lib::handle_provider_import_url(&arg) {
            codex_plus_manager_lib::focus_existing_manager_window();
        }
    }
    if std::env::args().any(|arg| arg == "--show-update") {
        unsafe {
            std::env::set_var("CODEX_PLUS_SHOW_UPDATE", "1");
        }
    }
    codex_plus_manager_lib::run();
}

#[cfg(target_os = "macos")]
fn launched_from_legacy_manager_compat_bundle() -> bool {
    std::env::current_exe()
        .ok()
        .map(|path| path.to_string_lossy().contains("Codex++ 管理工具.app"))
        .unwrap_or(false)
}

fn should_skip_autostart(args: impl IntoIterator<Item = String>, disabled: bool) -> bool {
    disabled && args.into_iter().any(|arg| arg == "--autostart")
}

#[derive(Debug, Default, PartialEq, Eq)]
struct ClaudeExtensionOptions {
    state_dir: Option<std::path::PathBuf>,
    settings_path: Option<std::path::PathBuf>,
}

fn claude_extension_options(args: &[std::ffi::OsString]) -> anyhow::Result<ClaudeExtensionOptions> {
    let mut options = ClaudeExtensionOptions::default();
    let mut args = args.chunks_exact(2);
    for pair in &mut args {
        let target = match pair[0].to_str() {
            Some("--state-dir") => &mut options.state_dir,
            Some("--settings-path") => &mut options.settings_path,
            _ => anyhow::bail!("Claude 拓展 MCP 仅接受 --state-dir 与 --settings-path"),
        };
        anyhow::ensure!(target.is_none(), "Claude 拓展 MCP 参数不能重复");
        let path = std::path::PathBuf::from(&pair[1]);
        anyhow::ensure!(path.is_absolute(), "Claude 拓展 MCP 路径必须是绝对路径");
        *target = Some(path);
    }
    anyhow::ensure!(args.remainder().is_empty(), "Claude 拓展 MCP 参数缺少路径");
    Ok(options)
}

fn run_claude_extension_mcp(args: &[std::ffi::OsString]) -> anyhow::Result<()> {
    let options = claude_extension_options(args)?;
    if let Some(directory) = options.state_dir {
        // 独立 stdio 入口尚未启动线程；插件必须使用所属管理器的状态目录。
        unsafe { std::env::set_var(codex_plus_core::paths::STATE_DIR_ENV, directory) };
    }
    codex_plus_core::extension_claude::run_mcp_cli(options.settings_path)
}

#[cfg(test)]
mod tests {
    use super::{ClaudeExtensionOptions, claude_extension_options, should_skip_autostart};

    #[test]
    fn claude_extension_mcp_rejects_unrecognized_or_relative_arguments() {
        use std::ffi::OsString;
        assert_eq!(claude_extension_options(&[]).unwrap(), ClaudeExtensionOptions::default());
        for args in [
            vec!["--state-dir"],
            vec!["--state-dir", "relative"],
            vec!["--settings-path", "relative"],
            vec!["--other", "/tmp"],
            vec!["--state-dir", "/tmp", "--state-dir", "/tmp"],
            vec!["--settings-path", "/tmp", "--settings-path", "/tmp"],
        ] {
            assert!(claude_extension_options(&args.into_iter().map(OsString::from).collect::<Vec<_>>()).is_err());
        }
        let directory = std::env::temp_dir();
        let settings = directory.join("custom-settings.json");
        for args in [
            vec![OsString::from("--state-dir"), directory.as_os_str().to_owned(), OsString::from("--settings-path"), settings.as_os_str().to_owned()],
            vec![OsString::from("--settings-path"), settings.as_os_str().to_owned(), OsString::from("--state-dir"), directory.as_os_str().to_owned()],
        ] {
            assert_eq!(claude_extension_options(&args).unwrap(), ClaudeExtensionOptions {
                state_dir: Some(directory.clone()),
                settings_path: Some(settings.clone()),
            });
        }
    }

    #[test]
    fn disabling_login_startup_still_allows_opening_the_gui() {
        for args in [
            vec!["codex-plus-plus"],
            vec!["codex-plus-plus", "--background"],
            vec!["codex-plus-plus", "--helper-only"],
        ] {
            assert!(!should_skip_autostart(
                args.into_iter().map(String::from),
                true
            ));
        }
        assert!(should_skip_autostart(
            ["codex-plus-plus", "--autostart"].map(String::from),
            true
        ));
        assert!(!should_skip_autostart(
            ["codex-plus-plus", "--autostart"].map(String::from),
            false
        ));
    }
}
