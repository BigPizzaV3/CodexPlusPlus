use codex_plus_core::assets::{
    injection_script_with_settings, statsig_gate_catalog, statsig_overrides_config,
};
use codex_plus_core::settings::{BackendSettings, SettingsStore, sanitize_statsig_gate_overrides};
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
    // 原生已支持但默认灰度不到的界面功能：消息队列、时间线、侧聊 attach、权限刷新
    assert_eq!(value["gates"]["2120612410"], json!(true));
    assert_eq!(value["gates"]["1189013788"], json!(true));
    assert_eq!(value["gates"]["1079089693"], json!(true));
    assert_eq!(value["gates"]["1450837220"], json!(true));
    assert_eq!(value["gates"]["4226282475"], json!(true));
    // 实验 / Layer 覆盖机制默认不启用具体条目，但配置节必须存在，注入脚本才不会读空
    assert_eq!(value["experiments"], json!({}));
    assert_eq!(value["layers"], json!({}));

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
    assert!(script.contains("2120612410"));
    assert!(script.contains("\"experiments\":{}"));
    assert!(script.contains("\"layers\":{}"));

    let off = injection_script_with_settings(1234, &settings_with(false));
    let marker = "window.__CODEX_PLUS_STATSIG_OVERRIDES__ = ";
    let start = off.find(marker).expect("注入脚本应带 Statsig 覆盖配置") + marker.len();
    let rest = &off[start..];
    let end = rest.find(";\n").expect("配置应以分号结尾");
    let parsed: Value = serde_json::from_str(&rest[..end]).unwrap();
    assert_eq!(parsed["enabled"], json!(false), "关掉时只下发 enabled:false");
}

/// 默认覆盖的六个闸门：清单从 JSON 资源读出来以后，行为必须和以前写死时完全一样。
const DEFAULT_GATES: [&str; 6] = [
    "1174206456",
    "2120612410",
    "1189013788",
    "1079089693",
    "1450837220",
    "4226282475",
];

#[test]
fn statsig_gate_catalog_is_well_formed() {
    let catalog = statsig_gate_catalog();
    assert!(catalog.len() >= 10, "清单应能解析出来：{}", catalog.len());
    let mut seen = std::collections::HashSet::new();
    for entry in &catalog {
        let id = entry["id"].as_str().expect("id 是字符串");
        assert!(
            !id.is_empty() && id.len() <= 12 && id.bytes().all(|byte| byte.is_ascii_digit()),
            "开关 ID 只能是数字：{id}"
        );
        assert!(seen.insert(id.to_string()), "开关 ID 重复：{id}");
        assert!(
            entry["title"].as_str().is_some_and(|title| !title.trim().is_empty()),
            "{id} 缺标题"
        );
        assert!(
            entry["description"].as_str().is_some_and(|text| !text.trim().is_empty()),
            "{id} 缺说明"
        );
        assert!(
            matches!(entry["risk"].as_str(), Some("low" | "medium" | "high")),
            "{id} 风险等级不对"
        );
        assert!(
            entry["defaultOverride"] == json!(true) || entry["defaultOverride"].is_null(),
            "{id} 的 defaultOverride 只能是 true 或 null"
        );
    }
}

#[test]
fn statsig_default_gates_come_from_catalog_and_stay_the_same() {
    let value = statsig_overrides_config(&settings_with(true));
    let gates = value["gates"].as_object().unwrap();
    let mut ids = gates.keys().cloned().collect::<Vec<_>>();
    ids.sort();
    let mut expected = DEFAULT_GATES.iter().map(|id| id.to_string()).collect::<Vec<_>>();
    expected.sort();
    assert_eq!(ids, expected, "默认覆盖集合不能因为改成清单而变化");
    assert!(gates.values().all(|value| *value == json!(true)));

    // 新加的开关只进清单，默认不覆盖
    for id in ["623263923", "983842163", "3009531526", "1372061905"] {
        assert!(!gates.contains_key(id), "{id} 默认不该被覆盖");
    }
}

#[test]
fn statsig_config_ships_catalog_for_feature_lab() {
    let value = statsig_overrides_config(&settings_with(false));
    let catalog = value["gateCatalog"].as_array().expect("关着总开关也要下发清单，Feature Lab 才能显示");
    let provider_page = catalog
        .iter()
        .find(|entry| entry["id"] == json!("623263923"))
        .expect("清单里应有「模型提供商」页的开关");
    assert_eq!(provider_page["risk"], json!("high"));
    assert!(provider_page["defaultOverride"].is_null());
    assert!(
        provider_page["note"].as_str().is_some_and(|note| note.contains("Codex++")),
        "要提示和 Codex++ 供应商页的冲突"
    );
}

#[test]
fn statsig_gate_overrides_drop_invalid_entries() {
    let source = json!({
        "623263923": "on",
        "983842163": "off",
        "3009531526": "native",
        " 1372061905 ": "on",
        "not-a-number": "on",
        "1234567890123": "on",
        "111": "maybe",
        "222": true,
        "": "on"
    });
    let cleaned = sanitize_statsig_gate_overrides(source.as_object().unwrap());
    assert_eq!(
        Value::Object(cleaned),
        json!({
            "623263923": "on",
            "983842163": "off",
            "3009531526": "native",
            "1372061905": "on"
        })
    );
}

#[test]
fn statsig_gate_overrides_round_trip_and_stay_out_of_file_when_empty() {
    let settings = BackendSettings::default();
    assert!(settings.codex_app_statsig_gate_overrides.is_empty());
    let serialized = serde_json::to_value(&settings).unwrap();
    assert!(
        serialized.get("codexAppStatsigGateOverrides").is_none(),
        "没设过时不写进设置文件"
    );

    let parsed: BackendSettings = serde_json::from_value(json!({
        "codexAppStatsigGateOverrides": { "623263923": "on" }
    }))
    .unwrap();
    assert_eq!(
        parsed.codex_app_statsig_gate_overrides.get("623263923").map(String::as_str),
        Some("on")
    );
}

#[test]
fn statsig_gate_overrides_saved_through_page_settings_patch() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    let store = SettingsStore::new(path.clone());

    // 页面经 /settings/set 整份提交逐项覆盖：非法条目丢掉，其余落盘。
    let updated = store
        .update(json!({
            "codexAppStatsigGateOverrides": {
                "983842163": "on",
                "1174206456": "native",
                "evil.key": "on"
            }
        }))
        .unwrap();
    assert_eq!(updated.codex_app_statsig_gate_overrides.len(), 2);
    assert_eq!(
        updated.codex_app_statsig_gate_overrides.get("1174206456").map(String::as_str),
        Some("native")
    );

    let saved: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        saved["codexAppStatsigGateOverrides"],
        json!({ "983842163": "on", "1174206456": "native" })
    );

    // 改总开关等别的字段不会冲掉逐项覆盖。
    let again = store.update(json!({ "codexAppStatsigOverrides": true })).unwrap();
    assert!(again.codex_app_statsig_overrides);
    assert_eq!(again.codex_app_statsig_gate_overrides.len(), 2);

    // 提交空表 = 全部恢复默认。
    let cleared = store.update(json!({ "codexAppStatsigGateOverrides": {} })).unwrap();
    assert!(cleared.codex_app_statsig_gate_overrides.is_empty());
}
