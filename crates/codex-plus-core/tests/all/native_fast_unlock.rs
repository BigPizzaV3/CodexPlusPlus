use std::process::Command;

#[test]
fn native_fast_unlock_preserves_requirements_and_existing_auth_paths() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/inject/native-fast-unlock.test.cjs");
    let output = Command::new("node")
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
