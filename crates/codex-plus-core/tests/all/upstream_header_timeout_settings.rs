// 上游流式响应头等待上限设置（`upstreamStreamHeaderTimeoutSecs`）：默认值、落盘/读取、
// 部分更新白名单与范围夹取。
//
// 背景（#3130 / #3260）：默认 120 秒会让首字时间经常超过 120s 的上游（长推理模型、
// 慢中转）在晚高峰大量 502，且单供应商时无从 failover。该键早已存在于 settings.json，
// #3260 把它暴露进管理器设置界面，这里守住：整份保存、`/settings/set` 部分更新、
// 手改配置三条路径的值都会被夹到 120~3600，null 表示用默认。
use codex_plus_core::settings::{BackendSettings, SettingsStore};
use serde_json::json;

#[test]
fn upstream_header_timeout_defaults_to_none() {
    let defaults = BackendSettings::default();
    assert_eq!(defaults.upstream_stream_header_timeout_secs, None);

    let raw = serde_json::to_value(&defaults).unwrap();
    assert_eq!(
        raw["upstreamStreamHeaderTimeoutSecs"],
        serde_json::Value::Null
    );

    // 旧 settings.json 没有该键时必须补默认（None → 运行时用 120），而不是报错。
    let settings: BackendSettings = serde_json::from_value(json!({
        "enhancementsEnabled": true,
    }))
    .unwrap();
    assert_eq!(settings.upstream_stream_header_timeout_secs, None);
    assert!(settings.enhancements_enabled);
}

#[test]
fn upstream_header_timeout_round_trips_and_clamps() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    let store = SettingsStore::new(path.clone());

    let mut settings = BackendSettings::default();
    settings.upstream_stream_header_timeout_secs = Some(600);
    store.save(&settings).unwrap();
    assert_eq!(
        store.load().unwrap().upstream_stream_header_timeout_secs,
        Some(600)
    );

    // 只改这一个键：必须命中 merge_known_setting_fields 白名单。
    store
        .update(json!({"upstreamStreamHeaderTimeoutSecs": 300}))
        .unwrap();
    assert_eq!(
        store.load().unwrap().upstream_stream_header_timeout_secs,
        Some(300)
    );

    // 超范围值夹取到 120~3600。
    store
        .update(json!({"upstreamStreamHeaderTimeoutSecs": 9999}))
        .unwrap();
    assert_eq!(
        store.load().unwrap().upstream_stream_header_timeout_secs,
        Some(3600)
    );
    store
        .update(json!({"upstreamStreamHeaderTimeoutSecs": 30}))
        .unwrap();
    assert_eq!(
        store.load().unwrap().upstream_stream_header_timeout_secs,
        Some(120)
    );

    // 写回 null：恢复默认 120。
    store
        .update(json!({"upstreamStreamHeaderTimeoutSecs": null}))
        .unwrap();
    assert_eq!(
        store.load().unwrap().upstream_stream_header_timeout_secs,
        None
    );

    // 手改 settings.json 写入超范围值，读取时同样夹取。
    let mut raw = serde_json::to_value(BackendSettings::default()).unwrap();
    raw["upstreamStreamHeaderTimeoutSecs"] = json!(7200);
    std::fs::write(&path, serde_json::to_vec(&raw).unwrap()).unwrap();
    assert_eq!(
        store.load().unwrap().upstream_stream_header_timeout_secs,
        Some(3600)
    );
}
