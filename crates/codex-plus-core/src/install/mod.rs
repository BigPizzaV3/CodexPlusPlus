use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

pub mod macos;
pub mod windows;

// 保留旧名称供存量安装识别；新入口统一使用 APP_*。
pub const APP_NAME: &str = "Codex++";
pub const APP_BINARY: &str = "codex-plus-plus";
pub const APP_BUNDLE_ID: &str = "com.bigpizzav3.codexplusplus";
pub const MACOS_APP_EXECUTABLE: &str = "CodexPlusPlus";
pub const SILENT_NAME: &str = "Codex++";
pub const MANAGER_NAME: &str = "Codex++ 管理工具";
pub const SILENT_BINARY: &str = "codex-plus-plus";
pub const MACOS_SILENT_EXECUTABLE: &str = "CodexPlusPlus";
pub const MANAGER_BINARY: &str = "codex-plus-plus-manager";
pub const SILENT_BUNDLE_ID: &str = "com.bigpizzav3.codexplusplus";
pub const MANAGER_BUNDLE_ID: &str = "com.bigpizzav3.codexplusplus.manager";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct InstallOptions {
    #[serde(default)]
    pub install_root: Option<PathBuf>,
    #[serde(default)]
    pub launcher_path: Option<PathBuf>,
    #[serde(default)]
    pub manager_path: Option<PathBuf>,
    #[serde(default)]
    pub remove_owned_data: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ShortcutState {
    pub installed: bool,
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EntryPointState {
    pub silent_shortcut: ShortcutState,
    pub management_shortcut: ShortcutState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InstallActionResult {
    pub status: String,
    pub message: String,
    pub silent_shortcut: ShortcutState,
    pub management_shortcut: ShortcutState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacosAppBundle {
    pub app_path: PathBuf,
    pub info_plist: String,
    pub launch_script: String,
    pub binary_source: Option<PathBuf>,
    pub binary_target_name: Option<String>,
}

impl ShortcutState {
    pub fn missing(path: Option<PathBuf>) -> Self {
        Self {
            installed: false,
            path: path.map(|path| path.to_string_lossy().to_string()),
        }
    }

    pub fn from_candidates(candidates: Vec<PathBuf>) -> Self {
        if let Some(path) = candidates.iter().find(|path| path.exists()) {
            return Self {
                installed: true,
                path: Some(path.to_string_lossy().to_string()),
            };
        }
        Self::missing(candidates.into_iter().next())
    }
}

pub fn shortcut_names() -> (&'static str, &'static str) {
    ("Codex++.lnk", "Codex++.lnk")
}

pub fn app_bundle_names() -> (&'static str, &'static str) {
    ("Codex++.app", "Codex++.app")
}

pub fn inspect_entrypoints() -> EntryPointState {
    let root = default_install_root();
    let candidates = entrypoint_candidates(&root);
    #[cfg(target_os = "macos")]
    let shortcut = candidates
        .iter()
        .find(|path| macos::validate_native_bundle(path).is_ok())
        .map(|path| ShortcutState {
            installed: true,
            path: Some(path.to_string_lossy().to_string()),
        })
        .unwrap_or_else(|| ShortcutState::missing(candidates.into_iter().next()));
    #[cfg(not(target_os = "macos"))]
    let shortcut = ShortcutState::from_candidates(candidates);
    // 字段名保留给旧 API 消费方，两者现在表示同一个界面入口。
    EntryPointState {
        silent_shortcut: shortcut.clone(),
        management_shortcut: shortcut,
    }
}

pub fn install_entrypoints(options: &InstallOptions) -> InstallActionResult {
    let result = platform_install(options);
    action_result(result, "入口已安装。")
}

pub fn uninstall_entrypoints(options: &InstallOptions) -> InstallActionResult {
    let result = platform_uninstall(options);
    if result.is_ok() && options.remove_owned_data {
        let _ = remove_owned_data();
    }
    action_result(result, "入口已卸载。")
}

pub fn repair_entrypoints(options: &InstallOptions) -> InstallActionResult {
    let result = platform_install(options);
    action_result(result, "入口已修复。")
}

pub fn build_windows_entrypoint_plan(options: &InstallOptions) -> windows::WindowsEntrypointPlan {
    windows::build_windows_entrypoint_plan(options)
}

pub fn build_macos_app_bundle(options: &InstallOptions, manager: bool) -> MacosAppBundle {
    macos::build_app_bundle(options, manager)
}

pub fn remove_owned_data() -> std::io::Result<()> {
    let dir = crate::paths::default_app_state_dir();
    if !dir.exists() {
        return Ok(());
    }
    // 卸载流程会递归删除，路径来自环境/推导，先过一道"不许删 CODEX_HOME 及其祖先"
    // 的兜底（#2146）。守卫只在这条路径确实指向 home 时才会拒绝，正常卸载不受影响。
    if let Err(error) = crate::codex_home::ensure_safe_recursive_removal(
        &dir,
        &crate::codex_home::default_codex_home_dir(),
    ) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            error.to_string(),
        ));
    }
    std::fs::remove_dir_all(dir)?;
    Ok(())
}

pub fn default_install_root() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        return crate::windows_integration::desktop_dir().or_else(|| {
            directories::UserDirs::new().and_then(|dirs| dirs.desktop_dir().map(PathBuf::from))
        });
    }

    #[cfg(target_os = "macos")]
    {
        let sys_apps = PathBuf::from("/Applications");
        if sys_apps.join(format!("{APP_NAME}.app")).exists()
            || sys_apps.join(format!("{MANAGER_NAME}.app")).exists()
        {
            return Some(sys_apps);
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = macos_applications_dir_from_exe(&exe) {
                if is_macos_applications_dir(&dir) {
                    return Some(dir);
                }
            }
        }
        return Some(sys_apps);
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    {
        directories::UserDirs::new().and_then(|dirs| dirs.desktop_dir().map(PathBuf::from))
    }
}

pub fn default_install_root_strategy() -> &'static str {
    if cfg!(windows) {
        "windows-known-folder"
    } else if cfg!(target_os = "macos") {
        "macos-applications"
    } else {
        "user-dirs-desktop"
    }
}

fn platform_install(options: &InstallOptions) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        windows::install_shortcuts(options)
    }

    #[cfg(target_os = "macos")]
    {
        macos::install_app_bundles(options)
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = options;
        anyhow::bail!("当前平台暂不支持安装 Codex++ 入口")
    }
}

fn platform_uninstall(options: &InstallOptions) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        windows::uninstall_shortcuts(options)
    }

    #[cfg(target_os = "macos")]
    {
        macos::uninstall_app_bundles(options)
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = options;
        anyhow::bail!("当前平台暂不支持卸载 Codex++ 入口")
    }
}

fn action_result(result: anyhow::Result<()>, success_message: &str) -> InstallActionResult {
    let state = inspect_entrypoints();
    match result {
        Ok(()) => InstallActionResult {
            status: "ok".to_string(),
            message: success_message.to_string(),
            silent_shortcut: state.silent_shortcut,
            management_shortcut: state.management_shortcut,
        },
        Err(error) => InstallActionResult {
            status: "failed".to_string(),
            message: error.to_string(),
            silent_shortcut: state.silent_shortcut,
            management_shortcut: state.management_shortcut,
        },
    }
}

fn entrypoint_candidates(root: &Option<PathBuf>) -> Vec<PathBuf> {
    let Some(root) = root else {
        return Vec::new();
    };
    let name = APP_NAME;
    if cfg!(windows) {
        vec![root.join(format!("{name}.lnk"))]
    } else if cfg!(target_os = "macos") {
        vec![root.join(format!("{name}.app"))]
    } else {
        vec![root.join(format!("{name}.desktop"))]
    }
}

pub(crate) fn application_source(options: &InstallOptions) -> PathBuf {
    if let Some(source) = &options.launcher_path {
        return source.clone();
    }
    if let Some(source) = &options.manager_path {
        if matches!(
            source.file_stem().and_then(|name| name.to_str()),
            Some(MANAGER_BINARY | "CodexPlusPlusManager")
        ) {
            return companion_binary_path_from_exe(source, APP_BINARY);
        }
        return source.clone();
    }
    option_or_current_exe(&None, APP_BINARY)
}

pub fn option_or_current_exe(value: &Option<PathBuf>, binary: &str) -> PathBuf {
    if let Some(value) = value {
        return value.clone();
    }
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
    companion_binary_path_from_exe(&exe, binary)
}

pub fn companion_binary_path(binary: &str) -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
    companion_binary_path_from_exe(&exe, binary)
}

pub fn spawn_companion<I, S>(binary: &str, args: I) -> anyhow::Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let args = args
        .into_iter()
        .map(|arg| arg.as_ref().to_os_string())
        .collect::<Vec<OsString>>();

    #[cfg(target_os = "macos")]
    {
        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
        if matches!(binary, APP_BINARY | MANAGER_BINARY)
            && macos_applications_dir_and_app_name_from_exe(&exe).is_some()
        {
            let fallback = companion_binary_path_from_exe(&exe, binary);
            // 当前原生界面包可以被 Finder 改名；先按 marker 验证当前包，再尝试安装位置。
            let app = macos::native_application_bundle_from_executable(&exe)
                .or_else(|_| macos::native_application_bundle_from_executable(&fallback))?;
            let executable = app.join("Contents/MacOS").join(MACOS_APP_EXECUTABLE);
            let launch_result = Command::new("/usr/bin/open")
                .arg(&app)
                .arg("--args")
                .args(&args)
                .status();
            if launch_result.as_ref().is_ok_and(|status| status.success()) {
                return Ok(format!("bundle:{}", app.display()));
            }
            let mut command = Command::new(&executable);
            command.args(&args);
            command
                .spawn()
                .map_err(|error| anyhow::anyhow!("无法启动 {}：{error}", executable.display()))?;
            return Ok(executable.to_string_lossy().to_string());
        }
    }

    let path = companion_binary_path(binary);
    let mut command = Command::new(&path);
    command.args(&args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(crate::windows_create_no_window());
    }
    command
        .spawn()
        .map_err(|error| anyhow::anyhow!("无法启动 {}：{error}", path.to_string_lossy()))?;
    Ok(path.to_string_lossy().to_string())
}

pub fn open_or_activate_manager() -> anyhow::Result<String> {
    // 空参数仅打开界面；启动 Codex 必须由界面中的按钮触发。
    spawn_companion(MANAGER_BINARY, std::iter::empty::<&str>())
}

pub fn macos_companion_bundle_identifier_from_exe(
    exe: &Path,
    binary: &str,
) -> Option<&'static str> {
    let (_, app_name) = macos_applications_dir_and_app_name_from_exe(exe)?;
    let known_bundle = [APP_NAME, MANAGER_NAME]
        .iter()
        .any(|name| app_name.eq_ignore_ascii_case(&format!("{name}.app")));
    if known_bundle && matches!(binary, APP_BINARY | MANAGER_BINARY) {
        Some(APP_BUNDLE_ID)
    } else {
        None
    }
}

pub fn companion_binary_path_from_exe(exe: &Path, binary: &str) -> PathBuf {
    let dir = exe.parent().unwrap_or_else(|| Path::new("."));
    if matches!(binary, APP_BINARY | MANAGER_BINARY) {
        #[cfg(target_os = "macos")]
        if macos::native_application_bundle_from_executable(exe).is_ok() {
            return exe.to_path_buf();
        }
        if let Some((applications_dir, app_name)) =
            macos_applications_dir_and_app_name_from_exe(exe)
        {
            if app_name.eq_ignore_ascii_case(&format!("{APP_NAME}.app")) {
                return exe.to_path_buf();
            }
            return applications_dir
                .join(format!("{APP_NAME}.app"))
                .join("Contents/MacOS")
                .join(MACOS_APP_EXECUTABLE);
        }
        if exe.file_stem().and_then(|name| name.to_str()) == Some(APP_BINARY) {
            return exe.to_path_buf();
        }
        let suffix = if cfg!(windows) { ".exe" } else { "" };
        return dir.join(format!("{APP_BINARY}{suffix}"));
    }
    let suffix = if cfg!(windows) { ".exe" } else { "" };
    dir.join(format!("{binary}{suffix}"))
}

#[cfg(target_os = "macos")]
fn macos_applications_dir_from_exe(exe: &Path) -> Option<PathBuf> {
    macos_applications_dir_and_app_name_from_exe(exe).map(|(dir, _)| dir)
}

fn macos_applications_dir_and_app_name_from_exe(exe: &Path) -> Option<(PathBuf, String)> {
    let mut path = exe;
    while let Some(parent) = path.parent() {
        if path.extension().and_then(|extension| extension.to_str()) == Some("app") {
            let app_name = path.file_name()?.to_string_lossy().to_string();
            return Some((parent.to_path_buf(), app_name));
        }
        path = parent;
    }
    None
}

#[cfg(target_os = "macos")]
fn is_macos_applications_dir(path: &Path) -> bool {
    if path == Path::new("/Applications") {
        return true;
    }
    directories::BaseDirs::new()
        .map(|dirs| path == dirs.home_dir().join("Applications"))
        .unwrap_or(false)
}

pub(crate) fn install_root_or_default(options: &InstallOptions) -> PathBuf {
    options
        .install_root
        .clone()
        .or_else(default_install_root)
        .unwrap_or_else(|| PathBuf::from("."))
}
