//! 「自动接管官方 App」的判据与状态机。
//!
//! 背景：Codex++ 原本只有「管理工具点启动」一条接管入口。用户自己双击 / `open -a`
//! 启动官方 App 时，App 不带 `--remote-debugging-port`，Codex++ 接不上去。
//! 本模块只负责**判断该不该动作**——纯逻辑、无 IO、无平台依赖，便于跨平台单测；
//! 真正的探测（`ps`/`osascript`）与退出在 `launcher` 里，触发源在管理器进程。
//!
//! 设计约束（来自对既有链路的实测，改动前请先读 `docs/plans/2026-10-13-官方App自动接管-design.md`）：
//!
//! 1. **宽限期**：`open` 返回后到页面可注入之间有 2.5~3.4 秒空窗，期间
//!    `cdp::endpoint_available` 为假。没有宽限期就会把「自己刚拉起的实例」判成
//!    「用户裸启动的实例」，进而自杀式重启，形成 quit 风暴。
//! 2. **会话归属**：launcher 被当库链进管理器进程，`launcher_guard_port()` 的锁在
//!    同一进程内自证无效，只能读 launcher 的内存态快照。
//! 3. **降级态**：注入失败会写 `running_degraded` 且不启 watchdog，再点启动也只走
//!    activate 不重注入。这种状态必须显式暴露，不能显示成「已接管」。
//! 4. **多开**：`osascript quit` 按应用名发事件，会把用户正在用的多开实例一起退出，
//!    必须先跳过。
//! 5. **平台**：Windows 走 AUMID 激活，语义完全不同，非 macOS 一律 `Skip`。

use std::path::Path;
use std::time::Duration;

/// 宽限期：距我们上次发起启动不足这么久，不判定「需要接管」。
pub const TAKEOVER_GRACE_MS: u64 = 10_000;
/// 睡眠/唤醒后的 CDP 恢复宽限。
pub const WAKE_GRACE_MS: u64 = 30_000;
/// 活跃探针间隔。
pub const PROBE_INTERVAL_ACTIVE_MS: u64 = 2_000;
/// App 长时间未出现后的探针间隔（降频，避免白烧 CPU）。
pub const PROBE_INTERVAL_IDLE_MS: u64 = 10_000;
/// 进入降频前，「App 未出现」需要持续多久。
pub const IDLE_BEFORE_SLOWDOWN_MS: u64 = 60_000;
/// 失败退避上限。
pub const BACKOFF_CAP_MS: u64 = 30 * 60 * 1000;
/// 滑动窗口长度与窗口内允许的重启次数上限（唯一的防风暴闸门）。
pub const RESTART_WINDOW_MS: u64 = 60 * 60 * 1000;
pub const MAX_RESTARTS_PER_WINDOW: usize = 6;

/// 一个 App 实例的代际键。`lstart` 用来防 PID 复用与换代误伤。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InstanceKey {
    pub pid: u32,
    pub lstart: String,
}

impl InstanceKey {
    pub fn new(pid: u32, lstart: impl Into<String>) -> Self {
        Self {
            pid,
            lstart: lstart.into(),
        }
    }

    /// 落盘用的稳定串（跳过标记按这个比对）。
    pub fn storage_key(&self) -> String {
        format!("{}|{}", self.pid, self.lstart)
    }
}

/// 官方 App 主进程的探测结果。**三态**：探测失败不能当成「没在运行」——
/// `is_macos_app_running` 原先就是把两者压成 false，导致退出等待误判「已退出」。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppProbe {
    Running(InstanceKey),
    NotRunning,
    Unknown,
}

impl Default for AppProbe {
    /// 默认是「未知」而不是「没在运行」：判据在未知时必须保守。
    fn default() -> Self {
        Self::Unknown
    }
}

/// launcher 会话的内存态快照（由 `codex_plus_launcher::session_snapshot` 提供）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionSnapshot {
    /// 本进程当前是否有 launcher 会话。
    pub owned: bool,
    /// 会话是否已 ready（注入流程走完）。
    pub ready: bool,
    /// `latest-status.json` 的 status（`running` / `running_degraded` / …）。
    pub status: Option<String>,
    pub debug_port: Option<u16>,
    pub started_at_ms: Option<u64>,
}

impl SessionSnapshot {
    pub fn is_degraded(&self) -> bool {
        self.owned && self.status.as_deref() == Some("running_degraded")
    }

    pub fn is_running(&self) -> bool {
        self.owned && self.status.as_deref() == Some("running")
    }
}

/// 「疑似有人在用」时的策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivePolicy {
    /// 疑似有人在用就先不动，等下一轮或用户手动触发（默认）。
    Defer,
    /// 不判断，直接按判据接管。
    Immediate,
}

impl ActivePolicy {
    pub fn parse(value: &str) -> Self {
        match value.trim() {
            "immediate" => Self::Immediate,
            _ => Self::Defer,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Defer => "defer",
            Self::Immediate => "immediate",
        }
    }
}

impl Default for ActivePolicy {
    fn default() -> Self {
        Self::Defer
    }
}

/// 除「当前实例」之外，是否还有同一个 App 的主进程在跑（多开实例）。
///
/// `current_pid` 是正在评估的那个主进程（`AppProbe::Running` 里的 pid）：
/// 必须把它排除，否则任何一次裸启动都会被判成「有多开实例」而永远跳过接管。
///
/// 为什么必须跳过：`osascript quit`（macOS）与 `taskkill /T`（Windows）都是按
/// **进程/应用**动作的，会把用户正在用的第二实例一起收掉。多开实例各自带着自己的
/// 调试端口，所以「端口不同」就是最可靠的判据。
#[cfg(any(target_os = "macos", windows, test))]
pub fn has_other_desktop_instance(
    app_dir: &Path,
    exclude_port: u16,
    current_pid: Option<u32>,
) -> bool {
    #[cfg(target_os = "macos")]
    {
        let expected = crate::app_paths::build_codex_executable(app_dir)
            .to_string_lossy()
            .to_string();
        let all = crate::watcher::find_macos_codex_desktop_main_processes(None);
        let excluded = crate::watcher::find_macos_codex_desktop_main_processes(Some(exclude_port));
        let mut excluded = excluded
            .into_iter()
            .map(|process| process.pid)
            .collect::<Vec<_>>();
        if let Some(pid) = current_pid {
            excluded.push(pid);
        }
        return all
            .iter()
            .any(|process| process.executable == expected && !excluded.contains(&process.pid));
    }
    #[cfg(windows)]
    {
        let expected = crate::app_paths::build_codex_executable(app_dir);
        let processes = crate::windows_integration::enumerate_processes();
        // 主实例 = 同名同路径且**不是** Electron 子进程。调试端口相同的那个是
        // 「我们正在管的实例」，也不算多开。
        let debug_port_pids = crate::watcher::find_codex_processes_for_debug_port(exclude_port);
        let main_pids = processes
            .iter()
            .filter(|process| {
                windows_executable_matches(process, &expected)
                    && !windows_is_electron_child(process)
            })
            .map(|process| process.process_id)
            .collect::<Vec<_>>();
        return main_pids.iter().any(|process_id| {
            !debug_port_pids.contains(process_id) && Some(*process_id) != current_pid
        });
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DesktopRuntime {
    pid: u32,
    /// macOS 用 `ps` 的 lstart；Windows 用进程启动时刻（秒）。
    incarnation: u64,
}

/// 一次完整探测的结果。把「进程判据」与「CDP 判据」一起算出来，便于端到端验证。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopProbe {
    pub app: AppProbe,
    pub cdp_ready: bool,
    pub actual_port: u16,
    /// 除 `actual_port` 之外还有同一个 App 的实例在跑（多开）。
    pub other_instance: bool,
}

/// 探测官方 App 的实时状态（只读：枚举进程 + 探测回环端口，无任何副作用）。
pub fn probe_desktop_app_state(app_dir: &Path, requested_port: u16) -> DesktopProbe {
    let actual_port = crate::ports::select_packaged_codex_debug_port(requested_port);
    let app = probe_desktop_app(app_dir);
    let current_pid = match &app {
        AppProbe::Running(key) => Some(key.pid),
        _ => None,
    };
    DesktopProbe {
        app,
        cdp_ready: crate::cdp::endpoint_available(actual_port),
        actual_port,
        other_instance: has_other_desktop_instance(app_dir, actual_port, current_pid),
    }
}

/// 官方 App 主进程的精确探测（macOS / Windows 各自实现）。
///
/// 判据只回答一件事：**同一个 App 目录对应的主进程在不在跑**。
/// 不在这里判断「有没有被接管」——那由 `cdp::endpoint_available` 与
/// `SessionSnapshot` 负责，混在一起会让两种失败无法区分。
pub fn probe_desktop_app(app_dir: &Path) -> AppProbe {
    #[cfg(target_os = "macos")]
    {
        let expected = crate::app_paths::build_codex_executable(app_dir)
            .to_string_lossy()
            .to_string();
        // 不带端口过滤：主进程可能在跑但没带调试端口（正是要接管的场景）。
        let processes = crate::watcher::find_macos_codex_desktop_main_processes(None);
        return match processes
            .into_iter()
            .find(|process| process.executable == expected)
        {
            Some(process) => AppProbe::Running(InstanceKey::new(
                process.pid,
                process.lstart.clone(),
            )),
            None => AppProbe::NotRunning,
        };
    }
    #[cfg(windows)]
    {
        return probe_windows_desktop_app(app_dir);
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = app_dir;
        AppProbe::Unknown
    }
}

#[cfg(windows)]
/// 进程是否「与官方 App 主进程同名同路径」。Windows 上 renderer / gpu / utility /
/// crashpad 全都叫 `ChatGPT.exe` 且在同一目录，只能靠命令行区分。
fn windows_executable_matches(
    process: &crate::windows_integration::WindowsProcessInfo,
    expected: &Path,
) -> bool {
    let Some(path) = process.executable_path.as_deref() else {
        return false;
    };
    if path == expected {
        return true;
    }
    let same_name = path
        .file_name()
        .zip(expected.file_name())
        .is_some_and(|(left, right)| left.eq_ignore_ascii_case(right));
    let same_dir = path
        .parent()
        .zip(expected.parent())
        .is_some_and(|(left, right)| {
            left.to_string_lossy()
                .eq_ignore_ascii_case(&right.to_string_lossy())
        });
    same_name && same_dir
}

/// 是否是 Electron 的**子进程**（renderer / gpu-process / utility / crashpad-handler …）。
///
/// 判据必须是「命令行里有 `--type=`」，不能只看父进程：本机 Windows 11 实测
/// crashpad 会再 fork 一个 crashpad，父进程同样叫 `ChatGPT.exe`，
/// 于是「父进程同名就当成子进程」会漏掉它，把子进程误判成主进程。
#[cfg(windows)]
fn windows_is_electron_child(
    process: &crate::windows_integration::WindowsProcessInfo,
) -> bool {
    process.exe_file.to_ascii_lowercase() == "chatgpt.exe"
        && crate::windows_integration::process_command_line(process.process_id)
            .is_some_and(|command_line| command_line.contains("--type="))
}

#[cfg(windows)]
fn probe_windows_desktop_app(app_dir: &Path) -> AppProbe {
    let expected = crate::app_paths::build_codex_executable(app_dir);
    let self_pid = std::process::id();
    for process in crate::windows_integration::enumerate_processes() {
        if process.process_id == self_pid {
            continue;
        }
        if !windows_executable_matches(&process, &expected) {
            continue;
        }
        if windows_is_electron_child(&process) {
            continue;
        }
        // 实例代际：用进程启动时刻（秒）。Windows 上也用于「本次不接管」的比对。
        let incarnation = match crate::watcher::inspect_process_instance(process.process_id) {
            crate::watcher::ProcessInstanceState::Running {
                started_at_secs, ..
            } => started_at_secs.unwrap_or_default(),
            _ => 0,
        };
        return AppProbe::Running(InstanceKey::new(
            process.process_id,
            incarnation.to_string(),
        ));
    }
    AppProbe::NotRunning
}

/// 要求官方 App 退出，返回是否真的退出了。
///
/// macOS 走 AppleEvent `quit`（能触发完整的退出流程：未保存内容询问、before-quit），
/// Windows 走 `taskkill`（先不带 `/F`，让窗口收到关闭请求；超时再强制结束）。
/// 两边的「不强制」窗口都由调用方给的预算控制。
#[allow(unused_variables)]
pub fn request_desktop_exit(app_dir: &Path, instance: &InstanceKey, graceful: bool) -> bool {
    #[cfg(target_os = "macos")]
    {
        return request_macos_exit(app_dir, instance, graceful);
    }
    #[cfg(windows)]
    {
        return request_windows_exit(instance, graceful);
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        false
    }
}

#[cfg(target_os = "macos")]
fn request_macos_exit(app_dir: &Path, _instance: &InstanceKey, _graceful: bool) -> bool {
    let app_name = app_dir
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("ChatGPT")
        .replace('"', "\\\"");
    let script = format!(r#"tell application "{app_name}" to quit"#);
    match std::process::Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
    {
        Ok(output) => {
            if !output.status.success() {
                // 既有实现把 osascript 的退出码丢掉了：自动化授权被拒时表现为
                // 「App 没退出」而不是「授权失败」，用户完全无从下手。
                let reason = String::from_utf8_lossy(&output.stderr);
                let reason = reason.trim();
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "takeover.exit_request_failed",
                    serde_json::json!({
                        "app_dir": app_dir.to_string_lossy(),
                        "message": if reason.is_empty() { "osascript 退出码非 0" } else { reason },
                    }),
                );
                return false;
            }
            true
        }
        Err(error) => {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "takeover.exit_request_failed",
                serde_json::json!({ "message": error.to_string() }),
            );
            false
        }
    }
}

#[cfg(windows)]
fn request_windows_exit(instance: &InstanceKey, graceful: bool) -> bool {
    // 优雅路径只发 `WM_CLOSE`：Windows 11 实测 `taskkill /T`（不带 /F）对 GUI 应用
    // 完全不生效——11 个 ChatGPT 进程一个都没退，静默失败比报错更糟。
    // 强制路径才用 `taskkill /F /T`（TerminateProcess 语义，会丢未落盘状态）。
    let accepted = crate::windows_integration::close_process_tree(instance.pid, !graceful);
    let _ = crate::diagnostic_log::append_diagnostic_log(
        if accepted {
            "takeover.exit_request"
        } else {
            "takeover.exit_request_failed"
        },
        serde_json::json!({
            "pid": instance.pid,
            "force": !graceful,
            "accepted": accepted,
        }),
    );
    accepted
}

#[derive(Debug, Clone, Default)]
pub struct TakeoverInputs {
    pub enabled: bool,
    /// 仅 macOS 为 true；其它平台恒 `Skip`。
    pub platform_supported: bool,
    pub session: SessionSnapshot,
    pub app: AppProbe,
    /// `cdp::endpoint_available(actual_port)`——严格判据。
    pub cdp_ready: bool,
    /// 实际调试端口（不是硬编码的 9229：端口被占时会自动换位）。
    pub actual_port: u16,
    /// 存在 Codex++ 多开 worker 实例。
    pub multi_instance_running: bool,
    /// 管理器进程内有启动/重启在途（`LAUNCH_IN_PROGRESS`）。
    pub takeover_in_progress: bool,
    /// 距我们上次发起启动的毫秒数（必须在调用 `start()` **之前**打点）。
    pub ms_since_our_launch: Option<u64>,
    /// 距上次接管尝试的毫秒数。
    pub ms_since_last_attempt: Option<u64>,
    /// 距上次「睡眠唤醒」的毫秒数。
    pub ms_since_wake: Option<u64>,
    /// 连续失败次数（决定退避档位）。
    pub consecutive_failures: u32,
    /// 当前是否疑似有人在用（本地只读信号）。
    pub user_active: bool,
    pub active_policy: ActivePolicy,
    /// 用户对某个实例点了「本次不接管」。
    pub paused_instance: Option<InstanceKey>,
    /// 滑动窗口内的重启时间戳（毫秒，越早越前）。
    pub restart_history_ms: Vec<u64>,
    /// 当前时间戳（毫秒），用于滑动窗口计算。
    pub now_ms: u64,
}

impl TakeoverInputs {
    /// 探针间隔：App 长时间没出现就降频。
    pub fn probe_interval(&self) -> Duration {
        match self.app {
            AppProbe::Running(_) => Duration::from_millis(PROBE_INTERVAL_ACTIVE_MS),
            _ => match self.ms_since_last_attempt {
                Some(elapsed) if elapsed < IDLE_BEFORE_SLOWDOWN_MS => {
                    Duration::from_millis(PROBE_INTERVAL_ACTIVE_MS)
                }
                _ => Duration::from_millis(PROBE_INTERVAL_IDLE_MS),
            },
        }
    }

    /// 退避是否未到期。
    fn backoff_pending(&self) -> bool {
        if self.consecutive_failures == 0 {
            return false;
        }
        match self.ms_since_last_attempt {
            Some(elapsed) => elapsed < backoff_delay_ms(self.consecutive_failures),
            None => false,
        }
    }

    fn restart_budget_left(&self) -> bool {
        restarts_in_window(&self.restart_history_ms, self.now_ms) < MAX_RESTARTS_PER_WINDOW
    }

    /// 手动「立即接管」是否还有额度。手动触发可以绕过「疑似有人在用」的等待，
    /// 但**不能绕过**防风暴闸门——那是防无限重启的最后一道。
    pub fn restart_budget_left_for_manual(&self) -> bool {
        self.restart_budget_left()
    }
}

/// 决策结果。`reason` 用稳定英文串，进诊断日志与状态行。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TakeoverDecision {
    Wait(&'static str),
    Skip(&'static str),
    Degraded(&'static str),
    Takeover,
}

impl TakeoverDecision {
    pub fn reason(&self) -> &'static str {
        match self {
            Self::Wait(reason) | Self::Skip(reason) | Self::Degraded(reason) => reason,
            Self::Takeover => "takeover",
        }
    }

    pub fn is_takeover(&self) -> bool {
        matches!(self, Self::Takeover)
    }
}

/// 退避阶梯：30s → 60s → 2min → … → 30min（指数，封顶）。
pub fn backoff_delay_ms(consecutive_failures: u32) -> u64 {
    if consecutive_failures == 0 {
        return 0;
    }
    let factor = 1_u64
        .checked_shl((consecutive_failures - 1).min(16))
        .unwrap_or(u64::MAX);
    (30_000_u64.saturating_mul(factor)).min(BACKOFF_CAP_MS)
}

/// 滑动一小时窗口内的重启次数。
pub fn restarts_in_window(history_ms: &[u64], now_ms: u64) -> usize {
    history_ms
        .iter()
        .filter(|timestamp| now_ms.saturating_sub(**timestamp) < RESTART_WINDOW_MS)
        .count()
}

/// 追加一条重启记录并丢弃过期项。
pub fn push_restart(history: &mut Vec<u64>, now_ms: u64) {
    history.retain(|timestamp| now_ms.saturating_sub(*timestamp) < RESTART_WINDOW_MS);
    history.push(now_ms);
}

/// 核心判据。顺序即优先级，注释里写了每条为什么必须在前面。
pub fn decide(inputs: &TakeoverInputs) -> TakeoverDecision {
    if !inputs.enabled {
        return TakeoverDecision::Skip("disabled");
    }
    // Windows 走 AUMID 激活，且 find_codex_processes_for_debug_port 在非 macOS
    // 会退化成不带端口过滤的实现，误伤面太大，直接门禁。
    if !inputs.platform_supported {
        return TakeoverDecision::Skip("platform_unsupported");
    }
    // 启动/重启在途（用户点了启动或本模块刚发起）：不叠加第二次动作。
    if inputs.takeover_in_progress {
        return TakeoverDecision::Wait("launch_in_progress");
    }
    // 注入降级：端口可能在、页面却没注入成功，且没有 watchdog 自愈。
    // 必须在「已有端口就当已接管」之前判掉，否则会谎报「已接管」。
    if inputs.session.is_degraded() {
        return TakeoverDecision::Degraded("injection_ready_failed");
    }
    // 多开实例在跑：osascript quit 按应用名走，会连带退出用户正在用的第二实例。
    if inputs.multi_instance_running {
        return TakeoverDecision::Skip("multi_instance");
    }
    // 探测失败 ≠ 没在运行。宁可不动，也不要基于未知状态重启。
    if matches!(inputs.app, AppProbe::Unknown) {
        return TakeoverDecision::Skip("probe_unknown");
    }
    if let (Some(paused), AppProbe::Running(current)) = (&inputs.paused_instance, &inputs.app)
        && paused == current
    {
        return TakeoverDecision::Wait("paused_for_instance");
    }
    if matches!(inputs.app, AppProbe::NotRunning) {
        return TakeoverDecision::Wait("app_not_running");
    }
    // 已接管：会话 running 才算，光有端口不算（降级态已在前面拦掉）。
    if inputs.cdp_ready && inputs.session.is_running() {
        return TakeoverDecision::Wait("already_attached");
    }
    // 唤醒后的 CDP 恢复宽限：睡眠/唤醒后端口常短暂不可连，等它自己恢复。
    if matches!(inputs.ms_since_wake, Some(elapsed) if elapsed < WAKE_GRACE_MS) {
        return TakeoverDecision::Wait("wake_grace");
    }
    // 宽限期：我们自己刚拉起 App，页面还没就绪属正常，不能据此重启。
    if matches!(inputs.ms_since_our_launch, Some(elapsed) if elapsed < TAKEOVER_GRACE_MS) {
        return TakeoverDecision::Wait("grace");
    }
    // 防风暴闸门：滑动一小时窗口内重启次数用尽。
    if !inputs.restart_budget_left() {
        return TakeoverDecision::Degraded("restart_budget_exhausted");
    }
    if inputs.backoff_pending() {
        return TakeoverDecision::Wait("backoff");
    }
    // 疑似有人在用：默认策略是等（用户可手动「立即接管」）。
    if inputs.user_active && inputs.active_policy == ActivePolicy::Defer {
        return TakeoverDecision::Wait("user_active");
    }
    TakeoverDecision::Takeover
}

#[cfg(test)]
mod tests {
    use super::*;

    fn running_instance() -> AppProbe {
        AppProbe::Running(InstanceKey::new(4242, "Tue Oct 13 13:19:56 2026"))
    }

    /// 基线：官方 App 裸启动（无调试端口），没有任何宽限/退避 → 应当接管。
    fn base_inputs() -> TakeoverInputs {
        TakeoverInputs {
            enabled: true,
            platform_supported: true,
            session: SessionSnapshot::default(),
            app: running_instance(),
            cdp_ready: false,
            actual_port: 9229,
            multi_instance_running: false,
            takeover_in_progress: false,
            ms_since_our_launch: None,
            ms_since_last_attempt: Some(600_000),
            ms_since_wake: Some(600_000),
            consecutive_failures: 0,
            user_active: false,
            active_policy: ActivePolicy::Defer,
            paused_instance: None,
            restart_history_ms: Vec::new(),
            now_ms: 1_000_000,
        }
    }

    #[test]
    fn bare_launch_is_taken_over() {
        assert!(decide(&base_inputs()).is_takeover());
    }

    #[test]
    fn disabled_or_unsupported_platform_skips() {
        let mut inputs = base_inputs();
        inputs.enabled = false;
        assert_eq!(decide(&inputs), TakeoverDecision::Skip("disabled"));

        let mut inputs = base_inputs();
        inputs.platform_supported = false;
        assert_eq!(
            decide(&inputs),
            TakeoverDecision::Skip("platform_unsupported")
        );
    }

    /// 最关键的一条：我们自己刚拉起 App 之后的那段空窗里，绝不能再次重启。
    #[test]
    fn grace_period_blocks_self_inflicted_restart() {
        let mut inputs = base_inputs();
        inputs.ms_since_our_launch = Some(TAKEOVER_GRACE_MS - 1);
        assert_eq!(decide(&inputs), TakeoverDecision::Wait("grace"));

        inputs.ms_since_our_launch = Some(TAKEOVER_GRACE_MS);
        assert!(decide(&inputs).is_takeover());
    }

    #[test]
    fn already_attached_session_waits() {
        let mut inputs = base_inputs();
        inputs.cdp_ready = true;
        inputs.session = SessionSnapshot {
            owned: true,
            ready: true,
            status: Some("running".to_string()),
            debug_port: Some(9229),
            started_at_ms: Some(1),
        };
        assert_eq!(decide(&inputs), TakeoverDecision::Wait("already_attached"));
    }

    /// 端口在、会话却降级：不能显示「已接管」，也不能重启 App。
    #[test]
    fn degraded_session_is_reported_not_restarted() {
        let mut inputs = base_inputs();
        inputs.cdp_ready = true;
        inputs.session = SessionSnapshot {
            owned: true,
            ready: true,
            status: Some("running_degraded".to_string()),
            debug_port: Some(9229),
            started_at_ms: Some(1),
        };
        assert_eq!(
            decide(&inputs),
            TakeoverDecision::Degraded("injection_ready_failed")
        );
    }

    #[test]
    fn multi_instance_is_skipped() {
        let mut inputs = base_inputs();
        inputs.multi_instance_running = true;
        assert_eq!(decide(&inputs), TakeoverDecision::Skip("multi_instance"));
    }

    #[test]
    fn unknown_probe_never_restarts() {
        let mut inputs = base_inputs();
        inputs.app = AppProbe::Unknown;
        assert_eq!(decide(&inputs), TakeoverDecision::Skip("probe_unknown"));
    }

    #[test]
    fn not_running_waits() {
        let mut inputs = base_inputs();
        inputs.app = AppProbe::NotRunning;
        assert_eq!(decide(&inputs), TakeoverDecision::Wait("app_not_running"));
    }

    #[test]
    fn paused_instance_waits_until_instance_changes() {
        let mut inputs = base_inputs();
        let current = match &inputs.app {
            AppProbe::Running(key) => key.clone(),
            _ => unreachable!(),
        };
        inputs.paused_instance = Some(current);
        assert_eq!(
            decide(&inputs),
            TakeoverDecision::Wait("paused_for_instance")
        );

        // 用户重启了 App（新的 lstart）→ 恢复尝试。
        inputs.app = AppProbe::Running(InstanceKey::new(4242, "Tue Oct 13 14:00:00 2026"));
        assert!(decide(&inputs).is_takeover());
    }

    #[test]
    fn failures_trigger_backoff_then_recovery() {
        let mut inputs = base_inputs();
        inputs.consecutive_failures = 1;
        inputs.ms_since_last_attempt = Some(1_000);
        assert_eq!(decide(&inputs), TakeoverDecision::Wait("backoff"));

        inputs.ms_since_last_attempt = Some(backoff_delay_ms(1));
        assert!(decide(&inputs).is_takeover());
    }

    #[test]
    fn backoff_ladder_is_exponential_and_capped() {
        assert_eq!(backoff_delay_ms(1), 30_000);
        assert_eq!(backoff_delay_ms(2), 60_000);
        assert_eq!(backoff_delay_ms(3), 120_000);
        assert_eq!(backoff_delay_ms(20), BACKOFF_CAP_MS);
    }

    #[test]
    fn restart_budget_is_a_hard_gate() {
        let mut inputs = base_inputs();
        for _ in 0..MAX_RESTARTS_PER_WINDOW {
            let now = inputs.now_ms;
            push_restart(&mut inputs.restart_history_ms, now);
        }
        assert_eq!(
            decide(&inputs),
            TakeoverDecision::Degraded("restart_budget_exhausted")
        );

        // 窗口外的旧记录不该占额度。
        inputs.restart_history_ms = vec![0];
        assert!(decide(&inputs).is_takeover());
    }

    #[test]
    fn restart_history_drops_expired_entries() {
        // 窗口是 1 小时：now=3600010 时，0 已过期，1000 刚好还在窗口内。
        let mut history = vec![0, 1_000];
        push_restart(&mut history, RESTART_WINDOW_MS + 10);
        assert_eq!(history, vec![1_000, RESTART_WINDOW_MS + 10]);
    }

    #[test]
    fn wake_grace_blocks_restart() {
        let mut inputs = base_inputs();
        inputs.ms_since_wake = Some(WAKE_GRACE_MS - 1);
        assert_eq!(decide(&inputs), TakeoverDecision::Wait("wake_grace"));

        inputs.ms_since_wake = Some(WAKE_GRACE_MS);
        assert!(decide(&inputs).is_takeover());
    }

    #[test]
    fn launch_in_progress_waits() {
        let mut inputs = base_inputs();
        inputs.takeover_in_progress = true;
        assert_eq!(
            decide(&inputs),
            TakeoverDecision::Wait("launch_in_progress")
        );
    }

    #[test]
    fn user_activity_defers_by_default_but_immediate_policy_overrides() {
        let mut inputs = base_inputs();
        inputs.user_active = true;
        assert_eq!(decide(&inputs), TakeoverDecision::Wait("user_active"));

        inputs.active_policy = ActivePolicy::Immediate;
        assert!(decide(&inputs).is_takeover());
    }

    /// 端口自动换位（9229 被占）后，判据必须跟着用实际端口——
    /// 拿错端口会判成「没有 CDP」而反复重启。
    #[test]
    fn actual_port_is_used_for_attachment_check() {
        let mut inputs = base_inputs();
        inputs.actual_port = 9231;
        inputs.cdp_ready = true;
        inputs.session = SessionSnapshot {
            owned: true,
            ready: true,
            status: Some("running".to_string()),
            debug_port: Some(9231),
            started_at_ms: Some(1),
        };
        assert_eq!(decide(&inputs), TakeoverDecision::Wait("already_attached"));
    }

    #[test]
    fn probe_interval_slows_down_when_app_absent() {
        let mut inputs = base_inputs();
        assert_eq!(
            inputs.probe_interval(),
            Duration::from_millis(PROBE_INTERVAL_ACTIVE_MS)
        );

        inputs.app = AppProbe::NotRunning;
        inputs.ms_since_last_attempt = Some(IDLE_BEFORE_SLOWDOWN_MS + 1);
        assert_eq!(
            inputs.probe_interval(),
            Duration::from_millis(PROBE_INTERVAL_IDLE_MS)
        );
    }

    #[test]
    fn active_policy_parsing_defaults_to_defer() {
        assert_eq!(ActivePolicy::parse(""), ActivePolicy::Defer);
        assert_eq!(ActivePolicy::parse("nonsense"), ActivePolicy::Defer);
        assert_eq!(ActivePolicy::parse("defer"), ActivePolicy::Defer);
        assert_eq!(ActivePolicy::parse("immediate"), ActivePolicy::Immediate);
        assert_eq!(ActivePolicy::Immediate.as_str(), "immediate");
    }
}
