use codex_plus_data::usage_history::{clear_history, query_history};
use serde_json::json;

#[test]
fn missing_home_is_unavailable_without_creating_it() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("missing");
    let result = query_history(&home, &json!({"period":"all"}));
    assert_eq!(result["status"], "unavailable");
    assert_eq!(result["complete"], false);
    assert!(!home.exists());
}

#[test]
fn malformed_parameters_are_rejected_before_accessing_settings() {
    let root = tempfile::tempdir().unwrap();
    for query in [
        json!({"period":"unrecognized"}),
        json!({"sessionId":"not-a-session"}),
        json!({"from":"not-a-date"}),
    ] {
        assert_eq!(query_history(root.path(), &query)["status"], "unavailable");
    }
    clear_history(root.path());
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}
