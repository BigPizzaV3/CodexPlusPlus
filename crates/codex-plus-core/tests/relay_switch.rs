use codex_plus_core::relay_config::sync_official_auth_from_live;
use codex_plus_core::relay_switch::switch_relay_profile_in_home;
use codex_plus_core::settings::{
    AggregateRelayMember, AggregateRelayProfile, AggregateRelayStrategy, BackendSettings,
    LaunchMode, RelayMode, RelayProfile, RelaySessionProvider, SettingsStore,
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
fn switch_syncs_live_chatgpt_auth_to_all_official_profiles() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex");
    std::fs::create_dir(&home).unwrap();
    std::fs::write(home.join("config.toml"), "").unwrap();
    let live_auth = r#"{
  "auth_mode": "chatgpt",
  "OPENAI_API_KEY": "must-not-be-copied",
  "tokens": {
    "access_token": "new-access",
    "id_token": "x.eyJlbWFpbCI6InVzZXJAZXhhbXBsZS5jb20iLCJodHRwczovL2FwaS5vcGVuYWkuY29tL2F1dGgiOnsiY2hhdGdwdF9hY2NvdW50X2lkIjoiYWNjb3VudC1hIn19.y",
    "account_id": "account-a",
    "refresh_token": "new-refresh"
  }
}"#;
    std::fs::write(home.join("auth.json"), live_auth).unwrap();
    let expected_auth = serde_json::json!({
        "auth_mode": "chatgpt",
        "tokens": {
            "access_token": "new-access",
            "id_token": "x.eyJlbWFpbCI6InVzZXJAZXhhbXBsZS5jb20iLCJodHRwczovL2FwaS5vcGVuYWkuY29tL2F1dGgiOnsiY2hhdGdwdF9hY2NvdW50X2lkIjoiYWNjb3VudC1hIn19.y",
            "account_id": "account-a",
            "refresh_token": "new-refresh"
        }
    });

    let store = SettingsStore::new(temp.path().join("settings.json"));
    let official_a = official_profile(
        "a",
        r#"{"auth_mode":"chatgpt","tokens":{"access_token":"old-a","account_id":"account-a"}}"#,
    );
    // Legacy snapshots without account_id still match by email.
    let official_b = official_profile(
        "b",
        r#"{"auth_mode":"chatgpt","tokens":{"access_token":"old-b","id_token":"x.eyJlbWFpbCI6InVzZXJAZXhhbXBsZS5jb20ifQ.y"}}"#,
    );
    let official_mix = RelayProfile {
        id: "mixed".to_string(),
        name: "Mixed".to_string(),
        relay_mode: RelayMode::Official,
        official_mix_api_key: true,
        config_contents: r#"model_provider = "custom"

[model_providers.custom]
name = "custom"
wire_api = "responses"
requires_openai_auth = true
base_url = "https://mixed.example/v1"
"#
        .to_string(),
        auth_contents: r#"{"auth_mode":"chatgpt","tokens":{"access_token":"old-mixed","id_token":"x.eyJlbWFpbCI6InVzZXJAZXhhbXBsZS5jb20iLCJodHRwczovL2FwaS5vcGVuYWkuY29tL2F1dGgiOnsiY2hhdGdwdF9hY2NvdW50X2lkIjoiYWNjb3VudC1hIn19.y"}}"#
            .to_string(),
        ..RelayProfile::default()
    };
    let other_account = official_profile(
        "other",
        r#"{"auth_mode":"chatgpt","tokens":{"account_id":"account-b","id_token":"x.eyJlbWFpbCI6InVzZXJAZXhhbXBsZS5jb20iLCJodHRwczovL2FwaS5vcGVuYWkuY29tL2F1dGgiOnsiY2hhdGdwdF9hY2NvdW50X2lkIjoiYWNjb3VudC1iIn19.y"}}"#,
    );
    let pure = pure_profile("api", "https://api.example/v1", "sk-api");
    let original = BackendSettings {
        active_relay_id: "a".to_string(),
        relay_profiles: vec![
            official_a.clone(),
            official_b.clone(),
            official_mix.clone(),
            other_account.clone(),
            pure.clone(),
        ],
        ..BackendSettings::default()
    };
    store.save(&original).unwrap();
    let next = BackendSettings {
        active_relay_id: "b".to_string(),
        relay_profiles: vec![
            official_a,
            official_b,
            official_mix,
            other_account.clone(),
            pure.clone(),
        ],
        ..BackendSettings::default()
    };

    switch_relay_profile_in_home(&store, &home, next, "a").unwrap();

    let stored = store.load().unwrap();
    for profile in stored
        .relay_profiles
        .iter()
        .filter(|profile| matches!(profile.id.as_str(), "a" | "b" | "mixed"))
    {
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&profile.auth_contents).unwrap(),
            expected_auth,
            "official profile {} should carry live auth",
            profile.id
        );
    }
    let stored_pure = stored
        .relay_profiles
        .iter()
        .find(|profile| profile.id == "api")
        .unwrap();
    assert_eq!(stored_pure.auth_contents, pure.auth_contents);
    let stored_other = stored
        .relay_profiles
        .iter()
        .find(|profile| profile.id == "other")
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&stored_other.auth_contents).unwrap(),
        serde_json::from_str::<serde_json::Value>(&other_account.auth_contents).unwrap()
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(
            &std::fs::read_to_string(home.join("auth.json")).unwrap()
        )
        .unwrap(),
        expected_auth
    );
}

#[test]
fn switch_to_different_official_account_keeps_target_auth() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex");
    std::fs::create_dir(&home).unwrap();
    std::fs::write(home.join("config.toml"), "").unwrap();
    std::fs::write(
        home.join("auth.json"),
        r#"{"auth_mode":"chatgpt","tokens":{"account_id":"account-a","access_token":"live-a"}}"#,
    )
    .unwrap();
    let account_a = official_profile(
        "a",
        r#"{"auth_mode":"chatgpt","tokens":{"account_id":"account-a","access_token":"old-a"}}"#,
    );
    let account_b = official_profile(
        "b",
        r#"{"auth_mode":"chatgpt","tokens":{"account_id":"account-b","access_token":"stored-b"}}"#,
    );
    let store = SettingsStore::new(temp.path().join("settings.json"));
    store
        .save(&BackendSettings {
            active_relay_id: "a".to_string(),
            relay_profiles: vec![account_a.clone(), account_b.clone()],
            ..BackendSettings::default()
        })
        .unwrap();

    switch_relay_profile_in_home(
        &store,
        &home,
        BackendSettings {
            active_relay_id: "b".to_string(),
            relay_profiles: vec![account_a, account_b],
            ..BackendSettings::default()
        },
        "a",
    )
    .unwrap();

    let live: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.join("auth.json")).unwrap()).unwrap();
    assert_eq!(live["tokens"]["account_id"], "account-b");
    assert_eq!(live["tokens"]["access_token"], "stored-b");
}

#[test]
fn switch_preserves_official_profiles_when_account_identity_is_unknown() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex");
    std::fs::create_dir(&home).unwrap();
    std::fs::write(home.join("config.toml"), "").unwrap();
    std::fs::write(
        home.join("auth.json"),
        r#"{"auth_mode":"chatgpt","tokens":{"refresh_token":"live-refresh"}}"#,
    )
    .unwrap();

    let first = official_profile(
        "first",
        r#"{"auth_mode":"chatgpt","tokens":{"refresh_token":"first-refresh"}}"#,
    );
    let second = official_profile(
        "second",
        r#"{"auth_mode":"chatgpt","tokens":{"refresh_token":"second-refresh"}}"#,
    );
    let store = SettingsStore::new(temp.path().join("settings.json"));
    let original = BackendSettings {
        active_relay_id: "first".to_string(),
        relay_profiles: vec![first.clone(), second.clone()],
        ..BackendSettings::default()
    };
    store.save(&original).unwrap();
    let next = BackendSettings {
        active_relay_id: "second".to_string(),
        relay_profiles: vec![first.clone(), second.clone()],
        ..BackendSettings::default()
    };

    switch_relay_profile_in_home(&store, &home, next, "").unwrap();

    let stored = store.load().unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&stored.relay_profiles[0].auth_contents).unwrap(),
        serde_json::from_str::<serde_json::Value>(&first.auth_contents).unwrap()
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&stored.relay_profiles[1].auth_contents).unwrap(),
        serde_json::from_str::<serde_json::Value>(&second.auth_contents).unwrap()
    );
}

#[test]
fn auth_sync_ignores_non_chatgpt_profile_auth() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("auth.json"),
        r#"{"auth_mode":"chatgpt","tokens":{"account_id":"account-a","access_token":"live"}}"#,
    )
    .unwrap();
    let original_auth =
        r#"{"auth_mode":"apikey","tokens":{"account_id":"account-a","access_token":"keep-me"}}"#;
    let mut profiles = vec![official_profile("api-auth", original_auth)];

    let updated = sync_official_auth_from_live(temp.path(), &mut profiles).unwrap();

    assert_eq!(updated, 0);
    assert_eq!(profiles[0].auth_contents, original_auth);
}

#[test]
fn auth_sync_requires_live_account_id_for_bound_profile() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("auth.json"),
        r#"{"auth_mode":"chatgpt","tokens":{"id_token":"x.eyJlbWFpbCI6InVzZXJAZXhhbXBsZS5jb20ifQ.y","access_token":"live"}}"#,
    )
    .unwrap();
    let original_auth = r#"{"auth_mode":"chatgpt","tokens":{"account_id":"bound-account","id_token":"x.eyJlbWFpbCI6InVzZXJAZXhhbXBsZS5jb20ifQ.y","access_token":"stored"}}"#;
    let mut profiles = vec![official_profile("bound", original_auth)];

    let updated = sync_official_auth_from_live(temp.path(), &mut profiles).unwrap();

    assert_eq!(updated, 0);
    assert_eq!(profiles[0].auth_contents, original_auth);
}

#[test]
fn switch_to_aggregate_relay_allows_empty_config_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex");
    std::fs::create_dir(&home).unwrap();
    let store = SettingsStore::new(temp.path().join("settings.json"));
    let api = pure_profile("api", "https://api.example/v1", "sk-api");
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
            session_provider: RelaySessionProvider::Custom,
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

    assert!(result.configured);
    assert_eq!(store.load().unwrap().active_relay_id, "agg");
    assert!(live.contains(r#"base_url = "http://127.0.0.1:57321/v1""#));
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
        hide_official_usage_alert: false,
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

fn official_profile(id: &str, auth_contents: &str) -> RelayProfile {
    RelayProfile {
        id: id.to_string(),
        name: id.to_uppercase(),
        relay_mode: RelayMode::Official,
        auth_contents: auth_contents.to_string(),
        ..RelayProfile::default()
    }
}
