pub mod commands;
pub mod install;

pub fn run() {
    let show_update = commands::startup_should_show_update();
    tauri::Builder::default()
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
