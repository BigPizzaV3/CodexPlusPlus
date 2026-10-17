// 429 错误自动继续：开关落盘 + 部分更新（/settings/set 白名单）回归。
//
// 这里特意用 `store.update()` 而不是 `store.save()`：管理器走 save() 整体覆盖写，
// 结构体有字段就能存；注入层菜单开关走 `/settings/set` → update()，那条路径是
// merge_known_setting_fields 白名单式合并，漏登记的键会被静默丢弃（HTTP 200 但值不变）。
// 所以这个用例同时守住"字段存在"和"白名单登记"两件事。
use codex_plus_core::settings::{BackendSettings, SettingsStore};
use serde_json::json;

#[test]
fn rate_limit_auto_resume_defaults_to_off_for_old_settings() {
    let defaults = BackendSettings::default();
    assert!(!defaults.codex_app_rate_limit_auto_resume);
    assert_eq!(
        serde_json::to_value(defaults).unwrap()["codexAppRateLimitAutoResume"],
        json!(false)
    );

    // 旧 settings.json 没有这个键时必须补默认值，而不是报错。
    let settings: BackendSettings = serde_json::from_value(json!({
        "enhancementsEnabled": true,
        "codexAppThreadIdBadge": true,
    }))
    .unwrap();
    assert!(!settings.codex_app_rate_limit_auto_resume);
    assert!(settings.codex_app_thread_id_badge);
}

#[test]
fn rate_limit_auto_resume_round_trips_through_partial_updates() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    let store = SettingsStore::new(path.clone());

    let mut settings = BackendSettings::default();
    settings.codex_app_rate_limit_auto_resume = true;
    store.save(&settings).unwrap();
    assert!(store.load().unwrap().codex_app_rate_limit_auto_resume);

    // 关掉再打开：两次都要真正写进去，证明 merge 白名单里有这个键。
    store
        .update(json!({ "codexAppRateLimitAutoResume": false }))
        .unwrap();
    assert!(!store.load().unwrap().codex_app_rate_limit_auto_resume);

    let updated = store
        .update(json!({ "codexAppRateLimitAutoResume": true }))
        .unwrap();
    assert!(updated.codex_app_rate_limit_auto_resume);
    assert!(store.load().unwrap().codex_app_rate_limit_auto_resume);

    let raw: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(raw["codexAppRateLimitAutoResume"], json!(true));

    // 只改这一个键不该动到别的增强开关。
    assert!(raw["codexAppThreadScrollRestore"].as_bool().unwrap());
}
