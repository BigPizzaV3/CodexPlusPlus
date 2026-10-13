//! 自动接管的真实环境验证。
//!
//! 这些用例需要本机正在运行官方 Codex App，默认 **忽略** 掉，避免在 CI 或没装
//! 官方 App 的机器上误报失败。手动跑：
//!
//! ```bash
//! cargo test -p codex-plus-core --test auto_takeover_live -- --ignored --nocapture
//! ```
//!
//! 只做只读探测：列出进程、断言判据。不会 quit / kill / 注入任何进程。

#![cfg(target_os = "macos")]

use codex_plus_core::cdp;

/// 探针能认出真实运行的官方 App 主进程，且不会把包内 CLI 算进来。
#[test]
#[ignore = "需要本机正在运行官方 Codex App"]
fn live_desktop_probe_finds_the_main_process_only() {
    let processes = codex_plus_core::watcher::find_macos_codex_desktop_main_processes(None);
    println!("检测到 {} 个主进程：", processes.len());
    for process in &processes {
        println!(
            "  pid={} lstart={:?} exe={} bundle={:?}",
            process.pid, process.lstart, process.executable, process.bundle_identifier
        );
    }

    // 包内官方 CLI（Contents/Resources/...）绝不能出现在名单里。
    for process in &processes {
        assert!(
            !process.executable.contains("/Contents/Resources/"),
            "包内 CLI 被误判成主进程：{}",
            process.executable
        );
        assert!(
            !process.executable.contains("/Helpers/"),
            "renderer/helper 被误判成主进程：{}",
            process.executable
        );
        // 代际键必须解析出来，否则「用户重启了 App」判不出来。
        assert_eq!(
            process.lstart.split_whitespace().count(),
            5,
            "lstart 解析异常：{:?}",
            process.lstart
        );
    }

    // 注意：不强制要求名单非空——用户可能根本没开官方 App。
    if processes.is_empty() {
        println!("提示：当前没有运行中的官方 App，跳过非空断言。");
    }
}

/// 与 `runtime_health` 的旧判据对比：新的精确判据不应该把 CLI 算成 App。
#[test]
#[ignore = "需要本机正在运行官方 Codex App"]
fn live_probe_is_stricter_than_find_codex_processes() {
    let loose = codex_plus_core::watcher::find_codex_processes();
    let strict = codex_plus_core::watcher::find_macos_codex_desktop_main_processes(None);
    println!("find_codex_processes（含 CLI 误判）: {loose:?}");
    println!(
        "精确主进程判据: {:?}",
        strict.iter().map(|p| p.pid).collect::<Vec<_>>()
    );

    for process in &strict {
        assert!(
            loose.contains(&process.pid),
            "精确判据命中的 pid {} 必须也在宽判据里（宽判据是超集才对）",
            process.pid
        );
    }
}

/// CDP 端点探测是严格判据：端口能连不等于可注入。
#[test]
#[ignore = "本机 9229 未必开着"]
fn live_cdp_endpoint_reports_injectable_targets() {
    let ready = cdp::endpoint_available(9229);
    println!("9229 可注入：{ready}");
}

/// 判据的端到端：真机上的官方 App（带调试端口）必须被判成「在运行」，
/// 且实例键能作为代际使用。这条一旦坏掉，表现是「开关打开了却永远不接管」。
#[test]
#[ignore = "需要本机正在运行官方 Codex App"]
fn live_takeover_decision_sees_an_attached_app() {
    use codex_plus_core::auto_takeover::{
        AppProbe, SessionSnapshot, TakeoverDecision, TakeoverInputs, decide,
    };

    let app = std::path::Path::new("/Applications/ChatGPT.app");
    if !app.exists() {
        println!("提示：本机没有 /Applications/ChatGPT.app，跳过。");
        return;
    }
    let expected = codex_plus_core::app_paths::build_codex_executable(app)
        .to_string_lossy()
        .to_string();
    let processes = codex_plus_core::watcher::find_macos_codex_desktop_main_processes(Some(9229));
    let app_probe = match processes
        .into_iter()
        .find(|process| process.executable == expected)
    {
        Some(process) => AppProbe::Running(codex_plus_core::auto_takeover::InstanceKey::new(
            process.pid,
            process.lstart,
        )),
        None => AppProbe::NotRunning,
    };
    println!("AppProbe = {app_probe:?}");
    assert!(
        matches!(app_probe, AppProbe::Running(_)),
        "带调试端口运行中的官方 App 必须被判成 Running"
    );

    let inputs = TakeoverInputs {
        enabled: true,
        platform_supported: true,
        session: SessionSnapshot {
            owned: true,
            ready: true,
            status: Some("running".to_string()),
            debug_port: Some(9229),
            started_at_ms: Some(1),
        },
        app: app_probe,
        cdp_ready: cdp::endpoint_available(9229),
        actual_port: 9229,
        takeover_in_progress: false,
        ms_since_last_attempt: Some(600_000),
        ms_since_wake: Some(600_000),
        ..TakeoverInputs::default()
    };
    assert_eq!(
        decide(&inputs),
        TakeoverDecision::Wait("already_attached"),
        "已接管的实例绝不能被再次重启"
    );
}
