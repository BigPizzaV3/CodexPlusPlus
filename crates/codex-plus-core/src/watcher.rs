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
pub const WATCHER_RUN_NAME: &str = "CodexPlusPlusWatcher";
pub const WATCHER_RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
pub const WATCHER_STARTUP_SHORTCUT_NAME: &str = "CodexPlusPlusWatcher.lnk";
#[cfg(any(test, target_os = "macos"))]
const LEGACY_MACOS_LAUNCHER_BINARY: &str = "CodexPlusPlus";

#[cfg(any(test, target_os = "macos"))]
fn macos_launcher_binary_names() -> [&'static str; 2] {
    [crate::install::SILENT_BINARY, LEGACY_MACOS_LAUNCHER_BINARY]
}

#[cfg(any(test, target_os = "macos"))]
fn macos_launcher_process_ids_from_pgrep_outputs(
    outputs: impl IntoIterator<Item = Vec<u8>>,
) -> Vec<u32> {
    let mut process_ids = outputs
        .into_iter()
        .flat_map(|output| {
            String::from_utf8_lossy(&output)
                .lines()
                .filter_map(|value| value.trim().parse::<u32>().ok())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    process_ids.sort_unstable();
    process_ids.dedup();
    process_ids
}

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

    #[test]
    fn macos_launcher_names_preserve_current_and_legacy_binaries() {
        assert_eq!(
            macos_launcher_binary_names(),
            ["codex-plus-plus", "CodexPlusPlus"]
        );
    }

    #[test]
    fn macos_launcher_process_ids_merge_names_and_deduplicate() {
        let process_ids = macos_launcher_process_ids_from_pgrep_outputs([
            b"20\n10\n".to_vec(),
            b"30\n20\ninvalid\n".to_vec(),
        ]);

        assert_eq!(process_ids, vec![10, 20, 30]);
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

    #[cfg(windows)]
    fn windows_process(
        process_id: u32,
        parent_process_id: u32,
        exe_file: &str,
        executable_path: &str,
    ) -> WindowsProcessInfo {
        WindowsProcessInfo {
            process_id,
            parent_process_id,
            exe_file: exe_file.to_string(),
            executable_path: Some(PathBuf::from(executable_path)),
        }
    }

    #[cfg(windows)]
    #[test]
    fn targeted_stop_selects_only_the_listener_codex_process_tree() {
        let target_root = windows_process(
            10,
            1,
            "ChatGPT.exe",
            r"C:\Program Files\WindowsApps\OpenAI.Codex_26.820.7780.0_x64__2p2nqsd0c76g0\app\ChatGPT.exe",
        );
        let target_child = windows_process(
            11,
            10,
            "ChatGPT.exe",
            r"C:\Program Files\WindowsApps\OpenAI.Codex_26.820.7780.0_x64__2p2nqsd0c76g0\app\ChatGPT.exe",
        );
        let target_helper = windows_process(
            12,
            11,
            "codex-code-mode-host.exe",
            r"C:\Program Files\WindowsApps\OpenAI.Codex_26.820.7780.0_x64__2p2nqsd0c76g0\app\codex-code-mode-host.exe",
        );
        let unrelated_root = windows_process(
            20,
            1,
            "ChatGPT.exe",
            r"C:\Program Files\WindowsApps\OpenAI.Codex_26.821.1.0_x64__2p2nqsd0c76g0\app\ChatGPT.exe",
        );
        let unrelated_child = windows_process(
            21,
            20,
            "ChatGPT.exe",
            r"C:\Program Files\WindowsApps\OpenAI.Codex_26.821.1.0_x64__2p2nqsd0c76g0\app\ChatGPT.exe",
        );

        let selected = target_codex_process_tree_from_snapshot(
            &[
                target_root,
                target_child,
                target_helper,
                unrelated_root,
                unrelated_child,
            ],
            &[11],
            &HashMap::from([(10, 100), (11, 110), (12, 120), (20, 200), (21, 210)]),
        );

        assert_eq!(selected, vec![12, 11, 10]);
    }

    #[cfg(windows)]
    #[test]
    fn targeted_stop_refuses_an_untrusted_listener_owner() {
        let unrelated = windows_process(30, 1, "other.exe", r"C:\Tools\other.exe");

        let selected = target_codex_process_tree_from_snapshot(
            &[unrelated],
            &[30],
            &HashMap::from([(30, 300)]),
        );

        assert!(selected.is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn targeted_stop_does_not_follow_a_newer_reused_parent() {
        let parent = windows_process(
            10,
            1,
            "ChatGPT.exe",
            r"C:\Program Files\WindowsApps\OpenAI.Codex_26.820.7780.0_x64__2p2nqsd0c76g0\app\ChatGPT.exe",
        );
        let owner = windows_process(
            11,
            10,
            "ChatGPT.exe",
            r"C:\Program Files\WindowsApps\OpenAI.Codex_26.820.7780.0_x64__2p2nqsd0c76g0\app\ChatGPT.exe",
        );

        let selected = target_codex_process_tree_from_snapshot(
            &[parent, owner],
            &[11],
            &HashMap::from([(10, 120), (11, 110)]),
        );

        assert_eq!(selected, vec![11]);
    }

    #[cfg(windows)]
    #[test]
    fn targeted_stop_refuses_incomplete_same_package_birth_identity() {
        let root = windows_process(
            10,
            1,
            "ChatGPT.exe",
            r"C:\Program Files\WindowsApps\OpenAI.Codex_26.820.7780.0_x64__2p2nqsd0c76g0\app\ChatGPT.exe",
        );
        let child = windows_process(
            11,
            10,
            "ChatGPT.exe",
            r"C:\Program Files\WindowsApps\OpenAI.Codex_26.820.7780.0_x64__2p2nqsd0c76g0\app\ChatGPT.exe",
        );
        let identity_root = process_identity_root(root.executable_path.as_deref().unwrap());

        let result = collect_identity_birth_ids(&[root, child], &identity_root, |process_id| {
            (process_id == 10).then_some(100)
        });

        assert_eq!(result, Err(11));
    }

    #[cfg(windows)]
    #[test]
    fn targeted_stop_rejects_another_listener_owner_appearing() {
        assert!(listener_owners_unchanged(&[10], &[10]));
        assert!(!listener_owners_unchanged(&[10], &[10, 20]));
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

#[cfg(windows)]
#[derive(Debug, Clone)]
struct WindowsTargetProcess {
    process_id: u32,
    birth_id: u64,
    executable_path: PathBuf,
}

#[cfg(windows)]
#[derive(Debug, Clone)]
pub struct WindowsCodexStopPlan {
    debug_port: u16,
    endpoint: Option<SocketAddr>,
    listener_process_ids: Vec<u32>,
    target_processes: Vec<WindowsTargetProcess>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetedStopOutcome {
    Stopped,
    AlreadyAbsent,
}

#[cfg(windows)]
fn process_identity_root(path: &Path) -> String {
    let normalized = path
        .to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase();
    if let Some((prefix, after_windows_apps)) = normalized.split_once("\\windowsapps\\")
        && let Some((package_name, _)) = after_windows_apps.split_once('\\')
    {
        return format!("{prefix}\\windowsapps\\{package_name}");
    }
    path.parent()
        .unwrap_or(path)
        .to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase()
}

#[cfg(windows)]
fn collect_identity_birth_ids<F>(
    processes: &[crate::windows_integration::WindowsProcessInfo],
    identity_root: &str,
    mut birth_id_for: F,
) -> Result<HashMap<u32, u64>, u32>
where
    F: FnMut(u32) -> Option<u64>,
{
    processes
        .iter()
        .filter(|process| {
            process
                .executable_path
                .as_deref()
                .is_some_and(|path| process_identity_root(path) == identity_root)
        })
        .map(|process| {
            birth_id_for(process.process_id)
                .map(|birth_id| (process.process_id, birth_id))
                .ok_or(process.process_id)
        })
        .collect()
}

#[cfg(windows)]
fn process_descends_from(
    process_id: u32,
    ancestor_process_id: u32,
    parents: &HashMap<u32, u32>,
) -> bool {
    let mut cursor = process_id;
    let mut visited = HashSet::new();
    while visited.insert(cursor) {
        if cursor == ancestor_process_id {
            return true;
        }
        let Some(parent) = parents.get(&cursor).copied() else {
            return false;
        };
        cursor = parent;
    }
    false
}

#[cfg(windows)]
fn listener_owners_unchanged(expected: &[u32], current: &[u32]) -> bool {
    expected == current
}

#[cfg(windows)]
fn target_codex_process_tree_from_snapshot(
    processes: &[crate::windows_integration::WindowsProcessInfo],
    listener_process_ids: &[u32],
    birth_ids: &HashMap<u32, u64>,
) -> Vec<u32> {
    let supported = find_codex_processes_from_snapshot(processes)
        .into_iter()
        .collect::<HashSet<_>>();
    let parents = processes
        .iter()
        .map(|process| (process.process_id, process.parent_process_id))
        .collect::<HashMap<_, _>>();
    let paths = processes
        .iter()
        .filter_map(|process| {
            process
                .executable_path
                .as_deref()
                .map(|path| (process.process_id, path))
        })
        .collect::<HashMap<_, _>>();
    let mut roots = HashSet::new();
    for process_id in listener_process_ids {
        if !supported.contains(process_id) || !birth_ids.contains_key(process_id) {
            continue;
        }
        let Some(owner_path) = paths.get(process_id) else {
            continue;
        };
        let identity_root = process_identity_root(owner_path);
        let mut root = *process_id;
        let mut cursor = *process_id;
        let mut visited = HashSet::new();
        while visited.insert(cursor) {
            let Some(parent) = parents.get(&cursor).copied() else {
                break;
            };
            if !supported.contains(&parent) {
                break;
            }
            let Some(parent_path) = paths.get(&parent) else {
                break;
            };
            let Some(parent_birth_id) = birth_ids.get(&parent).copied() else {
                break;
            };
            let Some(cursor_birth_id) = birth_ids.get(&cursor).copied() else {
                break;
            };
            if process_identity_root(parent_path) != identity_root
                || parent_birth_id > cursor_birth_id
            {
                break;
            }
            root = parent;
            cursor = parent;
        }
        roots.insert((root, identity_root));
    }
    if roots.is_empty() {
        return Vec::new();
    }

    let mut targets = Vec::new();
    for process in processes {
        let Some(process_path) = process.executable_path.as_deref() else {
            continue;
        };
        let Some(process_birth_id) = birth_ids.get(&process.process_id).copied() else {
            continue;
        };
        let mut cursor = process.process_id;
        let mut cursor_birth_id = process_birth_id;
        let mut depth = 0usize;
        let mut visited = HashSet::new();
        while visited.insert(cursor) {
            if roots.contains(&(cursor, process_identity_root(process_path))) {
                targets.push((depth, process.process_id));
                break;
            }
            let Some(parent) = parents.get(&cursor).copied() else {
                break;
            };
            let Some(parent_birth_id) = birth_ids.get(&parent).copied() else {
                break;
            };
            if parent_birth_id > cursor_birth_id {
                break;
            }
            cursor = parent;
            cursor_birth_id = parent_birth_id;
            depth = depth.saturating_add(1);
        }
    }
    targets.sort_unstable_by(|(left_depth, left_pid), (right_depth, right_pid)| {
        right_depth
            .cmp(left_depth)
            .then_with(|| left_pid.cmp(right_pid))
    });
    targets.dedup_by_key(|(_, process_id)| *process_id);
    targets
        .into_iter()
        .map(|(_, process_id)| process_id)
        .collect()
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
    let mut ids = ["Codex", "ChatGPT"]
        .into_iter()
        .flat_map(|name| {
            std::process::Command::new("pgrep")
                .args(["-x", name])
                .output()
                .ok()
                .into_iter()
                .flat_map(|output| {
                    String::from_utf8_lossy(&output.stdout)
                        .lines()
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
        })
        .filter_map(|value| value.trim().parse::<u32>().ok())
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids.dedup();
    ids
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
pub fn stop_launcher_processes_and_wait() -> Result<(), String> {
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
    if terminate_and_wait_for_exit(
        killable,
        RESTART_STOP_WAIT_TIMEOUT_MS,
        RESTART_STOP_WAIT_INTERVAL_MS,
    ) {
        Ok(())
    } else {
        Err("等待旧 Codex++ Launcher 退出超时，已中止重启。".to_string())
    }
}

#[cfg(target_os = "macos")]
pub fn stop_launcher_processes_and_wait() -> Result<(), String> {
    if terminate_macos_processes_and_wait(
        find_launcher_processes(),
        || find_launcher_processes(),
        RESTART_STOP_WAIT_TIMEOUT_MS,
        RESTART_STOP_WAIT_INTERVAL_MS,
    ) {
        Ok(())
    } else {
        Err("等待旧 Codex++ Launcher 退出超时，已中止重启。".to_string())
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn stop_launcher_processes_and_wait() -> Result<(), String> {
    Ok(())
}

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
    let _ = terminate_and_wait_for_exit(
        find_codex_processes(),
        RESTART_STOP_WAIT_TIMEOUT_MS,
        RESTART_STOP_WAIT_INTERVAL_MS,
    );
}

#[cfg(target_os = "macos")]
pub fn stop_codex_processes_and_wait() {
    let _ = terminate_macos_processes_and_wait(
        find_codex_processes(),
        || find_codex_processes(),
        RESTART_STOP_WAIT_TIMEOUT_MS,
        RESTART_STOP_WAIT_INTERVAL_MS,
    );
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn stop_codex_processes_and_wait() {}

#[cfg(target_os = "macos")]
pub fn stop_codex_processes_for_debug_port_and_wait(
    debug_port: u16,
) -> Result<TargetedStopOutcome, String> {
    let process_ids = find_macos_codex_processes_for_debug_port(debug_port);
    if process_ids.is_empty() {
        return Ok(TargetedStopOutcome::AlreadyAbsent);
    }
    if terminate_macos_processes_and_wait(
        process_ids,
        || find_macos_codex_processes_for_debug_port(debug_port),
        RESTART_STOP_WAIT_TIMEOUT_MS,
        RESTART_STOP_WAIT_INTERVAL_MS,
    ) {
        Ok(TargetedStopOutcome::Stopped)
    } else {
        Err("等待目标 Codex App 退出超时，已中止重启。".to_string())
    }
}

#[cfg(windows)]
pub fn prepare_windows_codex_stop_plan(
    debug_port: u16,
    endpoint: Option<SocketAddr>,
) -> Result<WindowsCodexStopPlan, String> {
    let listener_process_ids = if let Some(endpoint) = endpoint {
        let process_ids = crate::windows_integration::tcp_listener_process_ids(endpoint)
            .map_err(|error| format!("读取目标 Codex App 端口归属失败：{error}"))?;
        if process_ids.is_empty() {
            let remaining =
                crate::windows_integration::loopback_tcp_listener_process_ids(debug_port)
                    .map_err(|error| format!("复核 Codex App 调试端口失败：{error}"))?;
            if remaining.is_empty() {
                return Ok(WindowsCodexStopPlan {
                    debug_port,
                    endpoint: None,
                    listener_process_ids: Vec::new(),
                    target_processes: Vec::new(),
                });
            }
            return Err("目标 Codex App 调试端口在重启前发生了归属变化，已拒绝停止。".to_string());
        }
        let all_loopback_process_ids =
            crate::windows_integration::loopback_tcp_listener_process_ids(debug_port)
                .map_err(|error| format!("复核 Codex App 调试端口失败：{error}"))?;
        if !listener_owners_unchanged(&process_ids, &all_loopback_process_ids) {
            return Err(
                "同一调试端口存在另一个回环监听实例，已拒绝停止任何 Codex App。".to_string(),
            );
        }
        process_ids
    } else {
        let process_ids = crate::windows_integration::loopback_tcp_listener_process_ids(debug_port)
            .map_err(|error| format!("复核 Codex App 调试端口失败：{error}"))?;
        if !process_ids.is_empty() {
            return Err(
                "目标 Codex App CDP 不可确认，但调试端口仍有监听，已拒绝停止。".to_string(),
            );
        }
        return Ok(WindowsCodexStopPlan {
            debug_port,
            endpoint: None,
            listener_process_ids: Vec::new(),
            target_processes: Vec::new(),
        });
    };
    if listener_process_ids.len() != 1 {
        return Err("目标 Codex App 调试 endpoint 对应多个进程，已拒绝停止。".to_string());
    }

    let processes = crate::windows_integration::enumerate_processes();
    let owner_process_id = listener_process_ids[0];
    let owner = processes
        .iter()
        .find(|process| process.process_id == owner_process_id)
        .ok_or_else(|| "目标 Codex App 端口进程已退出，已拒绝按旧 PID 停止。".to_string())?;
    let owner_path = owner
        .executable_path
        .as_deref()
        .ok_or_else(|| "无法读取目标 Codex App 可执行路径，已拒绝停止。".to_string())?;
    let identity_root = process_identity_root(owner_path);
    let processes_by_id = processes
        .iter()
        .map(|process| (process.process_id, process))
        .collect::<HashMap<_, _>>();
    let mut ancestor_cursor = owner_process_id;
    let mut visited_ancestors = HashSet::new();
    while visited_ancestors.insert(ancestor_cursor) {
        let Some(process) = processes_by_id.get(&ancestor_cursor) else {
            break;
        };
        let Some(parent) = processes_by_id.get(&process.parent_process_id) else {
            break;
        };
        let Some(parent_path) = parent.executable_path.as_deref() else {
            if crate::app_paths::is_supported_app_executable_name(&parent.exe_file) {
                return Err(format!(
                    "无法读取目标候选父进程 {} 的可执行路径，已拒绝生成部分停止计划。",
                    parent.process_id
                ));
            }
            break;
        };
        if process_identity_root(parent_path) != identity_root {
            break;
        }
        ancestor_cursor = parent.process_id;
    }
    let birth_ids = collect_identity_birth_ids(&processes, &identity_root, |process_id| {
        crate::windows_integration::process_birth_id(process_id)
    })
    .map_err(|process_id| {
        format!("无法读取目标同包进程 {process_id} 的创建时间，已拒绝生成部分停止计划。")
    })?;
    let target_process_ids =
        target_codex_process_tree_from_snapshot(&processes, &listener_process_ids, &birth_ids);
    if target_process_ids.is_empty() {
        return Err("目标调试端口不属于受支持的 Codex App 进程树，已拒绝停止。".to_string());
    }
    let root_process_id = *target_process_ids
        .last()
        .ok_or_else(|| "目标 Codex App 停止计划缺少根进程。".to_string())?;
    let parents = processes
        .iter()
        .map(|process| (process.process_id, process.parent_process_id))
        .collect::<HashMap<_, _>>();
    if let Some(process) = processes.iter().find(|process| {
        process.executable_path.is_none()
            && process_descends_from(process.process_id, root_process_id, &parents)
    }) {
        return Err(format!(
            "无法读取目标进程树内 PID {} 的可执行路径，已拒绝生成部分停止计划。",
            process.process_id
        ));
    }
    let target_processes = target_process_ids
        .into_iter()
        .map(|process_id| {
            let process = processes
                .iter()
                .find(|process| process.process_id == process_id)
                .ok_or_else(|| format!("目标进程 {process_id} 已退出，无法建立安全停止计划。"))?;
            let birth_id = birth_ids
                .get(&process_id)
                .copied()
                .ok_or_else(|| format!("无法读取目标进程 {process_id} 的创建时间。"))?;
            let executable_path = process
                .executable_path
                .clone()
                .ok_or_else(|| format!("无法读取目标进程 {process_id} 的可执行路径。"))?;
            Ok(WindowsTargetProcess {
                process_id,
                birth_id,
                executable_path,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    Ok(WindowsCodexStopPlan {
        debug_port,
        endpoint,
        listener_process_ids,
        target_processes,
    })
}

#[cfg(windows)]
pub fn execute_windows_codex_stop_plan(
    plan: WindowsCodexStopPlan,
) -> Result<TargetedStopOutcome, String> {
    if plan.target_processes.is_empty() {
        let listeners =
            crate::windows_integration::loopback_tcp_listener_process_ids(plan.debug_port)
                .map_err(|error| format!("复核 Codex App 调试端口失败：{error}"))?;
        return if listeners.is_empty() {
            Ok(TargetedStopOutcome::AlreadyAbsent)
        } else {
            Err("Codex App 在停止 Launcher 后重新占用了调试端口，已中止重启。".to_string())
        };
    }

    let endpoint = plan
        .endpoint
        .ok_or_else(|| "安全停止计划缺少目标 endpoint。".to_string())?;
    let current_listener_process_ids =
        crate::windows_integration::loopback_tcp_listener_process_ids(plan.debug_port)
            .map_err(|error| format!("停止前复核目标 Codex App 端口归属失败：{error}"))?;
    if !listener_owners_unchanged(&plan.listener_process_ids, &current_listener_process_ids) {
        return Err("目标 Codex App 端口归属在停止前发生变化，已中止重启。".to_string());
    }

    let _ = crate::diagnostic_log::append_diagnostic_log(
        "watcher.targeted_stop_started",
        serde_json::json!({
            "debug_port": plan.debug_port,
            "endpoint": endpoint.to_string(),
            "listener_process_ids": &plan.listener_process_ids,
            "target_process_ids": plan.target_processes.iter().map(|process| process.process_id).collect::<Vec<_>>(),
        }),
    );
    for process in &plan.target_processes {
        match crate::windows_integration::terminate_process_if_identity_matches(
            process.process_id,
            process.birth_id,
            &process.executable_path,
        ) {
            Ok(true) => {}
            Ok(false)
                if crate::windows_integration::process_birth_id(process.process_id)
                    != Some(process.birth_id) => {}
            Ok(false) => {
                return Err(format!(
                    "目标进程 {} 的身份复核失败，已中止重启。",
                    process.process_id
                ));
            }
            Err(error) => {
                return Err(format!("停止目标进程 {} 失败：{error}", process.process_id));
            }
        }
    }

    let deadline = std::time::Instant::now() + Duration::from_millis(RESTART_STOP_WAIT_TIMEOUT_MS);
    loop {
        let remaining = plan
            .target_processes
            .iter()
            .filter(|process| {
                crate::windows_integration::process_birth_id(process.process_id)
                    == Some(process.birth_id)
            })
            .map(|process| process.process_id)
            .collect::<Vec<_>>();
        if remaining.is_empty() {
            break;
        }
        if std::time::Instant::now() >= deadline {
            return Err(format!(
                "等待目标 Codex App 退出超时，仍在运行的 PID：{remaining:?}"
            ));
        }
        std::thread::sleep(Duration::from_millis(RESTART_STOP_WAIT_INTERVAL_MS));
    }
    let remaining_listeners =
        crate::windows_integration::loopback_tcp_listener_process_ids(plan.debug_port)
            .map_err(|error| format!("停止后复核目标 Codex App 端口失败：{error}"))?;
    if !remaining_listeners.is_empty() {
        return Err("目标 Codex App 停止后调试 endpoint 仍被占用，已中止重启。".to_string());
    }
    Ok(TargetedStopOutcome::Stopped)
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn stop_codex_processes_for_debug_port_and_wait(
    _debug_port: u16,
) -> Result<TargetedStopOutcome, String> {
    Ok(TargetedStopOutcome::AlreadyAbsent)
}

#[cfg(target_os = "macos")]
fn terminate_macos_processes_and_wait<F>(
    process_ids: Vec<u32>,
    mut find_processes: F,
    timeout_ms: u64,
    interval_ms: u64,
) -> bool
where
    F: FnMut() -> Vec<u32>,
{
    if process_ids.is_empty() {
        return true;
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
            return remaining.is_empty();
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
    macos_launcher_process_ids_from_pgrep_outputs(
        macos_launcher_binary_names()
            .into_iter()
            .filter_map(|binary| {
                std::process::Command::new("pgrep")
                    .args(["-x", binary])
                    .output()
                    .ok()
                    .map(|output| output.stdout)
            }),
    )
}

#[cfg(target_os = "macos")]
fn find_macos_codex_processes_for_debug_port(debug_port: u16) -> Vec<u32> {
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
            let is_desktop_main = (args.contains(".app/Contents/MacOS/ChatGPT")
                || args.contains(".app/Contents/MacOS/Codex"))
                && !args.contains("/Helpers/");
            (is_desktop_main && args.contains(&debug_flag)).then_some(process_id)
        })
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids.dedup();
    ids
}

#[cfg(windows)]
fn terminate_and_wait_for_exit(process_ids: Vec<u32>, timeout_ms: u64, interval_ms: u64) -> bool {
    if process_ids.is_empty() {
        return true;
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
            return remaining.is_empty();
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
