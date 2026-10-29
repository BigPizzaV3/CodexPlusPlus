// 本地协议代理请求体上限设置：默认值、落盘/读取、部分更新白名单与范围夹取。
//
// 背景：旧默认 32 MiB 会让 1M 上下文的长会话（请求体 ~33 MB+ 且仍在增长）直接
// 收到 413 并中断对话，因此默认值改为 128 MiB，并提供 32~512 MiB 的设置项。
// 与 proxy_auto_retry_settings.rs 一样，这里同时覆盖 `store.save()`（管理器整份保存）
// 和 `store.update()`（`/settings/set` 部分更新白名单）：漏登记
// merge_known_setting_fields 的键会 HTTP 200 但值不变，必须由测试守住。
use codex_plus_core::settings::{BackendSettings, SettingsStore};
use serde_json::json;

#[test]
fn helper_body_limit_defaults_to_128_mib_for_old_settings() {
    let defaults = BackendSettings::default();
    assert_eq!(defaults.helper_max_request_body_mib, 128);
    assert_eq!(defaults.helper_max_request_body_bytes(), 128 * 1024 * 1024);

    let raw = serde_json::to_value(&defaults).unwrap();
    assert_eq!(raw["helperMaxRequestBodyMib"], json!(128));

    // 旧 settings.json 没有该键时必须补默认值，而不是落成 0 或报错。
    let settings: BackendSettings = serde_json::from_value(json!({
        "enhancementsEnabled": true,
        "codexAppThreadIdBadge": true,
    }))
    .unwrap();
    assert_eq!(settings.helper_max_request_body_mib, 128);
    assert_eq!(settings.helper_max_request_body_bytes(), 128 * 1024 * 1024);
    assert!(settings.codex_app_thread_id_badge);
}

#[test]
fn helper_body_limit_round_trips_and_clamps() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    let store = SettingsStore::new(path.clone());

    let mut settings = BackendSettings::default();
    settings.helper_max_request_body_mib = 256;
    store.save(&settings).unwrap();
    assert_eq!(store.load().unwrap().helper_max_request_body_mib, 256);

    // 只改这一个键：必须命中 merge_known_setting_fields 白名单。
    store
        .update(json!({"helperMaxRequestBodyMib": 300}))
        .unwrap();
    assert_eq!(store.load().unwrap().helper_max_request_body_mib, 300);
    let raw: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(raw["helperMaxRequestBodyMib"], json!(300));

    // 写入路径：超范围值夹取到 32~512。
    store
        .update(json!({"helperMaxRequestBodyMib": 9999}))
        .unwrap();
    assert_eq!(store.load().unwrap().helper_max_request_body_mib, 512);
    store.update(json!({"helperMaxRequestBodyMib": 1})).unwrap();
    assert_eq!(store.load().unwrap().helper_max_request_body_mib, 32);

    // 手改 settings.json 写入超范围值，读取时同样夹取。
    let mut raw = serde_json::to_value(BackendSettings::default()).unwrap();
    raw["helperMaxRequestBodyMib"] = json!(4096);
    std::fs::write(&path, serde_json::to_vec(&raw).unwrap()).unwrap();
    assert_eq!(store.load().unwrap().helper_max_request_body_mib, 512);
}

#[test]
fn helper_body_limit_bytes_never_overflow() {
    // 极端值（手改配置 / 未来字段类型变化）换算字节时不能溢出或回绕。
    let mut settings = BackendSettings::default();
    settings.helper_max_request_body_mib = u32::MAX;
    assert_eq!(settings.helper_max_request_body_bytes(), 512 * 1024 * 1024);

    settings.helper_max_request_body_mib = 0;
    assert_eq!(settings.helper_max_request_body_bytes(), 32 * 1024 * 1024);
}
