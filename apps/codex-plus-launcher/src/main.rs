#![cfg_attr(windows, windows_subsystem = "windows")]

use anyhow::{Context, Result};
use codex_plus_core::launcher::{
    BridgeReinjector, DefaultLaunchHooks, LaunchHooks, LaunchOptions, launch_and_inject_with_hooks,
};
use codex_plus_core::models::{DeleteResult, ExportResult, SessionRef};
use codex_plus_core::routes::{BridgeContext, BridgeDataService, BridgeRuntimeService};
use codex_plus_core::status::LaunchStatus;
use codex_plus_core::user_scripts::UserScriptManager;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
struct LauncherHooks {
    core: Arc<DefaultLaunchHooks>,
    data: Arc<LauncherDataService>,
    runtime: Arc<LauncherRuntimeService>,
    bridge_context: Arc<Mutex<Option<BridgeContext>>>,
    browser_monitor: Arc<Mutex<Option<codex_plus_core::native_browser::BrowserMonitor>>>,
}

impl Default for LauncherHooks {
    fn default() -> Self {
        Self {
            core: Arc::new(DefaultLaunchHooks::default()),
            data: Arc::new(LauncherDataService::default()),
            runtime: Arc::new(LauncherRuntimeService::new(
                9229,
                default_user_script_manager(),
            )),
            bridge_context: Arc::new(Mutex::new(None)),
            browser_monitor: Arc::new(Mutex::new(None)),
        }
    }
}

impl LauncherHooks {
    fn watchdog_bridge_context(&self) -> anyhow::Result<BridgeContext> {
        self.bridge_context
            .lock()
            .map_err(|_| anyhow::anyhow!("bridge context lock poisoned"))?
            .clone()
            .ok_or_else(|| anyhow::anyhow!("bridge context is not initialized"))
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let helper_only = args.iter().any(|arg| arg == "--helper-only");
    let options = parse_launch_options(args.iter());
    if let Err(error) = launcher_main(args, helper_only, options.clone()).await {
        let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
            "launcher.failed",
            json!({
                "message": error.to_string()
            }),
        );
        if !helper_only {
            let _ = options.status_store.save_latest(&LaunchStatus {
                status: "failed".to_string(),
                message: error.to_string(),
                started_at_ms: current_timestamp_ms(),
                debug_port: Some(options.debug_port),
                helper_port: Some(options.helper_port),
                codex_app: options
                    .app_dir
                    .map(|path| path.to_string_lossy().to_string()),
                aumid: None,
                // 终态不带进行中的阶段（issue #2244 的 phase/progress）。
                ..LaunchStatus::default()
            });
        }
        return Err(error);
    }
    Ok(())
}

async fn launcher_main(args: Vec<String>, helper_only: bool, options: LaunchOptions) -> Result<()> {
    if helper_only {
        let hooks = LauncherHooks::default();
        hooks.start_helper(options.helper_port).await?;
        std::future::pending::<()>().await;
        hooks.shutdown_helper(options.helper_port).await;
        return Ok(());
    }
    ensure_weixin_manager_started();
    let Some(_guard) = acquire_single_instance_guard(options.debug_port)? else {
        activate_existing_codex_app(&options).await?;
        options.status_store.save_latest(&LaunchStatus {
            status: "running".to_string(),
            message: "Existing Codex instance activated".to_string(),
            started_at_ms: current_timestamp_ms(),
            debug_port: Some(options.debug_port),
            helper_port: Some(options.helper_port),
            codex_app: options
                .app_dir
                .map(|path| path.to_string_lossy().to_string()),
            aumid: None,
            ..LaunchStatus::default()
        })?;
        return Ok(());
    };
    tokio::spawn(async {
        let _ = notify_manager_when_update_available().await;
    });
    let hooks = LauncherHooks::default();
    let handle = launch_and_inject_with_hooks(options, &hooks).await?;
    run_periodic_until_exit(
        handle.wait_for_codex_exit(),
        std::time::Duration::from_secs(30 * 60),
        || repair_session_index_automatically(true),
    )
    .await?;
    Ok(())
}

// 退出时不再安排下一次检查；已开始的数据库事务先完成，避免脱离启动器生命周期。
async fn run_periodic_until_exit<F, T, C, W>(exit: F, interval: std::time::Duration, mut check: C) -> T
where
    F: std::future::Future<Output = T>,
    C: FnMut() -> W,
    W: std::future::Future<Output = ()>,
{
    tokio::pin!(exit);
    loop {
        tokio::select! {
            biased;
            result = &mut exit => return result,
            _ = tokio::time::sleep(interval) => check().await,
        }
    }
}

async fn repair_session_index_automatically(check_setting: bool) {
    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        if check_setting && !codex_plus_core::settings::SettingsStore::default().load()?.provider_sync_enabled {
            return Ok(());
        }
        codex_plus_data::repair_session_index(None)?;
        Ok(())
    })
    .await
    .map_err(anyhow::Error::from)
    .and_then(|result| result);
    if let Err(error) = result {
        let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
            "launcher.session_index_repair.failed",
            json!({ "message": error.to_string() }),
        );
    }
}

/// 「激活已有实例」路径上的供应商同步。
///
/// 与完整启动流程保持一致：仅在设置里启用了供应商同步时执行，
/// 前后各取一次 app state 快照；会话索引修复由 run_provider_sync 本身串联
/// （见 LauncherHooks::run_provider_sync）。同步失败只记日志——用户这次点击
/// 的诉求是把已有窗口拉到前台，不能因为同步失败就整个中止。
async fn run_activation_provider_sync(
    hooks: &LauncherHooks,
    settings: &codex_plus_core::settings::BackendSettings,
) {
    if !settings.provider_sync_enabled {
        return;
    }
    let home = codex_plus_core::relay_config::default_codex_home_dir();
    codex_plus_core::codex_app_state::capture_app_state_snapshot_nonfatal(
        &home,
        "launcher.activate_existing.before",
    );
    if let Err(error) = hooks.run_provider_sync().await {
        let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
            "launcher.activate_existing_provider_sync.failed",
            json!({ "message": error.to_string() }),
        );
    }
    codex_plus_core::codex_app_state::sync_app_state_after_provider_switch_nonfatal(
        &home,
        "launcher.activate_existing.after_provider_sync",
    );
}

fn current_timestamp_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn ensure_weixin_manager_started() {
    let result = (|| -> anyhow::Result<()> {
        let settings = codex_plus_core::settings::SettingsStore::default().load()?;
        if should_start_weixin_manager(settings.weixin_connect_enabled, &settings.weixin_connect_token) {
            codex_plus_core::install::spawn_companion(
                codex_plus_core::install::MANAGER_BINARY,
                ["--background"],
            )?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
            "launcher.weixin_manager_start_failed",
            serde_json::json!({ "error": error.to_string() }),
        );
    }
}

fn should_start_weixin_manager(enabled: bool, token: &str) -> bool {
    enabled && !token.trim().is_empty()
}

#[cfg(test)]
mod weixin_startup_tests {
    use super::should_start_weixin_manager;

    #[test]
    fn only_enabled_and_authenticated_connections_start_manager() {
        assert!(should_start_weixin_manager(true, "test-token"));
        assert!(!should_start_weixin_manager(false, "test-token"));
        assert!(!should_start_weixin_manager(true, ""));
        assert!(!should_start_weixin_manager(true, "   "));
    }
}

fn acquire_single_instance_guard(
    debug_port: u16,
) -> anyhow::Result<Option<codex_plus_core::ports::LoopbackPortGuard>> {
    acquire_single_instance_guard_with_retry(debug_port, true)
}

fn acquire_single_instance_guard_with_retry(
    debug_port: u16,
    allow_stale_recovery: bool,
) -> anyhow::Result<Option<codex_plus_core::ports::LoopbackPortGuard>> {
    match try_acquire_single_instance_guard() {
        Ok(guard) => {
            if let Some(fallback_lock_path) = guard.fallback_path() {
                log_launcher_guard_fallback(fallback_lock_path);
            }
            Ok(Some(guard))
        }
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::AddrInUse
            ) =>
        {
            log_launcher_already_running(debug_port);
            let stale = allow_stale_recovery && should_recover_stale_launcher(debug_port);
            if should_retry_stale_launcher_guard(error.kind(), allow_stale_recovery, stale) {
                codex_plus_core::watcher::stop_launcher_processes();
                std::thread::sleep(std::time::Duration::from_millis(250));
                return acquire_single_instance_guard_with_retry(debug_port, false);
            }
            Ok(None)
        }
        Err(error) => Err(error)
            .with_context(|| {
                format!(
                    "failed to acquire launcher guard port {}",
                    codex_plus_core::ports::launcher_guard_port()
                )
            })
            .map(Some),
    }
}

fn should_retry_stale_launcher_guard(
    error_kind: std::io::ErrorKind,
    allow_stale_recovery: bool,
    stale_launcher: bool,
) -> bool {
    allow_stale_recovery
        && stale_launcher
        && matches!(
            error_kind,
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::AddrInUse
        )
}

fn try_acquire_single_instance_guard() -> std::io::Result<codex_plus_core::ports::LoopbackPortGuard>
{
    codex_plus_core::ports::acquire_resilient_loopback_port_guard(
        codex_plus_core::ports::launcher_guard_port(),
    )
}

fn log_launcher_guard_fallback(fallback_lock_path: &Path) {
    let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
        "launcher.guard_fallback",
        json!({
            "requested_guard_port": codex_plus_core::ports::launcher_guard_port(),
            "fallback_lock_path": fallback_lock_path
        }),
    );
}

fn should_recover_stale_launcher(debug_port: u16) -> bool {
    let has_codex_process = !codex_plus_core::watcher::find_codex_processes().is_empty();
    let cdp_listening = codex_plus_core::watcher::cdp_listening(debug_port);
    let recover =
        codex_plus_core::watcher::should_recover_stale_launcher(has_codex_process, cdp_listening);
    let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
        "launcher.stale_recovery_check",
        json!({
            "debug_port": debug_port,
            "has_codex_process": has_codex_process,
            "cdp_listening": cdp_listening,
            "recover": recover
        }),
    );
    recover
}

async fn activate_existing_codex_app(options: &LaunchOptions) -> anyhow::Result<()> {
    let hooks = LauncherHooks::default();
    let settings = hooks.load_settings().await?;
    let app_dir = hooks.resolve_app_dir(options.app_dir.as_deref(), &settings)?;
    let has_pending_recovery = hooks.has_pending_remote_control_session_recoveries();
    let blocking_process_ids = if has_pending_recovery {
        codex_plus_core::watcher::find_session_index_cleanup_blocking_processes()
    } else {
        Vec::new()
    };
    if should_finalize_pending_remote_control_recovery(has_pending_recovery, &blocking_process_ids)
    {
        hooks.run_remote_control_session_recovery().await?;
    } else if has_pending_recovery {
        let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
            "launcher.remote_control_session_finalization_deferred_existing_app",
            json!({"blocking_process_ids": blocking_process_ids}),
        );
    }
    // 快捷方式启动且 Codex 已在运行时，这里会走到「激活已有实例」的早退分支。
    // 过去该分支直接返回，跳过了完整启动流程里的供应商同步与会话索引修复，
    // 于是「切换登录方式后 model_provider 没跟着切换」「历史会话没被自动修复」，
    // 只有管理工具的「重启」才生效（issue #2080）。失败不阻断激活，只记日志。
    run_activation_provider_sync(&hooks, &settings).await;
    let launch_result = hooks
        .launch_codex(
            &app_dir,
            options.debug_port,
            &settings,
            &settings.codex_extra_args,
        )
        .await;
    let process_ids = codex_plus_core::watcher::find_codex_processes();
    #[cfg(windows)]
    let activated = process_ids
        .iter()
        .copied()
        .any(codex_plus_core::windows_activate_process_window);
    #[cfg(not(windows))]
    let activated = false;
    let helper_available = !settings.enhancements_enabled
        || codex_plus_core::ports::can_connect_loopback_port(options.helper_port);
    let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
        "launcher.activate_existing_codex",
        json!({
            "app_dir": app_dir.to_string_lossy(),
            "debug_port": options.debug_port,
            "helper_port": options.helper_port,
            "requested_helper_port": options.helper_port,
            "process_ids": process_ids,
            "activated": activated,
            "helper_available": helper_available,
            "launch_ok": launch_result.is_ok(),
            "launch_error": launch_result.as_ref().err().map(|error| error.to_string())
        }),
    );
    launch_result.map(|_| ())
}

fn should_finalize_pending_remote_control_recovery(
    has_pending_recovery: bool,
    blocking_process_ids: &[u32],
) -> bool {
    has_pending_recovery && blocking_process_ids.is_empty()
}

fn log_launcher_already_running(debug_port: u16) {
    let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
        "launcher.already_running",
        json!({
            "guard_port": codex_plus_core::ports::launcher_guard_port(),
            "debug_port": debug_port
        }),
    );
}

async fn notify_manager_when_update_available() -> anyhow::Result<bool> {
    let update =
        codex_plus_core::update::check_for_update(codex_plus_core::version::VERSION).await?;
    if !update.update_available {
        return Ok(false);
    }
    open_manager_with_update_prompt()?;
    Ok(true)
}

fn open_manager_with_update_prompt() -> anyhow::Result<()> {
    codex_plus_core::install::spawn_companion(
        codex_plus_core::install::MANAGER_BINARY,
        ["--show-update", "--background"],
    )
    .map(|_| ())
    .map_err(|error| anyhow::anyhow!("启动管理工具失败：{error}"))
}

fn parse_launch_options<I, S>(args: I) -> LaunchOptions
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut options = LaunchOptions::default();
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_ref() {
            "--app-path" => {
                if let Some(value) = iter.next() {
                    let value = value.as_ref().trim();
                    if !value.is_empty() {
                        options.app_dir = Some(PathBuf::from(value));
                    }
                }
            }
            "--debug-port" => {
                if let Some(value) = iter.next() {
                    if let Ok(port) = value.as_ref().parse::<u16>() {
                        options.debug_port = port;
                    }
                }
            }
            "--helper-port" => {
                if let Some(value) = iter.next() {
                    if let Ok(port) = value.as_ref().parse::<u16>() {
                        options.helper_port = port;
                    }
                }
            }
            _ => {}
        }
    }
    options
}

#[async_trait::async_trait(?Send)]
impl LaunchHooks for LauncherHooks {
    fn resolve_app_dir(
        &self,
        app_dir: Option<&std::path::Path>,
        settings: &codex_plus_core::settings::BackendSettings,
    ) -> anyhow::Result<std::path::PathBuf> {
        self.core.resolve_app_dir(app_dir, settings)
    }

    fn select_debug_port(&self, requested: u16) -> u16 {
        self.core.select_debug_port(requested)
    }

    fn select_helper_port(&self, requested: u16) -> u16 {
        self.core.select_helper_port(requested)
    }

    async fn load_settings(&self) -> anyhow::Result<codex_plus_core::settings::BackendSettings> {
        self.core.load_settings().await
    }

    async fn start_native_browser_compatibility(&self, settings: &codex_plus_core::settings::BackendSettings) {
        let monitor = codex_plus_core::native_browser::start_monitor(
            settings.enhancements_enabled && settings.codex_app_native_browser_require_identification,
        ).await;
        *self.browser_monitor.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = monitor;
    }

    async fn stop_native_browser_compatibility(&self) {
        let monitor = self.browser_monitor.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take();
        if let Some(monitor) = monitor {
            monitor.stop().await;
        }
    }

    fn cleanup_unsupported_config(&self) -> anyhow::Result<()> {
        self.core.cleanup_unsupported_config()
    }

    async fn run_provider_sync(&self) -> anyhow::Result<()> {
        // issue #2160：同步失败不再中断启动。前置读取（典型是 .codex-global-state.json
        // 为空或被截断）失败时，provider sync 返回 Skipped 并带上底层 serde_json 原文；
        // 以前这里用 `?` 把它变成致命的，用户看到一句无从下手的英文就直接退出了。
        // 口径与 run_activation_provider_sync 一致：失败只记诊断日志，不阻断启动。
        let outcome =
            tokio::task::spawn_blocking(|| codex_plus_data::run_provider_sync(None)).await;
        match outcome {
            Ok(result) => {
                if let Err(error) = require_completed_provider_sync(&result.status, &result.message)
                {
                    let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                        "launcher.provider_sync.degraded",
                        json!({
                            "status": format!("{:?}", result.status),
                            "message": error.to_string(),
                        }),
                    );
                }
            }
            Err(error) => {
                let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                    "launcher.provider_sync.degraded",
                    json!({ "message": format!("provider sync task failed: {error}") }),
                );
            }
        }
        // 同步没做成，索引该修的还是要修——它们各自独立，不能因为一路失败连坐。
        repair_session_index_automatically(false).await;
        Ok(())
    }

    fn has_pending_remote_control_session_recoveries(&self) -> bool {
        codex_plus_core::paths::default_pending_remote_control_recovery_path().exists()
    }

    fn remote_control_session_recovery_is_safe_to_run(&self) -> bool {
        codex_plus_core::watcher::find_session_index_cleanup_blocking_processes().is_empty()
    }

    async fn run_remote_control_session_recovery(&self) -> anyhow::Result<()> {
        let outcomes = tokio::task::spawn_blocking(|| {
            let requests = codex_plus_core::remote_control_recovery::load_pending_remote_control_recoveries(None)?;
            let settings = codex_plus_core::settings::SettingsStore::default()
                .load()?;
            let mut outcomes = Vec::with_capacity(requests.len());
            for request in requests {
                let current_profile = settings
                    .relay_profiles
                    .iter()
                    .find(|profile| profile.id == request.profile_id);
                if remote_control_recovery_is_superseded_by_openai(&settings, &request) {
                    let completion_error =
                        codex_plus_core::remote_control_recovery::complete_pending_remote_control_recovery(
                            None,
                            &request.thread_id,
                        )
                        .err()
                        .map(|error| error.to_string());
                    let completed = completion_error.is_none();
                    outcomes.push((
                        request,
                        codex_plus_data::ProviderSyncResult {
                            status: if completed {
                                codex_plus_data::ProviderSyncStatus::Synced
                            } else {
                                codex_plus_data::ProviderSyncStatus::Skipped
                            },
                            message: if completed {
                                "Remote Control session finalization discarded after switching to OpenAI session identity".to_string()
                            } else {
                                "Remote Control session finalization could not discard the superseded recovery request".to_string()
                            },
                            target_provider: "openai".to_string(),
                            backup_dir: None,
                            changed_session_files: 0,
                            sqlite_rows_updated: 0,
                            sqlite_provider_rows_updated: 0,
                            sqlite_user_event_rows_updated: 0,
                            sqlite_cwd_rows_updated: 0,
                            sqlite_catalog_rows_inserted: 0,
                            sqlite_catalog_rows_removed: 0,
                            updated_workspace_roots: 0,
                            skipped_locked_rollout_files: Vec::new(),
                            encrypted_content_warning: None,
                            repair_audit: codex_plus_data::ProviderSyncAudit::default(),
                        },
                        completion_error,
                    ));
                    continue;
                }
                let request_is_current = settings.active_relay_id == request.profile_id
                    && current_profile.is_some_and(|profile| {
                    codex_plus_core::remote_control_recovery::config_generation(
                        profile,
                        &request.target_provider,
                    ) == request.config_generation
                });
                if !request_is_current {
                    outcomes.push((
                        request,
                        codex_plus_data::ProviderSyncResult {
                            status: codex_plus_data::ProviderSyncStatus::Skipped,
                            message: "Remote Control session finalization deferred after relay profile changed".to_string(),
                            target_provider: String::new(),
                            backup_dir: None,
                            changed_session_files: 0,
                            sqlite_rows_updated: 0,
                            sqlite_provider_rows_updated: 0,
                            sqlite_user_event_rows_updated: 0,
                            sqlite_cwd_rows_updated: 0,
                            sqlite_catalog_rows_inserted: 0,
                            sqlite_catalog_rows_removed: 0,
                            updated_workspace_roots: 0,
                            skipped_locked_rollout_files: Vec::new(),
                            encrypted_content_warning: None,
                            repair_audit: codex_plus_data::ProviderSyncAudit::default(),
                        },
                        None,
                    ));
                    continue;
                }
                let result = codex_plus_data::run_remote_control_session_finalization_for_thread_with_target(
                    None,
                    &request.thread_id,
                    &request.target_provider,
                );
                let completed = result.status == codex_plus_data::ProviderSyncStatus::Synced;
                let completion_error = if completed {
                    codex_plus_core::remote_control_recovery::complete_pending_remote_control_recovery(
                        None,
                        &request.thread_id,
                    )
                    .err()
                    .map(|error| error.to_string())
                } else {
                    None
                };
                outcomes.push((request, result, completion_error));
            }
            Ok::<_, anyhow::Error>(outcomes)
        })
        .await
        .map_err(|error| anyhow::anyhow!("Remote Control session recovery task failed: {error}"))?;
        match outcomes {
            Ok(outcomes) => {
                for (request, result, completion_error) in outcomes {
                    let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                        "launcher.remote_control_session_finalization",
                        json!({
                            "thread_id": request.thread_id,
                            "profile_id": request.profile_id,
                            "target_provider": request.target_provider,
                            "config_generation": request.config_generation,
                            "status": result.status,
                            "message": result.message,
                            "completion_error": completion_error
                        }),
                    );
                }
            }
            Err(error) => {
                let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                    "launcher.remote_control_session_finalization_failed_nonfatal",
                    json!({"message": error.to_string()}),
                );
            }
        }
        Ok(())
    }

    async fn apply_active_relay_profile(
        &self,
        settings: &codex_plus_core::settings::BackendSettings,
    ) -> anyhow::Result<()> {
        self.core.apply_active_relay_profile(settings).await
    }

    async fn ensure_active_protocol_proxy_config(
        &self,
        settings: &codex_plus_core::settings::BackendSettings,
    ) -> anyhow::Result<()> {
        self.core
            .ensure_active_protocol_proxy_config(settings)
            .await
    }


    async fn start_helper(&self, helper_port: u16) -> anyhow::Result<()> {
        self.core.start_helper(helper_port).await
    }

    async fn launch_codex(
        &self,
        app_dir: &Path,
        debug_port: u16,
        settings: &codex_plus_core::settings::BackendSettings,
        extra_args: &[String],
    ) -> anyhow::Result<codex_plus_core::launcher::CodexLaunch> {
        self.core
            .launch_codex(app_dir, debug_port, settings, extra_args)
            .await
    }

    async fn bridge_context(
        &self,
        debug_port: u16,
        app_dir: &Path,
    ) -> anyhow::Result<Option<BridgeContext>> {
        self.runtime.set_debug_port(debug_port);
        let ctx = BridgeContext::core_with_data_and_app_dir(
            self.runtime.clone(),
            self.data.clone(),
            app_dir.to_path_buf(),
        );
        *self
            .bridge_context
            .lock()
            .map_err(|_| anyhow::anyhow!("bridge context lock poisoned"))? = Some(ctx.clone());
        Ok(Some(ctx))
    }

    async fn inject_bridge(
        &self,
        debug_port: u16,
        helper_port: u16,
        ctx: BridgeContext,
    ) -> anyhow::Result<()> {
        inject_with_context(debug_port, helper_port, ctx, self.runtime.clone()).await
    }

    async fn inject(&self, debug_port: u16, helper_port: u16) -> anyhow::Result<()> {
        self.core.inject(debug_port, helper_port).await
    }

    async fn capture_injected_launch_identity(&self, debug_port: u16) {
        self.core.capture_injected_launch_identity(debug_port).await;
    }

    async fn start_bridge_watchdog(&self, debug_port: u16, helper_port: u16) -> anyhow::Result<()> {
        let ctx = self.watchdog_bridge_context()?;
        let runtime = self.runtime.clone();
        let reinjector: BridgeReinjector = Arc::new(move || {
            let ctx = ctx.clone();
            let runtime = runtime.clone();
            Box::pin(
                async move { inject_with_context(debug_port, helper_port, ctx, runtime).await },
            )
        });
        self.core.set_bridge_reinjector(reinjector).await;
        self.core
            .start_bridge_watchdog(debug_port, helper_port)
            .await
    }

    async fn write_status(&self, status: &str) {
        self.core.write_status(status).await;
    }

    async fn wait_for_codex_exit(
        &self,
        launch: &codex_plus_core::launcher::CodexLaunch,
        debug_port: u16,
    ) -> anyhow::Result<()> {
        self.core.wait_for_codex_exit(launch, debug_port).await
    }

    async fn shutdown_helper(&self, helper_port: u16) {
        self.core.shutdown_helper(helper_port).await;
    }

    async fn terminate_codex(&self, launch: &codex_plus_core::launcher::CodexLaunch) {
        self.core.terminate_codex(launch).await;
    }
}

fn require_completed_provider_sync(
    status: &codex_plus_data::ProviderSyncStatus,
    message: &str,
) -> anyhow::Result<()> {
    if *status == codex_plus_data::ProviderSyncStatus::Synced {
        return Ok(());
    }
    anyhow::bail!("provider sync did not complete ({status:?}): {message}")
}

#[derive(Debug, Clone)]
struct LauncherDataService {
    db_path: PathBuf,
    backup_dir: PathBuf,
}

impl Default for LauncherDataService {
    fn default() -> Self {
        Self {
            db_path: default_codex_db_path(),
            backup_dir: codex_plus_core::paths::default_app_state_dir().join("backups"),
        }
    }
}

#[async_trait::async_trait]
impl BridgeDataService for LauncherDataService {
    async fn delete(&self, session: SessionRef) -> anyhow::Result<DeleteResult> {
        // 只读本地 SQLite 的实现不能接收远端/未知来源；同 ID 不代表同一主机。
        session.require_local_delete()?;
        let db_paths = self.candidate_db_paths();
        let backup_store = codex_plus_data::BackupStore::new(self.backup_dir.clone());
        tokio::task::spawn_blocking(move || {
            codex_plus_data::delete_local_from_paths(
                db_paths,
                backup_store,
                &session,
                Some(&codex_plus_core::codex_sqlite::default_codex_home_dir()),
            )
        })
        .await
        .map_err(|error| anyhow::anyhow!("delete task failed: {error}"))
    }

    async fn undo(&self, undo_token: String) -> anyhow::Result<DeleteResult> {
        let adapter = self.storage_adapter();
        tokio::task::spawn_blocking(move || adapter.undo(&undo_token))
            .await
            .map_err(|error| anyhow::anyhow!("undo task failed: {error}"))
    }

    async fn export_markdown(&self, session: SessionRef) -> anyhow::Result<ExportResult> {
        let db_paths = self.candidate_db_paths();
        tokio::task::spawn_blocking(move || {
            codex_plus_data::export_markdown_from_paths(db_paths, &session)
        })
        .await
        .map_err(|error| anyhow::anyhow!("export markdown task failed: {error}"))
    }

    async fn thread_usage_history(&self, session: SessionRef) -> anyhow::Result<Value> {
        let adapter = self.storage_adapter();
        tokio::task::spawn_blocking(move || adapter.codex_thread_usage_history(&session))
            .await
            .map_err(|error| anyhow::anyhow!("thread usage history task failed: {error}"))
    }

    async fn whale_session(&self, session: SessionRef) -> anyhow::Result<Value> {
        let adapter = self.storage_adapter();
        tokio::task::spawn_blocking(move || {
            codex_plus_data::whale_usage::session_summary(&adapter, &session)
        })
        .await
        .map_err(|error| anyhow::anyhow!("whale session task failed: {error}"))
    }

    async fn whale_history(&self, query: Value) -> anyhow::Result<Value> {
        let home = codex_plus_core::codex_sqlite::default_codex_home_dir();
        tokio::task::spawn_blocking(move || {
            codex_plus_data::whale_history::query_history(&home, &query)
        })
        .await
        .map_err(|error| anyhow::anyhow!("whale history task failed: {error}"))
    }

    async fn find_archived_thread_by_title(
        &self,
        title: String,
    ) -> anyhow::Result<Option<SessionRef>> {
        let adapter = self.storage_adapter();
        tokio::task::spawn_blocking(move || adapter.find_archived_thread_by_title(&title))
            .await
            .map_err(|error| anyhow::anyhow!("archived lookup task failed: {error}"))
    }

    async fn recover_remote_control_session(&self, thread_id: String) -> anyhow::Result<Value> {
        let settings = codex_plus_core::settings::SettingsStore::default()
            .load()
            .unwrap_or_default();
        let profile = settings.active_relay_profile();
        if !settings.relay_profiles_enabled
            || profile.relay_mode != codex_plus_core::settings::RelayMode::Official
            || !profile.official_mix_api_key
        {
            return Ok(json!({
                "status": "skipped",
                "message": "Remote Control session recovery is disabled for the active profile"
            }));
        }
        let home = codex_plus_core::codex_sqlite::default_codex_home_dir();
        let target_provider =
            codex_plus_core::model_catalog::codex_model_provider_for_relay_profile(&home, &profile);
        if target_provider.trim().is_empty() || target_provider == "openai" {
            return Ok(json!({
                "status": "skipped",
                "message": "Remote Control session recovery requires a non-openai target provider"
            }));
        }
        let candidate_thread_id = thread_id.clone();
        let candidate = tokio::task::spawn_blocking(move || {
            codex_plus_data::remote_control_session_recovery_candidate_exists(
                None,
                &candidate_thread_id,
            )
        })
        .await
        .map_err(|error| anyhow::anyhow!("Remote Control candidate check failed: {error}"))??;
        if !candidate {
            return Ok(json!({
                "status": "skipped",
                "message": "Remote Control session recovery is waiting for a recent openai thread"
            }));
        }
        let request = codex_plus_core::remote_control_recovery::PendingRemoteControlRecovery {
            thread_id: thread_id.clone(),
            profile_id: profile.id.clone(),
            target_provider: target_provider.clone(),
            config_generation: codex_plus_core::remote_control_recovery::config_generation(
                &profile,
                &target_provider,
            ),
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64,
        };
        codex_plus_core::remote_control_recovery::enqueue_pending_remote_control_recovery(
            None, request,
        )?;
        tokio::task::spawn_blocking(move || {
            serde_json::to_value(
                codex_plus_data::run_remote_control_session_catalog_recovery_for_thread_with_target(
                    None,
                    &thread_id,
                    &target_provider,
                ),
            )
            .map_err(anyhow::Error::from)
        })
        .await
        .map_err(|error| anyhow::anyhow!("Remote Control session recovery task failed: {error}"))?
    }

    async fn export_session_file(&self, session: SessionRef) -> anyhow::Result<Value> {
        LauncherDataService::export_session_file(self, session).await
    }

    async fn import_session_file(&self, payload: Value) -> anyhow::Result<Value> {
        LauncherDataService::import_session_file(self, payload).await
    }
}

impl LauncherDataService {
    fn candidate_db_paths(&self) -> Vec<PathBuf> {
        let mut paths = vec![self.db_path.clone()];
        for path in codex_plus_core::codex_sqlite::codex_session_db_paths_from_home(
            &codex_plus_core::codex_sqlite::default_codex_home_dir(),
        ) {
            if !paths.iter().any(|candidate| candidate == &path) {
                paths.push(path);
            }
        }
        paths
    }

    fn storage_adapter(&self) -> codex_plus_data::SQLiteStorageAdapter {
        let allowed_db_paths = self.candidate_db_paths();
        codex_plus_data::SQLiteStorageAdapter::new(
            self.db_path.clone(),
            codex_plus_data::BackupStore::new(self.backup_dir.clone()),
        )
        .with_allowed_db_paths(allowed_db_paths)
        .with_codex_home(codex_plus_core::codex_sqlite::default_codex_home_dir())
    }

    async fn export_session_file(&self, session: SessionRef) -> anyhow::Result<Value> {
        let home = codex_plus_core::codex_sqlite::default_codex_home_dir();
        tokio::task::spawn_blocking(move || {
            codex_plus_core::session_share::export_rollout(&home, &session.session_id)
        })
        .await
        .map_err(|error| anyhow::anyhow!("session export task failed: {error}"))?
    }

    async fn import_session_file(&self, payload: Value) -> anyhow::Result<Value> {
        let home = codex_plus_core::codex_sqlite::default_codex_home_dir();
        tokio::task::spawn_blocking(move || {
            codex_plus_core::session_share::import_rollout(&home, &payload)
        })
        .await
        .map_err(|error| anyhow::anyhow!("session import task failed: {error}"))?
    }
}

struct LauncherRuntimeService {
    debug_port: Mutex<u16>,
    websocket_url: Mutex<Option<String>>,
    user_scripts: UserScriptManager,
}

impl LauncherRuntimeService {
    fn new(debug_port: u16, user_scripts: UserScriptManager) -> Self {
        Self {
            debug_port: Mutex::new(debug_port),
            websocket_url: Mutex::new(None),
            user_scripts,
        }
    }

    fn set_debug_port(&self, debug_port: u16) {
        *self.debug_port.lock().unwrap() = debug_port;
    }

    fn set_websocket_url(&self, websocket_url: &str) {
        *self.websocket_url.lock().unwrap() = Some(websocket_url.to_string());
    }
}

#[async_trait::async_trait]
impl BridgeRuntimeService for LauncherRuntimeService {
    async fn user_script_inventory(&self) -> anyhow::Result<Value> {
        self.user_scripts.inventory()
    }

    async fn user_script_inventory_with_runtime_status(
        &self,
        payload: Value,
    ) -> anyhow::Result<Value> {
        self.user_scripts
            .inventory_with_runtime_status(payload.get("runtime_status"))
    }

    async fn set_user_scripts_enabled(&self, enabled: bool) -> anyhow::Result<Value> {
        self.user_scripts.set_global_enabled(enabled)?;
        self.user_scripts.inventory()
    }

    async fn set_user_script_enabled(&self, key: String, enabled: bool) -> anyhow::Result<Value> {
        self.user_scripts.set_script_enabled(&key, enabled)?;
        self.user_scripts.inventory()
    }

    async fn delete_user_script(&self, key: String) -> anyhow::Result<Value> {
        self.user_scripts.delete_user_script(&key)?;
        self.user_scripts.inventory()
    }

    async fn load_user_scripts(&self) -> anyhow::Result<Value> {
        let websocket_url = self
            .websocket_url
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Codex 页面尚未连接"))?;
        codex_plus_core::user_scripts::load_scripts_at(&websocket_url, &self.user_scripts).await
    }

    async fn reload_user_scripts(&self) -> anyhow::Result<Value> {
        let websocket_url = self
            .websocket_url
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Codex 页面尚未连接"))?;
        codex_plus_core::user_scripts::reload_scripts_at(&websocket_url, &self.user_scripts).await
    }

    async fn script_market_list(&self) -> anyhow::Result<Value> {
        codex_plus_core::script_market::list_market_scripts(&self.user_scripts).await
    }

    async fn script_market_install(&self, payload: Value) -> anyhow::Result<Value> {
        let id = payload
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default();
        if id.is_empty() {
            anyhow::bail!("脚本 id 不能为空");
        }
        codex_plus_core::script_market::install_market_script_by_id(&self.user_scripts, id).await
    }

    async fn open_devtools(&self) -> anyhow::Result<Value> {
        let debug_port = *self.debug_port.lock().unwrap();
        let targets = codex_plus_core::cdp::list_targets(debug_port).await?;
        let target = codex_plus_core::cdp::pick_page_target(&targets)?;
        let url = codex_plus_core::routes::devtools_url(debug_port, &target.id);
        open_url(&url)?;
        Ok(json!({
            "status": "ok",
            "target_id": target.id,
            "url": url
        }))
    }

    async fn open_manager(&self, payload: Value) -> anyhow::Result<Value> {
        let navigation =
            codex_plus_core::manager_navigation::save_pending_manager_navigation_from_payload(
                &payload,
            )?;
        let target = codex_plus_core::install::open_or_activate_manager()
            .map_err(|error| anyhow::anyhow!("启动管理工具失败：{error}"))
            .map_err(|error| {
                codex_plus_core::manager_navigation::rollback_pending_manager_navigation_after_launch_failure(
                    navigation.as_ref(),
                    error,
                )
            })?;
        Ok(json!({
            "status": "ok",
            "path": target,
            "navigation": navigation
        }))
    }

    async fn open_transient_manager(&self, payload: Value) -> anyhow::Result<Value> {
        let navigation =
            codex_plus_core::manager_navigation::save_pending_manager_navigation_from_payload(
                &payload,
            )?;
        let target = codex_plus_core::install::spawn_companion(
            codex_plus_core::install::MANAGER_BINARY,
            ["--transient"],
        )
        .map_err(|error| anyhow::anyhow!("启动管理工具失败：{error}"))
        .map_err(|error| {
            codex_plus_core::manager_navigation::rollback_pending_manager_navigation_after_launch_failure(
                navigation.as_ref(),
                error,
            )
        })?;
        Ok(json!({
            "status": "ok",
            "path": target,
            "navigation": navigation
        }))
    }

    async fn backend_status(&self) -> anyhow::Result<Value> {
        Ok(
            json!({"status": "ok", "message": "后端已连接", "version": codex_plus_core::version::VERSION}),
        )
    }

    async fn codex_model_catalog(&self) -> anyhow::Result<Value> {
        Ok(codex_plus_core::model_catalog::read_codex_model_catalog().await)
    }

    async fn ads(&self) -> anyhow::Result<Value> {
        codex_plus_core::ads::fetch_ad_list().await
    }
}

async fn inject_with_context(
    debug_port: u16,
    helper_port: u16,
    ctx: BridgeContext,
    runtime: Arc<LauncherRuntimeService>,
) -> anyhow::Result<()> {
    let mut last_error = None;
    for _ in 0..20 {
        match try_inject_with_context(debug_port, helper_port, ctx.clone(), runtime.clone()).await {
            Ok(()) => return Ok(()),
            Err(error) => {
                last_error = Some(error);
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("Codex injection failed")))
}

fn remote_control_recovery_is_superseded_by_openai(
    settings: &codex_plus_core::settings::BackendSettings,
    request: &codex_plus_core::remote_control_recovery::PendingRemoteControlRecovery,
) -> bool {
    settings.active_relay_id == request.profile_id
        && settings.active_relay_session_provider()
            == codex_plus_core::settings::RelaySessionProvider::Openai
}

async fn try_inject_with_context(
    debug_port: u16,
    helper_port: u16,
    ctx: BridgeContext,
    runtime: Arc<LauncherRuntimeService>,
) -> anyhow::Result<()> {
    let targets = codex_plus_core::cdp::list_targets(debug_port).await?;
    let target = codex_plus_core::cdp::pick_injectable_codex_page_target(&targets)?;
    let websocket_url = target
        .web_socket_debugger_url
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("selected CDP target has no websocket URL"))?;
    runtime.set_websocket_url(websocket_url);
    let settings = codex_plus_core::settings::SettingsStore::default()
        .load()
        .unwrap_or_default();
    let script = codex_plus_core::assets::injection_script_with_settings(helper_port, &settings);
    let new_document_scripts = vec![
        script,
        codex_plus_core::user_scripts::BOOTSTRAP_SCRIPT.to_string(),
    ];
    codex_plus_core::bridge::install_bridge(
        websocket_url,
        codex_plus_core::bridge::BRIDGE_BINDING_NAME,
        Arc::new(move |path, payload| {
            let ctx = ctx.clone();
            Box::pin(async move {
                Ok(codex_plus_core::routes::handle_bridge_request(ctx, &path, payload).await)
            })
        }),
        &new_document_scripts,
    )
    .await
}

fn default_codex_db_path() -> PathBuf {
    codex_plus_core::codex_sqlite::codex_session_db_path()
}

fn open_url(url: &str) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        codex_plus_core::windows_open_url(url)
            .map_err(|error| anyhow::anyhow!("failed to open DevTools URL: {error}"))
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(url)
            .spawn()
            .map(|_| ())
            .map_err(|error| anyhow::anyhow!("failed to open DevTools URL: {error}"))
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map(|_| ())
            .map_err(|error| anyhow::anyhow!("failed to open DevTools URL: {error}"))
    }

    #[cfg(not(any(windows, target_os = "macos", unix)))]
    {
        let _ = url;
        anyhow::bail!("opening DevTools URL is not supported on this platform")
    }
}

fn default_user_script_manager() -> UserScriptManager {
    let config_dir = default_user_scripts_config_dir();
    UserScriptManager::new(
        builtin_user_scripts_dir(),
        config_dir.join("user_scripts"),
        config_dir.join("user_scripts.json"),
    )
}

fn default_user_scripts_config_dir() -> PathBuf {
    if cfg!(windows) {
        if let Some(roaming) = std::env::var_os("APPDATA") {
            return PathBuf::from(roaming).join("Codex++");
        }
        if let Some(home) = directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf()) {
            return home.join("AppData").join("Roaming").join("Codex++");
        }
    }
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| directories::BaseDirs::new().map(|dirs| dirs.home_dir().join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"))
        .join("Codex++")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn session_index_monitor_does_not_start_after_exit() {
        let checks = std::cell::Cell::new(0);
        let result = run_periodic_until_exit(async { 42 }, std::time::Duration::ZERO, || {
            checks.set(checks.get() + 1);
            std::future::ready(())
        }).await;
        assert_eq!(result, 42);
        assert_eq!(checks.get(), 0);
    }

    #[tokio::test]
    async fn session_index_monitor_finishes_current_check_before_exit() {
        let (done, exit) = tokio::sync::oneshot::channel::<()>();
        let mut done = Some(done);
        let finished = std::cell::Cell::new(false);
        run_periodic_until_exit(exit, std::time::Duration::from_millis(1), || {
            done.take().expect("only one check").send(()).unwrap();
            let finished = &finished;
            async move {
                tokio::task::yield_now().await;
                finished.set(true);
            }
        }).await.unwrap();
        assert!(finished.get());
    }

    #[test]
    fn parse_launch_options_accepts_manager_forwarded_ports_and_app_path() {
        let options = parse_launch_options([
            "--app-path",
            "C:/Codex/App",
            "--debug-port",
            "9333",
            "--helper-port",
            "57322",
        ]);

        assert_eq!(options.app_dir, Some(PathBuf::from("C:/Codex/App")));
        assert_eq!(options.debug_port, 9333);
        assert_eq!(options.helper_port, 57322);
    }

    #[test]
    fn parse_launch_options_ignores_invalid_ports() {
        let options = parse_launch_options(["--debug-port", "nope", "--helper-port", "70000"]);

        assert_eq!(options.debug_port, LaunchOptions::default().debug_port);
        assert_eq!(options.helper_port, LaunchOptions::default().helper_port);
    }

    #[test]
    fn launcher_accepts_only_a_completed_provider_sync() {
        assert!(
            require_completed_provider_sync(
                &codex_plus_data::ProviderSyncStatus::Synced,
                "Provider sync complete",
            )
            .is_ok()
        );

        for status in [
            codex_plus_data::ProviderSyncStatus::Disabled,
            codex_plus_data::ProviderSyncStatus::Skipped,
        ] {
            let error = require_completed_provider_sync(&status, "target is unresolved")
                .expect_err("an incomplete provider sync must stop launch");
            assert!(error.to_string().contains("target is unresolved"));
        }
    }

    /// issue #2160：判定函数仍然把不完整同步视作失败，但启动路径必须把它降级成
    /// 一条诊断日志——.codex-global-state.json 坏掉不能把整次启动带下去。
    #[test]
    fn startup_provider_sync_failure_is_non_fatal() {
        let source = include_str!("main.rs");
        let start = source
            .find("async fn run_provider_sync(&self)")
            .expect("provider sync hook");
        let end = source[start..]
            .find("fn has_pending_remote_control_session_recoveries")
            .map(|offset| start + offset)
            .expect("next method after the hook");
        let body = &source[start..end];

        // 不再用 `?` 把同步失败变成致命错误。
        assert!(
            !body.contains("require_completed_provider_sync(&result.status, &result.message)?"),
            "provider sync 失败不得中断启动"
        );
        assert!(
            body.contains("launcher.provider_sync.degraded"),
            "降级要留下诊断日志"
        );
        // 同步失败不连坐：索引修复照常执行。
        assert!(body.contains("repair_session_index_automatically(false).await"));
        assert!(body.trim_end().ends_with("Ok(())\n    }"));
    }

    #[test]
    fn launcher_uses_single_instance_guard_before_launching() {
        let source = include_str!("main.rs");

        assert!(source.contains("acquire_single_instance_guard(options.debug_port)?"));
        assert!(source.contains("launcher_guard_port"));
        assert!(source.contains("launcher.already_running"));
        assert!(source.contains("Existing Codex instance activated"));
        assert!(source.contains("status: \"failed\".to_string()"));
    }

    #[test]
    fn stale_launcher_recovery_covers_port_and_fallback_lock_conflicts() {
        assert!(should_retry_stale_launcher_guard(
            std::io::ErrorKind::WouldBlock,
            true,
            true
        ));
        assert!(should_retry_stale_launcher_guard(
            std::io::ErrorKind::AddrInUse,
            true,
            true
        ));
        assert!(!should_retry_stale_launcher_guard(
            std::io::ErrorKind::WouldBlock,
            false,
            true
        ));
        assert!(!should_retry_stale_launcher_guard(
            std::io::ErrorKind::PermissionDenied,
            true,
            true
        ));
    }

    #[test]
    fn existing_launcher_path_drains_pending_remote_control_recovery_before_activation() {
        let source = include_str!("main.rs");
        let start = source
            .find("async fn activate_existing_codex_app")
            .expect("existing launcher activation function");
        let body = &source[start..];
        let recovery = body
            .find(
                "let has_pending_recovery = hooks.has_pending_remote_control_session_recoveries()",
            )
            .expect("pending recovery guard");
        let launch = body
            .find("let launch_result = hooks")
            .expect("Codex activation");

        assert!(recovery < launch);
        assert!(body[recovery..launch].contains("find_session_index_cleanup_blocking_processes"));
        assert!(body[recovery..launch].contains("should_finalize_pending_remote_control_recovery"));
        assert!(
            body[recovery..launch].contains("hooks.run_remote_control_session_recovery().await?")
        );
    }

    #[test]
    fn existing_launcher_path_reuses_the_primary_launcher_runtime() {
        let source = include_str!("main.rs");
        let start = source
            .find("async fn activate_existing_codex_app")
            .expect("existing launcher activation function");
        let end = source[start..]
            .find("fn should_finalize_pending_remote_control_recovery")
            .map(|offset| start + offset)
            .expect("next function after existing launcher activation");
        let body = &source[start..end];

        assert!(!body.contains("hooks.start_helper"));
        assert!(!body.contains("hooks.ensure_injection"));
        assert!(!body.contains("hooks.start_bridge_watchdog"));
        assert!(body.contains("can_connect_loopback_port(options.helper_port)"));
    }

    #[test]
    fn existing_launcher_path_runs_provider_sync_before_activation() {
        // issue #2080：快捷方式启动且 Codex 已在运行时走早退分支，过去直接返回，
        // 跳过了供应商同步与会话索引修复，只有管理工具的「重启」才生效。
        let source = include_str!("main.rs");
        let start = source
            .find("async fn activate_existing_codex_app")
            .expect("existing launcher activation function");
        let end = source[start..]
            .find("fn should_finalize_pending_remote_control_recovery")
            .map(|offset| start + offset)
            .expect("next function after existing launcher activation");
        let body = &source[start..end];

        let sync = body
            .find("run_activation_provider_sync(&hooks, &settings)")
            .expect("provider sync on the activation path");
        let launch = body
            .find("let launch_result = hooks")
            .expect("Codex activation");
        assert!(sync < launch, "provider sync must run before activation");
    }

    #[test]
    fn activation_provider_sync_respects_the_setting_and_is_non_fatal() {
        let source = include_str!("main.rs");
        let start = source
            .find("async fn run_activation_provider_sync")
            .expect("activation provider sync helper");
        let end = source[start..]
            .find("fn should_finalize_pending_remote_control_recovery")
            .map(|offset| start + offset)
            .expect("next function after the helper");
        let body = &source[start..end];

        // 只在设置启用时同步，与完整启动流程一致。
        assert!(body.contains("if !settings.provider_sync_enabled"));
        // 同步失败只记日志，不能把用户这次「激活已有窗口」的诉求一起弄失败。
        assert!(!body.contains("hooks.run_provider_sync().await?"));
        assert!(body.contains("launcher.activate_existing_provider_sync.failed"));
    }

    #[test]
    fn pending_remote_control_finalization_requires_an_idle_desktop() {
        assert!(should_finalize_pending_remote_control_recovery(true, &[]));
        assert!(!should_finalize_pending_remote_control_recovery(false, &[]));
        assert!(!should_finalize_pending_remote_control_recovery(
            true,
            &[42]
        ));
    }

    #[test]
    fn openai_session_identity_supersedes_only_its_active_pending_recovery() {
        let request = codex_plus_core::remote_control_recovery::PendingRemoteControlRecovery {
            thread_id: "mobile".to_string(),
            profile_id: "relay".to_string(),
            target_provider: "custom".to_string(),
            config_generation: "old-generation".to_string(),
            created_at: 1,
        };
        let mut settings = codex_plus_core::settings::BackendSettings {
            active_relay_id: "relay".to_string(),
            relay_profiles: vec![codex_plus_core::settings::RelayProfile {
                id: "relay".to_string(),
                config_contents: "model_provider = \"openai\"\n".to_string(),
                ..codex_plus_core::settings::RelayProfile::default()
            }],
            ..codex_plus_core::settings::BackendSettings::default()
        };

        assert!(remote_control_recovery_is_superseded_by_openai(
            &settings, &request
        ));

        settings.relay_profiles[0].config_contents = "model_provider = \"custom\"\n".to_string();
        assert!(!remote_control_recovery_is_superseded_by_openai(
            &settings, &request
        ));

        settings.relay_profiles[0].config_contents = "model_provider = \"openai\"\n".to_string();
        settings.active_relay_id = "other".to_string();
        assert!(!remote_control_recovery_is_superseded_by_openai(
            &settings, &request
        ));
    }

    #[test]
    fn launcher_hooks_forward_runtime_watchdog_and_protocol_proxy_methods() {
        let source = include_str!("main.rs");
        let compact_source = source.split_whitespace().collect::<String>();

        assert!(source.contains("async fn start_bridge_watchdog"));
        assert!(source.contains("self.watchdog_bridge_context()?"));
        assert!(source.contains("set_bridge_reinjector(reinjector)"));
        assert!(source.contains("inject_with_context(debug_port, helper_port, ctx, runtime)"));
        assert!(source.contains("async fn ensure_active_protocol_proxy_config"));
        assert!(
            compact_source
                .contains("self.core.ensure_active_protocol_proxy_config(settings).await")
        );
    }

    #[tokio::test]
    async fn watchdog_reuses_bridge_context_with_data_service() {
        let test_dir = std::env::temp_dir().join(format!(
            "codex-plus-launcher-watchdog-test-{}",
            std::process::id()
        ));
        let hooks = LauncherHooks {
            core: Arc::new(DefaultLaunchHooks::default()),
            data: Arc::new(LauncherDataService {
                db_path: test_dir.join("state.sqlite"),
                backup_dir: test_dir.join("backups"),
            }),
            runtime: Arc::new(LauncherRuntimeService::new(
                9229,
                UserScriptManager::new(
                    test_dir.join("builtin"),
                    test_dir.join("user"),
                    test_dir.join("settings.json"),
                ),
            )),
            bridge_context: Arc::new(Mutex::new(None)),
            browser_monitor: Arc::new(Mutex::new(None)),
        };

        hooks.bridge_context(9229, &test_dir).await.unwrap();
        let ctx = hooks.watchdog_bridge_context().unwrap();
        let result =
            codex_plus_core::routes::handle_bridge_request(ctx, "/backend/status", json!({})).await;

        assert_ne!(result["message"], "Unknown bridge path");
    }
}

fn builtin_user_scripts_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .map(|path| path.join("user_scripts"))
        .unwrap_or_else(|| PathBuf::from("user_scripts"))
}
