// 协议代理自动重试设置：默认值、部分更新白名单与落盘夹取。
//
// 与旧的容量自动重试设置测试同样，这里刻意同时覆盖 `store.save()`（管理器整份保存）
// 和 `store.update()`（注入层 `/settings/set` 的部分更新白名单）：漏登记
// merge_known_setting_fields 的键会 HTTP 200 但值不变，必须由测试守住。
use codex_plus_core::settings::{BackendSettings, SettingsStore};
use serde_json::json;

#[test]
fn proxy_auto_retry_defaults_to_on_for_old_settings() {
    let defaults = BackendSettings::default();
    assert!(defaults.protocol_proxy_auto_retry);
    assert_eq!(defaults.protocol_proxy_auto_retry_max_attempts, 5);
    assert_eq!(defaults.protocol_proxy_auto_retry_interval_seconds, 2);
    assert_eq!(defaults.protocol_proxy_auto_retry_mode, "streaming");
    assert_eq!(defaults.protocol_proxy_auto_retry_commit_delay_ms, 1000);
    let raw = serde_json::to_value(&defaults).unwrap();
    assert_eq!(raw["protocolProxyAutoRetry"], json!(true));
    assert_eq!(raw["protocolProxyAutoRetryMaxAttempts"], json!(5));
    assert_eq!(raw["protocolProxyAutoRetryIntervalSeconds"], json!(2));
    assert_eq!(raw["protocolProxyAutoRetryCommitDelayMs"], json!(1000));

    // 旧 settings.json 没有这些键时必须补默认值，而不是报错或落成 0。
    let settings: BackendSettings = serde_json::from_value(json!({
        "enhancementsEnabled": true,
        "codexAppThreadIdBadge": true,
    }))
    .unwrap();
    assert!(settings.protocol_proxy_auto_retry);
    assert_eq!(settings.protocol_proxy_auto_retry_max_attempts, 5);
    assert_eq!(settings.protocol_proxy_auto_retry_interval_seconds, 2);
    assert_eq!(settings.protocol_proxy_auto_retry_mode, "streaming");
    assert_eq!(settings.protocol_proxy_auto_retry_commit_delay_ms, 1000);
    assert!(settings.codex_app_thread_id_badge);
}

#[test]
fn proxy_auto_retry_round_trips_through_partial_updates() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    let store = SettingsStore::new(path.clone());

    let mut settings = BackendSettings::default();
    settings.protocol_proxy_auto_retry = false;
    settings.protocol_proxy_auto_retry_max_attempts = 3;
    settings.protocol_proxy_auto_retry_interval_seconds = 0;
    settings.protocol_proxy_auto_retry_commit_delay_ms = 2500;
    store.save(&settings).unwrap();
    let loaded = store.load().unwrap();
    assert!(!loaded.protocol_proxy_auto_retry);
    assert_eq!(loaded.protocol_proxy_auto_retry_max_attempts, 3);
    assert_eq!(loaded.protocol_proxy_auto_retry_interval_seconds, 0);
    assert_eq!(loaded.protocol_proxy_auto_retry_commit_delay_ms, 2500);

    // 四个键都要真正写进去，证明 merge 白名单里都有。
    store
        .update(json!({
            "protocolProxyAutoRetry": true,
            "protocolProxyAutoRetryMaxAttempts": 7,
            "protocolProxyAutoRetryIntervalSeconds": 9,
            "protocolProxyAutoRetryMode": "buffered",
            "protocolProxyAutoRetryCommitDelayMs": 0,
        }))
        .unwrap();
    let updated = store.load().unwrap();
    assert!(updated.protocol_proxy_auto_retry);
    assert_eq!(updated.protocol_proxy_auto_retry_max_attempts, 7);
    assert_eq!(updated.protocol_proxy_auto_retry_interval_seconds, 9);
    assert_eq!(updated.protocol_proxy_auto_retry_mode, "buffered");
    assert_eq!(updated.protocol_proxy_auto_retry_commit_delay_ms, 0);

    let raw: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(raw["protocolProxyAutoRetry"], json!(true));
    assert_eq!(raw["protocolProxyAutoRetryMaxAttempts"], json!(7));
    assert_eq!(raw["protocolProxyAutoRetryIntervalSeconds"], json!(9));
    assert_eq!(raw["protocolProxyAutoRetryMode"], json!("buffered"));
    assert_eq!(raw["protocolProxyAutoRetryCommitDelayMs"], json!(0));
    // 只改这三个键不该动到别的字段。
    assert!(raw["codexAppThreadScrollRestore"].as_bool().unwrap());
}

#[test]
fn proxy_auto_retry_ranges_are_clamped_on_load_and_update() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    let mut raw = serde_json::to_value(BackendSettings::default()).unwrap();
    raw["protocolProxyAutoRetryMaxAttempts"] = json!(999);
    raw["protocolProxyAutoRetryIntervalSeconds"] = json!(999);
    raw["protocolProxyAutoRetryCommitDelayMs"] = json!(99999);
    std::fs::write(&path, serde_json::to_vec(&raw).unwrap()).unwrap();

    let store = SettingsStore::new(path.clone());
    let loaded = store.load().unwrap();
    assert_eq!(loaded.protocol_proxy_auto_retry_max_attempts, 20);
    assert_eq!(loaded.protocol_proxy_auto_retry_interval_seconds, 30);
    assert_eq!(loaded.protocol_proxy_auto_retry_commit_delay_ms, 10000);

    let updated = store
        .update(json!({
            "protocolProxyAutoRetryMaxAttempts": 1000,
            "protocolProxyAutoRetryIntervalSeconds": 1000,
            "protocolProxyAutoRetryCommitDelayMs": 100000,
        }))
        .unwrap();
    assert_eq!(updated.protocol_proxy_auto_retry_max_attempts, 20);
    assert_eq!(updated.protocol_proxy_auto_retry_interval_seconds, 30);
    assert_eq!(updated.protocol_proxy_auto_retry_commit_delay_ms, 10000);

    let raw: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(raw["protocolProxyAutoRetryMaxAttempts"], json!(20));
    assert_eq!(raw["protocolProxyAutoRetryIntervalSeconds"], json!(30));
    assert_eq!(raw["protocolProxyAutoRetryCommitDelayMs"], json!(10000));
}

#[test]
fn legacy_streaming_mode_is_preserved_and_unknown_modes_fall_back() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    let store = SettingsStore::new(path);

    store
        .update(json!({"protocolProxyAutoRetryMode": "streaming-legacy"}))
        .unwrap();
    assert_eq!(
        store.load().unwrap().protocol_proxy_auto_retry_mode,
        "streaming-legacy"
    );

    // 未知/旧值一律回落到默认的新流式模式，不能写坏配置。
    store
        .update(json!({"protocolProxyAutoRetryMode": "bogus"}))
        .unwrap();
    assert_eq!(
        store.load().unwrap().protocol_proxy_auto_retry_mode,
        "streaming"
    );
}
