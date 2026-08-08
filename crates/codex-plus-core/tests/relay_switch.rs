use codex_plus_core::relay_switch::switch_relay_profile_in_home;
use codex_plus_core::settings::{
    AggregateRelayMember, AggregateRelayProfile, AggregateRelayStrategy, BackendSettings,
    LaunchMode, RelayMode, RelayProfile, SettingsStore,
};

#[test]
fn switch_rolls_back_active_settings_when_live_write_fails() {
    let temp = tempfile::tempdir().unwrap();
    let store = SettingsStore::new(temp.path().join("settings.json"));
    let original = BackendSettings {
        active_relay_id: "a".to_string(),
        relay_profiles: vec![pure_profile("a", "https://a.example/v1", "sk-a")],
        ..BackendSettings::default()
    };
    store.save(&original).unwrap();
    std::fs::create_dir(temp.path().join("codex")).unwrap();
    std::fs::write(
        temp.path().join("codex").join("auth.json"),
        r#"{"OPENAI_API_KEY":"sk-a"}"#,
    )
    .unwrap();
    std::fs::write(
        temp.path().join("codex").join("config.toml"),
        r#"model_provider = "custom"

[model_providers.custom]
name = "custom"
wire_api = "responses"
requires_openai_auth = true
base_url = "https://a.example/v1"
"#,
    )
    .unwrap();
    let next = BackendSettings {
        active_relay_id: "b".to_string(),
        relay_profiles: vec![
            pure_profile("a", "https://a.example/v1", "sk-a"),
            RelayProfile {
                id: "b".to_string(),
                name: "B".to_string(),
                relay_mode: RelayMode::PureApi,
                config_contents: "model_provider = \"custom\"\n".to_string(),
                auth_contents: "{bad json".to_string(),
                ..RelayProfile::default()
            },
        ],
        ..BackendSettings::default()
    };

    let error = switch_relay_profile_in_home(&store, &temp.path().join("codex"), next, "a")
        .expect_err("invalid auth should fail switch");

    assert!(error.to_string().contains("auth.json"));
    assert_eq!(store.load().unwrap().active_relay_id, "a");
    assert!(
        std::fs::read_to_string(temp.path().join("codex").join("config.toml"))
            .unwrap()
            .contains("https://a.example/v1")
    );
    assert_eq!(
        std::fs::read_to_string(temp.path().join("codex").join("auth.json")).unwrap(),
        r#"{"OPENAI_API_KEY":"sk-a"}"#
    );
}

#[test]
fn switch_rolls_back_live_files_when_post_write_status_check_fails() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex");
    std::fs::create_dir(&home).unwrap();
    std::fs::write(home.join("auth.json"), r#"{"OPENAI_API_KEY":"sk-a"}"#).unwrap();
    std::fs::write(
        home.join("config.toml"),
        r#"model_provider = "custom"

[model_providers.custom]
name = "custom"
wire_api = "responses"
requires_openai_auth = true
base_url = "https://a.example/v1"
"#,
    )
    .unwrap();
    let store = SettingsStore::new(temp.path().join("settings.json"));
    let original = BackendSettings {
        active_relay_id: "a".to_string(),
        relay_profiles: vec![pure_profile("a", "https://a.example/v1", "sk-a")],
        ..BackendSettings::default()
    };
    store.save(&original).unwrap();
    let next = BackendSettings {
        active_relay_id: "b".to_string(),
        relay_profiles: vec![
            pure_profile("a", "https://a.example/v1", "sk-a"),
            RelayProfile {
                id: "b".to_string(),
                name: "B".to_string(),
                relay_mode: RelayMode::PureApi,
                config_contents: r#"model_provider = "custom"

[model_providers.custom]
name = "custom"
wire_api = "responses"
requires_openai_auth = true
base_url = "https://b.example/v1"
"#
                .to_string(),
                auth_contents: "{}".to_string(),
                ..RelayProfile::default()
            },
        ],
        ..BackendSettings::default()
    };

    let error = switch_relay_profile_in_home(&store, &home, next, "a")
        .expect_err("missing api key should fail post-write status check");

    assert!(
        error
            .to_string()
            .contains("纯 API 配置写入后未检测到完整 custom provider")
    );
    assert_eq!(store.load().unwrap().active_relay_id, "a");
    assert!(
        std::fs::read_to_string(home.join("config.toml"))
            .unwrap()
            .contains("https://a.example/v1")
    );
    assert_eq!(
        std::fs::read_to_string(home.join("auth.json")).unwrap(),
        r#"{"OPENAI_API_KEY":"sk-a"}"#
    );
}

#[test]
fn switch_backfills_previous_profile_from_live_before_selecting_target() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex");
    std::fs::create_dir(&home).unwrap();
    std::fs::write(
        home.join("config.toml"),
        r#"model = "edited-live-model"
model_provider = "manual_a"
model_context_window = 1000000
model_auto_compact_token_limit = 900000

[model_providers.manual_a]
name = "manual_a"
wire_api = "responses"
requires_openai_auth = true
base_url = "https://edited-a.example/v1"
"#,
    )
    .unwrap();
    std::fs::write(
        home.join("auth.json"),
        r#"{"OPENAI_API_KEY":"sk-edited-a"}"#,
    )
    .unwrap();
    let store = SettingsStore::new(temp.path().join("settings.json"));
    let original = BackendSettings {
        active_relay_id: "a".to_string(),
        relay_profiles: vec![
            pure_profile("a", "https://a.example/v1", "sk-a"),
            pure_profile("b", "https://b.example/v1", "sk-b"),
        ],
        ..BackendSettings::default()
    };
    store.save(&original).unwrap();
    let next = BackendSettings {
        active_relay_id: "b".to_string(),
        relay_profiles: original.relay_profiles.clone(),
        ..BackendSettings::default()
    };

    switch_relay_profile_in_home(&store, &home, next, "a").unwrap();

    let stored = store.load().unwrap();
    let previous = stored
        .relay_profiles
        .iter()
        .find(|profile| profile.id == "a")
        .unwrap();
    assert!(previous.config_contents.contains("edited-live-model"));
    assert!(previous.config_contents.contains("manual_a"));
    assert_eq!(previous.context_window, "1000000");
    assert_eq!(previous.auto_compact_limit, "900000");
    assert_eq!(stored.active_relay_id, "b");
    assert_eq!(stored.launch_mode, LaunchMode::Patch);
}

#[test]
fn switch_to_aggregate_relay_allows_empty_config_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex");
    std::fs::create_dir(&home).unwrap();
    let store = SettingsStore::new(temp.path().join("settings.json"));
    let api = pure_profile("api", "https://api.example/v1", "sk-api");
    std::fs::write(home.join("config.toml"), &api.config_contents).unwrap();
    std::fs::write(home.join("auth.json"), &api.auth_contents).unwrap();
    let aggregate = RelayProfile {
        id: "agg".to_string(),
        name: "聚合供应商 1".to_string(),
        relay_mode: RelayMode::Aggregate,
        config_contents: String::new(),
        auth_contents: String::new(),
        ..RelayProfile::default()
    };
    let original = BackendSettings {
        active_relay_id: "api".to_string(),
        relay_profiles: vec![api.clone(), aggregate.clone()],
        ..BackendSettings::default()
    };
    store.save(&original).unwrap();
    let next = BackendSettings {
        active_relay_id: "agg".to_string(),
        relay_profiles: vec![api, aggregate],
        aggregate_relay_profiles: vec![AggregateRelayProfile {
            id: "agg".to_string(),
            name: "聚合供应商 1".to_string(),
            strategy: AggregateRelayStrategy::Failover,
            members: vec![AggregateRelayMember {
                relay_id: "api".to_string(),
                weight: 1,
            }],
        }],
        active_aggregate_relay_id: "agg".to_string(),
        ..BackendSettings::default()
    };

    let result = switch_relay_profile_in_home(&store, &home, next, "api").unwrap();
    let live = std::fs::read_to_string(home.join("config.toml")).unwrap();
    let auth: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.join("auth.json")).unwrap()).unwrap();

    assert!(result.configured);
    assert_eq!(store.load().unwrap().active_relay_id, "agg");
    assert_eq!(store.load().unwrap().launch_mode, LaunchMode::Patch);
    assert!(live.contains(r#"base_url = "http://127.0.0.1:57321/v1""#));
    assert_eq!(auth["OPENAI_API_KEY"], "codex-plus-aggregate");
}

#[test]
fn switch_away_from_aggregate_preserves_profile_and_can_switch_back() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex");
    std::fs::create_dir(&home).unwrap();
    std::fs::write(
        home.join("config.toml"),
        r#"model_provider = "custom"

[model_providers.custom]
name = "custom"
wire_api = "responses"
requires_openai_auth = true
base_url = "http://127.0.0.1:57321/v1"
experimental_bearer_token = "codex-plus-aggregate"
"#,
    )
    .unwrap();
    std::fs::write(
        home.join("auth.json"),
        r#"{"OPENAI_API_KEY":"codex-plus-aggregate"}"#,
    )
    .unwrap();

    let store = SettingsStore::new(temp.path().join("settings.json"));
    let api = pure_profile("api", "https://api.example/v1", "sk-api");
    let aggregate = RelayProfile {
        id: "agg".to_string(),
        name: "Aggregate".to_string(),
        relay_mode: RelayMode::Aggregate,
        ..RelayProfile::default()
    };
    let aggregate_config = AggregateRelayProfile {
        id: "agg".to_string(),
        name: "Aggregate".to_string(),
        strategy: AggregateRelayStrategy::Failover,
        members: vec![AggregateRelayMember {
            relay_id: "api".to_string(),
            weight: 1,
        }],
    };
    store
        .save(&BackendSettings {
            active_relay_id: "agg".to_string(),
            relay_profiles: vec![api.clone(), aggregate.clone()],
            aggregate_relay_profiles: vec![aggregate_config.clone()],
            active_aggregate_relay_id: "agg".to_string(),
            ..BackendSettings::default()
        })
        .unwrap();

    switch_relay_profile_in_home(
        &store,
        &home,
        BackendSettings {
            active_relay_id: "api".to_string(),
            relay_profiles: vec![api, aggregate],
            aggregate_relay_profiles: vec![aggregate_config],
            active_aggregate_relay_id: "agg".to_string(),
            ..BackendSettings::default()
        },
        "agg",
    )
    .unwrap();

    let stored = store.load().unwrap();
    let stored_aggregate = stored
        .relay_profiles
        .iter()
        .find(|profile| profile.id == "agg")
        .unwrap();
    assert_eq!(stored_aggregate.relay_mode, RelayMode::Aggregate);
    assert!(stored_aggregate.config_contents.is_empty());
    assert!(stored_aggregate.auth_contents.is_empty());
    assert!(stored.active_aggregate_relay_id.is_empty());

    let mut back = stored;
    back.active_relay_id = "agg".to_string();
    switch_relay_profile_in_home(&store, &home, back, "api").unwrap();

    let stored = store.load().unwrap();
    assert_eq!(stored.active_relay_id, "agg");
    assert_eq!(stored.active_aggregate_relay_id, "agg");
    assert!(stored.active_relay_uses_protocol_proxy());
    let auth: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.join("auth.json")).unwrap()).unwrap();
    assert_eq!(auth["OPENAI_API_KEY"], "codex-plus-aggregate");
}

#[test]
fn switch_to_aggregate_relay_rejects_stale_only_member() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex");
    std::fs::create_dir(&home).unwrap();
    let store = SettingsStore::new(temp.path().join("settings.json"));
    let api = pure_profile("api", "https://api.example/v1", "sk-api");
    store
        .save(&BackendSettings {
            active_relay_id: "api".to_string(),
            relay_profiles: vec![api.clone()],
            ..BackendSettings::default()
        })
        .unwrap();
    let next = BackendSettings {
        active_relay_id: "agg".to_string(),
        relay_profiles: vec![
            api,
            RelayProfile {
                id: "agg".to_string(),
                name: "Aggregate".to_string(),
                relay_mode: RelayMode::Aggregate,
                ..RelayProfile::default()
            },
        ],
        aggregate_relay_profiles: vec![AggregateRelayProfile {
            id: "agg".to_string(),
            name: "Aggregate".to_string(),
            strategy: AggregateRelayStrategy::Failover,
            members: vec![AggregateRelayMember {
                relay_id: "removed-member".to_string(),
                weight: 1,
            }],
        }],
        active_aggregate_relay_id: "stale-id".to_string(),
        ..BackendSettings::default()
    };

    let error = switch_relay_profile_in_home(&store, &home, next, "")
        .expect_err("an aggregate with no valid members must not be applied");

    assert!(error.to_string().contains("聚合供应商"));
    assert_eq!(store.load().unwrap().active_relay_id, "api");
    assert!(!home.join("config.toml").exists());
    assert!(!home.join("auth.json").exists());
}

#[test]
fn switch_relay_profile_forces_full_enhancement_mode() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex");
    std::fs::create_dir(&home).unwrap();
    let store = SettingsStore::new(temp.path().join("settings.json"));
    let relay_a = pure_profile("relay-a", "https://a.example/v1", "sk-a");
    let relay_b = pure_profile("relay-b", "https://b.example/v1", "sk-b");
    store
        .save(&BackendSettings {
            active_relay_id: "relay-a".to_string(),
            launch_mode: LaunchMode::Relay,
            relay_profiles: vec![relay_a.clone(), relay_b.clone()],
            ..BackendSettings::default()
        })
        .unwrap();

    let result = switch_relay_profile_in_home(
        &store,
        &home,
        BackendSettings {
            active_relay_id: "relay-b".to_string(),
            launch_mode: LaunchMode::Relay,
            relay_profiles: vec![relay_a, relay_b],
            ..BackendSettings::default()
        },
        "",
    )
    .unwrap();

    assert_eq!(result.settings.launch_mode, LaunchMode::Patch);
    assert_eq!(store.load().unwrap().launch_mode, LaunchMode::Patch);
}

#[test]
fn switch_to_aggregate_relay_uses_member_model_intersection_for_catalog() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex");
    std::fs::create_dir(&home).unwrap();
    let store = SettingsStore::new(temp.path().join("settings.json"));
    let relay_a = RelayProfile {
        model: "shared-model".to_string(),
        model_list: "shared-model\nonly-a".to_string(),
        model_windows: r#"{"shared-model":"1M","only-a":"1M"}"#.to_string(),
        ..pure_profile("relay-a", "https://a.example/v1", "sk-a")
    };
    let relay_b = RelayProfile {
        model: "shared-model".to_string(),
        model_list: "shared-model\nonly-b".to_string(),
        model_windows: r#"{"shared-model":"200K","only-b":"200K"}"#.to_string(),
        ..pure_profile("relay-b", "https://b.example/v1", "sk-b")
    };
    let aggregate = RelayProfile {
        id: "agg".to_string(),
        name: "Aggregate".to_string(),
        relay_mode: RelayMode::Aggregate,
        ..RelayProfile::default()
    };
    store
        .save(&BackendSettings {
            active_relay_id: "relay-a".to_string(),
            relay_profiles: vec![relay_a.clone(), relay_b.clone()],
            ..BackendSettings::default()
        })
        .unwrap();

    switch_relay_profile_in_home(
        &store,
        &home,
        BackendSettings {
            active_relay_id: "agg".to_string(),
            relay_profiles: vec![relay_a, relay_b, aggregate],
            aggregate_relay_profiles: vec![AggregateRelayProfile {
                id: "agg".to_string(),
                name: "Aggregate".to_string(),
                strategy: AggregateRelayStrategy::Failover,
                members: vec![
                    AggregateRelayMember {
                        relay_id: "relay-a".to_string(),
                        weight: 1,
                    },
                    AggregateRelayMember {
                        relay_id: "relay-b".to_string(),
                        weight: 1,
                    },
                ],
            }],
            active_aggregate_relay_id: "agg".to_string(),
            ..BackendSettings::default()
        },
        "",
    )
    .unwrap();

    let config = std::fs::read_to_string(home.join("config.toml")).unwrap();
    assert!(config.contains(r#"model = "shared-model""#));
    assert!(config.contains(r#"model_catalog_json = "model-catalogs/agg.json""#));
    let catalog = std::fs::read_to_string(home.join("model-catalogs").join("agg.json")).unwrap();
    assert!(catalog.contains(r#""slug": "shared-model""#));
    assert!(catalog.contains(r#""context_window": 200000"#));
    assert!(!catalog.contains("only-a"));
    assert!(!catalog.contains("only-b"));
}

#[test]
fn switch_returns_normalized_previous_official_profile_after_backfill() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex");
    std::fs::create_dir(&home).unwrap();
    std::fs::write(
        home.join("config.toml"),
        r#"model = "gpt-5.5"
model_reasoning_effort = "high"
model_provider = "custom"

[model_providers.custom]
name = "custom"
wire_api = "responses"
requires_openai_auth = true
base_url = "https://third-party.example/v1"

[features]
goals = true
"#,
    )
    .unwrap();
    std::fs::write(
        home.join("auth.json"),
        r#"{"OPENAI_API_KEY":"sk-third-party"}"#,
    )
    .unwrap();
    let store = SettingsStore::new(temp.path().join("settings.json"));
    let official = RelayProfile {
        id: "official".to_string(),
        name: "官方".to_string(),
        relay_mode: RelayMode::Official,
        official_mix_api_key: false,
        auth_contents: r#"{"auth_mode":"chatgpt","tokens":{"access_token":"official"}}"#
            .to_string(),
        ..RelayProfile::default()
    };
    let pure = pure_profile("api", "https://third-party.example/v1", "sk-third-party");
    let original = BackendSettings {
        active_relay_id: "official".to_string(),
        relay_profiles: vec![official.clone(), pure.clone()],
        ..BackendSettings::default()
    };
    store.save(&original).unwrap();
    let next = BackendSettings {
        active_relay_id: "api".to_string(),
        relay_profiles: vec![official, pure],
        ..BackendSettings::default()
    };

    let result = switch_relay_profile_in_home(&store, &home, next, "official").unwrap();
    let returned = result
        .settings
        .relay_profiles
        .iter()
        .find(|profile| profile.id == "official")
        .unwrap();

    assert_eq!(returned.relay_mode, RelayMode::Official);
    assert!(!returned.official_mix_api_key);
    assert!(returned.config_contents.is_empty());
    assert!(returned.api_key.is_empty());
}

#[test]
fn switch_captures_safe_app_state_before_writing_provider_config() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex");
    std::fs::create_dir(&home).unwrap();
    std::fs::write(
        home.join(".codex-global-state.json"),
        serde_json::json!({
            "electron-saved-workspace-roots": ["C:/work/app"],
            "prompt-history": ["do-not-copy"],
            "electron-persisted-atom-state": {
                "default-service-tier": "priority",
                "provider-token-cache": "do-not-copy"
            }
        })
        .to_string(),
    )
    .unwrap();
    let store = SettingsStore::new(temp.path().join("settings.json"));
    let original = BackendSettings {
        active_relay_id: "a".to_string(),
        relay_profiles: vec![
            pure_profile("a", "https://a.example/v1", "sk-a"),
            pure_profile("b", "https://b.example/v1", "sk-b"),
        ],
        ..BackendSettings::default()
    };
    store.save(&original).unwrap();
    let next = BackendSettings {
        active_relay_id: "b".to_string(),
        relay_profiles: original.relay_profiles.clone(),
        ..BackendSettings::default()
    };

    switch_relay_profile_in_home(&store, &home, next, "a").unwrap();

    let snapshot: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(
            home.join("backups_state")
                .join("app-state-sync")
                .join("latest-safe-state.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        snapshot["state"]["electron-saved-workspace-roots"],
        serde_json::json!(["C:\\work\\app"])
    );
    assert_eq!(
        snapshot["state"]["electron-persisted-atom-state"]["default-service-tier"],
        "priority"
    );
    assert!(snapshot["state"].get("prompt-history").is_none());
    assert!(
        snapshot["state"]["electron-persisted-atom-state"]
            .get("provider-token-cache")
            .is_none()
    );
}

fn pure_profile(id: &str, base_url: &str, key: &str) -> RelayProfile {
    RelayProfile {
        id: id.to_string(),
        name: id.to_uppercase(),
        relay_mode: RelayMode::PureApi,
        config_contents: format!(
            r#"model_provider = "custom"

[model_providers.custom]
name = "custom"
wire_api = "responses"
requires_openai_auth = true
base_url = "{base_url}"
"#
        ),
        auth_contents: format!(r#"{{"OPENAI_API_KEY":"{key}"}}"#),
        ..RelayProfile::default()
    }
}
