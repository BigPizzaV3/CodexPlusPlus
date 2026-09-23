use codex_plus_core::bridge::external_api_quota_breakpoint_condition;
use serde_json::json;
use std::process::Command;

#[test]
fn external_api_quota_condition_rejects_non_identifiers() {
    let value = json!({"quotaVariable": "Rt", "hostVariable": "et"});
    assert_eq!(
        external_api_quota_breakpoint_condition(&value).unwrap(),
        "(Rt&&window.__codexPlusExternalApiQuotaAllowed?.(et)===true&&(Rt=false),false)"
    );
    assert!(
        external_api_quota_breakpoint_condition(
            &json!({"quotaVariable": "Rt;evil()", "hostVariable": "et"})
        )
        .is_none()
    );
    assert!(
        external_api_quota_breakpoint_condition(
            &json!({"quotaVariable": "Rt", "hostVariable": "1bad"})
        )
        .is_none()
    );
}

#[test]
fn external_api_quota_gate_renderer_policy_and_locator() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/inject/api-quota-gate.test.cjs");
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
