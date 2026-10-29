use codex_plus_core::relay_config::{normalize_relay_profile_for_storage, relay_profile_api_key};
use codex_plus_core::settings::{
    BackendSettings, RelayApiKey, RelayMode, RelayProfile, SettingsStore,
};
use serde_json::json;

fn profile() -> RelayProfile {
    RelayProfile {
        id: "provider".into(),
        relay_mode: RelayMode::PureApi,
        base_url: "https://provider.example/v1".into(),
        api_key: "legacy-key".into(),
        ..Default::default()
    }
}

#[test]
fn legacy_key_migrates_and_selected_key_reaches_both_auth_locations() {
    let mut profile = profile();
    normalize_relay_profile_for_storage(&mut profile).unwrap();
    assert_eq!(profile.api_keys[0].api_key, "legacy-key");
    profile.api_keys.push(RelayApiKey {
        id: "backup".into(),
        name: "备用".into(),
        api_key: "backup-key".into(),
    });
    profile.active_api_key_id = "backup".into();
    normalize_relay_profile_for_storage(&mut profile).unwrap();
    assert_eq!(relay_profile_api_key(&profile), "backup-key");
    let auth: serde_json::Value = serde_json::from_str(&profile.auth_contents).unwrap();
    assert_eq!(auth["OPENAI_API_KEY"], "backup-key");
    assert!(
        profile
            .config_contents
            .contains("experimental_bearer_token = \"backup-key\"")
    );
}

#[test]
fn named_keys_survive_settings_save_load_and_tool_shard_sync() {
    let temp = tempfile::tempdir().unwrap();
    let store = SettingsStore::new(temp.path().join("settings.json"));
    let mut profile = profile();
    profile.api_keys = vec![
        RelayApiKey {
            id: "main".into(),
            name: "主账号".into(),
            api_key: "main-key".into(),
        },
        RelayApiKey {
            id: "backup".into(),
            name: "备用".into(),
            api_key: "backup-key".into(),
        },
    ];
    profile.active_api_key_id = "backup".into();
    store
        .save(&BackendSettings {
            active_relay_id: profile.id.clone(),
            relay_profiles: vec![profile],
            ..Default::default()
        })
        .unwrap();
    let loaded = store.load().unwrap();
    assert_eq!(loaded.relay_profiles[0].api_keys.len(), 2);
    assert_eq!(loaded.active_relay_profile().active_api_key_id, "backup");
    assert_eq!(
        relay_profile_api_key(&loaded.active_relay_profile()),
        "backup-key"
    );
    let raw: serde_json::Value =
        serde_json::from_slice(&std::fs::read(temp.path().join("settings.json")).unwrap()).unwrap();
    assert_eq!(raw["relayProfiles"][0]["activeApiKeyId"], "backup");
}

#[test]
fn missing_selection_falls_back_and_empty_current_key_does_not_borrow_another_key() {
    let mut profile = profile();
    profile.api_keys = serde_json::from_value(json!([
        {"id":"main","name":"主账号","apiKey":""},
        {"id":"backup","name":"备用","apiKey":"backup-key"}
    ]))
    .unwrap();
    profile.active_api_key_id = "missing".into();
    normalize_relay_profile_for_storage(&mut profile).unwrap();
    assert_eq!(profile.active_api_key_id, "main");
    assert_eq!(relay_profile_api_key(&profile), "");
}

#[test]
fn edited_selected_key_survives_storage_apply_and_backfill() {
    let temp = tempfile::tempdir().unwrap();
    let store = SettingsStore::new(temp.path().join("settings.json"));
    let mut profile = profile();
    normalize_relay_profile_for_storage(&mut profile).unwrap();
    profile.api_keys.push(RelayApiKey {
        id: "backup".into(),
        name: "备用".into(),
        api_key: "backup-key".into(),
    });
    profile.api_key = "rotated-key".into();
    profile.api_keys[0].api_key = profile.api_key.clone();
    profile.auth_contents = json!({"OPENAI_API_KEY":"rotated-key"}).to_string();
    profile.config_contents = profile.config_contents.replace("legacy-key", "rotated-key");
    store
        .save(&BackendSettings {
            active_relay_id: profile.id.clone(),
            relay_profiles: vec![profile],
            ..Default::default()
        })
        .unwrap();
    let mut reloaded = store.load().unwrap().active_relay_profile();
    codex_plus_core::relay_config::apply_relay_profile_to_home_with_switch_rules(
        temp.path(),
        &reloaded,
        "",
    )
    .unwrap();
    codex_plus_core::relay_config::backfill_relay_profile_from_home_with_common(
        temp.path(),
        &mut reloaded,
        &mut String::new(),
    )
    .unwrap();
    normalize_relay_profile_for_storage(&mut reloaded).unwrap();
    assert_eq!(reloaded.active_api_key_id, "default");
    assert_eq!(reloaded.api_keys[0].api_key, "rotated-key");
    assert_eq!(reloaded.api_keys[1].api_key, "backup-key");
    assert_eq!(relay_profile_api_key(&reloaded), "rotated-key");
    let auth: serde_json::Value = serde_json::from_str(&reloaded.auth_contents).unwrap();
    assert_eq!(auth["OPENAI_API_KEY"], "rotated-key");
    let config: toml::Value = std::fs::read_to_string(temp.path().join("config.toml"))
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(
        config["model_providers"]["custom"]["experimental_bearer_token"].as_str(),
        Some("rotated-key")
    );
}
