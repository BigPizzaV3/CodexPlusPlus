#[cfg(windows)]
#[test]
fn manager_binary_uses_windows_gui_subsystem_in_debug_and_release() {
    let main_rs = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/main.rs"))
        .expect("read manager main.rs");

    assert!(
        main_rs.contains("#![cfg_attr(windows, windows_subsystem = \"windows\")]"),
        "manager binary should not allocate a console window on Windows"
    );
}

#[test]
fn manager_release_binary_uses_embedded_frontend_assets() {
    let cargo_toml = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
        .expect("read manager Cargo.toml");

    assert!(
        cargo_toml.contains("custom-protocol"),
        "release manager binary should use Tauri custom protocol instead of devUrl localhost"
    );
}

#[test]
fn manager_uses_single_instance_guard_before_starting_tauri() {
    let lib_rs = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs"))
        .expect("read manager lib.rs");

    assert!(lib_rs.contains("acquire_single_instance_guard(app_mode)"));
    assert!(lib_rs.contains("MANAGER_GUARD_PORT"));
    assert!(lib_rs.contains("MANAGER_GUARD_PORT.saturating_sub(1)"));
    assert!(lib_rs.contains("manager.already_running"));
}

#[test]
fn launcher_binary_embeds_codex_icon_resource() {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let launcher_build = manifest_dir
        .parent()
        .and_then(std::path::Path::parent)
        .unwrap()
        .join("codex-plus-launcher/build.rs");
    let build_rs = std::fs::read_to_string(&launcher_build).expect("read launcher build.rs");

    assert!(build_rs.contains("WindowsResource"));
    assert!(build_rs.contains("icons/icon.ico"));
}

#[test]
fn windows_binaries_request_administrator_privileges() {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let manager_build =
        std::fs::read_to_string(manifest_dir.join("build.rs")).expect("read manager build.rs");
    let windows_manifest = std::fs::read_to_string(manifest_dir.join("windows-app-manifest.xml"))
        .expect("read windows app manifest");
    let launcher_build = manifest_dir
        .parent()
        .and_then(std::path::Path::parent)
        .unwrap()
        .join("codex-plus-launcher/build.rs");
    let launcher_build = std::fs::read_to_string(&launcher_build).expect("read launcher build.rs");
    let windows_installer = manifest_dir
        .parent()
        .and_then(std::path::Path::parent)
        .and_then(std::path::Path::parent)
        .unwrap()
        .join("scripts/installer/windows/CodexPlusPlus.nsi");
    let windows_installer =
        std::fs::read_to_string(&windows_installer).expect("read windows installer");

    assert!(manager_build.contains("windows-app-manifest.xml"));
    assert!(launcher_build.contains("windows-app-manifest.xml"));
    assert!(windows_manifest.contains("requireAdministrator"));
    assert!(windows_manifest.contains("Microsoft.Windows.Common-Controls"));
    assert!(windows_installer.contains("RequestExecutionLevel admin"));
}

#[test]
fn manager_launch_button_spawns_silent_launcher_binary() {
    let commands_rs =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/commands.rs"))
            .expect("read manager commands.rs");

    assert!(commands_rs.contains("SILENT_BINARY"));
    assert!(commands_rs.contains("std::process::Command::new"));
    assert!(!commands_rs.contains("launch_and_inject_with_hooks(options"));
}

#[test]
fn main_entry_does_not_auto_launch_codex_after_local_auth() {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let app_tsx = manifest_dir.parent().unwrap().join("src/App.tsx");
    let app_tsx = std::fs::read_to_string(&app_tsx).expect("read manager App.tsx");

    assert!(app_tsx.contains("手机号已验证，请点击进入 Codex。"));
    assert!(!app_tsx.contains("mainAutoLaunchRef"));
    assert!(!app_tsx.contains("await enterCodex();"));
    assert!(!app_tsx.contains("!localAuth?.authenticated || mainAutoLaunchRef.current"));
}

#[test]
fn macos_packager_hides_silent_launcher_but_not_manager() {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let packager = manifest_dir
        .parent()
        .and_then(std::path::Path::parent)
        .and_then(std::path::Path::parent)
        .unwrap()
        .join("scripts/installer/macos/package-dmg.sh");
    let script = std::fs::read_to_string(&packager).expect("read macOS packager");

    assert!(script.contains("<key>LSUIElement</key>"));
    assert!(script.contains("ARCH=\"${2:-$(uname -m)}\""));
    assert!(script.contains("BINARY_DIR=\"${BINARY_DIR:-$ROOT/target/release}\""));
    assert!(script.contains("CODEX_APP_SOURCE=\"${CODEX_APP_SOURCE:-/Applications/Codex.app}\""));
    assert!(
        script.contains("CODESIGN_IDENTITY=\"${JIYI_CODESIGN_IDENTITY:-${CODESIGN_IDENTITY:--}}\"")
    );
    assert!(script.contains("NOTARIZE=\"${JIYI_NOTARIZE:-0}\""));
    assert!(
        script.contains(
            "codesign --force --timestamp --options runtime --sign \"$CODESIGN_IDENTITY\""
        )
    );
    assert!(script.contains("xcrun notarytool submit \"$DMG\" --wait"));
    assert!(script.contains("xcrun stapler staple \"$DMG\""));
    assert!(script.contains("JiyiCodex-${VERSION}-macos-${ARCH}.dmg"));
    assert!(script.contains("install_silent_launcher \"$STAGE/极义codex.app\""));
    assert!(script.contains("install_silent_launcher \"$STAGE/极义codex 管理工具.app\""));
    assert!(script.contains("embed_codex_client \"$STAGE/极义codex.app\""));
    assert!(script.contains("verify_embedded_codex_client \"$STAGE/极义codex.app\""));
    assert!(script.contains("install_server_scripts \"$STAGE/极义codex.app\""));
    assert!(script.contains("install_server_scripts \"$STAGE/极义codex 管理工具.app\""));
    assert!(script.contains("jiyi-managed-proxy.env.example"));
    assert!(script.contains("install-managed-proxy-launchd.sh"));
    assert!(script.contains("install-managed-proxy-systemd.sh"));
    assert!(script.contains("apps/jiyi-managed-proxy/Dockerfile"));
    assert!(script.contains(
        "create_app \"极义codex\" \"JiyiCodex\" \"$BINARY_DIR/codex-plus-plus-manager\" \"com.jiyi.codex\" \"false\""
    ));
    assert!(script.contains(
        "create_app \"极义codex 管理工具\" \"JiyiCodexManager\" \"$BINARY_DIR/codex-plus-plus-manager\" \"com.jiyi.codex.manager\" \"false\""
    ));
}

#[test]
fn managed_proxy_launchd_scripts_keep_service_isolated() {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest_dir
        .parent()
        .and_then(std::path::Path::parent)
        .and_then(std::path::Path::parent)
        .unwrap();
    let install_script = root.join("scripts/server/macos/install-managed-proxy-launchd.sh");
    let uninstall_script = root.join("scripts/server/macos/uninstall-managed-proxy-launchd.sh");
    let env_example = root.join("scripts/server/macos/jiyi-managed-proxy.env.example");
    let install_script = std::fs::read_to_string(install_script).expect("read launchd install");
    let uninstall_script =
        std::fs::read_to_string(uninstall_script).expect("read launchd uninstall");
    let env_example = std::fs::read_to_string(env_example).expect("read managed proxy env");

    assert!(install_script.contains("com.jiyi.codex.managed-proxy"));
    assert!(install_script.contains("/Applications/极义codex.app"));
    assert!(install_script.contains("launchctl bootstrap"));
    assert!(install_script.contains("jiyi-managed-proxy.env"));
    assert!(install_script.contains("STATE_DIR/bin"));
    assert!(install_script.contains("RUNTIME_BINARY"));
    assert!(install_script.contains("jiyi-managed-proxy.out.log"));
    assert!(uninstall_script.contains("launchctl bootout"));
    assert!(uninstall_script.contains("RUNTIME_BINARY"));
    assert!(uninstall_script.contains("--purge-env"));
    assert!(env_example.contains("JIYI_MANAGED_PROXY_LISTEN=\"127.0.0.1:57421\""));
    assert!(env_example.contains("JIYI_MANAGED_PROXY_UPSTREAM_API_KEY=\"\""));
    assert!(env_example.contains("JIYI_MANAGED_PROXY_SYNC_API_KEY=\"\""));
    assert!(env_example.contains("JIYI_MANAGED_PROXY_ADMIN_API_KEY=\"\""));
    assert!(env_example.contains("JIYI_MANAGED_PROXY_USER_READ_API_KEY=\"\""));
    assert!(env_example.contains("JIYI_MANAGED_PROXY_BILLING_API_KEY=\"\""));
    assert!(env_example.contains("JIYI_MANAGED_PROXY_PAYMENT_WEBHOOK_API_KEY=\"\""));
    assert!(env_example.contains("JIYI_MANAGED_PROXY_PAYMENT_WEBHOOK_SIGNATURE_SECRET=\"\""));
    assert!(env_example.contains("JIYI_MANAGED_PROXY_ALIPAY_PUBLIC_KEY=\"\""));
    assert!(env_example.contains("JIYI_MANAGED_PROXY_ALIPAY_PUBLIC_KEY_PATH=\"\""));
    assert!(env_example.contains("JIYI_MANAGED_PROXY_WECHATPAY_PUBLIC_KEY=\"\""));
    assert!(env_example.contains("JIYI_MANAGED_PROXY_WECHATPAY_PUBLIC_KEY_PATH=\"\""));
    assert!(env_example.contains("JIYI_MANAGED_PROXY_ACCESS_API_KEY=\"\""));
    assert!(env_example.contains("JIYI_MANAGED_PROXY_AUDIT_API_KEY=\"\""));
    assert!(env_example.contains("JIYI_MANAGED_PROXY_DB_PATH="));
    assert!(!env_example.contains("sk-"));
}

#[test]
fn managed_proxy_remote_deploy_templates_keep_server_keys_out_of_client() {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest_dir
        .parent()
        .and_then(std::path::Path::parent)
        .and_then(std::path::Path::parent)
        .unwrap();
    let install_script =
        std::fs::read_to_string(root.join("scripts/server/linux/install-managed-proxy-systemd.sh"))
            .expect("read systemd install");
    let uninstall_script = std::fs::read_to_string(
        root.join("scripts/server/linux/uninstall-managed-proxy-systemd.sh"),
    )
    .expect("read systemd uninstall");
    let service =
        std::fs::read_to_string(root.join("scripts/server/linux/jiyi-managed-proxy.service"))
            .expect("read systemd service");
    let env_example =
        std::fs::read_to_string(root.join("scripts/server/linux/jiyi-managed-proxy.env.example"))
            .expect("read linux env");
    let dockerfile = std::fs::read_to_string(root.join("apps/jiyi-managed-proxy/Dockerfile"))
        .expect("read managed proxy Dockerfile");

    assert!(install_script.contains("systemctl enable"));
    assert!(install_script.contains("/etc/jiyi-codex"));
    assert!(install_script.contains("/var/lib/jiyi-codex"));
    assert!(uninstall_script.contains("systemctl disable --now"));
    assert!(service.contains("ExecStart=/usr/local/bin/jiyi-managed-proxy"));
    assert!(service.contains("EnvironmentFile=/etc/jiyi-codex/jiyi-managed-proxy.env"));
    assert!(service.contains("NoNewPrivileges=true"));
    assert!(env_example.contains("JIYI_MANAGED_PROXY_LISTEN=\"0.0.0.0:8080\""));
    assert!(env_example.contains("JIYI_MANAGED_PROXY_UPSTREAM_API_KEY=\"\""));
    assert!(env_example.contains("JIYI_MANAGED_PROXY_SYNC_API_KEY=\"\""));
    assert!(env_example.contains("JIYI_MANAGED_PROXY_ADMIN_API_KEY=\"\""));
    assert!(env_example.contains("JIYI_MANAGED_PROXY_USER_READ_API_KEY=\"\""));
    assert!(env_example.contains("JIYI_MANAGED_PROXY_BILLING_API_KEY=\"\""));
    assert!(env_example.contains("JIYI_MANAGED_PROXY_PAYMENT_WEBHOOK_API_KEY=\"\""));
    assert!(env_example.contains("JIYI_MANAGED_PROXY_PAYMENT_WEBHOOK_SIGNATURE_SECRET=\"\""));
    assert!(env_example.contains("JIYI_MANAGED_PROXY_ALIPAY_PUBLIC_KEY=\"\""));
    assert!(env_example.contains("JIYI_MANAGED_PROXY_ALIPAY_PUBLIC_KEY_PATH=\"\""));
    assert!(env_example.contains("JIYI_MANAGED_PROXY_WECHATPAY_PUBLIC_KEY=\"\""));
    assert!(env_example.contains("JIYI_MANAGED_PROXY_WECHATPAY_PUBLIC_KEY_PATH=\"\""));
    assert!(env_example.contains("JIYI_MANAGED_PROXY_ACCESS_API_KEY=\"\""));
    assert!(env_example.contains("JIYI_MANAGED_PROXY_AUDIT_API_KEY=\"\""));
    assert!(env_example.contains("JIYI_MANAGED_PROXY_DB_PATH=\"/var/lib/jiyi-codex"));
    assert!(dockerfile.contains("cargo build --release -p jiyi-managed-proxy"));
    assert!(dockerfile.contains("JIYI_MANAGED_PROXY_ADMIN_API_KEY"));
    assert!(dockerfile.contains("JIYI_MANAGED_PROXY_USER_READ_API_KEY"));
    assert!(dockerfile.contains("JIYI_MANAGED_PROXY_BILLING_API_KEY"));
    assert!(dockerfile.contains("JIYI_MANAGED_PROXY_PAYMENT_WEBHOOK_API_KEY"));
    assert!(dockerfile.contains("JIYI_MANAGED_PROXY_PAYMENT_WEBHOOK_SIGNATURE_SECRET"));
    assert!(dockerfile.contains("JIYI_MANAGED_PROXY_ALIPAY_PUBLIC_KEY"));
    assert!(dockerfile.contains("JIYI_MANAGED_PROXY_ALIPAY_PUBLIC_KEY_PATH"));
    assert!(dockerfile.contains("JIYI_MANAGED_PROXY_WECHATPAY_PUBLIC_KEY"));
    assert!(dockerfile.contains("JIYI_MANAGED_PROXY_WECHATPAY_PUBLIC_KEY_PATH"));
    assert!(dockerfile.contains("JIYI_MANAGED_PROXY_ACCESS_API_KEY"));
    assert!(dockerfile.contains("JIYI_MANAGED_PROXY_AUDIT_API_KEY"));
    assert!(dockerfile.contains("USER jiyi-codex"));
    assert!(dockerfile.contains("EXPOSE 8080"));
    assert!(!env_example.contains("sk-"));
    assert!(!dockerfile.contains("sk-"));
}

#[test]
fn github_release_workflow_builds_separate_macos_x64_and_arm64_dmgs() {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let workflow = manifest_dir
        .parent()
        .and_then(std::path::Path::parent)
        .and_then(std::path::Path::parent)
        .unwrap()
        .join(".github/workflows/release-assets.yml");
    let workflow = std::fs::read_to_string(&workflow).expect("read release assets workflow");

    assert!(workflow.contains("macos-15-intel"));
    assert!(workflow.contains("x86_64-apple-darwin"));
    assert!(workflow.contains("macos-14"));
    assert!(workflow.contains("aarch64-apple-darwin"));
    assert!(workflow.contains("package-dmg.sh \"$VERSION\" \"${{ matrix.arch }}\""));
    assert!(workflow.contains("target/${{ matrix.target }}/release"));
}

#[test]
fn github_release_workflow_uploads_static_latest_json() {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let workflow = manifest_dir
        .parent()
        .and_then(std::path::Path::parent)
        .and_then(std::path::Path::parent)
        .unwrap()
        .join(".github/workflows/release-assets.yml");
    let workflow = std::fs::read_to_string(&workflow).expect("read release assets workflow");

    assert!(workflow.contains("latest-json:"));
    assert!(workflow.contains("latest.json"));
    assert!(workflow.contains("gh release upload \"$TAG\" latest.json --clobber"));
}

#[test]
fn relay_settings_keeps_profile_config_and_auth_files_isolated() {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let app_tsx = manifest_dir.parent().unwrap().join("src/App.tsx");
    let app_tsx = std::fs::read_to_string(&app_tsx).expect("read manager App.tsx");
    let commands_rs = manifest_dir.join("src/commands.rs");
    let commands_rs = std::fs::read_to_string(&commands_rs).expect("read manager commands.rs");

    assert!(app_tsx.contains("snapshotActiveRelayFilesBeforeSwitch"));
    assert!(app_tsx.contains("backfill_relay_profile_from_live"));
    assert!(app_tsx.contains("relayProfileSwitchValidation(selectedBeforeSave)"));
    assert!(app_tsx.contains("缺少独立 config.toml"));
    assert!(app_tsx.contains("const command = relayProfileSwitchCommand(selectedAfterSave)"));
    assert!(!commands_rs.contains("缺少独立 auth.json"));
    assert!(commands_rs.contains("backfill_relay_profile_from_live"));
    assert!(commands_rs.contains("apply_relay_profile_to_home_with_switch_rules"));
}

#[test]
fn relay_context_management_is_global_not_supplier_scoped() {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let app_tsx = manifest_dir.parent().unwrap().join("src/App.tsx");
    let app_tsx = std::fs::read_to_string(&app_tsx).expect("read manager App.tsx");
    let styles = manifest_dir.parent().unwrap().join("src/styles.css");
    let styles = std::fs::read_to_string(&styles).expect("read manager styles.css");

    assert!(app_tsx.contains("作为全局配置独立管理"));
    assert!(app_tsx.contains("label: \"工具与插件\""));
    assert!(app_tsx.contains("title=\"Codex 工具与插件\""));
    assert!(!app_tsx.contains("label: \"上下文配置\""));
    assert!(!app_tsx.contains("title=\"上下文配置\""));
    assert!(!app_tsx.contains("<strong>Codex 上下文</strong>"));
    assert!(app_tsx.contains("id: \"context\""));
    assert!(app_tsx.contains("function ContextScreen"));
    assert!(app_tsx.contains("route === \"context\""));
    assert!(app_tsx.contains("if (next === \"context\")"));
    assert!(app_tsx.contains("selectedContextConfigToml(entries)"));
    assert!(app_tsx.contains("toggleContextEntryEnabled"));
    assert!(app_tsx.contains("relayFiles={relayFiles}"));
    assert!(app_tsx.contains("read_live_context_entries"));
    assert!(app_tsx.contains("sync_live_context_entries"));
    assert!(app_tsx.contains("refreshLiveContextEntries"));
    assert!(app_tsx.contains("syncLiveContextEntries(next, true)"));
    assert!(app_tsx.contains("function contextEntriesWithLiveEntries"));
    assert!(app_tsx.contains("liveByKind"));
    assert!(app_tsx.contains("mergeLiveContextEntries"));
    assert!(app_tsx.contains("withLiveEntryState"));
    assert!(app_tsx.contains("contextEnabledSwitch"));
    assert!(!app_tsx.contains("entry.enabled ? \"已启用\" : \"已禁用\""));
    assert!(!app_tsx.contains("空配置体"));
    assert!(app_tsx.contains("relay-context-delete"));
    assert!(!app_tsx.contains("切换供应商时只合并勾选项"));
    assert!(!app_tsx.contains("未勾选的条目不会写入"));
    assert!(!app_tsx.contains("className=\"context-switch\""));
    assert!(!styles.contains(".context-switch {"));
    assert!(styles.contains(".context-enabled-switch"));
    assert!(styles.contains(".context-switch-track"));
    assert!(styles.contains(".context-switch-thumb"));
    assert!(!styles.contains(".relay-context-row code"));
    assert!(styles.contains(".relay-context-delete"));
}

#[test]
fn manager_window_and_relay_detail_header_stay_usable() {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let app_tsx = manifest_dir.parent().unwrap().join("src/App.tsx");
    let app_tsx = std::fs::read_to_string(&app_tsx).expect("read manager App.tsx");
    let styles = manifest_dir.parent().unwrap().join("src/styles.css");
    let styles = std::fs::read_to_string(&styles).expect("read manager styles.css");
    let lib_rs =
        std::fs::read_to_string(manifest_dir.join("src/lib.rs")).expect("read manager lib.rs");
    let tauri_conf =
        std::fs::read_to_string(manifest_dir.join("tauri.conf.json")).expect("read tauri config");

    assert!(app_tsx.contains("relay-detail-sticky"));
    assert!(!app_tsx.contains("CardHead title=\"供应商详情\""));
    assert!(styles.contains(".relay-detail-sticky"));
    assert!(styles.contains("position: sticky"));
    assert!(styles.contains("top: 0"));
    assert!(styles.contains("margin: 0"));
    assert!(lib_rs.contains(".inner_size(app_mode.initial_width(), app_mode.initial_height())"));
    assert!(lib_rs.contains(".min_inner_size(app_mode.min_width(), app_mode.min_height())"));
    assert!(lib_rs.contains("AppMode::Manager => 1180.0"));
    assert!(lib_rs.contains("AppMode::Manager => 820.0"));
    assert!(lib_rs.contains("AppMode::Manager => 960.0"));
    assert!(lib_rs.contains("AppMode::Manager => 720.0"));
    assert!(tauri_conf.contains("\"width\": 1180"));
    assert!(tauri_conf.contains("\"height\": 820"));
    assert!(tauri_conf.contains("\"minWidth\": 960"));
    assert!(tauri_conf.contains("\"minHeight\": 720"));
}

#[test]
fn relay_preview_deduplicates_root_keys_when_merging_common_config() {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let app_tsx = manifest_dir.parent().unwrap().join("src/App.tsx");
    let app_tsx = std::fs::read_to_string(&app_tsx).expect("read manager App.tsx");

    assert!(app_tsx.contains("dedupeTomlRootLines"));
    assert!(app_tsx.contains("rootSeen.add(key)"));
    assert!(app_tsx.contains("joinTomlSectionsRootFirst"));
}
