use std::process::Command;

#[test]
fn official_usage_policy_lifecycle_preserves_native_state() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/inject/official-usage-lifecycle.test.cjs");
    let output = Command::new("node")
        .arg("--test")
        .arg(path)
        .output()
        .expect("node is required for renderer tests");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
