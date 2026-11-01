use std::process::Command;

#[test]
fn renderer_and_manager_ui_behaviors() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new("node")
        .arg("--test")
        .arg("src/*.test.ts")
        .current_dir(root.join("apps/codex-plus-manager"))
        .output()
        .expect("node is required for renderer and manager UI tests");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn session_delete_renderer_behaviors() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new("node")
        .args(["--test", "src/session-delete-runtime.test.ts"])
        .current_dir(root.join("apps/codex-plus-manager"))
        .output()
        .expect("node is required for session deletion UI tests");
    assert!(output.status.success(), "{}\n{}",
        String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
}

#[test]
fn manager_ui_typecheck() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let manager = root.join("apps/codex-plus-manager");
    let output = Command::new("node")
        .arg(manager.join("node_modules/typescript/bin/tsc"))
        .args(["--noEmit", "-p", "tsconfig.json"])
        .current_dir(&manager)
        .output()
        .expect("node is required for manager UI checks");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
