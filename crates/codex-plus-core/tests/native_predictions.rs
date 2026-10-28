use std::process::Command;

#[test]
fn native_predictions_rewriter_and_sampling_adapter_pass_renderer_tests() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/inject/native-predictions.test.cjs");
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
