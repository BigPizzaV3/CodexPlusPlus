// 目录外模型映射开关：默认值、落盘/读取、部分更新白名单。
//
// 与 helper_body_limit_settings.rs 一样，同时覆盖 `store.save()`（管理器整份
// 保存）和 `store.update()`（`/settings/set` 部分更新白名单）：漏登记
// merge_known_setting_fields 的键会 HTTP 200 但值不变，必须由测试守住。
use codex_plus_core::settings::{BackendSettings, SettingsStore};
use serde_json::json;

#[test]
fn map_unlisted_models_defaults_to_false_for_old_settings() {
    // 默认关闭（opt-in）：开启会改写用户手填但没写进 model_list 的模型，不能静默生效。
    let defaults = BackendSettings::default();
    assert!(!defaults.protocol_proxy_map_unlisted_models);

    let raw = serde_json::to_value(&defaults).unwrap();
    assert_eq!(raw["protocolProxyMapUnlistedModels"], json!(false));

    // 旧 settings.json 没有该键时必须补默认 false，升级后行为不变。
    let settings: BackendSettings = serde_json::from_value(json!({
        "enhancementsEnabled": true,
        "codexAppThreadIdBadge": true,
    }))
    .unwrap();
    assert!(!settings.protocol_proxy_map_unlisted_models);
    assert!(settings.codex_app_thread_id_badge);
}

#[test]
fn map_unlisted_models_round_trips_and_partial_updates() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    let store = SettingsStore::new(path.clone());

    let mut settings = BackendSettings::default();
    settings.protocol_proxy_map_unlisted_models = true;
    store.save(&settings).unwrap();
    assert!(store.load().unwrap().protocol_proxy_map_unlisted_models);

    // 只改这一个键：必须命中 merge_known_setting_fields 白名单。
    store
        .update(json!({"protocolProxyMapUnlistedModels": false}))
        .unwrap();
    assert!(!store.load().unwrap().protocol_proxy_map_unlisted_models);
    let raw: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(raw["protocolProxyMapUnlistedModels"], json!(false));
}
