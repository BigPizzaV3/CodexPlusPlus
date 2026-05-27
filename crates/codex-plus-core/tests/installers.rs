use codex_plus_core::install::{
    InstallOptions, MANAGER_BINARY, SILENT_BINARY, app_bundle_names, build_macos_app_bundle,
    build_windows_entrypoint_plan, companion_binary_path_from_exe, default_install_root_strategy,
    install_entrypoints, macos_launch_script_writes_executable, option_or_current_exe_from_exe,
    shortcut_names,
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
    assert!(silent.launch_script.contains("codex-plus-plus"));
    assert!(manager.launch_script.contains("codex-plus-plus-manager"));
}

#[test]
fn installer_exports_expected_two_entrypoint_names() {
    assert_eq!(shortcut_names(), ("Codex++.lnk", "Codex++ 管理工具.lnk"));
    assert_eq!(app_bundle_names(), ("Codex++.app", "Codex++ 管理工具.app"));
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
fn macos_manager_app_resolves_installed_silent_launcher_app() {
    let manager_exe = std::path::Path::new(
        "/Applications/Codex++ 管理工具.app/Contents/MacOS/CodexPlusPlusManager",
    );
    let install_root = std::path::Path::new("/Applications");
    let target =
        option_or_current_exe_from_exe(manager_exe, &None, SILENT_BINARY, Some(install_root));

    assert_eq!(
        target,
        std::path::PathBuf::from("/Applications/Codex++.app/Contents/MacOS/CodexPlusPlus")
    );
}

#[test]
fn macos_silent_app_resolves_installed_manager_app() {
    let launcher_exe =
        std::path::Path::new("/Applications/Codex++.app/Contents/MacOS/CodexPlusPlus");
    let install_root = std::path::Path::new("/Applications");
    let target =
        option_or_current_exe_from_exe(launcher_exe, &None, MANAGER_BINARY, Some(install_root));

    assert_eq!(
        target,
        std::path::PathBuf::from(
            "/Applications/Codex++ 管理工具.app/Contents/MacOS/CodexPlusPlusManager"
        )
    );
}

#[test]
fn macos_manager_app_resolves_its_own_executable_for_manager_entry() {
    let manager_exe = std::path::Path::new(
        "/Applications/Codex++ 管理工具.app/Contents/MacOS/CodexPlusPlusManager",
    );
    let install_root = std::path::Path::new("/Applications");
    let target =
        option_or_current_exe_from_exe(manager_exe, &None, MANAGER_BINARY, Some(install_root));

    assert_eq!(
        target,
        std::path::PathBuf::from(
            "/Applications/Codex++ 管理工具.app/Contents/MacOS/CodexPlusPlusManager"
        )
    );
}

#[test]
fn macos_installed_app_bundle_does_not_overwrite_its_own_executable_with_script() {
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

    assert!(!macos_launch_script_writes_executable(&silent));
    assert!(!macos_launch_script_writes_executable(&manager));
}

#[cfg(target_os = "macos")]
#[test]
fn macos_install_preserves_existing_installed_app_executables() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let launcher = root
        .join("Codex++.app")
        .join("Contents")
        .join("MacOS")
        .join("CodexPlusPlus");
    let manager = root
        .join("Codex++ 管理工具.app")
        .join("Contents")
        .join("MacOS")
        .join("CodexPlusPlusManager");
    std::fs::create_dir_all(launcher.parent().unwrap()).unwrap();
    std::fs::create_dir_all(manager.parent().unwrap()).unwrap();
    std::fs::write(&launcher, "launcher-binary").unwrap();
    std::fs::write(&manager, "manager-binary").unwrap();

    let result = install_entrypoints(&InstallOptions {
        install_root: Some(root.into()),
        launcher_path: Some(launcher.clone()),
        manager_path: Some(manager.clone()),
        remove_owned_data: false,
    });

    assert_eq!(result.status, "ok");
    assert_eq!(
        std::fs::read_to_string(launcher).unwrap(),
        "launcher-binary"
    );
    assert_eq!(std::fs::read_to_string(manager).unwrap(), "manager-binary");
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
