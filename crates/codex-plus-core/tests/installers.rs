use codex_plus_core::install::{
    InstallOptions, MANAGER_BUNDLE_ID, SILENT_BINARY, SILENT_BUNDLE_ID, app_bundle_names,
    build_macos_app_bundle, build_windows_entrypoint_plan, companion_binary_path_from_exe,
    default_install_root_strategy, macos_companion_bundle_identifier_from_exe, shortcut_names,
};

#[test]
fn windows_entrypoint_plan_contains_silent_and_manager_entrypoints() {
    let options = InstallOptions {
        install_root: Some("C:/Users/A/Desktop".into()),
        launcher_path: Some("C:/Tools/codex-plus-plus.exe".into()),
        manager_path: Some("C:/Tools/codex-plus-plus-manager.exe".into()),
        remove_owned_data: false,
    };

    let plan = build_windows_entrypoint_plan(&options);

    assert!(plan.silent_shortcut.ends_with("Codex++.lnk"));
    assert!(plan.manager_shortcut.ends_with("Codex++ 管理工具.lnk"));
    assert_eq!(plan.launcher_path, "C:/Tools/codex-plus-plus.exe");
    assert_eq!(plan.manager_path, "C:/Tools/codex-plus-plus-manager.exe");
    assert_eq!(plan.silent_icon_path, "C:/Tools/codex-plus-plus.exe");
    assert_eq!(
        plan.manager_icon_path,
        "C:/Tools/codex-plus-plus-manager.exe"
    );
    assert_eq!(plan.uninstall_key, "CodexPlusPlus");
    assert_eq!(plan.legacy_uninstall_key, "Codex++");
    assert_eq!(
        plan.uninstaller_path.replace('\\', "/"),
        "C:/Tools/uninstall.exe"
    );
    assert_eq!(
        plan.uninstall_command.replace('\\', "/"),
        "\"C:/Tools/uninstall.exe\""
    );
    assert_eq!(
        plan.quiet_uninstall_command.replace('\\', "/"),
        "\"C:/Tools/uninstall.exe\" /S"
    );
    assert_ne!(
        plan.uninstall_command,
        "\"C:/Tools/codex-plus-plus-manager.exe\""
    );
}

#[test]
fn windows_entrypoint_plan_can_request_owned_data_removal_without_shell_script() {
    let options = InstallOptions {
        install_root: Some("C:/Users/A/Desktop".into()),
        launcher_path: None,
        manager_path: None,
        remove_owned_data: true,
    };

    let plan = build_windows_entrypoint_plan(&options);

    assert!(plan.silent_shortcut.ends_with("Codex++.lnk"));
    assert!(plan.manager_shortcut.ends_with("Codex++ 管理工具.lnk"));
    assert!(plan.remove_owned_data);
}

#[test]
fn macos_bundle_metadata_contains_silent_and_manager_apps() {
    let options = InstallOptions {
        install_root: Some("/Applications".into()),
        launcher_path: Some("/opt/Codex++/codex-plus-plus".into()),
        manager_path: Some("/opt/Codex++/codex-plus-plus-manager".into()),
        remove_owned_data: false,
    };

    let silent = build_macos_app_bundle(&options, false);
    let manager = build_macos_app_bundle(&options, true);

    assert!(silent.app_path.ends_with("Codex++.app"));
    assert!(manager.app_path.ends_with("Codex++ 管理工具.app"));
    assert!(silent.info_plist.contains("<string>Codex++</string>"));
    assert!(
        manager
            .info_plist
            .contains("<string>Codex++ 管理工具</string>")
    );
    assert!(manager.info_plist.contains("<string>dreamskin</string>"));
    assert!(
        manager
            .info_plist
            .contains("<string>codexplusplus</string>")
    );
    assert!(!silent.info_plist.contains("<string>dreamskin</string>"));
    assert_eq!(
        silent.binary_target_name.as_deref(),
        Some("codex-plus-plus")
    );
    assert_eq!(
        manager.binary_target_name.as_deref(),
        Some("codex-plus-plus-manager")
    );
    assert!(silent.launch_script.contains("$DIR/codex-plus-plus"));
    assert!(
        manager
            .launch_script
            .contains("$DIR/codex-plus-plus-manager")
    );
}

#[test]
fn installer_exports_expected_two_entrypoint_names() {
    assert_eq!(shortcut_names(), ("Codex++.lnk", "Codex++ 管理工具.lnk"));
    assert_eq!(app_bundle_names(), ("Codex++.app", "Codex++ 管理工具.app"));
}

#[test]
fn windows_installer_writes_the_same_uninstall_key_as_the_runtime() {
    // issue #2339：NSIS 曾写/删 Uninstall\Codex++，而运行时 install::windows
    // 把 Uninstall\CodexPlusPlus 当正式键、Uninstall\Codex++ 当 legacy。
    // 两侧不一致会让卸载项残留（两种安装顺序各留一条、且都删不干净）。
    let nsi = std::fs::read_to_string("../../scripts/installer/windows/CodexPlusPlus.nsi")
        .expect("read Windows NSIS installer script");

    assert!(
        nsi.contains(r"Uninstall\CodexPlusPlus"),
        "安装器必须写正式卸载键 CodexPlusPlus，否则与运行时不一致"
    );
    assert!(
        nsi.contains(r#"DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\CodexPlusPlus""#),
        "卸载段必须删除正式键"
    );
    assert!(
        nsi.contains(
            r#"DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Codex++""#
        ),
        "卸载段必须保留 legacy 键清理：存量用户靠它收尸"
    );
    // 不得再往 legacy 键写卸载项，否则两条并存的问题会复发。
    assert!(
        !nsi.contains(
            r#"WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Codex++""#
        ),
        "不得再向 legacy 卸载键写入"
    );
}

#[test]
fn windows_installer_only_creates_desktop_shortcuts_on_first_install() {
    let nsi = std::fs::read_to_string("../../scripts/installer/windows/CodexPlusPlus.nsi")
        .expect("read Windows NSIS installer script");
    // Windows checkout 默认可转成 CRLF；两种换行都必须验证相同跳转策略。
    let lf = nsi.replace("\r\n", "\n");
    assert_first_install_shortcut_policy(&lf);
    assert_first_install_shortcut_policy(&lf.replace('\n', "\r\n"));
}

fn assert_first_install_shortcut_policy(nsi: &str) {
    let nsi = nsi.replace("\r\n", "\n");
    // 与运行时共用正式/legacy 登记；不能在写本轮登记之后才判断首次安装。
    for key in ["CodexPlusPlus", "Codex++"] {
        let read = format!(
            r#"ReadRegStr $ExistingInstall HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\{key}" "InstallLocation""#
        );
        assert!(
            nsi.find(&read).expect("read previous install registration")
                < nsi.find("CreateShortcut").unwrap()
        );
    }
    // 缺图标只在首次安装时补建。守住两个跳转方向，避免再次把已删除图标加回来。
    for (name, label) in [
        ("Codex++", "desktop_silent_done"),
        ("Codex++ 管理工具", "desktop_manager_done"),
    ] {
        let branch = format!(
            "StrCmp $ExistingInstall \"\" 0 {label}\n  IfFileExists \"$DESKTOP\\{name}.lnk\" {label} 0\n  CreateShortcut \"$DESKTOP\\{name}.lnk\""
        );
        assert!(nsi.contains(&branch));
    }
    assert!(nsi.contains(r#"CreateShortcut "$SMPROGRAMS\Codex++\Codex++.lnk""#));
    assert!(nsi.contains(r#"CreateShortcut "$SMPROGRAMS\Codex++\Codex++ 管理工具.lnk""#));
}

#[test]
fn macos_dmg_includes_applications_shortcut_for_drag_install() {
    let script = std::fs::read_to_string("../../scripts/installer/macos/package-dmg.sh")
        .expect("read macOS DMG packaging script");

    assert!(script.contains("ln -s /Applications \"$STAGE/Applications\""));
}

#[test]
fn companion_binary_path_resolves_macos_silent_app_next_to_manager_app() {
    let manager_exe = std::path::Path::new(
        "/Applications/Codex++ 管理工具.app/Contents/MacOS/CodexPlusPlusManager",
    );

    let companion = companion_binary_path_from_exe(manager_exe, SILENT_BINARY);

    assert_eq!(
        companion,
        std::path::PathBuf::from("/Applications/Codex++.app/Contents/MacOS/CodexPlusPlus")
    );
    assert_ne!(
        companion,
        std::path::PathBuf::from(
            "/Applications/Codex++ 管理工具.app/Contents/MacOS/codex-plus-plus"
        )
    );
}

#[test]
fn companion_binary_path_resolves_macos_manager_app_next_to_silent_app() {
    let silent_exe = std::path::Path::new("/Applications/Codex++.app/Contents/MacOS/CodexPlusPlus");

    let companion =
        companion_binary_path_from_exe(silent_exe, codex_plus_core::install::MANAGER_BINARY);

    assert_eq!(
        companion,
        std::path::PathBuf::from(
            "/Applications/Codex++ 管理工具.app/Contents/MacOS/CodexPlusPlusManager"
        )
    );
}

#[test]
fn macos_companion_launch_uses_bundle_ids_from_app_translocation() {
    let manager_exe = std::path::Path::new(
        "/private/var/folders/x/AppTranslocation/manager-id/d/Codex++ 管理工具.app/Contents/MacOS/CodexPlusPlusManager",
    );
    let silent_exe = std::path::Path::new(
        "/private/var/folders/x/AppTranslocation/silent-id/d/Codex++.app/Contents/MacOS/CodexPlusPlus",
    );

    assert_eq!(
        macos_companion_bundle_identifier_from_exe(manager_exe, SILENT_BINARY),
        Some(SILENT_BUNDLE_ID)
    );
    assert_eq!(
        macos_companion_bundle_identifier_from_exe(
            silent_exe,
            codex_plus_core::install::MANAGER_BINARY,
        ),
        Some(MANAGER_BUNDLE_ID)
    );
}

#[test]
fn macos_companion_launch_keeps_bare_binary_development_mode() {
    let manager_exe = std::path::Path::new("/tmp/target/debug/codex-plus-plus-manager");

    assert_eq!(
        macos_companion_bundle_identifier_from_exe(manager_exe, SILENT_BINARY),
        None
    );
}

#[cfg(target_os = "macos")]
#[test]
fn macos_companion_path_falls_back_to_workspace_release_launcher() {
    let root = tempfile::tempdir().unwrap();
    let bundle_exe = root.path().join(
        "target/release/bundle/macos/Codex++ Manager.app/Contents/MacOS/codex-plus-plus-manager",
    );
    let release_launcher = root.path().join("target/release/codex-plus-plus");
    std::fs::create_dir_all(bundle_exe.parent().unwrap()).unwrap();
    std::fs::create_dir_all(release_launcher.parent().unwrap()).unwrap();
    std::fs::write(&bundle_exe, b"manager").unwrap();
    std::fs::write(&release_launcher, b"launcher").unwrap();

    assert_eq!(
        companion_binary_path_from_exe(&bundle_exe, SILENT_BINARY),
        release_launcher
    );
}

#[test]
fn macos_bundle_does_not_wrap_the_bundle_executable_in_itself() {
    let options = InstallOptions {
        install_root: Some("/Applications".into()),
        launcher_path: Some("/Applications/Codex++.app/Contents/MacOS/CodexPlusPlus".into()),
        manager_path: Some(
            "/Applications/Codex++ 管理工具.app/Contents/MacOS/CodexPlusPlusManager".into(),
        ),
        remove_owned_data: false,
    };

    let silent = build_macos_app_bundle(&options, false);
    let manager = build_macos_app_bundle(&options, true);

    assert_eq!(
        silent.binary_source,
        Some(std::path::PathBuf::from(
            "/Applications/Codex++.app/Contents/MacOS/CodexPlusPlus"
        ))
    );
    assert_eq!(
        manager.binary_source,
        Some(std::path::PathBuf::from(
            "/Applications/Codex++ 管理工具.app/Contents/MacOS/CodexPlusPlusManager"
        ))
    );
    assert!(silent.launch_script.contains("$DIR/codex-plus-plus"));
    assert!(
        manager
            .launch_script
            .contains("$DIR/codex-plus-plus-manager")
    );
}

#[test]
fn windows_default_install_root_uses_known_folder_before_userprofile_desktop() {
    let strategy = default_install_root_strategy();

    if cfg!(windows) {
        assert_eq!(strategy, "windows-known-folder");
    } else if cfg!(target_os = "macos") {
        assert_eq!(strategy, "macos-applications");
    } else {
        assert_eq!(strategy, "user-dirs-desktop");
    }
}

#[cfg(target_os = "macos")]
mod macos_repair {
    use super::*;
    use std::path::{Path, PathBuf};

    fn native_options(root: &Path) -> InstallOptions {
        InstallOptions {
            install_root: Some(root.to_path_buf()),
            launcher_path: Some(root.join("Codex++.app/Contents/MacOS/CodexPlusPlus")),
            manager_path: Some(
                root.join("Codex++ 管理工具.app/Contents/MacOS/CodexPlusPlusManager"),
            ),
            remove_owned_data: false,
        }
    }

    fn write_binary(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut bytes = vec![0_u8; 2048];
        bytes[..4].copy_from_slice(&[0xcf, 0xfa, 0xed, 0xfe]);
        std::fs::write(path, bytes).unwrap();
    }

    fn write_native_app(root: &Path, manager: bool) -> Vec<PathBuf> {
        let (app, executable) = if manager {
            ("Codex++ 管理工具.app", "CodexPlusPlusManager")
        } else {
            ("Codex++.app", "CodexPlusPlus")
        };
        let contents = root.join(app).join("Contents");
        let binary = contents.join("MacOS").join(executable);
        write_binary(&binary);
        std::fs::create_dir_all(contents.join("_CodeSignature")).unwrap();
        std::fs::create_dir_all(contents.join("Resources")).unwrap();
        let plist = contents.join("Info.plist");
        std::fs::write(
            &plist,
            format!(
                "<plist><dict><key>CFBundleExecutable</key><string>{executable}</string><key>CFBundleIconFile</key><string>native.icns</string><key>LSUIElement</key><false/></dict></plist>"
            ),
        )
        .unwrap();
        let signature = contents.join("_CodeSignature/CodeResources");
        std::fs::write(&signature, b"sealed resource sentinel").unwrap();
        let icon = contents.join("Resources/native.icns");
        std::fs::write(&icon, b"native icon sentinel").unwrap();
        vec![plist, binary, signature, icon]
    }

    fn snapshot(paths: &[PathBuf]) -> Vec<Vec<u8>> {
        paths
            .iter()
            .map(|path| std::fs::read(path).unwrap())
            .collect()
    }

    #[test]
    fn repair_preserves_native_bundles_and_their_sealed_files() {
        let root = tempfile::tempdir().unwrap();
        let paths = [
            write_native_app(root.path(), false),
            write_native_app(root.path(), true),
        ]
        .concat();
        let before = snapshot(&paths);
        let options = native_options(root.path());

        for _ in 0..2 {
            codex_plus_core::install::macos::install_app_bundles(&options).unwrap();
            assert_eq!(snapshot(&paths), before);
        }
        assert!(
            !root
                .path()
                .join("Codex++.app/Contents/MacOS/codex-plus-plus")
                .exists()
        );
        assert!(
            !root
                .path()
                .join("Codex++ 管理工具.app/Contents/MacOS/codex-plus-plus-manager")
                .exists()
        );
    }

    #[test]
    fn repair_preserves_native_bundles_referenced_through_symlinks() {
        let root = tempfile::tempdir().unwrap();
        let aliases = tempfile::tempdir().unwrap();
        let paths = [
            write_native_app(root.path(), false),
            write_native_app(root.path(), true),
        ]
        .concat();
        let before = snapshot(&paths);
        let alias = aliases.path().join("apps");
        std::os::unix::fs::symlink(root.path(), &alias).unwrap();
        let mut options = native_options(&alias);
        options.install_root = Some(root.path().to_path_buf());

        codex_plus_core::install::macos::install_app_bundles(&options).unwrap();

        assert_eq!(snapshot(&paths), before);
    }

    #[test]
    fn repair_preserves_native_bundles_referenced_through_hard_links() {
        let root = tempfile::tempdir().unwrap();
        let paths = [
            write_native_app(root.path(), false),
            write_native_app(root.path(), true),
        ]
        .concat();
        let before = snapshot(&paths);
        let mut options = native_options(root.path());
        let launcher_alias = root.path().join("launcher-alias");
        std::fs::hard_link(options.launcher_path.as_ref().unwrap(), &launcher_alias).unwrap();
        options.launcher_path = Some(launcher_alias);

        codex_plus_core::install::macos::install_app_bundles(&options).unwrap();

        assert_eq!(snapshot(&paths), before);
    }

    #[test]
    fn missing_companion_does_not_modify_the_installed_app() {
        let root = tempfile::tempdir().unwrap();
        let paths = write_native_app(root.path(), false);
        let before = snapshot(&paths);

        assert!(
            codex_plus_core::install::macos::install_app_bundles(&native_options(root.path()))
                .is_err()
        );

        assert_eq!(snapshot(&paths), before);
        assert!(!root.path().join("Codex++ 管理工具.app").exists());
    }

    #[test]
    fn missing_companion_does_not_create_a_partial_new_installation() {
        let root = tempfile::tempdir().unwrap();
        let launcher = root.path().join("bin/codex-plus-plus");
        write_binary(&launcher);
        let options = InstallOptions {
            install_root: Some(root.path().join("apps")),
            launcher_path: Some(launcher),
            manager_path: Some(root.path().join("bin/missing-manager")),
            remove_owned_data: false,
        };

        assert!(codex_plus_core::install::macos::install_app_bundles(&options).is_err());

        assert!(!root.path().join("apps").exists());
    }

    #[test]
    fn rebuilding_a_signed_bundle_from_another_source_fails_without_writes() {
        let root = tempfile::tempdir().unwrap();
        let paths = [
            write_native_app(root.path(), false),
            write_native_app(root.path(), true),
        ]
        .concat();
        let before = snapshot(&paths);
        let launcher = root.path().join("bin/codex-plus-plus");
        write_binary(&launcher);
        let mut options = native_options(root.path());
        options.launcher_path = Some(launcher);

        let error = codex_plus_core::install::macos::install_app_bundles(&options).unwrap_err();

        assert!(error.to_string().contains("请重新安装"));
        assert_eq!(snapshot(&paths), before);
    }

    #[test]
    fn bare_binary_installation_and_unsigned_wrapper_repair_still_work() {
        let root = tempfile::tempdir().unwrap();
        let launcher = root.path().join("bin/codex-plus-plus");
        let manager = root.path().join("bin/codex-plus-plus-manager");
        write_binary(&launcher);
        write_binary(&manager);
        let options = InstallOptions {
            install_root: Some(root.path().join("apps")),
            launcher_path: Some(launcher.clone()),
            manager_path: Some(manager.clone()),
            remove_owned_data: false,
        };
        codex_plus_core::install::macos::install_app_bundles(&options).unwrap();
        let installed = native_options(options.install_root.as_ref().unwrap());
        codex_plus_core::install::macos::install_app_bundles(&installed).unwrap();

        assert_eq!(
            std::fs::read(
                installed
                    .launcher_path
                    .as_ref()
                    .unwrap()
                    .with_file_name("codex-plus-plus")
            )
            .unwrap(),
            std::fs::read(launcher).unwrap()
        );
        assert_eq!(
            std::fs::read(
                installed
                    .manager_path
                    .as_ref()
                    .unwrap()
                    .with_file_name("codex-plus-plus-manager")
            )
            .unwrap(),
            std::fs::read(manager).unwrap()
        );
        assert!(
            std::fs::read_to_string(installed.launcher_path.unwrap())
                .unwrap()
                .contains("exec \"$DIR/codex-plus-plus\"")
        );
        assert!(
            std::fs::read_to_string(installed.manager_path.unwrap())
                .unwrap()
                .contains("exec \"$DIR/codex-plus-plus-manager\"")
        );
    }
}
