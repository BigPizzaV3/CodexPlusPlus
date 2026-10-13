//! 自动接管探针的命令行验证工具。
//!
//! 存在的理由：管理器的守护线程跑在 Tauri 进程里，而 CI / 无人值守的 Windows
//! 会话没有桌面，Tauri 起不来（本机实测：连 notepad 都没有窗口句柄）。
//! 这个 example 只跑**判据**那一半，不依赖 GUI，用来在真机上验证
//! 「能不能认出官方 App」「CDP 通了没有」「多开是否存在」。
//!
//! 用法：
//! ```text
//! cargo run --example takeover_probe -- "C:\Program Files\WindowsApps\OpenAI.Codex_...\app" 9229
//! ```
//! 不给参数时自动探测 App 目录、端口用 9229。

use codex_plus_core::auto_takeover::{AppProbe, probe_desktop_app_state};

fn main() {
    let mut args = std::env::args().skip(1);
    let app_dir = args
        .next()
        .map(std::path::PathBuf::from)
        .or_else(|| codex_plus_core::app_paths::resolve_codex_app_dir(None));
    let Some(app_dir) = app_dir else {
        eprintln!("找不到官方 App 目录；请显式传入第一个参数。");
        std::process::exit(2);
    };
    let requested_port = args
        .next()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(9229);

    let probe = probe_desktop_app_state(&app_dir, requested_port);
    println!("app_dir      = {}", app_dir.display());
    println!("actual_port  = {}", probe.actual_port);
    println!("cdp_ready    = {}", probe.cdp_ready);
    println!("other_instance = {}", probe.other_instance);
    match &probe.app {
        AppProbe::Running(key) => {
            println!("app          = Running(pid={}, incarnation={})", key.pid, key.lstart)
        }
        AppProbe::NotRunning => println!("app          = NotRunning"),
        AppProbe::Unknown => println!("app          = Unknown"),
    }

    // 用真实探测结果喂给决策引擎，验证的不只是探针，而是「守护线程这一刻会做什么」。
    {
        use codex_plus_core::auto_takeover::{
            AppProbe as P, SessionSnapshot, TakeoverDecision, TakeoverInputs, decide,
        };
        let inputs = TakeoverInputs {
            enabled: true,
            platform_supported: cfg!(any(target_os = "macos", windows)),
            // 官方 App 已带着我们的调试端口在跑，就等价于会话已就绪——
            // 这正是 launcher 自己的语义（select_macos_debug_launch_action 的
            // ReuseRunningDebugApp）。用它验证「不会重复接管」。
            session: if probe.cdp_ready {
                SessionSnapshot {
                    owned: true,
                    ready: true,
                    status: Some("running".to_string()),
                    debug_port: Some(probe.actual_port),
                    started_at_ms: Some(1),
                }
            } else {
                SessionSnapshot::default()
            },
            app: probe.app.clone(),
            cdp_ready: probe.cdp_ready,
            actual_port: probe.actual_port,
            multi_instance_running: probe.other_instance,
            takeover_in_progress: false,
            ms_since_our_launch: None,
            ms_since_last_attempt: Some(600_000),
            ms_since_wake: Some(600_000),
            consecutive_failures: 0,
            user_active: false,
            active_policy: codex_plus_core::auto_takeover::ActivePolicy::Immediate,
            paused_instance: None,
            restart_history_ms: Vec::new(),
            now_ms: 0,
        };
        let decision = decide(&inputs);
        let label = match &decision {
            TakeoverDecision::Takeover => "Takeover",
            TakeoverDecision::Wait(reason) => reason,
            TakeoverDecision::Skip(reason) => reason,
            TakeoverDecision::Degraded(reason) => reason,
        };
        let kind = match decision {
            TakeoverDecision::Takeover => "takeover",
            TakeoverDecision::Wait(_) => "wait",
            TakeoverDecision::Skip(_) => "skip",
            TakeoverDecision::Degraded(_) => "degraded",
        };
        println!("engine       = {kind}({label})");
        let _ = P::NotRunning;
    }

    let verdict = match (&probe.app, probe.cdp_ready) {
        (AppProbe::Running(_), true) => "already_attached（无需动作）",
        (AppProbe::Running(_), false) if probe.other_instance => "skip: 检测到多开实例",
        (AppProbe::Running(_), false) => "takeover（需要接管：先退出再带参重开）",
        (AppProbe::NotRunning, _) => "wait: 官方 App 没在运行",
        (AppProbe::Unknown, _) => "skip: 探测失败（保守不动）",
    };
    println!("decision     = {verdict}");
}
