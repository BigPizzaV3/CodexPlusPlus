//! Claude Desktop 自动接管：守护线程与状态快照。
//!
//! 启动模式是**完整功能模式**时，用户自己从 Dock / 开始菜单打开的官方 Claude 不经本地服务，
//! 汉化、听写、页面扩展都没有。本模块起一个探针线程，发现「官方 Claude 在跑 + 没带我们的
//! `--proxy-server`」就先请它退出，等它完全退干净，再走 `claude_local_backend::perform("launch")`
//! 经本地服务重开。3p 模式、补丁副本不接管（判据见 `codex_plus_core::claude_auto_takeover`）。
//!
//! 结构照搬 Codex 的 `auto_takeover`：**采集输入 → 调判据 → 执行动作 → 维护状态**，
//! 宽限期、每小时额度、失败退避、「本次不接管」都复用 core 里同一套判据。和 Codex 不同的几处：
//! - 退出前先体检（证书、Node、上游）：完整功能模式起不来就不动用户的 Claude，只报降级。
//! - 退了却没起来时把官方 Claude 原样开回去，并跳过这个新实例，免得反复退出。
//! - 「疑似有人在用」看实例年龄（开了很久才被看到的不自动退），没有会话文件可看。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use codex_plus_core::auto_takeover::{
    ActivePolicy, AppProbe, InstanceKey, TakeoverDecision, TakeoverInputs, backoff_delay_ms, push_restart,
    restarts_in_window,
};
use codex_plus_core::claude_auto_takeover::{
    self as core, ClaudeProbe, ClaudeTakeoverInputs, ExitStage, LaunchMode, PLATFORM_SUPPORTED,
};
use codex_plus_core::diagnostic_log::append_diagnostic_log;
use codex_plus_core::settings::SettingsStore;
use serde::Serialize;
use serde_json::json;

/// 线程 tick 上限。
const TICK_CAP: Duration = Duration::from_millis(2_000);
/// 管理器启动后的静默期：让 `resume_attached_proxy` 先把还挂着的完整功能模式恢复好。
const STARTUP_QUIET: Duration = Duration::from_secs(10);
/// 请 Claude 退出后等它自己退的预算。Windows 上关窗口常常只是缩到托盘，等久了也没用。
const EXIT_GRACE: Duration = if cfg!(windows) { Duration::from_secs(5) } else { Duration::from_secs(15) };
/// 升级到 SIGTERM / 强制结束后的等待。
const EXIT_FORCE_GRACE: Duration = Duration::from_secs(5);
/// 主进程退了以后，等子进程（还在回写配置）也走完。
const CHILDREN_GRACE: Duration = Duration::from_secs(10);
const EXIT_POLL: Duration = Duration::from_millis(150);

/// 给前端的只读状态（camelCase，与 App 里的 ClaudeAutoTakeoverResult 对齐）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeAutoTakeoverPayload {
    /// disabled | probing | taking_over | degraded | paused
    pub state: &'static str,
    pub enabled: bool,
    pub platform_supported: bool,
    /// full | 3p | copy：不是 full 时不接管。
    pub mode: &'static str,
    pub last_takeover_at_ms: Option<u64>,
    pub last_reason: Option<String>,
    pub last_error: Option<String>,
    pub app_running: Option<bool>,
    pub attached: bool,
    pub copy_running: bool,
    pub restarts_last_hour: u32,
    pub restarts_total: u32,
    pub paused_instance: Option<String>,
    pub running: bool,
}

#[derive(Default)]
struct Inner {
    state: &'static str,
    last_takeover_at_ms: Option<u64>,
    last_reason: Option<String>,
    last_error: Option<String>,
    app_running: Option<bool>,
    attached: bool,
    copy_running: bool,
    restart_history_ms: Vec<u64>,
    restarts_total: u32,
    paused_instance: Option<InstanceKey>,
    /// 接管失败、已把官方 Claude 开回去：下一次看到的官方实例直接记成「本次不接管」。
    skip_next_instance: bool,
    /// 第一次看到当前官方实例时它是不是刚打开的：早就开着的（打开开关时、管理器启动时）
    /// 可能正在用，不自动退出。按第一次看到时算，体检失败后补好证书再重试时不会因为「开久了」被拦住。
    first_seen: Option<(InstanceKey, bool)>,
    last_attempt: Option<Instant>,
    consecutive_failures: u32,
    /// 我们上次启动**完成**的时刻（宽限期起点）。
    our_launch_at: Option<Instant>,
    last_wake_at: Option<Instant>,
    last_tick: Option<Instant>,
    last_tick_wall: Option<std::time::SystemTime>,
    started_at: Option<Instant>,
}

static INNER: OnceLock<Mutex<Inner>> = OnceLock::new();
static STOP: AtomicBool = AtomicBool::new(true);
static RUNNING: AtomicBool = AtomicBool::new(false);
/// 接管线程在途（体检、退出、启动整段）。
static IN_PROGRESS: AtomicBool = AtomicBool::new(false);

fn inner() -> &'static Mutex<Inner> {
    INNER.get_or_init(|| {
        Mutex::new(Inner {
            state: "disabled",
            ..Inner::default()
        })
    })
}

fn with_inner<T>(f: impl FnOnce(&mut Inner) -> T) -> T {
    let mut guard = inner().lock().unwrap_or_else(|error| error.into_inner());
    f(&mut guard)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn log_event(event: &str, detail: serde_json::Value) {
    let _ = append_diagnostic_log(event, detail);
}

/// 设置里的开关和当前启动模式。
fn settings_view() -> (bool, LaunchMode) {
    match SettingsStore::default().load() {
        Ok(settings) => (settings.claude_app_auto_takeover, LaunchMode::from_settings(&settings)),
        Err(_) => (false, LaunchMode::ThreeP),
    }
}

pub fn is_enabled() -> bool {
    settings_view().0
}

/// 启动守护线程（幂等）。设置里没开就什么都不做。
pub fn start_supervisor() {
    if !is_enabled() {
        with_inner(|state| state.state = "disabled");
        return;
    }
    // 先撤销停止请求：关了又马上开时让还在睡的旧线程接着跑（同 Codex 的守护线程）。
    STOP.store(false, Ordering::Release);
    if RUNNING.swap(true, Ordering::AcqRel) {
        return;
    }
    let paused = load_paused_instance();
    with_inner(|state| {
        state.started_at = Some(Instant::now());
        state.state = "probing";
        if state.paused_instance.is_none() {
            state.paused_instance = paused;
        }
    });
    let spawned = std::thread::Builder::new()
        .name("codex-plus-claude-auto-takeover".into())
        .spawn(supervisor_loop);
    match spawned {
        Ok(_) => log_event("claude_takeover.supervisor_started", json!({})),
        Err(error) => {
            RUNNING.store(false, Ordering::Release);
            with_inner(|state| {
                state.state = "degraded";
                state.last_error = Some(format!("守护线程启动失败：{error}"));
            });
            log_event(
                "claude_takeover.supervisor_start_failed",
                json!({ "message": error.to_string() }),
            );
        }
    }
}

/// 请求停止守护线程（关掉开关或程序退出），线程在下一次 tick 自行退出。
pub fn stop_supervisor() {
    STOP.store(true, Ordering::Release);
    with_inner(|state| {
        state.state = "disabled";
        state.last_reason = Some("stopped".to_string());
    });
}

pub fn apply_enabled(enabled: bool) {
    if enabled {
        start_supervisor();
    } else {
        stop_supervisor();
    }
}

/// 打开 / 关闭开关：写进 settings.json 后立刻生效。
pub fn set_enabled(enabled: bool) -> Result<(), String> {
    SettingsStore::default()
        .update(json!({ "claudeAppAutoTakeover": enabled }))
        .map_err(|error| format!("保存设置失败：{error}"))?;
    apply_enabled(enabled);
    Ok(())
}

/// 「本次不接管」：只跳过当前这个官方 Claude 实例，用户重开 Claude 后恢复。
pub fn pause_until_next_instance() -> Result<(), String> {
    let (_, probe) = core::probe_now();
    let key = match probe.app {
        AppProbe::Running(key) if !probe.attached => key,
        _ => return Err("当前没有检测到你自己打开的官方 Claude。".to_string()),
    };
    with_inner(|state| {
        state.paused_instance = Some(key.clone());
        state.last_reason = Some("paused_for_instance".to_string());
        state.state = "paused";
    });
    persist_paused_instance(&key);
    log_event(
        "claude_takeover.paused_for_instance",
        json!({ "pid": key.pid, "lstart": key.lstart }),
    );
    Ok(())
}

/// 「立即接管」：不看「疑似在用」「本次不接管」和退避，但启动模式、副本、多开、额度照样拦。
pub fn take_over_now() -> Result<(), String> {
    if !PLATFORM_SUPPORTED {
        return Err("当前系统不支持 Claude 自动接管。".to_string());
    }
    let inputs = collect_inputs(true);
    match inputs.mode {
        LaunchMode::Full => {}
        LaunchMode::ThreeP => return Err("Claude 的启动模式是 3p 模式，只在完整功能模式下接管。".to_string()),
        LaunchMode::Copy => return Err("Claude 的启动模式是补丁副本，只在完整功能模式下接管。".to_string()),
    }
    if inputs.copy_running {
        return Err("补丁副本正在运行，先退出它再接管。".to_string());
    }
    if inputs.base.takeover_in_progress {
        return Err("已有接管正在进行，请稍候。".to_string());
    }
    if inputs.base.multi_instance_running {
        return Err("检测到别的 Claude 实例在运行，已跳过接管。".to_string());
    }
    if inputs.base.cdp_ready && inputs.base.session.is_running() {
        return Err("Claude 已经经完整功能模式运行了。".to_string());
    }
    let instance = match &inputs.base.app {
        AppProbe::Running(key) => key.clone(),
        AppProbe::NotRunning => return Err("没有检测到官方 Claude 正在运行。".to_string()),
        AppProbe::Unknown => return Err("无法确认 Claude 的状态，已保守跳过。".to_string()),
    };
    if !inputs.base.restart_budget_left_for_manual() {
        return Err("一小时内接管次数已达上限，已停止自动重试。".to_string());
    }
    spawn_takeover(instance)
}

pub fn payload() -> ClaudeAutoTakeoverPayload {
    let (enabled, mode) = settings_view();
    with_inner(|state| ClaudeAutoTakeoverPayload {
        state: state.state,
        enabled,
        platform_supported: PLATFORM_SUPPORTED,
        mode: mode.as_str(),
        last_takeover_at_ms: state.last_takeover_at_ms,
        last_reason: state.last_reason.clone(),
        last_error: state.last_error.clone(),
        app_running: state.app_running,
        attached: state.attached,
        copy_running: state.copy_running,
        restarts_last_hour: restarts_in_window(&state.restart_history_ms, now_ms()) as u32,
        restarts_total: state.restarts_total,
        paused_instance: state.paused_instance.as_ref().map(InstanceKey::storage_key),
        running: RUNNING.load(Ordering::Acquire),
    })
}

// ---------------------------------------------------------------- 探针线程

fn supervisor_loop() {
    while !STOP.load(Ordering::Acquire) {
        let tick_started = Instant::now();
        let interval = tick();
        let sleep_for = interval.saturating_sub(tick_started.elapsed()).min(TICK_CAP);
        std::thread::sleep(sleep_for);
    }
    with_inner(|state| state.state = "disabled");
    RUNNING.store(false, Ordering::Release);
    log_event("claude_takeover.supervisor_stopped", json!({}));
    // 退出循环后、清 RUNNING 前又被打开：那次 start 以为线程还在而直接返回了，这里补起。
    if !STOP.load(Ordering::Acquire) {
        start_supervisor();
    }
}

fn tick() -> Duration {
    let now = Instant::now();
    let now_wall = std::time::SystemTime::now();
    let woke = with_inner(|state| {
        let mono_gap = state.last_tick.map(|last| now.duration_since(last));
        let wall_gap = state.last_tick_wall.map(|last| now_wall.duration_since(last));
        let woke = codex_plus_core::auto_takeover::is_wake_gap(mono_gap, wall_gap, Duration::from_secs(90));
        state.last_tick = Some(now);
        state.last_tick_wall = Some(now_wall);
        if woke {
            state.last_wake_at = Some(now);
        }
        woke
    });
    if woke {
        log_event("claude_takeover.wake_detected", json!({}));
    }
    let active = Duration::from_millis(codex_plus_core::auto_takeover::PROBE_INTERVAL_ACTIVE_MS);
    if IN_PROGRESS.load(Ordering::Acquire) {
        return active;
    }
    let starting_up = with_inner(|state| state.started_at.is_some_and(|started| started.elapsed() < STARTUP_QUIET));
    if starting_up {
        return active;
    }
    let inputs = collect_inputs(false);
    let decision = core::decide(&inputs);
    publish(&inputs, decision);
    if decision.is_takeover()
        && let AppProbe::Running(instance) = &inputs.base.app
    {
        let _ = spawn_takeover(instance.clone());
    }
    if inputs.mode == LaunchMode::Full {
        inputs.base.probe_interval()
    } else {
        Duration::from_millis(codex_plus_core::auto_takeover::PROBE_INTERVAL_IDLE_MS)
    }
}

/// 把结果写进状态，决策变化时记一条日志（避免 2 秒一条刷屏）。
fn publish(inputs: &ClaudeTakeoverInputs, decision: TakeoverDecision) {
    let reason = decision.reason();
    let changed = with_inner(|state| {
        state.app_running = match inputs.base.app {
            AppProbe::Running(_) => Some(true),
            AppProbe::NotRunning => Some(false),
            AppProbe::Unknown => None,
        };
        state.attached = inputs.base.cdp_ready;
        state.copy_running = inputs.copy_running;
        state.state = match decision {
            TakeoverDecision::Takeover => "taking_over",
            TakeoverDecision::Degraded(_) => "degraded",
            TakeoverDecision::Skip("disabled") => "disabled",
            _ if inputs.base.paused_instance.is_some() && state.state == "paused" => "paused",
            // 体检 / 启动失败的降级要一直显示，直到下一次成功或实例换代。
            _ if state.state == "degraded"
                && state.consecutive_failures > 0
                && matches!(inputs.base.app, AppProbe::Running(_)) =>
            {
                "degraded"
            }
            _ => "probing",
        };
        if matches!(decision, TakeoverDecision::Degraded(_)) {
            state.last_error = Some(reason.to_string());
        }
        let changed = state.last_reason.as_deref() != Some(reason);
        state.last_reason = Some(reason.to_string());
        changed
    });
    if changed {
        log_event(
            "claude_takeover.probe_decision",
            json!({
                "decision": match decision {
                    TakeoverDecision::Takeover => "takeover",
                    TakeoverDecision::Wait(_) => "wait",
                    TakeoverDecision::Skip(_) => "skip",
                    TakeoverDecision::Degraded(_) => "degraded",
                },
                "reason": reason,
                "mode": inputs.mode.as_str(),
                "appRunning": matches!(inputs.base.app, AppProbe::Running(_)),
                "attached": inputs.base.cdp_ready,
                "copyRunning": inputs.copy_running,
                "otherInstance": inputs.base.multi_instance_running,
                "inUse": inputs.base.user_active,
            }),
        );
    }
}

// ---------------------------------------------------------------- 输入采集

fn ms_since(instant: Option<Instant>, now: Instant) -> Option<u64> {
    instant.map(|instant| now.duration_since(instant).as_millis() as u64)
}

/// `manual`：手动「立即接管」时也要探测（开关可能关着）。
fn collect_inputs(manual: bool) -> ClaudeTakeoverInputs {
    let (enabled, mode) = settings_view();
    // 只有真可能接管时才扫进程：开关关着或不是完整功能模式时不白跑 pgrep。
    let probe = if PLATFORM_SUPPORTED && mode == LaunchMode::Full && (enabled || manual) {
        core::probe_now().1
    } else {
        ClaudeProbe {
            app: AppProbe::NotRunning,
            ..ClaudeProbe::default()
        }
    };
    let snapshot = with_inner(|state| {
        // 接管失败后已把官方 Claude 开回去：这个新实例记成「本次不接管」。
        if state.skip_next_instance
            && !probe.attached
            && let AppProbe::Running(key) = &probe.app
        {
            state.skip_next_instance = false;
            state.paused_instance = Some(key.clone());
            persist_paused_instance(key);
        }
        // 「本次不接管」只管那一个实例：它退了或换代就作废。
        let stale = match (&state.paused_instance, &probe.app) {
            (Some(paused), AppProbe::Running(current)) => paused != current,
            (Some(_), AppProbe::NotRunning) => true,
            _ => false,
        };
        let fresh = match &probe.app {
            AppProbe::Running(key) if !probe.attached => match &state.first_seen {
                Some((seen, fresh)) if seen == key => *fresh,
                _ => {
                    let fresh = !core::instance_seems_in_use(probe.instance_age_secs);
                    state.first_seen = Some((key.clone(), fresh));
                    fresh
                }
            },
            _ => true,
        };
        if stale {
            state.paused_instance = None;
            if state.state == "paused" {
                state.state = "probing";
            }
            let _ = std::fs::remove_file(paused_instance_path());
        }
        // 过期的重启记录在锁里丢掉（接管线程也在往里追加，不能拿旧快照整份写回）。
        let now_ms = now_ms();
        state
            .restart_history_ms
            .retain(|timestamp| now_ms.saturating_sub(*timestamp) < codex_plus_core::auto_takeover::RESTART_WINDOW_MS);
        (
            state.last_attempt,
            state.our_launch_at,
            state.last_wake_at,
            state.consecutive_failures,
            state.restart_history_ms.clone(),
            state.paused_instance.clone(),
            fresh,
        )
    });
    let now = Instant::now();
    let user_active = !snapshot.6;
    ClaudeTakeoverInputs {
        mode,
        copy_running: probe.copy_running,
        base: TakeoverInputs {
            enabled,
            platform_supported: PLATFORM_SUPPORTED,
            session: core::session_for(&probe),
            app: probe.app.clone(),
            cdp_ready: probe.attached,
            actual_port: 0,
            multi_instance_running: probe.other_instance,
            takeover_in_progress: IN_PROGRESS.load(Ordering::Acquire),
            ms_since_our_launch: ms_since(snapshot.1, now),
            ms_since_last_attempt: ms_since(snapshot.0, now),
            ms_since_wake: ms_since(snapshot.2, now),
            consecutive_failures: snapshot.3,
            user_active,
            active_policy: ActivePolicy::Defer,
            paused_instance: snapshot.5,
            restart_history_ms: snapshot.4,
            now_ms: now_ms(),
        },
    }
}

// ---------------------------------------------------------------- 执行动作

/// 这个实例还在不在（代际键一致才算同一个）。
fn instance_alive(instance: &InstanceKey) -> bool {
    core::instance_identity(instance.pid).is_some_and(|(current, _)| current == *instance)
}

fn wait_instance_exit(instance: &InstanceKey, budget: Duration) -> bool {
    let deadline = Instant::now() + budget;
    loop {
        if !instance_alive(instance) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(EXIT_POLL);
    }
}

/// 请官方 Claude 退出并等它连同子进程完全退干净。`Err` 是给人看的原因。
fn quiesce(instance: &InstanceKey, command_line: &str) -> Result<(), String> {
    log_event(
        "claude_takeover.quitting",
        json!({ "pid": instance.pid, "lstart": instance.lstart }),
    );
    let mut stage = "graceful";
    let _ = core::request_exit(instance.pid, command_line, ExitStage::Graceful);
    if !wait_instance_exit(instance, EXIT_GRACE) {
        stage = "terminate";
        let _ = core::request_exit(instance.pid, command_line, ExitStage::Terminate);
        if !wait_instance_exit(instance, EXIT_FORCE_GRACE) {
            stage = "kill";
            let _ = core::request_exit(instance.pid, command_line, ExitStage::Kill);
            if !wait_instance_exit(instance, EXIT_FORCE_GRACE) {
                log_event("claude_takeover.exit_result", json!({ "pid": instance.pid, "outcome": "still_running" }));
                return Err("官方 Claude 没有退出。".to_string());
            }
        }
    }
    // 主进程退了，子进程可能还在回写配置：等它们走完；超时再收掉遗留的（Windows 的 crashpad）。
    let mut reaped = 0;
    if !core::wait_all_exited(CHILDREN_GRACE) {
        reaped = core::reap_children(&core::scan_now());
        if !core::wait_all_exited(EXIT_FORCE_GRACE) {
            log_event("claude_takeover.exit_result", json!({ "pid": instance.pid, "outcome": "children_left" }));
            return Err("Claude 没有完全退出，还有进程在运行。".to_string());
        }
    }
    log_event(
        "claude_takeover.exit_result",
        json!({ "pid": instance.pid, "outcome": "exited", "stage": stage, "reapedChildren": reaped }),
    );
    Ok(())
}

fn record_failure(reason: &str, message: String) {
    let failures = with_inner(|state| {
        state.consecutive_failures = state.consecutive_failures.saturating_add(1);
        state.last_error = Some(message.clone());
        state.last_reason = Some(reason.to_string());
        state.state = "degraded";
        state.consecutive_failures
    });
    log_event(
        "claude_takeover.degraded",
        json!({
            "reason": reason,
            "message": message,
            "consecutiveFailures": failures,
            "backoffMs": backoff_delay_ms(failures),
        }),
    );
}

/// 发起一次接管（体检 → 复核实例 → 退出 → 经完整功能模式启动），在独立线程里跑。
fn spawn_takeover(instance: InstanceKey) -> Result<(), String> {
    if IN_PROGRESS.swap(true, Ordering::AcqRel) {
        return Err("已有接管正在进行，请稍候。".to_string());
    }
    with_inner(|state| {
        state.last_attempt = Some(Instant::now());
        state.state = "taking_over";
        state.last_reason = Some("takeover".to_string());
    });
    let spawned = std::thread::Builder::new()
        .name("codex-plus-claude-takeover-run".into())
        .spawn(move || {
            run_takeover(&instance);
            IN_PROGRESS.store(false, Ordering::Release);
        });
    if let Err(error) = spawned {
        IN_PROGRESS.store(false, Ordering::Release);
        record_failure("spawn_failed", error.to_string());
        return Err(format!("接管线程启动失败：{error}"));
    }
    Ok(())
}

fn run_takeover(instance: &InstanceKey) {
    if crate::app_is_exiting() {
        return;
    }
    // 体检：完整功能模式起不来就不动用户的 Claude（退了也只会让他什么都没有）。
    if let Err((code, message)) = core::check_launch_ready() {
        record_failure(code, message);
        return;
    }
    // 复核：体检要几秒，期间用户可能自己退了、换了实例、开了副本，任何变化都放弃这次。
    let (scan, probe) = core::probe_now();
    let still_same = !probe.attached
        && !probe.other_instance
        && !probe.copy_running
        && probe.app == AppProbe::Running(instance.clone());
    let Some(command_line) = scan
        .command_line(instance.pid)
        .filter(|_| still_same)
        .map(|line| line.to_string())
    else {
        with_inner(|state| state.state = "probing");
        log_event("claude_takeover.instance_changed", json!({ "pid": instance.pid }));
        return;
    };
    let now = now_ms();
    with_inner(|state| {
        push_restart(&mut state.restart_history_ms, now);
        state.restarts_total = state.restarts_total.saturating_add(1);
        state.last_takeover_at_ms = Some(now);
    });
    log_event("claude_takeover.restart_requested", json!({ "pid": instance.pid }));
    if let Err(message) = quiesce(instance, &command_line) {
        record_failure("quit_failed", message);
        return;
    }
    match codex_plus_core::claude_local_backend::perform("launch") {
        Ok(result) => {
            with_inner(|state| {
                state.consecutive_failures = 0;
                // 宽限期从启动完成算起。
                state.our_launch_at = Some(Instant::now());
                state.state = "probing";
                state.last_error = None;
            });
            log_event("claude_takeover.ready", json!({ "message": result.message }));
        }
        Err(error) => {
            record_failure("launch_failed", format!("{error:#}"));
            // 已经退了用户的 Claude 却没起来：把官方 Claude 原样开回去，并跳过这个新实例。
            with_inner(|state| state.skip_next_instance = true);
            if let Err(error) = core::reopen_official() {
                with_inner(|state| state.skip_next_instance = false);
                log_event("claude_takeover.reopen_failed", json!({ "message": format!("{error:#}") }));
            }
        }
    }
}

// ---------------------------------------------------------------- 跳过标记

fn paused_instance_path() -> PathBuf {
    codex_plus_core::paths::default_app_state_dir().join("claude-auto-takeover-skip.json")
}

fn persist_paused_instance(key: &InstanceKey) {
    let bytes = serde_json::to_vec_pretty(&json!({
        "pid": key.pid,
        "lstart": key.lstart,
        "atMs": now_ms(),
    }))
    .unwrap_or_default();
    let _ = codex_plus_core::settings::atomic_write(&paused_instance_path(), &bytes);
}

/// 读回上次的「本次不接管」：管理器重开后，用户跳过的那个实例仍要跳过。
fn load_paused_instance() -> Option<InstanceKey> {
    let bytes = std::fs::read(paused_instance_path()).ok()?;
    let value = serde_json::from_slice::<serde_json::Value>(&bytes).ok()?;
    Some(InstanceKey::new(
        u32::try_from(value.get("pid")?.as_u64()?).ok()?,
        value.get("lstart")?.as_str()?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_reports_the_build_platform() {
        assert_eq!(payload().platform_supported, cfg!(any(target_os = "macos", windows)));
    }

    #[test]
    fn exit_budget_is_shorter_on_windows() {
        assert!(EXIT_GRACE <= Duration::from_secs(15));
        assert_eq!(EXIT_GRACE == Duration::from_secs(5), cfg!(windows));
    }
}
