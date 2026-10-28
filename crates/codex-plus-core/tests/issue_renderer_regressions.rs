use std::process::Command;

fn run_fixture(mode: &str) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/test-issue-renderer-regressions.cjs");
    let output = Command::new("node").arg(path).arg(mode).output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    println!("{}", String::from_utf8_lossy(&output.stdout));
}

#[test]
fn diagnostic_bridge_success_stops_fallback_and_failures_try_once() {
    run_fixture("--unit");
}

#[test]
#[ignore = "需要已有 Playwright 与 Chromium；设置 NODE_PATH 和 CODEX_TEST_BROWSER_PATH 后运行"]
fn theme_cascade_real_browser_fixture() {
    run_fixture("--browser");
}
