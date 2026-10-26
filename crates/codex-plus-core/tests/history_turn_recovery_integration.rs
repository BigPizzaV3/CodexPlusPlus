use codex_plus_core::settings::{BackendSettings, SettingsStore};
use serde_json::json;

#[test]
fn recovery_is_opt_in_for_defaults_and_old_settings() {
    assert!(!BackendSettings::default().codex_app_history_turn_recovery);
    let old: BackendSettings = serde_json::from_value(json!({
        "codexAppThreadScrollRestore": true,
        "providerSyncEnabled": false,
    }))
    .unwrap();
    assert!(!old.codex_app_history_turn_recovery);
    assert!(old.codex_app_thread_scroll_restore);
    assert!(!old.provider_sync_enabled);
    assert_eq!(
        serde_json::to_value(old).unwrap()["codexAppHistoryTurnRecovery"],
        false
    );
}

#[test]
fn recovery_choice_round_trips_independently_of_the_index_repair_switch() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    let store = SettingsStore::new(path.clone());
    for recovery in [true, false, true] {
        let mut settings = store.load().unwrap();
        settings.codex_app_history_turn_recovery = recovery;
        store.save(&settings).unwrap();
        assert_eq!(
            store.load().unwrap().codex_app_history_turn_recovery,
            recovery
        );
        for repair in [false, true, false] {
            let updated = store
                .update(json!({ "providerSyncEnabled": repair }))
                .unwrap();
            assert_eq!(updated.codex_app_history_turn_recovery, recovery);
            let updated = store
                .update(json!({ "codexAppHistoryTurnRecovery": !recovery }))
                .unwrap();
            assert_eq!(updated.provider_sync_enabled, repair);
            assert_eq!(updated.codex_app_history_turn_recovery, !recovery);
            let updated = store
                .update(json!({ "codexAppHistoryTurnRecovery": recovery }))
                .unwrap();
            assert_eq!(updated.provider_sync_enabled, repair);
            assert_eq!(
                store.load().unwrap().codex_app_history_turn_recovery,
                recovery
            );
            let raw: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            assert_eq!(raw["codexAppHistoryTurnRecovery"], recovery);
            assert!(updated.codex_app_thread_scroll_restore);
            assert!(!updated.codex_app_capacity_auto_retry);
        }
    }
}

#[test]
fn invalid_recovery_patches_preserve_the_saved_choice() {
    let temp = tempfile::tempdir().unwrap();
    let store = SettingsStore::new(temp.path().join("settings.json"));
    store
        .update(json!({ "codexAppHistoryTurnRecovery": true }))
        .unwrap();
    for invalid in [json!(null), json!("true"), json!(1), json!([]), json!({})] {
        assert!(
            store
                .update(json!({ "codexAppHistoryTurnRecovery": invalid }))
                .unwrap()
                .codex_app_history_turn_recovery
        );
    }
}
