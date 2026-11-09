use codex_plus_core::assets::{injection_script_with_settings, statsig_overrides_config};
use codex_plus_core::settings::BackendSettings;
use serde_json::{Value, json};

fn settings_with(overrides: bool) -> BackendSettings {
    let mut settings = BackendSettings::default();
    settings.codex_app_statsig_overrides = overrides;
    settings
}

#[test]
fn statsig_overrides_defaults_to_off() {
    let settings = BackendSettings::default();
    assert!(!settings.codex_app_statsig_overrides);
    let value = statsig_overrides_config(&settings);
    assert_eq!(value["enabled"], json!(false));
}

#[test]
fn statsig_overrides_round_trips_through_json() {
    let parsed: BackendSettings =
        serde_json::from_value(json!({ "codexAppStatsigOverrides": true })).unwrap();
    assert!(parsed.codex_app_statsig_overrides);
    let serialized = serde_json::to_value(&parsed).unwrap();
    assert_eq!(serialized["codexAppStatsigOverrides"], json!(true));

    // 旧设置文件没有这个键时按关处理
    let legacy: BackendSettings = serde_json::from_value(json!({})).unwrap();
    assert!(!legacy.codex_app_statsig_overrides);
}

#[test]
fn statsig_overrides_config_only_carries_client_side_switches() {
    let value = statsig_overrides_config(&settings_with(true));
    assert_eq!(value["enabled"], json!(true));
    assert_eq!(value["gates"]["1174206456"], json!(true));

    let retry = &value["dynamicConfigs"]["3165814200"];
    assert_eq!(retry["MAX_RETRY_COUNT"], json!(15));
    assert_eq!(retry["MAX_RETRY_INTERVAL"], json!(8000));

    let sse = &value["dynamicConfigs"]["3608673692"];
    assert_eq!(sse["sse_initial_open_timeout_ms"], json!(90000));

    let ws = &value["dynamicConfigs"]["339548201"];
    assert_eq!(ws["websocket_idle_timeout_ms"], json!(45000));

    assert_eq!(value["dynamicConfigs"]["2808262316"]["enabled"], json!(true));

    // 服务端能力不在这一层：不该出现账号、套餐、额度之类的东西
    let text = value.to_string();
    for forbidden in ["plan_type", "account_id", "oauth", "auth.json"] {
        assert!(!text.contains(forbidden), "不该包含 {forbidden}");
    }
}

#[test]
fn injection_script_embeds_statsig_override_config() {
    let script = injection_script_with_settings(1234, &settings_with(true));
    assert!(script.contains("window.__CODEX_PLUS_STATSIG_OVERRIDES__ = "));
    assert!(script.contains("\"enabled\":true"));
    assert!(script.contains("1174206456"));

    let off = injection_script_with_settings(1234, &settings_with(false));
    let marker = "window.__CODEX_PLUS_STATSIG_OVERRIDES__ = ";
    let start = off.find(marker).expect("注入脚本应带 Statsig 覆盖配置") + marker.len();
    let rest = &off[start..];
    let end = rest.find(";\n").expect("配置应以分号结尾");
    let parsed: Value = serde_json::from_str(&rest[..end]).unwrap();
    assert_eq!(parsed["enabled"], json!(false), "关掉时只下发 enabled:false");
}
