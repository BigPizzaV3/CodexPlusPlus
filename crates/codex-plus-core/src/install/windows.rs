use std::path::{Path, PathBuf};

use super::{
    InstallOptions, MANAGER_BINARY, MANAGER_NAME, SILENT_BINARY, SILENT_NAME,
    install_root_or_default, option_or_current_exe,
};

const UNINSTALL_SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\CodexPlusPlus";
const LEGACY_UNINSTALL_SUBKEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Codex++";
const URL_PROTOCOL_SUBKEY: &str = r"Software\Classes\codexplusplus";
const DREAM_SKIN_URL_PROTOCOL_SUBKEY: &str = r"Software\Classes\dreamskin";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowsEntrypointPlan {
    pub install_root: String,
    pub silent_shortcut: String,
    pub manager_shortcut: String,
    pub launcher_path: String,
    pub manager_path: String,
    pub icon_path: String,
    pub silent_icon_path: String,
    pub manager_icon_path: String,
    pub uninstaller_path: String,
    pub uninstall_command: String,
    pub quiet_uninstall_command: String,
    pub uninstall_key: String,
    pub legacy_uninstall_key: String,
    pub remove_owned_data: bool,
}

pub fn build_windows_entrypoint_plan(options: &InstallOptions) -> WindowsEntrypointPlan {
    let install_root = install_root_or_default(options);
    let launcher_path = option_or_current_exe(&options.launcher_path, SILENT_BINARY);
    let manager_path = option_or_current_exe(&options.manager_path, MANAGER_BINARY);
    let icon_path = default_icon_path();
    let install_location = manager_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| install_root.clone());
    let uninstaller_path = install_location.join("uninstall.exe");
    let uninstall_command = format!("\"{}\"", uninstaller_path.to_string_lossy());
    let quiet_uninstall_command = format!("{uninstall_command} /S");
    WindowsEntrypointPlan {
        silent_shortcut: install_root
            .join("Codex++.lnk")
            .to_string_lossy()
            .to_string(),
        manager_shortcut: install_root
            .join("Codex++ 管理工具.lnk")
            .to_string_lossy()
            .to_string(),
        install_root: install_root.to_string_lossy().to_string(),
        launcher_path: launcher_path.to_string_lossy().to_string(),
        manager_path: manager_path.to_string_lossy().to_string(),
        icon_path: icon_path.to_string_lossy().to_string(),
        silent_icon_path: launcher_path.to_string_lossy().to_string(),
        manager_icon_path: manager_path.to_string_lossy().to_string(),
        uninstaller_path: uninstaller_path.to_string_lossy().to_string(),
        uninstall_command,
        quiet_uninstall_command,
        uninstall_key: "CodexPlusPlus".to_string(),
        legacy_uninstall_key: "Codex++".to_string(),
        remove_owned_data: options.remove_owned_data,
    }
}

#[cfg(windows)]
pub fn install_shortcuts(options: &InstallOptions) -> anyhow::Result<()> {
    let plan = build_windows_entrypoint_plan(options);
    // 必须在写入本轮卸载登记之前读取：图标缺失可能是用户主动删除，而非首次安装。
    let already_installed =
        has_existing_installation(crate::windows_integration::read_current_user_string_values)?;
    let install_root = PathBuf::from(&plan.install_root);
    std::fs::create_dir_all(&install_root)?;
    // 与 NSIS 同步：仅首次安装补桌面图标；升级/修复保留现状（issue #2376）。
    create_desktop_shortcut_on_first_install(
        Path::new(&plan.silent_shortcut),
        already_installed,
        || {
            create_entrypoint_shortcut(
                PathBuf::from(&plan.silent_shortcut),
                PathBuf::from(&plan.launcher_path),
                "Launch Codex++ silently",
                PathBuf::from(&plan.silent_icon_path),
            )
        },
    )?;
    create_desktop_shortcut_on_first_install(
        Path::new(&plan.manager_shortcut),
        already_installed,
        || {
            create_entrypoint_shortcut(
                PathBuf::from(&plan.manager_shortcut),
                PathBuf::from(&plan.manager_path),
                "Open Codex++ management tool",
                PathBuf::from(&plan.manager_icon_path),
            )
        },
    )?;
    register_url_protocol(&plan.manager_path)?;
    write_uninstall_registration(&plan)?;
    Ok(())
}

#[cfg(any(windows, test))]
fn has_existing_installation(
    mut read_values: impl FnMut(&str) -> anyhow::Result<Vec<(String, Option<String>)>>,
) -> anyhow::Result<bool> {
    for key in [UNINSTALL_SUBKEY, LEGACY_UNINSTALL_SUBKEY] {
        if read_values(key)?.iter().any(|(name, value)| {
            name.eq_ignore_ascii_case("InstallLocation")
                && value.as_ref().is_some_and(|value| !value.is_empty())
        }) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// 仅首次安装补建；后续缺失说明用户选择了无桌面图标，不能用缺失判定首次安装。
#[cfg(any(windows, test))]
fn create_desktop_shortcut_on_first_install(
    path: &Path,
    already_installed: bool,
    create: impl FnOnce() -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    if already_installed || path.exists() {
        return Ok(());
    }
    create()
}

#[cfg(windows)]
pub fn uninstall_shortcuts(options: &InstallOptions) -> anyhow::Result<()> {
    let plan = build_windows_entrypoint_plan(options);
    let _ = std::fs::remove_file(&plan.silent_shortcut);
    let _ = std::fs::remove_file(&plan.manager_shortcut);
    let _ = crate::windows_integration::delete_current_user_key(&format!(
        r"{URL_PROTOCOL_SUBKEY}\shell\open\command"
    ));
    let _ = crate::windows_integration::delete_current_user_key(&format!(
        r"{URL_PROTOCOL_SUBKEY}\shell\open"
    ));
    let _ = crate::windows_integration::delete_current_user_key(&format!(
        r"{URL_PROTOCOL_SUBKEY}\shell"
    ));
    let _ = crate::windows_integration::delete_current_user_key(URL_PROTOCOL_SUBKEY);
    let _ = crate::windows_integration::delete_current_user_key(&format!(
        r"{DREAM_SKIN_URL_PROTOCOL_SUBKEY}\shell\open\command"
    ));
    let _ = crate::windows_integration::delete_current_user_key(&format!(
        r"{DREAM_SKIN_URL_PROTOCOL_SUBKEY}\shell\open"
    ));
    let _ = crate::windows_integration::delete_current_user_key(&format!(
        r"{DREAM_SKIN_URL_PROTOCOL_SUBKEY}\shell"
    ));
    let _ = crate::windows_integration::delete_current_user_key(DREAM_SKIN_URL_PROTOCOL_SUBKEY);
    let _ = crate::windows_integration::delete_current_user_key(LEGACY_UNINSTALL_SUBKEY);
    let _ = crate::windows_integration::delete_current_user_key(UNINSTALL_SUBKEY);
    Ok(())
}

#[cfg(not(windows))]
pub fn install_shortcuts(_options: &InstallOptions) -> anyhow::Result<()> {
    anyhow::bail!("Windows shortcuts are only supported on Windows")
}

#[cfg(not(windows))]
pub fn uninstall_shortcuts(_options: &InstallOptions) -> anyhow::Result<()> {
    anyhow::bail!("Windows shortcuts are only supported on Windows")
}

#[cfg(windows)]
fn create_entrypoint_shortcut(
    path: PathBuf,
    target: PathBuf,
    description: &str,
    icon: PathBuf,
) -> anyhow::Result<()> {
    crate::windows_integration::create_shortcut(&crate::windows_integration::ShortcutSpec {
        working_directory: target.parent().map(Path::to_path_buf),
        path,
        target,
        arguments: String::new(),
        description: description.to_string(),
        icon: Some(icon),
        show_minimized: false,
    })
}

#[cfg(windows)]
fn write_uninstall_registration(plan: &WindowsEntrypointPlan) -> anyhow::Result<()> {
    let _ = crate::windows_integration::delete_current_user_key(LEGACY_UNINSTALL_SUBKEY);
    let install_location = Path::new(&plan.manager_path)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(&plan.install_root))
        .to_string_lossy()
        .to_string();
    for (name, value) in [
        ("DisplayName", "Codex++".to_string()),
        ("DisplayVersion", crate::version::VERSION.to_string()),
        ("Publisher", "BigPizzaV3".to_string()),
        ("DisplayIcon", plan.manager_icon_path.clone()),
        ("InstallLocation", install_location),
        ("UninstallString", plan.uninstall_command.clone()),
        ("QuietUninstallString", plan.quiet_uninstall_command.clone()),
    ] {
        crate::windows_integration::set_current_user_string_value(UNINSTALL_SUBKEY, name, &value)?;
    }
    Ok(())
}

#[cfg(windows)]
fn register_url_protocol(manager_path: &str) -> anyhow::Result<()> {
    register_url_protocol_key(
        URL_PROTOCOL_SUBKEY,
        "URL:Codex++ Import Protocol",
        manager_path,
    )?;
    register_url_protocol_key(
        DREAM_SKIN_URL_PROTOCOL_SUBKEY,
        "URL:DreamSkin Community Theme Protocol",
        manager_path,
    )
}

#[cfg(windows)]
fn register_url_protocol_key(
    key: &str,
    description: &str,
    manager_path: &str,
) -> anyhow::Result<()> {
    crate::windows_integration::set_current_user_string_value(key, "", description)?;
    crate::windows_integration::set_current_user_string_value(key, "URL Protocol", "")?;
    crate::windows_integration::set_current_user_string_value(
        &format!(r"{key}\shell\open\command"),
        "",
        &format!("\"{manager_path}\" \"%1\""),
    )?;
    Ok(())
}

fn default_icon_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .map(|path| path.join("codex-plus-plus.ico"))
        .unwrap_or_else(|| PathBuf::from("codex-plus-plus.ico"))
}

#[allow(dead_code)]
fn _entrypoint_names() -> (&'static str, &'static str) {
    (SILENT_NAME, MANAGER_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deleted_desktop_shortcuts_stay_absent_on_upgrade_or_repair() {
        let root = tempfile::tempdir().unwrap();
        let desktop = root.path().join("Desktop");
        std::fs::create_dir(&desktop).unwrap();
        // 安装记录来自另一个目录；空桌面不代表首次安装。
        let installed = has_existing_installation(|key| {
            Ok(if key == UNINSTALL_SUBKEY {
                vec![("InstallLocation".into(), Some("C:/Programs/Codex++".into()))]
            } else {
                Vec::new()
            })
        })
        .unwrap();
        for name in ["Codex++.lnk", "Codex++ 管理工具.lnk"] {
            let shortcut = desktop.join(name);
            create_desktop_shortcut_on_first_install(&shortcut, installed, || {
                std::fs::write(&shortcut, b"new shortcut")?;
                Ok(())
            })
            .unwrap();
            assert!(!shortcut.exists());
        }
    }

    #[test]
    fn first_install_creates_missing_shortcuts_and_preserves_existing_ones() {
        let root = tempfile::tempdir().unwrap();
        let installed = has_existing_installation(|_| Ok(Vec::new())).unwrap();
        let shortcut = root.path().join("Codex++.lnk");
        create_desktop_shortcut_on_first_install(&shortcut, installed, || {
            std::fs::write(&shortcut, b"original shortcut")?;
            Ok(())
        })
        .unwrap();
        create_desktop_shortcut_on_first_install(&shortcut, installed, || {
            std::fs::write(&shortcut, b"replacement shortcut")?;
            Ok(())
        })
        .unwrap();
        assert_eq!(std::fs::read(&shortcut).unwrap(), b"original shortcut");
    }

    #[test]
    fn legacy_installation_registration_also_preserves_a_clean_desktop() {
        let root = tempfile::tempdir().unwrap();
        let installed = has_existing_installation(|key| {
            Ok(if key == LEGACY_UNINSTALL_SUBKEY {
                vec![("InstallLocation".into(), Some("C:/Programs/Codex++".into()))]
            } else {
                Vec::new()
            })
        })
        .unwrap();
        let shortcut = root.path().join("Codex++.lnk");
        create_desktop_shortcut_on_first_install(&shortcut, installed, || {
            std::fs::write(&shortcut, b"unwanted shortcut")?;
            Ok(())
        })
        .unwrap();
        assert!(!shortcut.exists());
    }

    #[test]
    fn empty_install_location_does_not_block_first_install() {
        assert!(
            !has_existing_installation(|_| {
                Ok(vec![("InstallLocation".into(), Some(String::new()))])
            })
            .unwrap()
        );
    }
}
