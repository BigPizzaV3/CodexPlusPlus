//! 自动接管官方 App：守护线程与状态快照。
//!
//! 用户自己双击 / Dock / `open -a` 启动官方 App 时，App 不带
//! `--remote-debugging-port`，Codex++ 接不上去。本模块起一个探针线程，发现
//! 「官方 App 在跑 + 没有我们的调试端口」就调用既有 launcher 会话把它接管过来
//! （已在跑的实例由 launcher 负责优雅退出后带参重开）。
//!
//! 判据全部在 `codex_plus_core::auto_takeover`，本模块只负责：
//! **采集输入 → 调判据 → 执行动作 → 维护状态**。所有可能阻塞的操作（`ps`、
//! osascript、launcher 启动）都不在探针线程里做决策等待。
//!
//! 安全边界（每一条都对应一个实测出来的坑）：
//! - 宽限期：`open` 返回到页面可注入之间有 2.5~3.4 秒空窗，没有宽限期会自杀式重启。
//! - 串行化：复用管理器进程内的 `LAUNCH_IN_PROGRESS`，不与用户点「启动」抢跑。
//! - 多开：`osascript quit` 按应用名发事件，会把用户正在用的多开实例一起退出。
//! - 平台：仅 macOS；Windows 走 AUMID 激活，语义完全不同。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use codex_plus_core::auto_takeover::{
    ActivePolicy, AppProbe, InstanceKey, SessionSnapshot, TakeoverDecision, TakeoverInputs,
    backoff_delay_ms, push_restart, restarts_in_window,
};
use codex_plus_core::diagnostic_log::append_diagnostic_log;
use codex_plus_core::settings::SettingsStore;
use codex_plus_core::status::StatusStore;
use serde::Serialize;
use serde_json::json;

/// 线程 tick 上限：实际间隔由判据给出（活跃 2s / 静默 10s），这里只做心跳。
const TICK_CAP: Duration = Duration::from_millis(2_000);
/// 刚接管后的强制静默期：launcher 会话启动期间不再做任何判定。
const POST_TAKEOVER_QUIET: Duration = Duration::from_secs(10);
/// 管理器启动后的静默期。
///
/// 管理器自己的启动流程会先跑 `resume_existing_codex_background`（恢复已有 Codex 的
/// helper / 代理 / 注入），这段要几秒到几十秒。守护线程若同时开探，会看到
/// 「App 在跑 + 会话还没就绪」而误判成「需要接管」，撞上启动互斥锁后把自己写成
/// degraded —— 实测过一次（2026-10-13 14:08 本机日志），所以必须等启动流程走完。
const STARTUP_QUIET: Duration = Duration::from_secs(20);
/// 请求官方 App 退出后等它自己退出的预算。
///
/// macOS 侧实测：能退成功的样本耗时 752–3422ms，而原先 5 秒的预算有 7/25 次失败
/// （全部卡在 5175–5293ms），所以这里给到 15 秒。
const EXIT_GRACE: Duration = Duration::from_secs(15);
/// 退不掉时再给强制结束的窗口。
const EXIT_FORCE_GRACE: Duration = Duration::from_secs(5);
/// 退出轮询间隔（macOS 实测 100ms 足够）。
const EXIT_POLL: Duration = Duration::from_millis(100);

/// 「疑似有人在用」的会话事件流新鲜度阈值。
const USER_ACTIVE_WINDOW: Duration = Duration::from_secs(20);

/// 给前端的只读状态。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoTakeoverPayload {
    /// disabled | idle | probing | taking_over | degraded | paused
    pub state: &'static str,
    pub enabled: bool,
    pub platform_supported: bool,
    pub active_policy: &'static str,
    pub last_takeover_at_ms: Option<u64>,
    pub last_reason: Option<String>,
    pub last_error: Option<String>,
    pub actual_debug_port: Option<u16>,
    pub app_running: Option<bool>,
    pub session_owned: bool,
    pub session_status: Option<String>,
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
    actual_debug_port: Option<u16>,
    app_running: Option<bool>,
    session_owned: bool,
    session_status: Option<String>,
    restart_history_ms: Vec<u64>,
    restarts_total: u32,
    paused_instance: Option<InstanceKey>,
    last_attempt: Option<Instant>,
    last_success: Option<Instant>,
    last_failure: Option<Instant>,
    consecutive_failures: u32,
    /// 我们自己刚发起启动的时刻（宽限期起点，必须在调用 start() **之前**打点）。
    our_launch_at: Option<Instant>,
    /// 最近一次系统唤醒（睡眠/唤醒后给 CDP 恢复宽限）。
    last_wake_at: Option<Instant>,
    /// 上一次 tick 的单调时钟，用来识别挂起（睡眠后 Instant 会跳）。
    last_tick: Option<Instant>,
    /// 守护线程启动时刻，用于启动静默期。
    started_at: Option<Instant>,
    platform_supported: bool,
}

static INNER: OnceLock<Mutex<Inner>> = OnceLock::new();
static STOP: AtomicBool = AtomicBool::new(true);
static RUNNING: AtomicBool = AtomicBool::new(false);

fn inner() -> &'static Mutex<Inner> {
    INNER.get_or_init(|| {
        Mutex::new(Inner {
            state: "disabled",
            platform_supported: cfg!(target_os = "macos"),
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

/// 当前配置是否开启自动接管。
pub fn is_enabled() -> bool {
    SettingsStore::default()
        .load()
        .map(|settings| settings.codex_app_auto_takeover)
        .unwrap_or(false)
}

/// 启动守护线程（幂等）。设置里没开就什么都不做。
pub fn start_supervisor() {
    if !is_enabled() {
        with_inner(|state| state.state = "disabled");
        return;
    }
    if RUNNING.swap(true, Ordering::AcqRel) {
        return;
    }
    STOP.store(false, Ordering::Release);
    with_inner(|state| {
        state.started_at = Some(Instant::now());
        state.state = "probing";
    });
    let spawned = std::thread::Builder::new()
        .name("codex-plus-auto-takeover".into())
        .spawn(supervisor_loop);
    match spawned {
        Ok(_) => log_event("takeover.supervisor_started", json!({})),
        Err(error) => {
            RUNNING.store(false, Ordering::Release);
            with_inner(|state| {
                state.state = "degraded";
                state.last_error = Some(format!("守护线程启动失败：{error}"));
            });
            log_event(
                "takeover.supervisor_start_failed",
                json!({ "message": error.to_string() }),
            );
        }
    }
}

/// 请求停止守护线程（用户关掉开关或程序退出）。线程会在下一次 tick 自行退出。
pub fn stop_supervisor() {
    STOP.store(true, Ordering::Release);
    with_inner(|state| {
        state.state = "disabled";
        state.last_reason = Some("stopped".to_string());
    });
}

/// 设置变化后同步守护线程的启停。
pub fn apply_enabled(enabled: bool) {
    if enabled {
        start_supervisor();
    } else {
        stop_supervisor();
    }
}

/// 「本次不接管」：只跳过**当前这个 App 实例**，用户重启 App 后恢复。
pub fn pause_until_next_instance() -> Result<(), String> {
    let key = current_instance().ok_or_else(|| "当前没有检测到官方 App 实例。".to_string())?;
    with_inner(|state| {
        state.paused_instance = Some(key.clone());
        state.last_reason = Some("paused_for_instance".to_string());
        state.state = "paused";
    });
    persist_paused_instance(&key);
    log_event(
        "takeover.paused_for_instance",
        json!({ "pid": key.pid, "lstart": key.lstart }),
    );
    Ok(())
}

/// 「立即接管」：忽略「疑似有人在用」的等待，但仍保留宽限期与防风暴闸门。
pub fn take_over_now() -> Result<(), String> {
    let inputs = match collect_inputs() {
        Ok(inputs) => inputs,
        Err(error) => return Err(error),
    };
    if !inputs.platform_supported {
        return Err("自动接管目前只支持 macOS。".to_string());
    }
    if inputs.session.is_degraded() {
        return Err("当前会话注入已降级，请先重试注入。".to_string());
    }
    if inputs.multi_instance_running {
        return Err("检测到 Codex 多开实例，已跳过自动接管。".to_string());
    }
    if inputs.takeover_in_progress {
        return Err("已有启动/重启正在进行，请稍候。".to_string());
    }
    match inputs.app {
        AppProbe::NotRunning => return Err("没有检测到官方 App 正在运行。".to_string()),
        AppProbe::Unknown => return Err("无法确认官方 App 状态，已保守跳过。".to_string()),
        AppProbe::Running(_) => {}
    }
    if inputs.cdp_ready && inputs.session.is_running() {
        return Err("官方 App 已经被接管了。".to_string());
    }
    if !inputs.restart_budget_left_for_manual() {
        return Err("一小时内接管次数已达上限，已停止自动重试。".to_string());
    }
    spawn_takeover(inputs.actual_port);
    Ok(())
}

/// 只读状态快照，给前端。
pub fn payload() -> AutoTakeoverPayload {
    let enabled = is_enabled();
    with_inner(|state| {
        let restarts_last_hour = restarts_in_window(&state.restart_history_ms, now_ms()) as u32;
        AutoTakeoverPayload {
            state: state.state,
            enabled,
            platform_supported: state.platform_supported,
            active_policy: active_policy().as_str(),
            last_takeover_at_ms: state.last_takeover_at_ms,
            last_reason: state.last_reason.clone(),
            last_error: state.last_error.clone(),
            actual_debug_port: state.actual_debug_port,
            app_running: state.app_running,
            session_owned: state.session_owned,
            session_status: state.session_status.clone(),
            restarts_last_hour,
            restarts_total: state.restarts_total,
            paused_instance: state.paused_instance.as_ref().map(InstanceKey::storage_key),
            running: RUNNING.load(Ordering::Acquire),
        }
    })
}

fn active_policy() -> ActivePolicy {
    SettingsStore::default()
        .load()
        .map(|settings| ActivePolicy::parse(&settings.codex_app_auto_takeover_when_active))
        .unwrap_or(ActivePolicy::Defer)
}

// ---------------------------------------------------------------- 探针线程

fn supervisor_loop() {
    log_event(
        "takeover.probe_decision",
        json!({ "decision": "start", "platformSupported": cfg!(target_os = "macos") }),
    );
    while !STOP.load(Ordering::Acquire) {
        let tick_started = Instant::now();
        let interval = tick();
        let elapsed = tick_started.elapsed();
        let sleep_for = interval.saturating_sub(elapsed).min(TICK_CAP);
        std::thread::sleep(sleep_for);
    }
    with_inner(|state| {
        state.state = "disabled";
    });
    RUNNING.store(false, Ordering::Release);
    log_event("takeover.supervisor_stopped", json!({}));
}

/// 一次探测 + 决策。返回下一次 tick 的间隔。
fn tick() -> Duration {
    // 睡眠/唤醒检测：挂起期间 Instant 会跳跃，这里把它当作「刚唤醒」，
    // 给 CDP 一段恢复宽限，避免唤醒瞬间误重启。
    let now = Instant::now();
    let woke = with_inner(|state| {
        let woke = state
            .last_tick
            .is_some_and(|last| now.duration_since(last) > Duration::from_secs(90));
        state.last_tick = Some(now);
        if woke {
            state.last_wake_at = Some(now);
        }
        woke
    });
    if woke {
        log_event("takeover.wake_detected", json!({}));
    }

    // 刚发起接管后的静默期：launcher 会话正在启动，这期间不做任何判定，
    // 免得在它自己拉起的实例上再叠一次动作。
    let quiet = with_inner(|state| {
        state
            .last_attempt
            .is_some_and(|last| last.elapsed() < POST_TAKEOVER_QUIET)
    });
    if quiet {
        return Duration::from_millis(codex_plus_core::auto_takeover::PROBE_INTERVAL_ACTIVE_MS);
    }
    // 管理器启动静默期：让 resume_existing_codex_background 先把会话恢复好。
    let starting_up = with_inner(|state| {
        state
            .started_at
            .is_some_and(|started| started.elapsed() < STARTUP_QUIET)
    });
    if starting_up {
        return Duration::from_millis(codex_plus_core::auto_takeover::PROBE_INTERVAL_ACTIVE_MS);
    }

    let inputs = match collect_inputs() {
        Ok(inputs) => inputs,
        Err(error) => {
            with_inner(|state| {
                state.state = "degraded";
                state.last_error = Some(error.clone());
            });
            log_event("takeover.probe_failed", json!({ "message": error }));
            return Duration::from_millis(codex_plus_core::auto_takeover::PROBE_INTERVAL_IDLE_MS);
        }
    };

    let decision = codex_plus_core::auto_takeover::decide(&inputs);
    publish(&inputs, decision);

    if decision.is_takeover() {
        spawn_takeover(inputs.actual_port);
    }
    inputs.probe_interval()
}

/// 把探针结果与决策写进状态，并在**决策变化时**记一条日志（避免 2 秒一条刷屏）。
fn publish(inputs: &TakeoverInputs, decision: TakeoverDecision) {
    let reason = decision.reason();
    let changed = with_inner(|state| {
        state.actual_debug_port = Some(inputs.actual_port);
        state.app_running = Some(matches!(inputs.app, AppProbe::Running(_)));
        state.session_owned = inputs.session.owned;
        state.session_status = inputs.session.status.clone();
        state.state = match decision {
            TakeoverDecision::Takeover => "taking_over",
            TakeoverDecision::Degraded(_) => "degraded",
            TakeoverDecision::Skip("disabled") => "disabled",
            _ => {
                if inputs.paused_instance.is_some() && state.state == "paused" {
                    "paused"
                } else {
                    "probing"
                }
            }
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
            "takeover.probe_decision",
            json!({
                "decision": match decision {
                    TakeoverDecision::Takeover => "takeover",
                    TakeoverDecision::Wait(_) => "wait",
                    TakeoverDecision::Skip(_) => "skip",
                    TakeoverDecision::Degraded(_) => "degraded",
                },
                "reason": reason,
                "appRunning": matches!(inputs.app, AppProbe::Running(_)),
                "cdpReady": inputs.cdp_ready,
                "sessionOwned": inputs.session.owned,
                "sessionStatus": inputs.session.status,
                "multiInstance": inputs.multi_instance_running,
                "userActive": inputs.user_active,
                "actualPort": inputs.actual_port,
            }),
        );
    }
}

// ---------------------------------------------------------------- 输入采集

fn collect_inputs() -> Result<TakeoverInputs, String> {
    let mut state = with_inner(|state| {
        (
            state.last_attempt,
            state.our_launch_at,
            state.last_wake_at,
            state.consecutive_failures,
            state.restart_history_ms.clone(),
            state.paused_instance.clone(),
        )
    });
    let platform_supported = cfg!(any(target_os = "macos", windows));
    let enabled = is_enabled();
    let session_snapshot = codex_plus_launcher::session_snapshot();
    let latest = StatusStore::default().load_latest().ok().flatten();
    let actual_port = session_snapshot
        .debug_port
        .or_else(|| latest.as_ref().and_then(|status| status.debug_port))
        .unwrap_or_else(default_debug_port);
    let app_dir = resolve_app_dir(latest.as_ref());

    let app = match app_dir.as_deref() {
        Some(app_dir) => probe_desktop_app(app_dir),
        None => AppProbe::Unknown,
    };
    let cdp_ready = codex_plus_core::cdp::endpoint_available(actual_port);
    let current_pid = match &app {
        AppProbe::Running(key) => Some(key.pid),
        _ => None,
    };
    let multi_instance_running = app_dir
        .as_deref()
        .is_some_and(|app_dir| {
            codex_plus_core::auto_takeover::has_other_desktop_instance(
                app_dir,
                actual_port,
                current_pid,
            )
        })
        || multi_instance_running();
    let user_active = user_seems_active();

    let session = SessionSnapshot {
        owned: session_snapshot.owned,
        ready: session_snapshot.ready,
        // 只在本进程确实拥有会话时才认状态文件里的 status，否则会把上一代的
        // `running` 当成「已接管」，导致永远不接管。
        status: session_snapshot
            .owned
            .then(|| session_snapshot.status.clone())
            .flatten(),
        debug_port: session_snapshot.debug_port,
        started_at_ms: session_snapshot.started_at_ms,
    };

    let now = Instant::now();
    let inputs = TakeoverInputs {
        enabled,
        platform_supported,
        session,
        app,
        cdp_ready,
        actual_port,
        multi_instance_running,
        takeover_in_progress: crate::commands::launch_in_progress(),
        ms_since_our_launch: state
            .1
            .map(|instant| now.duration_since(instant).as_millis() as u64),
        ms_since_last_attempt: state
            .0
            .map(|instant| now.duration_since(instant).as_millis() as u64),
        ms_since_wake: state
            .2
            .map(|instant| now.duration_since(instant).as_millis() as u64),
        consecutive_failures: state.3,
        user_active,
        active_policy: active_policy(),
        paused_instance: state.5.clone(),
        restart_history_ms: state.4.clone(),
        now_ms: now_ms(),
    };
    // 历史必须写回：`collect_inputs` 之后 `spawn_takeover` 会往里追加本次重启，
    // 只读不写回的话防风暴闸门永远是 0 次。
    with_inner(|state| state.restart_history_ms = inputs.restart_history_ms.clone());
    Ok(inputs)
}

/// 官方 App 主进程探测：判据在 core（macOS 与 Windows 各自实现），这里只提供
/// 解析好的 App 目录。
fn probe_desktop_app(app_dir: &Path) -> AppProbe {
    codex_plus_core::auto_takeover::probe_desktop_app(app_dir)
}

fn current_instance() -> Option<InstanceKey> {
    let latest = StatusStore::default().load_latest().ok().flatten();
    let app_dir = resolve_app_dir(latest.as_ref())?;
    let port = latest
        .as_ref()
        .and_then(|status| status.debug_port)
        .unwrap_or_else(default_debug_port);
    match probe_desktop_app(&app_dir) {
        AppProbe::Running(key) => Some(key),
        _ => None,
    }
}

fn resolve_app_dir(latest: Option<&codex_plus_core::status::LaunchStatus>) -> Option<PathBuf> {
    let settings = SettingsStore::default().load().unwrap_or_default();
    codex_plus_core::app_paths::resolve_codex_app_dir_with_saved(
        latest
            .and_then(|status| status.codex_app.as_deref())
            .map(Path::new),
        Some(settings.codex_app_path.as_str()),
    )
}

fn default_debug_port() -> u16 {
    codex_plus_core::ports::select_packaged_codex_debug_port(9229)
}

/// 多开实例在跑：多开 worker 是管理器自己拉起的进程，清单落在 app state 下。
/// 必须跳过——`osascript quit` 按应用名发事件，会把用户正在用的第二实例一起退出。
fn multi_instance_running() -> bool {
    let root = codex_plus_core::paths::default_app_state_dir().join("codex-multi");
    let Ok(entries) = std::fs::read_dir(&root) else {
        return false;
    };
    for entry in entries.flatten() {
        let manifest = entry.path().join("instance.json");
        let Ok(bytes) = std::fs::read(&manifest) else {
            continue;
        };
        let Ok(instance) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            continue;
        };
        let Some(port) = instance
            .get("debugPort")
            .and_then(serde_json::Value::as_u64)
        else {
            continue;
        };
        let port = port as u16;
        if codex_plus_core::cdp::endpoint_available(port)
            || !codex_plus_core::watcher::find_codex_processes_for_debug_port(port).is_empty()
        {
            return true;
        }
    }
    false
}

/// 「疑似有人在用」：会话事件流最近有追加。运行中的任务会持续写
/// `~/.codex/sessions/**/*.jsonl`。只读、失败向保守（当作没在用，避免永不接管）。
fn user_seems_active() -> bool {
    let home = codex_plus_core::codex_sqlite::default_codex_home_dir();
    let sessions = home.join("sessions");
    let Ok(entries) = std::fs::read_dir(sessions) else {
        return false;
    };
    let mut newest: Option<std::time::SystemTime> = None;
    let mut stack = entries
        .flatten()
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    while let Some(path) = stack.pop() {
        let Ok(metadata) = std::fs::metadata(&path) else {
            continue;
        };
        if metadata.is_dir() {
            if let Ok(children) = std::fs::read_dir(&path) {
                stack.extend(children.flatten().map(|entry| entry.path()));
            }
            continue;
        }
        if path.extension().and_then(|value| value.to_str()) != Some("jsonl") {
            continue;
        }
        if let Ok(modified) = metadata.modified()
            && newest.is_none_or(|current| modified > current)
        {
            newest = Some(modified);
        }
    }
    newest
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|elapsed| elapsed < USER_ACTIVE_WINDOW)
}

// ---------------------------------------------------------------- 执行动作

/// 退出前再确认一次当前实例（拿到 (pid, 启动时刻) 代际键）。
fn probe_app_before_exit() -> AppProbe {
    match resolve_app_dir(StatusStore::default().load_latest().ok().flatten().as_ref()) {
        Some(app_dir) => codex_plus_core::auto_takeover::probe_desktop_app(&app_dir),
        None => AppProbe::Unknown,
    }
}

/// 请官方 App 退出并确认它真的退了。
///
/// 失败不是致命的：launcher 的 macOS 分支自己带「已有实例就重启」的逻辑，
/// 这里只是把「先退出」提前并做代际校验，避免 `open -W -a` 对运行中的 App 不传参。
fn quiesce_desktop_app(app_dir: &Path, instance: &InstanceKey) {
    log_event(
        "takeover.quitting",
        json!({
            "pid": instance.pid,
            "lstart": instance.lstart,
            "graceMs": EXIT_GRACE.as_millis() as u64,
        }),
    );
    let _ = codex_plus_core::auto_takeover::request_desktop_exit(app_dir, instance, true);
    let mut outcome = "exited";
    let mut forced = false;
    if !wait_for_instance_exit(app_dir, instance, EXIT_GRACE) {
        // 优雅关闭无效（自动化授权被拒 / 未响应 AppleEvent / 进程卡住）。
        // 强制结束是否可接受是产品决策，这里与 macOS 侧既有语义保持一致地做一次，
        // 并把代价写进诊断日志。
        forced = true;
        let _ = codex_plus_core::auto_takeover::request_desktop_exit(app_dir, instance, false);
        if !wait_for_instance_exit(app_dir, instance, EXIT_FORCE_GRACE) {
            outcome = "still_running";
        }
    }
    log_event(
        "takeover.exit_result",
        json!({ "pid": instance.pid, "outcome": outcome, "forced": forced }),
    );
}

/// 等实例真正退出。**代际必須匹配**：用户在空档里又手动启动了一次的话，
/// pid / 启动时刻都会变，那就直接放弃本次接管，绝不对他的新实例动手。
fn wait_for_instance_exit(app_dir: &Path, instance: &InstanceKey, budget: Duration) -> bool {
    let deadline = Instant::now() + budget;
    loop {
        match codex_plus_core::auto_takeover::probe_desktop_app(app_dir) {
            AppProbe::NotRunning => return true,
            AppProbe::Running(current) => {
                if current.pid != instance.pid || current.lstart != instance.lstart {
                    log_event(
                        "takeover.instance_changed",
                        json!({
                            "expectedPid": instance.pid,
                            "currentPid": current.pid,
                        }),
                    );
                    return true;
                }
            }
            AppProbe::Unknown => {}
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(EXIT_POLL);
    }
}

/// 发起一次接管。宽限期**在这里**打点（早于 `start()`），否则启动失败时下一轮
/// 探针会立刻二次接管。
fn spawn_takeover(actual_port: u16) {
    let started_at = Instant::now();
    // 这次到底是「重启官方 App」还是「附着到已带端口的实例」？
    // 后者不动用户窗口，不能算进重启额度，也不能记成重启（否则日志与预算都会失真）。
    let restarting = !codex_plus_core::cdp::endpoint_available(actual_port);
    with_inner(|state| {
        state.our_launch_at = Some(started_at);
        state.last_attempt = Some(started_at);
        state.state = "taking_over";
        state.last_reason = Some("takeover".to_string());
    });
    let now = now_ms();
    with_inner(|state| {
        if restarting {
            push_restart(&mut state.restart_history_ms, now);
            state.restarts_total = state.restarts_total.saturating_add(1);
        }
        state.last_takeover_at_ms = Some(now);
    });
    log_event(
        if restarting {
            "takeover.restart_requested"
        } else {
            "takeover.attach_requested"
        },
        json!({ "debugPort": actual_port, "restarting": restarting }),
    );

    // 实际启动放到独立线程：`launcher::start` 会阻塞到注入完成（冷启动可达百秒），
    // 探针线程不能卡在那里；串行锁交给启动线程持有，避免与用户点「启动」抢跑。
    std::thread::spawn(move || {
        let guard = match crate::commands::try_acquire_launch_guard_for_takeover() {
            Ok(guard) => guard,
            Err(message) => {
                log_event(
                    "takeover.skipped",
                    json!({ "reason": "launch_in_progress", "message": message }),
                );
                return;
            }
        };
        let latest = StatusStore::default().load_latest().ok().flatten();
        let app_dir = resolve_app_dir(latest.as_ref());
        // 需要重启时先请官方 App 退出：macOS 走 AppleEvent quit，Windows 走 taskkill。
        // 只在真的没有调试端口时才做——已有端口的实例交给 launcher 复用，不动用户窗口。
        if restarting
            && let (Some(app_dir), AppProbe::Running(instance)) =
                (app_dir.as_deref(), probe_app_before_exit())
        {
            quiesce_desktop_app(app_dir, &instance);
        }
        let request = crate::commands::LaunchRequest {
            app_path: app_dir
                .as_ref()
                .map(|path| path.to_string_lossy().to_string())
                .unwrap_or_default(),
            debug_port: codex_plus_core::ports::select_packaged_codex_debug_port(actual_port),
            helper_port: codex_plus_core::ports::select_platform_loopback_port(57321),
            sync_active_relay: true,
        };
        let debug_port = request.debug_port;
        let result = crate::commands::run_auto_takeover_launch(request);
        drop(guard);
        match result {
            Ok(()) => {
                with_inner(|state| {
                    state.consecutive_failures = 0;
                    state.last_success = Some(Instant::now());
                    state.our_launch_at = Some(Instant::now());
                    state.state = "probing";
                    state.last_error = None;
                });
                log_event(
                    if restarting {
                        "takeover.ready"
                    } else {
                        "takeover.attached"
                    },
                    json!({ "debugPort": debug_port, "restarting": restarting }),
                );
            }
            Err(error) => {
                let failures = with_inner(|state| {
                    state.consecutive_failures = state.consecutive_failures.saturating_add(1);
                    state.last_failure = Some(Instant::now());
                    state.last_error = Some(error.clone());
                    state.state = "degraded";
                    state.consecutive_failures
                });
                log_event(
                    "takeover.degraded",
                    json!({
                        "message": error,
                        "consecutiveFailures": failures,
                        "backoffMs": backoff_delay_ms(failures),
                    }),
                );
            }
        }
    });
}

// ---------------------------------------------------------------- 跳过标记

fn paused_instance_path() -> PathBuf {
    codex_plus_core::paths::default_app_state_dir().join("auto-takeover-skip.json")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_debug_port_is_a_loopback_port() {
        assert!(default_debug_port() >= 1024);
    }

    #[test]
    fn platform_support_matches_the_build_target() {
        let payload = payload();
        assert_eq!(payload.platform_supported, cfg!(target_os = "macos"));
    }

    #[test]
    fn pause_without_a_running_app_reports_a_readable_error() {
        // 没有可探测的实例时不应静默成功。
        let result = pause_until_next_instance();
        assert!(result.is_err() || current_instance().is_some());
    }
}
