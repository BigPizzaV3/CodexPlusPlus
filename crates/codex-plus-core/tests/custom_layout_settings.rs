use codex_plus_core::settings::{BackendSettings, SettingsStore};
use serde_json::json;

#[test]
fn custom_layout_is_opt_in_for_old_settings() {
    let defaults = BackendSettings::default();
    assert!(!defaults.codex_app_custom_layout_enabled);
    assert_eq!(
        serde_json::to_value(defaults).unwrap()["codexAppCustomLayoutEnabled"],
        false
    );

    let settings: BackendSettings = serde_json::from_value(json!({
        "enhancementsEnabled": true,
        "codexAppThreadIdBadge": true,
        "codexAppTypingEffect": "rainbow",
    }))
    .unwrap();
    assert!(!settings.codex_app_custom_layout_enabled);
    assert!(settings.codex_app_thread_id_badge);
    assert_eq!(settings.codex_app_typing_effect, "rainbow");
}

#[test]
fn custom_layout_round_trips_through_json_and_save() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    let store = SettingsStore::new(path.clone());

    for enabled in [true, false] {
        let mut settings = BackendSettings::default();
        settings.codex_app_custom_layout_enabled = enabled;
        let value = serde_json::to_value(&settings).unwrap();
        assert_eq!(value["codexAppCustomLayoutEnabled"], enabled);
        let parsed: BackendSettings = serde_json::from_value(value).unwrap();
        assert_eq!(parsed.codex_app_custom_layout_enabled, enabled);

        store.save(&settings).unwrap();
        assert_eq!(store.load().unwrap().codex_app_custom_layout_enabled, enabled);
        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved["codexAppCustomLayoutEnabled"], enabled);
    }
}

#[test]
fn custom_layout_patch_preserves_old_settings_and_unknown_fields() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    let store = SettingsStore::new(path.clone());
    let mut settings = BackendSettings::default();
    settings.codex_app_thread_id_badge = true;
    settings.codex_app_typing_effect = "stars".to_string();
    settings.relay_profiles[0].name = "保留的供应商".to_string();
    let mut old = serde_json::to_value(settings).unwrap();
    let old_object = old.as_object_mut().unwrap();
    old_object.remove("codexAppCustomLayoutEnabled");
    old_object.insert("unknownSetting".to_string(), json!({ "keep": true }));
    std::fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();

    assert!(!store.load().unwrap().codex_app_custom_layout_enabled);
    let updated = store
        .update(json!({ "codexAppCustomLayoutEnabled": true }))
        .unwrap();
    assert!(updated.codex_app_custom_layout_enabled);
    assert!(updated.codex_app_thread_id_badge);
    assert_eq!(updated.codex_app_typing_effect, "stars");
    assert_eq!(updated.relay_profiles[0].name, "保留的供应商");

    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(saved["codexAppCustomLayoutEnabled"], true);
    assert_eq!(saved["unknownSetting"], old["unknownSetting"]);
    assert_eq!(store.load().unwrap(), updated);
}

#[test]
fn custom_layout_partial_updates_accept_only_booleans_and_keep_other_settings() {
    let temp = tempfile::tempdir().unwrap();
    let store = SettingsStore::new(temp.path().join("settings.json"));
    store
        .update(json!({ "codexAppCustomLayoutEnabled": true, "codexAppThreadIdBadge": true }))
        .unwrap();

    for invalid in [json!("false"), json!(null), json!(0), json!([]), json!({})] {
        let updated = store
            .update(json!({ "codexAppCustomLayoutEnabled": invalid }))
            .unwrap();
        assert!(updated.codex_app_custom_layout_enabled);
        assert!(updated.codex_app_thread_id_badge);
    }

    let updated = store
        .update(json!({ "codexAppThreadIdBadge": false }))
        .unwrap();
    assert!(updated.codex_app_custom_layout_enabled);
    assert!(!updated.codex_app_thread_id_badge);

    let updated = store
        .update(json!({ "codexAppCustomLayoutEnabled": false }))
        .unwrap();
    assert!(!updated.codex_app_custom_layout_enabled);
    assert!(!updated.codex_app_thread_id_badge);
    assert_eq!(store.load().unwrap(), updated);
}
