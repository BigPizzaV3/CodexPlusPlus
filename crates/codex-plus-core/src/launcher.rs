use std::future::Future;
use std::io::Read;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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

static PET_OVERLAY_SYNC_FAILED: AtomicBool = AtomicBool::new(false);
#[cfg(any(windows, target_os = "macos"))]
static PET_CURSOR_DRIVER_FAILED: AtomicBool = AtomicBool::new(false);
const BRIDGE_HEALTH_FAILURE_THRESHOLD: u8 = 2;
/// 重注入退避：首次重注入立即执行，此后若桥仍不健康，重注入间隔按
/// 10s → 20s → 40s … 指数增长并封顶。对重注入修不好的失效（如应用更新
/// 改变了页面上下文），避免每 10~15 秒重发一次完整注入脚本（issue #2169）。
const BRIDGE_REINJECT_BACKOFF_BASE_SECS: u64 = 10;
const BRIDGE_REINJECT_BACKOFF_CAP_SECS: u64 = 300;
/// 重注入后紧接着的那次健康检查必然通过（`lastInjectionAt` 还在 5 秒窗口内），
/// 若一见健康就清零，退避会永远停在第一档、根本挡不住周期性重注入（issue #2330）。
/// 必须持续健康满这么久才清零。
const BRIDGE_REINJECT_BACKOFF_RESET_AFTER_SECS: u64 = 60;

/// 看门狗内跟踪重注入退避状态；持续健康一段时间或应用实例更换时重置。
#[derive(Debug, Default)]
struct BridgeReinjectBackoff {
    consecutive_attempts: u32,
    next_allowed_at: Option<std::time::Instant>,
    healthy_since: Option<std::time::Instant>,
}

fn reinject_backoff_delay(consecutive_attempts: u32) -> std::time::Duration {
    let factor = 1u64
        .checked_shl(consecutive_attempts.min(u64::BITS - 1))
        .unwrap_or(u64::MAX);
    std::time::Duration::from_secs(
        BRIDGE_REINJECT_BACKOFF_BASE_SECS
            .saturating_mul(factor)
            .min(BRIDGE_REINJECT_BACKOFF_CAP_SECS),
    )
}

impl BridgeReinjectBackoff {
    fn ready(&self, now: std::time::Instant) -> bool {
        self.next_allowed_at.is_none_or(|deadline| now >= deadline)
    }

    fn record_attempt(&mut self, now: std::time::Instant) {
        let delay = reinject_backoff_delay(self.consecutive_attempts);
        self.next_allowed_at = Some(now + delay);
        self.consecutive_attempts = self.consecutive_attempts.saturating_add(1);
    }

    fn reset(&mut self) {
        self.consecutive_attempts = 0;
        self.next_allowed_at = None;
        self.healthy_since = None;
    }

    /// 观测到一次健康：持续健康满 `BRIDGE_REINJECT_BACKOFF_RESET_AFTER_SECS` 才清零。
    fn observe_healthy(&mut self, now: std::time::Instant) {
        let since = *self.healthy_since.get_or_insert(now);
        if now.saturating_duration_since(since)
            >= std::time::Duration::from_secs(BRIDGE_REINJECT_BACKOFF_RESET_AFTER_SECS)
        {
            self.reset();
        }
    }

    /// 观测到一次不健康：重新计时。
    fn observe_unhealthy(&mut self) {
        self.healthy_since = None;
    }
}
const MACOS_DEBUG_TAKEOVER_WAIT_MS: u64 = 5_000;
const MACOS_DEBUG_TAKEOVER_INTERVAL_MS: u64 = 100;

/// 协议代理的端口写死在 `config.toml` 的 `base_url = "http://127.0.0.1:57321/v1"` 里，
/// 不能像普通 helper 端口那样临时换一个空闲的，否则 Codex CLI 会连到没人监听的地址。
/// 而管理器的「重启」是先强杀旧 launcher 再拉新的，旧 helper 交还监听要一小会儿；
/// 过去这里一次 bind 失败就整个启动中止，用户侧就是重启必失败、直接双击 exe 反而正常（issue #1933）。
/// 所以固定端口下给前任一个让位的窗口。端口被占用，以及 Windows 在旧进程
/// 退出瞬间暂时返回的 10013，都放在这一段里重试。窗口结束仍是 10013，
/// 才当成端口被系统永久保留。
const HELPER_BIND_RETRY_TIMEOUT_MS: u64 = 6_000;
const PROTOCOL_PROXY_BIND_RETRY_TIMEOUT_MS: u64 = 10_000;
const HELPER_BIND_RETRY_INTERVAL_MS: u64 = 200;

/// Asynchronous callback used by the bridge watchdog to restore a launcher-specific bridge.
///
/// Callers that install a custom [`crate::routes::BridgeContext`] should configure this callback
/// before starting the watchdog. Without one, the watchdog falls back to the core bridge injector.
pub type BridgeReinjector =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>> + Send + Sync>;

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
pub enum MacosDebugLaunchAction {
    LaunchNew,
    ReuseRunningDebugApp,
    RestartRunningApp,
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
    /// 启动命令携带的最终端口；包上下文 wrapper 可能改选过端口。
    pub fn debug_port(&self) -> Option<u16> {
        let tokens = match self {
            Self::Process { command, .. } => command.iter().map(String::as_str).collect::<Vec<_>>(),
            Self::PackagedActivation { arguments, .. } => arguments.split_whitespace().collect(),
        };
        let mut ports = tokens.iter().enumerate().filter_map(|(index, token)| {
            token
                .strip_prefix("--remote-debugging-port=")
                .or_else(|| {
                    (*token == "--remote-debugging-port")
                        .then(|| tokens.get(index + 1).copied())
                        .flatten()
                })
                .and_then(|value| value.parse::<u16>().ok())
        });
        let first = ports.next()?;
        (first != 0 && ports.all(|port| port == first)).then_some(first)
    }

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
    /// 本次启动使用的 Codex++ 状态目录（多开实例隔离用）。
    ///
    /// 多开原先只靠给 worker 换 `HOME`，但 Windows 上 `directories` 走
    /// `SHGetKnownFolderPath`、不读 `HOME`，导致实例会读写主实例的状态文件。
    /// 这里显式传路径：既用于子进程环境（`CODEX_PLUS_STATE_DIR`），也用于
    /// Windows 包上下文启动时选择 wrapper 落点。
    pub state_dir: Option<PathBuf>,
    /// Codex 配置目录（`CODEX_HOME`），多开实例各自独立；用于包上下文 wrapper。
    pub codex_home: Option<PathBuf>,
    /// Codex SQLite 目录（`CODEX_SQLITE_HOME`），多开实例各自独立。
    pub sqlite_home: Option<PathBuf>,
}

impl Default for LaunchOptions {
    fn default() -> Self {
        Self {
            app_dir: None,
            debug_port: 9229,
            helper_port: 57321,
            status_store: StatusStore::default(),
            state_dir: None,
            codex_home: None,
            sqlite_home: None,
        }
    }
}

/// 本次启动所属的实例隔离上下文（多开用；单实例启动时全是 `None`）。
///
/// 刻意用参数传递而不是进程环境：多开 worker 复用管理器进程且已是多线程，
/// edition 2024 下 `set_var` 属未定义行为；状态目录这种决定「数据写到哪」的
/// 关键输入也不该走全局可变状态。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaunchInstance {
    /// Codex++ 状态目录（`settings.json` / `latest-status.json` / 诊断日志）。
    pub state_dir: Option<PathBuf>,
    /// Codex 配置目录（`CODEX_HOME`）。
    pub codex_home: Option<PathBuf>,
    /// Codex SQLite 目录（`CODEX_SQLITE_HOME`）。
    pub sqlite_home: Option<PathBuf>,
}

impl LaunchInstance {
    pub fn from_options(options: &LaunchOptions) -> Self {
        Self {
            state_dir: options.state_dir.clone(),
            codex_home: options.codex_home.clone(),
            sqlite_home: options.sqlite_home.clone(),
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
    status_started_at_ms: u64,
    helper_started: bool,
    isolated_quota_gate: Option<Arc<IsolatedQuotaGateRuntime>>,
    hooks: Arc<dyn LaunchHooks>,
}

struct IsolatedQuotaGateRuntime {
    shutdown: tokio::sync::watch::Sender<bool>,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl IsolatedQuotaGateRuntime {
    fn start(debug_port: u16) -> Arc<Self> {
        let (shutdown, receiver) = tokio::sync::watch::channel(false);
        let task = tokio::spawn(crate::bridge::supervise_isolated_api_quota_gate(
            debug_port, receiver,
        ));
        Arc::new(Self {
            shutdown,
            task: Mutex::new(Some(task)),
        })
    }

    async fn stop(&self) {
        self.shutdown.send_replace(true);
        let mut owned_task = self.task.lock().await;
        if let Some(mut task) = owned_task.take() {
            // CDP 清理有自己的超时；最后兜底等待并终止任务，不能留下端口重连。
            if tokio::time::timeout(std::time::Duration::from_secs(4), &mut task)
                .await
                .is_err()
            {
                task.abort();
                let _ = task.await;
            }
        }
    }
}

impl Drop for IsolatedQuotaGateRuntime {
    fn drop(&mut self) {
        // 最后一个会话 handle 被释放时也必须取消，包括等待 future 被提前丢弃。
        self.shutdown.send_replace(true);
    }
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
    pub async fn stop_isolated_api_quota_gate(&self) {
        if let Some(runtime) = &self.isolated_quota_gate {
            runtime.stop().await;
        }
    }

    pub async fn wait_for_codex_exit(&self) -> anyhow::Result<()> {
        let result = self
            .hooks
            .wait_for_codex_exit(&self.launch, self.debug_port)
            .await;
        self.stop_isolated_api_quota_gate().await;
        if self.helper_started {
            self.hooks.shutdown_helper(self.helper_port).await;
        }
        self.hooks.stop_native_browser_compatibility().await;
        let (status, message) = match &result {
            Ok(()) => (
                "stopped",
                "Codex target is no longer available; launcher stopped.".to_string(),
            ),
            Err(error) => (
                "failed",
                format!("Codex process monitoring failed: {error:#}"),
            ),
        };
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "launcher.exited",
            serde_json::json!({
                "debug_port": self.debug_port,
                "helper_port": self.helper_port,
                "status": status,
                "message": message,
            }),
        );
        if let Err(error) = save_terminal_launch_status(
            &self.status_store,
            self.status_started_at_ms,
            status,
            &message,
        ) {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "launcher.exit_status_failed",
                serde_json::json!({ "message": format!("{error:#}") }),
            );
        }
        result
    }
}

fn save_terminal_launch_status(
    store: &StatusStore,
    started_at_ms: u64,
    status: &str,
    message: &str,
) -> anyhow::Result<()> {
    let Some(mut latest) = store.load_latest()? else {
        return Ok(());
    };
    // 重启过程中管理器可能已写入新请求；旧 launcher 不能把它覆盖成 stopped。
    if latest.started_at_ms != started_at_ms {
        return Ok(());
    }
    latest.status = status.to_string();
    latest.message = message.to_string();
    latest.phase = None;
    latest.progress = Some(100);
    store.save_latest(&latest)
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
    async fn start_native_browser_compatibility(&self, _settings: &BackendSettings) {}
    async fn stop_native_browser_compatibility(&self) {}
    fn cleanup_unsupported_config(&self) -> anyhow::Result<()> {
        Ok(())
    }
    async fn run_provider_sync(&self) -> anyhow::Result<()>;
    fn has_pending_remote_control_session_recoveries(&self) -> bool {
        false
    }
    fn remote_control_session_recovery_is_safe_to_run(&self) -> bool {
        true
    }
    async fn run_remote_control_session_recovery(&self) -> anyhow::Result<()>;
    async fn apply_active_relay_profile(&self, _settings: &BackendSettings) -> anyhow::Result<()> {
        Ok(())
    }
    async fn ensure_active_protocol_proxy_config(
        &self,
        _settings: &BackendSettings,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    async fn start_helper(&self, helper_port: u16) -> anyhow::Result<()>;
    async fn launch_codex(
        &self,
        app_dir: &Path,
        debug_port: u16,
        settings: &BackendSettings,
        extra_args: &[String],
        instance: &LaunchInstance,
    ) -> anyhow::Result<CodexLaunch>;
    async fn bridge_context(
        &self,
        _debug_port: u16,
        _app_dir: &Path,
    ) -> anyhow::Result<Option<crate::routes::BridgeContext>> {
        Ok(None)
    }
    async fn inject(&self, debug_port: u16, helper_port: u16) -> anyhow::Result<()>;
    async fn apply_native_fast_unlock_on_launch(&self, _debug_port: u16) -> anyhow::Result<()> {
        Ok(())
    }
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
    /// 初次启动初始化结束时固定身份（包含注入降级）；watchdog 不能重新绑定实例。
    async fn capture_injected_launch_identity(&self, _debug_port: u16) {}
    async fn start_bridge_watchdog(
        &self,
        _debug_port: u16,
        _helper_port: u16,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    async fn write_status(&self, status: &str);
    async fn wait_for_codex_exit(
        &self,
        launch: &CodexLaunch,
        debug_port: u16,
    ) -> anyhow::Result<()>;
    async fn shutdown_helper(&self, helper_port: u16);
    async fn terminate_codex(&self, launch: &CodexLaunch);
}

#[derive(Default)]
pub struct DefaultLaunchHooks {
    child: Mutex<Option<Child>>,
    helper: Mutex<Option<HelperRuntime>>,
    bridge_watchdog: Mutex<Option<BridgeWatchdogRuntime>>,
    bridge_reinjector: Mutex<Option<BridgeReinjector>>,
    #[cfg(windows)]
    packaged_executable: Mutex<Option<PathBuf>>,
    #[cfg(windows)]
    packaged_activation_process: Mutex<Option<crate::windows_integration::TrackedWindowsProcess>>,
    #[cfg(windows)]
    packaged_launch_identity: Mutex<Option<PackagedLaunchIdentity>>,
}

#[cfg(windows)]
#[derive(Default)]
struct PackagedLaunchIdentity {
    browser_id: Option<String>,
    process: Option<crate::windows_integration::TrackedWindowsProcess>,
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

/// 判断错误链里是不是「端口已被占用」。只有这一种失败值得等前任让位重试，
/// 其余（权限不足、地址非法等）重试多少次都一样，直接冒泡更快也更好排查。
pub(crate) fn error_is_address_in_use(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|io_error| io_error.kind() == std::io::ErrorKind::AddrInUse)
    })
}

/// 判断错误链里是不是「端口被系统禁止绑定」（issue #2189）。
/// Windows 上 Hyper-V/WSL 会在开机时把动态端口范围（49152-65535）里的一段段端口
/// 划进排除区间，落在区间里的端口 bind 报 os error 10013（PermissionDenied），
/// 和「被进程占用」不是一回事。真正被排除区间划走时重试不会成功，
/// 但重启瞬间旧套接字未放开也会暂时返回同一个错误，所以只有固定端口会在
/// 有限窗口内重试，窗口结束后仍失败才给出对症指引。
fn error_is_bind_forbidden(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(crate::ports::port_bind_forbidden)
    })
}

/// 把 helper 端口 bind 失败翻译成用户能照着处理的提示。
///
/// 过去只有「端口被占用」会追加中文说明（issue #1933 的重试逻辑），
/// Windows 保留端口区间（os error 10013）直接裸抛英文 bind 报错，
/// 用户既看不懂也不知道为什么混入 Responses key 之后就再也起不来。
fn describe_helper_bind_failure(
    error: anyhow::Error,
    helper_port: u16,
    protocol_proxy_enabled: bool,
    bind_retry_timeout_ms: u64,
) -> anyhow::Error {
    if protocol_proxy_enabled && error_is_address_in_use(&error) {
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "helper.bind_gave_up_on_busy_port",
            serde_json::json!({
                "helper_port": helper_port,
                "waited_ms": bind_retry_timeout_ms,
            }),
        );
        return error.context(format!(
            "协议代理端口 {helper_port} 被其他进程占用，等待 {} 秒后仍未释放。\
             该端口写在 config.toml 的 base_url 里，不能自动改用其他端口；\
             请退出仍在运行的 Codex++ 或占用该端口的程序后重试。",
            bind_retry_timeout_ms / 1000
        ));
    }
    if error_is_bind_forbidden(&error) {
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "helper.bind_forbidden_port",
            serde_json::json!({ "helper_port": helper_port }),
        );
        if cfg!(windows) {
            return error.context(format!(
                "helper 端口 {helper_port} 被 Windows 保留，无法绑定\
                 （os error 10013，常见于 Hyper-V/WSL 开机划走的动态端口排除区间）。\
                 请以管理员运行 netsh interface ipv4 show excludedportrange protocol=tcp \
                 确认该端口是否在排除区间内，重启电脑通常可重新分配；\
                 协议代理模式下也可以设置环境变量 CODEX_PLUS_PROTOCOL_PROXY_PORT \
                 换一个端口后重试。"
            ));
        }
        return error.context(format!(
            "helper 端口 {helper_port} 绑定被系统拒绝，请检查端口占用与权限。"
        ));
    }
    error
}

/// 按 `interval_ms` 重试启动 helper，直到成功或超过 `timeout_ms`。
/// 端口被占用会重试；`retry_forbidden` 为真时，暂时的 10013 也放在同一段等待里。
async fn start_helper_waiting_for_busy_port<F, Fut>(
    mut start: F,
    timeout_ms: u64,
    interval_ms: u64,
    retry_forbidden: bool,
) -> anyhow::Result<()>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = anyhow::Result<()>>,
{
    let mut waited_ms = 0;
    let mut attempts = 0;
    loop {
        attempts += 1;
        let error = match start().await {
            Ok(()) => {
                if attempts > 1 {
                    let _ = crate::diagnostic_log::append_diagnostic_log(
                        "helper.bind_recovered_after_busy_port",
                        serde_json::json!({
                            "attempts": attempts,
                            "waited_ms": waited_ms,
                        }),
                    );
                }
                return Ok(());
            }
            Err(error) => error,
        };
        let retryable =
            error_is_address_in_use(&error) || (retry_forbidden && error_is_bind_forbidden(&error));
        if !retryable || waited_ms >= timeout_ms {
            return Err(error);
        }
        tokio::time::sleep(std::time::Duration::from_millis(interval_ms)).await;
        waited_ms += interval_ms;
    }
}

fn helper_bind_retry_timeout_ms(protocol_proxy_enabled: bool, is_macos: bool) -> u64 {
    if protocol_proxy_enabled {
        PROTOCOL_PROXY_BIND_RETRY_TIMEOUT_MS
    } else if is_macos {
        HELPER_BIND_RETRY_TIMEOUT_MS
    } else {
        0
    }
}

pub fn protocol_proxy_bind_retry_timeout_ms() -> u64 {
    PROTOCOL_PROXY_BIND_RETRY_TIMEOUT_MS
}

pub fn helper_bind_retry_interval_ms() -> u64 {
    HELPER_BIND_RETRY_INTERVAL_MS
}

/// 这次启动如果必须占用写进 `config.toml` 的协议代理端口，返回那个端口。
/// 不需要协议代理时返回 `None`，调用方不应去等一个用不到的端口。
pub fn required_fixed_helper_port(settings: &BackendSettings) -> Option<u16> {
    let enabled =
        relay_protocol_proxy_enabled(settings) || remote_control_provider_proxy_enabled(settings);
    enabled.then(crate::protocol_proxy::protocol_proxy_port)
}

/// 在拉起新 launcher 之前确认固定端口已经能绑定。
/// `probe` 成功表示端口已释放；占用和暂时的 10013 都继续等，其它错误立即返回。
pub fn wait_for_fixed_helper_port(
    port: u16,
    timeout_ms: u64,
    interval_ms: u64,
    mut probe: impl FnMut(u16) -> std::io::Result<()>,
    mut sleep: impl FnMut(u64),
) -> anyhow::Result<()> {
    let mut waited_ms = 0u64;
    loop {
        match probe(port) {
            Ok(()) => return Ok(()),
            Err(error) => {
                let error = anyhow::Error::from(error);
                let retryable = error_is_address_in_use(&error) || error_is_bind_forbidden(&error);
                if !retryable || waited_ms >= timeout_ms {
                    return Err(describe_helper_bind_failure(error, port, true, timeout_ms));
                }
                sleep(interval_ms);
                waited_ms = waited_ms.saturating_add(interval_ms);
            }
        }
    }
}

pub async fn launch_and_inject_with_hooks<H>(
    options: LaunchOptions,
    hooks: H,
) -> anyhow::Result<LaunchHandle>
where
    H: IntoLaunchHooks,
{
    let hooks = hooks.into_launch_hooks();
    let mut debug_port = hooks.select_debug_port(options.debug_port);
    let mut helper_port = hooks.select_helper_port(options.helper_port);
    let settings = hooks.load_settings().await?;
    // 请求体上限必须在 helper 绑定端口、开始接收请求前生效；环境变量优先级更高，
    // 由 configured_http_body_bytes 统一处理。
    set_http_body_limit_bytes(settings.helper_max_request_body_bytes());
    let app_dir = hooks.resolve_app_dir(options.app_dir.as_deref(), &settings)?;
    let status_store = options.status_store.clone();
    let mut helper_started = false;
    let mut launched = None;
    let mut keep_launched_on_error = false;
    let mut status_started_at_ms = 0;

    let result: anyhow::Result<LaunchHandle> = async {
        // issue #2244：冷启动 80~110 秒期间既没有 latest-status 阶段，也没有带耗时的
        // 日志，用户无法区分「正在处理」与「已经卡死」。每个前置步骤前后都打一次
        // 阶段标记与耗时。
        let timeline = LaunchTimeline::new(options.status_store.clone());
        timeline.mark("start_native_browser_compatibility", 5);
        hooks.start_native_browser_compatibility(&settings).await;
        let home = crate::relay_config::default_codex_home_dir();
        timeline.mark("cleanup_unsupported_config", 10);
        hooks.cleanup_unsupported_config()?;
        crate::relay_config::ensure_windows_sandbox_usable_for_current_user(&home)?;
        if settings.provider_sync_enabled {
            timeline.mark("provider_sync", 15);
            crate::codex_app_state::capture_app_state_snapshot_nonfatal(&home, "launcher.before");
            hooks.run_provider_sync().await?;
            crate::codex_app_state::sync_app_state_after_provider_switch_nonfatal(
                &home,
                "launcher.after_provider_sync",
            );
        }
        if hooks.has_pending_remote_control_session_recoveries()
            && hooks.remote_control_session_recovery_is_safe_to_run()
        {
            timeline.mark("remote_control_session_recovery", 25);
            hooks.run_remote_control_session_recovery().await?;
        } else if hooks.has_pending_remote_control_session_recoveries() {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "launcher.remote_control_session_finalization_deferred",
                serde_json::json!({"reason": "desktop_writer_active"}),
            );
        }
        timeline.mark("dream_skin_theme", 30);
        crate::dream_skin::sync_default_dream_skin_base_theme(
            settings.enhancements_enabled
                && settings.codex_app_dream_skin_enabled
                && !settings.codex_app_dream_skin_paused,
            &settings.codex_app_dream_skin_theme_config,
        )?;
        // 这一步在 #2244 的采样里是嫌疑最大的同步全表扫描，单独标记起止，
        // 从 latest-status 就能看出冷启动时间是否耗在这里。
        timeline.mark("sanitize_historical_model_suffixes", 40);
        match crate::codex_sqlite::sanitize_historical_model_suffixes(&home) {
            Ok(result) if result.updated > 0 => {
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "launcher.sanitize_historical_model_suffixes",
                    serde_json::json!({
                        "scanned": result.scanned,
                        "updated": result.updated
                    }),
                );
            }
            Ok(_) => {}
            Err(error) => {
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "launcher.sanitize_historical_model_suffixes_failed",
                    serde_json::json!({
                        "error": error.to_string()
                    }),
                );
            }
        }
        timeline.mark("sanitize_historical_model_suffixes_done", 50);
        let protocol_proxy_enabled = relay_protocol_proxy_enabled(&settings)
            || remote_control_provider_proxy_enabled(&settings);
        // issue #2264：重启链路上的模型漂移修复。launcher 不重放完整 apply（那会
        // 覆盖 Codex 在 UI 选择后回写的 live 值），只在 live config 的 model 仍是
        // 工具写入的隐式默认且有未归档目标任务时，对齐到目标任务模型。
        if settings.relay_profiles_enabled {
            // 只迁移旧版受管 catalog 的路径表达，不重放 profile 或覆盖用户选定模型。
            if let Err(error) =
                crate::relay_config::migrate_managed_model_catalog_pointer_in_home(&home, &settings)
            {
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "launcher.model_catalog_pointer_migration_failed",
                    serde_json::json!({ "message": error.to_string() }),
                );
            }
            let profile = settings.active_relay_profile();
            if profile.relay_mode != crate::settings::RelayMode::Official {
                if let Err(error) =
                    crate::relay_config::align_live_config_model_with_goal_thread(&home, &profile)
                {
                    let _ = crate::diagnostic_log::append_diagnostic_log(
                        "launcher.goal_thread_model_align_failed",
                        serde_json::json!({
                            "message": error.to_string(),
                        }),
                    );
                }
            }
        }
        if protocol_proxy_enabled {
            timeline.mark("protocol_proxy_config", 55);
            // 端口落在 Windows 排除区间（10013）时永久不可绑，先迁移并持久化，
            // 再写 config.toml，保证 base_url、helper 与路由使用同一个端口（#1163）。
            helper_port = crate::protocol_proxy::ensure_protocol_proxy_port_available();
            hooks.ensure_active_protocol_proxy_config(&settings).await?;
        }
        if settings.enhancements_enabled || protocol_proxy_enabled {
            // macOS 重启时旧 launcher 的 socket 释放可能稍晚于进程退出。
            timeline.mark("helper_bind", 60);
            let bind_retry_timeout_ms =
                helper_bind_retry_timeout_ms(protocol_proxy_enabled, cfg!(target_os = "macos"));
            start_helper_waiting_for_busy_port(
                || hooks.start_helper(helper_port),
                bind_retry_timeout_ms,
                HELPER_BIND_RETRY_INTERVAL_MS,
                protocol_proxy_enabled,
            )
            .await
            .map_err(|error| {
                describe_helper_bind_failure(
                    error,
                    helper_port,
                    protocol_proxy_enabled,
                    bind_retry_timeout_ms,
                )
            })?;
            helper_started = true;
            timeline.mark("helper_listening", 65);
        }

        timeline.mark("launch_codex", 70);
        // 启动 Codex 之前清一遍选择器残留：换过模型目录之后，
        // 「最近使用」里会留着上一个目录的模型 ID，用户点到就是必然 400 的模型。
        crate::codex_app_state::prune_stale_model_references_after_switch_nonfatal(
            &home,
            &settings,
            "launcher.before_launch_codex",
        );
        let launch = hooks
            .launch_codex(
                &app_dir,
                debug_port,
                &settings,
                &settings.codex_extra_args,
                &LaunchInstance::from_options(&options),
            )
            .await?;
        debug_port = launch.debug_port().unwrap_or(debug_port);
        launched = Some(launch.clone());
        keep_launched_on_error = true;
        timeline.mark("launch_codex_done", 80);

        let mut injection_degraded = false;
        if settings.enhancements_enabled {
            timeline.mark("ensure_injection", 85);
            let injection_ready = hooks
                .ensure_injection(debug_port, helper_port, &app_dir)
                .await;
            timeline.mark("ensure_injection_done", 95);
            if injection_ready {
                keep_launched_on_error = false;
                if settings.codex_app_native_fast_unlock
                    && let Err(error) = hooks.apply_native_fast_unlock_on_launch(debug_port).await
                {
                    let _ = crate::diagnostic_log::append_diagnostic_log(
                        "launcher.native_fast_unlock_failed",
                        serde_json::json!({ "message": error.to_string() }),
                    );
                }
                // 注入成功后页面已加载，此时可以通过 CDP 清理 Electron Local Storage
                // 中残留的带后缀模型名，避免模型选择器继续显示废弃项。
                crate::codex_local_storage::sanitize_local_storage_model_suffixes_nonfatal(
                    debug_port,
                )
                .await;
            } else {
                let degraded = launch_status(
                    "running_degraded",
                    "Codex launched; Codex++ enhancements are still waiting for the page bridge.",
                    debug_port,
                    helper_port,
                    &app_dir,
                );
                options.status_store.save_latest(&degraded)?;
                status_started_at_ms = degraded.started_at_ms;
                hooks.write_status("running_degraded").await;
                injection_degraded = true;
            }
        }

        // 注入降级或只启用协议代理也要固定身份；后台重注入不能为旧启动器换绑实例。
        hooks.capture_injected_launch_identity(debug_port).await;
        if settings.enhancements_enabled && !injection_degraded {
            hooks.start_bridge_watchdog(debug_port, helper_port).await?;
        }

        if !settings.enhancements_enabled || !injection_degraded {
            timeline.mark("ready", 100);
            let status = launch_status(
                "running",
                "Codex++ launcher ready",
                debug_port,
                helper_port,
                &app_dir,
            );
            options.status_store.save_latest(&status)?;
            status_started_at_ms = status.started_at_ms;
            hooks.write_status("running").await;
        }

        // 全部可失败的启动步骤完成后才创建，错误清理路径不会遗留监督任务。
        let isolated_quota_gate = should_start_isolated_quota_gate(&settings)
            .then(|| IsolatedQuotaGateRuntime::start(debug_port));
        Ok(LaunchHandle {
            debug_port,
            helper_port,
            app_dir: app_dir.clone(),
            launch,
            status_store: status_store.clone(),
            status_started_at_ms,
            helper_started,
            isolated_quota_gate,
            hooks: Arc::clone(&hooks),
        })
    }
    .await;

    match result {
        Ok(handle) => Ok(handle),
        Err(error) => {
            hooks.stop_native_browser_compatibility().await;
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

fn should_start_isolated_quota_gate(settings: &BackendSettings) -> bool {
    !settings.enhancements_enabled && settings.codex_app_standalone_quota_unlock
}

fn relay_protocol_proxy_enabled(settings: &BackendSettings) -> bool {
    settings.active_relay_uses_protocol_proxy()
}

fn remote_control_provider_proxy_enabled(settings: &BackendSettings) -> bool {
    let profile = settings.active_relay_profile();
    profile.relay_mode == crate::settings::RelayMode::Official && profile.official_mix_api_key
}

fn active_relay_profile_is_official_mix(settings: &BackendSettings) -> bool {
    if !settings.relay_profiles_enabled {
        return false;
    }
    let profile = settings.active_relay_profile();
    profile.relay_mode == crate::settings::RelayMode::Official && profile.official_mix_api_key
}
#[cfg(windows)]
fn apply_codexplusplus_window_icon_after_launch(process_id: u32) {
    let icon_resource_path =
        std::env::current_exe().unwrap_or_else(|_| PathBuf::from("codex-plus-plus.exe"));
    tokio::spawn(async move {
        let mut state = crate::windows_integration::WindowIconApplyState::default();
        for attempt in 1..=30 {
            if crate::windows_integration::apply_codexplusplus_icon_attempt(
                process_id,
                &icon_resource_path,
                &mut state,
            ) {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            if attempt == 30 {
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "launcher.window_icon.apply_failed",
                    serde_json::json!({
                        "process_id": process_id,
                        "icon_resource_path": icon_resource_path.to_string_lossy()
                    }),
                );
            }
        }
    });
}

#[cfg(not(windows))]
fn apply_codexplusplus_window_icon_after_launch(_process_id: u32) {}

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

    /// Configures the launcher-specific callback used by subsequent watchdog reinjections.
    pub async fn set_bridge_reinjector(&self, reinjector: BridgeReinjector) {
        *self.bridge_reinjector.lock().await = Some(reinjector);
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
        .ok_or_else(|| {
            // issue #2204：原来四种失败原因（不存在 / 是 Codex++ 目录 / 无可执行文件 /
            // 纯自动探测落空）共用一句「Codex App directory not found」，用户看不出
            // 该改什么。这里按来源细分，并保留原句前缀以便既有日志检索与用户既有
            // 报障文本仍能对上。
            let saved = settings.codex_app_path.trim();
            let message = crate::app_paths::describe_codex_app_dir_not_found(app_dir, Some(saved));
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "launcher.app_dir_not_found",
                serde_json::json!({
                    "explicit_app_path": app_dir.map(|path| path.to_string_lossy().to_string()),
                    "saved_app_path": saved,
                    "reason": message,
                }),
            );
            anyhow::anyhow!(message)
        })
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

    fn cleanup_unsupported_config(&self) -> anyhow::Result<()> {
        let home = crate::relay_config::default_codex_home_dir();
        crate::relay_config::cleanup_unsupported_approval_policies_in_home(&home)?;
        Ok(())
    }

    async fn run_provider_sync(&self) -> anyhow::Result<()> {
        anyhow::bail!("provider sync requires launcher hooks with codex-plus-data integration")
    }

    async fn run_remote_control_session_recovery(&self) -> anyhow::Result<()> {
        anyhow::bail!(
            "Remote Control session recovery requires launcher hooks with codex-plus-data integration"
        )
    }

    fn remote_control_session_recovery_is_safe_to_run(&self) -> bool {
        crate::watcher::find_session_index_cleanup_blocking_processes().is_empty()
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
        crate::relay_config::apply_relay_profile_with_preservation(
            &home, &profile, &common_config, crate::protocol_proxy::protocol_proxy_port(), settings.relay_preserve_nonprovider_settings,
        )?;
        Ok(())
    }

    async fn ensure_active_protocol_proxy_config(
        &self,
        settings: &BackendSettings,
    ) -> anyhow::Result<()> {
        let home = crate::relay_config::default_codex_home_dir();
        crate::relay_config::ensure_active_protocol_proxy_config_in_home(&home, settings)?;
        Ok(())
    }

    async fn start_helper(&self, helper_port: u16) -> anyhow::Result<()> {
        // 常驻 helper 已经监听同一端口时直接复用：会话不再二次 bind，也不会在
        // Codex 退出时关掉它（本实例没有持有它的 shutdown 句柄）。
        if crate::resident_helper::resident_helper_port() == Some(helper_port) {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "helper.reuse_resident",
                serde_json::json!({ "helper_port": helper_port }),
            );
            return Ok(());
        }
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
        // 端口绑定成功后立刻落凭据：Claude 汉化注入的 hook 靠它调用听写路由。
        write_dictation_helper_receipt(helper_port);
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
        _settings: &BackendSettings,
        extra_args: &[String],
        instance: &LaunchInstance,
    ) -> anyhow::Result<CodexLaunch> {
        let launch_extra_args = normalize_codex_extra_args(extra_args);
        let isolated_multi_instance =
            std::env::var_os("CODEX_PLUS_MULTI_INSTANCE").is_some_and(|value| value == "1");
        // 打包版（MSIX/Store）上的多开必须另走一条路。
        //
        // 主实例走 AUMID 激活，它是 Windows 上唯一能给进程包身份的方式；但 AUMID 激活
        // 只会把已存在的主实例拉到前台，起不出第二份隔离实例。而绕开 AUMID 按路径
        // `CreateProcess` 包内 exe 会丢包身份（APPMODEL_ERROR_NO_PACKAGE / Access denied），
        // 进程秒退、CDP 永不监听——这正是多开在 Windows 上「点了没窗口」的原因。
        //
        // 所以多开改为「包上下文里跑 wrapper」：`Invoke-CommandInDesktopPackage
        // -PreventBreakaway` 让 wrapper 及其子进程都带该 AppId 的包身份，wrapper 内再带
        // 上实例自己的 CODEX_HOME / --user-data-dir / 调试端口拉起 exe。该 cmdlet 属
        // 非契约接口，失败时按下面的 fallback 退回原有按路径启动（至少不比旧行为更差）。
        if cfg!(windows) && isolated_multi_instance {
            match launch_packaged_multi_instance(
                app_dir,
                debug_port,
                &launch_extra_args,
                instance.state_dir.as_deref(),
                instance.codex_home.as_deref(),
                instance.sqlite_home.as_deref(),
            )
            .await
            {
                Ok(Some(launch)) => return Ok(launch),
                Ok(None) => {}
                Err(error) => {
                    let _ = crate::diagnostic_log::append_diagnostic_log(
                        "launcher.packaged_multi_instance_fallback",
                        serde_json::json!({
                            "app_dir": app_dir,
                            "debug_port": debug_port,
                            "message": format!("{error:#}"),
                        }),
                    );
                }
            }
        }
        if cfg!(windows) && !isolated_multi_instance {
            // AUMID 解析留在调用点：它要查系统里的 Store 注册表（#2140），是有副作用
            // 的一步；下面的构造函数只做纯拼接。解析不出来（目录名像包、但系统里没
            // 这个包）就不再走打包激活，继续往下按路径启动——与既有回退路径一致。
            let activation =
                crate::app_paths::packaged_app_user_model_id(app_dir).map(|app_user_model_id| {
                    build_packaged_activation(&app_user_model_id, debug_port, &launch_extra_args)
                });
            if let Some(activation) = activation {
                let CodexLaunch::PackagedActivation {
                    app_user_model_id,
                    arguments,
                    ..
                } = &activation
                else {
                    unreachable!();
                };
                match activate_packaged_app(app_user_model_id, arguments).await {
                    Ok(process_id) => {
                        #[cfg(windows)]
                        {
                            let executable = crate::app_paths::build_codex_executable(app_dir);
                            // 立即持有激活 PID，不能在漫长注入之后再按旧 PID 打开进程。
                            let process =
                                match crate::windows_integration::TrackedWindowsProcess::capture(
                                    process_id,
                                    &executable,
                                ) {
                                    Ok(process) => process,
                                    Err(error) => {
                                        let _ = crate::diagnostic_log::append_diagnostic_log(
                                            "launcher.packaged_process_wait_failed_nonfatal",
                                            serde_json::json!({ "process_id": process_id, "message": format!("{error:#}") }),
                                        );
                                        None
                                    }
                                };
                            *self.packaged_executable.lock().await = Some(executable);
                            *self.packaged_activation_process.lock().await = process;
                            *self.packaged_launch_identity.lock().await = None;
                        }
                        apply_codexplusplus_window_icon_after_launch(process_id);
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
                    Err(error) => {
                        // AUMID 激活失败（例如清单 Application Id 变化）时回退到
                        // 直接执行应用，避免整份配置无法启动。
                        let _ = crate::diagnostic_log::append_diagnostic_log(
                            "launcher.packaged_activation_fallback",
                            serde_json::json!({
                                "app_user_model_id": app_user_model_id,
                                "app_dir": app_dir,
                                "error": error.to_string()
                            }),
                        );
                        // 提权进程激活打包应用必然失败，按路径启动的进程会因缺少
                        // 包身份立即退出、注入也必然失败；此时必须如实失败并给出
                        // 指引，而不是静默回退导致 latest-status.json 误报 running
                        // （issue #2351）。
                        if let Some(reason) = packaged_activation_fallback_block_reason(
                            crate::windows_integration::current_process_is_elevated(),
                        ) {
                            anyhow::bail!("{reason}（激活错误：{error}）");
                        }
                    }
                }
            }
        }

        if app_dir.extension().and_then(|value| value.to_str()) == Some("app")
            && !isolated_multi_instance
        {
            let launch_action = select_macos_debug_launch_action(
                is_macos_app_running(app_dir).await,
                crate::cdp::endpoint_available(debug_port),
            );
            let cleanup_policy = match launch_action {
                MacosDebugLaunchAction::LaunchNew => MacosCleanupPolicy::QuitIfNotPreviouslyRunning,
                MacosDebugLaunchAction::ReuseRunningDebugApp => {
                    MacosCleanupPolicy::SkipQuitBecauseAlreadyRunning
                }
                MacosDebugLaunchAction::RestartRunningApp => {
                    let _ = crate::diagnostic_log::append_diagnostic_log(
                        "launcher.macos_existing_app_without_cdp_restart_requested",
                        serde_json::json!({
                            "app_dir": app_dir,
                            "debug_port": debug_port
                        }),
                    );
                    quit_macos_app_and_wait(app_dir).await?;
                    MacosCleanupPolicy::QuitIfNotPreviouslyRunning
                }
            };
            let command = build_macos_open_command(app_dir, debug_port, &launch_extra_args);
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

        let command = build_codex_command(app_dir, debug_port, &launch_extra_args);
        let executable = command
            .first()
            .ok_or_else(|| anyhow::anyhow!("Codex command is empty"))?;
        let mut child_command = Command::new(executable);
        child_command
            .args(&command[1..])
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // 实例状态目录要显式传给子进程：多开原先只靠给 worker 换 HOME，
        // 而 Windows 上 `directories` 走 SHGetKnownFolderPath、不读 HOME。
        if let Some(state_dir) = instance.state_dir.as_deref() {
            child_command.env(crate::paths::STATE_DIR_ENV, state_dir);
        }
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

    async fn apply_native_fast_unlock_on_launch(&self, debug_port: u16) -> anyhow::Result<()> {
        apply_native_fast_unlock_on_launch(debug_port).await
    }

    async fn capture_injected_launch_identity(&self, debug_port: u16) {
        #[cfg(windows)]
        {
            let mut identity = self.packaged_launch_identity.lock().await;
            if identity.is_some() {
                return;
            }
            let Some(executable) = self.packaged_executable.lock().await.clone() else {
                return;
            };
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
            let mut last_discovery_error = None;
            loop {
                let browser_id = validated_codex_browser_id(debug_port).await;
                let mut process = match crate::windows_integration::capture_debug_listener(
                    debug_port,
                    &executable,
                ) {
                    Ok(process) => process,
                    Err(error) => {
                        let message = format!("{error:#}");
                        if last_discovery_error.as_ref() != Some(&message) {
                            let _ = crate::diagnostic_log::append_diagnostic_log(
                                "launcher.packaged_process_identity_unavailable",
                                serde_json::json!({ "debug_port": debug_port, "message": message }),
                            );
                        }
                        last_discovery_error = Some(message);
                        None
                    }
                };
                let activation = self.packaged_activation_process.lock().await;
                let activation_matches = activation.as_ref().zip(process.as_ref()).is_some_and(
                    |(activation, process)| {
                        activation.process_id == process.process_id
                            && activation.birth_id == process.birth_id
                    },
                );
                drop(activation);
                // target 列表、UUID 和 listener 查询之间可能发生替换；只接受稳定的 Codex UUID。
                let browser_still_matches = if let Some(initial) = browser_id.as_deref() {
                    validated_codex_browser_id(debug_port).await.as_deref() == Some(initial)
                } else {
                    false
                };
                if browser_id.is_some() && !activation_matches && !browser_still_matches {
                    process = None;
                }
                let browser_id = browser_id.filter(|_| browser_still_matches);
                // 无 CDP 页时，已验证可执行文件和句柄的 listener 仍可作为本次进程身份。
                if browser_id.is_some() || process.is_some() {
                    let _ = crate::diagnostic_log::append_diagnostic_log(
                        "launcher.packaged_launch_identity_captured",
                        serde_json::json!({
                            "debug_port": debug_port,
                            "browser_identity_bound": browser_id.is_some(),
                            "process_id": process.as_ref().map(|process| process.process_id),
                            "birth_id": process.as_ref().map(|process| process.birth_id),
                        }),
                    );
                    *identity = Some(PackagedLaunchIdentity {
                        browser_id,
                        process,
                    });
                    return;
                }
                if tokio::time::Instant::now() >= deadline {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            }
        }
        #[cfg(not(windows))]
        let _ = debug_port;
    }
    async fn start_bridge_watchdog(&self, debug_port: u16, helper_port: u16) -> anyhow::Result<()> {
        let bridge_reinjector = self.bridge_reinjector.lock().await.clone();
        let (shutdown, mut shutdown_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            #[cfg(any(windows, target_os = "macos"))]
            let pet_cursor_task = tokio::spawn(run_pet_real_mouse_cursor_driver(debug_port));
            let mut observed_browser_id: Option<String> = None;
            let mut bridge_health_failures = 0u8;
            let mut bridge_reinject_backoff = BridgeReinjectBackoff::default();
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    _ = interval.tick() => {
                        let current_browser_id = match crate::cdp::browser_identity(debug_port).await {
                            Ok(identity) => identity.browser_id().ok(),
                            Err(_) => None,
                        };
                        let identity_changed = current_browser_id
                            .as_deref()
                            .is_some_and(|current| {
                                browser_identity_changed(observed_browser_id.as_deref(), current)
                            });
                        if let Some(current) = current_browser_id {
                            observed_browser_id = Some(current);
                        }
                        let (pet_result, _) = tokio::join!(
                            sync_pet_real_mouse_overlay(debug_port, helper_port),
                            check_and_reinject_bridge_inner(
                                debug_port,
                                helper_port,
                                identity_changed,
                                bridge_reinjector.clone(),
                                &mut bridge_health_failures,
                                &mut bridge_reinject_backoff,
                            ),
                        );
                        record_pet_overlay_sync_result(debug_port, helper_port, pet_result);
                    }
                }
            }
            #[cfg(any(windows, target_os = "macos"))]
            {
                pet_cursor_task.abort();
                let _ = pet_cursor_task.await;
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

    async fn wait_for_codex_exit(
        &self,
        launch: &CodexLaunch,
        debug_port: u16,
    ) -> anyhow::Result<()> {
        match launch {
            CodexLaunch::Process { .. } => {
                if let Some(mut child) = self.child.lock().await.take() {
                    let _ = child.wait().await;
                }
            }
            CodexLaunch::PackagedActivation { process_id, .. } => {
                #[cfg(windows)]
                {
                    let executable = self
                        .packaged_executable
                        .lock()
                        .await
                        .take()
                        .context("missing packaged Codex executable identity")?;
                    let activation = self.packaged_activation_process.lock().await.take();
                    let identity = self
                        .packaged_launch_identity
                        .lock()
                        .await
                        .take()
                        .unwrap_or_default();
                    return wait_for_windows_packaged_exit(
                        &executable,
                        *process_id,
                        activation,
                        identity,
                        debug_port,
                    )
                    .await;
                }
                #[cfg(not(windows))]
                if let Some(process_id) = process_id {
                    if let Err(error) = wait_for_windows_process_id(*process_id).await {
                        let _ = crate::diagnostic_log::append_diagnostic_log(
                            "launcher.packaged_process_wait_failed_nonfatal",
                            serde_json::json!({
                                "process_id": process_id,
                                "message": format!("{error:#}")
                            }),
                        );
                    }
                }
            }
        }
        // Another Codex process must not keep this launcher holding the native
        // browser monitor. Directly owned processes and this debug port can.
        //
        // 单次探测失败不足以判定退出：macOS 的 `ps` 采样和 `/json` 探测都会在
        // 进程换代/监听重建的空档里瞬断，而这里一旦返回就会关掉 helper，让写死在
        // config.toml 里的 57321 失去对端（见 PackagedExitGrace 的注释）。
        // 传入 Some(...) 包一层，是因为该宽限在 Windows 那条路径上把「进程身份
        // 未知」表示为 None，语义是回落到 cdp_available。
        let mut exit_grace = PackagedExitGrace::default();
        loop {
            let target_alive = owned_launcher_target_alive(
                launcher_owned_process_alive(launch, debug_port),
                crate::cdp::endpoint_available(debug_port),
            );
            if !exit_grace.keep_waiting(Some(target_alive), false, std::time::Instant::now()) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
        Ok(())
    }

    async fn shutdown_helper(&self, helper_port: u16) {
        if let Some(runtime) = self.bridge_watchdog.lock().await.take() {
            let _ = runtime.shutdown.send(());
            let _ = runtime.task.await;
        }
        if let Some(runtime) = self.helper.lock().await.take() {
            let _ = runtime.shutdown.send(());
            let _ = runtime.task.await;
            // 只有真正持有 helper 的实例才删除听写凭据：复用了常驻 helper 的
            // 会话收尾时不能把常驻 helper 的凭据删掉。
            remove_dictation_helper_receipt_for_port(helper_port);
        }
        // 会话 helper 启动时会写凭据并覆盖掉常驻 helper 的；收尾后若常驻实例
        // 还在监听，把凭据补回常驻端口，避免听写路由因为凭据指向已关闭端口而失效。
        if let Some(port) = crate::resident_helper::resident_helper_port()
            && port != helper_port
        {
            write_dictation_helper_receipt(port);
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

/// 听写 helper 的凭据文件名。Claude 汉化的注入脚本按同一路径读取，
/// 两边改动必须同步（见 assets/claude-localization/hook.js）。
const DICTATION_HELPER_RECEIPT: &str = "dictation-helper.json";

/// helper 运行期间把端口与能力令牌写进用户态目录。
///
/// 为什么需要它：Claude 汉化的页面脚本没有 Codex 页面的特权桥，只能由主进程 hook
/// 转发听写请求，而 helper 的能力令牌只存在于 launcher 进程内存里。文件限当前用户
/// 可读（0600），helper 停止即删除；令牌也只在 loopback 的听写路由上有效。
fn dictation_helper_receipt_path() -> PathBuf {
    crate::paths::default_app_state_dir().join(DICTATION_HELPER_RECEIPT)
}

fn write_dictation_helper_receipt(helper_port: u16) {
    write_dictation_helper_receipt_at(
        &dictation_helper_receipt_path(),
        helper_port,
        crate::dictation::helper_token(),
    );
}

fn write_dictation_helper_receipt_at(path: &Path, helper_port: u16, token: &str) {
    let Ok(text) = serde_json::to_vec(&serde_json::json!({
        "port": helper_port,
        "token": token,
    })) else {
        return;
    };
    let Some(parent) = path.parent() else {
        return;
    };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    // 先写临时文件再改名：读到写了一半的凭据会让调用方白失败一轮。
    let temporary = path.with_extension("json.tmp");
    if std::fs::write(&temporary, &text).is_err() {
        return;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600));
    }
    // Windows 上 rename 不覆盖已存在的目标，先删再加。
    let _ = std::fs::remove_file(path);
    let _ = std::fs::rename(&temporary, path);
}

/// 删除属于指定端口的听写 helper 凭据。
///
/// 同一进程里可能同时存在常驻 helper 与会话 helper（例如协议转换已关闭但
/// 常驻实例还没退出）：只删除端口匹配的凭据，避免一方收尾时把另一方的凭据
/// 误删。凭据整体只在 helper 停止时清理，与旧行为一致。
fn remove_dictation_helper_receipt_for_port(helper_port: u16) {
    remove_dictation_helper_receipt_at_for_port(&dictation_helper_receipt_path(), helper_port);
}

fn remove_dictation_helper_receipt_at_for_port(path: &Path, helper_port: u16) {
    let matches = std::fs::read_to_string(path)
        .ok()
        .and_then(|contents| serde_json::from_str::<Value>(&contents).ok())
        .and_then(|value| value.get("port").and_then(Value::as_u64))
        .is_some_and(|port| port == u64::from(helper_port));
    if matches {
        let _ = std::fs::remove_file(path);
    }
}

async fn handle_helper_connection(
    mut stream: tokio::net::TcpStream,
    remote_addr: Option<SocketAddr>,
) -> anyhow::Result<()> {
    let request = match read_http_request(&mut stream).await {
        Ok(request) => request,
        Err(error) => {
            let body = serde_json::to_vec(&serde_json::json!({
                "status": "failed",
                "message": error.to_string()
            }))?;
            write_http_response(
                &mut stream,
                error.status(),
                "application/json; charset=utf-8",
                &body,
            )
            .await?;
            stream.shutdown().await?;
            return Ok(());
        }
    };
    let request_headers = String::from_utf8_lossy(&request.headers);
    let request_line = request_headers.lines().next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let raw_path = parts.next().unwrap_or_default();
    let path = raw_path.split('?').next().unwrap_or(raw_path);
    let request_user_agent = header_value_from_headers(&request_headers, "user-agent");
    let request_beta_features =
        header_value_from_headers(&request_headers, "x-codex-beta-features");
    let request_session_id = header_value_from_headers(&request_headers, "session-id");
    let request_thread_id = header_value_from_headers(&request_headers, "thread-id");
    let request_opencode_session =
        header_value_from_headers(&request_headers, "x-opencode-session");
    let request_content_type = header_value_from_headers(&request_headers, "content-type");
    let request_content_encoding = header_value_from_headers(&request_headers, "content-encoding");
    let remote_addr_text = remote_addr.map(|addr| addr.to_string());

    let _ = crate::diagnostic_log::append_diagnostic_log(
        "helper.request",
        serde_json::json!({
            "method": method,
            "path": path,
            "request_line": request_line,
            "remote_addr": remote_addr_text,
            "body_bytes": request.body.len()
        }),
    );

    if matches!(path, "/dictation/transcribe" | "/dictation/status") {
        return handle_dictation_connection(
            &mut stream,
            method,
            path,
            &request_headers,
            &request.body,
            request_content_type.as_deref().unwrap_or_default(),
        )
        .await;
    }
    if crate::protocol_proxy::is_audio_transcriptions_proxy_path(path) && method == "POST" {
        return handle_audio_transcriptions_proxy_connection(
            &mut stream,
            &request.body,
            request_content_type.as_deref(),
            request_user_agent.as_deref(),
            method,
            path,
            remote_addr_text,
        )
        .await;
    }
    if (crate::protocol_proxy::is_image_generations_proxy_path(path)
        || crate::protocol_proxy::is_image_edits_proxy_path(path))
        && method == "OPTIONS"
    {
        write_http_response(
            &mut stream,
            "204 No Content",
            "application/json; charset=utf-8",
            &[],
        )
        .await?;
        stream.shutdown().await?;
        return Ok(());
    }
    if (crate::protocol_proxy::is_image_generations_proxy_path(path)
        || crate::protocol_proxy::is_image_edits_proxy_path(path))
        && method == "POST"
    {
        return handle_image_proxy_connection(
            &mut stream,
            &request.body,
            request_content_type.as_deref(),
            request_user_agent.as_deref(),
            method,
            path,
            remote_addr_text,
        )
        .await;
    }
    if crate::protocol_proxy::is_responses_proxy_path(path) && method == "GET" {
        let body = serde_json::to_vec(&serde_json::json!({
            "status": "upgrade_required",
            "message": "WebSocket transport is not supported by the local protocol proxy; retry with HTTP."
        }))?;
        write_http_response(
            &mut stream,
            "426 Upgrade Required",
            "application/json; charset=utf-8",
            &body,
        )
        .await?;
        log_helper_response(
            "helper.responses_websocket_upgrade_required",
            method,
            path,
            "426 Upgrade Required",
            remote_addr_text,
        );
        stream.shutdown().await?;
        return Ok(());
    }
    if crate::protocol_proxy::is_responses_proxy_path(path) && method == "POST" {
        let request_body = match decode_protocol_proxy_request_body(
            &request.body,
            request_content_encoding.as_deref(),
        ) {
            Ok(body) => body,
            Err(error) => {
                let body = serde_json::to_vec(&serde_json::json!({
                    "status": "failed",
                    "message": error.to_string()
                }))?;
                write_http_response(
                    &mut stream,
                    "400 Bad Request",
                    "application/json; charset=utf-8",
                    &body,
                )
                .await?;
                log_helper_response(
                    "helper.protocol_proxy_decode_failed",
                    method,
                    path,
                    "400 Bad Request",
                    remote_addr_text,
                );
                stream.shutdown().await?;
                return Ok(());
            }
        };
        return handle_protocol_proxy_connection(
            &mut stream,
            &request_body,
            request_user_agent.as_deref(),
            request_beta_features.as_deref(),
            crate::protocol_proxy::ProxySessionHeaders {
                session_id: request_session_id.as_deref(),
                thread_id: request_thread_id.as_deref(),
                opencode_session: request_opencode_session.as_deref(),
            },
            method,
            path,
            remote_addr_text,
        )
        .await;
    }
    let request_body = String::from_utf8_lossy(&request.body);
    if crate::protocol_proxy::is_chat_completions_proxy_path(path) && method == "POST" {
        return handle_chat_completions_proxy_connection(
            &mut stream,
            &request_body,
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

    let (status, body, content_type, log_event) = if path == "/backend/status"
        && matches!(method, "GET" | "POST" | "OPTIONS")
    {
        (
            "200 OK".to_string(),
            serde_json::to_vec(&serde_json::json!({
                "status": "ok",
                "message": "后端已连接",
                "version": crate::version::VERSION,
                "hideOfficialUsageAlert": crate::assets::hide_official_usage_alert_config(
                    &crate::settings::SettingsStore::default().load().unwrap_or_default()
                ),
                "transport": "http-helper"
            }))?,
            "application/json; charset=utf-8".to_string(),
            "helper.backend_status_ok",
        )
    } else if path == "/proxy-status" && matches!(method, "GET" | "POST" | "OPTIONS") {
        let mut active = crate::request_log::running_requests();
        active.sort_by_key(|entry| std::cmp::Reverse(entry.started_at_ms));
        let requests = active
            .into_iter()
            .map(|entry| {
                serde_json::json!({
                    "id": entry.id,
                    "model": entry.model,
                    "threadId": entry.thread_id,
                    "relayName": entry.relay_name,
                    "startedAtMs": entry.started_at_ms,
                    "attempts": entry.attempts,
                    "autoRetries": entry.auto_retries,
                    "retryReasons": entry.retry_reasons,
                    "phase": entry.phase,
                    "firstTokenMs": entry.first_token_ms,
                    "estimatedOutputTokens": entry.estimated_output_tokens,
                    "attemptDetails": entry.attempt_details,
                })
            })
            .collect::<Vec<_>>();
        (
            "200 OK".to_string(),
            serde_json::to_vec(&serde_json::json!({
                "status": "ok",
                "requests": requests
            }))?,
            "application/json; charset=utf-8".to_string(),
            "helper.proxy_status_ok",
        )
    } else if path == "/diagnostics/log" && matches!(method, "POST" | "OPTIONS") {
        if method == "POST" {
            let detail =
                serde_json::from_str::<serde_json::Value>(&request_body).unwrap_or_else(|error| {
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
            let _ =
                crate::diagnostic_log::append_diagnostic_log(&format!("renderer.{event}"), detail);
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
    } else if path == "/dream-skin/image" && matches!(method, "GET" | "OPTIONS") {
        if method == "OPTIONS" {
            (
                "200 OK".to_string(),
                Vec::new(),
                "application/octet-stream".to_string(),
                "helper.dream_skin_image_options",
            )
        } else {
            dream_skin_image_response()
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

fn decode_protocol_proxy_request_body(
    body: &[u8],
    content_encoding: Option<&str>,
) -> anyhow::Result<String> {
    let encoding = content_encoding.unwrap_or_default().trim();
    let decoded = if encoding.is_empty() || encoding.eq_ignore_ascii_case("identity") {
        body.to_vec()
    } else if encoding.eq_ignore_ascii_case("zstd") {
        let decoder = zstd::stream::read::Decoder::new(std::io::Cursor::new(body))?;
        let max_body_bytes = configured_http_body_bytes();
        let mut limited = decoder.take((max_body_bytes + 1) as u64);
        let mut decoded = Vec::new();
        limited.read_to_end(&mut decoded)?;
        if decoded.len() > max_body_bytes {
            anyhow::bail!("解压后的请求体超过大小限制");
        }
        decoded
    } else {
        anyhow::bail!("不支持的 Content-Encoding：{encoding}");
    };

    String::from_utf8(decoded)
        .map_err(|error| anyhow::anyhow!("Responses 请求体不是 UTF-8：{error}"))
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

fn dream_skin_image_response() -> (String, Vec<u8>, String, &'static str) {
    let not_found = || {
        (
            "404 Not Found".to_string(),
            serde_json::to_vec(&serde_json::json!({
                "status": "failed",
                "message": "皮肤图片未启用或图片不可用"
            }))
            .unwrap_or_default(),
            "application/json; charset=utf-8".to_string(),
            "helper.dream_skin_image_not_found",
        )
    };
    let settings = SettingsStore::default().load().unwrap_or_default();
    if !settings.codex_app_dream_skin_enabled {
        return not_found();
    }
    let image_path = PathBuf::from(settings.codex_app_dream_skin_image_path.trim());
    if image_path.as_os_str().is_empty() || !image_path.is_file() {
        let (content_type, image) = crate::assets::dream_skin_default_image();
        return (
            "200 OK".to_string(),
            image.to_vec(),
            content_type.to_string(),
            "helper.dream_skin_default_image_ok",
        );
    }
    let Some(content_type) = overlay_image_content_type(&image_path) else {
        return not_found();
    };
    match std::fs::read(&image_path) {
        Ok(bytes) => (
            "200 OK".to_string(),
            bytes,
            content_type.to_string(),
            "helper.dream_skin_image_ok",
        ),
        Err(_) => not_found(),
    }
}

fn logical_cursor_position() -> anyhow::Result<(i32, i32)> {
    #[cfg(windows)]
    {
        return windows_logical_cursor_position();
    }
    #[cfg(target_os = "macos")]
    {
        return macos_logical_cursor_position();
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        anyhow::bail!("logical cursor position is only available on Windows and macOS")
    }
}

/// CoreGraphics 全局坐标：主屏左上为原点、Y 向下、单位是点（DIP），
/// 与 Electron 的 window.screenX/screenY 一致。不要用 NSEvent.mouseLocation（左下原点）。
#[cfg(target_os = "macos")]
fn macos_logical_cursor_position() -> anyhow::Result<(i32, i32)> {
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGPoint {
        x: f64,
        y: f64,
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CGEventCreate(source: *const std::ffi::c_void) -> *mut std::ffi::c_void;
        fn CGEventGetLocation(event: *mut std::ffi::c_void) -> CGPoint;
        fn CFRelease(cf: *mut std::ffi::c_void);
    }
    unsafe {
        let event = CGEventCreate(std::ptr::null());
        if event.is_null() {
            anyhow::bail!("CGEventCreate failed");
        }
        let point = CGEventGetLocation(event);
        CFRelease(event);
        Ok((point.x.round() as i32, point.y.round() as i32))
    }
}

#[cfg(all(test, target_os = "macos"))]
#[test]
fn macos_logical_cursor_position_uses_points_from_core_graphics() {
    let (x, y) = macos_logical_cursor_position().expect("CGEventGetLocation should succeed");
    let _ = (x, y);
}

#[cfg(windows)]
fn windows_logical_cursor_position() -> anyhow::Result<(i32, i32)> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::HiDpi::{
        DPI_AWARENESS_CONTEXT_UNAWARE_GDISCALED, SetThreadDpiAwarenessContext,
    };
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

    let previous = unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_UNAWARE_GDISCALED) };
    if previous.0.is_null() {
        anyhow::bail!("SetThreadDpiAwarenessContext failed");
    }
    let mut point = POINT::default();
    let result = unsafe { GetCursorPos(&mut point) };
    unsafe {
        SetThreadDpiAwarenessContext(previous);
    }
    result.ok().context("GetCursorPos failed")?;
    Ok((point.x, point.y))
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
    let mut upstream = match crate::protocol_proxy::open_models_proxy_request(request_user_agent)
        .await
    {
        Ok(upstream) => upstream,
        Err(error) => {
            let body = serde_json::to_vec(
                &serde_json::json!({                 "status": "failed",                 "message": error.to_string()             }),
            )?;
            write_http_response(
                stream,
                "502 Bad Gateway",
                "application/json; charset=utf-8",
                &body,
            )
            .await?;
            log_helper_proxy_failure(
                "helper.models_proxy_failed",
                method,
                path,
                &error.to_string(),
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
    let body = upstream.read_body().await?;
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
/// 单次协议代理请求的日志草稿。
///
/// 用 Drop 兜底写盘：这个函数分支很多（错误、非 2xx、原生压缩、流式、协议转换），
/// 每个分支都手写一遍落盘容易漏；字段在分支里按需补齐，析构时统一写一条记录。
struct ProxyRequestLog {
    started: std::time::Instant,
    entry: crate::request_log::RequestLogEntry,
    written: bool,
}

impl ProxyRequestLog {
    fn begin(
        method: &str,
        path: &str,
        request: Option<&Value>,
        thread_id: Option<String>,
    ) -> Self {
        let started_at_ms = current_time_ms();
        let model = request
            .and_then(|request| request.get("model"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let log = Self {
            started: std::time::Instant::now(),
            entry: crate::request_log::RequestLogEntry {
                id: crate::request_log::RequestLogEntry::new_id(started_at_ms),
                timestamp_ms: started_at_ms,
                started_at_ms,
                duration_ms: 0,
                method: method.to_string(),
                path: path.to_string(),
                status_code: 0,
                success: false,
                stream: false,
                model,
                thread_id,
                upstream_model: String::new(),
                relay_id: String::new(),
                relay_name: String::new(),
                wire_api: String::new(),
                attempts: 0,
                auto_retries: 0,
                retry_reasons: Vec::new(),
                attempt_details: Vec::new(),
                state: Some("running".to_string()),
                first_token_ms: None,
                estimated_output_tokens: None,
                phase: Some("requesting".to_string()),
                error: None,
                stream_outcome: None,
                usage: None,
                cache_hit_ratio: None,
            },
            written: false,
        };
        crate::request_log::start_request(&log.entry);
        log
    }

    fn record_upstream(&mut self, upstream: &crate::protocol_proxy::UpstreamProxyResponse) {
        self.entry.status_code = upstream.status_code;
        self.entry.success = upstream.is_success();
        self.entry.stream = upstream.is_stream;
        if let Some(trace) = &upstream.request_trace {
            self.entry.relay_id = trace.relay_id.clone();
            self.entry.relay_name = trace.relay_name.clone();
            self.entry.upstream_model = trace.upstream_model.clone();
            self.entry.wire_api = trace.wire_api.clone();
            self.entry.attempts = trace.attempts;
            self.entry.auto_retries = trace.auto_retries;
            self.entry.retry_reasons = trace.retry_reasons.clone();
            self.entry.attempt_details = trace.attempt_details.clone();
            if let Some(first_output_at_ms) = trace.first_output_at_ms {
                self.entry.first_token_ms =
                    Some(first_output_at_ms.saturating_sub(self.entry.started_at_ms));
            }
        }
        crate::request_log::update_running(&self.entry);
    }

    fn record_error(&mut self, status: &str, error: String) {
        self.entry.status_code = status
            .split_whitespace()
            .next()
            .and_then(|code| code.parse::<u16>().ok())
            .unwrap_or(0);
        self.entry.success = false;
        self.entry.error = Some(error);
    }

    fn record_usage_json(&mut self, body: &[u8]) {
        if self.entry.usage.is_none() {
            self.entry.usage = crate::request_log::usage_from_body_bytes(body);
        }
    }

    fn record_stream(
        &mut self,
        failure: Option<&str>,
        usage: Option<crate::request_log::RequestUsage>,
        failure_detail: Option<String>,
    ) {
        match failure {
            Some(reason) => {
                self.entry.success = false;
                self.entry.stream_outcome = Some(reason.to_string());
                if self.entry.error.is_none() {
                    self.entry.error = Some(failure_detail.unwrap_or_else(|| reason.to_string()));
                }
            }
            None => {
                self.entry.success = true;
                self.entry.stream_outcome = Some("ok".to_string());
            }
        }
        if usage.is_some() {
            self.entry.usage = usage;
        }
    }

    /// 记录首字耗时（从请求开始到第一段上游输出）。只记第一次。
    fn mark_first_token_at(&mut self, first_output_at_ms: u64) {
        if self.entry.first_token_ms.is_none() {
            self.entry.first_token_ms =
                Some(first_output_at_ms.saturating_sub(self.entry.started_at_ms));
            self.entry.phase = Some("generating".to_string());
            crate::request_log::update_running(&self.entry);
        }
    }

    fn write(&mut self) {
        if self.written {
            return;
        }
        self.written = true;
        let finished_at_ms = current_time_ms();
        self.entry.finish(finished_at_ms);
        // 非流式响应没有首字概念，用总耗时作为首字耗时展示。
        if !self.entry.stream && self.entry.first_token_ms.is_none() {
            self.entry.first_token_ms = Some(self.entry.duration_ms);
        }
        crate::request_log::finish_request(&self.entry);
        let _ = self.started; // 时长由 entry.finish 统一计算，保留 Instant 供调试断点。
    }
}

impl Drop for ProxyRequestLog {
    fn drop(&mut self) {
        self.write();
    }
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

async fn handle_protocol_proxy_connection(
    stream: &mut tokio::net::TcpStream,
    request_body: &str,
    request_user_agent: Option<&str>,
    request_beta_features: Option<&str>,
    session_headers: crate::protocol_proxy::ProxySessionHeaders<'_>,
    method: &str,
    path: &str,
    remote_addr_text: Option<String>,
) -> anyhow::Result<()> {
    let request_json = serde_json::from_str::<serde_json::Value>(request_body).ok();
    let request_thread_id = session_headers
        .thread_id
        .or(session_headers.session_id)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let mut request_log =
        ProxyRequestLog::begin(method, path, request_json.as_ref(), request_thread_id);
    let live_request_id = request_log.entry.id.clone();
    let mut upstream =
        match crate::protocol_proxy::open_responses_proxy_request_for_path_with_session_headers_and_live_id(
            request_body,
            request_user_agent,
            path,
            request_beta_features,
            session_headers,
            Some(live_request_id.as_str()),
        )
        .await
        {
            Ok(upstream) => upstream,
            Err(error) => {
                let unsupported =
                    error.downcast_ref::<crate::protocol_proxy::UnsupportedEncryptedAgentContent>();
                let search_unsupported =
                    error.downcast_ref::<crate::protocol_proxy::UnsupportedNativeWebSearch>();
                let file_unsupported = error.downcast_ref::<crate::protocol_proxy::UnsupportedInputFile>();
                let vlm_failure = error.downcast_ref::<crate::vision::VlmPreprocessError>();
                let status = if unsupported.is_some() || search_unsupported.is_some() || file_unsupported.is_some()
                    || matches!(vlm_failure, Some(crate::vision::VlmPreprocessError::MissingConfiguration)) {
                    "400 Bad Request"
                } else {
                    "502 Bad Gateway"
                };
                // 502 响应与诊断日志都带上完整错误链，便于直接看到真实失败原因。
                let error_chain = crate::protocol_proxy::error_chain(&error);
                let body = serde_json::to_vec(
                    &unsupported
                        .map(|error| error.response_body())
                        .or_else(|| search_unsupported.map(|error| error.response_body()))
                        .or_else(|| file_unsupported.map(|error| error.response_body()))
                        .or_else(|| vlm_failure.map(|error| error.response_body()))
                        .unwrap_or_else(
                            || serde_json::json!({"status":"failed","message":error_chain.clone()}),
                        ),
                )?;
                write_http_response(stream, status, "application/json; charset=utf-8", &body)
                    .await?;
                log_helper_proxy_failure(
                    "helper.protocol_proxy_failed",
                    method,
                    path,
                    &error_chain,
                    remote_addr_text,
                );
                request_log.record_error(status, error_chain.clone());
                stream.shutdown().await?;
                return Ok(());
            }
        };
    request_log.record_upstream(&upstream);
    if !upstream.is_success() {
        let status = upstream.status();
        let upstream_content_type = upstream.content_type.clone();
        let upstream_body = upstream.read_body().await?;
        request_log.record_usage_json(&upstream_body);
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
    if upstream.native_compaction_passthrough {
        let body = upstream.read_body().await?;
        request_log.record_usage_json(&body);
        request_log.record_stream(None, None, None);
        let content_type = if upstream.content_type.is_empty() {
            if upstream.is_stream {
                "text/event-stream; charset=utf-8"
            } else {
                "application/json; charset=utf-8"
            }
        } else {
            &upstream.content_type
        };
        write_http_response(stream, "200 OK", content_type, &body).await?;
        log_helper_response(
            "helper.protocol_proxy_compaction_native",
            method,
            path,
            "200 OK",
            remote_addr_text,
        );
        stream.shutdown().await?;
        return Ok(());
    }
    if upstream.is_stream {
        write_http_stream_headers(stream, "200 OK", "text/event-stream; charset=utf-8").await?;
        if let Some(body) = upstream.buffered_body.take() {
            // buffered 模式：整段 SSE 已由协议代理收齐并重试完成，这里一次性转发。
            let mut transformer =
                crate::protocol_proxy::ResponsesSseTransformer::for_request(request_json.as_ref());
            let out = transformer.push_bytes(&body);
            stream.write_all(&out).await?;
            let usage = transformer.take_usage();
            let detail = transformer.take_failure_detail();
            let failure = transformer.finish();
            let detail_for_log = detail.clone();
            request_log.record_stream(failure.as_deref(), usage, detail);
            if let Some(reason) = failure {
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "helper.protocol_proxy_stream_failed",
                    serde_json::json!({
                        "method": method,
                        "path": path,
                        "status": "200 OK",
                        "stream_outcome": "failed",
                        "wireApi": upstream.wire_api,
                        "buffered": true,
                        "error": reason,
                        "detail": detail_for_log,
                        "remote_addr": remote_addr_text
                    }),
                );
            } else {
                log_helper_response(
                    "helper.protocol_proxy_stream_ok",
                    method,
                    path,
                    "200 OK",
                    remote_addr_text,
                );
            }
            stream.shutdown().await?;
            return Ok(());
        }
        if upstream.compaction {
            // v2 远程压缩：无论上游协议都重组为恰好一个 compaction 输出项，
            // 压缩无增量展示诉求，收齐上游文本后一次性下发。
            // SSE 事件可能跨网络 chunk 拆开，converter 内部按事件边界缓冲。
            let mut converter = crate::protocol_proxy::CompactionSseConverter::new(
                request_json
                    .as_ref()
                    .and_then(|request| request.get("model"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(""),
            );
            let is_anthropic_upstream =
                upstream.wire_api == crate::protocol_proxy::UpstreamWireApi::AnthropicMessages;
            if upstream.wire_api != crate::protocol_proxy::UpstreamWireApi::Responses {
                converter = converter.with_chat_upstream();
            }
            // Anthropic 事件流先落成 chat 事件流，再进既有的 chat 压缩聚合器。
            let mut anthropic_bridge = is_anthropic_upstream
                .then(crate::anthropic_bridge::AnthropicSseToChatSse::default);
            let mut bytes_stream = upstream.take_response()?.bytes_stream();
            while let Some(chunk) = bytes_stream.next().await {
                match chunk {
                    Ok(bytes) => {
                        let payload = match anthropic_bridge.as_mut() {
                            Some(bridge) => bridge.push_bytes(&bytes),
                            None => bytes.to_vec(),
                        };
                        if !payload.is_empty() {
                            converter.push_upstream_bytes(&payload);
                        }
                    }
                    Err(error) => {
                        converter.fail(format!("Stream error: {error}"), None);
                        break;
                    }
                }
            }
            if let Some(bridge) = anthropic_bridge.as_mut() {
                let tail = bridge.finish();
                converter.push_upstream_bytes(&tail);
                if !bridge.has_successful_compaction_completion() {
                    converter.fail("Anthropic 压缩流缺少正常 message_stop".to_string(), Some("compaction_incomplete".to_string()));
                }
            }
            let outcome = converter.finish_with_outcome();
            request_log.record_stream(
                (!outcome.succeeded).then_some("compaction stream failed"),
                None,
                None,
            );
            stream.write_all(&outcome.payload).await?;
            log_helper_response(
                if outcome.succeeded { "helper.protocol_proxy_compaction_ok" } else { "helper.protocol_proxy_compaction_failed" },
                method,
                path,
                "200 OK",
                remote_addr_text,
            );
            stream.shutdown().await?;
            return Ok(());
        }
        if upstream.wire_api == crate::protocol_proxy::UpstreamWireApi::Responses
            && upstream.exec_adapted
        {
            let mut converter = crate::responses_exec_sse::ExecSseAdapter::new();
            let mut bytes_stream = upstream.take_response()?.bytes_stream();
            while let Some(chunk) = bytes_stream.next().await {
                let converted = match chunk {
                    Ok(bytes) => match converter.push_bytes(&bytes) {
                        Ok(converted) => converted,
                        Err(_) => converter.fail(),
                    },
                    Err(_) => converter.fail(),
                };
                stream.write_all(&converted).await?;
                if converter.is_complete() {
                    break;
                }
            }
            stream.write_all(&converter.finish()?).await?;
            let exec_succeeded = converter.succeeded();
            if let Some(outcome) = upstream.exec_outcome.as_mut() {
                outcome.finish(exec_succeeded);
            }
            request_log.record_stream((!exec_succeeded).then_some("exec stream failed"), None, None);
            stream.shutdown().await?;
            return Ok(());
        }
        let stream_prefix = std::mem::take(&mut upstream.stream_prefix);
        let (failure, usage, failure_detail, first_output_at_ms) = forward_protocol_proxy_stream(
            stream,
            &stream_prefix,
            upstream.take_response()?,
            upstream.wire_api,
            request_json.as_ref(),
        )
        .await?;
        if let Some(first_output_at_ms) = first_output_at_ms {
            request_log.mark_first_token_at(first_output_at_ms);
        }
        let failure_detail_for_log = failure_detail.clone();
        request_log.record_stream(failure.as_deref(), usage, failure_detail);
        if let Some(reason) = failure {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "helper.protocol_proxy_stream_failed",
                serde_json::json!({
                    "method": method,
                    "path": path,
                    "status": "200 OK",
                    "stream_outcome": "failed",
                    "wireApi": upstream.wire_api,
                    "compactV2": request_json.as_ref().is_some_and(crate::protocol_proxy::request_has_compaction_trigger),
                    "error": reason,
                    "detail": failure_detail_for_log,
                    "remote_addr": remote_addr_text
                }),
            );
        } else {
            log_helper_response(
                "helper.protocol_proxy_stream_ok",
                method,
                path,
                "200 OK",
                remote_addr_text,
            );
        }
        stream.shutdown().await?;
        return Ok(());
    }
    let upstream_body = match upstream.read_body().await {
        Ok(body) => body,
        Err(error) => {
            if let Some(outcome) = upstream.exec_outcome.as_mut() {
                outcome.finish(false);
            }
            request_log.record_error("502 Bad Gateway", error.to_string());
            return Err(error);
        }
    };
    request_log.record_usage_json(&upstream_body);
    if upstream.compaction {
        // v2 远程压缩非流式路径：同样重组为单个 compaction 输出项。
        if upstream.wire_api == crate::protocol_proxy::UpstreamWireApi::AnthropicMessages {
            let original: serde_json::Value = serde_json::from_slice(&upstream_body)?;
            if let Err(error) = crate::protocol_proxy::validate_compaction_json_completion(&original, upstream.wire_api) {
                let mut converter = crate::protocol_proxy::CompactionSseConverter::new("");
                converter.fail(error.to_string(), Some("compaction_incomplete".to_string()));
                write_http_response(stream, "200 OK", "text/event-stream; charset=utf-8", &converter.finish()).await?;
                stream.shutdown().await?;
                return Ok(());
            }
        }
        let compaction_input = crate::anthropic_bridge::normalize_non_stream_body_for_compaction(
            upstream.wire_api,
            &upstream_body,
        )?;
        let body = crate::protocol_proxy::wrap_non_stream_response_as_compaction(
            &compaction_input,
            request_json
                .as_ref()
                .and_then(|request| request.get("model"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or(""),
        )?;
        request_log.record_stream(None, None, None);
        write_http_response(stream, "200 OK", "text/event-stream; charset=utf-8", &body).await?;
        log_helper_response(
            "helper.protocol_proxy_compaction_ok",
            method,
            path,
            "200 OK",
            remote_addr_text,
        );
        stream.shutdown().await?;
        return Ok(());
    }
    if upstream.wire_api == crate::protocol_proxy::UpstreamWireApi::Responses {
        let body = if upstream.exec_adapted {
            let (body, valid) = crate::responses_exec_transport::adapted_json(&upstream_body);
            write_http_response(stream, "200 OK", "application/json; charset=utf-8", &body).await?;
            if let Some(outcome) = upstream.exec_outcome.as_mut() {
                outcome.finish(valid);
            }
            stream.shutdown().await?;
            return Ok(());
        } else {
            upstream_body
        };
        request_log.record_usage_json(&body);
        write_http_response(
            stream,
            "200 OK",
            if upstream.content_type.is_empty() {
                "application/json; charset=utf-8"
            } else {
                &upstream.content_type
            },
            &body,
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
    let upstream_json: serde_json::Value = serde_json::from_slice(&upstream_body)?;
    // Anthropic 上游先落成 chat 响应体，再复用同一份 chat → Responses 映射；
    // 签名思考块要挂回 Responses 的 reasoning item，下一轮才能原样回放。
    let (chat_json, thinking_blocks) =
        if upstream.wire_api == crate::protocol_proxy::UpstreamWireApi::AnthropicMessages {
            let blocks =
                crate::anthropic_bridge::thinking_blocks_from_anthropic_message(&upstream_json);
            (
                crate::anthropic_bridge::anthropic_message_to_chat_completion(upstream_json)?,
                blocks,
            )
        } else {
            (upstream_json, Vec::new())
        };
    let mut response_json = if let Some(request_json) = request_json.as_ref() {
        crate::protocol_proxy::chat_completion_to_response_with_request(chat_json, request_json)?
    } else {
        crate::protocol_proxy::chat_completion_to_response(chat_json)?
    };
    if !thinking_blocks.is_empty() {
        crate::anthropic_bridge::attach_thinking_to_responses(&mut response_json, &thinking_blocks);
    }
    let body = serde_json::to_vec(&response_json)?;
    request_log.record_usage_json(&body);
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
async fn handle_dictation_connection(
    stream: &mut tokio::net::TcpStream,
    method: &str,
    path: &str,
    headers: &str,
    body: &[u8],
    content_type: &str,
) -> anyhow::Result<()> {
    let (status, response) = if method == "OPTIONS" {
        ("204 No Content".to_string(), Vec::new())
    } else if path == "/dictation/status" && method == "GET" {
        let settings = SettingsStore::default().load().unwrap_or_default();
        (
            "200 OK".to_string(),
            serde_json::to_vec(&crate::dictation::public_status(
                &crate::dictation::effective_settings(&settings),
            ))?,
        )
    } else if path == "/dictation/transcribe" && method == "POST" {
        let token = header_value_from_headers(headers, crate::dictation::TOKEN_HEADER);
        let result = if !crate::dictation::valid_helper_token(token.as_deref()) {
            Err(crate::dictation::DictationError {
                status: 403,
                message: "语音请求未授权，请从 Codex++ 重新发起".to_string(),
            })
        } else {
            let settings = SettingsStore::default().load().unwrap_or_default();
            let effective = crate::dictation::effective_settings(&settings);
            match transcribe_until_disconnect(stream, &effective, body, content_type).await {
                Some(result) => result,
                None => return Ok(()),
            }
        };
        match result {
            Ok(value) => ("200 OK".to_string(), serde_json::to_vec(&value)?),
            Err(error) => {
                let reason = match error.status {
                    400 => "Bad Request",
                    403 => "Forbidden",
                    413 => "Payload Too Large",
                    504 => "Gateway Timeout",
                    _ => "Bad Gateway",
                };
                (
                    format!("{} {reason}", error.status),
                    serde_json::to_vec(
                        &serde_json::json!({ "status": "failed", "message": error.message }),
                    )?,
                )
            }
        }
    } else {
        (
            "405 Method Not Allowed".to_string(),
            serde_json::to_vec(
                &serde_json::json!({ "status": "failed", "message": "语音接口不支持此请求方法" }),
            )?,
        )
    };
    let response_headers = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json; charset=utf-8\r\nCache-Control: no-store\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type, X-Codex-Plus-Dictation-Token\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response.len()
    );
    stream.write_all(response_headers.as_bytes()).await?;
    stream.write_all(&response).await?;
    stream.shutdown().await?;
    Ok(())
}

async fn transcribe_until_disconnect(
    stream: &mut tokio::net::TcpStream,
    settings: &crate::settings::DictationSettings,
    body: &[u8],
    content_type: &str,
) -> Option<Result<Value, crate::dictation::DictationError>> {
    // 浏览器取消请求、关闭连接时释放 reqwest future，停止上游上传或等待。
    tokio::select! {
        result = crate::dictation::transcribe(settings, body, content_type) => Some(result),
        _ = async {
            let mut buffer = [0u8; 64];
            loop {
                match stream.read(&mut buffer).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {},
                }
            }
        } => None,
    }
}

#[cfg(test)]
mod dictation_receipt_tests {
    use super::*;

    #[test]
    fn receipt_round_trip_keeps_port_token_and_owner_only_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dictation-helper.json");
        write_dictation_helper_receipt_at(&path, 57321, "token-abc");
        let value: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(value["port"], 57321);
        assert_eq!(value["token"], "token-abc");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "凭据文件必须只对当前用户可读");
        }
        // 覆盖写不能失败：Windows 上 rename 到已存在的目标会报错。
        write_dictation_helper_receipt_at(&path, 57322, "token-def");
        let again: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(again["port"], 57322);
        assert_eq!(again["token"], "token-def");
        assert!(!path.with_extension("json.tmp").exists(), "临时文件必须被改名走");
        // 端口不匹配时不能误删：常驻 helper 与会话 helper 可能同时在跑。
        remove_dictation_helper_receipt_at_for_port(&path, 57321);
        assert!(path.exists(), "非本端口的凭据必须保留");
        remove_dictation_helper_receipt_at_for_port(&path, 57322);
        assert!(!path.exists());
    }
}

#[cfg(test)]
mod dictation_http_tests {
    use super::*;

    #[tokio::test]
    async fn dictation_helper_rejects_requests_without_capability() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, remote) = listener.accept().await.unwrap();
            handle_helper_connection(stream, Some(remote))
                .await
                .unwrap();
        });
        let mut client = tokio::net::TcpStream::connect(addr).await.unwrap();
        client.write_all(format!("POST /dictation/transcribe HTTP/1.1\r\nHost: {addr}\r\nContent-Type: multipart/form-data; boundary=test\r\nContent-Length: 0\r\n\r\n").as_bytes()).await.unwrap();
        let mut response = Vec::new();
        client.read_to_end(&mut response).await.unwrap();
        let response = String::from_utf8(response).unwrap();
        assert!(response.starts_with("HTTP/1.1 403 Forbidden"));
        assert!(!response.contains(crate::dictation::helper_token()));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn dictation_helper_preflight_allows_capability_header() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, remote) = listener.accept().await.unwrap();
            handle_helper_connection(stream, Some(remote))
                .await
                .unwrap();
        });
        let mut client = tokio::net::TcpStream::connect(addr).await.unwrap();
        client
            .write_all(
                format!("OPTIONS /dictation/transcribe HTTP/1.1\r\nHost: {addr}\r\n\r\n")
                    .as_bytes(),
            )
            .await
            .unwrap();
        let mut response = Vec::new();
        client.read_to_end(&mut response).await.unwrap();
        let response = String::from_utf8(response).unwrap();
        assert!(response.starts_with("HTTP/1.1 204 No Content"));
        assert!(response.contains(crate::dictation::TOKEN_HEADER));
        assert!(!response.contains(crate::dictation::helper_token()));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn dictation_disconnect_stops_waiting_for_upstream() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_delay(std::time::Duration::from_secs(10)))
            .mount(&upstream)
            .await;
        let settings = crate::settings::DictationSettings {
            enabled: true,
            base_url: format!("{}/v1", upstream.uri()),
            ..Default::default()
        };
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let client = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (mut stream, _) = listener.accept().await.unwrap();
        let task = tokio::spawn(async move {
            transcribe_until_disconnect(&mut stream, &settings,
                b"--test\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.webm\"\r\n\r\naudio\r\n--test--\r\n",
                "multipart/form-data; boundary=test").await
        });
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if !upstream.received_requests().await.unwrap().is_empty() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        drop(client);
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(1), task)
                .await
                .unwrap()
                .unwrap()
                .is_none()
        );
    }
}

async fn handle_audio_transcriptions_proxy_connection(
    stream: &mut tokio::net::TcpStream,
    request_body: &[u8],
    request_content_type: Option<&str>,
    request_user_agent: Option<&str>,
    method: &str,
    path: &str,
    remote_addr_text: Option<String>,
) -> anyhow::Result<()> {
    let mut upstream = match crate::protocol_proxy::open_audio_transcriptions_proxy_request(
        request_body,
        request_content_type.unwrap_or_default(),
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
            log_helper_proxy_failure(
                "helper.audio_transcriptions_proxy_failed",
                method,
                path,
                &error.to_string(),
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
    let body = upstream.read_body().await?;
    write_http_response(stream, &status, &content_type, &body).await?;
    log_helper_response(
        if is_success {
            "helper.audio_transcriptions_proxy_ok"
        } else {
            "helper.audio_transcriptions_proxy_upstream_error"
        },
        method,
        path,
        &status,
        remote_addr_text,
    );
    stream.shutdown().await?;
    Ok(())
}

async fn handle_image_proxy_connection(
    stream: &mut tokio::net::TcpStream,
    request_body: &[u8],
    request_content_type: Option<&str>,
    request_user_agent: Option<&str>,
    method: &str,
    path: &str,
    remote_addr_text: Option<String>,
) -> anyhow::Result<()> {
    let upstream = if crate::protocol_proxy::is_image_generations_proxy_path(path) {
        crate::protocol_proxy::open_image_generations_proxy_request(
            request_body,
            request_user_agent,
        )
        .await
    } else {
        crate::protocol_proxy::open_image_edits_proxy_request(
            request_body,
            request_content_type.unwrap_or_default(),
            request_user_agent,
        )
        .await
    };
    let mut upstream = match upstream {
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
            log_helper_proxy_failure(
                "helper.image_proxy_failed",
                method,
                path,
                &error.to_string(),
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
    let body = upstream.read_body().await?;
    write_http_response(stream, &status, &content_type, &body).await?;
    log_helper_response(
        if is_success {
            "helper.image_proxy_ok"
        } else {
            "helper.image_proxy_upstream_error"
        },
        method,
        path,
        &status,
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
    let mut upstream = match crate::protocol_proxy::open_chat_completions_proxy_request(
        request_body,
        request_user_agent,
    )
    .await
    {
        Ok(upstream) => upstream,
        Err(error) => {
            let body = serde_json::to_vec(
                &serde_json::json!({                 "status": "failed",                 "message": error.to_string()             }),
            )?;
            write_http_response(
                stream,
                "502 Bad Gateway",
                "application/json; charset=utf-8",
                &body,
            )
            .await?;
            log_helper_proxy_failure(
                "helper.chat_completions_proxy_failed",
                method,
                path,
                &error.to_string(),
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
        let mut bytes_stream = upstream.take_response()?.bytes_stream();
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
    let body = upstream.read_body().await?;
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

// 返回流失败原因与（可选的）usage；HTTP 响应头已经发送，不能再把失败伪装成第二个 HTTP 502。
async fn forward_protocol_proxy_stream(
    stream: &mut tokio::net::TcpStream,
    prefix: &[u8],
    response: reqwest::Response,
    wire_api: crate::protocol_proxy::UpstreamWireApi,
    request: Option<&Value>,
) -> anyhow::Result<(
    Option<String>,
    Option<crate::request_log::RequestUsage>,
    Option<String>,
    Option<u64>,
)> {
    let mut chunks = response.bytes_stream();
    let mut first_output_at_ms = None;
    if wire_api == crate::protocol_proxy::UpstreamWireApi::Responses {
        let mut transformer = crate::protocol_proxy::ResponsesSseTransformer::for_request(request);
        // 代理预读过的开头必须先原样补写，否则 Codex 会丢掉 response.created 等事件。
        if !prefix.is_empty() {
            let out = transformer.push_bytes(prefix);
            stream.write_all(&out).await?;
            if transformer.has_failed() {
                let usage = transformer.take_usage();
                let detail = transformer.take_failure_detail();
                return Ok((transformer.finish(), usage, detail, first_output_at_ms));
            }
        }
        while let Some(chunk) = chunks.next().await {
            match chunk {
                Ok(bytes) => {
                    if !bytes.is_empty() && first_output_at_ms.is_none() {
                        first_output_at_ms = Some(current_time_ms());
                    }
                    let out = transformer.push_bytes(&bytes);
                    stream.write_all(&out).await?;
                    // 超限等终态失败要立即结束流，不能等上游 EOF；
                    // 否则上游保持连接时失败迟迟不上报，残片也会继续被转发。
                    if transformer.has_failed() {
                        let usage = transformer.take_usage();
                        let detail = transformer.take_failure_detail();
                        return Ok((transformer.finish(), usage, detail, first_output_at_ms));
                    }
                }
                Err(error) => {
                    let usage = transformer.take_usage();
                    let detail = transformer.take_failure_detail();
                    return Ok((
                        Some(error.without_url().to_string()),
                        usage,
                        detail,
                        first_output_at_ms,
                    ));
                }
            }
        }
        let usage = transformer.take_usage();
        let detail = transformer.take_failure_detail();
        return Ok((transformer.finish(), usage, detail, first_output_at_ms));
    }
    if wire_api == crate::protocol_proxy::UpstreamWireApi::AnthropicMessages {
        // Anthropic SSE → chat SSE → Responses SSE，终端事件语义与 chat 上游一致。
        let mut converter = request
            .map(crate::anthropic_bridge::AnthropicSseToResponsesConverter::with_request)
            .unwrap_or_default();
        while let Some(chunk) = chunks.next().await {
            match chunk {
                Ok(bytes) => {
                    if !bytes.is_empty() && first_output_at_ms.is_none() {
                        first_output_at_ms = Some(current_time_ms());
                    }
                    let converted = converter.push_bytes(&bytes);
                    if !converted.is_empty() {
                        stream.write_all(&converted).await?;
                    }
                    if converter.has_failed() {
                        return Ok((
                            Some("上游返回了流式错误事件".to_string()),
                            None,
                            None,
                            first_output_at_ms,
                        ));
                    }
                }
                Err(error) => {
                    let failed = converter.fail(
                        "上游响应流读取失败".to_string(),
                        Some("stream_error".to_string()),
                    );
                    stream.write_all(&failed).await?;
                    return Ok((
                        Some(error.without_url().to_string()),
                        None,
                        None,
                        first_output_at_ms,
                    ));
                }
            }
        }
        // Anthropic 的 `message_stop` 就是完成事件；缺它说明流被掐断，
        // 已发出的文本和工具增量不能自动重放。
        if !converter.has_terminal_event() {
            let failed = converter.fail(
                "上游在完成事件前结束了响应流".to_string(),
                Some("stream_incomplete".to_string()),
            );
            stream.write_all(&failed).await?;
            return Ok((
                Some("上游缺少完成事件".to_string()),
                None,
                None,
                first_output_at_ms,
            ));
        }
        stream.write_all(&converter.finish()).await?;
        return Ok((None, None, None, first_output_at_ms));
    }
    let mut converter = request
        .map(crate::protocol_proxy::ChatSseToResponsesConverter::with_request)
        .unwrap_or_default();
    while let Some(chunk) = chunks.next().await {
        match chunk {
            Ok(bytes) => {
                if !bytes.is_empty() && first_output_at_ms.is_none() {
                    first_output_at_ms = Some(current_time_ms());
                }
                let converted = converter.push_bytes(&bytes);
                stream.write_all(&converted).await?;
                if converter.has_failed() {
                    return Ok((
                        Some("上游返回了流式错误事件".to_string()),
                        None,
                        None,
                        first_output_at_ms,
                    ));
                }
            }
            Err(error) => {
                let failed = converter.fail(
                    "上游响应流读取失败".to_string(),
                    Some("stream_error".to_string()),
                );
                stream.write_all(&failed).await?;
                return Ok((
                    Some(error.without_url().to_string()),
                    None,
                    None,
                    first_output_at_ms,
                ));
            }
        }
    }
    // EOF 不是 Chat 协议的完成事件；已发出的文本和工具增量不能自动重放。
    if !converter.has_terminal_event() {
        let failed = converter.fail(
            "上游在完成事件前结束了响应流".to_string(),
            Some("stream_incomplete".to_string()),
        );
        stream.write_all(&failed).await?;
        return Ok((
            Some("上游缺少完成事件".to_string()),
            None,
            None,
            first_output_at_ms,
        ));
    }
    stream.write_all(&converter.finish()).await?;
    Ok((None, None, None, first_output_at_ms))
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

/// 代理失败（502）专用日志：在通用字段之外带上**失败原因**。
///
/// 这些分支把 error 写进了响应体，旧实现却只记 method/path/status，于是
/// 客户端只看到「unexpected status 502 Bad Gateway: Unknown error」，
/// 日志里也查不到为什么（issue #2385）。原因必须落进日志，否则
/// 「上游不可达 / 没配可用中转 / 请求体解析失败」这几种完全不同的故障
/// 在现场长得一模一样。
fn log_helper_proxy_failure(
    event: &str,
    method: &str,
    path: &str,
    error: &str,
    remote_addr_text: Option<String>,
) {
    let _ = crate::diagnostic_log::append_diagnostic_log(
        event,
        serde_json::json!({
            "method": method,
            "path": path,
            "status": "502 Bad Gateway",
            "error": error,
            "remote_addr": remote_addr_text
        }),
    );
}

#[cfg(test)]
mod computer_use_tests {
    use super::{header_value_from_headers, overlay_image_content_type};
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
            header_value_from_headers(request, "user-agent").as_deref(),
            Some("Codex/26.614")
        );
    }
}

const MAX_HTTP_HEADER_BYTES: usize = 64 * 1024;
/// 默认请求体上限 128 MiB；与设置项默认值共用同一组常量，避免两处漂移。
const DEFAULT_HTTP_BODY_BYTES: usize =
    crate::settings::DEFAULT_HELPER_MAX_REQUEST_BODY_MIB as usize * 1024 * 1024;
/// 允许的最低请求体上限 32 MiB；更低会误伤正常请求。
const MIN_HTTP_BODY_BYTES: usize =
    crate::settings::MIN_HELPER_MAX_REQUEST_BODY_MIB as usize * 1024 * 1024;
/// 允许的最高请求体上限 512 MiB；helper 会整段缓冲，再高内存开销不可控。
const MAX_HTTP_BODY_BYTES: usize =
    crate::settings::MAX_HELPER_MAX_REQUEST_BODY_MIB as usize * 1024 * 1024;

/// 运行期生效的请求体上限（字节）。0 表示设置尚未应用，回落到默认值。
static CONFIGURED_HTTP_BODY_BYTES: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// 把设置里的请求体上限应用给正在运行的 helper。
///
/// 启动流程在 helper 绑定端口前调用；管理器保存设置后也会调用，这样调大上限
/// 不必重启 Codex++/Codex 就能让下一次请求生效。超出 32~512 MiB 的值会被夹取。
pub fn set_http_body_limit_bytes(bytes: usize) {
    CONFIGURED_HTTP_BODY_BYTES.store(
        normalize_http_body_bytes(bytes),
        std::sync::atomic::Ordering::Relaxed,
    );
}

fn normalize_http_body_bytes(bytes: usize) -> usize {
    bytes.clamp(MIN_HTTP_BODY_BYTES, MAX_HTTP_BODY_BYTES)
}

/// 解析环境变量里的字节数：低于下限视为无效（打错不缩小上限），高于上限则夹取。
fn parse_env_http_body_bytes(value: &str) -> Option<usize> {
    value
        .trim()
        .parse::<usize>()
        .ok()
        .filter(|bytes| *bytes >= MIN_HTTP_BODY_BYTES)
        .map(normalize_http_body_bytes)
}

/// 解析最终生效的请求体上限：环境变量优先，其次设置，最后默认值。
fn resolve_http_body_bytes(env_value: Option<&str>, setting_bytes: usize) -> usize {
    env_value
        .and_then(parse_env_http_body_bytes)
        .unwrap_or_else(|| normalize_http_body_bytes(setting_bytes))
}

fn configured_http_body_bytes() -> usize {
    let stored = CONFIGURED_HTTP_BODY_BYTES.load(std::sync::atomic::Ordering::Relaxed);
    let setting_bytes = if stored == 0 {
        DEFAULT_HTTP_BODY_BYTES
    } else {
        stored
    };
    resolve_http_body_bytes(
        std::env::var("CODEX_PLUS_MAX_HTTP_BODY_BYTES")
            .ok()
            .as_deref(),
        setting_bytes,
    )
}

#[derive(Debug, Clone, Copy)]
struct HttpBodyLimits {
    decoded: usize,
    encoded: usize,
}

fn http_body_limits() -> HttpBodyLimits {
    http_body_limits_from(configured_http_body_bytes())
}

fn http_body_limits_from(decoded: usize) -> HttpBodyLimits {
    let decoded = normalize_http_body_bytes(decoded);
    HttpBodyLimits {
        decoded,
        // 请求体可能带压缩（zstd 等），编码后允许到解码上限的 2 倍，封顶最高上限。
        encoded: decoded.saturating_mul(2).min(MAX_HTTP_BODY_BYTES),
    }
}

struct HttpRequest {
    headers: Vec<u8>,
    body: Vec<u8>,
}

#[derive(Debug)]
struct HttpRequestReadError {
    status: &'static str,
    message: String,
}

impl HttpRequestReadError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: "400 Bad Request",
            message: message.into(),
        }
    }

    fn payload_too_large_with_limit(limit: usize) -> Self {
        let limit_mib = limit / (1024 * 1024);
        Self {
            status: "413 Payload Too Large",
            message: format!(
                "HTTP 请求体超过 {limit_mib} MiB（{limit} 字节）上限：请在 Codex++「代理设置」调大「请求体上限」，或设置环境变量 CODEX_PLUS_MAX_HTTP_BODY_BYTES"
            ),
        }
    }

    fn status(&self) -> &'static str {
        self.status
    }
}

impl std::fmt::Display for HttpRequestReadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for HttpRequestReadError {}

impl From<std::io::Error> for HttpRequestReadError {
    fn from(error: std::io::Error) -> Self {
        Self::bad_request(format!("读取 HTTP 请求失败: {error}"))
    }
}

#[derive(Debug)]
enum HttpBodyFraming {
    Empty,
    ContentLength(usize),
    Chunked,
}

#[derive(Debug)]
enum ChunkedBody {
    Incomplete,
    Complete(Vec<u8>),
}

#[derive(Debug)]
enum ChunkedBodyScan {
    Incomplete,
    Complete,
}

#[derive(Default)]
struct ChunkedScanState {
    position: usize,
    decoded_len: usize,
    complete: bool,
}

impl ChunkedScanState {
    fn advance(&mut self, encoded: &[u8]) -> Result<ChunkedBodyScan, HttpRequestReadError> {
        self.advance_with_limit(encoded, DEFAULT_HTTP_BODY_BYTES)
    }

    fn advance_with_limit(
        &mut self,
        encoded: &[u8],
        max_body_bytes: usize,
    ) -> Result<ChunkedBodyScan, HttpRequestReadError> {
        if self.complete {
            return Ok(ChunkedBodyScan::Complete);
        }
        loop {
            let chunk_start = self.position;
            let Some(line_end_offset) = encoded[chunk_start..]
                .windows(2)
                .position(|window| window == b"\r\n")
            else {
                if encoded.len().saturating_sub(chunk_start) > MAX_HTTP_HEADER_BYTES {
                    return Err(HttpRequestReadError::bad_request("chunk size 行过大"));
                }
                return Ok(ChunkedBodyScan::Incomplete);
            };
            if line_end_offset > MAX_HTTP_HEADER_BYTES {
                return Err(HttpRequestReadError::bad_request("chunk size 行过大"));
            }
            let line_end = chunk_start + line_end_offset;
            let size_text = std::str::from_utf8(&encoded[chunk_start..line_end])
                .map_err(|_| HttpRequestReadError::bad_request("chunk size 不是有效 ASCII"))?;
            let size_token = size_text.split(';').next().unwrap_or_default().trim();
            let chunk_size = usize::from_str_radix(size_token, 16)
                .map_err(|_| HttpRequestReadError::bad_request("chunk size 无效"))?;
            let data_start = line_end + 2;

            if chunk_size == 0 {
                let mut trailer_start = data_start;
                loop {
                    let Some(trailer_end_offset) = encoded[trailer_start..]
                        .windows(2)
                        .position(|window| window == b"\r\n")
                    else {
                        if encoded.len().saturating_sub(data_start) > MAX_HTTP_HEADER_BYTES {
                            return Err(HttpRequestReadError::bad_request("chunk trailer 过大"));
                        }
                        return Ok(ChunkedBodyScan::Incomplete);
                    };
                    if trailer_start + trailer_end_offset - data_start > MAX_HTTP_HEADER_BYTES {
                        return Err(HttpRequestReadError::bad_request("chunk trailer 过大"));
                    }
                    if trailer_end_offset == 0 {
                        self.position = trailer_start + 2;
                        self.complete = true;
                        return Ok(ChunkedBodyScan::Complete);
                    }
                    trailer_start += trailer_end_offset + 2;
                }
            }
            let next_decoded_len = self
                .decoded_len
                .checked_add(chunk_size)
                .ok_or_else(|| HttpRequestReadError::payload_too_large_with_limit(max_body_bytes))?;
            if next_decoded_len > max_body_bytes {
                return Err(HttpRequestReadError::payload_too_large_with_limit(max_body_bytes));
            }
            let chunk_end = data_start
                .checked_add(chunk_size)
                .ok_or_else(|| HttpRequestReadError::payload_too_large_with_limit(max_body_bytes))?;
            if encoded.len() < chunk_end + 2 {
                return Ok(ChunkedBodyScan::Incomplete);
            }
            if &encoded[chunk_end..chunk_end + 2] != b"\r\n" {
                return Err(HttpRequestReadError::bad_request("chunk 数据后缺少 CRLF"));
            }
            self.decoded_len = next_decoded_len;
            self.position = chunk_end + 2;
        }
    }
}

async fn read_http_request(
    stream: &mut tokio::net::TcpStream,
) -> Result<HttpRequest, HttpRequestReadError> {
    read_http_request_with_limits(stream, http_body_limits()).await
}

async fn read_http_request_with_limits(
    stream: &mut tokio::net::TcpStream,
    limits: HttpBodyLimits,
) -> Result<HttpRequest, HttpRequestReadError> {
    let mut buffer = Vec::new();
    let mut chunk = vec![0_u8; 4096];
    let mut header_end = None;
    let mut framing = HttpBodyFraming::Empty;
    let mut chunked_scan = ChunkedScanState::default();

    loop {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if header_end.is_none() {
            header_end = find_header_end(&buffer);
            if let Some(end) = header_end {
                if end > MAX_HTTP_HEADER_BYTES {
                    return Err(HttpRequestReadError::bad_request("HTTP 请求头过大"));
                }
                framing = http_body_framing(&buffer[..end])?;
            } else if buffer.len() > MAX_HTTP_HEADER_BYTES {
                return Err(HttpRequestReadError::bad_request("HTTP 请求头过大"));
            }
        }
        if let Some(end) = header_end {
            let body = &buffer[end + 4..];
            if body.len() > limits.encoded {
                return Err(HttpRequestReadError::payload_too_large_with_limit(limits.decoded));
            }
            match framing {
                HttpBodyFraming::Empty => break,
                HttpBodyFraming::ContentLength(content_length) => {
                    if content_length > limits.decoded {
                        return Err(HttpRequestReadError::payload_too_large_with_limit(limits.decoded));
                    }
                    if body.len() >= content_length {
                        break;
                    }
                }
                HttpBodyFraming::Chunked => match chunked_scan.advance_with_limit(body, limits.decoded)? {
                    ChunkedBodyScan::Incomplete => {}
                    ChunkedBodyScan::Complete => break,
                },
            }
        }
    }

    let header_end =
        header_end.ok_or_else(|| HttpRequestReadError::bad_request("HTTP 请求头不完整"))?;
    let headers = buffer[..header_end].to_vec();
    let encoded_body = &buffer[header_end + 4..];
    let body = match framing {
        HttpBodyFraming::Empty => Vec::new(),
        HttpBodyFraming::ContentLength(content_length) => {
            content_length_body_with_limit(encoded_body, content_length, limits.decoded)?
        }
        HttpBodyFraming::Chunked => match decode_chunked_body_with_limit(encoded_body, limits.decoded)? {
            ChunkedBody::Complete(body) => body,
            ChunkedBody::Incomplete => {
                return Err(HttpRequestReadError::bad_request(
                    "chunked HTTP 请求体不完整",
                ));
            }
        },
    };

    Ok(HttpRequest { headers, body })
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn http_body_framing(headers: &[u8]) -> Result<HttpBodyFraming, HttpRequestReadError> {
    let text = String::from_utf8_lossy(headers);
    let mut content_length = None;
    let mut transfer_encoding: Option<String> = None;
    for line in text.lines().skip(1) {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("content-length") {
            let parsed = value
                .trim()
                .parse::<usize>()
                .map_err(|_| HttpRequestReadError::bad_request("Content-Length 无效"))?;
            if content_length
                .replace(parsed)
                .is_some_and(|existing| existing != parsed)
            {
                return Err(HttpRequestReadError::bad_request(
                    "存在冲突的 Content-Length 请求头",
                ));
            }
        } else if name.trim().eq_ignore_ascii_case("transfer-encoding") {
            let value = value.trim().to_ascii_lowercase();
            if let Some(existing) = transfer_encoding.as_mut() {
                existing.push(',');
                existing.push_str(&value);
            } else {
                transfer_encoding = Some(value);
            }
        }
    }

    if transfer_encoding.is_some() && content_length.is_some() {
        return Err(HttpRequestReadError::bad_request(
            "Transfer-Encoding 与 Content-Length 不能同时使用",
        ));
    }
    match transfer_encoding.as_deref() {
        Some("chunked") => Ok(HttpBodyFraming::Chunked),
        Some(_) => Err(HttpRequestReadError::bad_request(
            "仅支持 Transfer-Encoding: chunked",
        )),
        None => Ok(content_length
            .map(HttpBodyFraming::ContentLength)
            .unwrap_or(HttpBodyFraming::Empty)),
    }
}

fn content_length_body(
    encoded: &[u8],
    content_length: usize,
) -> Result<Vec<u8>, HttpRequestReadError> {
    content_length_body_with_limit(encoded, content_length, DEFAULT_HTTP_BODY_BYTES)
}

fn content_length_body_with_limit(
    encoded: &[u8],
    content_length: usize,
    max_body_bytes: usize,
) -> Result<Vec<u8>, HttpRequestReadError> {
    if content_length > max_body_bytes {
        return Err(HttpRequestReadError::payload_too_large_with_limit(max_body_bytes));
    }
    if encoded.len() < content_length {
        return Err(HttpRequestReadError::bad_request("HTTP 请求体不完整"));
    }
    Ok(encoded[..content_length].to_vec())
}

fn decode_chunked_body(encoded: &[u8]) -> Result<ChunkedBody, HttpRequestReadError> {
    decode_chunked_body_with_limit(encoded, DEFAULT_HTTP_BODY_BYTES)
}

fn decode_chunked_body_with_limit(
    encoded: &[u8],
    max_body_bytes: usize,
) -> Result<ChunkedBody, HttpRequestReadError> {
    let mut decoded = Vec::new();
    let mut position = 0;
    loop {
        let Some(line_end_offset) = encoded[position..]
            .windows(2)
            .position(|window| window == b"\r\n")
        else {
            if encoded.len().saturating_sub(position) > MAX_HTTP_HEADER_BYTES {
                return Err(HttpRequestReadError::bad_request("chunk size 行过大"));
            }
            return Ok(ChunkedBody::Incomplete);
        };
        if line_end_offset > MAX_HTTP_HEADER_BYTES {
            return Err(HttpRequestReadError::bad_request("chunk size 行过大"));
        }
        let line_end = position + line_end_offset;
        let size_text = std::str::from_utf8(&encoded[position..line_end])
            .map_err(|_| HttpRequestReadError::bad_request("chunk size 不是有效 ASCII"))?;
        let size_token = size_text.split(';').next().unwrap_or_default().trim();
        let chunk_size = usize::from_str_radix(size_token, 16)
            .map_err(|_| HttpRequestReadError::bad_request("chunk size 无效"))?;
        position = line_end + 2;

        if chunk_size == 0 {
            loop {
                let Some(trailer_end_offset) = encoded[position..]
                    .windows(2)
                    .position(|window| window == b"\r\n")
                else {
                    if encoded.len().saturating_sub(line_end + 2) > MAX_HTTP_HEADER_BYTES {
                        return Err(HttpRequestReadError::bad_request("chunk trailer 过大"));
                    }
                    return Ok(ChunkedBody::Incomplete);
                };
                if position + trailer_end_offset - (line_end + 2) > MAX_HTTP_HEADER_BYTES {
                    return Err(HttpRequestReadError::bad_request("chunk trailer 过大"));
                }
                if trailer_end_offset == 0 {
                    return Ok(ChunkedBody::Complete(decoded));
                }
                position += trailer_end_offset + 2;
            }
        }
        if decoded.len().saturating_add(chunk_size) > max_body_bytes {
            return Err(HttpRequestReadError::payload_too_large_with_limit(max_body_bytes));
        }
        let chunk_end = position
            .checked_add(chunk_size)
            .ok_or_else(|| HttpRequestReadError::payload_too_large_with_limit(max_body_bytes))?;
        if encoded.len() < chunk_end + 2 {
            return Ok(ChunkedBody::Incomplete);
        }
        if &encoded[chunk_end..chunk_end + 2] != b"\r\n" {
            return Err(HttpRequestReadError::bad_request("chunk 数据后缺少 CRLF"));
        }
        decoded.extend_from_slice(&encoded[position..chunk_end]);
        position = chunk_end + 2;
    }
}

#[cfg(test)]
fn scan_chunked_body(encoded: &[u8]) -> Result<ChunkedBodyScan, HttpRequestReadError> {
    ChunkedScanState::default().advance(encoded)
}

fn header_value_from_headers(headers: &str, header_name: &str) -> Option<String> {
    headers
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

pub fn build_codex_arguments_for_settings(
    debug_port: u16,
    settings: &BackendSettings,
) -> Vec<String> {
    build_codex_arguments(debug_port, &settings.codex_extra_args)
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

/// 用已解析好的 AUMID 拼出打包激活命令。
///
/// AUMID 的解析（含「包是否已注册」这一查询，#2140）留在调用点，这里只做纯拼接。
/// 早先本函数直接吃 `app_dir` 并自行解析，导致它在 Windows 上依赖 Store 注册表状态、
/// 无法被测试驱动——测试机上的临时目录永远不算已注册，断言只能拿到 None。
pub fn build_packaged_activation(
    app_user_model_id: &str,
    debug_port: u16,
    extra_args: &[String],
) -> CodexLaunch {
    CodexLaunch::PackagedActivation {
        app_user_model_id: app_user_model_id.to_string(),
        arguments: command_line_arguments(&build_codex_arguments(debug_port, extra_args)),
        process_id: None,
    }
}

/// 打包版（MSIX/Store）多开：在包上下文里拉起一个带独立数据目录的 Codex 实例。
///
/// 返回 `Ok(None)` 表示「这次不该走包上下文」（例如 app_dir 根本不是已注册包、
/// 或缺少可用的状态目录），调用方按原路径继续；返回 `Err` 表示包上下文这条路
/// 试过但失败了，调用方记录诊断后同样退回按路径启动。
///
/// 成功返回的 `CodexLaunch::Process` 里带的 `debug_port` 是**脚本实际使用**的端口：
/// wrapper 里重新校验过一次端口可用性，极端竞争下可能与入参不同，注入必须打到
/// 真正监听的那个端口上。
async fn launch_packaged_multi_instance(
    app_dir: &Path,
    debug_port: u16,
    extra_args: &[String],
    state_dir: Option<&Path>,
    codex_home: Option<&Path>,
    sqlite_home: Option<&Path>,
) -> anyhow::Result<Option<CodexLaunch>> {
    let executable = crate::app_paths::build_codex_executable(app_dir);
    // 只有「目录确实对应一个已注册的 MSIX 包」时才认为需要包上下文：
    // 非打包安装（免安装/自解包）本来就能按路径直接启动，别把它拖进来。
    let Some(app_user_model_id) = crate::app_paths::packaged_app_user_model_id(app_dir) else {
        return Ok(None);
    };
    let Some(state_dir) = state_dir.filter(|dir| !dir.as_os_str().is_empty()) else {
        anyhow::bail!("多开实例缺少状态目录，无法准备包上下文 wrapper");
    };

    let arguments = build_codex_arguments(debug_port, extra_args);
    let user_data_dir = multi_instance_user_data_dir_from_args(&arguments)
        .map(PathBuf::from)
        .unwrap_or_else(|| state_dir.join("user-data"));
    let resolved_codex_home = codex_home
        .map(Path::to_path_buf)
        .unwrap_or_else(|| codex_plus_default_codex_home());
    let resolved_sqlite_home = sqlite_home
        .map(Path::to_path_buf)
        .unwrap_or_else(|| resolved_codex_home.clone());

    // 端口选择在写脚本之前定：脚本里的端口就是实例身份的一部分，必须能让调用方
    // 知道（否则注入会打到旧端口）。
    let known_ports = [debug_port, crate::ports::find_available_loopback_port()];
    let wrapper = crate::packaged_launch::prepare_wrapper_script_with_port(
        state_dir,
        &executable,
        &resolved_codex_home,
        &resolved_sqlite_home,
        &user_data_dir,
        debug_port,
        &known_ports,
    )?;
    let actual_port = wrapper.debug_port;
    let wrapper_path = wrapper.path;
    match crate::packaged_launch::activate_in_package_context(&app_user_model_id, &wrapper_path)
        .await
    {
        Ok(process_id) => {
            let process_id =
                (process_id != crate::packaged_launch::UNKNOWN_PROCESS_ID).then_some(process_id);
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "launcher.packaged_multi_instance_activated",
                serde_json::json!({
                    "app_user_model_id": app_user_model_id,
                    "wrapper_path": wrapper_path,
                    "debug_port": actual_port,
                    "process_id": process_id,
                }),
            );
            Ok(Some(CodexLaunch::Process {
                command: std::iter::once(executable.to_string_lossy().to_string())
                    .chain(build_codex_arguments(actual_port, extra_args))
                    .collect(),
                wait_strategy: ProcessWaitStrategy::ExternalWaitCommand,
                macos_cleanup_policy: None,
            }))
        }
        Err(error) => {
            // 包上下文不可用时退回按路径启动：打包版上它很可能仍会失败，
            // 但至少不比改动前更差，而且失败会由多开 worker 的就绪校验如实报出。
            match crate::packaged_launch::fallback_direct_launch(
                &executable,
                &build_codex_arguments(actual_port, extra_args),
                state_dir,
            ) {
                Ok(pid) => {
                    let _ = crate::diagnostic_log::append_diagnostic_log(
                        "launcher.packaged_multi_instance_direct_fallback",
                        serde_json::json!({
                            "process_id": pid,
                            "debug_port": actual_port,
                            "message": format!("{error:#}"),
                        }),
                    );
                    Ok(Some(CodexLaunch::Process {
                        command: std::iter::once(executable.to_string_lossy().to_string())
                            .chain(build_codex_arguments(actual_port, extra_args))
                            .collect(),
                        wait_strategy: ProcessWaitStrategy::ExternalWaitCommand,
                        macos_cleanup_policy: None,
                    }))
                }
                Err(fallback_error) => Err(error.context(format!(
                    "包上下文激活失败，按路径兜底启动也失败：{fallback_error:#}"
                ))),
            }
        }
    }
}

/// 从启动参数里取出实例自己的 `--user-data-dir`（多开由管理器注入）。
fn multi_instance_user_data_dir_from_args(args: &[String]) -> Option<String> {
    args.iter()
        .find_map(|arg| arg.strip_prefix("--user-data-dir="))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn codex_plus_default_codex_home() -> PathBuf {
    crate::codex_home::default_codex_home_dir()
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

async fn try_inject_isolated_quota_gate(debug_port: u16) -> anyhow::Result<()> {
    let targets = crate::cdp::list_targets(debug_port).await?;
    let target = crate::cdp::pick_injectable_codex_page_target(&targets)?;
    let websocket_url = target
        .web_socket_debugger_url
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("selected CDP target has no websocket URL"))?;
    crate::bridge::install_isolated_api_quota_gate(websocket_url).await
}

pub async fn check_and_reinject_bridge(debug_port: u16, helper_port: u16) -> bool {
    // This one-shot entry point preserves its historical immediate-repair behavior.
    let mut health_failures = BRIDGE_HEALTH_FAILURE_THRESHOLD.saturating_sub(1);
    let mut backoff = BridgeReinjectBackoff::default();
    check_and_reinject_bridge_inner(
        debug_port,
        helper_port,
        false,
        None,
        &mut health_failures,
        &mut backoff,
    )
    .await
}

pub fn browser_identity_changed(previous: Option<&str>, current: &str) -> bool {
    previous.is_some_and(|previous| previous != current)
}

fn owned_launcher_target_alive(owned_process_alive: bool, debug_port_open: bool) -> bool {
    owned_process_alive || debug_port_open
}

#[cfg(any(windows, test))]
async fn validated_codex_browser_id(debug_port: u16) -> Option<String> {
    let targets = crate::cdp::list_targets(debug_port).await.ok()?;
    crate::cdp::pick_injectable_codex_page_target(&targets).ok()?;
    crate::cdp::browser_identity(debug_port)
        .await
        .ok()?
        .browser_id()
        .ok()
}

/// 目标「探不到」时的宽限：连续不足 3 次探测失败、或距首次失败不足 10 秒，
/// 都仍继续等待。
///
/// 起初只给 Windows 打包版用（AUMID 返回的中转进程会让探测闪断）。macOS 的
/// 退出判定原先对单次失败零容忍，代价是：`ps` 恰好在进程换代空档里采样、
/// 或者 `/json` 因监听重建短暂拒连，都会被判成「Codex 已退出」，而
/// `wait_for_codex_exit` 一旦返回就会 `shutdown_helper`——57321 随之关闭，
/// 可 Codex 还开着。此时 `config.toml` 里写死的
/// `base_url = "http://127.0.0.1:57321/v1"` 就没了对端，页面停在启动壳上、
/// 注入层还在但什么都发不出去。所以 macOS 的退出等待也走同一套宽限。
///
/// 「同端口的新实例不能为旧 launcher 续命」这条约束仍然成立：宽限是有限次的，
/// 真退出只是晚约 10 秒被发现。
#[derive(Default)]
struct PackagedExitGrace {
    missing_since: Option<std::time::Instant>,
    missed_probes: u32,
}

impl PackagedExitGrace {
    fn keep_waiting(
        &mut self,
        process_alive: Option<bool>,
        cdp_available: bool,
        now: std::time::Instant,
    ) -> bool {
        // 已捕获进程退出后，同端口的新实例不能为旧 launcher 续命。
        if process_alive.unwrap_or(cdp_available) {
            self.missing_since = None;
            self.missed_probes = 0;
            return true;
        }
        self.missed_probes = self.missed_probes.saturating_add(1);
        let since = *self.missing_since.get_or_insert(now);
        self.missed_probes < 3
            || now.saturating_duration_since(since) < std::time::Duration::from_secs(10)
    }
}

#[cfg(any(windows, test))]
fn activation_process_is_startup_evidence(alive: bool, elapsed: std::time::Duration) -> bool {
    // AUMID 有时返回仍存活的中转进程；它只能给实际调试进程有限的出现时间。
    alive && elapsed < std::time::Duration::from_secs(30)
}

#[cfg(any(windows, test))]
struct PackagedBrowserBinding {
    initial_id: Option<String>,
    replaced: bool,
}

#[cfg(any(windows, test))]
impl PackagedBrowserBinding {
    fn owns(&mut self, current_id: Option<&str>) -> bool {
        if self.replaced {
            return false;
        }
        match (self.initial_id.as_deref(), current_id) {
            (Some(initial), Some(current)) if initial == current => true,
            (Some(_), Some(_)) => {
                self.replaced = true;
                false
            }
            _ => false,
        }
    }
}

#[cfg(windows)]
async fn wait_for_windows_packaged_exit(
    executable: &Path,
    activation_pid: Option<u32>,
    activation: Option<crate::windows_integration::TrackedWindowsProcess>,
    identity: PackagedLaunchIdentity,
    debug_port: u16,
) -> anyhow::Result<()> {
    use crate::windows_integration::{TrackedWindowsProcess, capture_debug_listener};
    let mut tracked = identity.process;
    let mut browser = PackagedBrowserBinding {
        initial_id: identity.browser_id,
        replaced: false,
    };
    let mut grace = PackagedExitGrace::default();
    let mut last_discovery_error: Option<String> = None;
    let started_waiting = std::time::Instant::now();
    loop {
        let mut owned_browser = false;
        if tracked.is_none() {
            let current_browser_id = if browser.initial_id.is_some() && !browser.replaced {
                crate::cdp::browser_identity(debug_port)
                    .await
                    .ok()
                    .and_then(|identity| identity.browser_id().ok())
            } else {
                None
            };
            owned_browser = browser.owns(current_browser_id.as_deref());
            let discovery = if browser.replaced {
                Ok(None)
            } else {
                capture_debug_listener(debug_port, executable).map(|process| {
                    process.filter(|process| {
                        // 重新查询只能沿用初始 browser，或启动时已持有的同一个内核进程。
                        owned_browser
                            || activation.as_ref().is_some_and(|activation| {
                                activation.process_id == process.process_id
                                    && activation.birth_id == process.birth_id
                            })
                    })
                })
            };
            let discovery = match discovery {
                Ok(Some(process)) => {
                    let activation_matches = activation.as_ref().is_some_and(|activation| {
                        activation.process_id == process.process_id
                            && activation.birth_id == process.birth_id
                    });
                    if !activation_matches {
                        let confirmation = crate::cdp::browser_identity(debug_port)
                            .await
                            .ok()
                            .and_then(|identity| identity.browser_id().ok());
                        owned_browser = browser.owns(confirmation.as_deref());
                    }
                    Ok((activation_matches || owned_browser).then_some(process))
                }
                other => other,
            };
            match discovery {
                Ok(Some(process)) => {
                    let _ = crate::diagnostic_log::append_diagnostic_log(
                        "launcher.packaged_process_tracked",
                        serde_json::json!({ "debug_port": debug_port, "process_id": process.process_id, "birth_id": process.birth_id, "activation_pid": activation_pid }),
                    );
                    tracked = Some(process);
                }
                Ok(None) => {}
                Err(error) => {
                    let message = format!("{error:#}");
                    if last_discovery_error.as_deref() != Some(&message) {
                        let _ = crate::diagnostic_log::append_diagnostic_log(
                            "launcher.packaged_process_identity_unavailable",
                            serde_json::json!({ "debug_port": debug_port, "message": message }),
                        );
                    }
                    last_discovery_error = Some(message);
                }
            }
        }
        let process_alive = if let Some(process) = &tracked {
            Some(process.is_alive()?)
        } else if browser.replaced {
            Some(false)
        } else {
            // AUMID PID 只作等待实际 listener 出现期间的退路，不能替换已捕获主进程。
            activation
                .as_ref()
                .map(TrackedWindowsProcess::is_alive)
                .transpose()?
                .filter(|alive| {
                    activation_process_is_startup_evidence(*alive, started_waiting.elapsed())
                })
        };
        if !grace.keep_waiting(process_alive, owned_browser, std::time::Instant::now()) {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "launcher.packaged_target_exited",
                serde_json::json!({
                    "debug_port": debug_port,
                    "activation_pid": activation_pid,
                    "process_id": tracked.as_ref().map(|process| process.process_id),
                    "process_alive": process_alive,
                    "owned_browser_available": owned_browser,
                    "browser_identity_bound": browser.initial_id.is_some(),
                    "browser_replaced": browser.replaced,
                    "missed_probes": grace.missed_probes,
                    "grace_ms": 10000,
                    "identity_error": last_discovery_error,
                    "activation_grace_ms": 30000,
                }),
            );
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

/// macOS `open` is not the app's parent, so its tracked child exits immediately.
/// Match the exact debug port in the app process command line instead.
#[cfg(target_os = "macos")]
fn launcher_owned_process_alive(launch: &CodexLaunch, debug_port: u16) -> bool {
    matches!(
        launch,
        CodexLaunch::Process {
            wait_strategy: ProcessWaitStrategy::ExternalWaitCommand,
            ..
        }
    ) && !crate::watcher::find_macos_codex_processes_for_debug_port(debug_port).is_empty()
}

#[cfg(not(target_os = "macos"))]
fn launcher_owned_process_alive(_launch: &CodexLaunch, _debug_port: u16) -> bool {
    false
}

fn should_reinject_after_health_result(
    healthy: Option<bool>,
    browser_identity_changed: bool,
    health_failures: &mut u8,
) -> bool {
    let Some(healthy) = healthy else {
        *health_failures = 0;
        return false;
    };
    if healthy {
        *health_failures = 0;
        return false;
    }
    if browser_identity_changed {
        *health_failures = BRIDGE_HEALTH_FAILURE_THRESHOLD;
    } else {
        *health_failures = health_failures.saturating_add(1);
    }
    *health_failures >= BRIDGE_HEALTH_FAILURE_THRESHOLD
}

async fn check_and_reinject_bridge_inner(
    debug_port: u16,
    helper_port: u16,
    browser_identity_changed: bool,
    bridge_reinjector: Option<BridgeReinjector>,
    health_failures: &mut u8,
    backoff: &mut BridgeReinjectBackoff,
) -> bool {
    let healthy = if browser_identity_changed {
        Some(false)
    } else {
        match bridge_health_ok(debug_port).await {
            Ok(healthy) => Some(healthy),
            Err(error) => {
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "bridge.health_check_failed",
                    serde_json::json!({
                        "debug_port": debug_port,
                        "helper_port": helper_port,
                        "message": error.to_string()
                    }),
                );
                // A CDP timeout only means that the renderer did not answer
                // this probe in time. The bridge heartbeat is the source of
                // truth for actual availability; do not reinject on an
                // indeterminate CDP result or a busy page will cause churn.
                None
            }
        }
    };
    match healthy {
        // 持续健康一段时间才清掉退避（见 BRIDGE_REINJECT_BACKOFF_RESET_AFTER_SECS）；
        // 探测不确定时不改变退避状态。失效计数由 should_reinject_* 统一管理。
        Some(true) => backoff.observe_healthy(std::time::Instant::now()),
        None => {}
        Some(false) => backoff.observe_unhealthy(),
    }
    if !should_reinject_after_health_result(healthy, browser_identity_changed, health_failures) {
        return false;
    }
    if browser_identity_changed {
        // 应用实例更换：旧退避针对的是旧页面，新页面需要立即注入。
        backoff.reset();
    }
    let now = std::time::Instant::now();
    if !backoff.ready(now) {
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "bridge.reinject_backoff_skipped",
            serde_json::json!({
                "debug_port": debug_port,
                "helper_port": helper_port,
                "consecutive_reinjections": backoff.consecutive_attempts,
                "next_reinject_allowed_in_ms": backoff
                    .next_allowed_at
                    .map(|deadline| deadline.saturating_duration_since(now).as_millis() as u64),
            }),
        );
        return false;
    }
    backoff.record_attempt(now);

    let _ = crate::diagnostic_log::append_diagnostic_log(
        "bridge.reinject_start",
        serde_json::json!({
            "debug_port": debug_port,
            "helper_port": helper_port,
            "browser_identity_changed": browser_identity_changed,
            "consecutive_health_failures": *health_failures
        }),
    );
    let default_reinjector: BridgeReinjector =
        Arc::new(move || Box::pin(async move { retry_injection(debug_port, helper_port).await }));
    let reinject_result = run_bridge_reinjector(bridge_reinjector, default_reinjector).await;
    match reinject_result {
        Ok(()) => {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "bridge.reinject_ok",
                serde_json::json!({
                    "debug_port": debug_port,
                    "helper_port": helper_port
                }),
            );
            *health_failures = 0;
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

async fn run_bridge_reinjector(
    bridge_reinjector: Option<BridgeReinjector>,
    default_reinjector: BridgeReinjector,
) -> anyhow::Result<()> {
    match bridge_reinjector {
        Some(reinject) => reinject().await,
        None => default_reinjector().await,
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

// 仅在用户启动 Codex 的流程里重载一次；心跳重注入不重载正在使用的页面。
async fn apply_native_fast_unlock_on_launch(debug_port: u16) -> anyhow::Result<()> {
    let targets = crate::cdp::list_targets(debug_port).await?;
    let target = crate::cdp::pick_injectable_codex_page_target(&targets)?;
    let websocket_url = target
        .web_socket_debugger_url
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("selected CDP target has no websocket URL"))?;
    let probe = "JSON.stringify({ready:window.__codexPlusNativeFastAssetPatchReady===true,compiled:window.__codexPlusNativeFastCompiled==='v1',failed:['unsupported','failed'].includes(window.__codexPlusNativeFastPatchState)})";
    let state = crate::bridge::evaluate_script(websocket_url, probe).await?;
    let state = native_fast_launch_state(&state);
    if state.1 {
        return Ok(());
    }
    anyhow::ensure!(
        !state.2,
        "当前 Codex 资产不支持原生 Fast 补丁，保留原生身份限制"
    );
    anyhow::ensure!(state.0, "原生 Fast 资源补丁未就绪，保留原生界面");
    crate::bridge::send_cdp_command(
        websocket_url,
        "Page.reload",
        serde_json::json!({ "ignoreCache": true }),
    )
    .await?;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        if let Ok(result) = crate::bridge::evaluate_script(websocket_url, probe).await {
            let state = native_fast_launch_state(&result);
            anyhow::ensure!(
                !state.2,
                "当前 Codex 资产不支持原生 Fast 补丁，保留原生身份限制"
            );
            if state.1 {
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "launcher.native_fast_unlock_ready",
                    serde_json::json!({ "debug_port": debug_port }),
                );
                return Ok(());
            }
        }
    }
    anyhow::bail!("当前 Codex 版本未完成原生 Fast 补丁加载，保留原生身份限制")
}

fn native_fast_launch_state(result: &Value) -> (bool, bool, bool) {
    let Some(text) = result.pointer("/result/result/value").and_then(Value::as_str) else {
        return (false, false, false);
    };
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return (false, false, false);
    };
    (
        value.get("ready").and_then(Value::as_bool) == Some(true),
        value.get("compiled").and_then(Value::as_bool) == Some(true),
        value.get("failed").and_then(Value::as_bool) == Some(true),
    )
}

async fn confirmed_pet_overlay_targets(
    debug_port: u16,
) -> anyhow::Result<Vec<crate::cdp::CdpTarget>> {
    let targets = crate::cdp::list_targets(debug_port).await?;
    let mut confirmed = Vec::new();
    for target in targets
        .into_iter()
        .filter(crate::cdp::is_avatar_overlay_page_target)
    {
        let Some(websocket_url) = target.web_socket_debugger_url.as_deref() else {
            continue;
        };
        // 推点只要求 overlay 上已经有 updateScreenPoint。
        // V2 精灵图 + vscode-api dispatcher 探针经常失败（modulepreload），
        // 用户脚本的 stub 也会让官方 inject skip，但不能因此停止推坐标。
        if pet_overlay_has_screen_point_hook(websocket_url)
            .await
            .unwrap_or(false)
        {
            confirmed.push(target);
        }
    }
    Ok(confirmed)
}

async fn pet_overlay_has_screen_point_hook(websocket_url: &str) -> anyhow::Result<bool> {
    let result = crate::bridge::evaluate_script(
        websocket_url,
        crate::assets::pet_real_mouse_hook_probe_script(),
    )
    .await?;
    Ok(runtime_evaluate_result_is_true(&result))
}

async fn sync_pet_real_mouse_overlay(debug_port: u16, _helper_port: u16) -> anyhow::Result<()> {
    let settings = SettingsStore::default().load().unwrap_or_default();
    let enabled = settings.enhancements_enabled && settings.codex_app_pet_real_mouse_look;
    let targets = crate::cdp::list_targets(debug_port).await?;
    for target in targets
        .iter()
        .filter(|target| crate::cdp::is_avatar_overlay_page_target(target))
    {
        let Some(websocket_url) = target.web_socket_debugger_url.as_deref() else {
            continue;
        };
        let script = if !enabled {
            Some(crate::assets::pet_real_mouse_stop_script())
        } else if pet_overlay_has_screen_point_hook(websocket_url)
            .await
            .unwrap_or(false)
        {
            // 已有官方 runtime 或用户脚本 stub 时保留它，避免重置其 dispatcher。
            None
        } else {
            // 旧的 V2 dispatcher 探针可能因 modulepreload 时序失败；先安装 runtime，
            // 后续 driver 会再次确认 hook，runtime 自己负责判断 dispatcher 是否可用。
            Some(crate::assets::pet_real_mouse_script())
        };
        let Some(script) = script else {
            continue;
        };
        crate::bridge::evaluate_script(websocket_url, script)
            .await
            .with_context(|| {
                format!(
                    "failed to evaluate pet overlay script in target {} ({})",
                    target.id, target.url
                )
            })?;
    }
    Ok(())
}

#[cfg(any(windows, target_os = "macos"))]
async fn run_pet_real_mouse_cursor_driver(debug_port: u16) {
    loop {
        let settings = SettingsStore::default().load().unwrap_or_default();
        if !settings.enhancements_enabled || !settings.codex_app_pet_real_mouse_look {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            continue;
        }

        let targets = confirmed_pet_overlay_targets(debug_port)
            .await
            .unwrap_or_default();
        if targets.is_empty() {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            continue;
        }
        let mut drivers = tokio::task::JoinSet::new();
        for target in targets.iter().cloned() {
            drivers.spawn(run_pet_real_mouse_target_driver(debug_port, target));
        }
        if let Some(result) = drivers.join_next().await {
            if let Err(error) = result {
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "pet.real_mouse_cursor_driver_join_failed",
                    serde_json::json!({
                        "debug_port": debug_port,
                        "message": error.to_string()
                    }),
                );
            }
        }
        drivers.abort_all();
        while drivers.join_next().await.is_some() {}
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
}

#[cfg(any(windows, target_os = "macos"))]
async fn run_pet_real_mouse_target_driver(debug_port: u16, target: crate::cdp::CdpTarget) {
    let Some(websocket_url) = target.web_socket_debugger_url.as_deref() else {
        return;
    };
    // 官方 inject 失败或被用户脚本 stub skip 都不能阻止后续推点。
    if let Err(error) =
        crate::bridge::evaluate_script(websocket_url, crate::assets::pet_real_mouse_script()).await
    {
        record_pet_cursor_driver_failure(debug_port, &target, error);
    }

    let mut ticks_until_settings_check = 10_u8;
    let result = crate::bridge::run_periodic_evaluations(
        websocket_url,
        std::time::Duration::from_millis(100),
        || {
            if ticks_until_settings_check == 0 {
                let settings = SettingsStore::default().load().unwrap_or_default();
                if !settings.enhancements_enabled || !settings.codex_app_pet_real_mouse_look {
                    return Ok(None);
                }
                ticks_until_settings_check = 10;
            }
            ticks_until_settings_check -= 1;
            let (x, y) = logical_cursor_position()?;
            Ok(Some(crate::assets::pet_real_mouse_update_script(x, y)))
        },
    )
    .await;

    if result.is_ok() {
        let _ = crate::bridge::evaluate_script(
            websocket_url,
            crate::assets::pet_real_mouse_stop_script(),
        )
        .await;
    }
    match result {
        Ok(()) => {
            PET_CURSOR_DRIVER_FAILED.store(false, Ordering::Relaxed);
        }
        Err(error) => record_pet_cursor_driver_failure(debug_port, &target, error),
    }
}

#[cfg(any(windows, target_os = "macos"))]
fn record_pet_cursor_driver_failure(
    debug_port: u16,
    target: &crate::cdp::CdpTarget,
    error: anyhow::Error,
) {
    if !PET_CURSOR_DRIVER_FAILED.swap(true, Ordering::Relaxed) {
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "pet.real_mouse_cursor_driver_disconnected",
            serde_json::json!({
                "debug_port": debug_port,
                "target_id": target.id,
                "target_url": target.url,
                "message": format!("{error:#}")
            }),
        );
    }
}

fn record_pet_overlay_sync_result(debug_port: u16, helper_port: u16, result: anyhow::Result<()>) {
    match result {
        Ok(()) => {
            if PET_OVERLAY_SYNC_FAILED.swap(false, Ordering::Relaxed) {
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "pet.real_mouse_overlay_sync_recovered",
                    serde_json::json!({
                        "debug_port": debug_port,
                        "helper_port": helper_port
                    }),
                );
            }
        }
        Err(error) => {
            if !PET_OVERLAY_SYNC_FAILED.swap(true, Ordering::Relaxed) {
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "pet.real_mouse_overlay_sync_failed",
                    serde_json::json!({
                        "debug_port": debug_port,
                        "helper_port": helper_port,
                        "message": format!("{error:#}")
                    }),
                );
            }
        }
    }
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

pub fn select_macos_debug_launch_action(
    app_running: bool,
    codex_cdp_available: bool,
) -> MacosDebugLaunchAction {
    match (app_running, codex_cdp_available) {
        (false, _) => MacosDebugLaunchAction::LaunchNew,
        (true, true) => MacosDebugLaunchAction::ReuseRunningDebugApp,
        (true, false) => MacosDebugLaunchAction::RestartRunningApp,
    }
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

async fn quit_macos_app_and_wait(app_dir: &Path) -> anyhow::Result<()> {
    run_macos_cleanup_command(app_dir, MacosCleanupPolicy::QuitIfNotPreviouslyRunning).await?;
    let deadline = tokio::time::Instant::now()
        + std::time::Duration::from_millis(MACOS_DEBUG_TAKEOVER_WAIT_MS);
    while is_macos_app_running(app_dir).await {
        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!(
                "macOS app did not exit before debug relaunch: {}",
                app_dir.display()
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(
            MACOS_DEBUG_TAKEOVER_INTERVAL_MS,
        ))
        .await;
    }
    let _ = crate::diagnostic_log::append_diagnostic_log(
        "launcher.macos_existing_app_without_cdp_stopped",
        serde_json::json!({
            "app_dir": app_dir
        }),
    );
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

#[cfg(windows)]
async fn terminate_windows_process_id(process_id: u32) -> anyhow::Result<()> {
    tokio::task::spawn_blocking(move || terminate_windows_process_id_blocking(process_id))
        .await
        .context("Windows process termination task failed")?
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
        aumid: crate::app_paths::packaged_app_user_model_id(app_dir),
        // 终态不再是「某个进行中的阶段」；进度固定 100 便于 manager 判断已结束。
        phase: None,
        progress: Some(100),
    }
}

/// 启动阶段埋点（issue #2244）。
///
/// 把每个前置步骤写进 `latest-status.json` 的 `phase`/`progress`，同时往
/// `codex-plus.log` 记一条带「距启动开始多少毫秒」的日志。冷启动慢的时候，
/// 用户与维护者由此能直接指出卡在哪一步，而不用靠进程采样反推。
struct LaunchTimeline {
    status_store: StatusStore,
    started: std::time::Instant,
}

impl LaunchTimeline {
    fn new(status_store: StatusStore) -> Self {
        Self {
            status_store,
            started: std::time::Instant::now(),
        }
    }

    /// 进入某个阶段。状态写入与日志写入都是 best-effort：埋点失败绝不能
    /// 影响启动本身。
    fn mark(&self, phase: &str, progress: u8) {
        let elapsed_ms = self.started.elapsed().as_millis() as u64;
        let _ = self.status_store.save_phase(phase, progress);
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "launcher.phase",
            serde_json::json!({
                "phase": phase,
                "progress": progress,
                "elapsed_ms": elapsed_ms,
            }),
        );
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

/// 提权运行时禁止「AUMID 激活失败 → 按路径直接启动」回退的原因；
/// 非提权返回 None 以保留回退（兜底清单 Application Id 变化等场景）。
///
/// 提权进程无法激活 MSIX 打包应用，按路径启动的进程会因缺少包身份
/// （APPMODEL_ERROR_NO_PACKAGE）立即退出，注入也必然失败；此时必须如实失败
/// 并给出指引，而不是静默回退导致 latest-status.json 误报 running（issue #2351）。
///
/// 不加 `#[cfg(windows)]`：调用点在 `if cfg!(windows)` 的运行时分支里，
/// 全平台都要编译到这个函数（纯逻辑，无平台依赖）。
fn packaged_activation_fallback_block_reason(is_elevated: bool) -> Option<&'static str> {
    if is_elevated {
        Some(
            "Codex 打包应用激活失败，且 Codex++ 当前以管理员（提权）身份运行：\
             Windows 不允许提权进程激活 MSIX 打包应用，按路径启动的进程会因缺少\
             包身份立即退出。请取消可执行文件「属性 → 兼容性 → 以管理员身份运行\
             此程序」的勾选（或清除 AppCompatFlags\\Layers 中的 RUNASADMIN 标记），\
             然后以普通权限重新启动 Codex++",
        )
    } else {
        None
    }
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
        CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
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
                CoCreateInstance(&ApplicationActivationManager, None, CLSCTX_ALL)?;
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
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn standalone_quota_unlock_is_opt_in_and_does_not_duplicate_full_enhancements() {
        let mut settings = BackendSettings::default();
        for enhancements in [false, true] {
            for standalone in [false, true] {
                settings.enhancements_enabled = enhancements;
                settings.codex_app_standalone_quota_unlock = standalone;
                assert_eq!(
                    should_start_isolated_quota_gate(&settings),
                    !enhancements && standalone
                );
            }
        }
    }

    #[tokio::test]
    async fn isolated_quota_runtime_stops_and_joins_before_restart() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let runtime = IsolatedQuotaGateRuntime::start(listener.local_addr().unwrap().port());
        let clone = Arc::clone(&runtime);
        runtime.stop().await;
        assert!(*runtime.shutdown.borrow());
        assert!(runtime.task.lock().await.is_none());
        clone.stop().await;
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), listener.accept())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn isolated_quota_runtime_cancels_only_when_last_owner_is_dropped() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let runtime = IsolatedQuotaGateRuntime::start(listener.local_addr().unwrap().port());
        let mut receiver = runtime.shutdown.subscribe();
        let task = runtime.task.lock().await.take().unwrap();
        let clone = Arc::clone(&runtime);
        drop(runtime);
        assert!(!*receiver.borrow());
        drop(clone);
        let _ = receiver.wait_for(|stop| *stop).await;
        tokio::time::timeout(std::time::Duration::from_secs(3), task)
            .await
            .unwrap()
            .unwrap();
    }

    // 提权分支依赖 issue #2351 的手动复现步骤验收（勾选兼容性「以管理员身份运行」
    // 后应得到指引错误）。这里不断言当前进程的提权状态：GitHub 的 Windows runner
    // 本身就以提权令牌运行，断言「CI shell 非提权」必然失败。
    #[test]
    fn elevated_activation_failure_blocks_path_fallback_with_guidance() {
        let reason = packaged_activation_fallback_block_reason(true)
            .expect("elevated activation failure must block the path fallback");
        assert!(reason.contains("管理员"));
        assert!(reason.contains("以普通权限"));
    }

    #[test]
    fn unelevated_activation_failure_keeps_path_fallback() {
        assert!(packaged_activation_fallback_block_reason(false).is_none());
    }

    fn counted_reinjector(calls: Arc<AtomicUsize>) -> BridgeReinjector {
        Arc::new(move || {
            let calls = calls.clone();
            Box::pin(async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        })
    }

    #[test]
    fn owned_launcher_exit_follows_its_debug_port_not_other_codex_processes() {
        assert!(owned_launcher_target_alive(false, true));
        assert!(!owned_launcher_target_alive(false, false));
    }

    #[test]
    fn owned_launcher_waits_for_its_codex_process_when_cdp_is_unavailable() {
        assert!(owned_launcher_target_alive(true, false));
    }

    /// macOS 退出等待的回归护栏：一次探测瞬断不能判成「Codex 已退出」。
    ///
    /// 这里决定的是 `wait_for_codex_exit` 何时返回，而它一返回就会
    /// `shutdown_helper`——57321 关闭后，写死在 config.toml 里的 base_url
    /// 失去对端，页面停在启动壳上。所以「一次失败仍继续等」必须是硬保证。
    #[test]
    fn exit_wait_survives_a_single_transient_probe_miss() {
        let now = std::time::Instant::now();
        let mut grace = PackagedExitGrace::default();
        // 探到目标：正常续命。
        assert!(grace.keep_waiting(Some(true), false, now));
        // 瞬断一次：仍在等待。
        assert!(grace.keep_waiting(Some(false), false, now + std::time::Duration::from_secs(2)));
        // 恢复：计数复位，之后再有瞬断也照样给宽限。
        assert!(grace.keep_waiting(Some(true), false, now + std::time::Duration::from_secs(4)));
        assert!(grace.keep_waiting(Some(false), false, now + std::time::Duration::from_secs(6)));
    }

    /// 与上一条互补：真退出必须被有限次探测判出，不能因为宽限而永久续命。
    ///
    /// 宽限条件是「不足 3 次探测失败 **或** 距首次失败不足 10 秒」，两者取宽，
    /// 所以持续探不到时约 10 秒才结束等待——晚一点，但一定会结束。
    #[test]
    fn exit_wait_still_ends_after_sustained_misses() {
        let now = std::time::Instant::now();
        let mut grace = PackagedExitGrace::default();
        assert!(grace.keep_waiting(Some(false), false, now));
        assert!(grace.keep_waiting(Some(false), false, now + std::time::Duration::from_secs(2)));
        // 4 秒：探测次数已超，但仍在 10 秒窗口内，继续等。
        assert!(grace.keep_waiting(Some(false), false, now + std::time::Duration::from_secs(4)));
        // 越过 10 秒窗口：结束等待。
        assert!(!grace.keep_waiting(Some(false), false, now + std::time::Duration::from_secs(12)));
    }

    /// cdp_available 仍要能独立续命：进程身份没捕获到时靠它兜底。
    #[test]
    fn exit_wait_keeps_alive_on_cdp_alone() {
        let now = std::time::Instant::now();
        let mut grace = PackagedExitGrace::default();
        assert!(grace.keep_waiting(None, true, now));
        assert!(grace.keep_waiting(None, true, now + std::time::Duration::from_secs(60)));
    }

    #[test]
    fn packaged_launcher_survives_one_failed_cdp_probe_after_activation_pid_disappears() {
        let now = std::time::Instant::now();
        let mut grace = PackagedExitGrace::default();
        assert!(grace.keep_waiting(None, false, now));
        assert!(grace.keep_waiting(None, true, now + std::time::Duration::from_secs(2)));
    }

    #[test]
    fn packaged_launcher_waits_for_exact_process_without_cdp() {
        let now = std::time::Instant::now();
        let mut grace = PackagedExitGrace::default();
        for seconds in [0, 10, 60, 3600] {
            assert!(grace.keep_waiting(
                Some(true),
                false,
                now + std::time::Duration::from_secs(seconds)
            ));
        }
        assert_eq!(grace.missed_probes, 0);
    }

    #[test]
    fn packaged_launcher_exits_after_bounded_continuous_absence() {
        let now = std::time::Instant::now();
        let mut grace = PackagedExitGrace::default();
        for seconds in [0, 2, 4, 6, 8] {
            assert!(grace.keep_waiting(None, false, now + std::time::Duration::from_secs(seconds)));
        }
        assert!(!grace.keep_waiting(None, false, now + std::time::Duration::from_secs(10)));
    }

    #[test]
    fn packaged_launcher_does_not_follow_another_instance_after_its_process_exits() {
        let now = std::time::Instant::now();
        let mut grace = PackagedExitGrace::default();
        assert!(grace.keep_waiting(Some(true), true, now));
        assert!(grace.keep_waiting(Some(false), true, now));
        assert!(grace.keep_waiting(Some(false), true, now + std::time::Duration::from_secs(2)));
        assert!(!grace.keep_waiting(Some(false), true, now + std::time::Duration::from_secs(10)));
    }

    #[test]
    fn packaged_launcher_recovery_resets_exit_grace() {
        let now = std::time::Instant::now();
        let mut grace = PackagedExitGrace::default();
        assert!(grace.keep_waiting(None, false, now));
        assert!(grace.keep_waiting(None, true, now + std::time::Duration::from_secs(8)));
        assert!(grace.keep_waiting(None, false, now + std::time::Duration::from_secs(20)));
        assert_eq!(grace.missed_probes, 1);
    }

    #[test]
    fn packaged_activation_pid_alone_cannot_keep_launcher_alive_indefinitely() {
        assert!(activation_process_is_startup_evidence(
            true,
            std::time::Duration::from_secs(29)
        ));
        assert!(!activation_process_is_startup_evidence(
            true,
            std::time::Duration::from_secs(30)
        ));
        assert!(!activation_process_is_startup_evidence(
            false,
            std::time::Duration::ZERO
        ));
    }

    #[test]
    fn packaged_browser_fallback_after_discovery_error_and_dead_activation_rejects_replacement() {
        let now = std::time::Instant::now();
        let mut binding = PackagedBrowserBinding {
            initial_id: Some("original".to_string()),
            replaced: false,
        };
        let mut grace = PackagedExitGrace::default();
        // discovery 失败，没有真实句柄，activation 已退出后原路径降级为 None。
        let process_alive = Some(false).filter(|alive| *alive);
        for seconds in [0, 2, 4, 6, 8] {
            assert!(grace.keep_waiting(
                process_alive,
                binding.owns(Some("replacement")),
                now + std::time::Duration::from_secs(seconds)
            ));
        }
        assert!(!grace.keep_waiting(
            process_alive,
            binding.owns(Some("replacement")),
            now + std::time::Duration::from_secs(10)
        ));
    }

    #[test]
    fn packaged_browser_fallback_without_initial_identity_is_bounded() {
        let now = std::time::Instant::now();
        let mut binding = PackagedBrowserBinding {
            initial_id: None,
            replaced: false,
        };
        let mut grace = PackagedExitGrace::default();
        assert!(grace.keep_waiting(None, binding.owns(Some("unbound")), now));
        assert!(grace.keep_waiting(
            None,
            binding.owns(Some("unbound")),
            now + std::time::Duration::from_secs(2)
        ));
        assert!(!grace.keep_waiting(
            None,
            binding.owns(Some("unbound")),
            now + std::time::Duration::from_secs(10)
        ));
    }

    #[tokio::test]
    async fn packaged_browser_capture_rejects_other_browsers_before_reading_their_identity() {
        for (url, accepted) in [("chrome://newtab", false), ("app://-/index.html", true)] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let recorded = requests.clone();
            let server = tokio::spawn(async move {
                for _ in 0..if accepted { 2 } else { 1 } {
                    let (mut stream, _) = listener.accept().await.unwrap();
                    let mut request = [0; 4096];
                    let count = stream.read(&mut request).await.unwrap();
                    let request = String::from_utf8_lossy(&request[..count]);
                    let path = request.split_whitespace().nth(1).unwrap().to_string();
                    recorded.lock().unwrap().push(path.clone());
                    let body = if path == "/json/version" {
                        serde_json::json!({ "Browser":"Chrome/fixture", "webSocketDebuggerUrl":format!("ws://127.0.0.1:{port}/devtools/browser/fixture-browser") })
                    } else {
                        serde_json::json!([{ "id":"page", "type":"page", "title":"fixture", "url":url,
                            "webSocketDebuggerUrl":format!("ws://127.0.0.1:{port}/devtools/page/fixture-page") }])
                    }.to_string();
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    stream.write_all(response.as_bytes()).await.unwrap();
                }
            });
            let identity = validated_codex_browser_id(port).await;
            server.await.unwrap();
            assert_eq!(identity.as_deref(), accepted.then_some("fixture-browser"));
            assert_eq!(
                requests.lock().unwrap().as_slice(),
                if accepted {
                    &["/json", "/json/version"][..]
                } else {
                    &["/json"][..]
                }
            );
        }
    }

    #[test]
    fn packaged_browser_fallback_after_discovery_error_keeps_only_same_initial_browser() {
        let now = std::time::Instant::now();
        let mut binding = PackagedBrowserBinding {
            initial_id: Some("original".to_string()),
            replaced: false,
        };
        let mut grace = PackagedExitGrace::default();
        let process_alive = Some(false).filter(|alive| *alive);
        for seconds in [0, 10, 60, 3600] {
            assert!(grace.keep_waiting(
                process_alive,
                binding.owns(Some("original")),
                now + std::time::Duration::from_secs(seconds)
            ));
        }
        assert!(!binding.owns(None));
        assert!(binding.owns(Some("original")));
        assert!(!binding.owns(Some("replacement")));
        assert!(
            !binding.owns(Some("original")),
            "实例替换后不能重新绑定旧 ID"
        );
    }

    #[test]
    fn degraded_packaged_launch_keeps_its_captured_browser_after_activation_grace_expires() {
        let now = std::time::Instant::now();
        let mut binding = PackagedBrowserBinding {
            initial_id: Some("captured-degraded-browser".to_string()),
            replaced: false,
        };
        let mut grace = PackagedExitGrace::default();
        for seconds in [0, 10, 30, 60, 3600] {
            let elapsed = std::time::Duration::from_secs(seconds);
            let activation_alive =
                Some(false).filter(|alive| activation_process_is_startup_evidence(*alive, elapsed));
            assert!(grace.keep_waiting(
                activation_alive,
                binding.owns(Some("captured-degraded-browser")),
                now + elapsed,
            ));
        }
        assert_eq!(grace.missed_probes, 0);
    }

    #[test]
    fn packaged_tracked_process_remains_authoritative_when_browser_probe_fails() {
        let now = std::time::Instant::now();
        let mut binding = PackagedBrowserBinding {
            initial_id: Some("original".to_string()),
            replaced: false,
        };
        let mut grace = PackagedExitGrace::default();
        assert!(grace.keep_waiting(Some(true), binding.owns(None), now));
        assert!(grace.keep_waiting(Some(true), binding.owns(Some("replacement")), now));
        assert!(grace.keep_waiting(
            Some(true),
            false,
            now + std::time::Duration::from_secs(3600)
        ));
    }

    #[test]
    fn terminal_status_preserves_run_identity_and_clears_in_progress_phase() {
        let temp = tempfile::tempdir().unwrap();
        let store = StatusStore::new(temp.path().join("status.json"));
        store
            .save_latest(&LaunchStatus {
                status: "running".to_string(),
                started_at_ms: 10,
                phase: Some("ready".to_string()),
                debug_port: Some(9229),
                ..LaunchStatus::default()
            })
            .unwrap();
        save_terminal_launch_status(&store, 10, "stopped", "Codex exited").unwrap();
        let status = store.load_latest().unwrap().unwrap();
        assert_eq!(status.status, "stopped");
        assert_eq!(status.message, "Codex exited");
        assert_eq!(status.started_at_ms, 10);
        assert_eq!(status.debug_port, Some(9229));
        assert_eq!(status.phase, None);
    }

    #[test]
    fn old_launcher_terminal_status_cannot_overwrite_new_restart_request() {
        let temp = tempfile::tempdir().unwrap();
        let store = StatusStore::new(temp.path().join("status.json"));
        let next_request = LaunchStatus {
            status: "starting".to_string(),
            started_at_ms: 20,
            ..LaunchStatus::default()
        };
        store.save_latest(&next_request).unwrap();
        save_terminal_launch_status(&store, 10, "stopped", "old process exited").unwrap();
        assert_eq!(store.load_latest().unwrap(), Some(next_request));
    }

    #[test]
    fn bridge_health_failures_reinject_only_after_consecutive_unhealthy_results() {
        let mut failures = 0;
        assert!(!should_reinject_after_health_result(
            Some(false),
            false,
            &mut failures
        ));
        assert_eq!(failures, 1);
        assert!(should_reinject_after_health_result(
            Some(false),
            false,
            &mut failures
        ));
        assert_eq!(failures, BRIDGE_HEALTH_FAILURE_THRESHOLD);
        assert!(!should_reinject_after_health_result(
            Some(true),
            false,
            &mut failures
        ));
        assert_eq!(failures, 0);

        failures = 1;
        assert!(!should_reinject_after_health_result(
            None,
            false,
            &mut failures
        ));
        assert_eq!(failures, 0);
        assert!(should_reinject_after_health_result(
            Some(false),
            true,
            &mut failures
        ));
    }

    #[test]
    fn reinject_backoff_delay_doubles_and_caps() {
        assert_eq!(
            reinject_backoff_delay(0),
            std::time::Duration::from_secs(10)
        );
        assert_eq!(
            reinject_backoff_delay(1),
            std::time::Duration::from_secs(20)
        );
        assert_eq!(
            reinject_backoff_delay(2),
            std::time::Duration::from_secs(40)
        );
        assert_eq!(
            reinject_backoff_delay(4),
            std::time::Duration::from_secs(160)
        );
        assert_eq!(
            reinject_backoff_delay(5),
            std::time::Duration::from_secs(300)
        );
        assert_eq!(
            reinject_backoff_delay(32),
            std::time::Duration::from_secs(300)
        );
    }

    #[test]
    fn reinject_backoff_blocks_until_deadline_and_resets_on_recovery() {
        let now = std::time::Instant::now();
        let mut backoff = BridgeReinjectBackoff::default();
        assert!(backoff.ready(now), "首次重注入应立即允许");

        backoff.record_attempt(now);
        assert_eq!(backoff.consecutive_attempts, 1);
        assert!(!backoff.ready(now), "重注入后应进入退避窗口");
        assert!(backoff.ready(now + reinject_backoff_delay(0)));

        backoff.record_attempt(now + reinject_backoff_delay(0));
        assert_eq!(backoff.consecutive_attempts, 2);
        assert!(!backoff.ready(now + reinject_backoff_delay(0)));
        assert!(backoff.ready(now + reinject_backoff_delay(0) + reinject_backoff_delay(1)));

        backoff.reset();
        assert_eq!(backoff.consecutive_attempts, 0);
        assert!(backoff.ready(now), "健康恢复后应立即允许重注入");
    }

    #[test]
    fn reinject_backoff_resets_only_after_sustained_health() {
        let now = std::time::Instant::now();
        let reset_after = std::time::Duration::from_secs(BRIDGE_REINJECT_BACKOFF_RESET_AFTER_SECS);
        let mut backoff = BridgeReinjectBackoff::default();

        backoff.record_attempt(now);
        backoff.observe_healthy(now + std::time::Duration::from_secs(1));
        assert_eq!(
            backoff.consecutive_attempts, 1,
            "注入后立刻健康不应清零退避，否则退避永远停在第一档"
        );

        backoff.observe_healthy(now + std::time::Duration::from_secs(30));
        assert_eq!(backoff.consecutive_attempts, 1, "未满阈值不应清零");

        // 中途出现一次不健康，重新计时。
        backoff.observe_unhealthy();
        let restart = now + std::time::Duration::from_secs(40);
        backoff.observe_healthy(restart);
        backoff.observe_healthy(now + std::time::Duration::from_secs(1) + reset_after);
        assert_eq!(
            backoff.consecutive_attempts, 1,
            "不健康之后必须从新起点重新计时"
        );

        backoff.observe_healthy(restart + reset_after);
        assert_eq!(backoff.consecutive_attempts, 0, "持续健康满阈值后应清零");
        assert!(backoff.healthy_since.is_none());
        assert!(backoff.ready(restart + reset_after));
    }

    #[test]
    fn helper_bind_retry_covers_fixed_proxy_ports_and_macos_restarts() {
        assert_eq!(
            helper_bind_retry_timeout_ms(true, false),
            PROTOCOL_PROXY_BIND_RETRY_TIMEOUT_MS
        );
        assert_eq!(
            helper_bind_retry_timeout_ms(false, true),
            HELPER_BIND_RETRY_TIMEOUT_MS
        );
        assert_eq!(helper_bind_retry_timeout_ms(false, false), 0);
        assert_eq!(
            helper_bind_retry_timeout_ms(true, true),
            PROTOCOL_PROXY_BIND_RETRY_TIMEOUT_MS
        );
    }

    #[test]
    fn fixed_helper_port_is_required_only_when_protocol_proxy_is_used() {
        assert_eq!(
            required_fixed_helper_port(&BackendSettings::default()),
            None
        );

        let official_mix = BackendSettings {
            active_relay_id: "official-mix".to_string(),
            relay_profiles: vec![crate::settings::RelayProfile {
                id: "official-mix".to_string(),
                relay_mode: crate::settings::RelayMode::Official,
                official_mix_api_key: true,
                protocol: crate::settings::RelayProtocol::Responses,
                ..crate::settings::RelayProfile::default()
            }],
            ..BackendSettings::default()
        };
        assert_eq!(
            required_fixed_helper_port(&official_mix),
            Some(crate::protocol_proxy::protocol_proxy_port())
        );
    }

    #[test]
    fn fixed_port_wait_retries_transient_forbidden_and_stops_on_other_errors() {
        let temp = tempfile::tempdir().unwrap();
        let log_path = temp.path().join("diagnostic.log");
        crate::diagnostic_log::set_diagnostic_log_path_for_tests(Some(log_path));
        struct RestoreLog;
        impl Drop for RestoreLog {
            fn drop(&mut self) {
                crate::diagnostic_log::set_diagnostic_log_path_for_tests(None);
            }
        }
        let _restore = RestoreLog;

        let forbidden_attempts = std::cell::Cell::new(0u32);
        let sleeps = std::cell::Cell::new(0u32);
        wait_for_fixed_helper_port(
            57321,
            400,
            200,
            |_| {
                let attempt = forbidden_attempts.get();
                forbidden_attempts.set(attempt + 1);
                if attempt < 2 {
                    Err(std::io::Error::from_raw_os_error(10013))
                } else {
                    Ok(())
                }
            },
            |_| sleeps.set(sleeps.get() + 1),
        )
        .unwrap();
        assert_eq!(forbidden_attempts.get(), 3);
        assert_eq!(sleeps.get(), 2);

        let probes = std::cell::Cell::new(0u32);
        let other_sleeps = std::cell::Cell::new(0u32);
        let error = wait_for_fixed_helper_port(
            57321,
            10_000,
            200,
            |_| {
                probes.set(probes.get() + 1);
                Err(std::io::Error::other("simulated unrelated failure"))
            },
            |_| other_sleeps.set(other_sleeps.get() + 1),
        )
        .unwrap_err();
        let message = format!("{error:#}");
        assert_eq!(probes.get(), 1);
        assert_eq!(other_sleeps.get(), 0);
        assert!(message.contains("simulated unrelated failure"), "{message}");
        assert!(!message.contains("被其他进程占用"), "{message}");
        assert!(!message.contains("被 Windows 保留"), "{message}");

        let permanent_probes = std::cell::Cell::new(0u32);
        let permanent_sleeps = std::cell::Cell::new(0u32);
        let error = wait_for_fixed_helper_port(
            57321,
            400,
            200,
            |_| {
                permanent_probes.set(permanent_probes.get() + 1);
                Err(std::io::Error::from_raw_os_error(10013))
            },
            |_| permanent_sleeps.set(permanent_sleeps.get() + 1),
        )
        .unwrap_err();
        let message = format!("{error:#}");
        assert_eq!(permanent_probes.get(), 3);
        assert_eq!(permanent_sleeps.get(), 2);
        assert!(message.contains("os error 10013"), "{message}");
        assert!(!message.contains("被其他进程占用"), "{message}");
        if cfg!(windows) {
            assert!(message.contains("被 Windows 保留"), "{message}");
        }
    }

    #[tokio::test]
    async fn bridge_reinjector_prefers_launcher_callback() {
        let launcher_calls = Arc::new(AtomicUsize::new(0));
        let default_calls = Arc::new(AtomicUsize::new(0));

        run_bridge_reinjector(
            Some(counted_reinjector(launcher_calls.clone())),
            counted_reinjector(default_calls.clone()),
        )
        .await
        .unwrap();

        assert_eq!(launcher_calls.load(Ordering::SeqCst), 1);
        assert_eq!(default_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn bridge_reinjector_falls_back_to_core_callback() {
        let default_calls = Arc::new(AtomicUsize::new(0));

        run_bridge_reinjector(None, counted_reinjector(default_calls.clone()))
            .await
            .unwrap();

        assert_eq!(default_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn http_body_framing_rejects_ambiguous_or_unsupported_headers() {
        let conflict = http_body_framing(
            b"POST / HTTP/1.1\r\nContent-Length: 4\r\nTransfer-Encoding: chunked",
        )
        .unwrap_err();
        assert_eq!(conflict.status(), "400 Bad Request");

        let unsupported =
            http_body_framing(b"POST / HTTP/1.1\r\nTransfer-Encoding: gzip").unwrap_err();
        assert_eq!(unsupported.status(), "400 Bad Request");

        let multiple = http_body_framing(
            b"POST / HTTP/1.1\r\nTransfer-Encoding: gzip\r\nTransfer-Encoding: chunked",
        )
        .unwrap_err();
        assert_eq!(multiple.status(), "400 Bad Request");
    }

    /// 测试用的小上限，避免为了验证边界在测试里真的分配/扫描 128 MiB。
    const TEST_HTTP_BODY_BYTES: usize = 4 * 1024;

    #[test]
    fn chunked_decoder_accepts_exact_body_limit_and_rejects_one_byte_more() {
        let mut exact = format!("{:X}\r\n", TEST_HTTP_BODY_BYTES).into_bytes();
        exact.resize(exact.len() + TEST_HTTP_BODY_BYTES, b'a');
        exact.extend_from_slice(b"\r\n0\r\n\r\n");
        let ChunkedBody::Complete(decoded) =
            decode_chunked_body_with_limit(&exact, TEST_HTTP_BODY_BYTES).unwrap()
        else {
            panic!("expected complete chunked body");
        };
        assert_eq!(decoded.len(), TEST_HTTP_BODY_BYTES);

        let oversized = format!("{:X}\r\n", TEST_HTTP_BODY_BYTES + 1).into_bytes();
        let error = decode_chunked_body_with_limit(&oversized, TEST_HTTP_BODY_BYTES).unwrap_err();
        assert_eq!(error.status(), "413 Payload Too Large");
    }

    #[test]
    fn chunked_decoder_handles_extensions_trailers_and_every_partial_prefix() {
        let encoded = b"3;name=value\r\n\x00\x80\xff\r\n2\r\nAB\r\n0\r\nX-Trace: yes\r\n\r\n";
        for prefix_len in 0..encoded.len() {
            assert!(matches!(
                scan_chunked_body(&encoded[..prefix_len]).unwrap(),
                ChunkedBodyScan::Incomplete
            ));
        }

        assert!(matches!(
            scan_chunked_body(encoded).unwrap(),
            ChunkedBodyScan::Complete
        ));
        let ChunkedBody::Complete(decoded) = decode_chunked_body(encoded).unwrap() else {
            panic!("expected complete chunked body");
        };
        assert_eq!(decoded, [0x00, 0x80, 0xff, b'A', b'B']);
    }

    #[test]
    fn chunked_decoder_rejects_oversized_size_lines_and_trailers() {
        let oversized_size_line = vec![b'f'; MAX_HTTP_HEADER_BYTES + 1];
        let error = scan_chunked_body(&oversized_size_line).unwrap_err();
        assert_eq!(error.status(), "400 Bad Request");

        let mut oversized_trailer = b"0\r\nX-Large: ".to_vec();
        oversized_trailer.resize(MAX_HTTP_HEADER_BYTES + 16, b'a');
        let error = scan_chunked_body(&oversized_trailer).unwrap_err();
        assert_eq!(error.status(), "400 Bad Request");
    }

    #[test]
    fn content_length_body_accepts_exact_limit_and_rejects_one_byte_more() {
        let exact = vec![b'a'; TEST_HTTP_BODY_BYTES];
        assert_eq!(
            content_length_body_with_limit(&exact, TEST_HTTP_BODY_BYTES, TEST_HTTP_BODY_BYTES)
                .unwrap()
                .len(),
            TEST_HTTP_BODY_BYTES
        );

        let error =
            content_length_body_with_limit(&[], TEST_HTTP_BODY_BYTES + 1, TEST_HTTP_BODY_BYTES)
                .unwrap_err();
        assert_eq!(error.status(), "413 Payload Too Large");
    }

    #[test]
    fn http_body_limit_defaults_to_128_mib_and_resolves_env_over_setting() {
        let mib = 1024 * 1024;
        assert_eq!(DEFAULT_HTTP_BODY_BYTES, 128 * mib);
        assert_eq!(MIN_HTTP_BODY_BYTES, 32 * mib);
        assert_eq!(MAX_HTTP_BODY_BYTES, 512 * mib);
        assert_eq!(
            resolve_http_body_bytes(None, DEFAULT_HTTP_BODY_BYTES),
            128 * mib
        );

        // 设置值生效；越界值夹取到 32~512 MiB。
        assert_eq!(resolve_http_body_bytes(None, 256 * mib), 256 * mib);
        assert_eq!(resolve_http_body_bytes(None, 8 * mib), 32 * mib);
        assert_eq!(resolve_http_body_bytes(None, 4096 * mib), 512 * mib);

        // 环境变量优先于设置；低于下限或非法值忽略，高于上限夹取。
        assert_eq!(resolve_http_body_bytes(Some(" 402653184 "), 64 * mib), 384 * mib);
        assert_eq!(resolve_http_body_bytes(Some("1048576"), 64 * mib), 64 * mib);
        assert_eq!(resolve_http_body_bytes(Some("not-a-number"), 64 * mib), 64 * mib);
        assert_eq!(resolve_http_body_bytes(Some("1073741824"), 64 * mib), 512 * mib);
    }

    #[test]
    fn http_body_limits_allow_double_encoded_body_and_cap_at_512_mib() {
        let mib = 1024 * 1024;
        let limits = http_body_limits_from(128 * mib);
        assert_eq!(limits.decoded, 128 * mib);
        assert_eq!(limits.encoded, 256 * mib);

        let limits = http_body_limits_from(MAX_HTTP_BODY_BYTES);
        assert_eq!(limits.decoded, MAX_HTTP_BODY_BYTES);
        assert_eq!(limits.encoded, MAX_HTTP_BODY_BYTES);

        // 低于下限的输入按 32 MiB 处理，避免配置错误把上限缩到不可用。
        assert_eq!(http_body_limits_from(1).decoded, 32 * mib);
    }

    #[tokio::test]
    async fn read_http_request_uses_injected_limit_instead_of_global_default() {
        let limits = HttpBodyLimits {
            decoded: TEST_HTTP_BODY_BYTES,
            encoded: TEST_HTTP_BODY_BYTES * 2,
        };

        // 超过注入上限：只发请求头，服务端必须在读正文前拒绝。
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let client = tokio::spawn(async move {
            let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
            stream
                .write_all(
                    format!(
                        "POST /v1/responses HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
                        TEST_HTTP_BODY_BYTES + 1
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        });
        let (mut server, _) = listener.accept().await.unwrap();
        let error = read_http_request_with_limits(&mut server, limits)
            .await
            .err()
            .expect("oversized body should be rejected");
        assert_eq!(error.status(), "413 Payload Too Large");
        client.await.unwrap();

        // 正好等于注入上限：正常读入正文。
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let payload = "a".repeat(TEST_HTTP_BODY_BYTES);
        let request_text = format!(
            "POST /v1/responses HTTP/1.1\r\nContent-Length: {}\r\n\r\n{}",
            TEST_HTTP_BODY_BYTES, payload
        );
        let client = tokio::spawn(async move {
            let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
            stream.write_all(request_text.as_bytes()).await.unwrap();
        });
        let (mut server, _) = listener.accept().await.unwrap();
        let request = read_http_request_with_limits(&mut server, limits)
            .await
            .unwrap();
        assert_eq!(request.body.len(), TEST_HTTP_BODY_BYTES);
        client.await.unwrap();
    }

    #[tokio::test]
    async fn helper_returns_400_for_ambiguous_body_framing() {
        let response = send_raw_helper_request(
            b"POST /v1/audio/transcriptions HTTP/1.1\r\nContent-Type: multipart/form-data; boundary=x\r\nContent-Length: 4\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n",
        )
        .await;

        assert!(String::from_utf8_lossy(&response).starts_with("HTTP/1.1 400 Bad Request"));
    }

    #[tokio::test]
    async fn helper_returns_413_before_reading_oversized_content_length_body() {
        let request = format!(
            "POST /v1/audio/transcriptions HTTP/1.1\r\nContent-Type: multipart/form-data; boundary=x\r\nContent-Length: {}\r\n\r\n",
            DEFAULT_HTTP_BODY_BYTES + 1
        );
        let response = send_raw_helper_request(request.as_bytes()).await;

        assert!(String::from_utf8_lossy(&response).starts_with("HTTP/1.1 413 Payload Too Large"));
    }

    #[tokio::test]
    async fn helper_returns_400_for_oversized_headers() {
        let mut request = b"GET /backend/status HTTP/1.1\r\nX-Large: ".to_vec();
        request.resize(MAX_HTTP_HEADER_BYTES + 1, b'a');
        request.extend_from_slice(b"\r\n\r\n");
        let response = send_raw_helper_request(&request).await;

        assert!(String::from_utf8_lossy(&response).starts_with("HTTP/1.1 400 Bad Request"));
    }

    #[tokio::test]
    async fn helper_returns_426_for_responses_websocket_upgrade() {
        let response = send_raw_helper_request(
            b"GET /v1/responses HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\n",
        )
        .await;

        assert!(String::from_utf8_lossy(&response).starts_with("HTTP/1.1 426 Upgrade Required"));
    }

    #[tokio::test]
    async fn helper_keeps_unknown_image_path_as_not_found() {
        let response = send_raw_helper_request(
            b"POST /v1/images/unknown HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
        )
        .await;

        let response = String::from_utf8_lossy(&response);
        assert!(response.starts_with("HTTP/1.1 404 Not Found"));
        assert!(response.contains("未知后端路径"));
    }

    #[tokio::test]
    async fn helper_proxies_image_generation_upstream_error_response() {
        let _settings_guard = crate::paths::settings_path_test_guard();
        let temp = tempfile::tempdir().unwrap();
        let settings_path = temp.path().join("settings.json");
        let previous_settings_path =
            crate::paths::set_settings_path_for_tests(Some(settings_path.clone()));
        let upstream_listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let upstream_addr = upstream_listener.local_addr().unwrap();
        let settings = serde_json::json!({
            "relayProfiles": [{
                "id": "images",
                "name": "Images",
                "baseUrl": format!("http://{upstream_addr}/v1"),
                "upstreamBaseUrl": format!("http://{upstream_addr}/v1"),
                "apiKey": "sk-test",
                "protocol": "responses",
                "relayMode": "mixedApi"
            }],
            "activeRelayId": "images"
        });
        std::fs::write(settings_path, serde_json::to_vec_pretty(&settings).unwrap()).unwrap();
        let upstream = tokio::spawn(async move {
            let (mut stream, _) = upstream_listener.accept().await.unwrap();
            let request = read_http_request(&mut stream).await.unwrap();
            let body = br#"{"error":{"message":"rate limited"}}"#;
            let response = format!(
                "HTTP/1.1 429 Too Many Requests\r\nContent-Type: application/problem+json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
            stream.write_all(body).await.unwrap();
            request
        });
        let request_body = br#"{"model":"gpt-image-2","prompt":"draw a square"}"#;
        let headers = format!(
            "POST /v1/images/generations HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            request_body.len()
        );
        let mut request = headers.into_bytes();
        request.extend_from_slice(request_body);

        let response = send_raw_helper_request(&request).await;

        let response_text = String::from_utf8_lossy(&response);
        assert!(response_text.starts_with("HTTP/1.1 429 Too Many Requests"));
        assert!(response_text.contains("Content-Type: application/problem+json"));
        assert!(response.ends_with(br#"{"error":{"message":"rate limited"}}"#));
        let upstream_request = upstream.await.unwrap();
        let request_line = String::from_utf8_lossy(&upstream_request.headers);
        assert!(request_line.starts_with("POST /v1/images/generations HTTP/1.1"));
        assert_eq!(upstream_request.body, request_body);
        crate::paths::set_settings_path_for_tests(previous_settings_path);
    }

    #[test]
    fn protocol_proxy_request_body_decodes_zstd() {
        let body = br#"{"model":"gpt-5.6-sol","input":"probe","stream":false}"#;
        let compressed = zstd::stream::encode_all(std::io::Cursor::new(body), 3).unwrap();

        let decoded = decode_protocol_proxy_request_body(&compressed, Some("zstd")).unwrap();

        assert_eq!(decoded.as_bytes(), body);
        assert!(decode_protocol_proxy_request_body(body, Some("gzip")).is_err());
    }

    #[tokio::test]
    async fn helper_replaces_chatgpt_auth_when_proxying_zstd_responses_request() {
        let _settings_guard = crate::paths::settings_path_test_guard();
        let temp = tempfile::tempdir().unwrap();
        let settings_path = temp.path().join("settings.json");
        let previous_settings_path =
            crate::paths::set_settings_path_for_tests(Some(settings_path.clone()));
        let upstream_listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let upstream_addr = upstream_listener.local_addr().unwrap();
        let settings = serde_json::json!({
            "relayProfilesEnabled": true,
            "relayProfiles": [{
                "id": "remote-control",
                "name": "Remote Control Relay",
                "baseUrl": format!("http://{upstream_addr}/v1"),
                "upstreamBaseUrl": format!("http://{upstream_addr}/v1"),
                "apiKey": "sk-upstream",
                "protocol": "responses",
                "relayMode": "official",
                "officialMixApiKey": true
            }],
            "activeRelayId": "remote-control"
        });
        std::fs::write(settings_path, serde_json::to_vec_pretty(&settings).unwrap()).unwrap();

        let upstream = tokio::spawn(async move {
            let (mut stream, _) = upstream_listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 4096];
            let mut expected_len = None;
            loop {
                let read = stream.read(&mut buffer).await.unwrap();
                assert!(read > 0, "upstream request ended before body completed");
                request.extend_from_slice(&buffer[..read]);
                if expected_len.is_none() {
                    if let Some(header_end) = find_header_end(&request) {
                        let headers = String::from_utf8_lossy(&request[..header_end]);
                        let content_length = header_value_from_headers(&headers, "content-length")
                            .unwrap()
                            .parse::<usize>()
                            .unwrap();
                        expected_len = Some(header_end + 4 + content_length);
                    }
                }
                if expected_len.is_some_and(|length| request.len() >= length) {
                    break;
                }
            }
            let response_body = br#"{"id":"resp_remote","object":"response"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                response_body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
            stream.write_all(response_body).await.unwrap();
            request
        });

        let body = br#"{"model":"gpt-5.6-sol","input":"probe","stream":false}"#;
        let compressed = zstd::stream::encode_all(std::io::Cursor::new(body), 3).unwrap();
        let helper_listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let helper_addr = helper_listener.local_addr().unwrap();
        let helper = tokio::spawn(async move {
            let (stream, remote_addr) = helper_listener.accept().await.unwrap();
            handle_helper_connection(stream, Some(remote_addr))
                .await
                .unwrap();
        });
        let mut client = tokio::net::TcpStream::connect(helper_addr).await.unwrap();
        let headers = format!(
            "POST /v1/responses HTTP/1.1\r\nHost: {helper_addr}\r\nAuthorization: Bearer chatgpt-secret\r\nSeSsIoN-Id: conversation-123\r\nTHREAD-ID: thread-123\r\nX-OpenCode-Session: opencode-123\r\nCookie: private-cookie\r\nX-Unrelated: private-value\r\nContent-Type: application/json\r\nContent-Encoding: zstd\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            compressed.len()
        );
        client.write_all(headers.as_bytes()).await.unwrap();
        client.write_all(&compressed).await.unwrap();
        let mut response = Vec::new();
        client.read_to_end(&mut response).await.unwrap();
        assert!(String::from_utf8_lossy(&response).starts_with("HTTP/1.1 200 OK"));

        helper.await.unwrap();
        let upstream_request = upstream.await.unwrap();
        let header_end = find_header_end(&upstream_request).unwrap();
        let upstream_headers =
            String::from_utf8_lossy(&upstream_request[..header_end]).to_ascii_lowercase();
        assert!(upstream_headers.starts_with("post /v1/responses http/1.1"));
        assert!(upstream_headers.contains("authorization: bearer sk-upstream"));
        assert!(!upstream_headers.contains("chatgpt-secret"));
        assert!(upstream_headers.contains("session-id: conversation-123"));
        assert!(upstream_headers.contains("thread-id: thread-123"));
        assert!(upstream_headers.contains("x-opencode-session: opencode-123"));
        assert!(!upstream_headers.contains("private-cookie"));
        assert!(!upstream_headers.contains("private-value"));
        let upstream_body: serde_json::Value =
            serde_json::from_slice(&upstream_request[header_end + 4..]).unwrap();
        assert_eq!(upstream_body["model"], "gpt-5.6-sol");
        crate::paths::set_settings_path_for_tests(previous_settings_path);
    }

    #[tokio::test]
    async fn helper_writes_request_log_with_usage_and_trace() {
        let _log_guard = crate::request_log::request_log_test_guard();
        let _settings_guard = crate::paths::settings_path_test_guard();
        let temp = tempfile::tempdir().unwrap();
        let settings_path = temp.path().join("settings.json");
        let log_path = temp.path().join("request-log.jsonl");
        let previous_settings_path =
            crate::paths::set_settings_path_for_tests(Some(settings_path.clone()));
        crate::request_log::set_request_log_path_for_tests(Some(log_path.clone()));

        let upstream_listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let upstream_addr = upstream_listener.local_addr().unwrap();
        let settings = serde_json::json!({
            "relayProfilesEnabled": true,
            "relayProfiles": [{
                "id": "request-log-relay",
                "name": "请求日志供应商",
                "baseUrl": format!("http://{upstream_addr}/v1"),
                "upstreamBaseUrl": format!("http://{upstream_addr}/v1"),
                "apiKey": "sk-upstream",
                "protocol": "responses",
                "relayMode": "pureApi"
            }],
            "activeRelayId": "request-log-relay"
        });
        std::fs::write(settings_path, serde_json::to_vec_pretty(&settings).unwrap()).unwrap();

        let upstream = tokio::spawn(async move {
            let (mut stream, _) = upstream_listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 4096];
            let mut expected_len = None;
            loop {
                let read = stream.read(&mut buffer).await.unwrap();
                assert!(read > 0, "upstream request ended before body completed");
                request.extend_from_slice(&buffer[..read]);
                if expected_len.is_none() {
                    if let Some(header_end) = find_header_end(&request) {
                        let headers = String::from_utf8_lossy(&request[..header_end]);
                        let content_length = header_value_from_headers(&headers, "content-length")
                            .unwrap()
                            .parse::<usize>()
                            .unwrap();
                        expected_len = Some(header_end + 4 + content_length);
                    }
                }
                if expected_len.is_some_and(|length| request.len() >= length) {
                    break;
                }
            }
            let response_body = br#"{"id":"resp_logged","object":"response","usage":{"input_tokens":120,"output_tokens":30,"input_tokens_details":{"cached_tokens":100}}}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                response_body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
            stream.write_all(response_body).await.unwrap();
            request
        });

        let helper_listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let helper_addr = helper_listener.local_addr().unwrap();
        let helper = tokio::spawn(async move {
            let (stream, remote_addr) = helper_listener.accept().await.unwrap();
            handle_helper_connection(stream, Some(remote_addr))
                .await
                .unwrap();
        });
        let mut client = tokio::net::TcpStream::connect(helper_addr).await.unwrap();
        let body = br#"{"model":"gpt-6.1-sol","input":"probe","stream":false}"#;
        let headers = format!(
            "POST /v1/responses HTTP/1.1\r\nHost: {helper_addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        client.write_all(headers.as_bytes()).await.unwrap();
        client.write_all(body).await.unwrap();
        let mut response = Vec::new();
        client.read_to_end(&mut response).await.unwrap();
        assert!(String::from_utf8_lossy(&response).starts_with("HTTP/1.1 200 OK"));

        helper.await.unwrap();
        upstream.await.unwrap();

        let entries = crate::request_log::read_request_logs_at(&log_path, 10).unwrap();
        assert_eq!(entries.len(), 1, "每次代理请求写一条请求日志");
        let entry = &entries[0];
        assert_eq!(entry.status_code, 200);
        assert!(entry.success);
        assert_eq!(entry.model, "gpt-6.1-sol");
        assert_eq!(entry.upstream_model, "gpt-6.1-sol");
        assert_eq!(entry.relay_name, "请求日志供应商");
        assert_eq!(entry.attempts, 1);
        assert_eq!(entry.auto_retries, 0);
        assert_eq!(
            entry.first_token_ms,
            Some(entry.duration_ms),
            "非流式请求用总耗时作为首字耗时"
        );
        let usage = entry.usage.as_ref().expect("usage");
        assert_eq!(usage.input_tokens, 120);
        assert_eq!(usage.output_tokens, 30);
        assert_eq!(usage.cached_tokens, 100);
        assert_eq!(entry.cache_hit_ratio, Some(100.0 / 120.0));

        crate::request_log::set_request_log_path_for_tests(None);
        crate::paths::set_settings_path_for_tests(previous_settings_path);
    }

    #[tokio::test]
    async fn helper_buffered_mode_retries_overload_and_hides_error() {
        let _log_guard = crate::request_log::request_log_test_guard();
        let _settings_guard = crate::paths::settings_path_test_guard();
        let temp = tempfile::tempdir().unwrap();
        let settings_path = temp.path().join("settings.json");
        let log_path = temp.path().join("request-log.jsonl");
        let previous_settings_path =
            crate::paths::set_settings_path_for_tests(Some(settings_path.clone()));
        crate::request_log::set_request_log_path_for_tests(Some(log_path.clone()));

        let upstream_listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let upstream_addr = upstream_listener.local_addr().unwrap();
        let settings = serde_json::json!({
            "relayProfilesEnabled": true,
            "relayProfiles": [{
                "id": "buffered-overload-relay",
                "name": "缓冲重试供应商",
                "baseUrl": format!("http://{upstream_addr}/v1"),
                "upstreamBaseUrl": format!("http://{upstream_addr}/v1"),
                "apiKey": "sk-upstream",
                "protocol": "responses",
                "relayMode": "pureApi"
            }],
            "activeRelayId": "buffered-overload-relay",
            "protocolProxyAutoRetryMode": "buffered",
            "protocolProxyAutoRetryMaxAttempts": 2,
            "protocolProxyAutoRetryIntervalSeconds": 0
        });
        std::fs::write(settings_path, serde_json::to_vec_pretty(&settings).unwrap()).unwrap();

        let upstream = tokio::spawn(async move {
            let overloaded = concat!(
                "event: response.created\n",
                "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_bad\",\"status\":\"in_progress\"}}\n\n",
                "event: error\n",
                "data: {\"type\":\"error\",\"error\":{\"code\":\"server_is_overloaded\",\"message\":\"Our servers are currently overloaded. Please try again later.\"}}\n\n",
            );
            let ok = concat!(
                "event: response.created\n",
                "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_ok\",\"status\":\"in_progress\"}}\n\n",
                "event: response.output_text.delta\n",
                "data: {\"type\":\"response.output_text.delta\",\"delta\":\"ok\"}\n\n",
                "event: response.completed\n",
                "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_ok\",\"status\":\"completed\",\"usage\":{\"input_tokens\":12,\"output_tokens\":3}}}\n\n",
            );
            for body in [overloaded, ok] {
                let (mut stream, _) = upstream_listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut buffer = [0_u8; 4096];
                let mut expected_len = None;
                loop {
                    let read = stream.read(&mut buffer).await.unwrap();
                    assert!(read > 0);
                    request.extend_from_slice(&buffer[..read]);
                    if expected_len.is_none() {
                        if let Some(header_end) = find_header_end(&request) {
                            let headers = String::from_utf8_lossy(&request[..header_end]);
                            let content_length = header_value_from_headers(&headers, "content-length")
                                .unwrap()
                                .parse::<usize>()
                                .unwrap();
                            expected_len = Some(header_end + 4 + content_length);
                        }
                    }
                    if expected_len.is_some_and(|length| request.len() >= length) {
                        break;
                    }
                }
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
                stream.write_all(body.as_bytes()).await.unwrap();
            }
        });

        let helper_listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let helper_addr = helper_listener.local_addr().unwrap();
        let helper = tokio::spawn(async move {
            let (stream, remote_addr) = helper_listener.accept().await.unwrap();
            handle_helper_connection(stream, Some(remote_addr))
                .await
                .unwrap();
        });
        let mut client = tokio::net::TcpStream::connect(helper_addr).await.unwrap();
        let body = br#"{"model":"gpt-6.1-sol","input":"probe","stream":true}"#;
        let headers = format!(
            "POST /v1/responses HTTP/1.1\r\nHost: {helper_addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        client.write_all(headers.as_bytes()).await.unwrap();
        client.write_all(body).await.unwrap();
        let mut response = Vec::new();
        client.read_to_end(&mut response).await.unwrap();
        helper.await.unwrap();
        upstream.await.unwrap();

        let text = String::from_utf8_lossy(&response);
        assert!(text.contains("resp_ok"), "{text}");
        assert!(!text.contains("server_is_overloaded"), "buffered 模式不应把过载错误交给客户端");
        let entries = crate::request_log::read_request_logs_at(&log_path, 10).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].success);
        assert_eq!(entries[0].attempts, 2);
        assert_eq!(entries[0].auto_retries, 1);
        assert_eq!(entries[0].usage.as_ref().map(|u| u.input_tokens), Some(12));

        crate::request_log::set_request_log_path_for_tests(None);
        crate::paths::set_settings_path_for_tests(previous_settings_path);
    }

    #[tokio::test]
    async fn helper_records_upstream_stream_error_detail_in_request_log() {
        let _log_guard = crate::request_log::request_log_test_guard();
        let _settings_guard = crate::paths::settings_path_test_guard();
        let temp = tempfile::tempdir().unwrap();
        let settings_path = temp.path().join("settings.json");
        let log_path = temp.path().join("request-log.jsonl");
        let previous_settings_path =
            crate::paths::set_settings_path_for_tests(Some(settings_path.clone()));
        crate::request_log::set_request_log_path_for_tests(Some(log_path.clone()));

        let upstream_listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let upstream_addr = upstream_listener.local_addr().unwrap();
        let settings = serde_json::json!({
            "relayProfilesEnabled": true,
            "relayProfiles": [{
                "id": "stream-error-relay",
                "name": "流错误供应商",
                "baseUrl": format!("http://{upstream_addr}/v1"),
                "upstreamBaseUrl": format!("http://{upstream_addr}/v1"),
                "apiKey": "sk-upstream",
                "protocol": "responses",
                "relayMode": "pureApi"
            }],
            "activeRelayId": "stream-error-relay"
        });
        std::fs::write(settings_path, serde_json::to_vec_pretty(&settings).unwrap()).unwrap();

        let upstream = tokio::spawn(async move {
            let (mut stream, _) = upstream_listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 4096];
            let mut expected_len = None;
            loop {
                let read = stream.read(&mut buffer).await.unwrap();
                assert!(read > 0);
                request.extend_from_slice(&buffer[..read]);
                if expected_len.is_none() {
                    if let Some(header_end) = find_header_end(&request) {
                        let headers = String::from_utf8_lossy(&request[..header_end]);
                        let content_length = header_value_from_headers(&headers, "content-length")
                            .unwrap()
                            .parse::<usize>()
                            .unwrap();
                        expected_len = Some(header_end + 4 + content_length);
                    }
                }
                if expected_len.is_some_and(|length| request.len() >= length) {
                    break;
                }
            }
            let body = concat!(
                "event: response.created\n",
                "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_error\"}}\n\n",
                "event: error\n",
                "data: {\"type\":\"error\",\"error\":{\"code\":\"server_error\",\"message\":\"upstream exploded\"}}\n\n",
            );
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
            stream.write_all(body.as_bytes()).await.unwrap();
        });

        let helper_listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let helper_addr = helper_listener.local_addr().unwrap();
        let helper = tokio::spawn(async move {
            let (stream, remote_addr) = helper_listener.accept().await.unwrap();
            handle_helper_connection(stream, Some(remote_addr))
                .await
                .unwrap();
        });
        let mut client = tokio::net::TcpStream::connect(helper_addr).await.unwrap();
        let body = br#"{"model":"gpt-6.1-sol","input":"probe","stream":true}"#;
        let headers = format!(
            "POST /v1/responses HTTP/1.1\r\nHost: {helper_addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        client.write_all(headers.as_bytes()).await.unwrap();
        client.write_all(body).await.unwrap();
        let mut response = Vec::new();
        client.read_to_end(&mut response).await.unwrap();
        helper.await.unwrap();
        upstream.await.unwrap();

        let entries = crate::request_log::read_request_logs_at(&log_path, 10).unwrap();
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.status_code, 200);
        assert!(!entry.success, "流内 error 事件应记为失败");
        assert_eq!(
            entry.stream_outcome.as_deref(),
            Some("原生 Responses 上游返回 error 事件"),
            "客户端可见的原因保持泛化，不泄露 payload"
        );
        let error = entry.error.as_deref().unwrap_or("");
        assert!(error.contains("server_error"), "{error}");
        assert!(error.contains("upstream exploded"), "{error}");

        crate::request_log::set_request_log_path_for_tests(None);
        crate::paths::set_settings_path_for_tests(previous_settings_path);
    }

    #[tokio::test]
    async fn helper_protocol_safety_rejects_files_before_upstream_and_checks_original_compaction_wire()
     {
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
        let _settings_guard = crate::paths::settings_path_test_guard();
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings.json");
        struct Restore(Option<PathBuf>);
        impl Drop for Restore {
            fn drop(&mut self) {
                crate::paths::set_settings_path_for_tests(self.0.take());
            }
        }
        let _restore = Restore(crate::paths::set_settings_path_for_tests(Some(
            path.clone(),
        )));
        async fn request(body: serde_json::Value) -> String {
            let body = body.to_string();
            let raw = format!(
                "POST /v1/responses HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            String::from_utf8(send_raw_helper_request(raw.as_bytes()).await).unwrap()
        }
        let write_profile = |protocol: &str, url: &str| {
            std::fs::write(&path, serde_json::to_vec(&serde_json::json!({"relayProfilesEnabled":true,
                "activeRelayId":"safety", "relayProfiles":[{"id":"safety","name":"Safety fixture","baseUrl":format!("{url}/v1"),
                "apiKey":"fixture-only","protocol":protocol,"relayMode":"pureApi"}]})).unwrap()).unwrap();
        };
        for protocol in ["chatCompletions", "anthropicMessages"] {
            let upstream = MockServer::start().await;
            write_profile(protocol, &upstream.uri());
            let response = request(
                serde_json::json!({"model":"fixture","input":[{"role":"user","content":[
                {"type":"input_file","file_data":"private-file-payload"}]}]}),
            )
            .await;
            assert!(
                response.starts_with("HTTP/1.1 400 Bad Request"),
                "{response}"
            );
            assert!(response.contains("unsupported_input_file"));
            assert!(response.contains("input[0].content[0]"));
            assert!(!response.contains("private-file-payload"));
            assert!(upstream.received_requests().await.unwrap().is_empty());
        }
        let anthropic_prefix = concat!(
            "data: {\"type\":\"message_start\",\"message\":{\"id\":\"fixture\",\"model\":\"fixture\",\"usage\":{}}}\n\n",
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"summary\"}}\n\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{}}\n\n"
        );
        for (protocol, stream, payload, complete) in [
            ("chatCompletions",false,serde_json::json!({"choices":[{"message":{"content":"summary"},"finish_reason":"stop"}]}).to_string(),true),
            ("chatCompletions",false,serde_json::json!({"choices":[{"message":{"content":"partial"},"finish_reason":"length"}]}).to_string(),false),
            ("anthropicMessages",false,serde_json::json!({"type":"message","content":[{"type":"text","text":"summary"}],"stop_reason":"end_turn"}).to_string(),true),
            ("anthropicMessages",false,serde_json::json!({"type":"message","content":[{"type":"text","text":"partial"}],"stop_reason":null}).to_string(),false),
            ("chatCompletions",true,"data: {\"choices\":[{\"delta\":{\"content\":\"summary\"},\"finish_reason\":\"stop\"}]}\n\n".to_string(),true),
            ("chatCompletions",true,"data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\ndata: [DONE]\n\n".to_string(),false),
            ("anthropicMessages",true,format!("{anthropic_prefix}data: {{\"type\":\"message_stop\"}}\n\n"),true),
            ("anthropicMessages",true,anthropic_prefix.to_string(),false),
            ("anthropicMessages",true,format!("{anthropic_prefix}data: [DONE]\n\n"),false),
        ] {
            let upstream = MockServer::start().await;
            Mock::given(method("POST")).respond_with(ResponseTemplate::new(200)
                .set_body_string(payload).insert_header("Content-Type", if stream {"text/event-stream"} else {"application/json"}))
                .mount(&upstream).await;
            write_profile(protocol, &upstream.uri());
            let response = request(serde_json::json!({"model":"fixture","stream":stream,"input":[{"type":"compaction_trigger"}]})).await;
            assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
            assert_eq!(response.contains("event: response.completed"), complete, "{protocol}: {response}");
            assert_eq!(response.contains("event: response.output_item.done"), complete, "{protocol}: {response}");
            assert_eq!(response.contains("event: response.failed"), !complete, "{protocol}: {response}");
            assert_eq!(upstream.received_requests().await.unwrap().len(), 1);
        }
    }

    #[tokio::test]
    async fn helper_rejects_encrypted_agent_content_as_bad_request_before_upstream_send() {
        let _settings_guard = crate::paths::settings_path_test_guard();
        let temp = tempfile::tempdir().unwrap();
        let settings_path = temp.path().join("settings.json");
        struct RestoreSettingsPath(Option<PathBuf>);
        impl Drop for RestoreSettingsPath {
            fn drop(&mut self) {
                crate::paths::set_settings_path_for_tests(self.0.take());
            }
        }
        let _restore = RestoreSettingsPath(crate::paths::set_settings_path_for_tests(Some(
            settings_path.clone(),
        )));
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        std::fs::write(
            settings_path,
            serde_json::to_vec(&serde_json::json!({
                "activeRelayId":"encrypted-agent-test",
                "relayProfiles":[{"id":"encrypted-agent-test","name":"Encrypted agent test",
                    "baseUrl":format!("http://{address}/v1"),"apiKey":"fixture-api-key",
                    "protocol":"chatCompletions","relayMode":"mixedApi"}]
            }))
            .unwrap(),
        )
        .unwrap();
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let upstream = tokio::spawn(async move {
            tokio::select! {
                accepted = listener.accept() => {
                    let (mut socket, _) = accepted.unwrap();
                    let _ = read_http_request(&mut socket).await.unwrap();
                    let body = br#"{"choices":[{"message":{"role":"assistant","content":"wrong"},"finish_reason":"stop"}]}"#;
                    socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes()).await.unwrap();
                    socket.write_all(body).await.unwrap();
                    true
                },
                _ = stopped => false,
            }
        });
        let body = br#"{"model":"custom-model","stream":false,"input":[{"type":"encrypted_content","encrypted_content":"fixture-private-payload"}]}"#;
        let mut request = format!("POST /v1/responses HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).into_bytes();
        request.extend_from_slice(body);
        let response = send_raw_helper_request(&request).await;
        let _ = stop.send(());
        let contacted = upstream.await.unwrap();
        let response = String::from_utf8(response).unwrap();
        assert!(
            response.starts_with("HTTP/1.1 400 Bad Request"),
            "{response}"
        );
        let body = response.split_once("\r\n\r\n").unwrap().1;
        let error: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(error["error"]["type"], "invalid_request_error");
        assert_eq!(
            error["error"]["code"],
            "unsupported_encrypted_agent_content"
        );
        assert!(!response.contains("fixture-private-payload"));
        assert!(!response.contains("fixture-api-key"));
        assert!(!contacted);

        // 只有新增的 typed unsupported 错误映射为 400，其余请求错误沿用 502。
        let response = send_raw_helper_request(b"POST /v1/responses HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: 1\r\nConnection: close\r\n\r\n{").await;
        let response = String::from_utf8(response).unwrap();
        assert!(response.starts_with("HTTP/1.1 502 Bad Gateway"));
        let body: serde_json::Value =
            serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(body["status"], "failed");
        assert!(!response.contains("unsupported_encrypted_agent_content"));
    }

    #[tokio::test]
    async fn completed_exec_stream_does_not_wait_for_upstream_eof() {
        let _settings_guard = crate::paths::settings_path_test_guard();
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings.json");
        let upstream_listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let upstream_addr = upstream_listener.local_addr().unwrap();
        let settings = BackendSettings {
            active_relay_id: "terminal-stream".into(),
            relay_profiles: vec![crate::settings::RelayProfile {
                id: "terminal-stream".into(),
                base_url: format!("http://{upstream_addr}/v1"),
                protocol: crate::settings::RelayProtocol::Responses,
                relay_mode: crate::settings::RelayMode::PureApi,
                api_key: "fixture-only".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        crate::settings::SettingsStore::new(path.clone())
            .save(&settings)
            .unwrap();
        let previous = crate::paths::set_settings_path_for_tests(Some(path));
        let upstream = tokio::spawn(async move {
            for attempt in 0..2 {
                let (mut socket, _) = upstream_listener.accept().await.unwrap();
                let mut buffer = Vec::new();
                loop {
                    let mut chunk = [0; 4096];
                    let read = socket.read(&mut chunk).await.unwrap();
                    assert!(read > 0);
                    buffer.extend_from_slice(&chunk[..read]);
                    if let Some(end) = find_header_end(&buffer) {
                        let headers = String::from_utf8_lossy(&buffer[..end]);
                        let length = header_value_from_headers(&headers, "content-length")
                            .unwrap()
                            .parse::<usize>()
                            .unwrap();
                        if buffer.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                if attempt == 0 {
                    let rejection = serde_json::json!({
                        "error": {
                            "type": "invalid_request_error",
                            "message": crate::responses_exec::EXEC_REJECTION_MESSAGE
                        }
                    })
                    .to_string();
                    write_http_response(
                        &mut socket,
                        "400 Bad Request",
                        "application/json",
                        rejection.as_bytes(),
                    )
                    .await
                    .unwrap();
                } else {
                    let event = serde_json::json!({
                        "type": "response.completed",
                        "response": {
                            "object": "response",
                            "status": "completed",
                            "output": [{
                                "type": "function_call",
                                "name": "exec",
                                "id": "fc_review",
                                "call_id": "review",
                                "arguments": "{\"input\":\"text('ok')\"}"
                            }]
                        }
                    });
                    let payload =
                        format!("event: response.completed\ndata: {event}\n\ndata: [DONE]\n\n");
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{payload}\r\n",
                        payload.len()
                    );
                    socket.write_all(response.as_bytes()).await.unwrap();
                    std::future::pending::<()>().await;
                }
            }
        });
        let helper_listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let helper_addr = helper_listener.local_addr().unwrap();
        let helper = tokio::spawn(async move {
            let (socket, address) = helper_listener.accept().await.unwrap();
            handle_helper_connection(socket, Some(address)).await
        });
        let body = serde_json::json!({
            "model": "terminal-stream",
            "stream": true,
            "store": false,
            "input": "fixture",
            "tools": [{"type": "custom", "name": "exec", "format": {"type": "text"}}]
        })
        .to_string();
        let request = format!(
            "POST /v1/responses HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let mut client = tokio::net::TcpStream::connect(helper_addr).await.unwrap();
        client.write_all(request.as_bytes()).await.unwrap();
        let mut response = Vec::new();
        // 先等待终态事件，再计时关闭，隔离并行测试的启动延迟。
        let terminal_seen = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let read = client.read_buf(&mut response).await?;
                if read == 0
                    || String::from_utf8_lossy(&response).contains("event: response.completed")
                {
                    return Ok::<_, std::io::Error>(());
                }
            }
        })
        .await;
        let completed = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            client.read_to_end(&mut response),
        )
        .await;
        upstream.abort();
        helper.abort();
        let _ = upstream.await;
        let _ = helper.await;
        crate::paths::set_settings_path_for_tests(previous);
        drop(_settings_guard);
        let response = String::from_utf8(response).unwrap();
        assert!(
            matches!(terminal_seen, Ok(Ok(()))),
            "{terminal_seen:?}: {response}"
        );
        assert!(response.contains("event: response.completed"), "{response}");
        assert!(response.contains("data: [DONE]"), "{response}");
        assert!(
            matches!(completed, Ok(Ok(_))),
            "completed exec SSE waits for upstream EOF: {response}"
        );
    }

    async fn send_raw_helper_request(request: &[u8]) -> Vec<u8> {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let helper = tokio::spawn(async move {
            let (stream, remote_addr) = listener.accept().await.unwrap();
            handle_helper_connection(stream, Some(remote_addr))
                .await
                .unwrap();
        });
        let mut client = tokio::net::TcpStream::connect(address).await.unwrap();
        client.write_all(request).await.unwrap();
        let mut response = Vec::new();
        client.read_to_end(&mut response).await.unwrap();
        helper.await.unwrap();
        response
    }

    #[tokio::test]
    async fn helper_serves_adapted_exec_json_and_sse_without_exposing_the_wrapper() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let _settings_guard = crate::paths::settings_path_test_guard();
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings.json");
        let server = MockServer::start().await;
        let settings = BackendSettings {
            active_relay_id: "served-exec".to_string(),
            relay_profiles: vec![crate::settings::RelayProfile {
                id: "served-exec".to_string(),
                base_url: format!("{}/v1", server.uri()),
                protocol: crate::settings::RelayProtocol::Responses,
                relay_mode: crate::settings::RelayMode::PureApi,
                api_key: "fixture-only".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        };
        crate::settings::SettingsStore::new(path.clone())
            .save(&settings)
            .unwrap();
        let previous = crate::paths::set_settings_path_for_tests(Some(path));
        let program = "text(\"served exec\");";
        let arguments = serde_json::json!({"input":program}).to_string();
        let item = serde_json::json!({
            "type":"function_call", "id":"fc_served", "call_id":"call_served",
            "name":"exec", "arguments":arguments, "status":"completed"
        });
        let response = serde_json::json!({
            "object":"response", "status":"completed", "output":[item]
        });
        Mock::given(method("POST"))
            .respond_with(move |incoming: &wiremock::Request| {
                let body: serde_json::Value = incoming.body_json().unwrap();
                if body["tools"][0]["type"] == "custom" {
                    return ResponseTemplate::new(400).set_body_json(serde_json::json!({
                        "error":{"type":"invalid_request_error",
                                 "message":crate::responses_exec::EXEC_REJECTION_MESSAGE,
                                 "param":"tools[0]"}
                    }));
                }
                if body["stream"] == true {
                    let event =
                        serde_json::json!({"type":"response.completed","response":response});
                    ResponseTemplate::new(200)
                        .insert_header("content-type", "text/event-stream")
                        .set_body_string(format!(
                            "event: response.completed\ndata: {event}\n\ndata: [DONE]\n\n"
                        ))
                } else {
                    ResponseTemplate::new(200).set_body_json(&response)
                }
            })
            .mount(&server)
            .await;
        for is_stream in [false, true] {
            let body = serde_json::json!({
                "model": if is_stream {"stream-model"} else {"json-model"},
                "stream":is_stream, "store":false, "input":"fixture",
                "tools":[{"type":"custom","name":"exec","format":{"type":"text"}}]
            })
            .to_string();
            let request = format!(
                "POST /v1/responses HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let bytes = tokio::time::timeout(
                std::time::Duration::from_secs(10),
                send_raw_helper_request(request.as_bytes()),
            )
            .await
            .unwrap();
            let headers_end = find_header_end(&bytes).unwrap();
            let body = String::from_utf8(bytes[headers_end + 4..].to_vec()).unwrap();
            assert!(body.contains("custom_tool_call"), "{body}");
            assert!(!body.contains("function_call"), "{body}");
            assert!(!body.contains("\"arguments\""), "{body}");
            if is_stream {
                assert!(body.contains("response.custom_tool_call_input.delta"));
                assert!(body.contains("data: [DONE]"));
            } else {
                let value: serde_json::Value = serde_json::from_str(&body).unwrap();
                assert_eq!(value["output"][0]["input"], program);
            }
        }
        assert_eq!(server.received_requests().await.unwrap().len(), 4);
        crate::paths::set_settings_path_for_tests(previous);
    }

    #[tokio::test]
    async fn helper_decodes_fragmented_chunked_binary_multipart_body() {
        let _settings_guard = crate::paths::settings_path_test_guard();
        let temp = tempfile::tempdir().unwrap();
        let settings_path = temp.path().join("settings.json");
        let previous_settings_path =
            crate::paths::set_settings_path_for_tests(Some(settings_path.clone()));
        let upstream_listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let upstream_addr = upstream_listener.local_addr().unwrap();
        let settings = serde_json::json!({
            "relayProfiles": [{
                "id": "audio",
                "name": "Audio",
                "baseUrl": format!("http://{upstream_addr}/v1"),
                "upstreamBaseUrl": format!("http://{upstream_addr}/v1"),
                "apiKey": "sk-test",
                "protocol": "chatCompletions",
                "relayMode": "mixedApi"
            }],
            "activeRelayId": "audio"
        });
        std::fs::write(settings_path, serde_json::to_vec_pretty(&settings).unwrap()).unwrap();

        let boundary = "codex-binary-boundary";
        let mut multipart = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\ngpt-4o-mini-transcribe\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"binary.wav\"\r\nContent-Type: audio/wav\r\n\r\n"
        )
        .into_bytes();
        multipart.extend_from_slice(&[0x00, 0x80, 0xff, b'A', b'B']);
        multipart.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        let expected_body = multipart.clone();

        let upstream = tokio::spawn(async move {
            let (mut stream, _) = upstream_listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 4096];
            let mut expected_len = None;
            loop {
                let read = stream.read(&mut buffer).await.unwrap();
                assert!(read > 0, "upstream request ended before body completed");
                request.extend_from_slice(&buffer[..read]);
                if expected_len.is_none() {
                    if let Some(header_end) = find_header_end(&request) {
                        let headers = String::from_utf8_lossy(&request[..header_end]);
                        let content_length = header_value_from_headers(&headers, "content-length")
                            .unwrap()
                            .parse::<usize>()
                            .unwrap();
                        expected_len = Some(header_end + 4 + content_length);
                    }
                }
                if expected_len.is_some_and(|length| request.len() >= length) {
                    break;
                }
            }
            let header_end = find_header_end(&request).unwrap();
            let body = request[header_end + 4..].to_vec();
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 13\r\nConnection: close\r\n\r\n{\"text\":\"ok\"}",
                )
                .await
                .unwrap();
            body
        });

        let helper_listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let helper_addr = helper_listener.local_addr().unwrap();
        let helper = tokio::spawn(async move {
            let (stream, remote_addr) = helper_listener.accept().await.unwrap();
            handle_helper_connection(stream, Some(remote_addr))
                .await
                .unwrap();
        });
        let mut client = tokio::net::TcpStream::connect(helper_addr).await.unwrap();
        let headers = format!(
            "POST /v1/audio/transcriptions HTTP/1.1\r\nHost: {helper_addr}\r\nContent-Type: multipart/form-data; boundary={boundary}\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
        );
        for fragment in headers.as_bytes().chunks(7) {
            client.write_all(fragment).await.unwrap();
        }
        for fragment in multipart.chunks(11) {
            let chunk_header = format!("{:X}\r\n", fragment.len());
            client.write_all(chunk_header.as_bytes()).await.unwrap();
            client.write_all(fragment).await.unwrap();
            client.write_all(b"\r\n").await.unwrap();
        }
        client.write_all(b"0\r\n\r\n").await.unwrap();
        let mut response = Vec::new();
        client.read_to_end(&mut response).await.unwrap();
        assert!(String::from_utf8_lossy(&response).starts_with("HTTP/1.1 200 OK"));

        helper.await.unwrap();
        assert_eq!(upstream.await.unwrap(), expected_body);
        crate::paths::set_settings_path_for_tests(previous_settings_path);
    }
}

#[cfg(test)]
mod proxy_stream_tests {
    use super::forward_protocol_proxy_stream;
    use crate::protocol_proxy::UpstreamWireApi;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn forward_fixture(
        body: &str,
        truncated: bool,
        wire: UpstreamWireApi,
    ) -> (Option<String>, String) {
        let (failure, output, _) =
            forward_fixture_with_prefix_and_delay(&[], body, truncated, wire, Duration::ZERO)
                .await;
        (failure, output)
    }

    async fn forward_fixture_with_prefix(
        prefix: &[u8],
        body: &str,
        truncated: bool,
        wire: UpstreamWireApi,
    ) -> (Option<String>, String) {
        let (failure, output, _) =
            forward_fixture_with_prefix_and_delay(prefix, body, truncated, wire, Duration::ZERO)
                .await;
        (failure, output)
    }

    async fn forward_fixture_with_prefix_and_delay(
        prefix: &[u8],
        body: &str,
        truncated: bool,
        wire: UpstreamWireApi,
        initial_delay: Duration,
    ) -> (Option<String>, String, Option<u64>) {
        let upstream = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = upstream.local_addr().unwrap();
        let body = body.to_owned();
        let prefix = prefix.to_vec();
        let upstream_task = tokio::spawn(async move {
            let (mut socket, _) = upstream.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let byte = socket.read_u8().await.unwrap();
                request.push(byte);
                if request.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n").await.unwrap();
            tokio::time::sleep(initial_delay).await;
            socket
                .write_all(format!("{:X}\r\n{}\r\n", body.len(), body).as_bytes())
                .await
                .unwrap();
            if !truncated {
                socket.write_all(b"0\r\n\r\n").await.unwrap();
            }
            socket.shutdown().await.unwrap();
        });
        let response = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .get(format!("http://{address}/stream?key=fixture-secret"))
            .send()
            .await
            .unwrap();
        let downstream = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut reader = tokio::net::TcpStream::connect(downstream.local_addr().unwrap())
            .await
            .unwrap();
        let (mut writer, _) = downstream.accept().await.unwrap();
        let forwarded = tokio::spawn(async move {
            let (result, _usage, _detail, first_output_at_ms) =
                forward_protocol_proxy_stream(&mut writer, &prefix, response, wire, None)
                    .await
                    .unwrap();
            writer.shutdown().await.unwrap();
            (result, first_output_at_ms)
        });
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await.unwrap();
        let (result, first_output_at_ms) = forwarded.await.unwrap();
        upstream_task.await.unwrap();
        (result, String::from_utf8(bytes).unwrap(), first_output_at_ms)
    }

    #[tokio::test]
    async fn first_output_timestamp_is_recorded_after_upstream_sends_data() {
        let started_at_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let (_, _, first_output_at_ms) = forward_fixture_with_prefix_and_delay(
            &[],
            "data: {\"choices\":[{\"delta\":{\"content\":\"hello\"}}]}\n\ndata: [DONE]\n\n",
            false,
            UpstreamWireApi::ChatCompletions,
            Duration::from_millis(80),
        )
        .await;
        let first_output_at_ms = first_output_at_ms.expect("应记录上游首个数据块时间");
        assert!(
            first_output_at_ms >= started_at_ms + 50,
            "首字时间不应在上游发送数据前记录: started={started_at_ms}, first={first_output_at_ms}"
        );
    }

    #[tokio::test]
    async fn transport_errors_are_failures_for_native_and_chat_streams() {
        for wire in [UpstreamWireApi::Responses, UpstreamWireApi::ChatCompletions] {
            let (failure, output) = forward_fixture(
                "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n",
                true,
                wire,
            )
            .await;
            let failure = failure.expect("truncated transfer must not log stream_ok");
            assert!(!failure.contains("fixture-secret"));
            assert!(!output.contains("event: response.completed"));
            if wire == UpstreamWireApi::ChatCompletions {
                assert!(output.contains("event: response.failed"));
            }
        }
    }

    #[tokio::test]
    async fn chat_eof_and_error_events_are_not_completed_or_replayed() {
        for body in [
            "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n",
            "event: error\ndata: {\"error\":{\"message\":\"failed\"}}\n\n",
        ] {
            let (failure, output) =
                forward_fixture(body, false, UpstreamWireApi::ChatCompletions).await;
            assert!(failure.is_some());
            assert!(output.contains("event: response.failed"));
            assert!(!output.contains("event: response.completed"));
        }
    }

    #[tokio::test]
    async fn native_responses_failure_events_are_observed_without_rewriting_or_echoing_payload() {
        for (body, event_type) in [
            (
                "event: response.failed\ndata: {\"type\":\"response.failed\",\"response\":{\"error\":{\"message\":\"private-upstream-error\"},\"output\":[{\"encrypted_content\":\"private-payload\"}]}}\n\n",
                "response.failed",
            ),
            (
                "data: {\"type\":\"response.incomplete\",\"response\":{\"incomplete_details\":{\"reason\":\"private-upstream-error\"}}}\n\n",
                "response.incomplete",
            ),
            (
                "event: error\ndata: {\"error\":{\"message\":\"private-upstream-error\"}}\r\n\r\n",
                "error",
            ),
        ] {
            let (failure, output) = forward_fixture(body, false, UpstreamWireApi::Responses).await;
            let failure = failure.expect("原生失败事件不得记成 stream_ok");
            assert!(failure.contains(event_type));
            assert!(!failure.contains("private-upstream-error"));
            assert!(!failure.contains("private-payload"));
            assert_eq!(output, body, "原生响应必须原字节转发");
        }
    }

    #[tokio::test]
    async fn native_responses_clean_http_eof_without_completed_is_diagnosed_without_replay() {
        let body = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\ndata: [DONE]\n\n";
        let (failure, output) = forward_fixture(body, false, UpstreamWireApi::Responses).await;
        assert!(
            failure
                .expect("HTTP EOF 和 DONE 均不能代替 response.completed")
                .contains("完成事件")
        );
        assert_eq!(output, body);
    }

    #[tokio::test]
    async fn successful_native_passthrough_and_chat_completion_are_preserved() {
        let native = "event: response.completed\ndata: {\"type\":\"response.completed\"}\n\n";
        let (failure, output) = forward_fixture(native, false, UpstreamWireApi::Responses).await;
        assert!(failure.is_none());
        assert_eq!(output, native);
        let (failure, output) = forward_fixture(
            "data: {\"choices\":[{\"delta\":{\"content\":\"done\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n",
            false, UpstreamWireApi::ChatCompletions,
        ).await;
        assert!(failure.is_none());
        assert!(output.contains("event: response.completed"));
        assert!(!output.contains("event: response.failed"));
    }

    /// Anthropic 上游：`message_stop` 是完成事件，缺失时按掐断上报而不是伪装成功。
    #[tokio::test]
    async fn anthropic_stream_is_converted_and_truncation_is_reported() {
        let complete = concat!(
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"model\":\"claude-sonnet-5\"}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"你好\"}}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":2}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"
        );
        let (failure, output) =
            forward_fixture(complete, false, UpstreamWireApi::AnthropicMessages).await;
        assert!(failure.is_none(), "{failure:?}");
        assert!(output.contains("event: response.output_text.delta"));
        assert!(output.contains("你好"));
        assert!(output.contains("event: response.completed"));
        assert!(!output.contains("event: response.failed"));

        // 少了 message_stop：已有增量不能重放，也不能记成正常结束。
        let truncated = complete
            .split("event: message_delta")
            .next()
            .expect("fixture 由多个事件拼成");
        let (failure, output) =
            forward_fixture(truncated, false, UpstreamWireApi::AnthropicMessages).await;
        assert!(
            failure.expect("流中途断开必须报错").contains("完成事件"),
            "错误原因要能指向缺失的完成事件"
        );
        assert!(output.contains("event: response.failed"));

        // 只有 message_stop 缺失、但 stop_reason 已到：与 chat 上游口径一致，
        // finish_reason 本身就是终止信号，按成功处理而不是报「被掐断」。
        let without_stop = complete.replace(
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
            "",
        );
        let (failure, output) =
            forward_fixture(&without_stop, false, UpstreamWireApi::AnthropicMessages).await;
        assert!(failure.is_none(), "{failure:?}");
        assert!(output.contains("event: response.completed"));
    }

    #[tokio::test]
    async fn stream_prefix_is_forwarded_before_remaining_body() {
        // 代理预读过的 response.created 必须原样补写，不能丢；
        // 剩余 response.completed 继续透传。
        let prefix = concat!(
            "event: response.created\n",
            "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\"}}\n\n",
        );
        let remainder = concat!(
            "event: response.completed\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\"}}\n\n",
        );
        let (failure, output) = forward_fixture_with_prefix(
            prefix.as_bytes(),
            remainder,
            false,
            UpstreamWireApi::Responses,
        )
        .await;
        assert!(failure.is_none(), "{failure:?}");
        assert!(output.starts_with(prefix), "前缀必须原样先行转发: {output}");
        assert!(output.contains("response.completed"), "{output}");
        assert_eq!(output.matches("event: response.created").count(), 1);
    }

    #[tokio::test]
    async fn native_oversized_event_is_forwarded_and_stream_completes() {
        use std::time::Duration;
        use tokio::time::{sleep, timeout};

        // 超过改写缓冲上限的事件（高分辨率生图等）必须完整转发，随后的
        // response.completed 照常让流以成功结束；过去在这里截断会让 Codex
        // 收不到终态事件，按断流重试并重复计费。上游发完后保持连接，转发层
        // 在收到完成事件后不必等 EOF 也能正确上报成功。
        let upstream = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = upstream.local_addr().unwrap();
        let upstream_task = tokio::spawn(async move {
            let (mut socket, _) = upstream.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let byte = socket.read_u8().await.unwrap();
                request.push(byte);
                if request.ends_with(b"\r\n\r\n") { break; }
            }
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
            let mut chunks = vec![b"data:{\"type\":\"response.output_item.done\",\"image\":\"".to_vec()];
            // 20 * 256 KiB = 5 MiB，超过 4 MiB 改写缓冲上限
            for _ in 0..20 {
                chunks.push(vec![b'A'; 256 * 1024]);
            }
            chunks.push(b"\"}\n\n".to_vec());
            chunks.push(b"data:{\"type\":\"response.completed\"}\n\n".to_vec());
            for chunk in chunks {
                socket
                    .write_all(format!("{:X}\r\n", chunk.len()).as_bytes())
                    .await
                    .unwrap();
                socket.write_all(&chunk).await.unwrap();
                socket.write_all(b"\r\n").await.unwrap();
            }
            socket.write_all(b"0\r\n\r\n").await.unwrap();
            sleep(Duration::from_secs(60)).await;
        });

        let response = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .get(format!("http://{address}/stream?key=fixture-secret"))
            .send()
            .await
            .unwrap();
        let downstream = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut reader = tokio::net::TcpStream::connect(downstream.local_addr().unwrap())
            .await
            .unwrap();
        let (mut writer, _) = downstream.accept().await.unwrap();
        // 下游必须并发读取：5 MiB 会塞满 TCP 缓冲区，否则转发端写入阻塞。
        let reader_task = tokio::spawn(async move {
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes).await.unwrap();
            bytes
        });

        let forwarded = timeout(
            Duration::from_secs(20),
            forward_protocol_proxy_stream(&mut writer, &[], response, UpstreamWireApi::Responses, None),
        )
        .await
        .expect("上游发完终止块后转发必须结束")
        .unwrap()
        .0;
        writer.shutdown().await.unwrap();
        assert!(forwarded.is_none(), "超大事件后有 response.completed，流应判为成功：{forwarded:?}");

        let forwarded_bytes = reader_task.await.unwrap();
        assert!(forwarded_bytes.len() > 5 * 1024 * 1024, "超大事件必须完整转发，收到 {} 字节", forwarded_bytes.len());
        assert!(forwarded_bytes.ends_with(b"data:{\"type\":\"response.completed\"}\n\n"));
        upstream_task.abort();
    }
}
