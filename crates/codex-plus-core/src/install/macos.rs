#[cfg(target_os = "macos")]
use std::fs;
#[cfg(target_os = "macos")]
use std::os::unix::fs::PermissionsExt;
#[cfg(target_os = "macos")]
use std::path::Path;

use super::{
    InstallOptions, MANAGER_BINARY, MANAGER_NAME, MacosAppBundle, SILENT_BINARY, SILENT_NAME,
    install_root_or_default, option_or_current_exe,
};

pub fn build_app_bundle(options: &InstallOptions, manager: bool) -> MacosAppBundle {
    let install_root = install_root_or_default(options);
    let display_name = if manager { MANAGER_NAME } else { SILENT_NAME };
    let executable_name = if manager {
        "CodexPlusPlusManager"
    } else {
        "CodexPlusPlus"
    };
    let binary = if manager {
        MANAGER_BINARY
    } else {
        SILENT_BINARY
    };
    let target = option_or_current_exe(
        if manager {
            &options.manager_path
        } else {
            &options.launcher_path
        },
        binary,
    );
    let identifier_suffix = if manager { ".manager" } else { "" };
    MacosAppBundle {
        app_path: install_root.join(format!("{display_name}.app")),
        info_plist: info_plist(display_name, executable_name, identifier_suffix),
        launch_script: format!("#!/bin/sh\nexec \"{}\"\n", target.to_string_lossy()),
    }
}

#[cfg(target_os = "macos")]
pub fn install_app_bundles(options: &InstallOptions) -> anyhow::Result<()> {
    write_bundle(&build_app_bundle(options, false))?;
    write_bundle(&build_app_bundle(options, true))?;
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn uninstall_app_bundles(options: &InstallOptions) -> anyhow::Result<()> {
    let install_root = install_root_or_default(options);
    for name in [SILENT_NAME, MANAGER_NAME] {
        let app = install_root.join(format!("{name}.app"));
        if app.exists() {
            fs::remove_dir_all(app)?;
        }
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn install_app_bundles(_options: &InstallOptions) -> anyhow::Result<()> {
    anyhow::bail!("macOS app bundles are only supported on macOS")
}

#[cfg(not(target_os = "macos"))]
pub fn uninstall_app_bundles(_options: &InstallOptions) -> anyhow::Result<()> {
    anyhow::bail!("macOS app bundles are only supported on macOS")
}

#[cfg(target_os = "macos")]
fn write_bundle(bundle: &MacosAppBundle) -> anyhow::Result<()> {
    let contents = bundle.app_path.join("Contents");
    let macos = contents.join("MacOS");
    let resources = contents.join("Resources");
    fs::create_dir_all(&macos)?;
    fs::create_dir_all(&resources)?;
    fs::write(contents.join("Info.plist"), &bundle.info_plist)?;
    let executable = macos.join(executable_name_from_plist(&bundle.info_plist));
    fs::write(&executable, &bundle.launch_script)?;
    let mut permissions = fs::metadata(&executable)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(executable, permissions)?;
    copy_icon(&resources)?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn copy_icon(resources: &Path) -> anyhow::Result<()> {
    let source = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .map(|path| path.join("codex-plus-plus.png"));
    if let Some(source) = source.filter(|path| path.exists()) {
        fs::copy(source, resources.join("codex-plus-plus.png"))?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn executable_name_from_plist(plist: &str) -> String {
    plist
        .split("<key>CFBundleExecutable</key>")
        .nth(1)
        .and_then(|tail| tail.split("<string>").nth(1))
        .and_then(|tail| tail.split("</string>").next())
        .unwrap_or("CodexPlusPlus")
        .to_string()
}

fn info_plist(display_name: &str, executable_name: &str, identifier_suffix: &str) -> String {
    let version = crate::version::VERSION;
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key>
  <string>{display_name}</string>
  <key>CFBundleDisplayName</key>
  <string>{display_name}</string>
  <key>CFBundleIdentifier</key>
  <string>com.bigpizzav3.codexplusplus{identifier_suffix}</string>
  <key>CFBundleVersion</key>
  <string>{version}</string>
  <key>CFBundleShortVersionString</key>
  <string>{version}</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleExecutable</key>
  <string>{executable_name}</string>
  <key>CFBundleIconFile</key>
  <string>codex-plus-plus.png</string>
  <key>LSUIElement</key>
  <true/>
  <key>LSMinimumSystemVersion</key>
  <string>12.0</string>
</dict>
</plist>"#
    )
}
