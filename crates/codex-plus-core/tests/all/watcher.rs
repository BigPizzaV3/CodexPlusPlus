use codex_plus_core::watcher::{
    MACOS_WATCHER_LABEL, build_spawn_launcher_command, build_watcher_install_plan, cdp_listening,
    codex_process_ids, disable_watcher_at, enable_watcher_at, filter_killable_launcher_processes,
    is_linux_codex_desktop_process, parse_linux_proc_stat_starttime,
    macos_launcher_process_names, macos_launch_agent_installed_at, macos_launch_agent_path_at,
    macos_launch_agent_plist, process_id_is_running, process_ids_still_running,
    remove_macos_launch_agent_at, should_recover_stale_launcher, watcher_disabled_flag,
    set_watcher_start_codex_at, set_watcher_start_codex_on_open_at, set_watcher_start_minimized_at,
    watcher_start_codex_at, watcher_start_codex_on_open_at, watcher_start_minimized_at,
    write_macos_launch_agent_at,
};

#[cfg(windows)]
use codex_plus_core::watcher::{
    WindowsProcessInfo, find_codex_processes_from_snapshot,
    find_session_index_cleanup_blocking_processes_from_snapshot,
};

#[test]
fn cdp_listening_returns_true_for_bound_loopback_port() {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();

    assert!(cdp_listening(port));
}

#[test]
fn cdp_listening_returns_true_for_bound_ipv6_loopback_port() {
    // Windows 环境可能禁用 IPv6 loopback 访问；不可用时跳过这个环境专用回归。
    let probe_listener = std::net::TcpListener::bind("[::1]:0").unwrap();
    let probe_port = probe_listener.local_addr().unwrap().port();
    if std::net::TcpStream::connect(("::1", probe_port)).is_err() {
        return;
    }
    drop(probe_listener);

    let listener = std::net::TcpListener::bind("[::1]:0").unwrap();
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
fn watcher_start_minimized_is_a_separate_persistent_preference() {
    let dir = tempfile::tempdir().unwrap();
    assert!(!watcher_start_minimized_at(dir.path()));

    set_watcher_start_minimized_at(dir.path(), true).unwrap();
    assert!(watcher_start_minimized_at(dir.path()));

    set_watcher_start_minimized_at(dir.path(), false).unwrap();
    assert!(!watcher_start_minimized_at(dir.path()));
}

#[test]
fn watcher_start_codex_is_a_separate_persistent_preference() {
    let dir = tempfile::tempdir().unwrap();
    assert!(!watcher_start_codex_at(dir.path()));

    set_watcher_start_codex_at(dir.path(), true).unwrap();
    assert!(watcher_start_codex_at(dir.path()));

    set_watcher_start_codex_at(dir.path(), false).unwrap();
    assert!(!watcher_start_codex_at(dir.path()));
}

#[test]
fn watcher_start_codex_on_open_is_a_separate_persistent_preference() {
    let dir = tempfile::tempdir().unwrap();
    assert!(!watcher_start_codex_on_open_at(dir.path()));

    set_watcher_start_codex_on_open_at(dir.path(), true).unwrap();
    assert!(watcher_start_codex_on_open_at(dir.path()));

    set_watcher_start_codex_on_open_at(dir.path(), false).unwrap();
    assert!(!watcher_start_codex_on_open_at(dir.path()));
}

#[test]
fn macos_launch_agent_registers_the_gui_at_login() {
    let home = tempfile::tempdir().unwrap();
    let launcher = home
        .path()
        .join("Codex++.app/Contents/MacOS/CodexPlusPlus");
    assert!(!macos_launch_agent_installed_at(home.path()));

    let path = write_macos_launch_agent_at(home.path(), &launcher).unwrap();

    assert_eq!(path, macos_launch_agent_path_at(home.path()));
    assert_eq!(
        path.file_name().unwrap().to_string_lossy(),
        format!("{MACOS_WATCHER_LABEL}.plist")
    );
    assert!(
        path.parent()
            .unwrap()
            .ends_with(std::path::Path::new("Library").join("LaunchAgents"))
    );
    assert!(macos_launch_agent_installed_at(home.path()));

    let plist = std::fs::read_to_string(&path).unwrap();
    assert!(plist.contains(&format!("<string>{MACOS_WATCHER_LABEL}</string>")));
    assert!(plist.contains(&launcher.to_string_lossy().to_string()));
    assert!(plist.contains("<string>--autostart</string>"));
    assert!(plist.contains("<key>RunAtLoad</key>\n  <true/>"));

    remove_macos_launch_agent_at(home.path()).unwrap();
    assert!(!macos_launch_agent_installed_at(home.path()));
    // 重复移除是幂等的：界面关掉开关时不会因为文件已不存在而报错。
    remove_macos_launch_agent_at(home.path()).unwrap();
}

#[test]
fn macos_launch_agent_plist_escapes_the_executable_path() {
    let plist = macos_launch_agent_plist(std::path::Path::new(
        "/Applications/A&B <x>.app/Contents/MacOS/CodexPlusPlus",
    ));

    assert!(plist.contains("/Applications/A&amp;B &lt;x&gt;.app/Contents/MacOS/CodexPlusPlus"));
    assert!(!plist.contains("A&B <x>"));
}

#[test]
fn watcher_install_plan_opens_the_gui_at_logon() {
    let plan = build_watcher_install_plan("C:/Tools/codex-plus-plus.exe".into(), 9333);

    assert_eq!(plan.run_value_name, "CodexPlusPlusWatcher");
    assert_eq!(
        plan.run_value,
        "\"C:/Tools/codex-plus-plus.exe\" --autostart"
    );
    assert_eq!(plan.shortcut_name, "CodexPlusPlusWatcher.lnk");
    assert_eq!(plan.shortcut_target, "C:/Tools/codex-plus-plus.exe");
    assert_eq!(plan.shortcut_arguments, "--autostart");
}

#[test]
fn startup_command_opens_only_the_unified_gui() {
    let command = build_spawn_launcher_command("C:/Tools/codex-plus-plus.exe", 9444);

    assert_eq!(command[0], "C:/Tools/codex-plus-plus.exe");
    assert_eq!(command, ["C:/Tools/codex-plus-plus.exe", "--autostart"]);
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
fn codex_process_filter_keeps_chatgpt_desktop_package_processes() {
    let processes = [
        (
            21,
            r"C:\Program Files\WindowsApps\OpenAI.ChatGPT-Desktop_1.2026.133.0_x64__abc\app\ChatGPT.exe",
        ),
        (
            22,
            r"C:\Program Files\WindowsApps\OpenAI.Codex_26.707.3748.0_x64__abc\app\ChatGPT.exe",
        ),
        (
            23,
            r"C:\Program Files\WindowsApps\OpenAI.ChatGPT-Desktop_1.2026.133.0_x64__abc\app\resources\ChatGPT.exe",
        ),
        (
            24,
            r"C:\Program Files\WindowsApps\Other.ChatGPT_1.0.0.0_x64__abc\app\ChatGPT.exe",
        ),
    ];

    assert_eq!(codex_process_ids(processes), vec![21, 22]);
}

#[test]
fn codex_process_filter_recognizes_beta_desktop_without_matching_cli_or_other_apps() {
    let processes = [
        (
            31,
            r"C:\Program Files\WindowsApps\OpenAI.CodexBeta_26.727.4816.0_x64__abc\app\ChatGPT (Beta).exe",
        ),
        (
            32,
            r"C:\Program Files\WindowsApps\OpenAI.CodexBeta_26.727.4816.0_x64__abc\app\resources\codex.exe",
        ),
        (
            33,
            r"C:\Program Files\WindowsApps\Other.App_1.0_x64__abc\app\ChatGPT (Beta).exe",
        ),
        (34, r"D:\Portable\ChatGPT (Beta).exe"),
    ];
    assert_eq!(codex_process_ids(processes), vec![31]);
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

#[test]
fn macos_launcher_process_names_cover_development_and_packaged_binaries() {
    assert_eq!(
        macos_launcher_process_names(),
        ["codex-plus-plus", "CodexPlusPlus"]
    );
}

#[test]
fn stale_launcher_recovery_only_runs_when_codex_and_cdp_are_absent() {
    assert!(should_recover_stale_launcher(false, false));
    assert!(!should_recover_stale_launcher(true, false));
    assert!(!should_recover_stale_launcher(false, true));
    assert!(!should_recover_stale_launcher(true, true));
}

#[test]
fn stop_wait_tracks_only_expected_process_ids() {
    assert_eq!(
        process_ids_still_running(&[10, 20, 30], [5, 20, 40, 30]),
        vec![20, 30]
    );
}

#[cfg(any(windows, target_os = "linux", target_os = "macos"))]
#[test]
fn process_liveness_distinguishes_current_and_missing_processes() {
    assert_eq!(process_id_is_running(std::process::id()), Some(true));
    assert_eq!(process_id_is_running(u32::MAX), Some(false));
}

#[cfg(windows)]
#[test]
fn find_codex_processes_finds_local_install_with_capitial_c() {
    let processes = [WindowsProcessInfo {
        process_id: 42,
        parent_process_id: 0,
        exe_file: "Codex.exe".to_string(),
        executable_path: Some(std::path::PathBuf::from(
            r"D:\360Downloads\codexapp\app\Codex.exe",
        )),
    }];

    assert_eq!(find_codex_processes_from_snapshot(&processes), vec![42]);
}

#[cfg(windows)]
#[test]
fn find_codex_processes_ignores_lowercase_local_cli_binary() {
    let processes = [WindowsProcessInfo {
        process_id: 43,
        parent_process_id: 0,
        exe_file: "codex.exe".to_string(),
        executable_path: Some(std::path::PathBuf::from(
            r"D:\360Downloads\codexapp\app\codex.exe",
        )),
    }];

    assert!(find_codex_processes_from_snapshot(&processes).is_empty());
}

#[cfg(windows)]
#[test]
fn find_codex_processes_ignores_npm_cli_binary() {
    let processes = [WindowsProcessInfo {
        process_id: 44,
        parent_process_id: 0,
        exe_file: "codex.exe".to_string(),
        executable_path: Some(std::path::PathBuf::from(
            r"C:\Users\me\AppData\Roaming\npm\node_modules\@openai\codex\node_modules\@openai\codex-win32-x64\vendor\x86_64-pc-windows-msvc\bin\codex.exe",
        )),
    }];

    assert!(find_codex_processes_from_snapshot(&processes).is_empty());
}

#[cfg(windows)]
#[test]
fn find_codex_processes_ignores_packaged_resource_cli_binary() {
    let processes = [WindowsProcessInfo {
        process_id: 45,
        parent_process_id: 0,
        exe_file: "codex.exe".to_string(),
        executable_path: Some(std::path::PathBuf::from(
            r"C:\Program Files\WindowsApps\OpenAI.Codex_1.0.0.0_x64__abc\app\resources\codex.exe",
        )),
    }];

    assert!(find_codex_processes_from_snapshot(&processes).is_empty());
}

#[cfg(windows)]
#[test]
fn find_codex_processes_combines_store_and_local_installs() {
    let processes = [
        WindowsProcessInfo {
            process_id: 11,
            parent_process_id: 0,
            exe_file: "ChatGPT.exe".to_string(),
            executable_path: Some(std::path::PathBuf::from(
                r"C:\Program Files\WindowsApps\OpenAI.ChatGPT-Desktop_1.2026.133.0_x64__abc\app\ChatGPT.exe",
            )),
        },
        WindowsProcessInfo {
            process_id: 42,
            parent_process_id: 0,
            exe_file: "Codex.exe".to_string(),
            executable_path: Some(std::path::PathBuf::from(
                r"D:\360Downloads\codexapp\app\Codex.exe",
            )),
        },
    ];

    assert_eq!(find_codex_processes_from_snapshot(&processes), vec![11, 42]);
}

#[cfg(windows)]
#[test]
fn session_index_cleanup_process_guard_blocks_desktop_apps_but_not_cli() {
    let processes = [
        WindowsProcessInfo {
            process_id: 11,
            parent_process_id: 0,
            exe_file: "ChatGPT.exe".to_string(),
            executable_path: Some(std::path::PathBuf::from(
                r"C:\Program Files\WindowsApps\OpenAI.ChatGPT-Desktop_1.2026.133.0_x64__abc\app\ChatGPT.exe",
            )),
        },
        WindowsProcessInfo {
            process_id: 12,
            parent_process_id: 0,
            exe_file: "ChatGPT.exe".to_string(),
            executable_path: Some(std::path::PathBuf::from(r"D:\Portable\ChatGPT\ChatGPT.exe")),
        },
        WindowsProcessInfo {
            process_id: 13,
            parent_process_id: 0,
            exe_file: "Codex.exe".to_string(),
            executable_path: Some(std::path::PathBuf::from(r"D:\Portable\Codex\Codex.exe")),
        },
        WindowsProcessInfo {
            process_id: 14,
            parent_process_id: 0,
            exe_file: "codex.exe".to_string(),
            executable_path: Some(std::path::PathBuf::from(
                r"C:\Users\me\AppData\Roaming\npm\node_modules\@openai\codex\bin\codex.exe",
            )),
        },
    ];

    assert_eq!(
        find_session_index_cleanup_blocking_processes_from_snapshot(&processes),
        vec![11, 12, 13]
    );
    assert_eq!(find_codex_processes_from_snapshot(&processes), vec![11, 13]);
}

#[cfg(windows)]
#[test]
fn beta_desktop_is_included_in_watcher_and_session_cleanup_guard() {
    let processes = [
        WindowsProcessInfo {
            process_id: 61,
            parent_process_id: 0,
            exe_file: "ChatGPT (Beta).exe".to_string(),
            executable_path: Some(std::path::PathBuf::from(
                r"C:\Program Files\WindowsApps\OpenAI.CodexBeta_26.727.4816.0_x64__abc\app\ChatGPT (Beta).exe",
            )),
        },
        WindowsProcessInfo {
            process_id: 62,
            parent_process_id: 0,
            exe_file: "codex.exe".to_string(),
            executable_path: Some(std::path::PathBuf::from(
                r"C:\Program Files\WindowsApps\OpenAI.CodexBeta_26.727.4816.0_x64__abc\app\resources\codex.exe",
            )),
        },
    ];
    assert_eq!(find_codex_processes_from_snapshot(&processes), vec![61]);
    assert_eq!(
        find_session_index_cleanup_blocking_processes_from_snapshot(&processes),
        vec![61]
    );
}

#[cfg(windows)]
#[test]
fn find_codex_processes_ignores_unrelated_processes() {
    let processes = [
        WindowsProcessInfo {
            process_id: 10,
            parent_process_id: 0,
            exe_file: "notepad.exe".to_string(),
            executable_path: Some(std::path::PathBuf::from(r"C:\Windows\notepad.exe")),
        },
        WindowsProcessInfo {
            process_id: 20,
            parent_process_id: 0,
            exe_file: "codex-plus-plus.exe".to_string(),
            executable_path: Some(std::path::PathBuf::from(
                r"D:\Programs\Codex++\codex-plus-plus.exe",
            )),
        },
    ];

    assert!(find_codex_processes_from_snapshot(&processes).is_empty());
}

#[test]
fn linux_proc_stat_starttime_parsing_tests() {
    // 1. 标准 /proc/<pid>/stat 输出：第 22 个字段（从最后一个 ')' 算起第 19 个 token）为 starttime
    let stat = "66483 (cat) R 61311 66483 66483 34818 66483 4194304 215 0 0 0 0 0 0 0 22 0 1 0 8908263 9170944 428 18446744073709551615";
    assert_eq!(
        parse_linux_proc_stat_starttime(stat),
        Some("8908263".to_string())
    );

    // 2. comm 中包含空格与嵌套括号的复杂真实场景（如 "(ChatGPT (Dev))"）
    let complex_stat = "12345 (ChatGPT (Dev)) S 1000 12345 12345 0 0 4194304 100 0 0 0 10 5 0 0 20 0 5 0 12345678 1000000 500";
    assert_eq!(
        parse_linux_proc_stat_starttime(complex_stat),
        Some("12345678".to_string())
    );

    // 3. 截断或非法的 stat 串，必须返回 None
    assert_eq!(parse_linux_proc_stat_starttime(""), None);
    assert_eq!(parse_linux_proc_stat_starttime("12345 (cat)"), None);
    assert_eq!(parse_linux_proc_stat_starttime("12345 cat R 1 2 3"), None);
}

#[cfg(target_os = "linux")]
#[test]
fn linux_find_codex_processes_live_test() {
    let pids = codex_plus_core::watcher::find_codex_processes();
    let p9229 = codex_plus_core::watcher::find_codex_processes_for_debug_port(9229);
    for pid in pids.iter().chain(p9229.iter()) {
        assert!(*pid > 0);
    }
}

#[test]
fn linux_codex_desktop_process_pure_function_argv_and_exe_tests() {
    use std::path::Path;

    // 1. 任意非 Codex 的 AppImage（如 Obsidian.AppImage）绝对不能被识别为桌面版
    assert!(!is_linux_codex_desktop_process(
        &["/home/user/Applications/Obsidian.AppImage".into()],
        Some(Path::new("/home/user/Applications/Obsidian.AppImage")),
        None,
        None,
    ));
    assert!(!is_linux_codex_desktop_process(
        &["Obsidian.AppImage".into()],
        Some(Path::new("/usr/bin/Obsidian.AppImage")),
        None,
        None,
    ));

    // AppImage 实际运行时 exe 指向挂载目录，未挂载文件路径本身不按文件名比对
    assert!(!is_linux_codex_desktop_process(
        &["/home/user/Applications/ChatGPT.AppImage".into()],
        Some(Path::new("/home/user/Applications/ChatGPT.AppImage")),
        None,
        None,
    ));
    // 挂载后的 AppImage 目录通过 expected_app_dir 比对
    let mount_dir = Path::new("/tmp/.mount_ChatGPT1234");
    let mount_exe = Path::new("/tmp/.mount_ChatGPT1234/ChatGPT");
    assert!(is_linux_codex_desktop_process(
        &["/tmp/.mount_ChatGPT1234/ChatGPT".into()],
        Some(mount_exe),
        Some(mount_dir),
        None,
    ));

    // 2. 带有引号参数的命令行（直接模拟 /proc/<pid>/cmdline 的 NUL 分隔真实 argv token）
    assert!(!is_linux_codex_desktop_process(
        &["codex".into(), "修一下 app.asar".into()],
        Some(Path::new("/usr/bin/codex")),
        None,
        None,
    ));
    assert!(!is_linux_codex_desktop_process(
        &["codex".into(), "--msg=remote-debugging-port=9222".into()],
        Some(Path::new("/usr/bin/codex")),
        None,
        None,
    ));
    assert!(!is_linux_codex_desktop_process(
        &["/usr/lib/codex/codex".into()],
        Some(Path::new("/usr/lib/codex/codex")),
        None,
        None,
    ));

    // 3. 官方桌面安装目录（白名单）
    assert!(is_linux_codex_desktop_process(
        &["/usr/lib/chatgpt/ChatGPT".into()],
        Some(Path::new("/usr/lib/chatgpt/ChatGPT")),
        None,
        None,
    ));
    assert!(is_linux_codex_desktop_process(
        &["/opt/codex/codex".into()],
        Some(Path::new("/opt/codex/codex")),
        None,
        None,
    ));

    // 4. 动态解析出来的应用目录（expected_app_dir / build_codex_executable）比对
    let custom_app_dir = Path::new("/custom/chatgpt");
    let custom_exe = Path::new("/custom/chatgpt/ChatGPT");
    assert!(is_linux_codex_desktop_process(
        &["/custom/chatgpt/ChatGPT".into()],
        Some(custom_exe),
        Some(custom_app_dir),
        None,
    ));
    // 目录不匹配且无调试端口时拒绝
    assert!(!is_linux_codex_desktop_process(
        &["/other/chatgpt/ChatGPT".into()],
        Some(Path::new("/other/chatgpt/ChatGPT")),
        Some(custom_app_dir),
        None,
    ));

    // 5. 调试端口过滤与多开隔离：主实例 (9229) vs 多开实例 (9230)
    let argv_9229 = vec![
        "/usr/lib/chatgpt/ChatGPT".into(),
        "--remote-debugging-port=9229".into(),
    ];
    let argv_9230 = vec![
        "/usr/lib/chatgpt/ChatGPT".into(),
        "--remote-debugging-port=9230".into(),
    ];
    let exe = Some(Path::new("/usr/lib/chatgpt/ChatGPT"));

    // 不指定 expected_port 时两者均为桌面主进程
    assert!(is_linux_codex_desktop_process(&argv_9229, exe, None, None));
    assert!(is_linux_codex_desktop_process(&argv_9230, exe, None, None));

    // 指定 expected_port 9229 时，仅匹配 argv_9229
    assert!(is_linux_codex_desktop_process(
        &argv_9229,
        exe,
        None,
        Some(9229)
    ));
    assert!(!is_linux_codex_desktop_process(
        &argv_9230,
        exe,
        None,
        Some(9229)
    ));

    // 指定 expected_port 9230 时，仅匹配 argv_9230
    assert!(!is_linux_codex_desktop_process(
        &argv_9229,
        exe,
        None,
        Some(9230)
    ));
    assert!(is_linux_codex_desktop_process(
        &argv_9230,
        exe,
        None,
        Some(9230)
    ));

    // 6. 空 argv 必须直接返回 false
    assert!(!is_linux_codex_desktop_process(&[], None, None, None));
}
