#[cfg(target_os = "macos")]
use std::fs;
#[cfg(target_os = "macos")]
use std::os::unix::fs::PermissionsExt;
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
    let binary_source = install_binary_source(
        option_or_current_exe(
            if manager {
                &options.manager_path
            } else {
                &options.launcher_path
            },
            binary,
        ),
        binary,
    );
    let identifier_suffix = if manager { ".manager" } else { "" };
    MacosAppBundle {
        app_path: install_root.join(format!("{display_name}.app")),
        info_plist: info_plist(display_name, executable_name, identifier_suffix),
        launch_script: launch_script(binary),
        binary_source: Some(binary_source),
        binary_target_name: Some(binary.to_string()),
    }
}

fn launch_script(binary: &str) -> String {
    format!(
        "#!/bin/sh\nDIR=$(CDPATH= cd -- \"$(dirname -- \"$0\")\" && pwd)\nexec \"$DIR/{binary}\" \"$@\"\n"
    )
}

fn install_binary_source(target: std::path::PathBuf, binary: &str) -> std::path::PathBuf {
    let sidecar = if is_bundle_macos_target(&target) {
        let sidecar = target
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(binary);
        sidecar.exists().then_some(sidecar)
    } else {
        None
    };
    if let Some(sidecar) = sidecar {
        return sidecar;
    }
    if target == Path::new(".") || !target.exists() || is_shell_script(&target) {
        if let Ok(exe) = std::env::current_exe() {
            let companion = exe
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(binary);
            if companion.exists() && !is_shell_script(&companion) {
                return companion;
            }
        }
    }
    target
}

fn is_bundle_macos_target(target: &Path) -> bool {
    target
        .parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
        == Some("MacOS")
        && target
            .parent()
            .and_then(|parent| parent.parent())
            .and_then(|parent| parent.file_name())
            .and_then(|name| name.to_str())
            == Some("Contents")
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
    let (source, target_name) = match (&bundle.binary_source, &bundle.binary_target_name) {
        (Some(source), Some(target_name)) => (source, target_name),
        _ => anyhow::bail!(
            "缺少二进制源信息，无法生成可启动的 App bundle（路径：{}）",
            bundle.app_path.display()
        ),
    };
    if !source.exists() {
        anyhow::bail!(
            "二进制源不存在：{}。请从 Codex++ DMG 重新安装，或确认 {} 未被移动/删除",
            source.display(),
            bundle.app_path.display()
        );
    }
    if is_shell_script(source) {
        anyhow::bail!(
            "二进制源是 shell 脚本而非可执行文件：{}。该 bundle 已损坏，请从 Codex++ DMG 重新安装",
            source.display()
        );
    }
    let target = macos.join(target_name);
    if source != &target {
        fs::copy(source, &target)?;
    } else if !is_real_binary(&target) {
        anyhow::bail!(
            "二进制位于 bundle 自身（{}），但目标文件缺失或非可执行文件，无法自修复。请从 Codex++ DMG 重新安装",
            target.display()
        );
    }
    fs::set_permissions(&target, fs::Permissions::from_mode(0o755))?;
    let executable = macos.join(executable_name_from_plist(&bundle.info_plist));
    fs::write(&executable, &bundle.launch_script)?;
    let mut permissions = fs::metadata(&executable)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(executable, permissions)?;
    copy_icon(&resources)?;
    Ok(())
}

fn is_shell_script(path: &Path) -> bool {
    fs::read(path)
        .ok()
        .map(|bytes| bytes.starts_with(b"#!"))
        .unwrap_or(false)
}

fn is_real_binary(path: &Path) -> bool {
    fs::metadata(path)
        .map(|metadata| metadata.len() > 1024 && !metadata.is_dir())
        .unwrap_or(false)
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
    let url_types = if identifier_suffix == ".manager" {
        r#"  <key>CFBundleURLTypes</key>
  <array>
    <dict>
      <key>CFBundleURLName</key>
      <string>Codex++ Links</string>
      <key>CFBundleURLSchemes</key>
      <array>
        <string>codexplusplus</string>
        <string>dreamskin</string>
      </array>
    </dict>
  </array>
"#
    } else {
        ""
    };
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
{url_types}  <key>LSUIElement</key>
  <true/>
  <key>LSMinimumSystemVersion</key>
  <string>12.0</string>
</dict>
</plist>"#
    )
}
