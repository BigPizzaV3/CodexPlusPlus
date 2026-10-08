use codex_plus_core::settings::{BackendSettings, SettingsStore};
use serde_json::json;

#[test]
fn whale_widget_is_opt_in_for_existing_settings() {
    assert!(!BackendSettings::default().codex_app_whale_widget_enabled);
    let settings: BackendSettings =
        serde_json::from_value(json!({"enhancementsEnabled": true})).unwrap();
    assert!(!settings.codex_app_whale_widget_enabled);
    assert_eq!(
        serde_json::to_value(settings).unwrap()["codexAppWhaleWidgetEnabled"],
        false
    );
}

#[test]
fn whale_widget_partial_updates_preserve_other_settings_and_ignore_invalid_patches() {
    let temp = tempfile::tempdir().unwrap();
    let store = SettingsStore::new(temp.path().join("settings.json"));
    let mut settings = BackendSettings::default();
    settings.codex_app_thread_id_badge = true;
    settings.relay_profiles[0].name = "原供应商".to_string();
    store.save(&settings).unwrap();
    let enabled = store
        .update(json!({"codexAppWhaleWidgetEnabled": true}))
        .unwrap();
    assert!(enabled.codex_app_whale_widget_enabled);
    assert!(enabled.codex_app_thread_id_badge);
    assert_eq!(enabled.relay_profiles[0].name, "原供应商");
    for invalid in [json!(null), json!("true"), json!(1), json!([]), json!({})] {
        let updated = store
            .update(json!({"codexAppWhaleWidgetEnabled": invalid}))
            .unwrap();
        assert!(updated.codex_app_whale_widget_enabled);
    }
    let updated = store
        .update(json!({"codexAppThreadIdBadge": false}))
        .unwrap();
    assert!(updated.codex_app_whale_widget_enabled);
    let disabled = store
        .update(json!({"codexAppWhaleWidgetEnabled": false}))
        .unwrap();
    assert!(!disabled.codex_app_whale_widget_enabled);
    assert_eq!(store.load().unwrap(), disabled);
}

#[test]
fn enhancement_switch_gates_whale_without_erasing_user_choice() {
    let settings = BackendSettings {
        codex_app_whale_widget_enabled: true,
        enhancements_enabled: false,
        ..BackendSettings::default()
    };
    assert!(!codex_plus_core::whale::enabled(&settings));
    assert!(settings.codex_app_whale_widget_enabled);
}

#[test]
fn custom_balance_settings_round_trip_and_preserve_unrelated_fields() {
    let temp = tempfile::tempdir().unwrap();
    let store = SettingsStore::new(temp.path().join("settings.json"));
    let defaults = store.load().unwrap();
    assert_eq!(defaults.codex_app_whale_balance_protocol, "auto");
    assert_eq!(defaults.codex_app_whale_balance_scale, 1.0);
    assert_eq!(defaults.codex_app_whale_balance_currency, "USD");
    let configured = store
        .update(json!({
            "codexAppWhaleWidgetEnabled": true,
            "codexAppWhaleBalanceProtocol": "custom",
            "codexAppWhaleBalancePath": " /api/balance ",
            "codexAppWhaleBalanceField": " data.balance ",
            "codexAppWhaleBalanceCurrency": " cny ",
            "codexAppWhaleBalanceScale": 0.000002,
        }))
        .unwrap();
    assert_eq!(configured.codex_app_whale_balance_protocol, "custom");
    assert_eq!(configured.codex_app_whale_balance_path, "/api/balance");
    assert_eq!(configured.codex_app_whale_balance_field, "data.balance");
    assert_eq!(configured.codex_app_whale_balance_currency, "CNY");
    assert_eq!(configured.codex_app_whale_balance_scale, 0.000002);
    assert_eq!(store.load().unwrap(), configured);
    for invalid in [json!(null), json!(-1), json!(0), json!("1"), json!(true)] {
        let updated = store.update(json!({"codexAppWhaleBalanceScale": invalid, "codexAppWhaleBalanceProtocol":"invalid"})).unwrap();
        assert_eq!(updated.codex_app_whale_balance_protocol, "custom");
        assert_eq!(updated.codex_app_whale_balance_scale, 0.000002);
    }
    let off = store
        .update(json!({"codexAppWhaleBalanceProtocol":"off"}))
        .unwrap();
    assert!(off.codex_app_whale_widget_enabled);
    assert_eq!(off.codex_app_whale_balance_field, "data.balance");
}
