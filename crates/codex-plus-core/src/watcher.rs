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
/// macOS 正常退出超时后，等待强制终止完成的最长时间。
#[cfg(target_os = "macos")]
const MACOS_FORCE_STOP_WAIT_TIMEOUT_MS: u64 = 1_000;
/// macOS 上 Codex 桌面应用主进程可能使用的名称。
#[cfg(target_os = "macos")]
const MACOS_CODEX_PROCESS_NAMES: &[&str] = &["Codex", "ChatGPT"];
/// macOS 安装包与本地调试构建使用的静默启动器进程名。
#[cfg(target_os = "macos")]
const MACOS_LAUNCHER_PROCESS_NAMES: &[&str] = &["CodexPlusPlus", "codex-plus-plus"];
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

/// 查找 macOS 上正在运行的 Codex 桌面应用主进程。
#[cfg(target_os = "macos")]
pub fn find_codex_processes() -> Vec<u32> {
    find_macos_processes_by_names(MACOS_CODEX_PROCESS_NAMES)
}

/// 从 macOS `ps -axo pid=,ucomm=` 输出中筛选指定名称的进程。
///
/// 使用 `ucomm` 而不是 `pgrep -x`，避免应用包内较长的可执行路径被系统截断后无法匹配。
#[cfg(target_os = "macos")]
pub fn macos_process_ids_from_ps_output(output: &str, process_names: &[&str]) -> Vec<u32> {
    let supported_names = process_names.iter().copied().collect::<HashSet<_>>();
    let mut process_ids = output
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let split_at = line.find(char::is_whitespace)?;
            let process_id = line[..split_at].parse::<u32>().ok()?;
            let process_name = line[split_at..].trim();
            supported_names.contains(process_name).then_some(process_id)
        })
        .collect::<Vec<_>>();
    process_ids.sort_unstable();
    process_ids.dedup();
    process_ids
}

/// 查询 macOS 当前进程并按可执行文件名精确筛选。
#[cfg(target_os = "macos")]
fn find_macos_processes_by_names(process_names: &[&str]) -> Vec<u32> {
    match Command::new("ps").args(["-axo", "pid=,ucomm="]).output() {
        Ok(output) if output.status.success() => macos_process_ids_from_ps_output(
            &String::from_utf8_lossy(&output.stdout),
            process_names,
        ),
        Ok(output) => {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "watcher.macos_process_scan_failed",
                serde_json::json!({
                    "status": output.status.code(),
                    "stderr": String::from_utf8_lossy(&output.stderr).trim()
                }),
            );
            Vec::new()
        }
        Err(error) => {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "watcher.macos_process_scan_failed",
                serde_json::json!({ "error": error.to_string() }),
            );
            Vec::new()
        }
    }
}

/// 查找可安全停止的 macOS 静默启动器，并保护当前正在执行清理的启动器实例。
#[cfg(target_os = "macos")]
fn find_killable_macos_launcher_processes() -> Vec<u32> {
    exclude_current_process_id(
        find_macos_processes_by_names(MACOS_LAUNCHER_PROCESS_NAMES),
        std::process::id(),
    )
}

/// 从待停止列表中排除当前进程，避免启动器在恢复旧实例时结束自身。
#[cfg(target_os = "macos")]
fn exclude_current_process_id(process_ids: Vec<u32>, current_process_id: u32) -> Vec<u32> {
    process_ids
        .into_iter()
        .filter(|process_id| *process_id != current_process_id)
        .collect()
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

/// 停止 macOS 上旧的静默启动器，避免新启动器因单实例锁而退化为“激活现有窗口”。
#[cfg(target_os = "macos")]
pub fn stop_launcher_processes() {
    terminate_macos_processes(find_killable_macos_launcher_processes());
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

/// 停止并等待 macOS 静默启动器退出，确保单实例锁已经释放。
#[cfg(target_os = "macos")]
pub fn stop_launcher_processes_and_wait() {
    terminate_macos_processes_and_wait(
        find_killable_macos_launcher_processes(),
        "launcher",
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

/// 停止 macOS 上的 Codex 桌面应用主进程。
#[cfg(target_os = "macos")]
pub fn stop_codex_processes() {
    terminate_macos_processes(find_codex_processes());
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn stop_codex_processes() {}

#[cfg(windows)]
pub fn stop_codex_processes_and_wait() {
    terminate_and_wait_for_exit(
        find_codex_processes(),
        RESTART_STOP_WAIT_TIMEOUT_MS,
        RESTART_STOP_WAIT_INTERVAL_MS,
    );
}

/// 停止并等待 macOS Codex 桌面应用退出，避免后续 `open` 仅激活旧窗口。
#[cfg(target_os = "macos")]
pub fn stop_codex_processes_and_wait() {
    terminate_macos_processes_and_wait(
        find_codex_processes(),
        "codex",
        RESTART_STOP_WAIT_TIMEOUT_MS,
        RESTART_STOP_WAIT_INTERVAL_MS,
    );
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn stop_codex_processes_and_wait() {}

/// 向 macOS 目标进程发送正常终止信号。
#[cfg(target_os = "macos")]
fn terminate_macos_processes(process_ids: Vec<u32>) {
    send_macos_signal(&process_ids, "-TERM", "term");
}

/// 正常终止 macOS 进程并等待退出；超时后使用强制终止，保证重启可以继续。
#[cfg(target_os = "macos")]
fn terminate_macos_processes_and_wait(
    process_ids: Vec<u32>,
    process_kind: &str,
    timeout_ms: u64,
    interval_ms: u64,
) {
    if process_ids.is_empty() {
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "watcher.macos_stop_skipped",
            serde_json::json!({ "process_kind": process_kind, "reason": "not_running" }),
        );
        return;
    }
    let _ = crate::diagnostic_log::append_diagnostic_log(
        "watcher.macos_stop_requested",
        serde_json::json!({
            "process_kind": process_kind,
            "process_ids": process_ids,
            "timeout_ms": timeout_ms
        }),
    );
    terminate_macos_processes(process_ids.clone());
    let remaining = wait_for_macos_processes_to_exit(&process_ids, timeout_ms, interval_ms);
    if remaining.is_empty() {
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "watcher.macos_stop_completed",
            serde_json::json!({ "process_kind": process_kind, "forced": false }),
        );
        return;
    }

    let _ = crate::diagnostic_log::append_diagnostic_log(
        "watcher.stop_wait_timeout",
        serde_json::json!({
            "process_kind": process_kind,
            "remaining_process_ids": remaining,
            "timeout_ms": timeout_ms
        }),
    );
    send_macos_signal(&remaining, "-KILL", "kill");
    let remaining_after_force =
        wait_for_macos_processes_to_exit(&remaining, MACOS_FORCE_STOP_WAIT_TIMEOUT_MS, interval_ms);
    let _ = crate::diagnostic_log::append_diagnostic_log(
        "watcher.macos_stop_completed",
        serde_json::json!({
            "process_kind": process_kind,
            "forced": true,
            "remaining_process_ids": remaining_after_force
        }),
    );
}

/// 调用系统 `kill` 命令向指定 macOS 进程发送信号，并记录失败详情。
#[cfg(target_os = "macos")]
fn send_macos_signal(process_ids: &[u32], signal: &str, signal_name: &str) {
    for process_id in process_ids {
        let result = Command::new("kill")
            .args([signal, &process_id.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let succeeded = result.as_ref().is_ok_and(|status| status.success());
        if !succeeded {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "watcher.macos_signal_failed",
                serde_json::json!({
                    "process_id": process_id,
                    "signal": signal_name,
                    "status": result.as_ref().ok().and_then(|status| status.code()),
                    "error": result.as_ref().err().map(|error| error.to_string())
                }),
            );
        }
    }
}

/// 等待指定 macOS 进程退出，并返回超时后仍存活的进程 ID。
#[cfg(target_os = "macos")]
fn wait_for_macos_processes_to_exit(
    process_ids: &[u32],
    timeout_ms: u64,
    interval_ms: u64,
) -> Vec<u32> {
    let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        let remaining = process_ids
            .iter()
            .copied()
            .filter(|process_id| macos_process_is_running(*process_id))
            .collect::<Vec<_>>();
        if remaining.is_empty() || std::time::Instant::now() >= deadline {
            return remaining;
        }
        std::thread::sleep(Duration::from_millis(interval_ms));
    }
}

/// 使用无副作用的信号探测指定 macOS 进程是否仍然存活。
#[cfg(target_os = "macos")]
fn macos_process_is_running(process_id: u32) -> bool {
    Command::new("kill")
        .args(["-0", &process_id.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
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

#[cfg(all(test, target_os = "macos"))]
mod macos_tests {
    use super::{exclude_current_process_id, terminate_macos_processes_and_wait};

    /// 验证旧启动器清理列表会保留当前正在执行恢复逻辑的进程。
    #[test]
    fn launcher_cleanup_excludes_current_process() {
        assert_eq!(
            exclude_current_process_id(vec![10, 20, 30], 20),
            vec![10, 30]
        );
    }

    /// 验证 macOS 重启停止逻辑会真实结束目标进程，而不是只等待或激活窗口。
    #[test]
    fn terminate_and_wait_stops_target_process() {
        let mut child = std::process::Command::new("/bin/sleep")
            .arg("30")
            .spawn()
            .expect("应能启动用于验证的临时进程");
        let process_id = child.id();
        let reaper = std::thread::spawn(move || child.wait().expect("应能回收临时进程"));

        terminate_macos_processes_and_wait(vec![process_id], "test", 1_000, 25);

        assert!(
            !reaper.join().expect("临时进程回收线程不应失败").success(),
            "目标进程应已退出"
        );
    }
}
