use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::Context;
use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, mpsc};
use tokio_tungstenite::tungstenite::Message;

use crate::settings::{BackendSettings, SettingsStore, normalize_codex_extra_args};
use crate::status::{LaunchStatus, StatusStore};

#[cfg(windows)]
const POST_LAUNCH_COMPUTER_USE_GUARD_SECONDS: &[u64] = &[0, 5, 15, 30, 60, 120, 180, 240, 300];
#[cfg_attr(not(windows), allow(dead_code))]
const POST_LAUNCH_COMPUTER_USE_GUARD_STABLE_ATTEMPTS: usize = 3;

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
    async fn ensure_computer_use_config(&self, _settings: &BackendSettings) -> anyhow::Result<()> {
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
    async fn start_computer_use_guard_watchdog(
        &self,
        _settings: &BackendSettings,
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
    mobile_relay_host: Mutex<Option<MobileRelayHostRuntime>>,
    bridge_watchdog: Mutex<Option<BridgeWatchdogRuntime>>,
    computer_use_guard_watchdog: Mutex<Option<ComputerUseGuardWatchdogRuntime>>,
    computer_use_guard_artifacts: Mutex<Option<crate::computer_use_guard::GuardArtifacts>>,
}

struct HelperRuntime {
    shutdown: tokio::sync::oneshot::Sender<()>,
    task: tokio::task::JoinHandle<()>,
}

struct MobileRelayHostRuntime {
    shutdown: tokio::sync::oneshot::Sender<()>,
    task: tokio::task::JoinHandle<()>,
}

struct BridgeWatchdogRuntime {
    shutdown: tokio::sync::oneshot::Sender<()>,
    task: tokio::task::JoinHandle<()>,
}

struct ComputerUseGuardWatchdogRuntime {
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
        if settings.computer_use_guard_enabled {
            hooks.ensure_computer_use_config(&settings).await?;
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
        if settings.computer_use_guard_enabled {
            hooks.start_computer_use_guard_watchdog(&settings).await?;
        }

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
                    "Codex launched; Codex++ enhancements are still waiting for the page bridge.",
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
    settings.active_relay_uses_protocol_proxy()
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

    async fn start_mobile_relay_host(&self, helper_port: u16) -> anyhow::Result<()> {
        let settings = SettingsStore::default().load().unwrap_or_default();
        let Some(config) = MobileRelayHostConfig::from_settings_and_env(&settings) else {
            return Ok(());
        };
        let (shutdown, mut shutdown_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            run_mobile_relay_host(helper_port, config, &mut shutdown_rx).await;
        });
        if let Some(runtime) = self
            .mobile_relay_host
            .lock()
            .await
            .replace(MobileRelayHostRuntime { shutdown, task })
        {
            let _ = runtime.shutdown.send(());
            let _ = runtime.task.await;
        }
        Ok(())
    }
}

fn helper_bind_host() -> String {
    std::env::var("CODEX_PLUS_HELPER_BIND")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "127.0.0.1".to_string())
}

#[async_trait(?Send)]
impl LaunchHooks for DefaultLaunchHooks {
    fn resolve_app_dir(
        &self,
        app_dir: Option<&Path>,
        settings: &BackendSettings,
    ) -> anyhow::Result<PathBuf> {
        crate::app_paths::resolve_codex_app_dir_with_saved(
            app_dir,
            Some(settings.codex_app_path.as_str()),
        )
        .ok_or_else(|| anyhow::anyhow!("Codex App directory not found"))
    }

    fn select_debug_port(&self, requested: u16) -> u16 {
        crate::ports::select_packaged_codex_debug_port(requested)
    }

    fn select_helper_port(&self, requested: u16) -> u16 {
        crate::ports::select_platform_loopback_port(requested)
    }

    async fn load_settings(&self) -> anyhow::Result<BackendSettings> {
        SettingsStore::default().load()
    }

    async fn run_provider_sync(&self) -> anyhow::Result<()> {
        anyhow::bail!("provider sync requires launcher hooks with codex-plus-data integration")
    }

    async fn apply_active_relay_profile(&self, settings: &BackendSettings) -> anyhow::Result<()> {
        if !settings.relay_profiles_enabled {
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
            crate::relay_config::clear_relay_config_to_home_with_auth_and_computer_use_guard(
                &home,
                auth_contents,
                settings.computer_use_guard_enabled,
            )?;
            return Ok(());
        }
        crate::relay_config::apply_relay_profile_to_home_with_switch_rules_and_computer_use_guard(
            &home,
            &profile,
            &common_config,
            settings.computer_use_guard_enabled,
        )?;
        Ok(())
    }

    async fn ensure_computer_use_config(&self, settings: &BackendSettings) -> anyhow::Result<()> {
        if !settings.computer_use_guard_enabled {
            return Ok(());
        }
        let home = crate::relay_config::default_codex_home_dir();
        let artifacts = crate::computer_use_guard::resolve_computer_use_guard_artifacts(&home)?;
        crate::computer_use_guard::ensure_computer_use_config_with_artifacts(&home, &artifacts)?;
        *self.computer_use_guard_artifacts.lock().await = Some(artifacts);
        Ok(())
    }

    async fn start_helper(&self, helper_port: u16) -> anyhow::Result<()> {
        let bind_host = helper_bind_host();
        let listener = tokio::net::TcpListener::bind((bind_host.as_str(), helper_port))
            .await
            .with_context(|| {
                format!("failed to bind helper runtime on {bind_host}:{helper_port}")
            })?;
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "helper.listening",
            serde_json::json!({
                "helper_port": helper_port,
                "bind_host": bind_host,
                "address": format!("http://{bind_host}:{helper_port}")
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
        self.start_mobile_relay_host(helper_port).await?;
        Ok(())
    }

    async fn launch_codex(
        &self,
        app_dir: &Path,
        debug_port: u16,
        extra_args: &[String],
    ) -> anyhow::Result<CodexLaunch> {
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
            let command = build_macos_open_command(app_dir, debug_port, extra_args);
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
        let mut child_command = Command::new(executable);
        child_command
            .args(&command[1..])
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        child_command.creation_flags(crate::windows_integration::CREATE_NO_WINDOW);
        let child = child_command
            .spawn()
            .with_context(|| format!("failed to launch Codex executable {executable}"))?;
        *self.child.lock().await = Some(child);
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

    async fn start_computer_use_guard_watchdog(
        &self,
        settings: &BackendSettings,
    ) -> anyhow::Result<()> {
        if !settings.computer_use_guard_enabled {
            return Ok(());
        }
        #[cfg(windows)]
        {
            let home = crate::relay_config::default_codex_home_dir();
            let artifacts = self.computer_use_guard_artifacts.lock().await.clone();
            let (shutdown, mut shutdown_rx) = tokio::sync::oneshot::channel();
            let task = tokio::spawn(async move {
                run_post_launch_computer_use_guard(home, artifacts, &mut shutdown_rx).await;
            });
            if let Some(runtime) = self
                .computer_use_guard_watchdog
                .lock()
                .await
                .replace(ComputerUseGuardWatchdogRuntime { shutdown, task })
            {
                let _ = runtime.shutdown.send(());
                let _ = runtime.task.await;
            }
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
            }
            CodexLaunch::PackagedActivation { process_id, .. } => {
                if let Some(process_id) = process_id {
                    wait_for_windows_process_id(*process_id).await?;
                }
            }
        }
        let mut empty_streak = 0u32;
        loop {
            if crate::watcher::find_codex_processes().is_empty() {
                empty_streak = empty_streak.saturating_add(1);
                if empty_streak >= 3 {
                    break;
                }
            } else {
                empty_streak = 0;
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
        Ok(())
    }

    async fn shutdown_helper(&self, _helper_port: u16) {
        if let Some(runtime) = self.mobile_relay_host.lock().await.take() {
            let _ = runtime.shutdown.send(());
            let _ = runtime.task.await;
        }
        if let Some(runtime) = self.computer_use_guard_watchdog.lock().await.take() {
            let _ = runtime.shutdown.send(());
            let _ = runtime.task.await;
        }
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

struct AppServerRuntime {
    port: u16,
    source: &'static str,
    child: Option<Mutex<Child>>,
}

impl AppServerRuntime {
    async fn process_id(&self) -> Option<u32> {
        self.child.as_ref()?.lock().await.id()
    }
}

static APP_SERVER_RUNTIME: OnceLock<Mutex<Option<Arc<AppServerRuntime>>>> = OnceLock::new();

async fn handle_helper_connection(
    mut stream: tokio::net::TcpStream,
    remote_addr: Option<SocketAddr>,
) -> anyhow::Result<()> {
    let request_bytes = read_http_request(&mut stream).await?;
    let request = String::from_utf8_lossy(&request_bytes);
    let request_line = request.lines().next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let raw_path = parts.next().unwrap_or_default();
    let path = raw_path.split('?').next().unwrap_or(raw_path);
    let request_body = http_request_body(&request);
    let request_user_agent = header_value_from_request(&request, "user-agent");
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

    if path == "/mobile" && matches!(method, "GET" | "OPTIONS") {
        return handle_mobile_page_connection(&mut stream, method).await;
    }
    if path == "/app-server/ws" && matches!(method, "GET" | "OPTIONS") {
        return handle_app_server_websocket_proxy_connection(&mut stream, &request, method).await;
    }
    if path == "/app-server/rpc" && matches!(method, "POST" | "OPTIONS") {
        return handle_app_server_rpc_connection(&mut stream, method, request_body).await;
    }
    if path == "/app-server/status" && matches!(method, "GET" | "OPTIONS") {
        return handle_app_server_status_connection(&mut stream, method).await;
    }

    if crate::protocol_proxy::is_responses_proxy_path(path) && method == "POST" {
        return handle_protocol_proxy_connection(
            &mut stream,
            request_body,
            request_user_agent.as_deref(),
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
            request_user_agent.as_deref(),
            method,
            path,
            remote_addr_text,
        )
        .await;
    }
    if crate::protocol_proxy::is_models_proxy_path(path) && matches!(method, "GET" | "OPTIONS") {
        return handle_models_proxy_connection(
            &mut stream,
            request_user_agent.as_deref(),
            method,
            path,
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
        } else if path == "/overlay/image" && matches!(method, "GET" | "OPTIONS") {
            if method == "OPTIONS" {
                (
                    "200 OK".to_string(),
                    Vec::new(),
                    "application/octet-stream".to_string(),
                    "helper.overlay_image_options",
                )
            } else {
                overlay_image_response()
            }
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

#[derive(Debug, Clone)]
struct MobileRelayHostConfig {
    relay_url: String,
    room: String,
    token: String,
    encryption_key: String,
}

struct MobileRelayAppServerSession {
    sender: mpsc::UnboundedSender<Message>,
}

impl MobileRelayHostConfig {
    fn from_settings_and_env(settings: &BackendSettings) -> Option<Self> {
        if !settings.mobile_control_enabled && std::env::var("CODEX_PLUS_MOBILE_RELAY_URL").is_err()
        {
            return None;
        }
        let relay_url = env_or_setting(
            "CODEX_PLUS_MOBILE_RELAY_URL",
            &settings.mobile_control_relay_url,
        )?;
        let room = env_or_setting(
            "CODEX_PLUS_MOBILE_RELAY_ROOM",
            &settings.mobile_control_room,
        )?;
        let token = std::env::var("CODEX_PLUS_MOBILE_RELAY_TOKEN")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| room.clone());
        let encryption_key =
            env_or_setting("CODEX_PLUS_MOBILE_RELAY_KEY", &settings.mobile_control_key)?;
        Some(Self {
            relay_url,
            room,
            token,
            encryption_key,
        })
    }

    fn cipher(&self) -> Aes256Gcm {
        mobile_relay_cipher(&self.encryption_key)
    }

    fn host_url(&self) -> String {
        let separator = if self.relay_url.contains('?') {
            '&'
        } else {
            '?'
        };
        let role_path_url =
            if self.relay_url.ends_with("/host") || self.relay_url.contains("/host?") {
                self.relay_url.clone()
            } else {
                format!("{}/host", self.relay_url.trim_end_matches('/'))
            };
        format!(
            "{role_path_url}{separator}room={}&token={}",
            percent_encode_query(&self.room),
            percent_encode_query(&self.token)
        )
    }
}

fn env_or_setting(env_name: &str, setting: &str) -> Option<String> {
    std::env::var(env_name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            let value = setting.trim();
            (!value.is_empty()).then(|| value.to_string())
        })
}

async fn run_mobile_relay_host(
    helper_port: u16,
    config: MobileRelayHostConfig,
    shutdown_rx: &mut tokio::sync::oneshot::Receiver<()>,
) {
    let mut retry_delay = std::time::Duration::from_secs(1);
    loop {
        tokio::select! {
            _ = &mut *shutdown_rx => break,
            result = run_mobile_relay_host_once(helper_port, &config) => {
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "mobile_relay.host_disconnected",
                    serde_json::json!({
                        "helper_port": helper_port,
                        "relay_url": config.relay_url,
                        "room": config.room,
                        "message": result.err().map(|error| error.to_string())
                    }),
                );
            }
        }
        tokio::select! {
            _ = &mut *shutdown_rx => break,
            _ = tokio::time::sleep(retry_delay) => {}
        }
        retry_delay = (retry_delay * 2).min(std::time::Duration::from_secs(30));
    }
}

async fn run_mobile_relay_host_once(
    helper_port: u16,
    config: &MobileRelayHostConfig,
) -> anyhow::Result<()> {
    let host_url = config.host_url();
    let (mut socket, _) = tokio_tungstenite::connect_async(&host_url)
        .await
        .with_context(|| format!("failed to connect mobile relay host {host_url}"))?;
    let _ = crate::diagnostic_log::append_diagnostic_log(
        "mobile_relay.host_connected",
        serde_json::json!({
            "helper_port": helper_port,
            "relay_url": config.relay_url,
            "room": config.room
        }),
    );
    let cipher = config.cipher();
    let (relay_tx, mut relay_rx) = mpsc::unbounded_channel::<Message>();
    let mut sessions: std::collections::HashMap<String, MobileRelayAppServerSession> =
        std::collections::HashMap::new();
    loop {
        tokio::select! {
            relay_message = relay_rx.recv() => {
                let Some(relay_message) = relay_message else {
                    break;
                };
                socket
                    .send(relay_message)
                    .await
                    .context("failed to send mobile relay async message")?;
                continue;
            }
            inbound = socket.next() => {
                let Some(inbound) = inbound else {
                    break;
                };
                let message = inbound.context("failed to read mobile relay message")?;
                if message.is_close() {
                    break;
                }
                let Some(response) = handle_mobile_relay_host_message(
                    helper_port,
                    &cipher,
                    message,
                    relay_tx.clone(),
                    &mut sessions,
                ).await
                else {
                    continue;
                };
                socket
                    .send(Message::Text(response.to_string().into()))
                    .await
                    .context("failed to send mobile relay response")?;
            }
        }
    }
    for (_, sender) in sessions {
        let _ = sender.sender.send(Message::Close(None));
    }
    Ok(())
}

async fn handle_mobile_relay_host_message(
    helper_port: u16,
    cipher: &Aes256Gcm,
    message: Message,
    relay_tx: mpsc::UnboundedSender<Message>,
    app_server_sessions: &mut std::collections::HashMap<String, MobileRelayAppServerSession>,
) -> Option<serde_json::Value> {
    let text = match message {
        Message::Text(text) => text.to_string(),
        Message::Binary(bytes) => String::from_utf8(bytes.to_vec()).ok()?,
        _ => return None,
    };
    let envelope = serde_json::from_str::<serde_json::Value>(&text).ok()?;
    let plaintext_mode = envelope.get("type").and_then(Value::as_str) == Some("plaintext");
    let request = decrypt_mobile_relay_request(cipher, &envelope).ok()?;
    if request.get("type").and_then(Value::as_str) == Some("appServerConnect") {
        return handle_mobile_relay_app_server_connect(
            helper_port,
            cipher,
            &request,
            plaintext_mode,
            relay_tx,
            app_server_sessions,
        )
        .await;
    }
    if request.get("type").and_then(Value::as_str) == Some("appServerMessage") {
        return handle_mobile_relay_app_server_message(&request, app_server_sessions).await;
    }
    if request.get("type").and_then(Value::as_str) == Some("appServerClose") {
        return handle_mobile_relay_app_server_close(&request, app_server_sessions).await;
    }
    if request.get("type").and_then(Value::as_str) != Some("httpRequest") {
        return None;
    }
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let response = match proxy_mobile_relay_http_request(helper_port, &request).await {
        Ok(response) => serde_json::json!({
            "type": "httpResponse",
            "id": id,
            "status": response.status,
            "headers": response.headers,
            "body": response.body
        }),
        Err(error) => serde_json::json!({
            "type": "httpResponse",
            "id": id,
            "status": 502,
            "headers": {"content-type": "application/json; charset=utf-8"},
            "body": serde_json::json!({
                "status": "failed",
                "message": error.to_string()
            }).to_string()
        }),
    };
    encode_mobile_relay_payload(cipher, plaintext_mode, &response).ok()
}

async fn handle_mobile_relay_app_server_connect(
    _helper_port: u16,
    cipher: &Aes256Gcm,
    request: &Value,
    plaintext_mode: bool,
    relay_tx: mpsc::UnboundedSender<Message>,
    app_server_sessions: &mut std::collections::HashMap<String, MobileRelayAppServerSession>,
) -> Option<Value> {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let session_id = request
        .get("sessionId")
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    if let Some(previous) = app_server_sessions.remove(&session_id) {
        let _ = previous.sender.send(Message::Close(None));
    }
    let (app_tx, app_rx) = mpsc::unbounded_channel::<Message>();
    app_server_sessions.insert(
        session_id.clone(),
        MobileRelayAppServerSession { sender: app_tx },
    );
    let session_cipher = cipher.clone();
    tokio::spawn(run_mobile_relay_app_server_session(
        session_cipher,
        plaintext_mode,
        relay_tx,
        session_id.clone(),
        app_rx,
    ));
    encode_mobile_relay_payload(
        &cipher,
        plaintext_mode,
        &serde_json::json!({
            "type": "appServerConnected",
            "id": id,
            "sessionId": session_id
        }),
    )
    .ok()
}

async fn handle_mobile_relay_app_server_message(
    request: &Value,
    app_server_sessions: &mut std::collections::HashMap<String, MobileRelayAppServerSession>,
) -> Option<Value> {
    let session_id = request.get("sessionId").and_then(Value::as_str)?;
    let text = request.get("message").and_then(Value::as_str)?;
    let session = app_server_sessions.get(session_id)?;
    let _ = session.sender.send(Message::Text(text.to_string().into()));
    None
}

async fn handle_mobile_relay_app_server_close(
    request: &Value,
    app_server_sessions: &mut std::collections::HashMap<String, MobileRelayAppServerSession>,
) -> Option<Value> {
    let session_id = request.get("sessionId").and_then(Value::as_str)?;
    if let Some(sender) = app_server_sessions.remove(session_id) {
        let _ = sender.sender.send(Message::Close(None));
    }
    None
}

async fn run_mobile_relay_app_server_session(
    cipher: Aes256Gcm,
    plaintext_mode: bool,
    relay_tx: mpsc::UnboundedSender<Message>,
    session_id: String,
    mut app_rx: mpsc::UnboundedReceiver<Message>,
) {
    let result = async {
        let runtime = ensure_app_server_runtime().await?;
        let url = format!("ws://127.0.0.1:{}/rpc", runtime.port);
        let (mut upstream, _) = tokio_tungstenite::connect_async(&url)
            .await
            .with_context(|| format!("failed to connect Codex app-server {url}"))?;
        loop {
            tokio::select! {
                outbound = app_rx.recv() => {
                    let Some(outbound) = outbound else {
                        break;
                    };
                    if outbound.is_close() {
                        break;
                    }
                    upstream
                        .send(outbound)
                        .await
                        .context("failed to send app-server message")?;
                }
                inbound = upstream.next() => {
                    let Some(inbound) = inbound else {
                        break;
                    };
                    let inbound = inbound.context("failed to read app-server message")?;
                    if inbound.is_close() {
                        break;
                    }
                let message = match inbound {
                    Message::Text(text) => text.to_string(),
                    Message::Binary(bytes) => String::from_utf8(bytes.to_vec())
                        .context("app-server returned non-utf8 binary")?,
                    Message::Ping(_) | Message::Pong(_) => continue,
                    Message::Close(_) => break,
                    Message::Frame(_) => continue,
                };
                if let Ok(value) = serde_json::from_str::<Value>(&message) {
                    let _ = crate::diagnostic_log::append_diagnostic_log(
                        "mobile_relay.app_server_message",
                        serde_json::json!({
                            "sessionId": session_id,
                            "id": value.get("id").cloned().unwrap_or(Value::Null),
                            "method": value.get("method").and_then(Value::as_str),
                            "hasError": value.get("error").is_some()
                        }),
                    );
                }
                    let envelope = encode_mobile_relay_payload(
                        &cipher,
                        plaintext_mode,
                        &serde_json::json!({
                            "type": "appServerMessage",
                            "sessionId": session_id,
                            "message": message
                        }),
                    )?;
                    let _ = relay_tx.send(Message::Text(envelope.to_string().into()));
                }
            }
        }
        anyhow::Ok(())
    }
    .await;
    let detail = match result {
        Ok(()) => serde_json::json!({
            "type": "appServerClosed",
            "sessionId": session_id
        }),
        Err(error) => serde_json::json!({
            "type": "appServerClosed",
            "sessionId": session_id,
            "error": error.to_string()
        }),
    };
    if let Ok(envelope) = encrypt_mobile_relay_payload(&cipher, &detail) {
        let _ = relay_tx.send(Message::Text(envelope.to_string().into()));
    }
}

struct MobileRelayHttpResponse {
    status: u16,
    headers: serde_json::Map<String, Value>,
    body: String,
}

async fn proxy_mobile_relay_http_request(
    helper_port: u16,
    request: &serde_json::Value,
) -> anyhow::Result<MobileRelayHttpResponse> {
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("GET")
        .to_ascii_uppercase();
    let path = request
        .get("path")
        .and_then(Value::as_str)
        .filter(|path| path.starts_with('/'))
        .unwrap_or("/");
    let body = request.get("body").and_then(Value::as_str).unwrap_or("");
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", helper_port))
        .await
        .with_context(|| format!("failed to connect helper on 127.0.0.1:{helper_port}"))?;
    let content_type = request
        .get("headers")
        .and_then(Value::as_object)
        .and_then(|headers| {
            headers
                .get("content-type")
                .or_else(|| headers.get("Content-Type"))
                .and_then(Value::as_str)
        })
        .unwrap_or("application/json; charset=utf-8");
    let wire = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{helper_port}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.as_bytes().len()
    );
    stream.write_all(wire.as_bytes()).await?;
    stream.shutdown().await?;
    let mut response_bytes = Vec::new();
    stream.read_to_end(&mut response_bytes).await?;
    parse_mobile_relay_http_response(&response_bytes)
}

fn parse_mobile_relay_http_response(bytes: &[u8]) -> anyhow::Result<MobileRelayHttpResponse> {
    let text = String::from_utf8_lossy(bytes);
    let (header_text, body) = text
        .split_once("\r\n\r\n")
        .ok_or_else(|| anyhow::anyhow!("helper returned an invalid HTTP response"))?;
    let mut lines = header_text.lines();
    let status_line = lines.next().unwrap_or_default();
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(502);
    let mut headers = serde_json::Map::new();
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        headers.insert(
            name.trim().to_ascii_lowercase(),
            Value::String(value.trim().to_string()),
        );
    }
    Ok(MobileRelayHttpResponse {
        status,
        headers,
        body: body.to_string(),
    })
}

fn percent_encode_query(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(byte as char)
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

fn mobile_relay_cipher(key_text: &str) -> Aes256Gcm {
    let digest = Sha256::digest(key_text.as_bytes());
    Aes256Gcm::new_from_slice(&digest).expect("sha256 always returns 32 bytes")
}

fn mobile_relay_nonce() -> [u8; 12] {
    let now = now_ms();
    let mut nonce = [0_u8; 12];
    nonce[..8].copy_from_slice(&now.to_le_bytes());
    let random = uuid::Uuid::new_v4();
    nonce[8..].copy_from_slice(&random.as_bytes()[..4]);
    nonce
}

fn encrypt_mobile_relay_payload(cipher: &Aes256Gcm, payload: &Value) -> anyhow::Result<Value> {
    let nonce = mobile_relay_nonce();
    let plaintext = serde_json::to_vec(payload)?;
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), plaintext.as_slice())
        .map_err(|_| anyhow::anyhow!("手机控制数据加密失败"))?;
    Ok(serde_json::json!({
        "type": "encrypted",
        "nonce": URL_SAFE_NO_PAD.encode(nonce),
        "payload": URL_SAFE_NO_PAD.encode(ciphertext)
    }))
}

fn encode_mobile_relay_payload(
    cipher: &Aes256Gcm,
    plaintext_mode: bool,
    payload: &Value,
) -> anyhow::Result<Value> {
    if plaintext_mode {
        return Ok(serde_json::json!({
            "type": "plaintext",
            "payload": payload
        }));
    }
    encrypt_mobile_relay_payload(cipher, payload)
}

fn decrypt_mobile_relay_request(cipher: &Aes256Gcm, envelope: &Value) -> anyhow::Result<Value> {
    if envelope.get("type").and_then(Value::as_str) == Some("plaintext") {
        return envelope
            .get("payload")
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("手机控制明文数据包缺少 payload"));
    }
    decrypt_mobile_relay_envelope(cipher, envelope)
}

fn decrypt_mobile_relay_envelope(cipher: &Aes256Gcm, envelope: &Value) -> anyhow::Result<Value> {
    if envelope.get("type").and_then(Value::as_str) != Some("encrypted") {
        anyhow::bail!("手机控制数据包未加密");
    }
    let nonce_text = envelope
        .get("nonce")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("手机控制数据包缺少 nonce"))?;
    let payload_text = envelope
        .get("payload")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("手机控制数据包缺少 payload"))?;
    let nonce = URL_SAFE_NO_PAD.decode(nonce_text)?;
    if nonce.len() != 12 {
        anyhow::bail!("手机控制 nonce 长度无效");
    }
    let ciphertext = URL_SAFE_NO_PAD.decode(payload_text)?;
    let plaintext = cipher
        .decrypt(Nonce::from_slice(&nonce), ciphertext.as_slice())
        .map_err(|_| anyhow::anyhow!("手机控制数据解密失败"))?;
    Ok(serde_json::from_slice(&plaintext)?)
}

fn overlay_image_response() -> (String, Vec<u8>, String, &'static str) {
    let not_found = || {
        (
            "404 Not Found".to_string(),
            serde_json::to_vec(&serde_json::json!({
                "status": "failed",
                "message": "图片覆盖层未启用或图片不可用"
            }))
            .unwrap_or_default(),
            "application/json; charset=utf-8".to_string(),
            "helper.overlay_image_not_found",
        )
    };
    let settings = SettingsStore::default().load().unwrap_or_default();
    if !settings.codex_app_image_overlay_enabled {
        return not_found();
    }
    let image_path = PathBuf::from(settings.codex_app_image_overlay_path.trim());
    if image_path.as_os_str().is_empty() || !image_path.is_file() {
        return not_found();
    }
    let Some(content_type) = overlay_image_content_type(&image_path) else {
        return not_found();
    };
    match std::fs::read(&image_path) {
        Ok(bytes) => (
            "200 OK".to_string(),
            bytes,
            content_type.to_string(),
            "helper.overlay_image_ok",
        ),
        Err(_) => not_found(),
    }
}

fn overlay_image_content_type(path: &Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => Some("image/png"),
        Some("jpg") | Some("jpeg") => Some("image/jpeg"),
        Some("webp") => Some("image/webp"),
        Some("gif") => Some("image/gif"),
        Some("bmp") => Some("image/bmp"),
        _ => None,
    }
}

async fn handle_mobile_page_connection(
    stream: &mut tokio::net::TcpStream,
    method: &str,
) -> anyhow::Result<()> {
    if method == "OPTIONS" {
        write_options_response(stream).await?;
        stream.shutdown().await?;
        return Ok(());
    }
    write_http_no_store_response(
        stream,
        "200 OK",
        "text/html; charset=utf-8",
        mobile_page_html(&serde_json::to_string(&mobile_model_catalog_value())?).as_bytes(),
    )
    .await?;
    stream.shutdown().await?;
    Ok(())
}

async fn handle_app_server_status_connection(
    stream: &mut tokio::net::TcpStream,
    method: &str,
) -> anyhow::Result<()> {
    if method == "OPTIONS" {
        write_options_response(stream).await?;
        stream.shutdown().await?;
        return Ok(());
    }
    let body = serde_json::to_vec(&app_server_status_response().await)?;
    write_http_response(stream, "200 OK", "application/json; charset=utf-8", &body).await?;
    stream.shutdown().await?;
    Ok(())
}

async fn handle_app_server_rpc_connection(
    stream: &mut tokio::net::TcpStream,
    method: &str,
    request_body: &str,
) -> anyhow::Result<()> {
    if method == "OPTIONS" {
        write_options_response(stream).await?;
        stream.shutdown().await?;
        return Ok(());
    }
    let payload = serde_json::from_str::<Value>(request_body).unwrap_or_else(|error| {
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": null,
            "error": {"code": -32700, "message": error.to_string()}
        })
    });
    let body = match app_server_rpc_once(payload).await {
        Ok(response) => serde_json::to_vec(&response)?,
        Err(error) => serde_json::to_vec(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": null,
            "error": {"code": -32000, "message": error.to_string()}
        }))?,
    };
    write_http_response(stream, "200 OK", "application/json; charset=utf-8", &body).await?;
    stream.shutdown().await?;
    Ok(())
}

async fn app_server_rpc_once(payload: Value) -> anyhow::Result<Value> {
    let runtime = ensure_app_server_runtime().await?;
    let url = format!("ws://127.0.0.1:{}/rpc", runtime.port);
    let (mut socket, _) = tokio_tungstenite::connect_async(&url)
        .await
        .with_context(|| format!("failed to connect Codex app-server {url}"))?;
    if payload.get("method").and_then(Value::as_str) != Some("initialize") {
        let init_id = "__codex_plus_mobile_init__";
        let init_payload = serde_json::json!({
            "jsonrpc": "2.0",
            "id": init_id,
            "method": "initialize",
            "params": {
                "clientInfo": {"name": "Codex++ Mobile Relay", "version": "1.0.0"},
                "capabilities": {"experimentalApi": true}
            }
        });
        socket
            .send(Message::Text(init_payload.to_string().into()))
            .await
            .context("failed to send app-server initialize")?;
        let init_response = read_app_server_rpc_response(
            &mut socket,
            Some(Value::String(init_id.to_string())),
            std::time::Duration::from_secs(20),
        )
        .await?;
        if let Some(error) = init_response.get("error") {
            anyhow::bail!("app-server initialize failed: {error}");
        }
    }
    socket
        .send(Message::Text(payload.to_string().into()))
        .await
        .context("failed to send app-server rpc")?;
    let requested_id = payload.get("id").cloned();
    if payload.get("method").and_then(Value::as_str) == Some("turn/start") {
        let thread_id = payload
            .get("params")
            .and_then(|params| params.get("threadId").or_else(|| params.get("thread_id")))
            .and_then(Value::as_str)
            .map(str::to_string);
        let response = read_app_server_rpc_response(
            &mut socket,
            requested_id,
            std::time::Duration::from_secs(60),
        )
        .await?;
        if response.get("error").is_none() {
            tokio::spawn(async move {
                drain_app_server_turn_socket(
                    socket,
                    thread_id,
                    std::time::Duration::from_secs(600),
                )
                .await;
            });
        }
        Ok(response)
    } else {
        read_app_server_rpc_response(
            &mut socket,
            requested_id,
            std::time::Duration::from_secs(60),
        )
        .await
    }
}

async fn read_app_server_rpc_response(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    requested_id: Option<Value>,
    timeout: std::time::Duration,
) -> anyhow::Result<Value> {
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => anyhow::bail!("app-server rpc timed out"),
            message = socket.next() => {
                let Some(message) = message else {
                    anyhow::bail!("app-server rpc connection closed");
                };
                let message = message.context("failed to read app-server rpc")?;
                let response = app_server_message_json(message)?;
                if response.get("id") == requested_id.as_ref() {
                    return Ok(response);
                }
            }
        }
    }
}

async fn drain_app_server_turn_socket(
    mut socket: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    thread_id: Option<String>,
    timeout: std::time::Duration,
) {
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => {
                break;
            }
            message = socket.next() => {
                let Some(message) = message else {
                    break;
                };
                let Ok(message) = message else {
                    break;
                };
                if matches!(message, Message::Ping(_) | Message::Pong(_)) {
                    continue;
                }
                let Ok(value) = app_server_message_json(message) else {
                    continue;
                };
                if app_server_turn_finished_for_thread(&value, thread_id.as_deref()) {
                    break;
                }
            }
        }
    }
    let _ = socket.close(None).await;
}

fn app_server_message_json(message: Message) -> anyhow::Result<Value> {
    let text = match message {
        Message::Text(text) => text.to_string(),
        Message::Binary(bytes) => {
            String::from_utf8(bytes.to_vec()).context("app-server rpc returned non-utf8 binary")?
        }
        Message::Close(_) => anyhow::bail!("app-server rpc connection closed"),
        _ => anyhow::bail!("app-server rpc returned unsupported websocket frame"),
    };
    serde_json::from_str::<Value>(&text).context("app-server rpc returned invalid json")
}

fn app_server_turn_finished_for_thread(message: &Value, thread_id: Option<&str>) -> bool {
    let method = message
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !matches!(
        method,
        "turn/completed" | "turn/failed" | "turn/cancelled" | "thread/status/changed"
    ) {
        return false;
    }
    if method == "thread/status/changed"
        && !matches!(
            message
                .get("params")
                .and_then(|params| params.get("status"))
                .and_then(Value::as_str),
            Some("idle" | "completed" | "failed" | "cancelled")
        )
    {
        return false;
    }
    let Some(expected) = thread_id else {
        return true;
    };
    app_server_event_thread_id(message)
        .as_deref()
        .map(|actual| actual == expected)
        .unwrap_or_else(|| message.to_string().contains(expected))
}

fn app_server_event_thread_id(message: &Value) -> Option<String> {
    let params = message.get("params")?;
    params
        .get("threadId")
        .or_else(|| params.get("thread_id"))
        .or_else(|| params.get("thread").and_then(|thread| thread.get("id")))
        .or_else(|| params.get("turn").and_then(|turn| turn.get("threadId")))
        .or_else(|| params.get("turn").and_then(|turn| turn.get("thread_id")))
        .or_else(|| params.get("item").and_then(|item| item.get("threadId")))
        .or_else(|| params.get("item").and_then(|item| item.get("thread_id")))
        .and_then(Value::as_str)
        .map(str::to_string)
}

async fn handle_app_server_websocket_proxy_connection(
    stream: &mut tokio::net::TcpStream,
    request: &str,
    method: &str,
) -> anyhow::Result<()> {
    if method == "OPTIONS" {
        write_options_response(stream).await?;
        stream.shutdown().await?;
        return Ok(());
    }
    let runtime = match ensure_app_server_runtime().await {
        Ok(runtime) => runtime,
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
            stream.shutdown().await?;
            return Ok(());
        }
    };
    let upstream_request = rewrite_app_server_ws_request(request, runtime.port);
    let mut upstream = tokio::net::TcpStream::connect(("127.0.0.1", runtime.port)).await?;
    upstream.write_all(upstream_request.as_bytes()).await?;
    let _ = tokio::io::copy_bidirectional(stream, &mut upstream).await?;
    stream.shutdown().await?;
    Ok(())
}

fn rewrite_app_server_ws_request(request: &str, app_server_port: u16) -> String {
    let mut out = format!("GET /rpc HTTP/1.1\r\nHost: 127.0.0.1:{app_server_port}\r\n");
    for line in request.lines().skip(1) {
        if line.is_empty() {
            break;
        }
        let Some((name, _)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim();
        if name.eq_ignore_ascii_case("host")
            || name.eq_ignore_ascii_case("origin")
            || name.eq_ignore_ascii_case("sec-websocket-protocol")
        {
            continue;
        }
        out.push_str(line);
        out.push_str("\r\n");
    }
    out.push_str("\r\n");
    out
}

async fn app_server_status_response() -> Value {
    match ensure_app_server_runtime().await {
        Ok(runtime) => serde_json::json!({
            "status": "ok",
            "port": runtime.port,
            "pid": runtime.process_id().await,
            "source": runtime.source,
            "rpcUrl": format!("ws://127.0.0.1:{}/rpc", runtime.port),
            "transport": "codex-app-server"
        }),
        Err(error) => serde_json::json!({
            "status": "failed",
            "message": error.to_string()
        }),
    }
}

async fn ensure_app_server_runtime() -> anyhow::Result<Arc<AppServerRuntime>> {
    let runtime_slot = APP_SERVER_RUNTIME.get_or_init(|| Mutex::new(None));
    let mut guard = runtime_slot.lock().await;
    if let Some(runtime) = guard.as_ref() {
        if app_server_ready(runtime.port).await {
            return Ok(runtime.clone());
        }
    }
    if let Some(runtime) = existing_app_server_runtime().await {
        *guard = Some(runtime.clone());
        return Ok(runtime);
    }
    let runtime = Arc::new(start_app_server_runtime().await?);
    *guard = Some(runtime.clone());
    Ok(runtime)
}

async fn existing_app_server_runtime() -> Option<Arc<AppServerRuntime>> {
    for key in ["CODEX_PLUS_APP_SERVER_URL", "CODEX_APP_SERVER_URL"] {
        let Ok(value) = std::env::var(key) else {
            continue;
        };
        let Some(port) = app_server_port_from_url(&value) else {
            continue;
        };
        if app_server_ready(port).await {
            return Some(Arc::new(AppServerRuntime {
                port,
                source: "external",
                child: None,
            }));
        }
    }
    None
}

fn app_server_port_from_url(value: &str) -> Option<u16> {
    let trimmed = value.trim();
    let without_scheme = trimmed
        .strip_prefix("ws://")
        .or_else(|| trimmed.strip_prefix("http://"))?;
    let authority = without_scheme.split('/').next().unwrap_or(without_scheme);
    let (host, port) = authority.rsplit_once(':')?;
    matches!(host, "127.0.0.1" | "localhost").then(|| port.parse().ok())?
}

async fn start_app_server_runtime() -> anyhow::Result<AppServerRuntime> {
    let port = reserve_app_server_port()?;
    let codex = resolve_codex_cli_path();
    let mut command = Command::new(&codex);
    command
        .arg("app-server")
        .arg("--listen")
        .arg(format!("ws://127.0.0.1:{port}"))
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    command.creation_flags(crate::windows_integration::CREATE_NO_WINDOW);
    let child = command
        .spawn()
        .with_context(|| format!("无法启动 Codex app-server：{codex}"))?;
    wait_for_app_server_ready(port).await?;
    Ok(AppServerRuntime {
        port,
        source: "managed",
        child: Some(Mutex::new(child)),
    })
}

fn resolve_codex_cli_path() -> String {
    std::env::var("CODEX_CLI_PATH")
        .ok()
        .filter(|path| !path.trim().is_empty())
        .filter(|path| Path::new(path).is_file())
        .or_else(|| {
            crate::cli_wrapper::resolve_real_codex().map(|path| path.to_string_lossy().to_string())
        })
        .unwrap_or_else(|| "codex".to_string())
}

fn reserve_app_server_port() -> anyhow::Result<u16> {
    for _ in 0..20 {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
        let port = listener.local_addr()?.port();
        if port != crate::protocol_proxy::DEFAULT_PROTOCOL_PROXY_PORT {
            return Ok(port);
        }
    }
    anyhow::bail!("无法为 Codex app-server 预留端口")
}

async fn wait_for_app_server_ready(port: u16) -> anyhow::Result<()> {
    for _ in 0..80 {
        if app_server_ready(port).await {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    anyhow::bail!("Codex app-server 启动超时")
}

async fn app_server_ready(port: u16) -> bool {
    let Ok(mut stream) = tokio::net::TcpStream::connect(("127.0.0.1", port)).await else {
        return false;
    };
    let request = "GET /readyz HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
    if stream.write_all(request.as_bytes()).await.is_err() {
        return false;
    }
    let Ok(response) = read_http_request(&mut stream).await else {
        return false;
    };
    response.starts_with(b"HTTP/1.1 200") || response.starts_with(b"HTTP/1.0 200")
}

fn mobile_model_catalog_value() -> Value {
    let settings = SettingsStore::default().load().unwrap_or_default();
    let profile = settings.active_relay_profile();
    let mut models = Vec::new();
    for value in profile
        .model_list
        .split(['\r', '\n', ','])
        .chain(std::iter::once(profile.model.as_str()))
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if !models.iter().any(|existing| existing == value) {
            models.push(value.to_string());
        }
    }
    let default_model = if models.iter().any(|model| model == &profile.model) {
        profile.model.trim().to_string()
    } else {
        models.first().cloned().unwrap_or_default()
    };
    serde_json::json!({
        "status": if models.is_empty() { "not_configured" } else { "ok" },
        "model": profile.model.trim(),
        "model_provider": profile.id.trim(),
        "provider_name": if profile.name.trim().is_empty() { profile.id.trim() } else { profile.name.trim() },
        "default_model": default_model,
        "models": models
    })
}

fn mobile_page_html(model_catalog_json: &str) -> String {
    let html = r#"<!doctype html>
<html lang="zh-CN">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover" />
  <title>Codex++ Mobile</title>
  <style>
    :root {
      color-scheme: light dark;
      --bg: #f6f7f8;
      --panel: #ffffff;
      --line: #d8dde3;
      --text: #101418;
      --muted: #69727d;
      --accent: #0f766e;
      --accent-2: #0b5f59;
      --danger: #b42318;
      --bubble-user: #e7f4f1;
      --bubble-agent: #ffffff;
    }
    @media (prefers-color-scheme: dark) {
      :root {
        --bg: #111315;
        --panel: #191c20;
        --line: #2e343b;
        --text: #f2f4f7;
        --muted: #a4adb8;
        --accent: #2dd4bf;
        --accent-2: #5eead4;
        --danger: #ff8a80;
        --bubble-user: #123c38;
        --bubble-agent: #20242a;
      }
    }
    * { box-sizing: border-box; }
    html, body { margin: 0; min-height: 100%; background: var(--bg); color: var(--text); font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; }
    body { overflow: hidden; }
    button, input, select, textarea { font: inherit; }
    button { border: 1px solid var(--line); color: var(--text); background: var(--panel); border-radius: 8px; padding: 9px 12px; }
    button.icon { width: 34px; height: 34px; padding: 0; display: inline-grid; place-items: center; font-weight: 700; }
    button.primary { border-color: var(--accent); background: var(--accent); color: #fff; }
    button:disabled { opacity: .55; }
    select { min-width: 0; border: 1px solid var(--line); border-radius: 8px; padding: 8px 10px; background: var(--bg); color: var(--text); outline: none; }
    .app { height: 100vh; height: 100dvh; display: grid; grid-template-rows: auto 1fr; }
    .topbar { display: flex; align-items: center; gap: 10px; padding: calc(env(safe-area-inset-top) + 10px) 12px 10px; border-bottom: 1px solid var(--line); background: var(--panel); }
    .title { font-weight: 700; white-space: nowrap; }
    .status { min-width: 0; flex: 1; color: var(--muted); font-size: 13px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .layout { min-height: 0; display: grid; grid-template-columns: 360px 1fr; }
    .sessions { min-height: 0; border-right: 1px solid var(--line); background: var(--panel); display: grid; grid-template-rows: auto 1fr; }
    .search { padding: 10px; border-bottom: 1px solid var(--line); }
    .search input { width: 100%; border: 1px solid var(--line); border-radius: 8px; padding: 10px 12px; background: var(--bg); color: var(--text); outline: none; }
    .list { overflow: auto; }
    .group { border-bottom: 1px solid var(--line); }
    .group-title { width: 100%; position: sticky; top: 0; z-index: 1; padding: 8px 10px; background: color-mix(in srgb, var(--panel) 92%, var(--bg)); color: var(--muted); font-size: 12px; font-weight: 700; border-bottom: 1px solid var(--line); display: grid; grid-template-columns: auto minmax(0, 1fr) auto auto; gap: 8px; align-items: center; cursor: pointer; }
    .group-name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .group-count { color: var(--muted); font-weight: 500; }
    .group-new { width: 28px; height: 28px; padding: 0; border-radius: 7px; }
    .chevron { color: var(--muted); width: 12px; text-align: center; }
    .group.collapsed .item { display: none; }
    .item { width: 100%; display: block; text-align: left; border: 0; border-bottom: 1px solid var(--line); border-radius: 0; padding: 12px; background: transparent; }
    .group .item:last-child { border-bottom: 0; }
    .item.active { background: color-mix(in srgb, var(--accent) 12%, transparent); }
    .preview { font-size: 14px; line-height: 1.38; max-height: 39px; overflow: hidden; }
    .meta { margin-top: 6px; color: var(--muted); font-size: 12px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .detail { min-height: 0; display: grid; grid-template-rows: auto 1fr auto; }
    .thread-head { padding: 12px; border-bottom: 1px solid var(--line); background: var(--panel); display: grid; gap: 10px; }
    .thread-title { font-weight: 700; line-height: 1.35; }
    .thread-meta { margin-top: 6px; color: var(--muted); font-size: 12px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .controls { display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: 8px; align-items: end; }
    .field { min-width: 0; display: grid; gap: 4px; color: var(--muted); font-size: 12px; }
    .field select { width: 100%; color: var(--text); font-size: 13px; }
    .messages { overflow: auto; padding: 12px; }
    .composer { display: grid; grid-template-columns: 1fr auto; gap: 8px; padding: 10px; border-top: 1px solid var(--line); background: var(--panel); }
    .composer textarea { width: 100%; min-height: 42px; max-height: 130px; resize: vertical; border: 1px solid var(--line); border-radius: 8px; padding: 10px 12px; background: var(--bg); color: var(--text); outline: none; line-height: 1.35; }
    .composer button { align-self: end; min-height: 42px; }
    .empty { color: var(--muted); padding: 18px; text-align: center; }
    .turn { margin: 0 0 12px; }
    .bubble { border: 1px solid var(--line); background: var(--bubble-agent); border-radius: 8px; padding: 10px 12px; white-space: pre-wrap; overflow-wrap: anywhere; font-size: 14px; line-height: 1.45; }
    .bubble.user { background: var(--bubble-user); }
    .role { margin: 0 0 4px; color: var(--muted); font-size: 12px; }
    .error { color: var(--danger); }
    .mobile-back { display: none; }
    @media (max-width: 760px) {
      body { overflow: hidden; }
      .layout { grid-template-columns: 1fr; }
      .sessions, .detail { min-width: 0; }
      .sessions.hidden, .detail.hidden { display: none; }
      .sessions { border-right: 0; }
      .mobile-back { display: inline-block; }
    }
  </style>
</head>
<body>
  <div class="app">
    <header class="topbar">
      <strong class="title">Codex++</strong>
      <span id="status" class="status">正在连接 WebSocket...</span>
    </header>
    <main class="layout">
      <section id="sessionsPane" class="sessions">
        <div class="search"><input id="filter" placeholder="搜索会话" autocomplete="off" /></div>
        <div id="sessions" class="list"></div>
      </section>
      <section id="detailPane" class="detail hidden">
        <div class="thread-head">
          <button id="back" class="mobile-back">返回</button>
          <div>
            <div id="threadTitle" class="thread-title">选择一个会话</div>
            <div id="threadMeta" class="thread-meta"></div>
          </div>
          <div class="controls">
            <label class="field">模型<select id="modelSelect"></select></label>
            <label class="field">思考<select id="effortSelect">
              <option value="">继承</option>
              <option value="low">低</option>
              <option value="medium">中</option>
              <option value="high">高</option>
            </select></label>
          </div>
        </div>
        <div id="messages" class="messages"><div class="empty">从左侧选择会话，或在项目目录里点 + 新建</div></div>
        <form id="composer" class="composer">
          <textarea id="messageInput" placeholder="输入消息，新建会话会作为首条消息发送" rows="1"></textarea>
          <button id="send" class="primary" type="submit">发送</button>
        </form>
      </section>
    </main>
  </div>
  <script>
    const MODEL_CATALOG = __MODEL_CATALOG_JSON__;
    const state = { sessions: [], selectedId: null, selectedCwd: "", filter: "", expandedProjects: new Set(), pendingMessages: new Map(), socket: null, pendingRpc: new Map(), initialized: false, reconnecting: false, streaming: null, thinking: null, modelOptions: [], selectedModel: "", selectedEffort: "" };
    let nextId = 1;
    const $ = (id) => document.getElementById(id);
    const statusEl = $("status");
    const sessionsEl = $("sessions");
    const messagesEl = $("messages");
    const titleEl = $("threadTitle");
    const metaEl = $("threadMeta");
    const sessionsPane = $("sessionsPane");
    const detailPane = $("detailPane");
    const modelSelect = $("modelSelect");
    const effortSelect = $("effortSelect");

    function setStatus(text, error = false) {
      statusEl.textContent = text;
      statusEl.classList.toggle("error", error);
    }

    async function ensureSocket() {
      if (state.socket?.readyState === WebSocket.OPEN && state.initialized) return state.socket;
      if (state.reconnecting) {
        await new Promise((resolve) => setTimeout(resolve, 150));
        return ensureSocket();
      }
      state.reconnecting = true;
      state.initialized = false;
      const scheme = location.protocol === "https:" ? "wss:" : "ws:";
      const socket = new WebSocket(`${scheme}//${location.host}/app-server/ws`);
      state.socket = socket;
      socket.addEventListener("message", onSocketMessage);
      socket.addEventListener("close", () => {
        state.initialized = false;
        for (const pending of state.pendingRpc.values()) pending.reject(new Error("连接已断开"));
        state.pendingRpc.clear();
      });
      try {
        await new Promise((resolve, reject) => {
          socket.addEventListener("open", resolve, { once: true });
          socket.addEventListener("error", () => reject(new Error("WebSocket 连接失败")), { once: true });
        });
        await rpcRaw("initialize", {
          clientInfo: { name: "Codex++ Mobile", version: "1.0.0" },
          capabilities: { experimentalApi: true }
        });
        state.initialized = true;
        return socket;
      } catch (error) {
        state.initialized = false;
        try { socket.close(); } catch {}
        throw error;
      } finally {
        state.reconnecting = false;
      }
    }

    function onSocketMessage(event) {
      let message;
      try { message = JSON.parse(event.data); } catch { return; }
      if (message.id != null) {
        const pending = state.pendingRpc.get(String(message.id));
        if (!pending) return;
        state.pendingRpc.delete(String(message.id));
        if (message.error) pending.reject(new Error(message.error.message || "请求失败"));
        else pending.resolve(message.result);
        return;
      }
      if (!message.method || !state.selectedId) return;
      const params = message.params || {};
      const threadId = eventThreadId(params);
      if (threadId && threadId !== state.selectedId) return;
      if (!threadId && !JSON.stringify(params).includes(state.selectedId)) return;

      if (message.method === "item/agentMessage/delta") {
        const delta = extractDeltaText(params);
        if (delta) {
          appendAgentDelta(params, delta);
          setStatus("正在接收回复...");
        }
        return;
      }

      if (message.method === "turn/started") {
        state.streaming = null;
        appendThinkingNode();
        setStatus("正在思考...");
        return;
      }

      if (message.method === "item/completed") {
        handleCompletedItem(params);
        setStatus("收到完整消息");
        return;
      }

      if (message.method === "turn/completed" || message.method === "thread/status/changed") {
        setStatus(`收到更新：${message.method}`);
      }
    }

    function eventThreadId(params) {
      return params.threadId || params.thread_id || params.thread?.id || params.turn?.threadId || params.item?.threadId || "";
    }

    function extractDeltaText(params) {
      const value = params.delta ?? params.text ?? params.chunk ?? params.content ?? params.item?.text ?? "";
      if (typeof value === "string") return value;
      if (Array.isArray(value)) {
        return value.map((part) => part?.text || part?.content || part?.delta || "").filter(Boolean).join("");
      }
      if (value && typeof value === "object") {
        return value.text || value.content || value.delta || value.value || "";
      }
      return "";
    }

    function appendAgentDelta(params, delta) {
      if (messagesEl.querySelector(".empty")) messagesEl.innerHTML = "";
      clearThinkingNode();
      const turnId = params.turnId || params.turn_id || params.turn?.id || "";
      const itemId = params.itemId || params.item_id || params.item?.id || "";
      const key = `${turnId}:${itemId}`;
      if (!state.streaming || state.streaming.key !== key) {
        const node = appendMessageNode("Codex", "", false);
        state.streaming = { key, node, bubble: node.querySelector(".bubble"), text: "" };
      }
      state.streaming.text += delta;
      state.streaming.bubble.textContent = state.streaming.text;
      messagesEl.scrollTop = messagesEl.scrollHeight;
    }

    function handleCompletedItem(params) {
      const item = params.item || {};
      const text = itemText(item);
      if (!text) return;
      const role = itemRole(item);
      if (role === "用户") {
        const confirmedUserTexts = new Map([[text, 1]]);
        reconcilePendingMessages(state.selectedId, confirmedUserTexts);
        confirmPendingMessageNode(text);
        return;
      }
      if (role !== "Codex") return;
      clearThinkingNode();
      if (state.streaming?.bubble) {
        state.streaming.text = text;
        state.streaming.bubble.textContent = text;
      } else {
        if (messagesEl.querySelector(".empty")) messagesEl.innerHTML = "";
        appendMessageNode("Codex", text, false);
      }
      messagesEl.scrollTop = messagesEl.scrollHeight;
    }

    async function rpc(method, params = {}) {
      await ensureSocket();
      return await rpcRaw(method, params);
    }

    async function rpcRaw(method, params = {}) {
      const socket = state.socket;
      if (!socket || socket.readyState !== WebSocket.OPEN) throw new Error("WebSocket 未连接");
      const id = nextId++;
      const payload = { jsonrpc: "2.0", id, method, params };
      const promise = new Promise((resolve, reject) => {
        state.pendingRpc.set(String(id), { resolve, reject });
        setTimeout(() => {
          if (state.pendingRpc.delete(String(id))) reject(new Error(`${method} 超时`));
        }, 60000);
      });
      socket.send(JSON.stringify(payload));
      return promise;
    }

    async function sendMessage(threadId, text, options = {}) {
      if (!options.skipResume) await rpc("thread/resume", { threadId });
      const params = {
        threadId,
        clientUserMessageId: `codex-plus-mobile-${Date.now()}`,
        input: [{ type: "text", text }]
      };
      if (state.selectedModel) params.model = state.selectedModel;
      if (state.selectedEffort) {
        params.modelReasoningEffort = state.selectedEffort;
        params.model_reasoning_effort = state.selectedEffort;
        params.reasoning = { effort: state.selectedEffort };
      }
      return await rpc("turn/start", params);
    }

    async function createThread(cwd) {
      const params = {};
      const cleanCwd = String(cwd || "").trim();
      if (cleanCwd) params.cwd = cleanCwd;
      if (state.selectedModel) params.model = state.selectedModel;
      if (state.selectedEffort) {
        params.modelReasoningEffort = state.selectedEffort;
        params.model_reasoning_effort = state.selectedEffort;
        params.reasoning = { effort: state.selectedEffort };
      }
      const result = await rpc("thread/start", params);
      const thread = result?.thread || result?.data || result;
      const threadId = thread?.id || result?.threadId || result?.id || "";
      if (!threadId) throw new Error("新建会话失败：app-server 未返回 thread id");
      const item = {
        id: threadId,
        preview: thread?.preview || thread?.name || "",
        name: thread?.name || "",
        cwd: thread?.cwd || result?.cwd || cleanCwd,
        modelProvider: thread?.modelProvider || result?.modelProvider || "",
        model: result?.model || thread?.model || "",
        createdAt: thread?.createdAt || Math.floor(Date.now() / 1000),
        updatedAt: thread?.updatedAt || Math.floor(Date.now() / 1000)
      };
      upsertSession(item);
      const key = normalizeProjectKey(item.cwd);
      state.expandedProjects.add(key);
      state.selectedId = threadId;
      state.selectedCwd = "";
      renderSessions();
      titleEl.textContent = item.preview || item.name || item.id;
      metaEl.textContent = `${item.modelProvider || ""} · ${item.cwd || ""}`;
      messagesEl.innerHTML = "";
      return item;
    }

    function upsertSession(item) {
      const index = state.sessions.findIndex((entry) => entry.id === item.id);
      if (index >= 0) state.sessions[index] = { ...state.sessions[index], ...item };
      else state.sessions.unshift(item);
      renderSessions();
    }

    async function loadSessions() {
      setStatus("正在加载会话...");
      const result = await rpc("thread/list", {});
      state.sessions = Array.isArray(result?.data) ? result.data : [];
      renderSessions();
      setStatus(`已加载 ${state.sessions.length} 个会话`);
    }

    function visibleSessions() {
      const filter = state.filter.trim().toLowerCase();
      if (!filter) return state.sessions;
      return state.sessions.filter((item) => {
        return [item.preview, item.cwd, item.id, item.modelProvider]
          .filter(Boolean)
          .join("\\n")
          .toLowerCase()
          .includes(filter);
      });
    }

    function renderSessions() {
      const items = visibleSessions();
      if (!items.length) {
        sessionsEl.innerHTML = `<div class="empty">没有会话</div>`;
        return;
      }
      sessionsEl.innerHTML = "";
      for (const group of groupSessionsByProject(items)) {
        const section = document.createElement("div");
        const collapsed = !state.expandedProjects.has(group.key);
        section.className = "group" + (collapsed ? " collapsed" : "");
        const title = document.createElement("div");
        title.className = "group-title";
        title.innerHTML = `<span class="chevron"></span><span class="group-name"></span><span class="group-count"></span><button class="group-new" type="button" title="新建会话">+</button>`;
        title.querySelector(".chevron").textContent = collapsed ? ">" : "v";
        title.querySelector(".group-name").textContent = group.label;
        title.querySelector(".group-count").textContent = String(group.items.length);
        title.title = group.cwd || group.label;
        title.addEventListener("click", () => toggleProject(group.key));
        title.querySelector(".group-new").addEventListener("click", (event) => {
          event.stopPropagation();
          newThreadInProject(group.cwd).catch((error) => setStatus(error.message, true));
        });
        section.appendChild(title);
        for (const item of group.items) {
          const button = document.createElement("button");
          button.className = "item" + (item.id === state.selectedId ? " active" : "");
          button.type = "button";
          button.innerHTML = `
            <div class="preview"></div>
            <div class="meta"></div>
          `;
          button.querySelector(".preview").textContent = item.preview || item.name || item.id;
          button.querySelector(".meta").textContent = `${formatTime(item.updatedAt || item.createdAt)} · ${item.modelProvider || "provider 未记录"}`;
          button.addEventListener("click", () => selectThread(item.id));
          section.appendChild(button);
        }
        sessionsEl.appendChild(section);
      }
    }

    function toggleProject(key) {
      if (state.expandedProjects.has(key)) state.expandedProjects.delete(key);
      else state.expandedProjects.add(key);
      renderSessions();
    }

    async function newThreadInProject(cwd) {
      state.selectedId = null;
      state.selectedCwd = String(cwd || "").trim();
      renderSessions();
      titleEl.textContent = "新建会话";
      metaEl.textContent = state.selectedCwd || "未知目录";
      messagesEl.innerHTML = `<div class="empty">输入第一条消息后发送</div>`;
      if (window.matchMedia("(max-width: 760px)").matches) {
        sessionsPane.classList.add("hidden");
        detailPane.classList.remove("hidden");
      }
      $("messageInput").focus();
    }

    function groupSessionsByProject(items) {
      const groups = [];
      const seen = new Map();
      for (const item of items) {
        const key = normalizeProjectKey(item.cwd);
        let group = seen.get(key);
        if (!group) {
          group = { key, label: projectLabel(item.cwd), cwd: item.cwd || "", items: [] };
          seen.set(key, group);
          groups.push(group);
        }
        group.items.push(item);
      }
      return groups;
    }

    function normalizeProjectKey(cwd) {
      return String(cwd || "").trim().toLowerCase() || "__unknown__";
    }

    function projectLabel(cwd) {
      const value = String(cwd || "").trim();
      if (!value) return "未知目录";
      return value.split(/[\\\\/]/).filter(Boolean).pop() || value;
    }

    async function selectThread(threadId) {
      state.selectedId = threadId;
      state.selectedCwd = "";
      renderSessions();
      if (window.matchMedia("(max-width: 760px)").matches) {
        sessionsPane.classList.add("hidden");
        detailPane.classList.remove("hidden");
      }
      const item = state.sessions.find((entry) => entry.id === threadId);
      titleEl.textContent = item?.preview || item?.name || threadId;
      metaEl.textContent = `${item?.modelProvider || ""} · ${item?.cwd || ""}`;
      syncControlsForThread(item);
      messagesEl.innerHTML = `<div class="empty">正在通过 WebSocket 同步会话...</div>`;
      await refreshThread(threadId, item);
    }

    async function refreshThread(threadId, fallbackItem = null) {
      const item = fallbackItem || state.sessions.find((entry) => entry.id === threadId);
      const resumePromise = rpc("thread/resume", { threadId });
      const turnsPromise = rpc("thread/turns/list", { threadId });
      try {
        const turnsValue = await turnsPromise;
        const threadFromTurns = extractThread(turnsValue);
        const turns = extractTurns(turnsValue) || extractTurns(threadFromTurns);
        renderThread(threadFromTurns || item, normalizeTurns(turns), threadId);
        setStatus("会话内容已加载");
      } catch (error) {
        messagesEl.innerHTML = `<div class="empty error"></div>`;
        messagesEl.querySelector(".error").textContent = `消息列表不可用：${error.message}`;
        setStatus(`消息列表不可用：${error.message}`, true);
      }
      try {
        const resumeValue = await resumePromise;
        const thread = extractThread(resumeValue);
        if (thread && threadId === state.selectedId) {
          titleEl.textContent = thread.preview || thread.name || thread.id || "会话";
          metaEl.textContent = `${formatTime(thread.updatedAt || thread.createdAt)} · ${thread.cwd || ""}`;
        }
        setStatus("会话已打开");
      } catch (error) {
        setStatus(`会话内容已加载，打开状态同步失败：${error.message}`, true);
      }
    }

    function renderThread(thread, turns, threadId = state.selectedId) {
      if (thread) {
        titleEl.textContent = thread.preview || thread.name || thread.id || "会话";
        metaEl.textContent = `${formatTime(thread.updatedAt || thread.createdAt)} · ${thread.cwd || ""}`;
      }
      const pending = pendingMessagesFor(threadId);
      if (!turns.length) {
        messagesEl.innerHTML = "";
        if (pending.length) {
          for (const message of pending) appendMessageNode("用户", message.text, true);
          messagesEl.scrollTop = messagesEl.scrollHeight;
        } else {
          messagesEl.innerHTML = `<div class="empty">这个会话暂时没有可显示的消息</div>`;
        }
        return;
      }
      messagesEl.innerHTML = "";
      const confirmedUserTexts = new Map();
      for (const turn of turns) {
        const items = turnItems(turn);
        for (const item of items) {
          const text = itemText(item);
          if (!text) continue;
          const role = itemRole(item);
          if (role === "用户") {
            confirmedUserTexts.set(text, (confirmedUserTexts.get(text) || 0) + 1);
          }
          appendMessageNode(role, text, false);
        }
      }
      reconcilePendingMessages(threadId, confirmedUserTexts);
      for (const message of pendingMessagesFor(threadId)) appendMessageNode("用户", message.text, true);
      if (!messagesEl.children.length) {
        messagesEl.innerHTML = `<div class="empty">没有文本消息</div>`;
      }
      messagesEl.scrollTop = messagesEl.scrollHeight;
    }

    function turnItems(turn) {
      if (!turn || typeof turn !== "object") return [turn];
      if (Array.isArray(turn.items)) return turn.items;
      if (Array.isArray(turn.messages)) return turn.messages;
      const items = [];
      if (turn.input != null) items.push({ type: "userMessage", content: turn.input });
      if (turn.output != null) items.push({ type: "agentMessage", content: turn.output });
      if (turn.request != null) items.push({ type: "userMessage", content: turn.request });
      if (turn.response != null) items.push({ type: "agentMessage", content: turn.response });
      return items.length ? items : [turn];
    }

    function extractThread(value) {
      if (!value || typeof value !== "object") return null;
      return value.thread || value.data?.thread || value.result?.thread || value.conversation || null;
    }

    function extractTurns(value) {
      if (!value) return null;
      if (Array.isArray(value)) return value;
      if (Array.isArray(value.data)) return value.data;
      if (Array.isArray(value.turns)) return value.turns;
      if (Array.isArray(value.items)) return [{ items: value.items, createdAt: value.createdAt, updatedAt: value.updatedAt }];
      if (Array.isArray(value.messages)) return value.messages;
      if (Array.isArray(value.thread?.turns)) return value.thread.turns;
      if (Array.isArray(value.thread?.items)) return [{ items: value.thread.items, createdAt: value.thread.createdAt, updatedAt: value.thread.updatedAt }];
      if (Array.isArray(value.data?.turns)) return value.data.turns;
      if (Array.isArray(value.data?.items)) return [{ items: value.data.items, createdAt: value.data.createdAt, updatedAt: value.data.updatedAt }];
      if (Array.isArray(value.conversation?.turns)) return value.conversation.turns;
      if (Array.isArray(value.conversation?.items)) return [{ items: value.conversation.items, createdAt: value.conversation.createdAt, updatedAt: value.conversation.updatedAt }];
      return null;
    }

    function normalizeTurns(turns) {
      if (!Array.isArray(turns)) return [];
      return [...turns].sort((left, right) => turnTimestamp(left) - turnTimestamp(right));
    }

    function turnTimestamp(turn) {
      const value = turn?.startedAt || turn?.createdAt || turn?.completedAt || 0;
      return value < 100000000000 ? value * 1000 : value;
    }

    function itemRole(item) {
      const raw = String(item?.role || item?.author?.role || item?.message?.role || item?.item?.role || item?.type || "").toLowerCase();
      if (raw === "user" || raw === "usermessage" || raw === "input_text" || raw === "input") return "用户";
      if (raw === "assistant" || raw === "agent" || raw === "codex" || raw === "agentmessage" || raw === "assistantmessage" || raw === "output_text" || raw === "output") return "Codex";
      if (raw === "toolcall" || raw === "tool_call" || raw === "function_call") return "工具";
      if (raw === "toolresult" || raw === "tool_result" || raw === "function_call_output") return "工具结果";
      return item?.type || item?.role || "消息";
    }

    function itemText(item) {
      const text = extractText(item, 0);
      return typeof text === "string" ? text.trim() : "";
    }

    function extractText(value, depth) {
      if (value == null || depth > 6) return "";
      if (typeof value === "string") return value;
      if (Array.isArray(value)) return value.map((part) => extractText(part, depth + 1)).filter(Boolean).join("\\n");
      if (typeof value !== "object") return "";
      for (const key of ["text", "output_text", "input_text", "markdown", "value", "delta", "content", "message", "output", "input", "parts", "payload", "item", "data"]) {
        if (value[key] == null) continue;
        const text = extractText(value[key], depth + 1);
        if (text) return text;
      }
      return "";
    }

    function formatTime(value) {
      if (!value) return "未知时间";
      const ms = value < 100000000000 ? value * 1000 : value;
      return new Date(ms).toLocaleString();
    }

    function initControls() {
      const models = Array.isArray(MODEL_CATALOG?.models) ? MODEL_CATALOG.models.filter(Boolean) : [];
      state.modelOptions = [...new Set(models.map(String))];
      state.selectedModel = MODEL_CATALOG?.default_model || MODEL_CATALOG?.model || state.modelOptions[0] || "";
      renderModelSelect();
      effortSelect.value = state.selectedEffort;
    }

    function renderModelSelect() {
      modelSelect.innerHTML = "";
      if (!state.modelOptions.length && !state.selectedModel) {
        const option = document.createElement("option");
        option.value = "";
        option.textContent = "继承当前配置";
        modelSelect.appendChild(option);
        modelSelect.value = "";
        return;
      }
      const inherit = document.createElement("option");
      inherit.value = "";
      inherit.textContent = "继承当前配置";
      modelSelect.appendChild(inherit);
      for (const model of state.modelOptions) {
        const option = document.createElement("option");
        option.value = model;
        option.textContent = model;
        modelSelect.appendChild(option);
      }
      if (state.selectedModel && !state.modelOptions.includes(state.selectedModel)) {
        const option = document.createElement("option");
        option.value = state.selectedModel;
        option.textContent = state.selectedModel;
        modelSelect.appendChild(option);
      }
      modelSelect.value = state.selectedModel;
    }

    function syncControlsForThread(item) {
      const model = item?.model || item?.modelId || item?.modelName || "";
      if (model && !state.selectedModel) {
        state.selectedModel = model;
        renderModelSelect();
      }
    }

    function appendThinkingNode() {
      if (state.thinking?.isConnected) return state.thinking;
      if (messagesEl.querySelector(".empty")) messagesEl.innerHTML = "";
      const node = appendMessageNode("Codex", "正在思考...", false);
      node.dataset.thinking = "true";
      state.thinking = node;
      messagesEl.scrollTop = messagesEl.scrollHeight;
      return node;
    }

    function clearThinkingNode() {
      if (state.thinking?.isConnected) state.thinking.remove();
      state.thinking = null;
    }

    modelSelect.addEventListener("change", (event) => {
      state.selectedModel = event.target.value;
    });
    effortSelect.addEventListener("change", (event) => {
      state.selectedEffort = event.target.value;
    });
    $("filter").addEventListener("input", (event) => {
      state.filter = event.target.value;
      renderSessions();
    });
    $("back").addEventListener("click", () => {
      detailPane.classList.add("hidden");
      sessionsPane.classList.remove("hidden");
    });
    $("composer").addEventListener("submit", async (event) => {
      event.preventDefault();
      const input = $("messageInput");
      const text = input.value.trim();
      if (!text) return;
      const button = $("send");
      button.disabled = true;
      input.disabled = true;
      setStatus("正在发送...");
      let targetThreadId = state.selectedId;
      let isNewThread = false;
      try {
        if (!targetThreadId) {
          if (!state.selectedCwd) throw new Error("请先选择会话，或在项目目录里点 + 新建");
          setStatus("正在新建会话...");
          const thread = await createThread(state.selectedCwd);
          targetThreadId = thread.id;
          isNewThread = true;
        }
        rememberPendingMessage(targetThreadId, text);
        appendLocalMessage("用户", text, true);
        appendThinkingNode();
        await sendMessage(targetThreadId, text, { skipResume: isNewThread });
        input.value = "";
        setStatus("已发送，正在思考...");
      } catch (error) {
        forgetPendingMessage(targetThreadId, text);
        removePendingMessageNode(text);
        clearThinkingNode();
        setStatus(error.message, true);
      } finally {
        button.disabled = false;
        input.disabled = false;
        input.focus();
      }
    });

    function appendLocalMessage(role, text, pending = false) {
      if (messagesEl.querySelector(".empty")) messagesEl.innerHTML = "";
      appendMessageNode(role, text, pending);
      messagesEl.scrollTop = messagesEl.scrollHeight;
    }

    function appendMessageNode(role, text, pending = false) {
      const wrap = document.createElement("div");
      wrap.className = "turn";
      wrap.innerHTML = `<div class="role"></div><div class="bubble"></div>`;
      wrap.querySelector(".role").textContent = pending ? `${role} · 待同步` : role;
      const bubble = wrap.querySelector(".bubble");
      bubble.classList.toggle("user", role === "用户");
      bubble.textContent = text;
      messagesEl.appendChild(wrap);
      return wrap;
    }

    function confirmPendingMessageNode(text) {
      for (const node of messagesEl.querySelectorAll(".turn")) {
        const role = node.querySelector(".role");
        const bubble = node.querySelector(".bubble");
        if (role?.textContent === "用户 · 待同步" && bubble?.textContent === text) {
          role.textContent = "用户";
          return;
        }
      }
    }

    function removePendingMessageNode(text) {
      for (const node of messagesEl.querySelectorAll(".turn")) {
        const role = node.querySelector(".role");
        const bubble = node.querySelector(".bubble");
        if (role?.textContent === "用户 · 待同步" && bubble?.textContent === text) {
          node.remove();
          return;
        }
      }
    }

    function rememberPendingMessage(threadId, text) {
      const list = pendingMessagesFor(threadId);
      list.push({ text, createdAt: Date.now() });
      state.pendingMessages.set(threadId, list);
    }

    function pendingMessagesFor(threadId) {
      if (!threadId) return [];
      return state.pendingMessages.get(threadId) || [];
    }

    function forgetPendingMessage(threadId, text) {
      const remaining = pendingMessagesFor(threadId).filter((message) => message.text !== text);
      if (remaining.length) state.pendingMessages.set(threadId, remaining);
      else state.pendingMessages.delete(threadId);
    }

    function reconcilePendingMessages(threadId, confirmedUserTexts = new Map()) {
      const pending = pendingMessagesFor(threadId);
      if (!pending.length) return;
      const remaining = pending.filter((message) => {
        const expired = Date.now() - message.createdAt > 120000;
        const confirmedCount = confirmedUserTexts.get(message.text) || 0;
        if (confirmedCount > 0) {
          confirmedUserTexts.set(message.text, confirmedCount - 1);
          return false;
        }
        return !expired;
      });
      if (remaining.length) state.pendingMessages.set(threadId, remaining);
      else state.pendingMessages.delete(threadId);
    }

    initControls();
    loadSessions().catch((error) => setStatus(error.message, true));
  </script>
</body>
</html>"#;
    html.replace(
        "__MODEL_CATALOG_JSON__",
        &script_safe_json(model_catalog_json),
    )
}

fn script_safe_json(json: &str) -> String {
    json.replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

async fn handle_models_proxy_connection(
    stream: &mut tokio::net::TcpStream,
    request_user_agent: Option<&str>,
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

    let upstream = match crate::protocol_proxy::open_models_proxy_request(request_user_agent).await
    {
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
    request_user_agent: Option<&str>,
    method: &str,
    path: &str,
    remote_addr_text: Option<String>,
) -> anyhow::Result<()> {
    let request_json = serde_json::from_str::<serde_json::Value>(request_body).ok();
    let upstream =
        match crate::protocol_proxy::open_responses_proxy_request(request_body, request_user_agent)
            .await
        {
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

    if !upstream.is_success() {
        let status = upstream.status();
        let upstream_content_type = upstream.content_type.clone();
        let upstream_body = upstream.response.bytes().await?.to_vec();
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
        if upstream.wire_api == crate::protocol_proxy::UpstreamWireApi::Responses {
            let mut bytes_stream = upstream.response.bytes_stream();
            while let Some(chunk) = bytes_stream.next().await {
                if let Ok(bytes) = chunk {
                    stream.write_all(&bytes).await?;
                } else {
                    break;
                }
            }
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

        let mut converter = request_json
            .as_ref()
            .map(crate::protocol_proxy::ChatSseToResponsesConverter::with_request)
            .unwrap_or_default();
        let mut bytes_stream = upstream.response.bytes_stream();
        let mut stream_failed = false;

        while let Some(chunk) = bytes_stream.next().await {
            match chunk {
                Ok(bytes) => {
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
    if upstream.wire_api == crate::protocol_proxy::UpstreamWireApi::Responses {
        write_http_response(
            stream,
            "200 OK",
            if upstream.content_type.is_empty() {
                "application/json; charset=utf-8"
            } else {
                &upstream.content_type
            },
            &upstream_body,
        )
        .await?;
        log_helper_response(
            "helper.protocol_proxy_ok",
            method,
            path,
            "200 OK",
            remote_addr_text,
        );
        stream.shutdown().await?;
        return Ok(());
    }

    let chat_json: serde_json::Value = serde_json::from_slice(&upstream_body)?;
    let response_json = if let Some(request_json) = request_json.as_ref() {
        crate::protocol_proxy::chat_completion_to_response_with_request(chat_json, request_json)?
    } else {
        crate::protocol_proxy::chat_completion_to_response(chat_json)?
    };
    let body = serde_json::to_vec(&response_json)?;
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
    request_user_agent: Option<&str>,
    method: &str,
    path: &str,
    remote_addr_text: Option<String>,
) -> anyhow::Result<()> {
    let upstream = match crate::protocol_proxy::open_chat_completions_proxy_request(
        request_body,
        request_user_agent,
    )
    .await
    {
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

async fn write_http_no_store_response(
    stream: &mut tokio::net::TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
) -> anyhow::Result<()> {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nCache-Control: no-store\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type, Authorization\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    stream.write_all(body).await?;
    Ok(())
}

async fn write_options_response(stream: &mut tokio::net::TcpStream) -> anyhow::Result<()> {
    stream
        .write_all(
            b"HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type, Authorization\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )
        .await?;
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

#[cfg(test)]
mod computer_use_tests {
    use super::{
        MobileRelayHostConfig, header_value_from_request, overlay_image_content_type,
        percent_encode_query,
    };
    use std::path::Path;

    #[test]
    fn overlay_image_content_type_accepts_common_images_only() {
        assert_eq!(
            overlay_image_content_type(Path::new("overlay.PNG")),
            Some("image/png")
        );
        assert_eq!(
            overlay_image_content_type(Path::new("overlay.jpeg")),
            Some("image/jpeg")
        );
        assert_eq!(
            overlay_image_content_type(Path::new("overlay.webp")),
            Some("image/webp")
        );
        assert_eq!(overlay_image_content_type(Path::new("overlay.txt")), None);
    }

    #[test]
    fn header_value_from_request_reads_user_agent_case_insensitively() {
        let request = "POST /v1/chat/completions HTTP/1.1\r\nHost: 127.0.0.1\r\nUser-Agent: Codex/26.614\r\nContent-Length: 2\r\n\r\n{}";

        assert_eq!(
            header_value_from_request(request, "user-agent").as_deref(),
            Some("Codex/26.614")
        );
    }

    #[test]
    fn mobile_relay_host_url_appends_host_path_and_credentials() {
        let config = MobileRelayHostConfig {
            relay_url: "ws://example.test:57323".to_string(),
            room: "项目 A".to_string(),
            token: "a+b&c".to_string(),
            encryption_key: "test-key".to_string(),
        };
        assert_eq!(
            config.host_url(),
            "ws://example.test:57323/host?room=%E9%A1%B9%E7%9B%AE%20A&token=a%2Bb%26c"
        );
    }

    #[test]
    fn mobile_relay_percent_encode_keeps_url_safe_bytes() {
        assert_eq!(percent_encode_query("abc-._~ 1+2"), "abc-._~%201%2B2");
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

fn header_value_from_request(request: &str, header_name: &str) -> Option<String> {
    request
        .split_once("\r\n\r\n")
        .map(|(headers, _)| headers)
        .unwrap_or(request)
        .lines()
        .skip(1)
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case(header_name)
                .then(|| value.trim().to_string())
        })
        .filter(|value| !value.is_empty())
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
    let target = crate::cdp::pick_injectable_codex_page_target(&targets)?;
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
    let target = crate::cdp::pick_injectable_codex_page_target(&targets)?;
    let websocket_url = target
        .web_socket_debugger_url
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("selected CDP target has no websocket URL"))?;
    let settings = SettingsStore::default().load().unwrap_or_default();
    let script = crate::assets::injection_script_with_settings(helper_port, &settings);
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

pub fn build_macos_open_command(
    app_dir: &Path,
    debug_port: u16,
    extra_args: &[String],
) -> Vec<String> {
    let mut command = vec![
        "open".to_string(),
        "-W".to_string(),
        "-a".to_string(),
        app_dir.to_string_lossy().to_string(),
        "--args".to_string(),
    ];
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
    let app_index = command.iter().position(|part| part == "-a")?;
    command.get(app_index + 1).map(PathBuf::from)
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

#[cfg_attr(not(windows), allow(dead_code))]
fn post_launch_guard_artifacts_ready(
    artifacts: &crate::computer_use_guard::GuardArtifacts,
) -> bool {
    artifacts.notify_exe.is_some()
        && artifacts.marketplace_path.is_some()
        && (!artifacts.runtime_exports_needed || artifacts.sky_package_json.is_some())
}

#[cfg_attr(not(windows), allow(dead_code))]
fn should_stop_post_launch_computer_use_guard(
    stable_unchanged_attempts: usize,
    artifacts: &crate::computer_use_guard::GuardArtifacts,
) -> bool {
    stable_unchanged_attempts >= POST_LAUNCH_COMPUTER_USE_GUARD_STABLE_ATTEMPTS
        && post_launch_guard_artifacts_ready(artifacts)
}

#[cfg(windows)]
async fn run_post_launch_computer_use_guard(
    home: PathBuf,
    mut artifacts: Option<crate::computer_use_guard::GuardArtifacts>,
    shutdown_rx: &mut tokio::sync::oneshot::Receiver<()>,
) {
    let mut previous_delay = 0_u64;
    let mut stable_unchanged_attempts = 0_usize;
    for (index, delay) in POST_LAUNCH_COMPUTER_USE_GUARD_SECONDS
        .iter()
        .copied()
        .enumerate()
    {
        let wait_seconds = delay.saturating_sub(previous_delay);
        previous_delay = delay;
        if wait_seconds > 0 {
            tokio::select! {
                _ = &mut *shutdown_rx => return,
                _ = tokio::time::sleep(std::time::Duration::from_secs(wait_seconds)) => {}
            }
        }
        let attempt = index + 1;
        let resolved_artifacts = match artifacts.take() {
            Some(artifacts) => artifacts,
            None => match crate::computer_use_guard::resolve_computer_use_guard_artifacts(&home) {
                Ok(resolved) => resolved,
                Err(error) => {
                    stable_unchanged_attempts = 0;
                    let _ = crate::diagnostic_log::append_diagnostic_log(
                        "computer_use_guard.post_launch_failed",
                        serde_json::json!({
                            "attempt": attempt,
                            "delay_seconds": delay,
                            "phase": "resolve_artifacts",
                            "message": error.to_string()
                        }),
                    );
                    continue;
                }
            },
        };
        let artifacts_ready = post_launch_guard_artifacts_ready(&resolved_artifacts);
        artifacts = artifacts_ready.then_some(resolved_artifacts.clone());
        match crate::computer_use_guard::ensure_computer_use_config_with_artifacts(
            &home,
            &resolved_artifacts,
        ) {
            Ok(result) => {
                if !result.changed && artifacts_ready {
                    stable_unchanged_attempts += 1;
                } else {
                    stable_unchanged_attempts = 0;
                }
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "computer_use_guard.post_launch_ok",
                    serde_json::json!({
                        "attempt": attempt,
                        "delay_seconds": delay,
                        "changed": result.changed,
                        "stable_unchanged_attempts": stable_unchanged_attempts,
                        "notify_exe": result
                            .notify_exe
                            .map(|path| path.to_string_lossy().to_string())
                    }),
                );
                if should_stop_post_launch_computer_use_guard(
                    stable_unchanged_attempts,
                    &resolved_artifacts,
                ) {
                    let _ = crate::diagnostic_log::append_diagnostic_log(
                        "computer_use_guard.post_launch_stable_stop",
                        serde_json::json!({
                            "attempt": attempt,
                            "delay_seconds": delay,
                            "stable_unchanged_attempts": stable_unchanged_attempts
                        }),
                    );
                    return;
                }
            }
            Err(error) => {
                stable_unchanged_attempts = 0;
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "computer_use_guard.post_launch_failed",
                    serde_json::json!({
                        "attempt": attempt,
                        "delay_seconds": delay,
                        "message": error.to_string()
                    }),
                );
            }
        }
    }
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
    fn post_launch_guard_stops_after_stable_ready_artifacts() {
        let artifacts = crate::computer_use_guard::GuardArtifacts {
            notify_exe: Some(PathBuf::from("codex-computer-use.exe")),
            marketplace_path: Some(PathBuf::from("openai-bundled")),
            sky_package_json: None,
            runtime_exports_needed: false,
        };

        assert!(!should_stop_post_launch_computer_use_guard(2, &artifacts));
        assert!(should_stop_post_launch_computer_use_guard(3, &artifacts));
    }

    #[test]
    fn post_launch_guard_keeps_retrying_until_artifacts_are_ready() {
        let missing_notify = crate::computer_use_guard::GuardArtifacts {
            notify_exe: None,
            marketplace_path: Some(PathBuf::from("openai-bundled")),
            sky_package_json: None,
            runtime_exports_needed: false,
        };
        let missing_marketplace = crate::computer_use_guard::GuardArtifacts {
            notify_exe: Some(PathBuf::from("codex-computer-use.exe")),
            marketplace_path: None,
            sky_package_json: None,
            runtime_exports_needed: false,
        };
        let missing_runtime_package = crate::computer_use_guard::GuardArtifacts {
            notify_exe: Some(PathBuf::from("codex-computer-use.exe")),
            marketplace_path: Some(PathBuf::from("openai-bundled")),
            sky_package_json: None,
            runtime_exports_needed: true,
        };

        assert!(!should_stop_post_launch_computer_use_guard(
            3,
            &missing_notify
        ));
        assert!(!should_stop_post_launch_computer_use_guard(
            3,
            &missing_marketplace
        ));
        assert!(!should_stop_post_launch_computer_use_guard(
            3,
            &missing_runtime_package
        ));
    }
}
