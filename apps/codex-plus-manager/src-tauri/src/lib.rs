pub mod commands;
pub mod install;

pub fn run() {
    let Some(_guard) = acquire_single_instance_guard() else {
        return;
    };
    let show_update = commands::startup_should_show_update();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            let url = if show_update {
                "index.html?showUpdate=1"
            } else {
                "index.html"
            };
            tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::App(url.into()))
                .title("Codex++ 管理工具")
                .inner_size(960.0, 720.0)
                .build()?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::backend_version,
            commands::startup_options,
            commands::load_overview,
            commands::launch_codex_plus,
            commands::restart_codex_plus,
            commands::load_settings,
            commands::save_settings,
            commands::load_ccs_providers,
            commands::import_ccs_providers,
            commands::sync_providers_now,
            commands::load_ads,
            commands::refresh_script_market,
            commands::install_market_script,
            commands::open_external_url,
            commands::install_entrypoints,
            commands::uninstall_entrypoints,
            commands::repair_shortcuts,
            commands::repair_backend,
            commands::check_update,
            commands::perform_update,
            commands::load_watcher_state,
            commands::install_watcher,
            commands::uninstall_watcher,
            commands::enable_watcher,
            commands::disable_watcher,
            commands::read_latest_logs,
            commands::copy_diagnostics,
            commands::reset_settings,
            commands::relay_status,
            commands::read_relay_files,
            commands::save_relay_file,
            commands::test_relay_profile,
            commands::apply_relay_injection,
            commands::apply_pure_api_injection,
            commands::clear_relay_injection
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Codex++ manager");
}

fn acquire_single_instance_guard() -> Option<std::net::TcpListener> {
    match codex_plus_core::ports::acquire_loopback_port_guard(
        codex_plus_core::ports::MANAGER_GUARD_PORT,
    ) {
        Ok(listener) => Some(listener),
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
            let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                "manager.already_running",
                serde_json::json!({
                    "guard_port": codex_plus_core::ports::MANAGER_GUARD_PORT
                }),
            );
            None
        }
        Err(error) => {
            let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                "manager.guard_failed",
                serde_json::json!({
                    "guard_port": codex_plus_core::ports::MANAGER_GUARD_PORT,
                    "error": error.to_string()
                }),
            );
            Some(
                std::net::TcpListener::bind(("127.0.0.1", 0))
                    .expect("fallback manager guard should bind"),
            )
        }
    }
}
