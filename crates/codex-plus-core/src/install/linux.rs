use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use super::{
    APP_NAME, InstallOptions, application_source, install_root_or_default,
};

pub fn build_desktop_entry_content(exec_path: &Path) -> String {
    let exec_str = exec_path.to_string_lossy();
    format!(
        "[Desktop Entry]\n\
         Name={name}\n\
         Comment=Configure and launch the official Codex desktop app\n\
         Exec={exec_str} %u\n\
         Icon=codex-plus-plus\n\
         Type=Application\n\
         Categories=Development;Utility;\n\
         Terminal=false\n\
         StartupNotify=true\n\
         StartupWMClass=codex-plus-plus\n\
         MimeType=x-scheme-handler/codexplusplus;x-scheme-handler/dreamskin;\n",
        name = APP_NAME,
        exec_str = exec_str
    )
}

pub fn install_shortcuts(options: &InstallOptions) -> anyhow::Result<()> {
    let install_root = install_root_or_default(options);
    fs::create_dir_all(&install_root)?;

    let launcher_path = application_source(options);
    let content = build_desktop_entry_content(&launcher_path);

    // 1. 安装到指定的 install_root（通常是 ~/Desktop）
    let desktop_shortcut = install_root.join(format!("{APP_NAME}.desktop"));
    fs::write(&desktop_shortcut, &content)?;
    #[cfg(unix)]
    let _ = fs::set_permissions(&desktop_shortcut, fs::Permissions::from_mode(0o755));

    // 2. 同时确保 ~/.local/share/applications/ 下存在桌面入口，供应用菜单和 MIME handler 识别
    if let Some(base_dirs) = directories::BaseDirs::new() {
        let app_dir = base_dirs.data_local_dir().join("applications");
        if fs::create_dir_all(&app_dir).is_ok() {
            let app_entry = app_dir.join("codex-plus-plus.desktop");
            let _ = fs::write(&app_entry, &content);
            #[cfg(unix)]
            let _ = fs::set_permissions(&app_entry, fs::Permissions::from_mode(0o755));

            // 更新桌面数据库和 MIME 关联
            #[cfg(target_os = "linux")]
            let _ = std::process::Command::new("update-desktop-database")
                .arg(&app_dir)
                .status();
            #[cfg(target_os = "linux")]
            let _ = std::process::Command::new("xdg-mime")
                .args(["default", "codex-plus-plus.desktop", "x-scheme-handler/codexplusplus"])
                .status();
            #[cfg(target_os = "linux")]
            let _ = std::process::Command::new("xdg-mime")
                .args(["default", "codex-plus-plus.desktop", "x-scheme-handler/dreamskin"])
                .status();
        }
    }

    Ok(())
}

pub fn uninstall_shortcuts(options: &InstallOptions) -> anyhow::Result<()> {
    let install_root = install_root_or_default(options);
    let candidates = [
        install_root.join(format!("{APP_NAME}.desktop")),
        install_root.join("codex-plus-plus.desktop"),
    ];

    for path in &candidates {
        if path.is_file() {
            let _ = fs::remove_file(path);
        }
    }

    if let Some(base_dirs) = directories::BaseDirs::new() {
        let app_entry = base_dirs.data_local_dir().join("applications/codex-plus-plus.desktop");
        if app_entry.is_file() {
            let _ = fs::remove_file(&app_entry);
            #[cfg(target_os = "linux")]
            let _ = std::process::Command::new("update-desktop-database")
                .arg(base_dirs.data_local_dir().join("applications"))
                .status();
        }
    }

    Ok(())
}
