pub mod commands;
pub mod install;
mod runtime_health;

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager, WindowEvent};

const TRAY_ID: &str = "codex_plus_tray";

static APP_EXITING: AtomicBool = AtomicBool::new(false);
static SHUTDOWN_STARTED: AtomicBool = AtomicBool::new(false);
static SHUTDOWN_COMPLETE: AtomicBool = AtomicBool::new(false);
const TRAY_MENU_SHOW: &str = "tray_show_main";
const TRAY_MENU_QUIT: &str = "tray_quit_app";
const MANAGER_NAVIGATION_EVENT: &str = "manager-navigation-requested";

pub(crate) fn app_is_exiting() -> bool {
    APP_EXITING.load(Ordering::SeqCst)
}

pub fn run() {
    install_panic_logger();
    let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
        "manager.start",
        serde_json::json!({
            "version": env!("CARGO_PKG_VERSION")
        }),
    );
    let Some(_guard) = acquire_single_instance_guard() else {
        return;
    };
    #[cfg(target_os = "macos")]
    if let Some(root) = codex_plus_core::install::default_install_root() {
        match codex_plus_core::install::macos::migrate_legacy_manager_bundle(&root) {
            Ok(Some(archive)) => {
                let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                    "manager.legacy_manager_migrated",
                    serde_json::json!({ "archive": archive }),
                );
            }
            Ok(None) => {}
            Err(error) => {
                let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                    "manager.legacy_manager_migration_failed",
                    serde_json::json!({ "message": error.to_string() }),
                );
            }
        }
    }
    if let Ok(settings) = codex_plus_core::settings::SettingsStore::default().load()
        && let Err(error) = codex_plus_core::dream_skin::sync_default_dream_skin_base_theme(
            settings.enhancements_enabled
                && settings.codex_app_dream_skin_enabled
                && !settings.codex_app_dream_skin_paused,
            &settings.codex_app_dream_skin_theme_config,
        )
    {
        let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
            "manager.dream_skin_base_theme_sync_failed",
            serde_json::json!({ "message": error.to_string() }),
        );
    }
    let show_update = commands::startup_should_show_update();
    let app_result = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            let url = if show_update {
                "/index.html?showUpdate=1"
            } else {
                "/index.html"
            };
            let mut main_window_builder =
                tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::App(url.into()))
                    .title("Codex++")
                    .visible(true)
                    .focused(true)
                    .inner_size(1180.0, 820.0)
                    .min_inner_size(960.0, 720.0);
            if let Some(icon) = app.default_window_icon().cloned() {
                main_window_builder = main_window_builder.icon(icon)?;
            }
            let main_window = main_window_builder.build()?;
            install_tray(app)?;
            let navigation_app = app.handle().clone();
            codex_plus_launcher::set_navigation_handler(move |_payload| {
                let app = navigation_app.clone();
                navigation_app.run_on_main_thread(move || {
                    show_main_window(&app);
                    let _ = app.emit(MANAGER_NAVIGATION_EVENT, ());
                })?;
                Ok(())
            });
            start_window_activation_listener(app.handle().clone());
            commands::start_weixin_connect_from_saved_settings();
            register_main_window_events(main_window);
            commands::resume_existing_codex_background(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::backend_version,
            commands::startup_options,
            commands::consume_pending_manager_navigation,
            commands::load_overview,
            commands::launch_codex_plus,
            commands::restart_codex_plus,
            commands::load_settings,
            commands::native_browser_status,
            commands::save_settings,
            commands::list_tools,
            commands::test_vlm,
            commands::load_grok_config,
            commands::save_grok_config,
            commands::load_grok_providers,
            commands::apply_grok_relay_profile,
            commands::weixin_connect_qr_start,
            commands::weixin_connect_qr_status,
            commands::weixin_connect_status,
            commands::weixin_connect_start,
            commands::weixin_connect_stop,
            commands::find_desktop_codex_cli,
            commands::query_builtin_model_metadata,
            commands::builtin_model_metadata_index,
            commands::dream_skin_status,
            commands::import_dream_skin_image,
            commands::reset_dream_skin_image,
            commands::reset_dream_skin_theme,
            commands::apply_dream_skin,
            commands::restore_dream_skin,
            commands::verify_dream_skin,
            commands::list_dream_skin_themes,
            commands::refresh_dream_skin_market,
            commands::refresh_dream_skin_community,
            commands::load_pending_dream_skin_community,
            commands::confirm_pending_dream_skin_community,
            commands::dismiss_pending_dream_skin_community,
            commands::install_dream_skin_market_theme,
            commands::install_dream_skin_community_theme,
            commands::import_dream_skin_theme_package,
            commands::load_dream_skin_theme,
            commands::create_dream_skin_theme,
            commands::save_dream_skin_theme,
            commands::rename_dream_skin_theme,
            commands::delete_dream_skin_theme,
            commands::activate_dream_skin_theme,
            commands::load_ccs_providers,
            commands::import_ccs_providers,
            commands::load_pending_provider_import,
            commands::confirm_pending_provider_import,
            commands::dismiss_pending_provider_import,
            commands::list_local_sessions,
            commands::delete_local_session,
            commands::load_provider_sync_targets,
            commands::repair_session_index,
            commands::load_session_index_repair_report,
            commands::preview_session_index_cleanup,
            commands::preview_provider_sync,
            commands::apply_session_index_cleanup,
            commands::sync_providers_now,
            commands::load_ads,
            commands::refresh_script_market,
            commands::refresh_plugin_market,
            commands::install_plugin_market_item,
            commands::plugin_market_install_status,
            commands::refresh_user_script_inventory,
            commands::reload_user_scripts,
            commands::install_market_script,
            commands::set_user_scripts_enabled,
            commands::set_user_script_enabled,
            commands::delete_user_script,
            commands::refresh_skill_catalog,
            commands::list_installed_skills,
            commands::install_skill,
            commands::update_skill,
            commands::set_skill_enabled,
            commands::uninstall_skill,
            commands::restore_skill_backup,
            commands::delete_skill_backup,
            commands::upsert_skill_repo,
            commands::delete_skill_repo,
            commands::open_external_url,
            commands::install_entrypoints,
            commands::uninstall_entrypoints,
            commands::repair_shortcuts,
            commands::check_update,
            commands::perform_update,
            commands::load_watcher_state,
            commands::install_watcher,
            commands::uninstall_watcher,
            commands::enable_watcher,
            commands::disable_watcher,
            commands::read_latest_logs,
            commands::clear_logs,
            commands::scan_agent_cache,
            commands::clean_agent_cache,
            commands::copy_diagnostics,
            commands::reset_settings,
            commands::reset_image_overlay_settings,
            commands::relay_status,
            commands::read_relay_files,
            commands::check_env_conflicts,
            commands::check_relay_environment,
            commands::remove_env_conflicts,
            commands::save_relay_file,
            commands::write_diagnostic_event,
            commands::backfill_relay_profile_from_live,
            commands::list_context_entries,
            commands::read_live_context_entries,
            commands::sync_live_context_entries,
            commands::upsert_context_entry,
            commands::delete_context_entry,
            commands::parse_mcp_entry,
            commands::build_mcp_entry,
            commands::preview_mcp_servers_json,
            commands::import_mcp_servers_json,
            commands::extract_relay_common_config,
            commands::test_relay_profile,
            commands::diagnose_relay_profile,
            commands::test_stepwise_settings,
            commands::fetch_relay_profile_models,
            commands::fetch_sub2api_billing,
            commands::switch_relay_profile,
            commands::apply_relay_injection,
            commands::apply_pure_api_injection,
            commands::clear_relay_injection,
            manager_exit_app,
            manager_hide_to_tray,
            update_tray_labels
        ])
        .build(tauri::generate_context!());
    match app_result {
        Ok(app) => app.run(|app_handle, event| {
            if let tauri::RunEvent::ExitRequested { api, .. } = &event {
                if !SHUTDOWN_COMPLETE.load(Ordering::SeqCst) {
                    api.prevent_exit();
                    if !SHUTDOWN_STARTED.swap(true, Ordering::SeqCst) {
                        APP_EXITING.store(true, Ordering::SeqCst);
                        let app = app_handle.clone();
                        tauri::async_runtime::spawn_blocking(move || {
                            if let Err(error) = codex_plus_launcher::shutdown() {
                                let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                                    "app.shutdown_failed",
                                    serde_json::json!({ "message": error.to_string() }),
                                );
                            }
                            SHUTDOWN_COMPLETE.store(true, Ordering::SeqCst);
                            app.exit(0);
                        });
                    }
                }
            }
            #[cfg(target_os = "macos")]
            match event {
                tauri::RunEvent::Opened { urls } => {
                    for url in urls {
                        if handle_dream_skin_url(url.as_str())
                        {
                            show_main_window(app_handle);
                        }
                    }
                }
                tauri::RunEvent::Reopen { .. } => {
                    show_main_window(app_handle);
                }
                _ => {}
            }

            #[cfg(not(target_os = "macos"))]
            let _ = (app_handle, event);
        }),
        Err(error) => {
            let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                "manager.run_failed",
                serde_json::json!({
                    "error": error.to_string()
                }),
            );
        }
    }
}

pub fn handle_dream_skin_url(url: &str) -> bool {
    if !url.starts_with("dreamskin://") {
        return false;
    }
    match codex_plus_core::dream_skin_community::save_pending_community_link(url) {
        Ok(version_id) => {
            let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                "manager.dream_skin_link.pending",
                serde_json::json!({ "versionId": version_id }),
            );
            true
        }
        Err(error) => {
            let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                "manager.dream_skin_link.failed",
                serde_json::json!({ "error": error.to_string() }),
            );
            false
        }
    }
}

fn install_tray<R: tauri::Runtime>(app: &tauri::App<R>) -> tauri::Result<()> {
    let show_item = MenuItem::with_id(app, TRAY_MENU_SHOW, "显示主窗口", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, TRAY_MENU_QUIT, "退出程序", true, None::<&str>)?;
    let tray_menu = Menu::with_items(app, &[&show_item, &quit_item])?;

    let mut tray_builder = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&tray_menu)
        .tooltip("Codex++")
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id.as_ref() {
            TRAY_MENU_SHOW => {
                show_main_window(app);
            }
            TRAY_MENU_QUIT => {
                APP_EXITING.store(true, Ordering::SeqCst);
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| match event {
            TrayIconEvent::DoubleClick {
                button: MouseButton::Left,
                ..
            } => {
                show_main_window(&tray.app_handle());
            }
            _ => {}
        });

    #[cfg(target_os = "macos")]
    {
        // 使用项目原有云朵图标；系统按 18pt 显示，不加标题占据菜单栏空间。
        // 禁用模板模式以保留品牌渐变颜色。
        tray_builder = tray_builder.icon(macos_tray_icon()).icon_as_template(false);
    }
    #[cfg(not(target_os = "macos"))]
    if let Some(icon) = app.default_window_icon().cloned() {
        tray_builder = tray_builder.icon(icon);
    }

    // Tauri 的资源表持有托盘对象，局部句柄释放不会移除菜单栏入口。
    let _ = tray_builder.build(app)?;
    Ok(())
}

#[cfg(any(target_os = "macos", test))]
fn macos_tray_icon() -> tauri::image::Image<'static> {
    tauri::include_image!("icons/icon.png")
}

#[cfg(test)]
mod tray_tests {
    #[test]
    fn macos_tray_preserves_the_brand_icons_colour_and_transparency() {
        let icon = super::macos_tray_icon();
        assert_eq!(icon.width(), icon.height());
        assert!(icon.width() > 0);
        assert!(
            icon.rgba()
                .chunks_exact(4)
                .any(|pixel| pixel[3] > 0 && pixel[2] > pixel[0])
        );
        assert!(icon.rgba().chunks_exact(4).any(|pixel| pixel[3] == 0));
    }
}

fn register_main_window_events<R: tauri::Runtime>(window: tauri::WebviewWindow<R>) {
    let event_window = window.clone();
    let close_event_window = event_window.clone();
    let close_event_app = event_window.app_handle().clone();
    let focus_event_window = event_window.clone();

    event_window.on_window_event(move |event| match event {
        WindowEvent::Focused(true) => {
            // 外部实例通过 Win32 ShowWindow 唤起时，Tao 的 VISIBLE 标记可能仍为 false。
            // 同步框架状态，否则后续 hide() 会被当作重复操作而跳过。
            #[cfg(windows)]
            let _ = focus_event_window.show();
            let _ = focus_event_window.emit(MANAGER_NAVIGATION_EVENT, ());
        }
        WindowEvent::CloseRequested { api, .. } => {
            if APP_EXITING.load(Ordering::SeqCst) {
                return;
            }

            api.prevent_close();
            let _ = close_event_window.hide();
            set_manager_activation_policy(&close_event_app, false);
        }
        _ => {}
    });
}

#[tauri::command]
fn manager_exit_app<R: tauri::Runtime>(app: tauri::AppHandle<R>) {
    APP_EXITING.store(true, Ordering::SeqCst);
    app.exit(0);
}

#[tauri::command]
fn manager_hide_to_tray<R: tauri::Runtime>(window: tauri::WebviewWindow<R>) {
    let app_handle = window.app_handle();
    let _ = window.hide();
    set_manager_activation_policy(&app_handle, false);
}

#[tauri::command]
fn update_tray_labels<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    show_label: String,
    quit_label: String,
    window_title: String,
) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let show_item = MenuItem::with_id(&app, TRAY_MENU_SHOW, &show_label, true, None::<&str>);
        let quit_item = MenuItem::with_id(&app, TRAY_MENU_QUIT, &quit_label, true, None::<&str>);
        if let (Ok(show), Ok(quit)) = (show_item, quit_item) {
            if let Ok(menu) = Menu::with_items(&app, &[&show, &quit]) {
                let _ = tray.set_menu(Some(menu));
            }
        }
    }
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_title(&window_title);
    }
}

fn show_main_window<R: tauri::Runtime>(app_handle: &tauri::AppHandle<R>) {
    if let Some(window) = app_handle.get_webview_window("main") {
        set_manager_activation_policy(app_handle, true);
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[cfg(target_os = "macos")]
fn set_manager_activation_policy<R: tauri::Runtime>(
    app_handle: &tauri::AppHandle<R>,
    main_window_visible: bool,
) {
    // 关闭窗口收起到菜单栏；重新打开窗口时才恢复 Dock 图标。
    let policy = if main_window_visible {
        tauri::ActivationPolicy::Regular
    } else {
        tauri::ActivationPolicy::Accessory
    };
    let _ = app_handle.set_activation_policy(policy);
}

#[cfg(not(target_os = "macos"))]
fn set_manager_activation_policy<R: tauri::Runtime>(
    _app_handle: &tauri::AppHandle<R>,
    _main_window_visible: bool,
) {
}

/// Restores and focuses an existing manager window on desktop platforms.
pub fn focus_existing_manager_window() {
    // 锁持有者在主进程内消费请求，Linux 及隐藏窗口也能重新打开。
    let path = window_activation_request_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let request = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .to_string();
    let _ = std::fs::write(path, request);
    #[cfg(windows)]
    {
        let current_process_id = std::process::id();
        for process in codex_plus_core::windows_enumerate_processes() {
            if process.process_id == current_process_id {
                continue;
            }
            if ["codex-plus-plus.exe", "codex-plus-plus-manager.exe"]
                .iter()
                .any(|name| process.exe_file.eq_ignore_ascii_case(name))
                && codex_plus_core::windows_activate_process_window(process.process_id)
            {
                break;
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Ok(executable) = std::env::current_exe()
            && let Ok(app) =
                codex_plus_core::install::macos::native_application_bundle_from_executable(
                    &executable,
                )
        {
            let _ = std::process::Command::new("/usr/bin/open")
                .arg(app)
                .status();
        }
    }
}

fn window_activation_request_path() -> std::path::PathBuf {
    codex_plus_core::paths::default_app_state_dir().join(format!(
        "window-activation-{}.request",
        codex_plus_core::ports::manager_guard_port()
    ))
}

fn start_window_activation_listener(app: tauri::AppHandle) {
    let path = window_activation_request_path();
    let mut previous = std::fs::read(&path).unwrap_or_default();
    std::thread::spawn(move || {
        while !SHUTDOWN_STARTED.load(Ordering::SeqCst) {
            std::thread::sleep(std::time::Duration::from_millis(300));
            let current = std::fs::read(&path).unwrap_or_default();
            if !current.is_empty() && current != previous {
                previous = current;
                let app = app.clone();
                let handle = app.clone();
                let _ = handle.run_on_main_thread(move || {
                    show_main_window(&app);
                    let _ = app.emit(MANAGER_NAVIGATION_EVENT, ());
                });
            }
        }
    });
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

fn acquire_single_instance_guard() -> Option<codex_plus_core::ports::LoopbackPortGuard> {
    match codex_plus_core::ports::acquire_resilient_loopback_port_guard(
        codex_plus_core::ports::manager_guard_port(),
    ) {
        Ok(guard) => {
            if let Some(fallback_lock_path) = guard.fallback_path() {
                let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                    "manager.guard_fallback",
                    serde_json::json!({
                        "requested_guard_port": codex_plus_core::ports::manager_guard_port(),
                        "fallback_lock_path": fallback_lock_path
                    }),
                );
            }
            Some(guard)
        }
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::AddrInUse | std::io::ErrorKind::WouldBlock
            ) =>
        {
            let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                "manager.already_running",
                serde_json::json!({
                    "guard_port": codex_plus_core::ports::manager_guard_port()
                }),
            );
            focus_existing_manager_window();
            None
        }
        Err(error) => {
            let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                "manager.guard_failed",
                serde_json::json!({
                    "guard_port": codex_plus_core::ports::manager_guard_port(),
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
