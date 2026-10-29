use codex_plus_core::provider_import::initialize_local_config_provider;
use codex_plus_core::settings::{
    AggregateRelayMember, AggregateRelayProfile, AggregateRelayRoute, AggregateRelayStrategy,
    BackendSettings, RelayMode, RelayProfile, RelayProtocol, RelaySessionProvider, SettingsStore,
};

fn api_profile(id: &str) -> RelayProfile {
    RelayProfile {
        id: id.to_string(),
        name: id.to_string(),
        relay_mode: RelayMode::PureApi,
        base_url: format!("https://{id}.example/v1"),
        api_key: format!("test-{id}"),
        config_contents: format!(
            "model = \"{id}-model\"\nmodel_provider = \"{id}\"\n[model_providers.{id}]\nname = \"{id}\"\nbase_url = \"https://{id}.example/v1\"\nwire_api = \"responses\"\nrequires_openai_auth = true\n"
        ),
        auth_contents: format!(r#"{{"OPENAI_API_KEY":"test-{id}"}}"#),
        use_common_config: false,
        ..RelayProfile::default()
    }
}

#[test]
fn imports_local_files_as_an_ordinary_selectable_provider() {
    let temp = tempfile::tempdir().unwrap();
    let store = SettingsStore::new(temp.path().join("settings.json"));
    store
        .save(&BackendSettings {
            relay_profiles_enabled: false,
            ..BackendSettings::default()
        })
        .unwrap();
    let local = api_profile("local");
    std::fs::write(temp.path().join("config.toml"), &local.config_contents).unwrap();
    std::fs::write(temp.path().join("auth.json"), &local.auth_contents).unwrap();

    let settings = initialize_local_config_provider(&store, temp.path()).unwrap();
    assert_eq!(settings.relay_profiles.len(), 1);
    let imported = &settings.relay_profiles[0];
    assert_eq!(imported.name, "系统默认");
    assert_eq!(imported.relay_mode, RelayMode::PureApi);
    assert_eq!(imported.base_url, local.base_url);
    assert_eq!(imported.api_key, local.api_key);
    assert_eq!(imported.model_list, "local-model");
    let config: toml::Value = imported.config_contents.parse().unwrap();
    assert_eq!(config["model"].as_str(), Some("local-model"));
    assert_eq!(
        config["model_providers"]["local"]["base_url"].as_str(),
        Some(local.base_url.as_str())
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&imported.auth_contents).unwrap(),
        serde_json::from_str::<serde_json::Value>(&local.auth_contents).unwrap()
    );
    assert_eq!(settings.active_relay_id, imported.id);
    assert!(settings.local_config_provider_imported);
    assert!(!imported.use_common_config);
    assert!(!settings.relay_profiles_enabled);
    assert_eq!(
        std::fs::read_to_string(temp.path().join("config.toml")).unwrap(),
        local.config_contents
    );
    assert_eq!(
        std::fs::read_to_string(temp.path().join("auth.json")).unwrap(),
        local.auth_contents
    );
}

#[test]
fn preserves_existing_selection_and_does_not_reimport_after_edit_or_delete() {
    let temp = tempfile::tempdir().unwrap();
    let store = SettingsStore::new(temp.path().join("settings.json"));
    let existing = api_profile("existing");
    store
        .save(&BackendSettings {
            relay_profiles: vec![existing.clone()],
            active_relay_id: existing.id.clone(),
            relay_profiles_enabled: true,
            ..BackendSettings::default()
        })
        .unwrap();
    let existing = store.load().unwrap().relay_profiles[0].clone();
    let mut settings = initialize_local_config_provider(&store, temp.path()).unwrap();
    assert_eq!(settings.relay_profiles.len(), 2);
    assert_eq!(settings.active_relay_id, existing.id);
    assert_eq!(settings.relay_profiles[1], existing);
    let imported_id = settings.relay_profiles[0].id.clone();
    settings.relay_profiles[0].name = "我的本地配置".to_string();
    store.save(&settings).unwrap();
    std::fs::write(temp.path().join("config.toml"), "invalid [").unwrap();
    let mut reloaded = initialize_local_config_provider(&store, temp.path()).unwrap();
    assert_eq!(reloaded.relay_profiles[0].name, "我的本地配置");
    reloaded
        .relay_profiles
        .retain(|profile| profile.id != imported_id);
    store.save(&reloaded).unwrap();
    assert_eq!(
        initialize_local_config_provider(&store, temp.path())
            .unwrap()
            .relay_profiles,
        vec![existing]
    );
}

#[test]
fn missing_local_files_create_an_editable_official_provider() {
    let temp = tempfile::tempdir().unwrap();
    let store = SettingsStore::new(temp.path().join("settings.json"));
    let settings = initialize_local_config_provider(&store, temp.path()).unwrap();
    assert_eq!(settings.relay_profiles.len(), 1);
    assert_eq!(settings.relay_profiles[0].relay_mode, RelayMode::Official);
    assert!(!settings.relay_profiles[0].official_mix_api_key);
    assert!(!temp.path().join("config.toml").exists());
    assert!(!temp.path().join("auth.json").exists());
}

#[test]
fn invalid_local_config_does_not_mark_import_complete_or_change_settings() {
    let temp = tempfile::tempdir().unwrap();
    let store = SettingsStore::new(temp.path().join("settings.json"));
    store.save(&BackendSettings::default()).unwrap();
    let before = std::fs::read(temp.path().join("settings.json")).unwrap();
    std::fs::write(temp.path().join("config.toml"), "invalid [").unwrap();
    assert!(initialize_local_config_provider(&store, temp.path()).is_err());
    assert_eq!(
        std::fs::read(temp.path().join("settings.json")).unwrap(),
        before
    );
    std::fs::write(temp.path().join("config.toml"), "model = \"gpt-5.5\"\n").unwrap();
    assert!(
        initialize_local_config_provider(&store, temp.path())
            .unwrap()
            .local_config_provider_imported
    );
}

#[test]
fn keeps_a_previously_imported_default_and_legacy_credentials() {
    let temp = tempfile::tempdir().unwrap();
    let store = SettingsStore::new(temp.path().join("settings.json"));
    store
        .save(&BackendSettings {
            relay_base_url: "https://legacy.example/v1".to_string(),
            relay_api_key: "legacy-key".to_string(),
            ..BackendSettings::default()
        })
        .unwrap();
    let settings = initialize_local_config_provider(&store, temp.path()).unwrap();
    assert_eq!(settings.active_relay_id, "default");
    assert_eq!(settings.active_relay_profile().api_key, "legacy-key");
    assert_eq!(settings.relay_profiles.len(), 2);

    let mut existing_default = api_profile("imported");
    existing_default.name = "系统默认配置".to_string();
    store
        .save(&BackendSettings {
            settings_revision: store.revision().unwrap(),
            relay_profiles: vec![existing_default.clone()],
            active_relay_id: existing_default.id.clone(),
            ..BackendSettings::default()
        })
        .unwrap();
    let saved = store.load().unwrap().relay_profiles;
    assert_eq!(
        initialize_local_config_provider(&store, temp.path())
            .unwrap()
            .relay_profiles,
        saved
    );
}

#[test]
fn local_proxy_import_keeps_real_upstream_and_protocol() {
    let temp = tempfile::tempdir().unwrap();
    let store = SettingsStore::new(temp.path().join("settings.json"));
    let mut existing = api_profile("custom");
    existing.protocol = RelayProtocol::ChatCompletions;
    existing.upstream_base_url = existing.base_url.clone();
    store
        .save(&BackendSettings {
            relay_profiles: vec![existing.clone()],
            active_relay_id: existing.id.clone(),
            relay_profiles_enabled: true,
            ..BackendSettings::default()
        })
        .unwrap();
    let proxy = codex_plus_core::protocol_proxy::local_responses_proxy_base_url(
        codex_plus_core::protocol_proxy::DEFAULT_PROTOCOL_PROXY_PORT,
    );
    std::fs::write(
        temp.path().join("config.toml"),
        existing.config_contents.replace(&existing.base_url, &proxy),
    )
    .unwrap();
    std::fs::write(temp.path().join("auth.json"), &existing.auth_contents).unwrap();
    let settings = initialize_local_config_provider(&store, temp.path()).unwrap();
    assert_eq!(
        settings.relay_profiles[0].protocol,
        RelayProtocol::ChatCompletions
    );
    assert_eq!(
        settings.relay_profiles[0].upstream_base_url,
        existing.base_url
    );
    assert_eq!(settings.relay_profiles[0].api_key, existing.api_key);
}

#[test]
fn imported_aggregate_preserves_members_and_routes_after_switching_away_and_back() {
    let temp = tempfile::tempdir().unwrap();
    let store = SettingsStore::new(temp.path().join("settings.json"));
    let aggregate = AggregateRelayProfile {
        id: "aggregate".into(),
        name: "聚合".into(),
        session_provider: RelaySessionProvider::Custom,
        model_aggregation_enabled: false,
        strategy: AggregateRelayStrategy::WeightedRoundRobin,
        members: vec![
            AggregateRelayMember {
                relay_id: "first".into(),
                weight: 2,
            },
            AggregateRelayMember {
                relay_id: "second".into(),
                weight: 3,
            },
        ],
        routes: vec![AggregateRelayRoute {
            pattern: "second-*".into(),
            relay_id: "second".into(),
            priority: 5,
        }],
        default_relay_id: "first".into(),
        sync_default_member_login: true,
        auto_review_relay_id: String::new(),
    };
    let settings = BackendSettings {
        relay_profiles_enabled: true,
        active_relay_id: aggregate.id.clone(),
        active_aggregate_relay_id: aggregate.id.clone(),
        relay_profiles: vec![
            RelayProfile {
                id: aggregate.id.clone(),
                name: aggregate.name.clone(),
                relay_mode: RelayMode::Aggregate,
                use_common_config: false,
                ..RelayProfile::default()
            },
            api_profile("first"),
            api_profile("second"),
        ],
        aggregate_relay_profiles: vec![aggregate.clone()],
        ..BackendSettings::default()
    };
    codex_plus_core::relay_switch::switch_relay_profile_in_home(&store, temp.path(), settings, "")
        .unwrap();
    assert!(codex_plus_core::relay_config::responses_proxy_configured_in_home(temp.path()));
    let mut settings = initialize_local_config_provider(&store, temp.path()).unwrap();
    let imported = settings.relay_profiles[0].clone();
    assert_ne!(imported.id, aggregate.id);
    assert_eq!(imported.relay_mode, RelayMode::Aggregate);
    assert_eq!(settings.active_relay_id, aggregate.id);
    assert_eq!(
        settings.active_aggregate_relay_profile(),
        Some(aggregate.clone())
    );
    let mut expected = aggregate.clone();
    expected.id = imported.id.clone();
    expected.name = imported.name.clone();
    assert_eq!(
        settings
            .aggregate_relay_profiles
            .iter()
            .find(|item| item.id == imported.id),
        Some(&expected)
    );

    settings.active_relay_id = "first".into();
    settings.active_aggregate_relay_id.clear();
    let switched = codex_plus_core::relay_switch::switch_relay_profile_in_home(
        &store,
        temp.path(),
        settings,
        &aggregate.id,
    )
    .unwrap();
    let mut settings = switched.settings;
    settings.active_relay_id = imported.id.clone();
    settings.active_aggregate_relay_id = imported.id.clone();
    let restored = codex_plus_core::relay_switch::switch_relay_profile_in_home(
        &store,
        temp.path(),
        settings,
        "first",
    )
    .unwrap();
    assert_eq!(
        restored.settings.active_aggregate_relay_profile(),
        Some(expected)
    );
    assert!(
        restored
            .settings
            .active_relay_transport_uses_protocol_proxy()
    );
    assert!(codex_plus_core::relay_config::responses_proxy_configured_in_home(temp.path()));
    let selected = codex_plus_core::relay_rotation::select_relay_for_request(
        &restored.settings,
        codex_plus_core::relay_rotation::RotationContext {
            model: Some("second-model".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(selected.id, "second");
    let reloaded = initialize_local_config_provider(&store, temp.path()).unwrap();
    assert_eq!(
        reloaded.aggregate_relay_profiles,
        restored.settings.aggregate_relay_profiles
    );
}

#[test]
fn default_provider_uses_the_same_switch_path_as_other_providers() {
    let temp = tempfile::tempdir().unwrap();
    let store = SettingsStore::new(temp.path().join("settings.json"));
    let local = api_profile("local");
    std::fs::write(temp.path().join("config.toml"), &local.config_contents).unwrap();
    std::fs::write(temp.path().join("auth.json"), &local.auth_contents).unwrap();
    let mut settings = initialize_local_config_provider(&store, temp.path()).unwrap();
    let imported_id = settings.active_relay_id.clone();
    settings.relay_profiles_enabled = true;
    settings.relay_profiles.push(api_profile("other"));
    settings.active_relay_id = "other".to_string();
    let switched = codex_plus_core::relay_switch::switch_relay_profile_in_home(
        &store,
        temp.path(),
        settings,
        &imported_id,
    )
    .unwrap();
    assert!(
        std::fs::read_to_string(temp.path().join("config.toml"))
            .unwrap()
            .contains("other.example")
    );
    let mut settings = switched.settings;
    settings.active_relay_id = imported_id.clone();
    let restored = codex_plus_core::relay_switch::switch_relay_profile_in_home(
        &store,
        temp.path(),
        settings,
        "other",
    )
    .unwrap();
    assert_eq!(restored.settings.active_relay_id, imported_id);
    assert!(restored.settings.relay_profiles_enabled);
    assert!(
        std::fs::read_to_string(temp.path().join("config.toml"))
            .unwrap()
            .contains("local.example")
    );
    assert!(
        std::fs::read_to_string(temp.path().join("auth.json"))
            .unwrap()
            .contains("test-local")
    );
}

#[test]
fn switching_back_to_imported_official_provider_restores_its_local_config() {
    let temp = tempfile::tempdir().unwrap();
    let store = SettingsStore::new(temp.path().join("settings.json"));
    std::fs::write(
        temp.path().join("config.toml"),
        "model = \"gpt-5.5\"\napproval_policy = \"never\"\n",
    )
    .unwrap();
    std::fs::write(
        temp.path().join("auth.json"),
        r#"{"auth_mode":"chatgpt","tokens":{"access_token":"local-token"}}"#,
    )
    .unwrap();
    let mut settings = initialize_local_config_provider(&store, temp.path()).unwrap();
    let imported_id = settings.active_relay_id.clone();
    settings.relay_profiles_enabled = true;
    settings.relay_profiles.push(api_profile("other"));
    settings.active_relay_id = "other".to_string();
    let switched = codex_plus_core::relay_switch::switch_relay_profile_in_home(
        &store,
        temp.path(),
        settings,
        &imported_id,
    )
    .unwrap();
    let mut settings = switched.settings;
    settings.active_relay_id = imported_id;
    codex_plus_core::relay_switch::switch_relay_profile_in_home(
        &store,
        temp.path(),
        settings,
        "other",
    )
    .unwrap();
    let config = std::fs::read_to_string(temp.path().join("config.toml")).unwrap();
    assert!(config.contains("gpt-5.5"));
    assert!(config.contains("approval_policy = \"never\""));
    assert!(!config.contains("other-model"));
    assert!(
        std::fs::read_to_string(temp.path().join("auth.json"))
            .unwrap()
            .contains("local-token")
    );
}
