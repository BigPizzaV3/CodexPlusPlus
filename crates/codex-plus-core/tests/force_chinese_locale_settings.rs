use codex_plus_core::assets::injection_script_with_settings;
use codex_plus_core::settings::{BackendSettings, SettingsStore};
use serde_json::{Value, json};

const RETIRED_KEYS: &[&str] = &[
    "codexAppFastStartup",
    "codexAppForceChineseLocale",
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
fn retired_language_and_fast_startup_do_not_affect_injection() {
    let settings: BackendSettings = serde_json::from_value(old_settings()).unwrap();
    let script = injection_script_with_settings(0, &settings);
    assert!(!script.contains("__CODEX_PLUS_FORCE_CHINESE_LOCALE__"));
    assert!(!script.contains("__codexPlusForceChineseLocaleInstalled"));
    assert!(!script.contains("installForceChineseLocalePatch"));
    assert!(!script.contains("__CODEX_PLUS_FAST_STARTUP__"));
    assert!(!script.contains("__codexPlusFastStartupInstalled"));
    assert!(!script.contains("statsigTimeoutMs"));
}
