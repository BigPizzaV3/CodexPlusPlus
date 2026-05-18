use codex_plus_core::watcher::{
    build_spawn_launcher_command, build_watcher_install_plan, cdp_listening, codex_process_ids,
    disable_watcher_at, enable_watcher_at, filter_killable_launcher_processes,
    watcher_disabled_flag,
};

#[test]
fn cdp_listening_returns_true_for_bound_loopback_port() {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();

    assert!(cdp_listening(port));
}

#[test]
fn cdp_listening_returns_false_for_closed_port() {
    let port = {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.local_addr().unwrap().port()
    };

    assert!(!cdp_listening(port));
}

#[test]
fn watcher_enable_and_disable_toggle_flag() {
    let dir = tempfile::tempdir().unwrap();
    let flag = watcher_disabled_flag(dir.path());

    disable_watcher_at(dir.path()).unwrap();
    assert!(flag.exists());

    enable_watcher_at(dir.path()).unwrap();
    assert!(!flag.exists());
}

#[test]
fn watcher_install_plan_registers_rust_launcher_at_logon() {
    let plan = build_watcher_install_plan("C:/Tools/codex-plus-plus.exe".into(), 9333);

    assert_eq!(plan.run_value_name, "CodexPlusPlusWatcher");
    assert_eq!(
        plan.run_value,
        "\"C:/Tools/codex-plus-plus.exe\" --debug-port 9333"
    );
    assert_eq!(plan.shortcut_name, "CodexPlusPlusWatcher.lnk");
    assert_eq!(plan.shortcut_target, "C:/Tools/codex-plus-plus.exe");
    assert_eq!(plan.shortcut_arguments, "--debug-port 9333");
}

#[test]
fn spawn_launcher_command_points_to_silent_binary_only() {
    let command = build_spawn_launcher_command("C:/Tools/codex-plus-plus.exe", 9444);

    assert_eq!(command[0], "C:/Tools/codex-plus-plus.exe");
    assert!(command.contains(&"--debug-port".to_string()));
    assert!(command.contains(&"9444".to_string()));
    assert!(!command.iter().any(|part| part.contains("manager")));
}

#[test]
fn codex_process_filter_keeps_only_windowsapps_codex_processes() {
    let processes = [
        (
            11,
            r"C:\Program Files\WindowsApps\OpenAI.Codex_1.0.0.0_x64__abc\app\Codex.exe",
        ),
        (12, r"C:\Tools\Codex.exe"),
        (
            13,
            r"C:\Program Files\WindowsApps\Other.App_1.0.0.0_x64__abc\app\Codex.exe",
        ),
    ];

    assert_eq!(codex_process_ids(processes), vec![11]);
}

#[test]
fn launcher_process_filter_protects_current_process_ancestry() {
    let processes = [
        (10, 0, "codex-plus-plus.exe"),
        (20, 10, "codex-plus-plus.exe"),
        (30, 20, "codex-plus-plus.exe"),
        (40, 10, "codex-plus-plus.exe"),
        (50, 10, "codex-plus-plus-manager.exe"),
    ];

    assert_eq!(filter_killable_launcher_processes(processes, 30), vec![40]);
}
