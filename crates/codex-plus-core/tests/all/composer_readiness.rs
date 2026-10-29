use std::process::Command;

#[test]
fn mixed_key_composer_recovers_stalled_native_home_reads() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/inject/composer-readiness.test.cjs");
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

#[test]
fn composer_recovery_is_loaded_before_the_renderer_and_runs_on_policy_sync() {
    let script = codex_plus_core::assets::injection_script(57321);
    let module = script
        .find("function createRecovery(")
        .expect("recovery module is bundled");
    let tick = script
        .find("window.__codexPlusComposerReadiness?.tick(client, officialUsagePolicy().unlockSend)")
        .expect("policy heartbeat drives recovery");
    assert!(module < tick);
}
