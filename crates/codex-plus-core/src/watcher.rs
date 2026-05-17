use std::collections::{HashMap, HashSet};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

pub const WATCHER_INTERVAL_SECONDS: f64 = 3.0;
pub const CDP_PROBE_TIMEOUT_SECONDS: f64 = 0.5;
pub const TAKEOVER_FAILURE_BACKOFF_SECONDS: f64 = 30.0;
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
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_ok()
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
            let executable = executable.to_ascii_lowercase();
            executable
                .contains("\\windowsapps\\openai.codex_")
                .then_some(process_id)
        })
        .collect()
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
    codex_process_ids(
        crate::windows_integration::enumerate_processes()
            .into_iter()
            .filter(|process| process.exe_file.eq_ignore_ascii_case("codex.exe"))
            .filter_map(|process| {
                process
                    .executable_path
                    .as_deref()
                    .map(|path| (process.process_id, path.to_string_lossy().to_string()))
            })
            .collect::<Vec<_>>()
            .iter()
            .map(|(pid, path)| (*pid, path.as_str())),
    )
}

#[cfg(not(windows))]
pub fn find_codex_processes() -> Vec<u32> {
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

#[cfg(not(windows))]
pub fn stop_launcher_processes() {}

#[cfg(windows)]
pub fn stop_codex_processes() {
    for process_id in find_codex_processes() {
        let _ = crate::windows_integration::terminate_process(process_id);
    }
}

#[cfg(not(windows))]
pub fn stop_codex_processes() {}

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
