use codex_plus_core::settings::{BackendSettings, SettingsStore};
use serde_json::json;

#[test]
fn history_turn_recovery_is_opt_in_for_new_and_old_settings() {
    let defaults = BackendSettings::default();
    assert!(!defaults.codex_app_history_turn_recovery);
    assert_eq!(
        serde_json::to_value(defaults).unwrap()["codexAppHistoryTurnRecovery"],
        json!(false)
    );
    let old: BackendSettings =
        serde_json::from_value(json!({ "codexAppThreadScrollRestore": true })).unwrap();
    assert!(!old.codex_app_history_turn_recovery);
    assert!(old.codex_app_thread_scroll_restore);
}

#[test]
fn history_turn_recovery_round_trips_and_partial_updates_preserve_other_settings() {
    let temp = tempfile::tempdir().unwrap();
    let store = SettingsStore::new(temp.path().join("settings.json"));
    let mut settings = BackendSettings::default();
    settings.codex_app_history_turn_recovery = true;
    store.save(&settings).unwrap();
    assert!(store.load().unwrap().codex_app_history_turn_recovery);
    for enabled in [false, true, false] {
        let updated = store
            .update(json!({ "codexAppHistoryTurnRecovery": enabled }))
            .unwrap();
        assert_eq!(updated.codex_app_history_turn_recovery, enabled);
        let reloaded = store.load().unwrap();
        assert_eq!(reloaded.codex_app_history_turn_recovery, enabled);
        assert!(reloaded.codex_app_thread_scroll_restore);
        assert!(!reloaded.codex_app_capacity_auto_retry);
    }
    store
        .update(json!({ "codexAppHistoryTurnRecovery": "true" }))
        .unwrap();
    assert!(!store.load().unwrap().codex_app_history_turn_recovery);
}
