#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    let args: Vec<_> = std::env::args_os().collect();
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
    // 安装器覆盖安装前请求正在运行的实例正常退出：本进程只写请求文件后立即返回，
    // 不启动界面，也不强制结束进程（清理由运行中的实例自己完成）。
    if requests_installer_quit(std::env::args()) {
        codex_plus_manager_lib::request_running_manager_quit();
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
        } else if arg.starts_with("codexplusplus://") {
            match codex_plus_core::provider_import::save_pending_provider_import_from_url(&arg) {
                Ok(request) => {
                    let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                        "manager.provider_import_url.pending",
                        serde_json::json!({
                            "name": request.name,
                            "baseUrl": request.base_url
                        }),
                    );
                    codex_plus_manager_lib::focus_existing_manager_window();
                }
                Err(error) => {
                    let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                        "manager.provider_import_url.failed",
                        serde_json::json!({
                            "error": error.to_string()
                        }),
                    );
                }
            }
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

/// 安装器用 `--quit-for-install` 拉起第二个进程，由它把退出请求交给正在运行的实例。
fn requests_installer_quit(args: impl IntoIterator<Item = String>) -> bool {
    args.into_iter()
        .any(|arg| arg == codex_plus_manager_lib::QUIT_FOR_INSTALL_FLAG)
}

#[cfg(test)]
mod tests {
    use super::should_skip_autostart;

    #[test]
    fn installer_quit_flag_only_matches_its_own_argument() {
        assert!(super::requests_installer_quit(
            ["codex-plus-plus", "--quit-for-install"].map(String::from)
        ));
        for args in [
            vec!["codex-plus-plus"],
            vec!["codex-plus-plus", "--autostart"],
            vec!["codex-plus-plus", "--show-update"],
            vec!["codex-plus-plus", "--quit"],
        ] {
            assert!(!super::requests_installer_quit(
                args.into_iter().map(String::from)
            ));
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
