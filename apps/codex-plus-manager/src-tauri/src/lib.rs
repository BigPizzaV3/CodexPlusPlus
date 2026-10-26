mod auto_takeover;
mod agent_collaboration_commands;
mod computer_use_commands;
mod cu_pip;
mod global_dictation;
pub mod commands;
pub mod install;
mod runtime_health;

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager, WindowEvent};

const TRAY_ID: &str = "codex_plus_tray";

static APP_EXITING: AtomicBool = AtomicBool::new(false);
static SHUTDOWN_STARTED: AtomicBool = AtomicBool::new(false);
static SHUTDOWN_COMPLETE: AtomicBool = AtomicBool::new(false);
/// 退出清理的等待上限（秒）：超过就直接退出，避免卡在托盘里退不掉（#2611）。
const SHUTDOWN_WATCHDOG: u64 = 10;
const TRAY_MENU_SHOW: &str = "tray_show_main";
const TRAY_MENU_QUIT: &str = "tray_quit_app";
const MANAGER_NAVIGATION_EVENT: &str = "manager-navigation-requested";

pub(crate) fn app_is_exiting() -> bool {
    APP_EXITING.load(Ordering::SeqCst)
}

pub fn run_agent_collaboration_mcp(args: &[std::ffi::OsString]) -> anyhow::Result<()> {
    agent_collaboration_commands::run_mcp(args)
}

/// Computer Use 诊断入口。只给命令行与排障使用，不经过 GUI，也不注册到路由。
/// Computer Use 的 MCP 入口（stdio）。作为代理运行，直到客户端断开。
pub fn run_computer_use_mcp() -> anyhow::Result<()> {
    codex_plus_core::computer_use_proxy::serve()
}

/// 浏览器控制的 MCP 入口（stdio）。与 CU 分开：原生 CU server 没有浏览器工具，
/// 浏览器面走 Codex 的 Chrome 扩展桥，需要 cua-repl 且每次调用带 turn 元数据。
pub fn run_browser_use_mcp() -> anyhow::Result<()> {
    codex_plus_core::browser_use::serve()
}

pub fn run_computer_use_cli(args: &[std::ffi::OsString]) -> anyhow::Result<()> {
    // CLI 进程没有 GUI，抓帧前必须先连上 WindowServer，否则 CoreGraphics 会断言崩溃
    codex_plus_core::window_capture::ensure_graphics_initialized();
    let action = args
        .first()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| "status".to_string());
    let target = args.get(1).map(|value| value.to_string_lossy().into_owned());
    let output = match action.as_str() {
        "status" => serde_json::to_value(codex_plus_core::computer_use::status()?)?,
        "check" => serde_json::to_value(codex_plus_core::computer_use::check())?,
        "install" => {
            let id = target.as_deref().ok_or_else(|| anyhow::anyhow!("install 需要 target：claudeCode / claudeDesktop / codex / dsh / zcode / workBuddy"))?;
            let parsed = codex_plus_core::computer_use::Target::parse(id)?;
            serde_json::to_value(codex_plus_core::computer_use::install(parsed)?)?
        }
        "uninstall" => {
            let id = target.as_deref().ok_or_else(|| anyhow::anyhow!("uninstall 需要 target：claudeCode / claudeDesktop / codex / dsh / zcode / workBuddy"))?;
            let parsed = codex_plus_core::computer_use::Target::parse(id)?;
            serde_json::to_value(codex_plus_core::computer_use::uninstall(parsed)?)?
        }
        // 浏览器回合钩子：install / uninstall，不传就是 read。
        "hooks" => {
            let action = target.as_deref().unwrap_or("read");
            let receipt = match action {
                "install" => serde_json::to_value(codex_plus_core::claude_hooks::install()?)?,
                "uninstall" => serde_json::to_value(codex_plus_core::claude_hooks::uninstall()?)?,
                "read" => serde_json::json!({
                    "installed": codex_plus_core::claude_hooks::is_installed(),
                    "events": codex_plus_core::claude_hooks::installed_events(),
                    "settings": codex_plus_core::claude_hooks::claude_settings_path().display().to_string(),
                }),
                other => anyhow::bail!("hooks 只支持 install / uninstall / read，收到 {other}"),
            };
            receipt
        }
        // 窗口取流诊断：list 列出可预览窗口；frame <window_id> [缩放] 抓一帧存到临时文件
        "preview" => {
            let action = target.as_deref().unwrap_or("list");
            match action {
                "list" => serde_json::json!({
                    "permitted": codex_plus_core::window_capture::has_permission(),
                    "windows": codex_plus_core::window_capture::list_windows()?,
                }),
                "frame" => {
                    let id = args
                        .get(2)
                        .and_then(|v| v.to_string_lossy().parse::<u32>().ok())
                        .ok_or_else(|| anyhow::anyhow!("frame 需要 window_id"))?;
                    let scale = args
                        .get(3)
                        .and_then(|v| v.to_string_lossy().parse::<f64>().ok())
                        .unwrap_or(0.5);
                    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
                    let handle = codex_plus_core::window_capture::start_preview(
                        id,
                        scale,
                        12.0,
                        codex_plus_core::window_capture::CaptureMode::Screenshot,
                        move |jpeg| {
                            let _ = tx.send(jpeg);
                        },
                    )?;
                    let frame = rx.recv_timeout(std::time::Duration::from_secs(20));
                    let thread_error = handle.error();
                    handle.stop();
                    match frame {
                        Ok(bytes) => {
                            let path = std::env::temp_dir().join("codex-plus-preview-frame.jpg");
                            std::fs::write(&path, &bytes)?;
                            serde_json::json!({
                                "windowId": id,
                                "bytes": bytes.len(),
                                "path": path.display().to_string(),
                            })
                        }
                        Err(error) => {
                            let reason = thread_error.unwrap_or_else(|| "（线程没有留下原因）".into());
                            anyhow::bail!("20 秒内没有拿到帧：{error}；线程报告：{reason}")
                        }
                    }
                }
                other => anyhow::bail!("preview 只支持 list / frame，收到 {other}"),
            }
        }
        other => anyhow::bail!("未知的 Computer Use 诊断动作：{other}"),
    };
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
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
    if let Ok(settings) = codex_plus_core::settings::SettingsStore::default().load() {
        // 请求体上限是运行期策略：管理器启动即按设置应用，之后启动的 helper 直接继承，
        // 不必等完整启动流程。环境变量仍由 launcher 侧统一优先。
        codex_plus_core::launcher::set_http_body_limit_bytes(
            settings.helper_max_request_body_bytes(),
        );
        if let Err(error) = codex_plus_core::dream_skin::sync_default_dream_skin_base_theme(
            settings.enhancements_enabled
                && settings.codex_app_dream_skin_enabled
                && !settings.codex_app_dream_skin_paused,
            &settings.codex_app_dream_skin_theme_config,
        ) {
            let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                "manager.dream_skin_base_theme_sync_failed",
                serde_json::json!({ "message": error.to_string() }),
            );
        }
    }
    let show_update = commands::startup_should_show_update();
    let autostart = std::env::args().any(|arg| arg == "--autostart");
    let start_minimized = autostart
        && codex_plus_core::watcher::watcher_start_minimized();
    let start_codex_at_login = autostart && codex_plus_core::watcher::watcher_start_codex();
    let start_codex_on_open = !autostart && codex_plus_core::watcher::watcher_start_codex_on_open();
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
                    .visible(!start_minimized)
                    .focused(!start_minimized)
                    .inner_size(1180.0, 820.0)
                    .min_inner_size(960.0, 720.0);
            if let Some(icon) = app.default_window_icon().cloned() {
                main_window_builder = main_window_builder.icon(icon)?;
            }
            let main_window = main_window_builder.build()?;
            install_tray(app)?;
            if start_minimized {
                set_manager_activation_policy(app.handle(), false);
            }
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
            // Computer Use 操作后台应用时的悬浮预览（同 Codex 的画中画）。
            cu_pip::start(app.handle().clone());
            commands::start_weixin_connect_from_saved_settings();
            register_main_window_events(main_window);
            commands::resume_existing_codex_background(
                app.handle().clone(),
                start_codex_at_login || start_codex_on_open,
            );
            // 自动接管官方 App（默认关闭）：设置里开着才起守护线程。
            auto_takeover::start_supervisor();
            if let Err(error) = global_dictation::apply_config() {
                let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                    "dictation.global_hotkey_start_failed",
                    serde_json::json!({ "message": error.to_string() }),
                );
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            agent_collaboration_commands::agent_collaboration_request,
            commands::backend_version,
            commands::startup_options,
            commands::consume_pending_manager_navigation,
            commands::load_overview,
            commands::launch_codex_plus,
            commands::launch_codex_multi_instance,
            commands::list_codex_multi_instances,
            commands::stop_codex_multi_instance,
            commands::restart_codex_multi_instance,
            commands::restart_codex_plus,
            commands::load_settings,
            commands::load_request_logs,
            commands::clear_request_logs,
            commands::dictation_local_model_status,
            commands::download_dictation_local_model,
            commands::load_global_dictation_hotkey,
            commands::set_global_dictation_hotkey,
            commands::claude_localization_status,
            commands::claude_localization_action,
            commands::claude_localization_set_voice_input,
            commands::claude_localization_set_localization,
            commands::claude_localization_set_computer_use,
            computer_use_commands::computer_use_status,
            computer_use_commands::computer_use_install,
            computer_use_commands::computer_use_uninstall,
            computer_use_commands::computer_use_custom_add,
            computer_use_commands::computer_use_custom_rename,
            computer_use_commands::computer_use_mode_set,
            computer_use_commands::computer_use_pip_set,
            cu_pip::cu_pip_target,
            cu_pip::cu_pip_dismiss,
            cu_pip::cu_pip_activate,
            computer_use_commands::computer_use_custom_remove,
            computer_use_commands::computer_use_check,
            computer_use_commands::computer_use_open_app,
            computer_use_commands::computer_use_browser_extension_status,
            computer_use_commands::computer_use_open_extension_store,
            computer_use_commands::computer_use_approval_get,
            computer_use_commands::computer_use_approval_set,
            computer_use_commands::computer_use_approval_forget,
            computer_use_commands::computer_use_approval_clear,
            computer_use_commands::computer_use_preview_windows,
            computer_use_commands::computer_use_preview_request_permission,
            computer_use_commands::computer_use_preview_start,
            computer_use_commands::computer_use_preview_stop,
            commands::native_browser_status,
            commands::save_settings,
            commands::list_tools,
            commands::test_vlm,
            commands::load_grok_config,
            commands::save_grok_config,
            commands::load_grok_providers,
            commands::apply_grok_relay_profile,
            commands::load_claude_providers,
            commands::save_claude_provider,
            commands::delete_claude_provider,
            commands::switch_claude_provider,
            commands::import_claude_live_provider,
            commands::import_claude_providers_from_cc_switch,
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
            commands::list_project_sessions,
            commands::delete_local_session,
            commands::archive_local_sessions,
            commands::load_provider_sync_targets,
            commands::repair_session_index,
            commands::load_session_index_repair_report,
            commands::preview_session_index_cleanup,
            commands::preview_provider_sync,
            commands::apply_session_index_cleanup,
            commands::sync_providers_now,
            commands::load_provider_sync_recovery,
            commands::recover_provider_sync,
            commands::load_ads,
            commands::refresh_script_market,
            commands::refresh_plugin_market,
            commands::install_plugin_market_item,
            commands::plugin_market_install_status,
            commands::refresh_user_script_inventory,
            commands::reload_user_scripts,
            commands::install_market_script,
            commands::set_user_scripts_enabled,
            commands::set_user_scripts_skip_next_start,
            commands::set_user_script_enabled,
            commands::delete_user_script,
            commands::refresh_skill_catalog,
            commands::list_installed_skills,
            commands::import_ccs_skills,
            commands::install_skill,
            commands::update_skill,
            commands::set_skill_enabled,
            commands::uninstall_skill,
            commands::restore_skill_backup,
            commands::delete_skill_backup,
            commands::upsert_skill_repo,
            commands::inspect_skill_repo,
            commands::delete_skill_repo,
            commands::open_external_url,
            commands::repair_shortcuts,
            commands::check_update,
            commands::perform_update,
            commands::load_watcher_state,
            commands::set_watcher_enabled,
            commands::set_watcher_start_minimized,
            commands::set_watcher_start_codex,
            commands::set_watcher_start_codex_on_open,
            commands::load_relay_proxy,
            commands::set_relay_proxy,
            commands::load_relay_feature,
            commands::set_relay_feature,
            // 自动接管官方 App：前端 invoke 找不到命令时是运行期静默失败，
            // 没有编译期兜底，所以这三个必须注册在这里（契约测试会守）。
            commands::load_auto_takeover_state,
            commands::take_over_codex_now,
            commands::pause_auto_takeover_for_instance,
            commands::read_latest_logs,
            commands::clear_logs,
            commands::scan_agent_cache,
            commands::clean_agent_cache,
            commands::copy_diagnostics,
            commands::export_settings,
            commands::preview_import_settings,
            commands::set_settings_path,
            commands::reset_settings_path,
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
            commands::inspect_mcp_startup,
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
                        auto_takeover::stop_supervisor();
                        // 退出清理里任何一步卡住（例如等待启动器或常驻代理），
                        // 进程就会停在「已阻止退出、尚未完成」的状态：托盘菜单点了没反应，
                        // 再点也不会重来（SHUTDOWN_STARTED 已经是 true），任务管理器里也
                        // 只能强杀（#2611）。这里给清理设一个上限，到点直接退出，
                        // 先写日志留下是超时退出的证据。
                        std::thread::spawn(|| {
                            std::thread::sleep(std::time::Duration::from_secs(SHUTDOWN_WATCHDOG));
                            if SHUTDOWN_COMPLETE.load(Ordering::SeqCst) {
                                return;
                            }
                            let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                                "app.shutdown_watchdog_forced",
                                serde_json::json!({ "waitedSeconds": SHUTDOWN_WATCHDOG }),
                            );
                            std::process::exit(0);
                        });
                        let app = app_handle.clone();
                        tauri::async_runtime::spawn_blocking(move || {
                            // 退出前先等待自有 CLI 任务清理，避免进程退出后遗留执行器。
                            tauri::async_runtime::block_on(
                                codex_plus_core::agent_collaboration::executor::shutdown_owned_tasks(),
                            );
                            if let Err(error) = codex_plus_launcher::shutdown() {
                                let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                                    "app.shutdown_failed",
                                    serde_json::json!({ "message": error.to_string() }),
                                );
                            }
                            // 常驻本地代理只在 Codex++ 退出时停止；Codex 会话结束
                            // 不会关它（见 core::resident_helper）。
                            codex_plus_core::resident_helper::stop_resident_helper();
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
                        if handle_dream_skin_url(url.as_str()) {
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
        .on_tray_icon_event(|tray, event| {
            if tray_event_shows_main_window(&event) {
                show_main_window(tray.app_handle());
            }
        });

    // 左键单击是否弹出菜单按平台区分（issue #2676）：Windows 的托盘惯例是
    // 左键唤起主窗口、右键弹菜单；macOS 菜单栏图标惯例是任意点击弹出菜单，
    // Linux 同样保持既有行为不变。
    #[cfg(windows)]
    {
        tray_builder = tray_builder.show_menu_on_left_click(false);
    }
    #[cfg(not(windows))]
    {
        tray_builder = tray_builder.show_menu_on_left_click(true);
    }

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

// 托盘图标事件是否唤起主窗口（issue #2676）。Windows 遵循「左键单击唤窗、
// 右键弹菜单」的平台惯例，单击在按钮抬起时触发一次；其余平台左键仍由托盘
// 菜单处理，只保留原有的左键双击唤窗行为，macOS / Linux 行为不变。
fn tray_event_shows_main_window(event: &TrayIconEvent) -> bool {
    let left_click_up = matches!(
        event,
        TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        }
    );
    let left_double_click = matches!(
        event,
        TrayIconEvent::DoubleClick {
            button: MouseButton::Left,
            ..
        }
    );
    left_double_click || (cfg!(windows) && left_click_up)
}

#[cfg(any(target_os = "macos", test))]
fn macos_tray_icon() -> tauri::image::Image<'static> {
    tauri::include_image!("icons/icon.png")
}

#[cfg(test)]
mod tray_tests {
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent, TrayIconId};
    use tauri::{PhysicalPosition, Rect};

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

    fn click(button: MouseButton, button_state: MouseButtonState) -> TrayIconEvent {
        TrayIconEvent::Click {
            id: TrayIconId::new(super::TRAY_ID),
            position: PhysicalPosition::default(),
            rect: Rect::default(),
            button,
            button_state,
        }
    }

    fn double_click(button: MouseButton) -> TrayIconEvent {
        TrayIconEvent::DoubleClick {
            id: TrayIconId::new(super::TRAY_ID),
            position: PhysicalPosition::default(),
            rect: Rect::default(),
            button,
        }
    }

    // issue #2676：左键单击唤窗只在 Windows 生效；右键留给托盘菜单，
    // 按下的中间过程不触发；左键双击唤窗是各平台原有行为，保持不变。
    // CI 只在 Windows 跑测试，平台门控的守护见 windows_subsystem.rs 的
    // 文本断言；这里按平台各写死字面量，避免 assert_eq!(…, cfg!(windows))
    // 在单一平台上沦为同义反复。
    #[test]
    fn tray_left_click_restores_the_main_window_only_on_windows() {
        #[cfg(windows)]
        assert!(super::tray_event_shows_main_window(&click(MouseButton::Left, MouseButtonState::Up)));
        #[cfg(not(windows))]
        assert!(!super::tray_event_shows_main_window(&click(MouseButton::Left, MouseButtonState::Up)));
        assert!(!super::tray_event_shows_main_window(&click(
            MouseButton::Right,
            MouseButtonState::Up
        )));
        assert!(!super::tray_event_shows_main_window(&click(
            MouseButton::Middle,
            MouseButtonState::Up
        )));
        assert!(!super::tray_event_shows_main_window(&click(
            MouseButton::Left,
            MouseButtonState::Down
        )));
        assert!(super::tray_event_shows_main_window(&double_click(MouseButton::Left)));
        assert!(!super::tray_event_shows_main_window(&double_click(MouseButton::Right)));
        assert!(!super::tray_event_shows_main_window(&TrayIconEvent::Enter {
            id: TrayIconId::new(super::TRAY_ID),
            position: PhysicalPosition::default(),
            rect: Rect::default(),
        }));
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
            // Windows 上 Tao 的 VISIBLE 标记可能停在 false（窗口曾被 Win32 直接显示
            // 或隐藏），此时 hide() 会被框架当成重复操作跳过，窗口既不关闭也不隐藏
            // （issue #1542）。先强制同步为可见再隐藏，保证 hide() 真正执行。
            #[cfg(windows)]
            let _ = close_event_window.show();
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
