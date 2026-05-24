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

#[cfg(target_os = "macos")]
pub fn stop_launcher_processes() {
    if let Some(bundle_path) = macos_launcher_bundle_path() {
        stop_macos_processes_matching(bundle_path.to_string_lossy().as_ref());
    }
}

#[cfg(all(not(windows), not(target_os = "macos")))]
pub fn stop_launcher_processes() {}

#[cfg(windows)]
pub fn stop_codex_processes() {
    for process_id in find_codex_processes() {
        let _ = crate::windows_integration::terminate_process(process_id);
    }
}

#[cfg(target_os = "macos")]
pub fn stop_codex_processes() {
    if let Some(app_dir) = macos_codex_app_dir_from_settings() {
        stop_codex_processes_at(&app_dir);
    }
}

#[cfg(all(not(windows), not(target_os = "macos")))]
pub fn stop_codex_processes() {}

#[cfg(target_os = "macos")]
pub fn stop_codex_processes_at(app_dir: &Path) {
    stop_macos_processes_matching(app_dir.to_string_lossy().as_ref());
}

#[cfg(all(not(windows), not(target_os = "macos")))]
pub fn stop_codex_processes_at(_app_dir: &Path) {}

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

#[cfg(target_os = "macos")]
fn macos_launcher_bundle_path() -> Option<PathBuf> {
    let launcher = crate::install::companion_binary_path(crate::install::SILENT_BINARY);
    launcher.ancestors().nth(3).map(Path::to_path_buf)
}

#[cfg(target_os = "macos")]
fn macos_codex_app_dir_from_settings() -> Option<PathBuf> {
    let settings = crate::settings::SettingsStore::default().load().ok()?;
    crate::app_paths::resolve_codex_app_dir_with_saved(None, Some(settings.codex_app_path.as_str()))
}

pub fn filter_macos_codex_related_port_owner_processes<'a>(
    processes: impl IntoIterator<Item = (u32, &'a str)>,
    current_process_id: u32,
    trusted_fragments: impl IntoIterator<Item = &'a str>,
) -> Vec<u32> {
    let fragments = trusted_fragments
        .into_iter()
        .map(str::trim)
        .filter(|fragment| !fragment.is_empty())
        .collect::<Vec<_>>();
    processes
        .into_iter()
        .filter(|(process_id, command)| {
            *process_id != current_process_id
                && fragments.iter().any(|fragment| command.contains(fragment))
        })
        .map(|(process_id, _)| process_id)
        .collect()
}

#[cfg(target_os = "macos")]
pub fn stop_codex_related_processes_listening_on_ports(app_dir: &Path, ports: &[u16]) {
    let fragments = macos_codex_related_process_fragments(
        app_dir,
        &crate::relay_config::default_codex_home_dir(),
    );
    stop_macos_port_owner_processes(ports, &fragments, "TERM");
    std::thread::sleep(Duration::from_millis(300));
    stop_macos_port_owner_processes(ports, &fragments, "KILL");
}

#[cfg(target_os = "macos")]
fn stop_macos_port_owner_processes(ports: &[u16], trusted_fragments: &[String], signal: &str) {
    let owners = macos_listening_port_owner_processes(ports);
    let killable = filter_macos_codex_related_port_owner_processes(
        owners
            .iter()
            .map(|(process_id, command)| (*process_id, command.as_str())),
        std::process::id(),
        trusted_fragments.iter().map(String::as_str),
    );
    for process_id in killable {
        let _ = macos_signal_process(process_id, signal);
    }
}

#[cfg(target_os = "macos")]
fn macos_codex_related_process_fragments(app_dir: &Path, codex_home: &Path) -> Vec<String> {
    let mut fragments = vec![
        app_dir.to_string_lossy().to_string(),
        codex_home.to_string_lossy().to_string(),
    ];
    if let Some(bundle_path) = macos_launcher_bundle_path() {
        fragments.push(bundle_path.to_string_lossy().to_string());
    }
    fragments
}

#[cfg(target_os = "macos")]
fn macos_listening_port_owner_processes(ports: &[u16]) -> Vec<(u32, String)> {
    let mut process_ids = HashSet::new();
    for port in ports.iter().copied().filter(|port| *port != 0) {
        let Ok(output) = Command::new("lsof")
            .arg("-nP")
            .arg(format!("-iTCP:{port}"))
            .arg("-sTCP:LISTEN")
            .arg("-t")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
        else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            if let Ok(process_id) = line.trim().parse::<u32>() {
                process_ids.insert(process_id);
            }
        }
    }

    process_ids
        .into_iter()
        .filter_map(|process_id| {
            macos_command_for_pid(process_id).map(|command| (process_id, command))
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn macos_command_for_pid(process_id: u32) -> Option<String> {
    let output = Command::new("ps")
        .arg("-p")
        .arg(process_id.to_string())
        .arg("-o")
        .arg("command=")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(target_os = "macos")]
fn stop_macos_processes_matching(command_fragment: &str) {
    let command_fragment = command_fragment.trim();
    if command_fragment.is_empty() {
        return;
    }

    let process_ids = macos_process_ids_matching(command_fragment);
    if process_ids.is_empty() {
        return;
    }

    for process_id in &process_ids {
        let _ = macos_signal_process(*process_id, "TERM");
    }
    std::thread::sleep(Duration::from_millis(300));

    for process_id in macos_process_ids_matching(command_fragment) {
        let _ = macos_signal_process(process_id, "KILL");
    }
}

#[cfg(target_os = "macos")]
fn macos_process_ids_matching(command_fragment: &str) -> Vec<u32> {
    let Ok(output) = Command::new("ps")
        .args(["-axww", "-o", "pid=,command="])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
    else {
        return Vec::new();
    };

    if !output.status.success() {
        return Vec::new();
    }

    let command_fragment = command_fragment.trim();
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let process_id = parts.next()?.parse::<u32>().ok()?;
            let command = parts.collect::<Vec<_>>().join(" ");
            command.contains(command_fragment).then_some(process_id)
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn macos_signal_process(
    process_id: u32,
    signal: &str,
) -> std::io::Result<std::process::ExitStatus> {
    Command::new("kill")
        .arg(format!("-{signal}"))
        .arg(process_id.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
}
