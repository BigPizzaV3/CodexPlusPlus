#[cfg(target_os = "macos")]
use std::fs;
#[cfg(target_os = "macos")]
use std::os::unix::fs::MetadataExt;
#[cfg(target_os = "macos")]
use std::path::{Path, PathBuf};

#[cfg(target_os = "macos")]
use anyhow::Context;

use super::{
    APP_BUNDLE_ID, APP_NAME, InstallOptions, MACOS_APP_EXECUTABLE, MacosAppBundle,
    application_source, install_root_or_default,
};

#[cfg(target_os = "macos")]
const LEGACY_MANAGER_BUNDLE_NAME: &str = "Codex++ 管理工具.app";

pub fn build_app_bundle(options: &InstallOptions, _manager: bool) -> MacosAppBundle {
    // 保留 manager 参数供已有调用方使用；安装计划始终只有同一个原生界面 app。
    let binary_source = application_source(options);
    MacosAppBundle {
        app_path: install_root_or_default(options).join(format!("{APP_NAME}.app")),
        info_plist: info_plist(),
        launch_script: String::new(),
        binary_source: Some(binary_source),
        binary_target_name: Some(MACOS_APP_EXECUTABLE.to_string()),
    }
}

#[cfg(target_os = "macos")]
pub fn install_app_bundles(options: &InstallOptions) -> anyhow::Result<()> {
    let plan = build_app_bundle(options, false);
    let source = plan
        .binary_source
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("macOS app 缺少来源"))?;
    let destination_executable = plan
        .app_path
        .join("Contents/MacOS")
        .join(MACOS_APP_EXECUTABLE);
    // 原生 bundle 修复只校验，不写 plist、入口或签名资源；硬链接/符号链接也一样。
    if paths_refer_to_same_file(source, &destination_executable) {
        validate_native_bundle(&plan.app_path)?;
        return Ok(());
    }
    let source_bundle = native_application_bundle_from_executable(source)?;
    if plan.app_path.exists() {
        anyhow::bail!(
            "目标 app 已存在，请重新安装完整 Codex++.app；不会改写包内签名文件：{}",
            plan.app_path.display()
        );
    }
    // 完整复制原生包，保留签名、资源和扩展属性；旧双 app 不在这里自动删除。
    fs::create_dir_all(plan.app_path.parent().unwrap_or_else(|| Path::new(".")))?;
    let result = std::process::Command::new("/usr/bin/ditto")
        .args(["--rsrc", "--extattr"])
        .arg(&source_bundle)
        .arg(&plan.app_path)
        .output()?;
    if !result.status.success() {
        anyhow::bail!(
            "复制 Codex++.app 失败：{}",
            String::from_utf8_lossy(&result.stderr).trim()
        );
    }
    validate_native_bundle(&plan.app_path)?;
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn uninstall_app_bundles(options: &InstallOptions) -> anyhow::Result<()> {
    let app = install_root_or_default(options).join(format!("{APP_NAME}.app"));
    if app.exists() {
        fs::remove_dir_all(app)?;
    }
    Ok(())
}

/// 将旧双应用布局中的管理工具移到可恢复的迁移备份目录。
///
/// 旧版更新器只能处理两个 app，因此它无法直接安装新版单 app DMG。
/// 新版首次启动时执行这一步，清理 Applications 中的旧入口，同时保留完整
/// bundle 供用户需要时恢复；后续更新不会再次触发迁移。
#[cfg(target_os = "macos")]
pub fn migrate_legacy_manager_bundle(root: &Path) -> anyhow::Result<Option<PathBuf>> {
    migrate_legacy_manager_bundle_to(
        root,
        &crate::paths::default_app_state_dir().join("legacy-migrations"),
    )
}

#[cfg(target_os = "macos")]
fn migrate_legacy_manager_bundle_to(
    root: &Path,
    archive_root: &Path,
) -> anyhow::Result<Option<PathBuf>> {
    let legacy = root.join(LEGACY_MANAGER_BUNDLE_NAME);
    if !legacy.exists() {
        return Ok(None);
    }
    if fs::symlink_metadata(&legacy)?.file_type().is_symlink() {
        anyhow::bail!("旧管理工具是符号链接，未自动迁移：{}", legacy.display());
    }
    fs::create_dir_all(archive_root)?;
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let archive = archive_root.join(format!("legacy-manager-{timestamp}-{}", std::process::id()));
    fs::rename(&legacy, &archive).with_context(|| {
        format!(
            "迁移旧管理工具失败：{} → {}",
            legacy.display(),
            archive.display()
        )
    })?;
    Ok(Some(archive))
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
/// 返回当前主程序所属的完整新版界面包；允许包目录改名，拒绝旧静默包和辅助文件。
pub fn native_application_bundle_from_executable(executable: &Path) -> anyhow::Result<PathBuf> {
    let canonical = fs::canonicalize(executable).map_err(|error| {
        anyhow::anyhow!(
            "请从完整的 Codex++.app 安装；可执行文件不可用：{}：{error}",
            executable.display()
        )
    })?;
    let app = canonical
        .parent()
        .filter(|macos| macos.file_name().is_some_and(|name| name == "MacOS"))
        .and_then(Path::parent)
        .filter(|contents| contents.file_name().is_some_and(|name| name == "Contents"))
        .and_then(Path::parent)
        .filter(|app| app.extension().is_some_and(|extension| extension == "app"))
        .ok_or_else(|| {
            anyhow::anyhow!("请从完整的 Codex++.app 安装；不再从裸二进制重建 macOS app。")
        })?;
    validate_native_bundle(app)?;
    if !paths_refer_to_same_file(
        &canonical,
        &app.join("Contents/MacOS").join(MACOS_APP_EXECUTABLE),
    ) {
        anyhow::bail!(
            "当前可执行文件不是 Codex++ 原生界面的主程序：{}",
            executable.display()
        );
    }
    Ok(app.to_path_buf())
}

#[cfg(target_os = "macos")]
fn paths_refer_to_same_file(left: &Path, right: &Path) -> bool {
    match (fs::metadata(left), fs::metadata(right)) {
        (Ok(left), Ok(right)) => left.dev() == right.dev() && left.ino() == right.ino(),
        _ => false,
    }
}

#[cfg(target_os = "macos")]
pub(super) fn validate_native_bundle(app: &Path) -> anyhow::Result<()> {
    let plist = fs::read_to_string(app.join("Contents/Info.plist")).map_err(|error| {
        anyhow::anyhow!(
            "macOS app 缺少有效 Info.plist，请重新安装：{}：{error}",
            app.display()
        )
    })?;
    if plist_value(&plist, "CFBundleIdentifier") != Some(APP_BUNDLE_ID)
        || plist_value(&plist, "CFBundleExecutable") != Some(MACOS_APP_EXECUTABLE)
        || plist_value(&plist, "CFBundleName") != Some(APP_NAME)
        || !plist_true_value(&plist, "CodexPlusUnifiedApp")
    {
        anyhow::bail!(
            "macOS app 不是完整的 Codex++ 原生界面包，请重新安装：{}",
            app.display()
        );
    }
    let executable = app.join("Contents/MacOS").join(MACOS_APP_EXECUTABLE);
    let metadata = fs::metadata(&executable)?;
    if !metadata.is_file() || metadata.len() < 1024 {
        anyhow::bail!("Codex++ 可执行文件缺失或无效：{}", executable.display());
    }
    if !fs::canonicalize(&executable)?.starts_with(fs::canonicalize(app)?) {
        anyhow::bail!(
            "Codex++ 可执行文件越出 app 包目录：{}",
            executable.display()
        );
    }
    use std::io::Read;
    let mut magic = [0_u8; 4];
    fs::File::open(&executable)?.read_exact(&mut magic)?;
    if !matches!(
        u32::from_be_bytes(magic),
        0xfeedface
            | 0xcefaedfe
            | 0xfeedfacf
            | 0xcffaedfe
            | 0xcafebabe
            | 0xbebafeca
            | 0xcafebabf
            | 0xbfbafeca
    ) {
        anyhow::bail!(
            "Codex++ 可执行文件不是 Mach-O；不会生成脚本入口：{}",
            executable.display()
        );
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn plist_value<'a>(plist: &'a str, key: &str) -> Option<&'a str> {
    let (_, tail) = plist.split_once(&format!("<key>{key}</key>"))?;
    let (_, tail) = tail.split_once("<string>")?;
    let (value, _) = tail.split_once("</string>")?;
    Some(value.trim())
}

#[cfg(target_os = "macos")]
fn plist_true_value(plist: &str, key: &str) -> bool {
    plist
        .split_once(&format!("<key>{key}</key>"))
        .is_some_and(|(_, tail)| tail.trim_start().starts_with("<true/>"))
}

#[cfg(all(test, target_os = "macos"))]
mod migration_tests {
    use super::*;

    #[test]
    fn legacy_manager_is_moved_out_of_applications_and_kept_recoverable() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("Applications");
        let archive = temp.path().join("migration-backups");
        let legacy = root.join(LEGACY_MANAGER_BUNDLE_NAME);
        fs::create_dir_all(legacy.join("Contents/MacOS")).unwrap();
        fs::write(
            legacy.join("Contents/MacOS/CodexPlusPlusManager"),
            b"legacy",
        )
        .unwrap();

        let moved = migrate_legacy_manager_bundle_to(&root, &archive)
            .unwrap()
            .expect("legacy manager should be migrated");
        assert!(!legacy.exists());
        assert_eq!(
            fs::read(moved.join("Contents/MacOS/CodexPlusPlusManager")).unwrap(),
            b"legacy"
        );
        assert!(moved.starts_with(&archive));
        assert!(
            migrate_legacy_manager_bundle_to(&root, &archive)
                .unwrap()
                .is_none()
        );
    }
}

fn info_plist() -> String {
    let version = crate::version::VERSION;
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CodexPlusUnifiedApp</key>
  <true/>
  <key>CFBundleName</key>
  <string>{APP_NAME}</string>
  <key>CFBundleDisplayName</key>
  <string>{APP_NAME}</string>
  <key>CFBundleIdentifier</key>
  <string>{APP_BUNDLE_ID}</string>
  <key>CFBundleVersion</key>
  <string>{version}</string>
  <key>CFBundleShortVersionString</key>
  <string>{version}</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleExecutable</key>
  <string>{MACOS_APP_EXECUTABLE}</string>
  <key>CFBundleIconFile</key>
  <string>codex-plus-plus.png</string>
  <key>CFBundleURLTypes</key>
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
  <key>LSUIElement</key>
  <false/>
  <key>LSMinimumSystemVersion</key>
  <string>12.0</string>
</dict>
</plist>"#
    )
}
