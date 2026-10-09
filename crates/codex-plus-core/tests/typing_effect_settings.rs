use codex_plus_core::settings::{BackendSettings, SettingsStore};
use serde_json::json;

#[test]
fn typing_effect_is_opt_in_for_old_settings() {
    let defaults = BackendSettings::default();
    assert_eq!(defaults.codex_app_typing_effect, "off");
    assert_eq!(
        serde_json::to_value(defaults).unwrap()["codexAppTypingEffect"],
        "off"
    );

    let settings: BackendSettings = serde_json::from_value(json!({
        "enhancementsEnabled": true,
        "codexAppThreadIdBadge": true,
    }))
    .unwrap();
    assert_eq!(settings.codex_app_typing_effect, "off");
    assert!(settings.codex_app_thread_id_badge);
}

#[test]
fn typing_effect_modes_round_trip_through_save_and_partial_updates() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    let store = SettingsStore::new(path.clone());

    for mode in ["rainbow", "fireworks", "stars", "off"] {
        let mut settings = BackendSettings::default();
        settings.codex_app_typing_effect = mode.to_string();
        store.save(&settings).unwrap();
        assert_eq!(store.load().unwrap().codex_app_typing_effect, mode);

        store
            .update(json!({ "codexAppTypingEffect": "off" }))
            .unwrap();
        let updated = store
            .update(json!({ "codexAppTypingEffect": mode }))
            .unwrap();
        assert_eq!(updated.codex_app_typing_effect, mode);
        assert_eq!(store.load().unwrap().codex_app_typing_effect, mode);
        let raw: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(raw["codexAppTypingEffect"], mode);
    }
}

#[test]
fn invalid_saved_typing_effect_values_load_as_off() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    let store = SettingsStore::new(path.clone());

    for invalid in [
        json!("unknown"),
        json!(""),
        json!(null),
        json!(true),
        json!(42),
        json!([]),
        json!({}),
    ] {
        let raw = json!({ "codexAppTypingEffect": invalid, "codexAppThreadIdBadge": true });
        std::fs::write(&path, serde_json::to_vec(&raw).unwrap()).unwrap();
        let settings = store.load().unwrap();
        assert_eq!(settings.codex_app_typing_effect, "off");
        assert!(settings.codex_app_thread_id_badge);
    }

    let mut settings = BackendSettings::default();
    settings.codex_app_typing_effect = "unknown".to_string();
    store.save(&settings).unwrap();
    assert_eq!(store.load().unwrap().codex_app_typing_effect, "off");
}

#[test]
fn invalid_typing_effect_patches_keep_the_previous_choice() {
    let temp = tempfile::tempdir().unwrap();
    let store = SettingsStore::new(temp.path().join("settings.json"));
    store
        .update(json!({ "codexAppTypingEffect": "rainbow" }))
        .unwrap();

    for invalid in [
        json!("unknown"),
        json!(" stars "),
        json!(""),
        json!(null),
        json!(true),
        json!(42),
        json!([]),
        json!({}),
    ] {
        let updated = store
            .update(json!({ "codexAppTypingEffect": invalid, "codexAppThreadIdBadge": true }))
            .unwrap();
        assert_eq!(updated.codex_app_typing_effect, "rainbow");
        assert!(updated.codex_app_thread_id_badge);
        assert_eq!(store.load().unwrap().codex_app_typing_effect, "rainbow");
    }
}

#[test]
fn partial_updates_preserve_typing_effect_and_other_settings() {
    let temp = tempfile::tempdir().unwrap();
    let store = SettingsStore::new(temp.path().join("settings.json"));
    let mut settings = BackendSettings::default();
    settings.codex_app_thread_id_badge = true;
    settings.dictation.model = "whisper-1".to_string();
    settings.relay_profiles[0].name = "保留的供应商".to_string();
    store.save(&settings).unwrap();

    let updated = store
        .update(json!({ "codexAppTypingEffect": "fireworks" }))
        .unwrap();
    assert_eq!(updated.codex_app_typing_effect, "fireworks");
    assert!(updated.codex_app_thread_id_badge);
    assert_eq!(updated.dictation.model, "whisper-1");
    assert_eq!(updated.relay_profiles[0].name, "保留的供应商");

    let updated = store
        .update(json!({ "codexAppThreadIdBadge": false }))
        .unwrap();
    assert_eq!(updated.codex_app_typing_effect, "fireworks");
    assert!(!updated.codex_app_thread_id_badge);
    assert_eq!(updated.dictation.model, "whisper-1");
    assert_eq!(store.load().unwrap(), updated);
}
