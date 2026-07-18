use codex_plus_core::dream_skin_runtime::{
    DreamSkinRuntimeStatus, DreamSkinState, apply_dream_skin_live, macos_arch_name,
    parse_renderer_verification, windows_app_path_matches_registered_root,
};

#[test]
fn maps_rust_apple_silicon_arch_to_lipo_name() {
    assert_eq!(macos_arch_name("aarch64"), "arm64");
    assert_eq!(macos_arch_name("x86_64"), "x86_64");
}

#[test]
fn windows_identity_uses_native_package_api_without_powershell() {
    let source = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/dream_skin_runtime.rs"
    ))
    .unwrap();

    assert!(source.contains("registered_windows_packages"));
    assert!(!source.contains("Command::new(\"powershell\")"));
}

#[test]
fn live_apply_prefers_lightweight_update_and_keeps_full_injection_fallback() {
    let source = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/dream_skin_runtime.rs"
    ))
    .unwrap();
    let live_apply = source
        .split("pub async fn apply_dream_skin_live")
        .nth(1)
        .unwrap()
        .split("pub async fn pause_dream_skin_live")
        .next()
        .unwrap();

    assert!(live_apply.contains("dream_skin_live_update_probe_script"));
    assert!(live_apply.contains("dream_skin_live_update_script"));
    assert!(live_apply.contains("injection_script_with_settings"));
    assert!(!live_apply.contains("reload_dream_skin_live"));
    assert!(source.contains("reload_dream_skin_live"));
    assert!(source.contains("window.location.reload()"));
    assert!(!source.contains("Duration::from_millis(220)"));
}

#[test]
fn status_distinguishes_not_running_from_failed_verification() {
    let status = DreamSkinRuntimeStatus::not_running(true, false);

    assert_eq!(status.state, DreamSkinState::NotRunning);
    assert!(status.enabled);
    assert!(!status.paused);
    assert!(!status.live_applied);
    assert_eq!(status.checks[0].level.as_str(), "warning");
}

#[test]
fn changed_theme_status_requires_a_clean_restart() {
    let status = DreamSkinRuntimeStatus::pending_restart(true, false);

    assert_eq!(status.state, DreamSkinState::Warning);
    assert!(status.enabled);
    assert!(!status.paused);
    assert!(!status.live_applied);
    assert!(status.checks[0].message.contains("重启 Codex"));
}

#[test]
fn verification_requires_visible_composer() {
    let result = parse_renderer_verification(serde_json::json!({
        "installed": true,
        "version": "codex-plus:windows:custom",
        "stylePresent": true,
        "chromePresent": true,
        "chromePointerEvents": "none",
        "homeRoute": false,
        "homePresent": false,
        "visibleCardCount": 0,
        "projectButton": null,
        "composer": { "visible": false },
        "sidebar": { "visible": true },
        "documentOverflow": { "x": false, "y": false }
    }))
    .unwrap();

    assert_eq!(result.state, DreamSkinState::Fail);
    assert!(!result.pass);
    assert!(
        result
            .checks
            .iter()
            .any(|check| check.id == "composer" && check.level.as_str() == "fail")
    );
}

#[test]
fn verification_accepts_target_project_live_contract() {
    let result = parse_renderer_verification(serde_json::json!({
        "installed": true,
        "version": "codex-plus:windows:custom",
        "stylePresent": true,
        "chromePresent": true,
        "chromePointerEvents": "none",
        "homeRoute": true,
        "homePresent": true,
        "hero": { "visible": true, "width": 900, "height": 220 },
        "visibleCardCount": 4,
        "projectButton": { "visible": true },
        "composer": { "visible": true },
        "sidebar": { "visible": true },
        "documentOverflow": { "x": false, "y": false }
    }))
    .unwrap();

    assert_eq!(result.state, DreamSkinState::Pass);
    assert!(result.pass);
}

#[test]
fn windows_identity_requires_a_path_inside_the_registered_package_root() {
    let root = std::path::Path::new(
        r"C:\Program Files\WindowsApps\OpenAI.Codex_26.707.1.0_x64__2p2nqsd0c76g0",
    );

    assert!(windows_app_path_matches_registered_root(root, root));
    assert!(windows_app_path_matches_registered_root(
        &root.join("app"),
        root
    ));
    assert!(!windows_app_path_matches_registered_root(
        std::path::Path::new(
            r"C:\Program Files\WindowsApps\OpenAI.Codex_26.707.1.0_x64__2p2nqsd0c76g0-copy\app",
        ),
        root,
    ));
}

#[tokio::test]
#[ignore = "requires a running Codex Desktop CDP renderer"]
async fn live_apply_keeps_the_running_renderer_available() {
    let debug_port = std::env::var("CODEX_PLUS_TEST_DEBUG_PORT")
        .expect("CODEX_PLUS_TEST_DEBUG_PORT is required")
        .parse()
        .expect("CODEX_PLUS_TEST_DEBUG_PORT must be a port");
    let helper_port = std::env::var("CODEX_PLUS_TEST_HELPER_PORT")
        .unwrap_or_else(|_| "57321".to_string())
        .parse()
        .expect("CODEX_PLUS_TEST_HELPER_PORT must be a port");
    let settings = codex_plus_core::settings::SettingsStore::default()
        .load()
        .expect("Dream Skin settings should load");
    codex_plus_core::dream_skin::sync_default_dream_skin_base_theme(
        true,
        &settings.codex_app_dream_skin_theme_config,
    )
    .expect("Dream Skin base theme should sync");

    let status = apply_dream_skin_live(debug_port, helper_port)
        .await
        .expect("live apply should succeed");
    assert!(status.live_applied, "live apply status: {status:?}");
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    assert!(
        !codex_plus_core::cdp::list_targets(debug_port)
            .await
            .expect("Codex CDP should remain available")
            .is_empty()
    );
}
