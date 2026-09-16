use codex_plus_core::settings::{BackendSettings, SettingsStore};
use serde_json::json;

#[test]
fn native_browser_compatibility_is_opt_in_for_old_settings() {
    let settings: BackendSettings = serde_json::from_value(json!({})).unwrap();
    assert!(!settings.codex_app_native_browser_require_identification);
    assert!(!BackendSettings::default().codex_app_native_browser_require_identification);
}

#[test]
fn save_and_partial_update_preserve_explicit_choice() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    let store = SettingsStore::new(path.clone());
    let mut settings = BackendSettings::default();
    settings.codex_app_native_browser_require_identification = true;
    store.save(&settings).unwrap();
    assert!(
        store
            .load()
            .unwrap()
            .codex_app_native_browser_require_identification
    );
    assert!(
        store
            .update(json!({"codexAppThreadIdBadge": true}))
            .unwrap()
            .codex_app_native_browser_require_identification
    );
    assert!(
        !store
            .update(json!({"codexAppNativeBrowserRequireIdentification": false}))
            .unwrap()
            .codex_app_native_browser_require_identification
    );
    // Saving this option writes settings only, not browser runtime or recovery files.
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
}
