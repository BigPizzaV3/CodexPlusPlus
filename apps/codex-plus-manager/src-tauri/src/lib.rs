pub mod commands;
pub mod install;

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            commands::backend_version,
            commands::load_overview,
            commands::launch_codex_plus,
            commands::restart_codex_plus,
            commands::load_settings,
            commands::save_settings,
            commands::sync_providers_now,
            commands::load_ads,
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
            commands::apply_relay_injection,
            commands::clear_relay_injection
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Codex++ manager");
}
