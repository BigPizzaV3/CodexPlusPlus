pub mod commands;
pub mod install;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppMode {
    Main,
    Manager,
}

pub fn run() {
    install_panic_logger();
    let app_mode = detect_app_mode();
    codex_plus_core::launcher::start_official_codex_config_guard_for_startup();
    let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
        "manager.start",
        serde_json::json!({
            "version": env!("CARGO_PKG_VERSION"),
            "app_mode": app_mode.as_str()
        }),
    );
    let Some(_guard) = acquire_single_instance_guard(app_mode) else {
        return;
    };
    let show_update = commands::startup_should_show_update();
    let run_result = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            let url = app_mode.startup_url(show_update);
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Regular);
            let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                "manager.setup_start",
                serde_json::json!({
                    "app_mode": app_mode.as_str(),
                    "url": url
                }),
            );
            let window =
                tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::App(url.into()))
                    .title(app_mode.window_title())
                    .inner_size(app_mode.initial_width(), app_mode.initial_height())
                    .min_inner_size(app_mode.min_width(), app_mode.min_height())
                    .build()?;
            let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                "manager.window_built",
                serde_json::json!({
                    "app_mode": app_mode.as_str(),
                    "label": "main"
                }),
            );
            window.show()?;
            let _ = window.set_focus();
            let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                "manager.window_visible",
                serde_json::json!({
                    "app_mode": app_mode.as_str(),
                    "label": "main"
                }),
            );
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::backend_version,
            commands::startup_options,
            commands::load_local_auth_state,
            commands::load_local_usage_state,
            commands::request_local_sms_code,
            commands::login_with_local_sms_code,
            commands::load_sms_provider_settings,
            commands::save_sms_provider_settings,
            commands::update_local_entitlement,
            commands::export_local_identity_report,
            commands::prepare_identity_sync_request,
            commands::sync_identity_to_service,
            commands::load_local_backend_state,
            commands::apply_identity_sync_locally,
            commands::load_admin_console,
            commands::admin_console_set_user_access,
            commands::admin_console_update_user_entitlement,
            commands::admin_console_update_team_entitlement,
            commands::admin_console_record_billing_renewal,
            commands::admin_console_reconcile_billing,
            commands::logout_local_auth,
            commands::reset_local_auth_state,
            commands::launch_embedded_codex,
            commands::load_overview,
            commands::launch_codex_plus,
            commands::restart_codex_plus,
            commands::load_settings,
            commands::save_settings,
            commands::list_local_sessions,
            commands::list_zed_remote_projects,
            commands::open_zed_remote,
            commands::forget_zed_remote_project,
            commands::delete_local_session,
            commands::load_ccs_providers,
            commands::import_ccs_providers,
            commands::load_provider_sync_targets,
            commands::sync_providers_now,
            commands::load_ads,
            commands::refresh_script_market,
            commands::install_market_script,
            commands::set_user_script_enabled,
            commands::delete_user_script,
            commands::open_external_url,
            commands::install_entrypoints,
            commands::uninstall_entrypoints,
            commands::repair_shortcuts,
            commands::repair_backend,
            commands::repair_official_codex_isolation,
            commands::managed_proxy_status,
            commands::start_managed_proxy,
            commands::stop_managed_proxy,
            commands::check_update,
            commands::perform_update,
            commands::load_watcher_state,
            commands::install_watcher,
            commands::uninstall_watcher,
            commands::enable_watcher,
            commands::disable_watcher,
            commands::read_latest_logs,
            commands::copy_diagnostics,
            commands::release_readiness,
            commands::reset_settings,
            commands::relay_status,
            commands::read_relay_files,
            commands::save_relay_file,
            commands::write_diagnostic_event,
            commands::backfill_relay_profile_from_live,
            commands::list_context_entries,
            commands::read_live_context_entries,
            commands::sync_live_context_entries,
            commands::upsert_context_entry,
            commands::delete_context_entry,
            commands::extract_relay_common_config,
            commands::test_relay_profile,
            commands::fetch_relay_profile_models,
            commands::apply_relay_injection,
            commands::apply_pure_api_injection,
            commands::clear_relay_injection
        ])
        .run(tauri::generate_context!());
    if let Err(error) = run_result {
        let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
            "manager.run_failed",
            serde_json::json!({
                "error": error.to_string()
            }),
        );
    }
}

impl AppMode {
    fn as_str(self) -> &'static str {
        match self {
            AppMode::Main => "main",
            AppMode::Manager => "manager",
        }
    }

    fn startup_url(self, show_update: bool) -> String {
        let mode = self.as_str();
        if show_update && self == AppMode::Manager {
            format!("index.html?mode={mode}&showUpdate=1")
        } else {
            format!("index.html?mode={mode}")
        }
    }

    fn window_title(self) -> &'static str {
        match self {
            AppMode::Main => "极义codex",
            AppMode::Manager => "极义codex 管理工具",
        }
    }

    fn guard_port(self) -> u16 {
        match self {
            AppMode::Main => codex_plus_core::ports::MANAGER_GUARD_PORT.saturating_sub(1),
            AppMode::Manager => codex_plus_core::ports::MANAGER_GUARD_PORT,
        }
    }

    fn initial_width(self) -> f64 {
        match self {
            AppMode::Main => 920.0,
            AppMode::Manager => 1180.0,
        }
    }

    fn initial_height(self) -> f64 {
        match self {
            AppMode::Main => 720.0,
            AppMode::Manager => 820.0,
        }
    }

    fn min_width(self) -> f64 {
        match self {
            AppMode::Main => 760.0,
            AppMode::Manager => 960.0,
        }
    }

    fn min_height(self) -> f64 {
        match self {
            AppMode::Main => 620.0,
            AppMode::Manager => 720.0,
        }
    }
}

fn detect_app_mode() -> AppMode {
    if let Ok(value) = std::env::var("JIYI_CODEX_APP_MODE") {
        match value.trim().to_ascii_lowercase().as_str() {
            "main" | "app" => return AppMode::Main,
            "manager" | "admin" => return AppMode::Manager,
            _ => {}
        }
    }
    for arg in std::env::args() {
        match arg.as_str() {
            "--main" | "--app-mode=main" | "--app-mode=app" => return AppMode::Main,
            "--manager" | "--app-mode=manager" | "--app-mode=admin" => return AppMode::Manager,
            _ => {}
        }
    }
    let executable_path = std::env::current_exe().ok();
    let executable_text = executable_path
        .as_ref()
        .map(|path| path.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if executable_text.contains("极义codex 管理工具.app") {
        return AppMode::Manager;
    }
    if executable_text.contains("极义codex.app") {
        return AppMode::Main;
    }
    let executable = executable_path
        .as_ref()
        .and_then(|path| {
            path.file_stem()
                .map(|stem| stem.to_string_lossy().to_string())
        })
        .unwrap_or_default()
        .to_ascii_lowercase();
    if matches!(executable.as_str(), "jiyicodex" | "jiyicodex.bin") {
        AppMode::Main
    } else {
        AppMode::Manager
    }
}

fn install_panic_logger() {
    std::panic::set_hook(Box::new(|panic_info| {
        let payload = panic_info
            .payload()
            .downcast_ref::<&str>()
            .map(|message| (*message).to_string())
            .or_else(|| panic_info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "非字符串 panic payload".to_string());
        let location = panic_info.location().map(|location| {
            serde_json::json!({
                "file": location.file(),
                "line": location.line(),
                "column": location.column()
            })
        });
        let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
            "manager.panic",
            serde_json::json!({
                "payload": payload,
                "location": location
            }),
        );
    }));
}

fn acquire_single_instance_guard(
    app_mode: AppMode,
) -> Option<codex_plus_core::ports::LoopbackPortGuard> {
    let guard_port = app_mode.guard_port();
    match codex_plus_core::ports::acquire_resilient_loopback_port_guard(guard_port) {
        Ok(guard) => {
            if let Some(fallback_lock_path) = guard.fallback_path() {
                let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                    "manager.guard_fallback",
                    serde_json::json!({
                        "requested_guard_port": guard_port,
                        "app_mode": app_mode.as_str(),
                        "fallback_lock_path": fallback_lock_path
                    }),
                );
            }
            Some(guard)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
            let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                "manager.already_running",
                serde_json::json!({
                    "guard_port": guard_port,
                    "app_mode": app_mode.as_str()
                }),
            );
            None
        }
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
            let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                "manager.already_running",
                serde_json::json!({
                    "guard_port": guard_port,
                    "app_mode": app_mode.as_str()
                }),
            );
            None
        }
        Err(error) => {
            let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                "manager.guard_failed",
                serde_json::json!({
                    "guard_port": guard_port,
                    "app_mode": app_mode.as_str(),
                    "error": error.to_string()
                }),
            );
            match std::net::TcpListener::bind(("127.0.0.1", 0)) {
                Ok(listener) => Some(codex_plus_core::ports::LoopbackPortGuard::listener(
                    listener,
                )),
                Err(fallback_error) => {
                    let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                        "manager.guard_fallback_failed",
                        serde_json::json!({
                            "error": fallback_error.to_string()
                        }),
                    );
                    None
                }
            }
        }
    }
}
