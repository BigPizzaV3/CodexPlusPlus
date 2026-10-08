#[cfg(target_os = "macos")]
use std::fs;
#[cfg(target_os = "macos")]
use std::os::unix::fs::{MetadataExt, PermissionsExt};
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
    if is_bundle_macos_target(&target) {
        let sidecar = target
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(binary);
        if sidecar.exists() {
            return sidecar;
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
    let bundles = [
        build_app_bundle(options, false),
        build_app_bundle(options, true),
    ];
    // 两个入口都先检查，避免缺少另一份二进制时已经改动了第一个 app。
    let preserve = [
        preserve_installed_bundle(&bundles[0])?,
        preserve_installed_bundle(&bundles[1])?,
    ];
    for (bundle, preserve) in bundles.iter().zip(preserve) {
        if !preserve {
            write_bundle(bundle)?;
        }
    }
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
fn preserve_installed_bundle(bundle: &MacosAppBundle) -> anyhow::Result<bool> {
    let source = bundle
        .binary_source
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("macOS bundle is missing its binary source"))?;
    validate_binary_source(source)?;
    let contents = bundle.app_path.join("Contents");
    let executable = contents
        .join("MacOS")
        .join(executable_name_from_plist(&bundle.info_plist));
    // DMG 的入口本身就是 Mach-O。不能把正在运行的入口覆成脚本，也不能
    // 重写已签名 app 的 plist/资源（issue #2410）。路径别名也要按同一文件处理。
    if paths_refer_to_same_file(source, &executable) {
        if !contents.join("Info.plist").is_file() {
            anyhow::bail!(
                "macOS app 缺少 Info.plist，请重新安装：{}",
                bundle.app_path.display()
            );
        }
        return Ok(true);
    }
    if contents.join("_CodeSignature").exists() {
        anyhow::bail!(
            "无法安全重建已签名 macOS app，请重新安装：{}",
            bundle.app_path.display()
        );
    }
    Ok(false)
}

#[cfg(target_os = "macos")]
fn paths_refer_to_same_file(left: &Path, right: &Path) -> bool {
    match (fs::metadata(left), fs::metadata(right)) {
        (Ok(left), Ok(right)) => left.dev() == right.dev() && left.ino() == right.ino(),
        _ => false,
    }
}

#[cfg(target_os = "macos")]
fn write_bundle(bundle: &MacosAppBundle) -> anyhow::Result<()> {
    let contents = bundle.app_path.join("Contents");
    let macos = contents.join("MacOS");
    let resources = contents.join("Resources");
    fs::create_dir_all(&macos)?;
    fs::create_dir_all(&resources)?;
    fs::write(contents.join("Info.plist"), &bundle.info_plist)?;
    if let (Some(source), Some(target_name)) = (&bundle.binary_source, &bundle.binary_target_name) {
        validate_binary_source(source)?;
        let target = macos.join(target_name);
        if !paths_refer_to_same_file(source, &target) {
            fs::copy(source, &target)?;
        }
        validate_binary_source(&target)?;
        let mut permissions = fs::metadata(&target)?.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(target, permissions)?;
    } else {
        anyhow::bail!("macOS bundle is missing its binary source");
    }
    let executable = macos.join(executable_name_from_plist(&bundle.info_plist));
    fs::write(&executable, &bundle.launch_script)?;
    let mut permissions = fs::metadata(&executable)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(executable, permissions)?;
    copy_icon(&resources)?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn validate_binary_source(path: &Path) -> anyhow::Result<()> {
    let metadata = fs::metadata(path).map_err(|error| {
        anyhow::anyhow!(
            "macOS bundle binary is unavailable at {}: {error}",
            path.display()
        )
    })?;
    if !metadata.is_file() {
        anyhow::bail!(
            "macOS bundle binary is not a regular file: {}",
            path.display()
        );
    }
    if metadata.len() < 1024 {
        anyhow::bail!(
            "macOS bundle binary is unexpectedly small: {}",
            path.display()
        );
    }
    let mut header = [0_u8; 2];
    let mut file = fs::File::open(path)?;
    use std::io::Read;
    let read = file.read(&mut header)?;
    if read == header.len() && header == *b"#!" {
        anyhow::bail!(
            "macOS bundle binary points to a shell script: {}",
            path.display()
        );
    }
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
