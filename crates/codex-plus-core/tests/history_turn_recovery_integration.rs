use codex_plus_core::settings::{BackendSettings, SettingsStore};
use serde_json::json;

#[test]
fn recovery_has_no_independent_setting_and_legacy_key_is_ignored() {
    let defaults = serde_json::to_value(BackendSettings::default()).unwrap();
    assert!(defaults.get("codexAppHistoryTurnRecovery").is_none());
    let old: BackendSettings = serde_json::from_value(json!({
        "codexAppHistoryTurnRecovery": true,
        "codexAppThreadScrollRestore": true,
        "providerSyncEnabled": false,
    }))
    .unwrap();
    assert!(old.codex_app_thread_scroll_restore);
    assert!(!old.provider_sync_enabled);
    assert!(
        serde_json::to_value(old)
            .unwrap()
            .get("codexAppHistoryTurnRecovery")
            .is_none()
    );
}

#[test]
fn old_repair_switch_is_not_changed_by_removed_enhancement_key() {
    let temp = tempfile::tempdir().unwrap();
    let store = SettingsStore::new(temp.path().join("settings.json"));
    for enabled in [false, true, false] {
        store
            .update(json!({ "providerSyncEnabled": enabled }))
            .unwrap();
        let updated = store
            .update(json!({ "codexAppHistoryTurnRecovery": true }))
            .unwrap();
        assert_eq!(updated.provider_sync_enabled, enabled);
        assert_eq!(store.load().unwrap().provider_sync_enabled, enabled);
        assert!(updated.codex_app_thread_scroll_restore);
        assert!(!updated.codex_app_capacity_auto_retry);
    }
}
