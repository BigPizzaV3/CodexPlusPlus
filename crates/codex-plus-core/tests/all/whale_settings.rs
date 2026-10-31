use codex_plus_core::settings::{AgentPetSettings, BackendSettings, SettingsStore};
use serde_json::json;

#[test]
fn whale_widget_is_opt_in_for_existing_settings() {
    // 大肥鱼是 Agent 桌宠的一种类型，默认不选；旧开关不再写出。
    assert!(!codex_plus_core::whale::enabled(&BackendSettings::default()));
    let settings: BackendSettings =
        serde_json::from_value(json!({"enhancementsEnabled": true})).unwrap();
    assert!(!codex_plus_core::whale::enabled(&settings));
    assert_eq!(settings.agent_pet.kind, "image");
    let value = serde_json::to_value(settings).unwrap();
    assert!(value.get("codexAppWhaleWidgetEnabled").is_none());
    assert_eq!(value["agentPet"]["kind"], "image");
}

#[test]
fn activity_overlay_right_layout_is_opt_in_and_persisted() {
    let temp = tempfile::tempdir().unwrap();
    let store = SettingsStore::new(temp.path().join("settings.json"));
    let defaults = store.load().unwrap();
    assert!(!defaults.codex_app_activity_overlay_right);

    let enabled = store
        .update(json!({"codexAppActivityOverlayRight": true}))
        .unwrap();
    assert!(enabled.codex_app_activity_overlay_right);
    assert!(store.load().unwrap().codex_app_activity_overlay_right);

    let disabled = store
        .update(json!({"codexAppActivityOverlayRight": false}))
        .unwrap();
    assert!(!disabled.codex_app_activity_overlay_right);
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
        .update(json!({"agentPet": {"enabled": true, "kind": "whaleWidget"}}))
        .unwrap();
    assert!(codex_plus_core::whale::enabled(&enabled));
    assert!(enabled.codex_app_thread_id_badge);
    assert_eq!(enabled.relay_profiles[0].name, "原供应商");
    // agentPet 补丁不是对象，或对象里没有 kind / enabled 时，不会把大肥鱼关掉。
    for invalid in [json!(null), json!("true"), json!(1), json!([]), json!({})] {
        let updated = store.update(json!({"agentPet": invalid})).unwrap();
        assert!(codex_plus_core::whale::enabled(&updated), "{invalid}");
    }
    // 已移除的旧开关不再生效。
    let updated = store
        .update(json!({"codexAppWhaleWidgetEnabled": false, "codexAppThreadIdBadge": false}))
        .unwrap();
    assert!(codex_plus_core::whale::enabled(&updated));
    let disabled = store
        .update(json!({"agentPet": {"enabled": false}}))
        .unwrap();
    assert!(!codex_plus_core::whale::enabled(&disabled));
    assert_eq!(
        disabled.agent_pet.kind, "whaleWidget",
        "关掉桌宠不丢类型选择"
    );
    assert_eq!(store.load().unwrap(), disabled);
}

#[test]
fn whale_follows_agent_pet_selection_not_enhancement_switch() {
    // 与其他 Agent 桌宠一致：不受「Codex 增强」总开关影响，关掉增强也不会抹掉用户的选择。
    let settings = BackendSettings {
        agent_pet: AgentPetSettings::whale_widget(),
        enhancements_enabled: false,
        ..BackendSettings::default()
    };
    assert!(codex_plus_core::whale::enabled(&settings));
    // 选回简单桌宠或取消勾选 Codex，大肥鱼就关闭。
    let mut image = settings.clone();
    image.agent_pet.kind = "image".into();
    assert!(!codex_plus_core::whale::enabled(&image));
    let mut no_codex = settings;
    no_codex.agent_pet.agents.clear();
    assert!(!codex_plus_core::whale::enabled(&no_codex));
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
            "agentPet": {"enabled": true, "kind": "whaleWidget"},
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
    assert!(codex_plus_core::whale::enabled(&off));
    assert_eq!(off.codex_app_whale_balance_field, "data.balance");
}
