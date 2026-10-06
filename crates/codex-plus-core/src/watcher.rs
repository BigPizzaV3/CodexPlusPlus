use std::collections::{HashMap, HashSet};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
#[cfg(any(windows, target_os = "macos"))]
use std::process::{Command, Stdio};
use std::time::Duration;

#[cfg(windows)]
pub use crate::windows_integration::WindowsProcessInfo;

pub const WATCHER_INTERVAL_SECONDS: f64 = 3.0;
pub const CDP_PROBE_TIMEOUT_SECONDS: f64 = 0.5;
pub const TAKEOVER_FAILURE_BACKOFF_SECONDS: f64 = 30.0;
pub const RESTART_STOP_WAIT_TIMEOUT_MS: u64 = 5_000;
const RESTART_STOP_WAIT_INTERVAL_MS: u64 = 100;
/// 「等旧启动器退出」轮询退避的上限。
const LAUNCHER_WAIT_BACKOFF_MAX_MS: u64 = 1_000;
/// 强制结束后，再等多久确认残留实例已经消失。
const LAUNCHER_FORCE_KILL_GRACE_MS: u64 = 2_000;
pub const WATCHER_RUN_NAME: &str = "CodexPlusPlusWatcher";
pub const WATCHER_RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
pub const WATCHER_STARTUP_SHORTCUT_NAME: &str = "CodexPlusPlusWatcher.lnk";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatcherInstallPlan {
    pub run_value_name: String,
    pub run_value: String,
    pub shortcut_name: String,
    pub shortcut_target: String,
    pub shortcut_arguments: String,
}

pub fn watcher_disabled_flag(root: &Path) -> PathBuf {
    root.join("watcher.disabled")
}

pub fn default_watcher_disabled_flag() -> PathBuf {
    watcher_disabled_flag(&crate::paths::default_app_state_dir())
}

pub fn enable_watcher_at(root: &Path) -> std::io::Result<()> {
    let flag = watcher_disabled_flag(root);
    if flag.exists() {
        std::fs::remove_file(flag)?;
    }
    Ok(())
}

pub fn disable_watcher_at(root: &Path) -> std::io::Result<()> {
    let flag = watcher_disabled_flag(root);
    if let Some(parent) = flag.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(flag, b"disabled")
}

pub fn enable_watcher() -> std::io::Result<()> {
    enable_watcher_at(&crate::paths::default_app_state_dir())
}

pub fn disable_watcher() -> std::io::Result<()> {
    disable_watcher_at(&crate::paths::default_app_state_dir())
}

pub fn cdp_listening(port: u16) -> bool {
    [
        SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
        SocketAddr::from((Ipv6Addr::LOCALHOST, port)),
    ]
    .into_iter()
    .any(|addr| TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_ok())
}

pub fn build_spawn_launcher_command(launcher_path: &str, debug_port: u16) -> Vec<String> {
    vec![
        launcher_path.to_string(),
        "--debug-port".to_string(),
        debug_port.to_string(),
    ]
}

pub fn build_watcher_install_plan(launcher_path: PathBuf, debug_port: u16) -> WatcherInstallPlan {
    let launcher = launcher_path.to_string_lossy().to_string();
    let arguments = format!("--debug-port {debug_port}");
    WatcherInstallPlan {
        run_value_name: WATCHER_RUN_NAME.to_string(),
        run_value: format!("\"{launcher}\" {arguments}"),
        shortcut_name: WATCHER_STARTUP_SHORTCUT_NAME.to_string(),
        shortcut_target: launcher,
        shortcut_arguments: arguments,
    }
}

pub fn codex_process_ids<'a>(processes: impl IntoIterator<Item = (u32, &'a str)>) -> Vec<u32> {
    processes
        .into_iter()
        .filter_map(|(process_id, executable)| {
            is_windowsapps_codex_app_process(executable).then_some(process_id)
        })
        .collect()
}

fn is_windowsapps_codex_app_process(executable: &str) -> bool {
    let executable = executable.replace('/', "\\").to_ascii_lowercase();
    let Some((_, after_windows_apps)) = executable.split_once("\\windowsapps\\") else {
        return false;
    };
    let Some((package_name, after_package)) = after_windows_apps.split_once('\\') else {
        return false;
    };
    let supported_package = crate::app_paths::is_supported_windows_app_package_name(package_name)
        || package_name.starts_with("openai.chatgpt-desktop_");
    supported_package
        && after_package.starts_with("app\\")
        && !after_package.starts_with("app\\resources\\")
        && after_package
            .rsplit('\\')
            .next()
            .is_some_and(crate::app_paths::is_supported_app_executable_name)
}

pub fn filter_killable_launcher_processes<'a>(
    processes: impl IntoIterator<Item = (u32, u32, &'a str)>,
    current_process_id: u32,
) -> Vec<u32> {
    let processes = processes.into_iter().collect::<Vec<_>>();
    let parents = processes
        .iter()
        .map(|(process_id, parent_process_id, _)| (*process_id, *parent_process_id))
        .collect::<HashMap<_, _>>();
    let mut protected = HashSet::new();
    let mut cursor = current_process_id;
    while cursor != 0 && protected.insert(cursor) {
        cursor = parents.get(&cursor).copied().unwrap_or(0);
    }
    processes
        .into_iter()
        .filter(|(process_id, _, exe_file)| {
            !protected.contains(process_id) && exe_file.eq_ignore_ascii_case("codex-plus-plus.exe")
        })
        .map(|(process_id, _, _)| process_id)
        .collect()
}

pub fn should_recover_stale_launcher(has_codex_process: bool, cdp_listening: bool) -> bool {
    !has_codex_process && !cdp_listening
}

pub fn process_ids_still_running(
    expected: &[u32],
    running: impl IntoIterator<Item = u32>,
) -> Vec<u32> {
    let expected = expected.iter().copied().collect::<HashSet<_>>();
    running
        .into_iter()
        .filter(|process_id| expected.contains(process_id))
        .collect()
}

pub fn macos_launcher_process_names() -> [&'static str; 2] {
    [
        crate::install::SILENT_BINARY,
        crate::install::MACOS_SILENT_EXECUTABLE,
    ]
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessInstanceState {
    NotRunning,
    Running {
        started_at_secs: Option<u64>,
        birth_id: Option<String>,
    },
    Unknown,
}

#[cfg(windows)]
pub fn inspect_process_instance(process_id: u32) -> ProcessInstanceState {
    if process_id == 0 {
        return ProcessInstanceState::NotRunning;
    }
    let processes = crate::windows_integration::enumerate_processes();
    if processes.is_empty() {
        return ProcessInstanceState::Unknown;
    }
    if !processes
        .iter()
        .any(|process| process.process_id == process_id)
    {
        return ProcessInstanceState::NotRunning;
    }
    let birth_id = crate::windows_integration::process_birth_id(process_id);
    ProcessInstanceState::Running {
        started_at_secs: birth_id
            .and_then(crate::windows_integration::process_started_at_secs_from_birth_id),
        birth_id: birth_id.map(|birth_id| birth_id.to_string()),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub fn inspect_process_instance(process_id: u32) -> ProcessInstanceState {
    match process_id_is_running(process_id) {
        Some(false) => ProcessInstanceState::NotRunning,
        Some(true) => {
            let (started_at_secs, birth_id) = unix_process_identity(process_id);
            ProcessInstanceState::Running {
                started_at_secs,
                birth_id,
            }
        }
        None => ProcessInstanceState::Unknown,
    }
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
pub fn inspect_process_instance(process_id: u32) -> ProcessInstanceState {
    if process_id == 0 {
        ProcessInstanceState::NotRunning
    } else {
        ProcessInstanceState::Unknown
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn unix_process_identity(process_id: u32) -> (Option<u64>, Option<String>) {
    let process_id_arg = process_id.to_string();
    let output = std::process::Command::new("ps")
        .args([
            "-p",
            process_id_arg.as_str(),
            "-o",
            "etime=",
            "-o",
            "lstart=",
        ])
        .env("LC_ALL", "C")
        .output();
    let Ok(output) = output else {
        return (None, None);
    };
    if !output.status.success() {
        return (None, None);
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let text = text.trim();
    let Some(split_at) = text.find(char::is_whitespace) else {
        return (None, None);
    };
    let elapsed = parse_ps_elapsed_seconds(&text[..split_at]);
    let birth_id = text[split_at..].trim();
    let started_at_secs = elapsed.and_then(|elapsed| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|now| now.as_secs().saturating_sub(elapsed))
    });
    (
        started_at_secs,
        (!birth_id.is_empty()).then(|| birth_id.to_string()),
    )
}

#[cfg(any(target_os = "linux", target_os = "macos", test))]
fn parse_ps_elapsed_seconds(value: &str) -> Option<u64> {
    let (days, time) = if let Some((days, time)) = value.split_once('-') {
        (days.parse().ok()?, time)
    } else {
        (0, value)
    };
    let parts = time
        .split(':')
        .map(str::parse::<u64>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    let (hours, minutes, seconds) = match parts.as_slice() {
        [minutes, seconds] => (0, *minutes, *seconds),
        [hours, minutes, seconds] => (*hours, *minutes, *seconds),
        _ => return None,
    };
    Some(days * 86_400 + hours * 3_600 + minutes * 60 + seconds)
}

#[cfg(test)]
mod process_identity_tests {
    use super::*;

    #[test]
    fn parses_ps_elapsed_time_formats() {
        assert_eq!(parse_ps_elapsed_seconds("03:04"), Some(184));
        assert_eq!(parse_ps_elapsed_seconds("02:03:04"), Some(7_384));
        assert_eq!(parse_ps_elapsed_seconds("2-02:03:04"), Some(180_184));
        assert_eq!(parse_ps_elapsed_seconds("invalid"), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_codex_process_scan_matches_app_executables_not_command_arguments() {
        let processes = [
            "  42 /Applications/ChatGPT.app/Contents/MacOS/ChatGPT --remote-debugging-port=9229",
            "  11 /Applications/Codex Dev.app/Contents/MacOS/Codex Dev --remote-debugging-port=9229",
            "  43 /Applications/ChatGPT.app/Contents/Frameworks/Codex Framework.framework/Helpers/Codex (Renderer).app/Contents/MacOS/Codex (Renderer)",
            "  44 /Applications/Codex++.app/Contents/MacOS/CodexPlusPlus",
            "  45 /bin/zsh -lc '/Applications/ChatGPT.app/Contents/MacOS/ChatGPT'",
            "  46 /usr/bin/open -W -a /Applications/ChatGPT.app",
        ];

        // 真机上的 `ChatGPT.app` 是 Codex 桌面版（bundle id com.openai.codex），
        // `Codex Dev` 用点分子标识符；注入查表避免依赖本机是否装了这两个 App。
        let identifiers = |app_dir: &Path| match app_dir.to_string_lossy().as_ref() {
            "/Applications/ChatGPT.app" => Some("com.openai.codex".to_string()),
            "/Applications/Codex Dev.app" => Some("com.openai.codex.dev".to_string()),
            _ => None,
        };

        assert_eq!(
            macos_codex_process_ids_with(processes, identifiers),
            vec![11, 42]
        );
    }

    /// issue #2222：`ChatGPT Classic.app` 的主可执行文件也叫 `ChatGPT`，只按可执行名
    /// 匹配会把「普通 ChatGPT 会话」误判成「Codex 在运行」，导致 provider sync 的前置
    /// 守卫拦下供应商切换。bundle id 是 `com.openai.chat`，必须判为不是 Codex。
    #[cfg(target_os = "macos")]
    #[test]
    fn chatgpt_classic_is_not_treated_as_codex_desktop() {
        let processes = [
            "  51 /Applications/ChatGPT Classic.app/Contents/MacOS/ChatGPT",
            "  52 /Applications/ChatGPT.app/Contents/MacOS/ChatGPT",
        ];

        let identifiers = |app_dir: &Path| match app_dir.to_string_lossy().as_ref() {
            "/Applications/ChatGPT Classic.app" => Some("com.openai.chat".to_string()),
            "/Applications/ChatGPT.app" => Some("com.openai.codex".to_string()),
            _ => None,
        };

        assert_eq!(
            macos_codex_process_ids_with(processes, identifiers),
            vec![52]
        );
    }

    /// bundle id 读不到（二进制 plist / 文件缺失）时才退回可执行名判据。
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_codex_scan_falls_back_to_executable_name_without_bundle_identifier() {
        assert!(is_macos_codex_desktop_main_with(
            "/Applications/ChatGPT.app/Contents/MacOS/ChatGPT",
            |_| None
        ));
        assert!(is_macos_codex_desktop_main_with(
            "/Applications/Codex Dev.app/Contents/MacOS/Codex Dev",
            |_| None
        ));
        // 可执行名不在兜底列表里，即便读不到 bundle id 也不能命中。
        assert!(!is_macos_codex_desktop_main_with(
            "/Applications/ChatGPT Classic.app/Contents/MacOS/ChatGPT Classic",
            |_| None
        ));
    }

    #[cfg(any(target_os = "macos", test))]
    #[test]
    fn codex_bundle_identifier_matches_only_the_codex_family() {
        assert!(is_macos_codex_bundle_identifier("com.openai.codex"));
        assert!(is_macos_codex_bundle_identifier("com.openai.codex.dev"));
        assert!(is_macos_codex_bundle_identifier("com.openai.codex.beta"));
        assert!(!is_macos_codex_bundle_identifier("com.openai.chat"));
        assert!(!is_macos_codex_bundle_identifier("com.openai.chatgpt"));
        // 前缀相同但不是点分子标识符，不能算同一族。
        assert!(!is_macos_codex_bundle_identifier(
            "com.openai.codexextension"
        ));
        assert!(!is_macos_codex_bundle_identifier(""));
    }

    #[cfg(any(target_os = "macos", test))]
    #[test]
    fn plist_string_value_reads_the_requested_key() {
        let plist = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
<dict>
	<key>CFBundleIdentifier</key>
	<string>com.openai.chat</string>
	<key>CFBundleName</key>
	<string>ChatGPT</string>
</dict>
</plist>
"#;

        assert_eq!(
            plist_string_value(plist, "CFBundleIdentifier").as_deref(),
            Some("com.openai.chat")
        );
        assert_eq!(
            plist_string_value(plist, "CFBundleName").as_deref(),
            Some("ChatGPT")
        );
        assert_eq!(plist_string_value(plist, "CFBundleVersion"), None);
    }

    /// 端到端走一遍真实的 plist 读取：临时目录里摆出 `.app` 布局，
    /// 断言从可执行路径推出的 bundle id 判据生效。
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_codex_app_layout_is_detected_through_the_real_plist_reader() {
        let temp = tempfile::tempdir().unwrap();
        let write_app = |name: &str, executable: &str, identifier: &str| {
            let contents = temp.path().join(name).join("Contents");
            std::fs::create_dir_all(contents.join("MacOS")).unwrap();
            std::fs::write(
                contents.join("Info.plist"),
                format!(
                    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\">\n<dict>\n\t<key>CFBundleIdentifier</key>\n\t<string>{identifier}</string>\n</dict>\n</plist>\n"
                ),
            )
            .unwrap();
            format!(
                "{}/Contents/MacOS/{executable}",
                temp.path().join(name).display()
            )
        };

        let codex = write_app("ChatGPT.app", "ChatGPT", "com.openai.codex");
        let classic = write_app("ChatGPT Classic.app", "ChatGPT", "com.openai.chat");

        assert!(is_macos_codex_desktop_main(&codex));
        assert!(
            !is_macos_codex_desktop_main(&classic),
            "ChatGPT Classic.app 不能被当成 Codex 桌面版"
        );
    }

    #[cfg(windows)]
    #[test]
    fn current_windows_process_has_a_stable_birth_identity() {
        let ProcessInstanceState::Running {
            started_at_secs,
            birth_id,
        } = inspect_process_instance(std::process::id())
        else {
            panic!("current process should be visible");
        };

        assert!(started_at_secs.is_some());
        assert!(birth_id.is_some());
    }
}

#[cfg(windows)]
pub fn process_id_is_running(process_id: u32) -> Option<bool> {
    match inspect_process_instance(process_id) {
        ProcessInstanceState::NotRunning => Some(false),
        ProcessInstanceState::Running { .. } => Some(true),
        ProcessInstanceState::Unknown => None,
    }
}

#[cfg(target_os = "linux")]
pub fn process_id_is_running(process_id: u32) -> Option<bool> {
    if process_id == 0 {
        return Some(false);
    }
    match std::fs::metadata(Path::new("/proc").join(process_id.to_string())) {
        Ok(_) => Some(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(false),
        Err(_) => None,
    }
}

#[cfg(target_os = "macos")]
pub fn process_id_is_running(process_id: u32) -> Option<bool> {
    if process_id == 0 {
        return Some(false);
    }
    let process_id_arg = process_id.to_string();
    let output = Command::new("ps")
        .args(["-p", process_id_arg.as_str(), "-o", "pid="])
        .output()
        .ok()?;
    if !output.status.success() {
        return match output.status.code() {
            Some(1) => Some(false),
            _ => None,
        };
    }
    let process_ids = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.trim().parse::<u32>())
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    Some(process_ids.contains(&process_id))
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
pub fn process_id_is_running(_process_id: u32) -> Option<bool> {
    None
}

#[cfg(windows)]
pub fn install_watcher(launcher_path: &Path, debug_port: u16) -> anyhow::Result<()> {
    let plan = build_watcher_install_plan(launcher_path.to_path_buf(), debug_port);
    crate::windows_integration::set_current_user_string_value(
        WATCHER_RUN_KEY,
        &plan.run_value_name,
        &plan.run_value,
    )?;
    create_startup_shortcut(launcher_path, &plan.shortcut_arguments)?;
    spawn_launcher(launcher_path, debug_port);
    Ok(())
}

#[cfg(not(windows))]
pub fn install_watcher(_launcher_path: &Path, _debug_port: u16) -> anyhow::Result<()> {
    anyhow::bail!("watcher install is only supported on Windows")
}

#[cfg(windows)]
pub fn uninstall_watcher() -> anyhow::Result<()> {
    let _ =
        crate::windows_integration::delete_current_user_value(WATCHER_RUN_KEY, WATCHER_RUN_NAME);
    if let Some(shortcut) = startup_shortcut_path() {
        let _ = std::fs::remove_file(shortcut);
    }
    stop_launcher_processes();
    Ok(())
}

#[cfg(not(windows))]
pub fn uninstall_watcher() -> anyhow::Result<()> {
    Ok(())
}

#[cfg(windows)]
pub fn find_codex_processes() -> Vec<u32> {
    let processes: Vec<_> = crate::windows_integration::enumerate_processes()
        .into_iter()
        .filter(|process| crate::app_paths::is_supported_app_executable_name(&process.exe_file))
        .collect();
    find_codex_processes_from_snapshot(&processes)
}

/// Filter the list of already enumerated Windows processes for Codex processes.
/// Exposed so the Windows-specific logic can be unit-tested without scanning the live system.
#[cfg(windows)]
pub fn find_codex_processes_from_snapshot(
    processes: &[crate::windows_integration::WindowsProcessInfo],
) -> Vec<u32> {
    let mut ids = codex_process_ids(
        processes
            .iter()
            .filter_map(|process| {
                process
                    .executable_path
                    .as_deref()
                    .map(|path| (process.process_id, path.to_string_lossy().to_string()))
            })
            .collect::<Vec<_>>()
            .iter()
            .map(|(pid, path)| (*pid, path.as_str())),
    );

    // Local/portable installs use Codex.exe as the Electron main process. Do not match
    // lowercase codex.exe here; that is commonly the CLI binary. ChatGPT.exe is accepted
    // only for packaged Store apps above, because the standalone ChatGPT app can be a
    // normal ChatGPT session rather than Codex.
    for process in processes {
        if process.exe_file == "Codex.exe" {
            ids.push(process.process_id);
        }
    }

    ids.sort_unstable();
    ids.dedup();
    ids
}

/// Return desktop processes that can write Codex task state while a destructive
/// session-index cleanup is running. This is intentionally stricter than the
/// watcher filter: any supported ChatGPT desktop process blocks deletion,
/// including portable installs outside WindowsApps.
#[cfg(windows)]
pub fn find_session_index_cleanup_blocking_processes() -> Vec<u32> {
    find_session_index_cleanup_blocking_processes_from_snapshot(
        &crate::windows_integration::enumerate_processes(),
    )
}

#[cfg(windows)]
pub fn find_session_index_cleanup_blocking_processes_from_snapshot(
    processes: &[crate::windows_integration::WindowsProcessInfo],
) -> Vec<u32> {
    let mut ids = processes
        .iter()
        .filter(|process| process.exe_file == "Codex.exe" || process.exe_file == "ChatGPT.exe")
        .map(|process| process.process_id)
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids.dedup();
    ids
}

#[cfg(target_os = "macos")]
pub fn find_codex_processes() -> Vec<u32> {
    let Ok(output) = std::process::Command::new("ps")
        .args(["-axo", "pid=,args="])
        .output()
    else {
        return Vec::new();
    };
    macos_codex_process_ids(String::from_utf8_lossy(&output.stdout).lines())
}

#[cfg(target_os = "macos")]
pub fn find_session_index_cleanup_blocking_processes() -> Vec<u32> {
    find_codex_processes()
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn find_codex_processes() -> Vec<u32> {
    Vec::new()
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn find_session_index_cleanup_blocking_processes() -> Vec<u32> {
    Vec::new()
}

#[cfg(windows)]
pub fn stop_launcher_processes() {
    let processes = crate::windows_integration::enumerate_processes();
    let killable = filter_killable_launcher_processes(
        processes.iter().map(|process| {
            (
                process.process_id,
                process.parent_process_id,
                process.exe_file.as_str(),
            )
        }),
        std::process::id(),
    );
    for process_id in killable {
        let _ = crate::windows_integration::terminate_process(process_id);
    }
}

#[cfg(target_os = "macos")]
pub fn stop_launcher_processes() {
    for process_id in find_launcher_processes() {
        let _ = terminate_macos_process(process_id);
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn stop_launcher_processes() {}

#[cfg(windows)]
pub fn stop_launcher_processes_and_wait() {
    let processes = crate::windows_integration::enumerate_processes();
    let killable = filter_killable_launcher_processes(
        processes.iter().map(|process| {
            (
                process.process_id,
                process.parent_process_id,
                process.exe_file.as_str(),
            )
        }),
        std::process::id(),
    );
    terminate_and_wait_for_exit(
        killable,
        RESTART_STOP_WAIT_TIMEOUT_MS,
        RESTART_STOP_WAIT_INTERVAL_MS,
    );
}

#[cfg(windows)]
pub struct LauncherExitSnapshot {
    processes: Vec<(u32, u64)>,
}

#[cfg(windows)]
impl LauncherExitSnapshot {
    pub fn capture() -> anyhow::Result<Self> {
        let processes = crate::windows_integration::enumerate_processes();
        let ids = filter_killable_launcher_processes(
            processes.iter().map(|p| (p.process_id, p.parent_process_id, p.exe_file.as_str())),
            std::process::id(),
        );
        let mut captured = Vec::new();
        for pid in ids {
            if let Some(birth) = crate::windows_integration::process_birth_id(pid) {
                captured.push((pid, birth));
            } else {
                anyhow::bail!("Cannot verify launcher process identity: {pid}");
            }
        }
        Ok(Self { processes: captured })
    }

    pub fn wait_for_exit_or_force(self, timeout: Duration) -> LauncherExitOutcome {
        let outcome = wait_for_launcher_exit_with(
            &self.processes,
            timeout,
            |pid| crate::windows_integration::process_birth_id(pid),
            std::time::Instant::now,
            |pid| {
                let _ = crate::windows_integration::terminate_process(pid);
            },
            |duration| std::thread::sleep(duration),
        );
        if !outcome.forced_process_ids.is_empty() {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "watcher.launcher_force_terminated",
                serde_json::json!({
                    "process_ids": outcome.forced_process_ids,
                    "timeout_ms": timeout.as_millis() as u64,
                    "still_running": outcome.still_running
                }),
            );
        }
        outcome
    }
}

/// 「等待旧启动器退出」的结果。
///
/// 旧实现超时后直接抛错，既不结束任何进程也不启动新实例，用户侧表现为
/// 「重启失败后没有任何可用实例，还要手工结束进程」（issue #2403）。
#[cfg(any(windows, test))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LauncherExitOutcome {
    /// 超时后被强制结束的启动器进程；旧启动器自行退出时为空。
    pub forced_process_ids: Vec<u32>,
    /// 强制结束后是否仍有残留启动器未退出。
    pub still_running: bool,
}

#[cfg(any(windows, test))]
impl LauncherExitOutcome {
    /// 旧启动器是否已确认全部退出（可以安全启动新实例）。
    pub fn is_clean(&self) -> bool {
        !self.still_running
    }
}

/// 等旧启动器退出的轮询间隔：起步 [`RESTART_STOP_WAIT_INTERVAL_MS`]，
/// 逐次翻倍到上限 [`LAUNCHER_WAIT_BACKOFF_MAX_MS`]。
#[cfg(any(windows, test))]
fn launcher_wait_interval_ms(attempt: u32) -> u64 {
    RESTART_STOP_WAIT_INTERVAL_MS
        .saturating_mul(1u64 << attempt.min(8))
        .min(LAUNCHER_WAIT_BACKOFF_MAX_MS)
}

/// 等待旧启动器退出；超时则强制结束残留实例。
///
/// 所有副作用（读进程身份、结束进程、取时间、休眠）都通过参数注入，
/// 便于在非 Windows 平台上用替代实现做单元测试。
#[cfg(any(windows, test))]
fn wait_for_launcher_exit_with(
    captured: &[(u32, u64)],
    timeout: Duration,
    mut birth_id: impl FnMut(u32) -> Option<u64>,
    mut now: impl FnMut() -> std::time::Instant,
    mut terminate: impl FnMut(u32),
    mut sleep: impl FnMut(Duration),
) -> LauncherExitOutcome {
    let start = now();
    let mut attempt: u32 = 0;

    // 阶段一：先给旧启动器自己退出的机会。带退避轮询，避免 10 秒里打满 CPU。
    loop {
        if launcher_incarnations_still_running(captured, &mut birth_id).is_empty() {
            return LauncherExitOutcome {
                forced_process_ids: Vec::new(),
                still_running: false,
            };
        }
        let waited = now().saturating_duration_since(start);
        if waited >= timeout {
            break;
        }
        let remaining_ms = timeout.saturating_sub(waited).as_millis().max(1) as u64;
        sleep(Duration::from_millis(
            launcher_wait_interval_ms(attempt).min(remaining_ms),
        ));
        attempt += 1;
    }

    // 阶段二：超时未退出，强制结束残留启动器。只结束「进程身份仍与快照一致」的
    // 实例——pid 可能已被系统回收给别的进程，身份复核是避免误杀的关键。
    let remaining = launcher_incarnations_still_running(captured, &mut birth_id);
    if remaining.is_empty() {
        // 恰好在超时边界上退出了，属于自行退出。
        return LauncherExitOutcome {
            forced_process_ids: Vec::new(),
            still_running: false,
        };
    }
    let mut forced_process_ids = Vec::new();
    for process_id in remaining {
        terminate(process_id);
        forced_process_ids.push(process_id);
    }

    // 阶段三：确认强制结束是否生效，仍有残留就如实上报，不再假装成功。
    let grace_start = now();
    loop {
        if launcher_incarnations_still_running(captured, &mut birth_id).is_empty() {
            return LauncherExitOutcome {
                forced_process_ids,
                still_running: false,
            };
        }
        if now().saturating_duration_since(grace_start)
            >= Duration::from_millis(LAUNCHER_FORCE_KILL_GRACE_MS)
        {
            return LauncherExitOutcome {
                forced_process_ids,
                still_running: true,
            };
        }
        sleep(Duration::from_millis(RESTART_STOP_WAIT_INTERVAL_MS));
    }
}

#[cfg(any(windows, test))]
fn launcher_incarnations_still_running(
    captured: &[(u32, u64)],
    mut birth_id: impl FnMut(u32) -> Option<u64>,
) -> Vec<u32> {
    captured.iter().filter_map(|(pid, birth)| {
        (birth_id(*pid) == Some(*birth)).then_some(*pid)
    }).collect()
}

#[cfg(test)]
mod launcher_exit_tests {
    use super::{
        LAUNCHER_FORCE_KILL_GRACE_MS, LAUNCHER_WAIT_BACKOFF_MAX_MS, RESTART_STOP_WAIT_INTERVAL_MS,
        launcher_incarnations_still_running, launcher_wait_interval_ms,
        wait_for_launcher_exit_with,
    };
    use std::cell::{Cell, RefCell};
    use std::time::{Duration, Instant};

    #[test]
    fn only_the_captured_launcher_incarnation_is_waited_for() {
        let captured = [(10, 100), (20, 200), (30, 300)];
        let remaining = launcher_incarnations_still_running(&captured, |pid| match pid {
            10 => Some(100),
            20 => Some(999),
            _ => None,
        });
        assert_eq!(remaining, [10]);
    }

    #[test]
    fn launcher_wait_backoff_grows_then_plateaus() {
        assert_eq!(launcher_wait_interval_ms(0), RESTART_STOP_WAIT_INTERVAL_MS);
        assert_eq!(
            launcher_wait_interval_ms(1),
            RESTART_STOP_WAIT_INTERVAL_MS * 2
        );
        assert_eq!(
            launcher_wait_interval_ms(2),
            RESTART_STOP_WAIT_INTERVAL_MS * 4
        );
        // 退避必须封顶，否则长等待会睡过头。
        assert_eq!(launcher_wait_interval_ms(30), LAUNCHER_WAIT_BACKOFF_MAX_MS);
    }

    /// 旧启动器在超时前自行退出：不该强制结束任何进程。
    #[test]
    fn launcher_exiting_on_its_own_is_never_force_killed() {
        let clock = Cell::new(Instant::now());
        let terminated = RefCell::new(Vec::new());
        let captured = [(10, 100)];
        // 第一次查询已判定退出。
        let alive = Cell::new(false);

        let outcome = wait_for_launcher_exit_with(
            &captured,
            Duration::from_secs(10),
            |_| alive.get().then_some(100),
            || clock.get(),
            |pid| terminated.borrow_mut().push(pid),
            |duration| clock.set(clock.get() + duration),
        );

        assert_eq!(outcome.forced_process_ids, Vec::<u32>::new());
        assert!(outcome.is_clean());
        assert!(terminated.borrow().is_empty());
    }

    /// issue #2403：超时后必须强制结束残留启动器，并如实报告结果。
    #[test]
    fn stuck_launcher_is_force_killed_after_timeout() {
        let clock = Cell::new(Instant::now());
        let terminated = RefCell::new(Vec::new());
        let captured = [(10, 100), (20, 200)];
        // 两个实例在超时前都不肯退出，被强制结束后才消失。
        let killed = RefCell::new(Vec::new());

        let outcome = wait_for_launcher_exit_with(
            &captured,
            Duration::from_millis(500),
            |pid| match pid {
                10 if !killed.borrow().contains(&10) => Some(100),
                20 if !killed.borrow().contains(&20) => Some(200),
                _ => None,
            },
            || clock.get(),
            |pid| {
                terminated.borrow_mut().push(pid);
                killed.borrow_mut().push(pid);
            },
            |duration| clock.set(clock.get() + duration),
        );

        // 两个残留实例都被强制结束，且等待期间带退避而不是忙等。
        assert_eq!(terminated.borrow().as_slice(), &[10, 20]);
        assert_eq!(outcome.forced_process_ids, vec![10, 20]);
        assert!(outcome.is_clean());
    }

    /// 强制结束也失败时，必须如实上报 still_running，不能假装成功。
    #[test]
    fn force_kill_that_does_not_take_effect_reports_still_running() {
        let clock = Cell::new(Instant::now());
        let captured = [(10, 100)];
        let kill_attempts = Cell::new(0u32);

        let outcome = wait_for_launcher_exit_with(
            &captured,
            Duration::from_millis(500),
            |_| Some(100),
            || clock.get(),
            |_| kill_attempts.set(kill_attempts.get() + 1),
            |duration| clock.set(clock.get() + duration),
        );

        assert_eq!(kill_attempts.get(), 1);
        assert_eq!(outcome.forced_process_ids, vec![10]);
        assert!(outcome.still_running);
        assert!(!outcome.is_clean());
    }

    /// pid 已被系统回收给别的进程时，身份复核必须拦住误杀。
    #[test]
    fn recycled_pid_is_not_force_killed() {
        let clock = Cell::new(Instant::now());
        let terminated = RefCell::new(Vec::new());
        let captured = [(10, 100)];

        let outcome = wait_for_launcher_exit_with(
            &captured,
            Duration::from_millis(500),
            // 记录的 10 号实例已经不在，当前 10 号是别人。
            |_| Some(999),
            || clock.get(),
            |pid| terminated.borrow_mut().push(pid),
            |duration| clock.set(clock.get() + duration),
        );

        assert!(terminated.borrow().is_empty());
        assert_eq!(outcome.forced_process_ids, Vec::<u32>::new());
        assert!(outcome.is_clean());
    }

    /// 强制结束后的确认窗口有上限，不会无限等待。
    #[test]
    fn force_kill_confirmation_window_is_bounded() {
        let start = Instant::now();
        let clock = Cell::new(start);
        let captured = [(10, 100)];

        wait_for_launcher_exit_with(
            &captured,
            Duration::from_millis(500),
            |_| Some(100),
            || clock.get(),
            |_| {},
            |duration| clock.set(clock.get() + duration),
        );

        // 500ms 等待 + 2s 确认窗口，加上退避的整段开销；有上限即可，不能是无限循环。
        let elapsed = clock.get().saturating_duration_since(start);
        assert!(
            elapsed >= Duration::from_millis(500),
            "至少应等满传入的超时时间，实际 {elapsed:?}"
        );
        assert!(
            elapsed
                < Duration::from_millis(500)
                    + Duration::from_millis(LAUNCHER_FORCE_KILL_GRACE_MS)
                    + Duration::from_secs(1),
            "确认窗口必须封顶，实际 {elapsed:?}"
        );
    }
}

#[cfg(target_os = "macos")]
pub fn stop_launcher_processes_and_wait() {
    terminate_macos_processes_and_wait(
        find_launcher_processes(),
        || find_launcher_processes(),
        RESTART_STOP_WAIT_TIMEOUT_MS,
        RESTART_STOP_WAIT_INTERVAL_MS,
    );
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn stop_launcher_processes_and_wait() {}

#[cfg(windows)]
pub fn stop_codex_processes() {
    for process_id in find_codex_processes() {
        let _ = crate::windows_integration::terminate_process(process_id);
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn stop_codex_processes() {}

#[cfg(target_os = "macos")]
pub fn stop_codex_processes() {
    for process_id in find_codex_processes() {
        let _ = terminate_macos_process(process_id);
    }
}

#[cfg(windows)]
pub fn stop_codex_processes_and_wait() {
    terminate_and_wait_for_exit(
        find_codex_processes(),
        RESTART_STOP_WAIT_TIMEOUT_MS,
        RESTART_STOP_WAIT_INTERVAL_MS,
    );
}

#[cfg(target_os = "macos")]
pub fn stop_codex_processes_and_wait() {
    terminate_macos_processes_and_wait(
        find_codex_processes(),
        || find_codex_processes(),
        RESTART_STOP_WAIT_TIMEOUT_MS,
        RESTART_STOP_WAIT_INTERVAL_MS,
    );
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn stop_codex_processes_and_wait() {}

#[cfg(target_os = "macos")]
pub fn find_codex_processes_for_debug_port(debug_port: u16) -> Vec<u32> {
    find_macos_codex_processes_for_debug_port(debug_port)
}

#[cfg(not(target_os = "macos"))]
pub fn find_codex_processes_for_debug_port(_debug_port: u16) -> Vec<u32> {
    find_codex_processes()
}

#[cfg(target_os = "macos")]
pub fn stop_codex_processes_for_debug_port_and_wait(debug_port: u16) {
    terminate_macos_processes_and_wait(
        find_macos_codex_processes_for_debug_port(debug_port),
        || find_macos_codex_processes_for_debug_port(debug_port),
        RESTART_STOP_WAIT_TIMEOUT_MS,
        RESTART_STOP_WAIT_INTERVAL_MS,
    );
}

#[cfg(not(target_os = "macos"))]
pub fn stop_codex_processes_for_debug_port_and_wait(_debug_port: u16) {
    stop_codex_processes_and_wait();
}

#[cfg(target_os = "macos")]
fn terminate_macos_processes_and_wait<F>(
    process_ids: Vec<u32>,
    mut find_processes: F,
    timeout_ms: u64,
    interval_ms: u64,
) where
    F: FnMut() -> Vec<u32>,
{
    if process_ids.is_empty() {
        return;
    }
    for process_id in &process_ids {
        let _ = terminate_macos_process(*process_id);
    }
    let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        let remaining = process_ids_still_running(&process_ids, find_processes());
        if remaining.is_empty() || std::time::Instant::now() >= deadline {
            if !remaining.is_empty() {
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "watcher.stop_wait_timeout",
                    serde_json::json!({
                        "remaining_process_ids": remaining,
                        "timeout_ms": timeout_ms,
                        "platform": "macos"
                    }),
                );
            }
            break;
        }
        std::thread::sleep(Duration::from_millis(interval_ms));
    }
}

#[cfg(target_os = "macos")]
fn terminate_macos_process(process_id: u32) -> std::io::Result<()> {
    Command::new("kill")
        .arg(process_id.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|_| ())
}

#[cfg(target_os = "macos")]
fn find_launcher_processes() -> Vec<u32> {
    let current_process_id = std::process::id();
    let mut process_ids = macos_launcher_process_names()
        .into_iter()
        .flat_map(|process_name| {
            std::process::Command::new("pgrep")
                .args(["-x", process_name])
                .output()
                .ok()
                .into_iter()
                .flat_map(|output| {
                    String::from_utf8_lossy(&output.stdout)
                        .lines()
                        .filter_map(|value| value.trim().parse::<u32>().ok())
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        })
        .filter(|process_id| *process_id != current_process_id)
        .collect::<Vec<_>>();
    process_ids.sort_unstable();
    process_ids.dedup();
    process_ids
}

#[cfg(target_os = "macos")]
pub(crate) fn find_macos_codex_processes_for_debug_port(debug_port: u16) -> Vec<u32> {
    let Ok(output) = std::process::Command::new("ps")
        .args(["-axo", "pid=,args="])
        .output()
    else {
        return Vec::new();
    };
    macos_codex_process_ids_for_debug_port(
        String::from_utf8_lossy(&output.stdout).lines(),
        debug_port,
    )
}

#[cfg(target_os = "macos")]
fn macos_codex_process_ids_for_debug_port<'a>(
    process_lines: impl IntoIterator<Item = &'a str>,
    debug_port: u16,
) -> Vec<u32> {
    let debug_flag = format!("remote-debugging-port={debug_port}");
    let mut ids = process_lines
        .into_iter()
        .filter_map(|line| {
            let trimmed = line.trim_start();
            let (pid, args) = trimmed.split_once(char::is_whitespace)?;
            let process_id = pid.parse::<u32>().ok()?;
            let is_desktop_main = is_macos_codex_desktop_main(args);
            (is_desktop_main && args.contains(&debug_flag)).then_some(process_id)
        })
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids.dedup();
    ids
}

#[cfg(target_os = "macos")]
fn macos_codex_process_ids<'a>(process_lines: impl IntoIterator<Item = &'a str>) -> Vec<u32> {
    macos_codex_process_ids_with(process_lines, read_macos_bundle_identifier)
}

#[cfg(any(target_os = "macos", test))]
fn macos_codex_process_ids_with<'a>(
    process_lines: impl IntoIterator<Item = &'a str>,
    bundle_identifier: impl Fn(&Path) -> Option<String>,
) -> Vec<u32> {
    let mut ids = process_lines
        .into_iter()
        .filter_map(|line| {
            let trimmed = line.trim_start();
            let (pid, args) = trimmed.split_once(char::is_whitespace)?;
            let process_id = pid.parse::<u32>().ok()?;
            is_macos_codex_desktop_main_with(args, &bundle_identifier).then_some(process_id)
        })
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids.dedup();
    ids
}

#[cfg(target_os = "macos")]
fn is_macos_codex_desktop_main(args: &str) -> bool {
    is_macos_codex_desktop_main_with(args, read_macos_bundle_identifier)
}

/// 读 `.app` 内 `Contents/Info.plist` 的 `CFBundleIdentifier`。
///
/// `app_paths` 里同名函数是私有的，跨模块用不了；这里保留一份等价的局部实现。
/// 只解析文本 plist：macOS 上 OpenAI 官方 App 都是 XML plist，若遇到二进制 plist
/// （以 `bplist00` 开头）会读不到标识符，此时退回到可执行名判据，不会更宽松。
#[cfg(target_os = "macos")]
fn read_macos_bundle_identifier(app_dir: &Path) -> Option<String> {
    let plist = std::fs::read_to_string(app_dir.join("Contents").join("Info.plist")).ok()?;
    plist_string_value(&plist, "CFBundleIdentifier")
}

/// 从 plist 文本里抠出某个 key 对应的首个 `<string>` 值。
#[cfg(any(target_os = "macos", test))]
fn plist_string_value(plist: &str, key: &str) -> Option<String> {
    let (_, after_key) = plist.split_once(&format!("<key>{key}</key>"))?;
    let (_, after_open) = after_key.split_once("<string>")?;
    let (value, _) = after_open.split_once("</string>")?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

/// ChatGPT 桌面版的正式 bundle id；`Codex Dev` 等衍生构建使用其点分子标识符。
#[cfg(any(target_os = "macos", test))]
const MACOS_CODEX_BUNDLE_IDENTIFIER: &str = "com.openai.codex";

/// bundle id 是否指向 Codex 桌面应用。用点分子标识符判定，避免把
/// `com.openai.chat`（ChatGPT Classic）、`com.openai.chatgpt` 这类同名可执行
/// 文件的产品误判成 Codex（issue #2222）。
#[cfg(any(target_os = "macos", test))]
fn is_macos_codex_bundle_identifier(identifier: &str) -> bool {
    identifier == MACOS_CODEX_BUNDLE_IDENTIFIER
        || identifier
            .strip_prefix(MACOS_CODEX_BUNDLE_IDENTIFIER)
            .is_some_and(|suffix| suffix.starts_with('.'))
}

#[cfg(any(target_os = "macos", test))]
fn is_macos_codex_desktop_main_with(
    args: &str,
    bundle_identifier: impl Fn(&Path) -> Option<String>,
) -> bool {
    let args = args.trim_start();
    if !args.starts_with('/') || args.contains("/Helpers/") {
        return false;
    }

    let executable_end = args.find(" -").unwrap_or(args.len());
    let executable = args[..executable_end].trim_end();
    let Some((app_dir, executable_name)) = executable.rsplit_once(".app/Contents/MacOS/") else {
        return false;
    };
    let app_dir = format!("{app_dir}.app");

    // 一级判据：`.app` 的 bundle id。读得到就以此为准——`ChatGPT.app`（Codex 桌面版）
    // 与 `ChatGPT Classic.app` 的主可执行文件都叫 `ChatGPT`，只有 bundle id 能区分。
    if let Some(identifier) = bundle_identifier(Path::new(&app_dir)) {
        return is_macos_codex_bundle_identifier(&identifier);
    }

    // 二级判据：读不到 bundle id 时才退回可执行名。
    matches!(
        executable_name,
        "Codex" | "Codex Dev" | "ChatGPT" | "ChatGPT Dev"
    )
}

#[cfg(windows)]
fn terminate_and_wait_for_exit(process_ids: Vec<u32>, timeout_ms: u64, interval_ms: u64) {
    if process_ids.is_empty() {
        return;
    }
    for process_id in &process_ids {
        let _ = crate::windows_integration::terminate_process(*process_id);
    }
    let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        let running_process_ids = crate::windows_integration::enumerate_processes()
            .into_iter()
            .map(|process| process.process_id);
        let remaining = process_ids_still_running(&process_ids, running_process_ids);
        if remaining.is_empty() || std::time::Instant::now() >= deadline {
            if !remaining.is_empty() {
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "watcher.stop_wait_timeout",
                    serde_json::json!({
                        "remaining_process_ids": remaining,
                        "timeout_ms": timeout_ms
                    }),
                );
            }
            break;
        }
        std::thread::sleep(Duration::from_millis(interval_ms));
    }
}

#[cfg(windows)]
fn create_startup_shortcut(launcher_path: &Path, arguments: &str) -> anyhow::Result<()> {
    let Some(shortcut_path) = startup_shortcut_path() else {
        anyhow::bail!("无法定位 Windows 启动目录")
    };
    crate::windows_integration::create_shortcut(&crate::windows_integration::ShortcutSpec {
        path: shortcut_path,
        target: launcher_path.to_path_buf(),
        arguments: arguments.to_string(),
        working_directory: launcher_path.parent().map(Path::to_path_buf),
        description: "Codex++ watcher".to_string(),
        icon: None,
        show_minimized: true,
    })
}

#[cfg(windows)]
fn spawn_launcher(launcher_path: &Path, debug_port: u16) {
    let command = build_spawn_launcher_command(&launcher_path.to_string_lossy(), debug_port);
    if let Some((exe, args)) = command.split_first() {
        let mut command = Command::new(exe);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        use std::os::windows::process::CommandExt;
        command.creation_flags(crate::windows_integration::CREATE_NO_WINDOW);
        let _ = command.spawn();
    }
}

#[cfg(windows)]
fn startup_shortcut_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|appdata| {
        PathBuf::from(appdata)
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs")
            .join("Startup")
            .join(WATCHER_STARTUP_SHORTCUT_NAME)
    })
}
