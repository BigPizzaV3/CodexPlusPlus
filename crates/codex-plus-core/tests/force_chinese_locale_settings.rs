use codex_plus_core::assets::injection_script_with_settings;
use codex_plus_core::settings::{BackendSettings, SettingsStore};
use serde_json::{Value, json};

const RETIRED_KEYS: &[&str] = &[
    "codexAppFastStartup",
    "codexAppNativeMenuLocalization",
    "codexAppZedRemoteOpen",
    "zedRemoteOpenStrategy",
    "zedRemoteProjectRegistryEnabled",
    "zedRemoteSyncToZedSettings",
    "codexAppUpstreamWorktreeCreate",
];

fn old_settings() -> Value {
    json!({
        "codexAppFastStartup": true,
        "codexAppForceChineseLocale": true, "codexAppNativeMenuLocalization": true,
        "codexAppZedRemoteOpen": true, "zedRemoteOpenStrategy": "addToFocusedWorkspace",
        "zedRemoteProjectRegistryEnabled": true, "zedRemoteSyncToZedSettings": true,
        "codexAppUpstreamWorktreeCreate": true, "codexAppNativeMenuPlacement": true,
        "codexAppStepwiseEnabled": true, "dictation": {"enabled": true},
        "preservedUnknownSetting": "keep",
    })
}

#[test]
fn retired_features_are_ignored_when_loading_old_settings() {
    let settings: BackendSettings = serde_json::from_value(old_settings()).unwrap();
    let serialized = serde_json::to_value(&settings).unwrap();
    for key in RETIRED_KEYS {
        assert!(serialized.get(*key).is_none(), "{key}");
    }
    assert!(settings.codex_app_native_menu_placement);
    assert!(settings.codex_app_stepwise_enabled);
    assert!(settings.dictation.enabled);
}

#[test]
fn updates_discard_retired_fields_and_preserve_other_settings() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    std::fs::write(&path, serde_json::to_vec(&old_settings()).unwrap()).unwrap();
    let store = SettingsStore::new(path.clone());
    let updated = store.update(old_settings()).unwrap();
    let raw: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    for key in RETIRED_KEYS {
        assert!(raw.get(*key).is_none(), "{key}");
    }
    assert_eq!(raw["preservedUnknownSetting"], "keep");
    assert!(updated.codex_app_native_menu_placement);
    assert!(updated.codex_app_stepwise_enabled);
    assert!(updated.dictation.enabled);
}

#[test]
fn force_chinese_locale_defaults_to_false() {
    let settings = BackendSettings::default();
    assert!(!settings.codex_app_force_chinese_locale);

    let json = serde_json::to_value(&settings).expect("serialize default settings");
    assert_eq!(
        json.get("codexAppForceChineseLocale")
            .and_then(|v| v.as_bool()),
        Some(false),
        "default BackendSettings JSON should include codexAppForceChineseLocale = false"
    );
    assert!(
        json.get("codexAppFastStartup").is_none(),
        "fast startup stays retired"
    );
}

#[test]
fn force_chinese_locale_missing_from_old_json_defaults_to_false() {
    let json = serde_json::json!({
        "codexAppPath": "",
        "enhancementsEnabled": true,
    });

    let parsed: BackendSettings = serde_json::from_value(json)
        .expect("old settings JSON without codexAppForceChineseLocale should still load");
    assert!(
        !parsed.codex_app_force_chinese_locale,
        "用户从没勾过这个开关，升级后不该被替他们打开"
    );
}

#[test]
fn force_chinese_locale_true_round_trips_through_json() {
    let mut settings = BackendSettings::default();
    settings.codex_app_force_chinese_locale = true;

    let json = serde_json::to_value(&settings).expect("serialize");
    assert_eq!(
        json.get("codexAppForceChineseLocale")
            .and_then(|v| v.as_bool()),
        Some(true)
    );

    let parsed: BackendSettings =
        serde_json::from_value(json).expect("deserialize codexAppForceChineseLocale");
    assert!(parsed.codex_app_force_chinese_locale);
}

#[test]
fn force_chinese_locale_config_follows_the_switch() {
    let mut settings = BackendSettings::default();
    settings.codex_app_force_chinese_locale = true;
    assert_eq!(
        codex_plus_core::assets::force_chinese_locale_config(&settings),
        json!({ "enabled": true, "locale": "zh-CN" })
    );

    settings.codex_app_force_chinese_locale = false;
    assert_eq!(
        codex_plus_core::assets::force_chinese_locale_config(&settings),
        json!({ "enabled": false, "locale": "zh-CN" })
    );
}

#[test]
fn force_chinese_locale_is_injected_and_fast_startup_is_not() {
    let settings: BackendSettings = serde_json::from_value(old_settings()).unwrap();
    let script = injection_script_with_settings(0, &settings);
    assert!(script.contains("window.__CODEX_PLUS_FORCE_CHINESE_LOCALE__ = {\"enabled\":true,\"locale\":\"zh-CN\"}"));
    assert!(script.contains("installCodexPlusForceChineseLocale"));
    // 撤下后没有回来：清理 shim 和快速启动都不得再出现在产物里。
    assert!(!script.contains("restoreCodexPlusManagedLocale"));
    assert!(!script.contains("__CODEX_PLUS_FAST_STARTUP__"));
    assert!(!script.contains("__codexPlusFastStartupInstalled"));
    assert!(!script.contains("statsigTimeoutMs"));
}
