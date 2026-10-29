use std::process::Command;

#[test]
fn native_plugin_navigation_has_one_entry_and_preserves_market_adapter() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new("node")
        .arg("--test")
        .arg(root.join("assets/inject/plugin-market-navigation.test.cjs"))
        .arg(root.join("assets/inject/plugin-market-ui.test.cjs"))
        .arg(root.join("assets/inject/plugin-market-adapter.test.cjs"))
        .output()
        .expect("node is required for plugin navigation tests");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
