// 输入框下方用量状态栏：开关默认值 + 部分更新（/settings/set 白名单）回归。
//
// 与 capacity_auto_retry_settings.rs 同样的理由：管理器走 store.save() 整体覆盖写，
// 只要结构体有字段就能存；注入层菜单开关走 `/settings/set` → store.update()，那条
// 路径是 merge_known_setting_fields 白名单式合并，漏登记的键会被静默丢弃。
//
// 这里还多守一件事：这个开关默认是**开**的，老 settings.json 里没有这个键时必须补
// true，而不是跟着 bool 的 Default 变成 false。
use codex_plus_core::settings::{BackendSettings, SettingsStore};
use serde_json::json;

#[test]
fn usage_status_line_defaults_to_on_for_old_settings() {
    let defaults = BackendSettings::default();
    assert!(defaults.codex_app_usage_status_line);
    assert_eq!(
        serde_json::to_value(defaults).unwrap()["codexAppUsageStatusLine"],
        json!(true)
    );

    let settings: BackendSettings = serde_json::from_value(json!({
        "enhancementsEnabled": true,
        "codexAppThreadIdBadge": true,
    }))
    .unwrap();
    assert!(settings.codex_app_usage_status_line);
    assert!(settings.codex_app_thread_id_badge);
}

#[test]
fn usage_status_line_round_trips_through_partial_updates() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    let store = SettingsStore::new(path.clone());

    let settings = BackendSettings::default();
    store.save(&settings).unwrap();
    assert!(store.load().unwrap().codex_app_usage_status_line);

    store
        .update(json!({ "codexAppUsageStatusLine": false }))
        .unwrap();
    assert!(!store.load().unwrap().codex_app_usage_status_line);

    let updated = store
        .update(json!({ "codexAppUsageStatusLine": true }))
        .unwrap();
    assert!(updated.codex_app_usage_status_line);
    assert!(store.load().unwrap().codex_app_usage_status_line);

    let raw: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(raw["codexAppUsageStatusLine"], json!(true));
}
