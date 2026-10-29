// Codex++ 更新提醒偏好（#1272）：「启动时自动检查更新」默认开，「跳过此版本」默认空；
// 两个键都要能整份保存，也要能经 `store.update()` 部分更新（管理器单独写入跳过的版本）。
use codex_plus_core::settings::{BackendSettings, SettingsStore};
use serde_json::json;

#[test]
fn update_reminder_defaults_keep_old_behavior() {
    let defaults = BackendSettings::default();
    assert!(defaults.update_auto_check);
    assert!(defaults.update_skipped_version.is_empty());

    let raw = serde_json::to_value(&defaults).unwrap();
    assert_eq!(raw["updateAutoCheck"], json!(true));
    assert_eq!(raw["updateSkippedVersion"], json!(""));

    // 旧 settings.json 没有这两个键：升级后仍自动检查、不跳过任何版本。
    let settings: BackendSettings = serde_json::from_value(json!({
        "enhancementsEnabled": true,
    }))
    .unwrap();
    assert!(settings.update_auto_check);
    assert!(settings.update_skipped_version.is_empty());
}

#[test]
fn update_reminder_round_trips_and_partial_updates() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    let store = SettingsStore::new(path.clone());

    let mut settings = BackendSettings::default();
    settings.update_auto_check = false;
    store.save(&settings).unwrap();
    assert!(!store.load().unwrap().update_auto_check);

    // 部分更新必须命中 merge_known_setting_fields 白名单，且不动别的键。
    store
        .update(json!({"updateSkippedVersion": " v1.12.0 "}))
        .unwrap();
    let loaded = store.load().unwrap();
    assert_eq!(loaded.update_skipped_version, "v1.12.0");
    assert!(!loaded.update_auto_check);

    store.update(json!({"updateAutoCheck": true})).unwrap();
    let raw: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(raw["updateAutoCheck"], json!(true));
    assert_eq!(raw["updateSkippedVersion"], json!("v1.12.0"));
}
