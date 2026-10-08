use std::process::Command;

#[test]
fn custom_layout_runtime_and_settings_contracts() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new("node")
        .arg("--test")
        .arg(root.join("apps/codex-plus-manager/src/custom-layout-runtime.test.ts"))
        .arg(root.join("apps/codex-plus-manager/src/custom-layout-physics.test.ts"))
        .arg(root.join("apps/codex-plus-manager/src/custom-layout-settings.test.ts"))
        .output()
        .expect("node is required for custom layout renderer tests");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn custom_layout_renderer_integration_preserves_existing_enhancements() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new("node")
        .arg("--test")
        .arg(root.join("apps/codex-plus-manager/src/renderer-inject.test.ts"))
        .arg(root.join("apps/codex-plus-manager/src/enhancement-settings.test.ts"))
        .output()
        .expect("node is required for renderer integration tests");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
