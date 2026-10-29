use codex_plus_core::settings::{BackendSettings, SettingsStore};
use serde_json::json;

#[test]
fn standalone_quota_unlock_is_disabled_in_defaults_and_legacy_settings() {
    let defaults = BackendSettings::default();
    assert!(!defaults.codex_app_standalone_quota_unlock);
    assert_eq!(
        serde_json::to_value(defaults).unwrap()["codexAppStandaloneQuotaUnlock"],
        false
    );
    let legacy: BackendSettings =
        serde_json::from_value(json!({ "enhancementsEnabled": false })).unwrap();
    assert!(!legacy.codex_app_standalone_quota_unlock);
}

#[test]
fn standalone_quota_unlock_persists_independently_of_the_master_switch() {
    let temp = tempfile::tempdir().unwrap();
    let store = SettingsStore::new(temp.path().join("settings.json"));
    store.save(&BackendSettings::default()).unwrap();
    let settings = store
        .update(json!({
            "enhancementsEnabled": false,
            "codexAppStandaloneQuotaUnlock": true,
        }))
        .unwrap();
    assert!(!settings.enhancements_enabled);
    assert!(settings.codex_app_standalone_quota_unlock);
    assert!(store.load().unwrap().codex_app_standalone_quota_unlock);
    let settings = store.update(json!({ "enhancementsEnabled": true })).unwrap();
    assert!(settings.codex_app_standalone_quota_unlock);
    let settings = store
        .update(json!({ "codexAppStandaloneQuotaUnlock": false }))
        .unwrap();
    assert!(settings.enhancements_enabled);
    assert!(!settings.codex_app_standalone_quota_unlock);
}
