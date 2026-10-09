use codex_plus_core::settings::{BackendSettings, SettingsStore};
use serde_json::json;

#[test]
fn retired_layout_flag_is_ignored_without_affecting_other_enhancements() {
    let settings: BackendSettings = serde_json::from_value(json!({
        "codexAppCustomLayoutEnabled": true,
        "enhancementsEnabled": true,
        "codexAppConversationView": true,
        "codexAppWhaleWidgetEnabled": true,
        "codexAppTypingEffect": "stars",
        "codexAppThreadIdBadge": true,
        "codexAppSessionShare": false,
    }))
    .unwrap();
    assert!(settings.codex_app_conversation_view);
    assert!(settings.codex_app_whale_widget_enabled);
    assert_eq!(settings.codex_app_typing_effect, "stars");
    assert!(settings.codex_app_thread_id_badge);
    assert!(!settings.codex_app_session_share);
    let serialized = serde_json::to_value(settings).unwrap();
    assert!(serialized.get("codexAppCustomLayoutEnabled").is_none());
    assert_eq!(serialized["codexAppConversationView"], true);
    assert_eq!(serialized["codexAppWhaleWidgetEnabled"], true);
    assert_eq!(serialized["codexAppTypingEffect"], "stars");
}

#[test]
fn saving_old_settings_and_partial_updates_never_reintroduce_the_retired_flag() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    std::fs::write(
        &path,
        serde_json::to_vec(&json!({
            "codexAppCustomLayoutEnabled": true,
            "codexAppConversationView": true,
            "codexAppWhaleWidgetEnabled": true,
            "codexAppTypingEffect": "stars",
            "codexAppThreadIdBadge": true,
        }))
        .unwrap(),
    )
    .unwrap();
    let store = SettingsStore::new(path.clone());
    let loaded = store.load().unwrap();
    store.save(&loaded).unwrap();
    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert!(saved.get("codexAppCustomLayoutEnabled").is_none());
    let updated = store
        .update(json!({
            "codexAppCustomLayoutEnabled": true,
            "codexAppTypingEffect": "rainbow",
            "codexAppThreadIdBadge": false,
        }))
        .unwrap();
    assert!(updated.codex_app_conversation_view);
    assert!(updated.codex_app_whale_widget_enabled);
    assert_eq!(updated.codex_app_typing_effect, "rainbow");
    assert!(!updated.codex_app_thread_id_badge);
    assert_eq!(store.load().unwrap(), updated);
    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert!(saved.get("codexAppCustomLayoutEnabled").is_none());
    assert_eq!(saved["codexAppConversationView"], true);
    assert_eq!(saved["codexAppWhaleWidgetEnabled"], true);
}
