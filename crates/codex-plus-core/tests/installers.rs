use codex_plus_core::install::{
    InstallOptions, SILENT_BINARY, app_bundle_names, build_macos_app_bundle,
    build_windows_entrypoint_plan, companion_binary_path_from_exe, default_install_root_strategy,
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

    assert!(plan.silent_shortcut.ends_with("极义codex.lnk"));
    assert!(plan.manager_shortcut.ends_with("极义codex 管理工具.lnk"));
    assert_eq!(plan.launcher_path, "C:/Tools/codex-plus-plus.exe");
    assert_eq!(plan.manager_path, "C:/Tools/codex-plus-plus-manager.exe");
    assert_eq!(plan.silent_icon_path, "C:/Tools/codex-plus-plus.exe");
    assert_eq!(
        plan.manager_icon_path,
        "C:/Tools/codex-plus-plus-manager.exe"
    );
    assert_eq!(plan.uninstall_key, "JiyiCodex");
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

    assert!(plan.silent_shortcut.ends_with("极义codex.lnk"));
    assert!(plan.manager_shortcut.ends_with("极义codex 管理工具.lnk"));
    assert!(plan.remove_owned_data);
}

#[test]
fn macos_bundle_metadata_contains_silent_and_manager_apps() {
    let options = InstallOptions {
        install_root: Some("/Applications".into()),
        launcher_path: Some("/opt/jiyi-codex/codex-plus-plus".into()),
        manager_path: Some("/opt/jiyi-codex/codex-plus-plus-manager".into()),
        remove_owned_data: false,
    };

    let silent = build_macos_app_bundle(&options, false);
    let manager = build_macos_app_bundle(&options, true);

    assert!(silent.app_path.ends_with("极义codex.app"));
    assert!(manager.app_path.ends_with("极义codex 管理工具.app"));
    assert!(silent.info_plist.contains("<string>极义codex</string>"));
    assert!(
        manager
            .info_plist
            .contains("<string>极义codex 管理工具</string>")
    );
    assert!(silent.launch_script.contains("codex-plus-plus-manager"));
    assert!(manager.launch_script.contains("codex-plus-plus-manager"));
}

#[test]
fn installer_exports_expected_two_entrypoint_names() {
    assert_eq!(
        shortcut_names(),
        ("极义codex.lnk", "极义codex 管理工具.lnk")
    );
    assert_eq!(
        app_bundle_names(),
        ("极义codex.app", "极义codex 管理工具.app")
    );
}

#[test]
fn companion_binary_path_prefers_same_bundle_sidecar_on_macos() {
    let root =
        std::env::temp_dir().join(format!("jiyi-codex-installer-test-{}", std::process::id()));
    let macos_dir = root.join("极义codex.app").join("Contents").join("MacOS");
    std::fs::create_dir_all(&macos_dir).unwrap();
    let main_exe = macos_dir.join("JiyiCodex");
    let sidecar = macos_dir.join("codex-plus-plus");
    std::fs::write(&main_exe, "").unwrap();
    std::fs::write(&sidecar, "").unwrap();

    let companion = companion_binary_path_from_exe(&main_exe, SILENT_BINARY);

    assert_eq!(companion, sidecar);
    assert_ne!(companion, main_exe);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn macos_bundle_does_not_wrap_the_bundle_executable_in_itself() {
    let options = InstallOptions {
        install_root: Some("/Applications".into()),
        launcher_path: Some("/Applications/极义codex.app/Contents/MacOS/JiyiCodex".into()),
        manager_path: Some(
            "/Applications/极义codex 管理工具.app/Contents/MacOS/JiyiCodexManager".into(),
        ),
        remove_owned_data: false,
    };

    let silent = build_macos_app_bundle(&options, false);
    let manager = build_macos_app_bundle(&options, true);

    assert!(!silent.launch_script.contains("JiyiCodex\""));
    assert!(!manager.launch_script.contains("JiyiCodexManager\""));
    assert!(silent.launch_script.contains("JiyiCodexManager"));
    assert!(manager.launch_script.contains("codex-plus-plus-manager"));
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
