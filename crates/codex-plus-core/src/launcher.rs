use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;
use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

use crate::settings::{BackendSettings, SettingsStore, normalize_codex_extra_args};
use crate::status::{LaunchStatus, StatusStore};

const JIYI_SENSITIVE_ENV_KEYS: &[&str] = &[
    "OPENAI_API_KEY",
    "OPENAI_BASE_URL",
    "OPENAI_API_BASE_URL",
    "OPENAI_API_BASE",
    "OPENAI_API_URL",
    "CUSTOM_OPENAI_API_KEY",
    "CODEX_PLUS_OPENAI_API_KEY",
    "CODEX_PLUS_API_KEY",
    "CODEX_PLUS_OPENAI_BASE_URL",
    "CODEX_PLUS_BASE_URL",
    "DASHSCOPE_API_KEY",
    "DASHSCOPE_BASE_URL",
    "BAILIAN_API_KEY",
    "BAILIAN_BASE_URL",
    "ALIYUN_BAILIAN_API_KEY",
    "QWEN_API_KEY",
    "APIMART_API_KEY",
    "JIYI_CODEX_API_KEY",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodexLaunch {
    Process {
        command: Vec<String>,
        wait_strategy: ProcessWaitStrategy,
        macos_cleanup_policy: Option<MacosCleanupPolicy>,
    },
    PackagedActivation {
        app_user_model_id: String,
        arguments: String,
        process_id: Option<u32>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessWaitStrategy {
    TrackedChild,
    ExternalWaitCommand,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MacosCleanupPolicy {
    QuitIfNotPreviouslyRunning,
    SkipQuitBecauseAlreadyRunning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsProcessControlStrategy {
    NativeWindowsApi,
}

#[cfg(windows)]
pub fn windows_process_control_strategy() -> WindowsProcessControlStrategy {
    WindowsProcessControlStrategy::NativeWindowsApi
}

impl CodexLaunch {
    pub fn process_id(&self) -> Option<u32> {
        match self {
            Self::PackagedActivation { process_id, .. } => *process_id,
            Self::Process { .. } => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct LaunchOptions {
    pub app_dir: Option<PathBuf>,
    pub debug_port: u16,
    pub helper_port: u16,
    pub status_store: StatusStore,
}

impl Default for LaunchOptions {
    fn default() -> Self {
        Self {
            app_dir: None,
            debug_port: 9229,
            helper_port: 57321,
            status_store: StatusStore::default(),
        }
    }
}

#[derive(Clone)]
pub struct LaunchHandle {
    pub debug_port: u16,
    pub helper_port: u16,
    pub app_dir: PathBuf,
    pub launch: CodexLaunch,
    pub status_store: StatusStore,
    helper_started: bool,
    hooks: Arc<dyn LaunchHooks>,
}

impl std::fmt::Debug for LaunchHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LaunchHandle")
            .field("debug_port", &self.debug_port)
            .field("helper_port", &self.helper_port)
            .field("app_dir", &self.app_dir)
            .field("launch", &self.launch)
            .field("status_store", &self.status_store)
            .finish_non_exhaustive()
    }
}

impl LaunchHandle {
    pub async fn wait_for_codex_exit(&self) -> anyhow::Result<()> {
        let result = self.hooks.wait_for_codex_exit(&self.launch).await;
        if self.helper_started {
            self.hooks.shutdown_helper(self.helper_port).await;
        }
        result
    }
}

#[async_trait(?Send)]
pub trait LaunchHooks: Send + Sync {
    fn resolve_app_dir(
        &self,
        app_dir: Option<&Path>,
        settings: &BackendSettings,
    ) -> anyhow::Result<PathBuf>;
    fn select_debug_port(&self, requested: u16) -> u16;
    fn select_helper_port(&self, requested: u16) -> u16;
    async fn load_settings(&self) -> anyhow::Result<BackendSettings>;
    async fn run_provider_sync(&self) -> anyhow::Result<()>;
    async fn apply_active_relay_profile(&self, _settings: &BackendSettings) -> anyhow::Result<()> {
        Ok(())
    }
    async fn start_helper(&self, helper_port: u16) -> anyhow::Result<()>;
    async fn launch_codex(
        &self,
        app_dir: &Path,
        debug_port: u16,
        extra_args: &[String],
    ) -> anyhow::Result<CodexLaunch>;
    async fn bridge_context(
        &self,
        _debug_port: u16,
        _app_dir: &Path,
    ) -> anyhow::Result<Option<crate::routes::BridgeContext>> {
        Ok(None)
    }
    async fn inject(&self, debug_port: u16, helper_port: u16) -> anyhow::Result<()>;
    async fn inject_bridge(
        &self,
        debug_port: u16,
        helper_port: u16,
        _ctx: crate::routes::BridgeContext,
    ) -> anyhow::Result<()> {
        self.inject(debug_port, helper_port).await
    }
    async fn ensure_injection(&self, debug_port: u16, helper_port: u16, app_dir: &Path) -> bool {
        for attempt in 1..=120 {
            let result = match self.bridge_context(debug_port, app_dir).await {
                Ok(Some(ctx)) => self.inject_bridge(debug_port, helper_port, ctx).await,
                Ok(None) => self.inject(debug_port, helper_port).await,
                Err(error) => Err(error),
            };
            match result {
                Ok(()) => return true,
                Err(error) => {
                    let _ = crate::diagnostic_log::append_diagnostic_log(
                        "launcher.ensure_injection_retry_failed",
                        serde_json::json!({
                            "debug_port": debug_port,
                            "helper_port": helper_port,
                            "attempt": attempt,
                            "message": error.to_string()
                        }),
                    );
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
            }
        }
        false
    }
    async fn start_bridge_watchdog(
        &self,
        _debug_port: u16,
        _helper_port: u16,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    async fn write_status(&self, status: &str);
    async fn wait_for_codex_exit(&self, launch: &CodexLaunch) -> anyhow::Result<()>;
    async fn shutdown_helper(&self, helper_port: u16);
    async fn terminate_codex(&self, launch: &CodexLaunch);
}

#[derive(Default)]
pub struct DefaultLaunchHooks {
    child: Mutex<Option<Child>>,
    helper: Mutex<Option<HelperRuntime>>,
    bridge_watchdog: Mutex<Option<BridgeWatchdogRuntime>>,
}

struct HelperRuntime {
    shutdown: tokio::sync::oneshot::Sender<()>,
    task: tokio::task::JoinHandle<()>,
}

struct BridgeWatchdogRuntime {
    shutdown: tokio::sync::oneshot::Sender<()>,
    task: tokio::task::JoinHandle<()>,
}

pub async fn launch_and_inject(options: LaunchOptions) -> anyhow::Result<LaunchHandle> {
    launch_and_inject_with_hooks(options, DefaultLaunchHooks::shared()).await
}

pub async fn launch_and_inject_with_hooks<H>(
    options: LaunchOptions,
    hooks: H,
) -> anyhow::Result<LaunchHandle>
where
    H: IntoLaunchHooks,
{
    let hooks = hooks.into_launch_hooks();
    let debug_port = hooks.select_debug_port(options.debug_port);
    let mut helper_port = hooks.select_helper_port(options.helper_port);
    let settings = hooks.load_settings().await?;
    let app_dir = hooks.resolve_app_dir(options.app_dir.as_deref(), &settings)?;
    let status_store = options.status_store.clone();
    let mut helper_started = false;
    let mut launched = None;
    let mut keep_launched_on_error = false;

    let result: anyhow::Result<LaunchHandle> = async {
        if settings.provider_sync_enabled {
            hooks.run_provider_sync().await?;
        }
        let protocol_proxy_enabled = relay_protocol_proxy_enabled(&settings);
        if protocol_proxy_enabled {
            helper_port = crate::protocol_proxy::DEFAULT_PROTOCOL_PROXY_PORT;
        }
        if settings.enhancements_enabled || protocol_proxy_enabled {
            hooks.start_helper(helper_port).await?;
            helper_started = true;
        }

        let launch = hooks
            .launch_codex(&app_dir, debug_port, &settings.codex_extra_args)
            .await?;
        launched = Some(launch.clone());
        keep_launched_on_error = true;

        let mut injection_degraded = false;
        if settings.enhancements_enabled {
            let injection_ready = hooks
                .ensure_injection(debug_port, helper_port, &app_dir)
                .await;
            if injection_ready {
                keep_launched_on_error = false;
                hooks.start_bridge_watchdog(debug_port, helper_port).await?;
            } else {
                let degraded = launch_status(
                    "running_degraded",
                    "Codex 已启动，Codex++ 增强仍在等待页面就绪。",
                    debug_port,
                    helper_port,
                    &app_dir,
                );
                options.status_store.save_latest(&degraded)?;
                hooks.write_status("running_degraded").await;
                injection_degraded = true;
            }
        }

        if !settings.enhancements_enabled || !injection_degraded {
            let status = launch_status(
                "running",
                "Codex++ launcher ready",
                debug_port,
                helper_port,
                &app_dir,
            );
            options.status_store.save_latest(&status)?;
            hooks.write_status("running").await;
        }

        Ok(LaunchHandle {
            debug_port,
            helper_port,
            app_dir: app_dir.clone(),
            launch,
            status_store: status_store.clone(),
            helper_started,
            hooks: Arc::clone(&hooks),
        })
    }
    .await;

    match result {
        Ok(handle) => Ok(handle),
        Err(error) => {
            if helper_started {
                hooks.shutdown_helper(helper_port).await;
            }
            if let Some(launch) = &launched {
                if !keep_launched_on_error {
                    hooks.terminate_codex(launch).await;
                }
            }
            let message = error.to_string();
            let failure = launch_status("failed", &message, debug_port, helper_port, &app_dir);
            let _ = status_store.save_latest(&failure);
            hooks.write_status("failed").await;
            Err(error)
        }
    }
}

fn relay_protocol_proxy_enabled(settings: &BackendSettings) -> bool {
    let active = settings.active_relay_profile();
    if active.protocol == crate::settings::RelayProtocol::ChatCompletions {
        return true;
    }
    settings.jiyi_local_proxy_enabled
        && settings.relay_profiles_enabled
        && active.protocol == crate::settings::RelayProtocol::Responses
        && active.relay_mode == crate::settings::RelayMode::PureApi
        && !crate::protocol_proxy::resolved_relay_api_key(settings, &active)
            .trim()
            .is_empty()
}

pub trait IntoLaunchHooks {
    fn into_launch_hooks(self) -> Arc<dyn LaunchHooks>;
}

impl<T> IntoLaunchHooks for &T
where
    T: LaunchHooks + Clone + 'static,
{
    fn into_launch_hooks(self) -> Arc<dyn LaunchHooks> {
        Arc::new(self.clone())
    }
}

impl IntoLaunchHooks for Arc<dyn LaunchHooks> {
    fn into_launch_hooks(self) -> Arc<dyn LaunchHooks> {
        self
    }
}

impl IntoLaunchHooks for DefaultLaunchHooks {
    fn into_launch_hooks(self) -> Arc<dyn LaunchHooks> {
        Arc::new(self)
    }
}

impl DefaultLaunchHooks {
    pub fn shared() -> Arc<dyn LaunchHooks> {
        Arc::new(Self::default())
    }
}

#[async_trait(?Send)]
impl LaunchHooks for DefaultLaunchHooks {
    fn resolve_app_dir(
        &self,
        app_dir: Option<&Path>,
        settings: &BackendSettings,
    ) -> anyhow::Result<PathBuf> {
        #[cfg(target_os = "macos")]
        {
            return crate::app_paths::resolve_jiyi_codex_client_app_dir_with_saved(
                app_dir,
                Some(settings.codex_app_path.as_str()),
            )
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "未找到极义内置 Codex 客户端；为避免影响原版 Codex，不会回退 /Applications/Codex.app"
                )
            });
        }
        #[cfg(not(target_os = "macos"))]
        {
            crate::app_paths::resolve_codex_app_dir_with_saved(
                app_dir,
                Some(settings.codex_app_path.as_str()),
            )
            .ok_or_else(|| anyhow::anyhow!("Codex App directory not found"))
        }
    }

    fn select_debug_port(&self, requested: u16) -> u16 {
        crate::ports::select_platform_loopback_port(requested)
    }

    fn select_helper_port(&self, requested: u16) -> u16 {
        crate::ports::select_platform_loopback_port(requested)
    }

    async fn load_settings(&self) -> anyhow::Result<BackendSettings> {
        let mut settings = SettingsStore::default().load()?;
        hydrate_live_ccs_profiles(&mut settings);
        Ok(settings)
    }

    async fn run_provider_sync(&self) -> anyhow::Result<()> {
        anyhow::bail!("provider sync requires launcher hooks with codex-plus-data integration")
    }

    async fn apply_active_relay_profile(&self, settings: &BackendSettings) -> anyhow::Result<()> {
        if !settings.relay_profiles_enabled {
            return Ok(());
        }
        if crate::config_coordinator::effective_ownership(settings)
            == crate::settings::ConfigOwnership::CcSwitch
        {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "launcher.apply_active_relay_profile.skipped",
                serde_json::json!({
                    "reason": "ccswitch_ownership",
                    "activeRelayId": settings.active_relay_id
                }),
            );
            return Ok(());
        }
        let write_decision = crate::config_coordinator::evaluate_live_write(settings, false);
        if !write_decision.allowed {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "launcher.apply_active_relay_profile.skipped",
                serde_json::json!({
                    "reason": "write_guard",
                    "message": write_decision.message,
                    "activeRelayId": settings.active_relay_id
                }),
            );
            return Ok(());
        }
        let profile = settings.active_relay_profile();
        let home = crate::relay_config::default_codex_home_dir();
        let common_config = crate::relay_config::normalize_config_text(
            &[
                settings.relay_common_config_contents.as_str(),
                settings.relay_context_config_contents.as_str(),
            ]
            .into_iter()
            .map(str::trim)
            .filter(|section| !section.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n"),
        );
        if profile.relay_mode == crate::settings::RelayMode::Official
            && !profile.official_mix_api_key
        {
            let auth_contents = (!profile.auth_contents.trim().is_empty())
                .then_some(profile.auth_contents.as_str());
            crate::relay_config::clear_relay_config_to_home_with_auth(&home, auth_contents)?;
            return Ok(());
        }
        crate::relay_config::apply_relay_profile_to_home_with_switch_rules(
            &home,
            &profile,
            &common_config,
        )?;
        Ok(())
    }

    async fn start_helper(&self, helper_port: u16) -> anyhow::Result<()> {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", helper_port))
            .await
            .with_context(|| format!("failed to bind helper runtime on 127.0.0.1:{helper_port}"))?;
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "helper.listening",
            serde_json::json!({
                "helper_port": helper_port,
                "address": format!("http://127.0.0.1:{helper_port}")
            }),
        );
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    accepted = listener.accept() => {
                        if let Ok((stream, addr)) = accepted {
                            tokio::spawn(async move {
                                let _ = handle_helper_connection(stream, Some(addr)).await;
                            });
                        }
                    }
                }
            }
        });
        *self.helper.lock().await = Some(HelperRuntime {
            shutdown: shutdown_tx,
            task,
        });
        Ok(())
    }

    async fn launch_codex(
        &self,
        app_dir: &Path,
        debug_port: u16,
        extra_args: &[String],
    ) -> anyhow::Result<CodexLaunch> {
        let official_config_guard = capture_official_codex_config_guard();
        if cfg!(windows) {
            if let Some(activation) = build_packaged_activation(app_dir, debug_port, extra_args) {
                let CodexLaunch::PackagedActivation {
                    app_user_model_id,
                    arguments,
                    ..
                } = &activation
                else {
                    unreachable!();
                };
                let process_id = activate_packaged_app(app_user_model_id, arguments).await?;
                return Ok(match activation {
                    CodexLaunch::PackagedActivation {
                        app_user_model_id,
                        arguments,
                        ..
                    } => CodexLaunch::PackagedActivation {
                        app_user_model_id,
                        arguments,
                        process_id: Some(process_id),
                    },
                    CodexLaunch::Process { .. } => unreachable!(),
                });
            }
        }

        if app_dir.extension().and_then(|value| value.to_str()) == Some("app") {
            let cleanup_policy = if is_macos_app_running(app_dir).await {
                MacosCleanupPolicy::SkipQuitBecauseAlreadyRunning
            } else {
                MacosCleanupPolicy::QuitIfNotPreviouslyRunning
            };
            let codex_home = crate::relay_config::default_codex_home_dir();
            let unix_home = crate::paths::default_jiyi_unix_home_dir();
            let browser_user_data_dir = crate::paths::default_jiyi_browser_user_data_dir();
            std::fs::create_dir_all(&codex_home).with_context(|| {
                format!(
                    "failed to create isolated Codex home {}",
                    codex_home.display()
                )
            })?;
            std::fs::create_dir_all(&unix_home).with_context(|| {
                format!("failed to create isolated HOME {}", unix_home.display())
            })?;
            std::fs::create_dir_all(&browser_user_data_dir).with_context(|| {
                format!(
                    "failed to create isolated browser user data dir {}",
                    browser_user_data_dir.display()
                )
            })?;
            let command = build_macos_open_command_with_isolated_home(
                app_dir,
                debug_port,
                extra_args,
                &codex_home,
                &unix_home,
                &browser_user_data_dir,
            );
            let executable = command
                .first()
                .ok_or_else(|| anyhow::anyhow!("macOS open command is empty"))?;
            let child = Command::new(executable)
                .args(&command[1..])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .context("failed to launch macOS Codex app")?;
            *self.child.lock().await = Some(child);
            start_official_codex_config_guard(official_config_guard);
            return Ok(CodexLaunch::Process {
                command,
                wait_strategy: ProcessWaitStrategy::ExternalWaitCommand,
                macos_cleanup_policy: Some(cleanup_policy),
            });
        }

        let command = build_codex_command(app_dir, debug_port, extra_args);
        let executable = command
            .first()
            .ok_or_else(|| anyhow::anyhow!("Codex command is empty"))?;
        let codex_home = crate::relay_config::default_codex_home_dir();
        std::fs::create_dir_all(&codex_home).with_context(|| {
            format!(
                "failed to create isolated Codex home {}",
                codex_home.display()
            )
        })?;
        #[cfg(unix)]
        let unix_home = {
            let path = crate::paths::default_jiyi_unix_home_dir();
            std::fs::create_dir_all(&path)
                .with_context(|| format!("failed to create isolated HOME {}", path.display()))?;
            path
        };
        let mut child_command = Command::new(executable);
        child_command
            .args(&command[1..])
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(unix)]
        configure_jiyi_child_process_env(&mut child_command, &codex_home, Some(&unix_home));
        #[cfg(not(unix))]
        configure_jiyi_child_process_env(&mut child_command, &codex_home, None);
        #[cfg(windows)]
        child_command.creation_flags(crate::windows_integration::CREATE_NO_WINDOW);
        let child = child_command
            .spawn()
            .with_context(|| format!("failed to launch Codex executable {executable}"))?;
        *self.child.lock().await = Some(child);
        start_official_codex_config_guard(official_config_guard);
        Ok(CodexLaunch::Process {
            command,
            wait_strategy: ProcessWaitStrategy::TrackedChild,
            macos_cleanup_policy: None,
        })
    }

    async fn inject(&self, debug_port: u16, helper_port: u16) -> anyhow::Result<()> {
        retry_injection(debug_port, helper_port).await
    }

    async fn start_bridge_watchdog(&self, debug_port: u16, helper_port: u16) -> anyhow::Result<()> {
        let (shutdown, mut shutdown_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    _ = interval.tick() => {
                        let _ = check_and_reinject_bridge(debug_port, helper_port).await;
                    }
                }
            }
        });
        if let Some(runtime) = self
            .bridge_watchdog
            .lock()
            .await
            .replace(BridgeWatchdogRuntime { shutdown, task })
        {
            let _ = runtime.shutdown.send(());
            let _ = runtime.task.await;
        }
        Ok(())
    }

    async fn write_status(&self, _status: &str) {}

    async fn wait_for_codex_exit(&self, launch: &CodexLaunch) -> anyhow::Result<()> {
        match launch {
            CodexLaunch::Process { .. } => {
                if let Some(mut child) = self.child.lock().await.take() {
                    let _ = child.wait().await;
                }
                Ok(())
            }
            CodexLaunch::PackagedActivation { process_id, .. } => {
                if let Some(process_id) = process_id {
                    wait_for_windows_process_id(*process_id).await?;
                }
                Ok(())
            }
        }
    }

    async fn shutdown_helper(&self, _helper_port: u16) {
        if let Some(runtime) = self.bridge_watchdog.lock().await.take() {
            let _ = runtime.shutdown.send(());
            let _ = runtime.task.await;
        }
        if let Some(runtime) = self.helper.lock().await.take() {
            let _ = runtime.shutdown.send(());
            let _ = runtime.task.await;
        }
    }

    async fn terminate_codex(&self, launch: &CodexLaunch) {
        match launch {
            CodexLaunch::Process {
                wait_strategy: ProcessWaitStrategy::ExternalWaitCommand,
                command,
                macos_cleanup_policy,
            } => {
                if let Some(mut child) = self.child.lock().await.take() {
                    let _ = child.kill().await;
                }
                if let (Some(app_dir), Some(cleanup_policy)) = (
                    macos_app_dir_from_open_command(command),
                    *macos_cleanup_policy,
                ) {
                    let _ = run_macos_cleanup_command(&app_dir, cleanup_policy).await;
                }
            }
            CodexLaunch::Process { .. } => {
                if let Some(mut child) = self.child.lock().await.take() {
                    let _ = child.kill().await;
                }
            }
            CodexLaunch::PackagedActivation {
                process_id: Some(process_id),
                ..
            } => {
                let _ = terminate_windows_process_id(*process_id).await;
            }
            CodexLaunch::PackagedActivation {
                process_id: None, ..
            } => {}
        }
    }
}

fn hydrate_live_ccs_profiles(settings: &mut BackendSettings) {
    if !settings.ccs_link_enabled {
        return;
    }
    settings
        .relay_profiles
        .retain(|profile| profile.linked_ccs_provider_id.trim().is_empty());
    let _ = crate::ccs_import::sync_linked_profiles_from_default_db(&mut settings.relay_profiles);
}

async fn handle_helper_connection(
    mut stream: tokio::net::TcpStream,
    remote_addr: Option<SocketAddr>,
) -> anyhow::Result<()> {
    let request_bytes = read_http_request(&mut stream).await?;
    let request = String::from_utf8_lossy(&request_bytes);
    let request_line = request.lines().next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let path = parts.next().unwrap_or_default();
    let path_only = path.split_once('?').map(|(value, _)| value).unwrap_or(path);
    let request_body = http_request_body(&request);
    let remote_addr_text = remote_addr.map(|addr| addr.to_string());

    let _ = crate::diagnostic_log::append_diagnostic_log(
        "helper.request",
        serde_json::json!({
            "method": method,
            "path": path,
            "request_line": request_line,
            "remote_addr": remote_addr_text,
            "body_bytes": request_body.len()
        }),
    );

    if crate::protocol_proxy::is_responses_proxy_path(path) && method == "POST" {
        return handle_protocol_proxy_connection(
            &mut stream,
            request_body,
            method,
            path,
            remote_addr_text,
        )
        .await;
    }
    if crate::protocol_proxy::is_chat_completions_proxy_path(path) && method == "POST" {
        return handle_chat_completions_proxy_connection(
            &mut stream,
            request_body,
            method,
            path,
            remote_addr_text,
        )
        .await;
    }
    if crate::protocol_proxy::is_models_proxy_path(path) && matches!(method, "GET" | "OPTIONS") {
        return handle_models_proxy_connection(&mut stream, method, path, remote_addr_text).await;
    }
    if is_local_backend_api_path(path_only) && matches!(method, "GET" | "POST" | "OPTIONS") {
        return handle_local_backend_api_connection(
            &mut stream,
            method,
            path_only,
            &request,
            request_body,
            remote_addr_text,
        )
        .await;
    }

    let (status, body, content_type, log_event) =
        if matches!(path, "/backend/status" | "/backend/repair")
            && matches!(method, "GET" | "POST" | "OPTIONS")
        {
            (
                "200 OK".to_string(),
                serde_json::to_vec(&serde_json::json!({
                    "status": "ok",
                    "message": "后端已连接",
                    "version": crate::version::VERSION,
                    "transport": "http-helper"
                }))?,
                "application/json; charset=utf-8".to_string(),
                if path == "/backend/status" {
                    "helper.backend_status_ok"
                } else {
                    "helper.backend_repair_ok"
                },
            )
        } else if path == "/diagnostics/log" && matches!(method, "POST" | "OPTIONS") {
            if method == "POST" {
                let detail = serde_json::from_str::<serde_json::Value>(request_body)
                    .unwrap_or_else(|error| {
                        serde_json::json!({
                            "parse_error": error.to_string(),
                            "raw": request_body
                        })
                    });
                let event = detail
                    .get("event")
                    .and_then(serde_json::Value::as_str)
                    .map(sanitize_diagnostic_event)
                    .unwrap_or_else(|| "event".to_string());
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    &format!("renderer.{event}"),
                    detail,
                );
            }
            (
                "200 OK".to_string(),
                serde_json::to_vec(&serde_json::json!({
                    "status": "ok",
                    "message": "日志已记录"
                }))?,
                "application/json; charset=utf-8".to_string(),
                "helper.diagnostics_log_ok",
            )
        } else {
            (
                "404 Not Found".to_string(),
                serde_json::to_vec(&serde_json::json!({
                    "status": "failed",
                    "message": "未知后端路径"
                }))?,
                "application/json; charset=utf-8".to_string(),
                "helper.unknown_path",
            )
        };
    let _ = crate::diagnostic_log::append_diagnostic_log(
        log_event,
        serde_json::json!({
            "method": method,
            "path": path,
            "status": status,
            "remote_addr": remote_addr_text
        }),
    );
    let response = if method == "OPTIONS" {
        format!(
            "HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type, Authorization\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
    } else {
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type, Authorization\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
    };
    stream.write_all(response.as_bytes()).await?;
    if method != "OPTIONS" {
        stream.write_all(&body).await?;
    }
    stream.shutdown().await?;
    Ok(())
}

fn is_local_backend_api_path(path: &str) -> bool {
    matches!(
        path,
        "/jiyi/v1/health"
            | "/jiyi/v1/sessions/verify"
            | "/jiyi/v1/sessions/revoke"
            | "/jiyi/v1/me"
            | "/jiyi/v1/quota/today"
            | "/jiyi/v1/usage/record"
    )
}

async fn handle_local_backend_api_connection(
    stream: &mut tokio::net::TcpStream,
    method: &str,
    path: &str,
    request: &str,
    request_body: &str,
    remote_addr_text: Option<String>,
) -> anyhow::Result<()> {
    if method == "OPTIONS" {
        write_http_response(
            stream,
            "204 No Content",
            "application/json; charset=utf-8",
            &[],
        )
        .await?;
        stream.shutdown().await?;
        return Ok(());
    }

    let store = crate::local_backend::LocalBackendStore::default();
    let (status, body, log_event) = match path {
        "/jiyi/v1/health" => match store.state() {
            Ok(state) => (
                "200 OK",
                serde_json::to_vec(&serde_json::json!({
                    "status": "ok",
                    "message": "极义本地账号后端已连接",
                    "version": crate::version::VERSION,
                    "transport": "http-helper",
                    "backend": state
                }))?,
                "helper.local_backend_health_ok",
            ),
            Err(error) => (
                "500 Internal Server Error",
                serde_json::to_vec(&serde_json::json!({
                    "status": "failed",
                    "message": error.to_string()
                }))?,
                "helper.local_backend_health_failed",
            ),
        },
        "/jiyi/v1/sessions/verify" => {
            let access_token = local_backend_access_token(request, request_body);
            let verification = store.verify_session_token(&access_token)?;
            (
                "200 OK",
                serde_json::to_vec(&serde_json::json!({
                    "status": "ok",
                    "authenticated": verification.authenticated,
                    "reason": verification.reason,
                    "subject": verification.subject
                }))?,
                if verification.authenticated {
                    "helper.local_backend_session_verify_ok"
                } else {
                    "helper.local_backend_session_verify_rejected"
                },
            )
        }
        "/jiyi/v1/sessions/revoke" => {
            if method != "POST" {
                (
                    "405 Method Not Allowed",
                    serde_json::to_vec(&serde_json::json!({
                        "status": "failed",
                        "message": "session 吊销必须使用 POST"
                    }))?,
                    "helper.local_backend_session_revoke_method_not_allowed",
                )
            } else {
                let access_token = local_backend_access_token(request, request_body);
                let revocation = store.revoke_session_token(&access_token)?;
                if revocation.authenticated {
                    (
                        "200 OK",
                        serde_json::to_vec(&serde_json::json!({
                            "status": "ok",
                            "authenticated": true,
                            "subject": revocation.subject,
                            "revokedAtMs": revocation.revoked_at_ms
                        }))?,
                        "helper.local_backend_session_revoke_ok",
                    )
                } else {
                    (
                        "401 Unauthorized",
                        serde_json::to_vec(&serde_json::json!({
                            "status": "failed",
                            "authenticated": false,
                            "reason": revocation.reason,
                            "message": "本地账号服务端 session 无效或已过期"
                        }))?,
                        "helper.local_backend_session_revoke_unauthorized",
                    )
                }
            }
        }
        "/jiyi/v1/me" => {
            let access_token = local_backend_access_token(request, request_body);
            let verification = store.verify_session_token(&access_token)?;
            if verification.authenticated {
                (
                    "200 OK",
                    serde_json::to_vec(&serde_json::json!({
                        "status": "ok",
                        "authenticated": true,
                        "subject": verification.subject
                    }))?,
                    "helper.local_backend_me_ok",
                )
            } else {
                (
                    "401 Unauthorized",
                    serde_json::to_vec(&serde_json::json!({
                        "status": "failed",
                        "authenticated": false,
                        "reason": verification.reason,
                        "message": "本地账号服务端 session 无效或已过期"
                    }))?,
                    "helper.local_backend_me_unauthorized",
                )
            }
        }
        "/jiyi/v1/quota/today" => {
            let access_token = local_backend_access_token(request, request_body);
            let snapshot = store.quota_snapshot(&access_token)?;
            if snapshot.authenticated {
                (
                    "200 OK",
                    serde_json::to_vec(&serde_json::json!({
                        "status": "ok",
                        "authenticated": true,
                        "subject": snapshot.subject,
                        "quota": snapshot.quota
                    }))?,
                    "helper.local_backend_quota_today_ok",
                )
            } else {
                (
                    "401 Unauthorized",
                    serde_json::to_vec(&serde_json::json!({
                        "status": "failed",
                        "authenticated": false,
                        "reason": snapshot.reason,
                        "message": "本地账号服务端 session 无效或已过期"
                    }))?,
                    "helper.local_backend_quota_today_unauthorized",
                )
            }
        }
        "/jiyi/v1/usage/record" => {
            if method != "POST" {
                (
                    "405 Method Not Allowed",
                    serde_json::to_vec(&serde_json::json!({
                        "status": "failed",
                        "message": "服务端用量写入必须使用 POST"
                    }))?,
                    "helper.local_backend_usage_record_method_not_allowed",
                )
            } else {
                let access_token = local_backend_access_token(request, request_body);
                match local_backend_usage_event_from_body(request_body) {
                    Ok(event) => {
                        let receipt = store.record_usage_event(&access_token, &event)?;
                        if receipt.authenticated {
                            (
                                "200 OK",
                                serde_json::to_vec(&serde_json::json!({
                                    "status": "ok",
                                    "authenticated": true,
                                    "subject": receipt.subject,
                                    "day": receipt.day,
                                    "recordedTokens": receipt.recorded_tokens,
                                    "totalUsedTokens": receipt.total_used_tokens,
                                    "totalRequestCount": receipt.total_request_count
                                }))?,
                                "helper.local_backend_usage_record_ok",
                            )
                        } else {
                            (
                                "401 Unauthorized",
                                serde_json::to_vec(&serde_json::json!({
                                    "status": "failed",
                                    "authenticated": false,
                                    "reason": receipt.reason,
                                    "message": "本地账号服务端 session 无效或已过期"
                                }))?,
                                "helper.local_backend_usage_record_unauthorized",
                            )
                        }
                    }
                    Err(error) => (
                        "400 Bad Request",
                        serde_json::to_vec(&serde_json::json!({
                            "status": "failed",
                            "message": error.to_string()
                        }))?,
                        "helper.local_backend_usage_record_bad_request",
                    ),
                }
            }
        }
        _ => unreachable!("local backend API path was prefiltered"),
    };

    write_http_response(stream, status, "application/json; charset=utf-8", &body).await?;
    log_helper_response(log_event, method, path, status, remote_addr_text);
    stream.shutdown().await?;
    Ok(())
}

fn local_backend_access_token(request: &str, request_body: &str) -> String {
    let bearer =
        http_header_value(request, "authorization").and_then(|value| bearer_token(value.as_str()));
    if let Some(token) = bearer {
        return token;
    }

    serde_json::from_str::<serde_json::Value>(request_body)
        .ok()
        .and_then(|value| {
            ["accessToken", "access_token", "token"]
                .into_iter()
                .find_map(|key| value.get(key).and_then(serde_json::Value::as_str))
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
        })
        .unwrap_or_default()
}

fn local_backend_usage_event_from_body(
    request_body: &str,
) -> anyhow::Result<crate::local_usage::LocalUsageEvent> {
    let value = serde_json::from_str::<serde_json::Value>(request_body)?;
    let method = json_string_field(&value, &["method"]).unwrap_or_else(|| "POST".to_string());
    let path = json_string_field(&value, &["path"]).unwrap_or_else(|| "/v1/responses".to_string());
    let upstream_protocol = json_string_field(&value, &["upstreamProtocol", "upstream_protocol"])
        .unwrap_or_else(|| "responses".to_string());
    let status_code = json_i64_field(&value, &["statusCode", "status_code"])
        .unwrap_or(200)
        .clamp(100, 599) as u16;
    let request_bytes = json_i64_field(&value, &["requestBytes", "request_bytes"])
        .unwrap_or(0)
        .max(0) as usize;
    let response_bytes = json_i64_field(&value, &["responseBytes", "response_bytes"])
        .unwrap_or(0)
        .max(0) as usize;
    let token_usage = local_backend_token_usage_from_body(&value);
    if request_bytes == 0 && response_bytes == 0 && token_usage.is_none() {
        anyhow::bail!("用量写入缺少 requestBytes、responseBytes 或 tokenUsage。");
    }
    Ok(crate::local_usage::LocalUsageEvent {
        method,
        path,
        upstream_protocol,
        status_code,
        request_bytes,
        response_bytes,
        token_usage,
    })
}

fn local_backend_token_usage_from_body(
    value: &serde_json::Value,
) -> Option<crate::local_usage::TokenUsage> {
    if let Some(usage) = value.get("usage").filter(|usage| usage.is_object()) {
        let wrapped = serde_json::json!({ "usage": usage });
        if let Some(parsed) = crate::local_usage::token_usage_from_value(&wrapped) {
            return Some(parsed);
        }
    }
    let usage = value
        .get("tokenUsage")
        .or_else(|| value.get("token_usage"))
        .filter(|usage| usage.is_object())?;
    let input_tokens = json_i64_field(usage, &["inputTokens", "input_tokens", "prompt_tokens"]);
    let output_tokens = json_i64_field(
        usage,
        &["outputTokens", "output_tokens", "completion_tokens"],
    );
    let total_tokens = json_i64_field(usage, &["totalTokens", "total_tokens"]).or_else(|| {
        input_tokens
            .zip(output_tokens)
            .map(|(input, output)| input + output)
    });
    (input_tokens.is_some() || output_tokens.is_some() || total_tokens.is_some()).then_some(
        crate::local_usage::TokenUsage {
            input_tokens,
            output_tokens,
            total_tokens,
        },
    )
}

fn json_string_field(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(serde_json::Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn json_i64_field(value: &serde_json::Value, keys: &[&str]) -> Option<i64> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(serde_json::Value::as_i64)
            .or_else(|| {
                value
                    .get(*key)
                    .and_then(serde_json::Value::as_u64)
                    .and_then(|value| i64::try_from(value).ok())
            })
    })
}

fn http_header_value(request: &str, header_name: &str) -> Option<String> {
    let expected = header_name.to_ascii_lowercase();
    request
        .lines()
        .skip(1)
        .take_while(|line| !line.trim().is_empty())
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            (name.trim().eq_ignore_ascii_case(&expected)).then(|| value.trim().to_string())
        })
}

fn bearer_token(value: &str) -> Option<String> {
    let (scheme, token) = value.trim().split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("bearer")
        .then(|| token.trim().to_string())
        .filter(|token| !token.is_empty())
}

async fn handle_models_proxy_connection(
    stream: &mut tokio::net::TcpStream,
    method: &str,
    path: &str,
    remote_addr_text: Option<String>,
) -> anyhow::Result<()> {
    if method == "OPTIONS" {
        write_http_response(
            stream,
            "204 No Content",
            "application/json; charset=utf-8",
            &[],
        )
        .await?;
        stream.shutdown().await?;
        return Ok(());
    }

    let upstream = match crate::protocol_proxy::open_models_proxy_request().await {
        Ok(upstream) => upstream,
        Err(error) => {
            let body = serde_json::to_vec(&serde_json::json!({
                "status": "failed",
                "message": error.to_string()
            }))?;
            write_http_response(
                stream,
                "502 Bad Gateway",
                "application/json; charset=utf-8",
                &body,
            )
            .await?;
            log_helper_response(
                "helper.models_proxy_failed",
                method,
                path,
                "502 Bad Gateway",
                remote_addr_text,
            );
            stream.shutdown().await?;
            return Ok(());
        }
    };

    let status = upstream.status();
    let is_success = upstream.is_success();
    let content_type = if upstream.content_type.is_empty() {
        "application/json; charset=utf-8".to_string()
    } else {
        upstream.content_type.clone()
    };
    let body = upstream.response.bytes().await?.to_vec();
    write_http_response(stream, &status, &content_type, &body).await?;
    log_helper_response(
        if is_success {
            "helper.models_proxy_ok"
        } else {
            "helper.models_proxy_upstream_error"
        },
        method,
        path,
        &status,
        remote_addr_text,
    );
    stream.shutdown().await?;
    Ok(())
}

async fn handle_protocol_proxy_connection(
    stream: &mut tokio::net::TcpStream,
    request_body: &str,
    method: &str,
    path: &str,
    remote_addr_text: Option<String>,
) -> anyhow::Result<()> {
    let request_json = serde_json::from_str::<serde_json::Value>(request_body).ok();
    let usage_store = crate::local_usage::LocalUsageStore::default();
    let usage_policy = crate::local_usage::LocalUsagePolicy::from_settings(
        &SettingsStore::default().load().unwrap_or_default(),
    );
    if let Err(error) = usage_store.preflight_request(usage_policy, request_body.len()) {
        let body = serde_json::to_vec(&serde_json::json!({
            "error": {
                "message": error.to_string(),
                "type": "jiyi_quota_exceeded"
            }
        }))?;
        write_http_response(
            stream,
            "429 Too Many Requests",
            "application/json; charset=utf-8",
            &body,
        )
        .await?;
        log_helper_response(
            "helper.local_quota_exceeded",
            method,
            path,
            "429 Too Many Requests",
            remote_addr_text,
        );
        stream.shutdown().await?;
        return Ok(());
    }

    let upstream = match crate::protocol_proxy::open_responses_proxy_request(request_body).await {
        Ok(upstream) => upstream,
        Err(error) => {
            let body = serde_json::to_vec(&serde_json::json!({
                "status": "failed",
                "message": error.to_string()
            }))?;
            write_http_response(
                stream,
                "502 Bad Gateway",
                "application/json; charset=utf-8",
                &body,
            )
            .await?;
            log_helper_response(
                "helper.protocol_proxy_failed",
                method,
                path,
                "502 Bad Gateway",
                remote_addr_text,
            );
            stream.shutdown().await?;
            return Ok(());
        }
    };

    if upstream.response_kind == crate::protocol_proxy::UpstreamResponseKind::Responses {
        let status = upstream.status();
        let status_code = upstream.status_code;
        let is_success = upstream.is_success();
        let content_type = if upstream.content_type.trim().is_empty() {
            "application/json; charset=utf-8".to_string()
        } else {
            upstream.content_type.clone()
        };
        if upstream.is_stream && is_success {
            write_http_stream_headers(stream, &status, &content_type).await?;
            let mut bytes_stream = upstream.response.bytes_stream();
            let mut response_bytes = 0usize;
            while let Some(chunk) = bytes_stream.next().await {
                let bytes = chunk?;
                response_bytes = response_bytes.saturating_add(bytes.len());
                stream.write_all(&bytes).await?;
            }
            record_local_usage_event(
                method,
                path,
                "responses",
                status_code,
                request_body.len(),
                response_bytes,
                None,
            );
            log_helper_response(
                "helper.responses_proxy_stream_ok",
                method,
                path,
                &status,
                remote_addr_text,
            );
            stream.shutdown().await?;
            return Ok(());
        }

        let body = upstream.response.bytes().await?.to_vec();
        let token_usage = serde_json::from_slice::<serde_json::Value>(&body)
            .ok()
            .and_then(|value| crate::local_usage::token_usage_from_value(&value));
        record_local_usage_event(
            method,
            path,
            "responses",
            status_code,
            request_body.len(),
            body.len(),
            token_usage,
        );
        write_http_response(stream, &status, &content_type, &body).await?;
        log_helper_response(
            if is_success {
                "helper.responses_proxy_ok"
            } else {
                "helper.responses_proxy_upstream_error"
            },
            method,
            path,
            &status,
            remote_addr_text,
        );
        stream.shutdown().await?;
        return Ok(());
    }

    if !upstream.is_success() {
        let status = upstream.status();
        let status_code = upstream.status_code;
        let upstream_content_type = upstream.content_type.clone();
        let upstream_body = upstream.response.bytes().await?.to_vec();
        record_local_usage_event(
            method,
            path,
            "chatCompletions",
            status_code,
            request_body.len(),
            upstream_body.len(),
            None,
        );
        let error = crate::protocol_proxy::responses_error_from_upstream(
            upstream.status_code,
            &upstream_content_type,
            &upstream_body,
        );
        let body = serde_json::to_vec(&error)?;
        write_http_response(stream, &status, "application/json; charset=utf-8", &body).await?;
        log_helper_response(
            "helper.protocol_proxy_upstream_error",
            method,
            path,
            &status,
            remote_addr_text,
        );
        stream.shutdown().await?;
        return Ok(());
    }

    if upstream.is_stream {
        write_http_stream_headers(stream, "200 OK", "text/event-stream; charset=utf-8").await?;
        let mut converter = request_json
            .as_ref()
            .map(crate::protocol_proxy::ChatSseToResponsesConverter::with_request)
            .unwrap_or_default();
        let mut bytes_stream = upstream.response.bytes_stream();
        let mut stream_failed = false;
        let mut response_bytes = 0usize;

        while let Some(chunk) = bytes_stream.next().await {
            match chunk {
                Ok(bytes) => {
                    response_bytes = response_bytes.saturating_add(bytes.len());
                    let converted = converter.push_bytes(&bytes);
                    if !converted.is_empty() {
                        stream.write_all(&converted).await?;
                    }
                }
                Err(error) => {
                    let failed = converter.fail(
                        format!("Stream error: {error}"),
                        Some("stream_error".to_string()),
                    );
                    if !failed.is_empty() {
                        stream.write_all(&failed).await?;
                    }
                    stream_failed = true;
                    break;
                }
            }
        }

        if !stream_failed {
            let tail = converter.finish();
            if !tail.is_empty() {
                stream.write_all(&tail).await?;
            }
        }
        record_local_usage_event(
            method,
            path,
            "chatCompletions",
            200,
            request_body.len(),
            response_bytes,
            None,
        );
        log_helper_response(
            "helper.protocol_proxy_stream_ok",
            method,
            path,
            "200 OK",
            remote_addr_text,
        );
        stream.shutdown().await?;
        return Ok(());
    }

    let upstream_body = upstream.response.bytes().await?;
    let chat_json: serde_json::Value = serde_json::from_slice(&upstream_body)?;
    let token_usage = crate::local_usage::token_usage_from_value(&chat_json);
    let response_json = if let Some(request_json) = request_json.as_ref() {
        crate::protocol_proxy::chat_completion_to_response_with_request(chat_json, request_json)?
    } else {
        crate::protocol_proxy::chat_completion_to_response(chat_json)?
    };
    let body = serde_json::to_vec(&response_json)?;
    record_local_usage_event(
        method,
        path,
        "chatCompletions",
        200,
        request_body.len(),
        upstream_body.len(),
        token_usage,
    );
    write_http_response(stream, "200 OK", "application/json; charset=utf-8", &body).await?;
    log_helper_response(
        "helper.protocol_proxy_ok",
        method,
        path,
        "200 OK",
        remote_addr_text,
    );
    stream.shutdown().await?;
    Ok(())
}

async fn handle_chat_completions_proxy_connection(
    stream: &mut tokio::net::TcpStream,
    request_body: &str,
    method: &str,
    path: &str,
    remote_addr_text: Option<String>,
) -> anyhow::Result<()> {
    let upstream =
        match crate::protocol_proxy::open_chat_completions_proxy_request(request_body).await {
            Ok(upstream) => upstream,
            Err(error) => {
                let body = serde_json::to_vec(&serde_json::json!({
                    "status": "failed",
                    "message": error.to_string()
                }))?;
                write_http_response(
                    stream,
                    "502 Bad Gateway",
                    "application/json; charset=utf-8",
                    &body,
                )
                .await?;
                log_helper_response(
                    "helper.chat_completions_proxy_failed",
                    method,
                    path,
                    "502 Bad Gateway",
                    remote_addr_text,
                );
                stream.shutdown().await?;
                return Ok(());
            }
        };

    let status = upstream.status();
    let is_success = upstream.is_success();
    let content_type = if upstream.content_type.is_empty() {
        "application/json; charset=utf-8".to_string()
    } else {
        upstream.content_type.clone()
    };

    if upstream.is_stream && is_success {
        write_http_stream_headers(stream, &status, &content_type).await?;
        let mut bytes_stream = upstream.response.bytes_stream();
        while let Some(chunk) = bytes_stream.next().await {
            stream.write_all(&chunk?).await?;
        }
        log_helper_response(
            "helper.chat_completions_proxy_stream_ok",
            method,
            path,
            &status,
            remote_addr_text,
        );
        stream.shutdown().await?;
        return Ok(());
    }

    let body = upstream.response.bytes().await?.to_vec();
    write_http_response(stream, &status, &content_type, &body).await?;
    log_helper_response(
        if is_success {
            "helper.chat_completions_proxy_ok"
        } else {
            "helper.chat_completions_proxy_upstream_error"
        },
        method,
        path,
        &status,
        remote_addr_text,
    );
    stream.shutdown().await?;
    Ok(())
}

async fn write_http_response(
    stream: &mut tokio::net::TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
) -> anyhow::Result<()> {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type, Authorization\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    stream.write_all(body).await?;
    Ok(())
}

async fn write_http_stream_headers(
    stream: &mut tokio::net::TcpStream,
    status: &str,
    content_type: &str,
) -> anyhow::Result<()> {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nCache-Control: no-cache\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type, Authorization\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(response.as_bytes()).await?;
    Ok(())
}

fn log_helper_response(
    event: &str,
    method: &str,
    path: &str,
    status: &str,
    remote_addr_text: Option<String>,
) {
    let _ = crate::diagnostic_log::append_diagnostic_log(
        event,
        serde_json::json!({
            "method": method,
            "path": path,
            "status": status,
            "remote_addr": remote_addr_text
        }),
    );
}

fn record_local_usage_event(
    method: &str,
    path: &str,
    upstream_protocol: &str,
    status_code: u16,
    request_bytes: usize,
    response_bytes: usize,
    token_usage: Option<crate::local_usage::TokenUsage>,
) {
    let event = crate::local_usage::LocalUsageEvent {
        method: method.to_string(),
        path: path.to_string(),
        upstream_protocol: upstream_protocol.to_string(),
        status_code,
        request_bytes,
        response_bytes,
        token_usage,
    };
    let backend_event = event.clone();
    if let Err(error) = crate::local_usage::LocalUsageStore::default().record_event(event) {
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "helper.local_usage_record_failed",
            serde_json::json!({
                "method": method,
                "path": path,
                "status_code": status_code,
                "error": error.to_string()
            }),
        );
    }
    record_local_backend_usage_event(&backend_event);
}

fn record_local_backend_usage_event(event: &crate::local_usage::LocalUsageEvent) {
    let token = crate::secret_store::resolve_local_backend_session_token();
    if token.trim().is_empty() {
        return;
    }
    match crate::local_backend::LocalBackendStore::default().record_usage_event(&token, event) {
        Ok(receipt) if receipt.authenticated => {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "helper.local_backend_usage_record_ok",
                serde_json::json!({
                    "method": event.method,
                    "path": event.path,
                    "status_code": event.status_code,
                    "day": receipt.day,
                    "recorded_tokens": receipt.recorded_tokens,
                    "total_used_tokens": receipt.total_used_tokens,
                    "total_request_count": receipt.total_request_count
                }),
            );
        }
        Ok(receipt) => {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "helper.local_backend_usage_record_rejected",
                serde_json::json!({
                    "method": event.method,
                    "path": event.path,
                    "status_code": event.status_code,
                    "reason": receipt.reason
                }),
            );
        }
        Err(error) => {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "helper.local_backend_usage_record_failed",
                serde_json::json!({
                    "method": event.method,
                    "path": event.path,
                    "status_code": event.status_code,
                    "error": error.to_string()
                }),
            );
        }
    }
}

async fn read_http_request(stream: &mut tokio::net::TcpStream) -> anyhow::Result<Vec<u8>> {
    let mut buffer = Vec::new();
    let mut chunk = vec![0_u8; 4096];
    let mut header_end = None;
    let mut content_length = 0_usize;

    loop {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if header_end.is_none() {
            header_end = find_header_end(&buffer);
            if let Some(end) = header_end {
                content_length = content_length_from_headers(&buffer[..end]).unwrap_or(0);
            }
        }
        if let Some(end) = header_end {
            if buffer.len() >= end + 4 + content_length {
                break;
            }
        }
        if buffer.len() > 32 * 1024 * 1024 {
            anyhow::bail!("HTTP 请求过大");
        }
    }

    Ok(buffer)
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn content_length_from_headers(headers: &[u8]) -> Option<usize> {
    let text = String::from_utf8_lossy(headers);
    text.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if name.trim().eq_ignore_ascii_case("content-length") {
            value.trim().parse().ok()
        } else {
            None
        }
    })
}

fn http_request_body(request: &str) -> &str {
    request
        .split_once("\r\n\r\n")
        .map(|(_, body)| body)
        .unwrap_or_default()
}

fn sanitize_diagnostic_event(event: &str) -> String {
    let sanitized = event
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if sanitized.is_empty() {
        "event".to_string()
    } else {
        sanitized
    }
}

pub fn build_codex_arguments(debug_port: u16, extra_args: &[String]) -> Vec<String> {
    let mut args = vec![
        format!("--remote-debugging-port={debug_port}"),
        format!("--remote-allow-origins=http://127.0.0.1:{debug_port}"),
    ];
    args.extend(normalize_codex_extra_args(extra_args));
    args
}

pub fn build_codex_command(app_dir: &Path, debug_port: u16, extra_args: &[String]) -> Vec<String> {
    let mut command = vec![
        crate::app_paths::build_codex_executable(app_dir)
            .to_string_lossy()
            .to_string(),
    ];
    command.extend(build_codex_arguments(debug_port, extra_args));
    command
}

pub fn build_packaged_activation(
    app_dir: &Path,
    debug_port: u16,
    extra_args: &[String],
) -> Option<CodexLaunch> {
    Some(CodexLaunch::PackagedActivation {
        app_user_model_id: crate::app_paths::packaged_app_user_model_id(app_dir)?,
        arguments: command_line_arguments(&build_codex_arguments(debug_port, extra_args)),
        process_id: None,
    })
}

async fn retry_injection(debug_port: u16, helper_port: u16) -> anyhow::Result<()> {
    let mut last_error = None;
    for _ in 0..20 {
        match try_inject(debug_port, helper_port).await {
            Ok(()) => return Ok(()),
            Err(error) => {
                last_error = Some(error);
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("Codex injection failed")))
}

pub async fn check_and_reinject_bridge(debug_port: u16, helper_port: u16) -> bool {
    let healthy = match bridge_health_ok(debug_port).await {
        Ok(healthy) => healthy,
        Err(error) => {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "bridge.health_check_failed",
                serde_json::json!({
                    "debug_port": debug_port,
                    "helper_port": helper_port,
                    "message": error.to_string()
                }),
            );
            false
        }
    };
    if healthy {
        return false;
    }

    let _ = crate::diagnostic_log::append_diagnostic_log(
        "bridge.reinject_start",
        serde_json::json!({
            "debug_port": debug_port,
            "helper_port": helper_port
        }),
    );
    match retry_injection(debug_port, helper_port).await {
        Ok(()) => {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "bridge.reinject_ok",
                serde_json::json!({
                    "debug_port": debug_port,
                    "helper_port": helper_port
                }),
            );
            true
        }
        Err(error) => {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "bridge.reinject_failed",
                serde_json::json!({
                    "debug_port": debug_port,
                    "helper_port": helper_port,
                    "message": error.to_string()
                }),
            );
            false
        }
    }
}

async fn bridge_health_ok(debug_port: u16) -> anyhow::Result<bool> {
    let targets = crate::cdp::list_targets(debug_port).await?;
    let target = crate::cdp::pick_page_target(&targets)?;
    let websocket_url = target
        .web_socket_debugger_url
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("selected CDP target has no websocket URL"))?;
    let result = crate::bridge::evaluate_script_with_await_promise(
        websocket_url,
        crate::bridge::bridge_health_check_script(),
        true,
    )
    .await?;
    Ok(runtime_evaluate_result_is_true(&result))
}

fn runtime_evaluate_result_is_true(result: &Value) -> bool {
    result
        .get("result")
        .and_then(|result| result.get("result"))
        .and_then(|result| result.get("value"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

async fn try_inject(debug_port: u16, helper_port: u16) -> anyhow::Result<()> {
    let targets = crate::cdp::list_targets(debug_port).await?;
    let target = crate::cdp::pick_page_target(&targets)?;
    let websocket_url = target
        .web_socket_debugger_url
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("selected CDP target has no websocket URL"))?;
    let script = crate::assets::injection_script(helper_port);
    let ctx = crate::routes::BridgeContext::core(Arc::new(crate::routes::CoreRuntimeService::new(
        debug_port,
        StatusStore::default(),
    )));
    crate::bridge::install_bridge(
        websocket_url,
        crate::bridge::BRIDGE_BINDING_NAME,
        Arc::new(move |path, payload| {
            let ctx = ctx.clone();
            Box::pin(
                async move { Ok(crate::routes::handle_bridge_request(ctx, &path, payload).await) },
            )
        }),
        &[script],
    )
    .await
}

#[derive(Debug, Clone)]
struct OfficialCodexConfigGuard {
    config_path: PathBuf,
    auth_path: PathBuf,
    config_snapshot: Option<Vec<u8>>,
    auth_snapshot: Option<Vec<u8>>,
}

fn capture_official_codex_config_guard() -> OfficialCodexConfigGuard {
    let home = crate::paths::default_official_codex_home_dir();
    let config_path = home.join("config.toml");
    let auth_path = home.join("auth.json");
    OfficialCodexConfigGuard {
        config_snapshot: std::fs::read(&config_path).ok(),
        auth_snapshot: std::fs::read(&auth_path).ok(),
        config_path,
        auth_path,
    }
}

fn start_official_codex_config_guard(guard: OfficialCodexConfigGuard) {
    std::thread::spawn(move || {
        for _ in 0..80 {
            std::thread::sleep(std::time::Duration::from_millis(250));
            if let Err(error) = restore_official_codex_config_if_contaminated(&guard) {
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "official_codex_config_guard.restore_failed",
                    serde_json::json!({ "error": error.to_string() }),
                );
            }
        }
    });
}

pub fn start_official_codex_config_guard_for_startup() {
    start_official_codex_config_guard(capture_official_codex_config_guard());
}

fn restore_official_codex_config_if_contaminated(
    guard: &OfficialCodexConfigGuard,
) -> anyhow::Result<()> {
    restore_guarded_file_if_contaminated(&guard.config_path, guard.config_snapshot.as_deref())?;
    restore_guarded_file_if_contaminated(&guard.auth_path, guard.auth_snapshot.as_deref())?;
    Ok(())
}

fn restore_guarded_file_if_contaminated(
    path: &Path,
    snapshot: Option<&[u8]>,
) -> anyhow::Result<()> {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return Ok(());
    };
    if !official_codex_config_is_jiyi_contaminated(&contents) {
        return Ok(());
    }

    if let Some(snapshot) = snapshot {
        std::fs::write(path, snapshot)
            .with_context(|| format!("failed to restore guarded file {}", path.display()))?;
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "official_codex_config_guard.restored",
            serde_json::json!({ "path": path.to_string_lossy() }),
        );
    }
    Ok(())
}

fn official_codex_config_is_jiyi_contaminated(contents: &str) -> bool {
    contents.contains(".codex-session-delete")
        || contents.contains("JiyiCodex")
        || contents.contains("Jiyi")
        || contents.contains("极义codex")
        || contents.contains("dashscope.aliyuncs.com")
        || contents.contains("aliyuncs.com/compatible-mode")
        || contents.contains("DASHSCOPE_API_KEY")
        || contents.contains("BAILIAN_API_KEY")
        || contents.contains("ALIYUN_BAILIAN_API_KEY")
        || contents.contains("QWEN_API_KEY")
        || contents.contains("apimart.ai")
        || contents.contains("api.apimart.ai")
        || contents.contains("qwen3.7-plus")
        || contents.contains("gpt-5.5")
        || contents.contains("jiyi-local-proxy")
        || contents.contains("jiyi-keychain:")
}

pub fn jiyi_sensitive_environment_keys() -> &'static [&'static str] {
    JIYI_SENSITIVE_ENV_KEYS
}

fn jiyi_isolated_environment_pairs(codex_home: &Path, unix_home: &Path) -> Vec<(String, String)> {
    let config_home = unix_home.join(".config");
    let data_home = unix_home.join(".local").join("share");
    let cache_home = unix_home.join(".cache");
    let mut pairs = vec![
        (
            "CODEX_HOME".to_string(),
            codex_home.to_string_lossy().to_string(),
        ),
        ("HOME".to_string(), unix_home.to_string_lossy().to_string()),
        (
            "XDG_CONFIG_HOME".to_string(),
            config_home.to_string_lossy().to_string(),
        ),
        (
            "XDG_DATA_HOME".to_string(),
            data_home.to_string_lossy().to_string(),
        ),
        (
            "XDG_CACHE_HOME".to_string(),
            cache_home.to_string_lossy().to_string(),
        ),
    ];
    pairs.extend(
        jiyi_sensitive_environment_keys()
            .iter()
            .map(|key| ((*key).to_string(), String::new())),
    );
    pairs
}

fn configure_jiyi_child_process_env(
    command: &mut Command,
    codex_home: &Path,
    unix_home: Option<&Path>,
) {
    command.env("CODEX_HOME", codex_home);
    if let Some(unix_home) = unix_home {
        command
            .env("HOME", unix_home)
            .env("XDG_CONFIG_HOME", unix_home.join(".config"))
            .env("XDG_DATA_HOME", unix_home.join(".local").join("share"))
            .env("XDG_CACHE_HOME", unix_home.join(".cache"));
    }
    for key in jiyi_sensitive_environment_keys() {
        command.env_remove(key);
    }
}

pub fn build_macos_open_command(
    app_dir: &Path,
    debug_port: u16,
    extra_args: &[String],
) -> Vec<String> {
    let codex_home = crate::relay_config::default_codex_home_dir();
    let unix_home = crate::paths::default_jiyi_unix_home_dir();
    let browser_user_data_dir = crate::paths::default_jiyi_browser_user_data_dir();
    build_macos_open_command_with_isolated_home(
        app_dir,
        debug_port,
        extra_args,
        &codex_home,
        &unix_home,
        &browser_user_data_dir,
    )
}

pub fn build_macos_open_command_with_isolated_home(
    app_dir: &Path,
    debug_port: u16,
    extra_args: &[String],
    codex_home: &Path,
    unix_home: &Path,
    browser_user_data_dir: &Path,
) -> Vec<String> {
    let mut command = vec!["open".to_string(), "-n".to_string(), "-W".to_string()];
    for (key, value) in jiyi_isolated_environment_pairs(codex_home, unix_home) {
        command.push("--env".to_string());
        command.push(format!("{key}={value}"));
    }
    command.extend([
        app_dir.to_string_lossy().to_string(),
        "--args".to_string(),
        format!(
            "--user-data-dir={}",
            browser_user_data_dir.to_string_lossy()
        ),
    ]);
    command.extend(build_codex_arguments(debug_port, extra_args));
    command
}

pub fn build_macos_cleanup_command(
    app_dir: &Path,
    policy: MacosCleanupPolicy,
) -> Option<Vec<String>> {
    if policy == MacosCleanupPolicy::SkipQuitBecauseAlreadyRunning {
        return None;
    }
    let app_name = app_dir
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("Codex");
    Some(vec![
        "osascript".to_string(),
        "-e".to_string(),
        format!(
            r#"tell application "{}" to quit"#,
            app_name.replace('"', "\\\"")
        ),
    ])
}

async fn run_macos_cleanup_command(
    app_dir: &Path,
    policy: MacosCleanupPolicy,
) -> anyhow::Result<()> {
    let Some(command) = build_macos_cleanup_command(app_dir, policy) else {
        return Ok(());
    };
    let Some(executable) = command.first() else {
        return Ok(());
    };
    let _ = Command::new(executable)
        .args(&command[1..])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .with_context(|| format!("failed to request macOS app quit for {}", app_dir.display()))?;
    Ok(())
}

fn macos_app_dir_from_open_command(command: &[String]) -> Option<PathBuf> {
    command
        .iter()
        .skip(1)
        .find(|part| part.ends_with(".app"))
        .map(PathBuf::from)
}

async fn is_macos_app_running(app_dir: &Path) -> bool {
    if !cfg!(target_os = "macos") {
        return false;
    }
    let app_name = app_dir
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("Codex");
    let script = format!(
        r#"application "{}" is running"#,
        app_name.replace('"', "\\\"")
    );
    let Ok(output) = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await
    else {
        return false;
    };
    output.status.success()
        && String::from_utf8_lossy(&output.stdout)
            .trim()
            .eq_ignore_ascii_case("true")
}

#[cfg(windows)]
async fn wait_for_windows_process_id(process_id: u32) -> anyhow::Result<()> {
    tokio::task::spawn_blocking(move || wait_for_windows_process_id_blocking(process_id))
        .await
        .context("Windows process wait task failed")?
}

#[cfg(windows)]
async fn terminate_windows_process_id(process_id: u32) -> anyhow::Result<()> {
    tokio::task::spawn_blocking(move || terminate_windows_process_id_blocking(process_id))
        .await
        .context("Windows process termination task failed")?
}

#[cfg(windows)]
fn wait_for_windows_process_id_blocking(process_id: u32) -> anyhow::Result<()> {
    use windows::Win32::Foundation::{CloseHandle, WAIT_FAILED};
    use windows::Win32::System::Threading::{
        INFINITE, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
        WaitForSingleObject,
    };

    unsafe {
        let handle = OpenProcess(
            PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
            false,
            process_id,
        )
        .with_context(|| format!("failed to open Windows process id {process_id}"))?;
        let wait_result = WaitForSingleObject(handle, INFINITE);
        let _ = CloseHandle(handle);
        if wait_result == WAIT_FAILED {
            anyhow::bail!("failed to wait for Windows process id {process_id}");
        }
    }
    Ok(())
}

#[cfg(windows)]
fn terminate_windows_process_id_blocking(process_id: u32) -> anyhow::Result<()> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE, TerminateProcess,
    };

    unsafe {
        let handle = OpenProcess(
            PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION,
            false,
            process_id,
        )
        .with_context(|| format!("failed to open Windows process id {process_id}"))?;
        let terminate_result = TerminateProcess(handle, 1);
        let _ = CloseHandle(handle);
        terminate_result
            .with_context(|| format!("failed to terminate Windows process id {process_id}"))?;
    }
    Ok(())
}

#[cfg(not(windows))]
async fn wait_for_windows_process_id(process_id: u32) -> anyhow::Result<()> {
    anyhow::bail!("cannot wait for Windows process id {process_id} on this platform")
}

#[cfg(not(windows))]
async fn terminate_windows_process_id(process_id: u32) -> anyhow::Result<()> {
    anyhow::bail!("cannot terminate Windows process id {process_id} on this platform")
}

fn launch_status(
    status: &str,
    message: &str,
    debug_port: u16,
    helper_port: u16,
    app_dir: &Path,
) -> LaunchStatus {
    LaunchStatus {
        status: status.to_string(),
        message: message.to_string(),
        started_at_ms: now_ms(),
        debug_port: Some(debug_port),
        helper_port: Some(helper_port),
        codex_app: Some(app_dir.to_string_lossy().to_string()),
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn command_line_arguments(args: &[String]) -> String {
    args.iter()
        .map(|arg| quote_windows_argument(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

fn quote_windows_argument(arg: &str) -> String {
    if !arg.is_empty() && !arg.bytes().any(|byte| matches!(byte, b' ' | b'\t' | b'"')) {
        return arg.to_string();
    }
    let mut output = String::from("\"");
    let mut backslashes = 0;
    for ch in arg.chars() {
        match ch {
            '\\' => backslashes += 1,
            '"' => {
                output.push_str(&"\\".repeat(backslashes * 2 + 1));
                output.push('"');
                backslashes = 0;
            }
            _ => {
                output.push_str(&"\\".repeat(backslashes));
                output.push(ch);
                backslashes = 0;
            }
        }
    }
    output.push_str(&"\\".repeat(backslashes * 2));
    output.push('"');
    output
}

#[cfg(not(windows))]
pub async fn activate_packaged_app(
    _app_user_model_id: &str,
    _arguments: &str,
) -> anyhow::Result<u32> {
    anyhow::bail!("Packaged app activation is only supported on Windows")
}

#[cfg(windows)]
pub async fn activate_packaged_app(
    app_user_model_id: &str,
    arguments: &str,
) -> anyhow::Result<u32> {
    let app_user_model_id = app_user_model_id.to_string();
    let arguments = arguments.to_string();
    tokio::task::spawn_blocking(move || {
        activate_packaged_app_blocking(&app_user_model_id, &arguments)
    })
    .await
    .context("packaged app activation task failed")?
}

#[cfg(windows)]
fn activate_packaged_app_blocking(app_user_model_id: &str, arguments: &str) -> anyhow::Result<u32> {
    use windows::Win32::System::Com::{
        CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
        CoUninitialize,
    };
    use windows::Win32::UI::Shell::{ApplicationActivationManager, IApplicationActivationManager};
    use windows::core::HSTRING;

    unsafe {
        let coinit = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let should_uninitialize = coinit.is_ok();
        coinit.ok().or_else(|error| {
            const RPC_E_CHANGED_MODE: i32 = -2147417850;
            if error.code().0 == RPC_E_CHANGED_MODE {
                Ok(())
            } else {
                Err(error)
            }
        })?;

        let result: windows::core::Result<u32> = (|| {
            let manager: IApplicationActivationManager =
                CoCreateInstance(&ApplicationActivationManager, None, CLSCTX_LOCAL_SERVER)?;
            let process_id = manager.ActivateApplication(
                &HSTRING::from(app_user_model_id),
                &HSTRING::from(arguments),
                windows::Win32::UI::Shell::ACTIVATEOPTIONS(0),
            )?;
            Ok(process_id)
        })();

        if should_uninitialize {
            CoUninitialize();
        }
        result.map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_open_command_uses_exact_app_path() {
        let app_dir =
            Path::new("/Applications/极义codex.app/Contents/Resources/JiyiCodexClient.app");
        let command = build_macos_open_command(app_dir, 9234, &["--foo".to_string()]);

        assert_eq!(command[0], "open");
        assert!(command.iter().any(|part| part == "-n"));
        assert!(command.iter().any(|part| part == "-W"));
        assert!(!command.iter().any(|part| part == "-a"));
        assert!(
            command
                .iter()
                .any(|part| part == app_dir.to_string_lossy().as_ref())
        );
        assert_eq!(
            macos_app_dir_from_open_command(&command),
            Some(app_dir.to_path_buf())
        );
    }

    #[test]
    fn official_codex_config_guard_detects_jiyi_contamination() {
        assert!(official_codex_config_is_jiyi_contaminated(
            r#"notify = ["/Users/lv/.codex-session-delete/codex-home/computer-use/app"]"#
        ));
        assert!(official_codex_config_is_jiyi_contaminated(
            r#"base_url = "https://api.apimart.ai/v1""#
        ));
        assert!(official_codex_config_is_jiyi_contaminated(
            r#"base_url = "https://dashscope.aliyuncs.com/compatible-mode/v1""#
        ));
    }

    #[test]
    fn official_codex_config_guard_allows_official_config() {
        assert!(!official_codex_config_is_jiyi_contaminated(
            r#"notify = ["/Users/lv/.codex/computer-use/app", "turn-ended"]"#
        ));
        assert!(!official_codex_config_is_jiyi_contaminated(
            r#"{"OPENAI_API_KEY":"sk-official-user-key"}"#
        ));
    }
}
