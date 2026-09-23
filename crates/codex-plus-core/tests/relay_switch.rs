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
    let original_auth = r#"{"OPENAI_API_KEY":"sk-a"}"#;
    let original_config = r#"model_provider = "custom"

[hooks.state."plugin-a@personal:hooks/hooks.json:pre_tool_use:0:0"]
trusted_hash = "live-a-hash"

[hooks.state."plugin-b@openai-bundled:hooks/hooks.json:user_prompt_submit:1:0"]
trusted_hash = "live-b-hash"

[model_providers.custom]
name = "custom"
wire_api = "responses"
requires_openai_auth = true
base_url = "https://a.example/v1"
"#;
    std::fs::write(home.join("auth.json"), original_auth).unwrap();
    std::fs::write(home.join("config.toml"), original_config).unwrap();
    let store = SettingsStore::new(temp.path().join("settings.json"));
    let original = BackendSettings {
        active_relay_id: "a".to_string(),
        relay_profiles: vec![pure_profile("a", "https://a.example/v1", "sk-a")],
        ..BackendSettings::default()
    };
    store.save(&original).unwrap();
    let persisted_original = store.load().unwrap();
    let original_settings_bytes = std::fs::read(temp.path().join("settings.json")).unwrap();
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
    assert_eq!(store.load().unwrap(), persisted_original);
    assert_eq!(
        std::fs::read(temp.path().join("settings.json")).unwrap(),
        original_settings_bytes
    );
    assert_eq!(
        std::fs::read_to_string(home.join("config.toml")).unwrap(),
        original_config
    );
    assert_eq!(
        std::fs::read_to_string(home.join("auth.json")).unwrap(),
        original_auth
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

[hooks.state."plugin-a@personal:hooks/hooks.json:pre_tool_use:0:0"]
trusted_hash = "live-a-hash"

[hooks.state."plugin-b@openai-bundled:hooks/hooks.json:user_prompt_submit:1:0"]
trusted_hash = "live-b-hash"

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
    let live: toml::Value = std::fs::read_to_string(home.join("config.toml"))
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(
        live["hooks"]["state"]["plugin-a@personal:hooks/hooks.json:pre_tool_use:0:0"]
            ["trusted_hash"]
            .as_str(),
        Some("live-a-hash")
    );
    assert_eq!(
        live["hooks"]["state"]["plugin-b@openai-bundled:hooks/hooks.json:user_prompt_submit:1:0"]
            ["trusted_hash"]
            .as_str(),
        Some("live-b-hash")
    );
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
            routes: Vec::new(),
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

/// 回归（issue #1604）：从聚合切回普通供应商时，backfill 必须把 live 里的聚合凭据
/// 存回聚合 profile，不能把认证状态写成空（否则再次切回聚合又丢凭据）。
#[test]
fn switch_from_aggregate_backfills_aggregate_auth_state() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex");
    std::fs::create_dir(&home).unwrap();
    std::fs::write(home.join("auth.json"), r#"{"OPENAI_API_KEY":"sk-old-api"}"#).unwrap();
    let store = SettingsStore::new(temp.path().join("settings.json"));
    let api = pure_profile("api", "https://api.example/v1", "sk-api");
    let aggregate = aggregate_profile("agg");
    let original = BackendSettings {
        active_relay_id: "api".to_string(),
        relay_profiles: vec![api.clone(), aggregate.clone()],
        ..BackendSettings::default()
    };
    store.save(&original).unwrap();

    // 先切到聚合，让 live auth.json 带上聚合代理凭据
    switch_relay_profile_in_home(
        &store,
        &home,
        aggregate_target_settings(api.clone(), aggregate.clone()),
        "api",
    )
    .unwrap();

    // 再从聚合切回普通供应商：这一步会 backfill 聚合 profile
    switch_relay_profile_in_home(
        &store,
        &home,
        BackendSettings {
            active_relay_id: "api".to_string(),
            relay_profiles: vec![api.clone(), aggregate.clone()],
            ..BackendSettings::default()
        },
        "agg",
    )
    .unwrap();

    let saved = store.load().unwrap();
    let backfilled = saved
        .relay_profiles
        .iter()
        .find(|profile| profile.id == "agg")
        .expect("聚合 profile 应仍在配置中");
    let value: serde_json::Value = serde_json::from_str(&backfilled.auth_contents)
        .expect("backfill 后聚合 auth_contents 必须是合法 JSON");
    assert_eq!(
        value.get("OPENAI_API_KEY").and_then(|item| item.as_str()),
        Some("codex-plus-aggregate"),
        "backfill 不能把聚合凭据写丢：{}",
        backfilled.auth_contents
    );

    // 切回普通供应商后，live auth.json 应是它自己的 key
    let live: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.join("auth.json")).unwrap()).unwrap();
    assert_eq!(
        live.get("OPENAI_API_KEY").and_then(|item| item.as_str()),
        Some("sk-api")
    );
}

/// 聚合供应商 profile：config_contents 故意留空，模拟从单供应商新建聚合后直接切换
/// 回归（issue #1604）：live auth.json 损坏，但聚合 profile 里有已验证的合法快照时，
/// 应该用快照修复 live（对应“或退回已验证的 profile 快照”），既不能中止也不能写空。
#[test]
fn switch_to_aggregate_restores_from_profile_snapshot_when_live_auth_is_corrupt() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex");
    std::fs::create_dir(&home).unwrap();
    std::fs::write(home.join("auth.json"), r#"{"OPENAI_API_KEY": "sk-broken""#).unwrap();

    let snapshot = r#"{"auth_mode":"chatgpt","tokens":{"access_token":"snapshot-token"}}"#;
    switch_api_to_aggregate_with(&home, &temp, aggregate_profile_with_auth("agg", snapshot)).unwrap();

    let auth: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.join("auth.json")).unwrap())
            .expect("应该用 profile 快照修复 live auth.json");
    assert_eq!(
        auth.get("tokens")
            .and_then(|tokens| tokens.get("access_token"))
            .and_then(|token| token.as_str()),
        Some("snapshot-token")
    );
    assert_eq!(
        auth.get("OPENAI_API_KEY").and_then(|item| item.as_str()),
        Some("codex-plus-aggregate")
    );
}

/// 回归（issue #1604）：live 合法时，profile 里损坏的快照不能阻断切换——
/// live 才是凭据的权威来源，损坏快照应被忽略。
#[test]
fn switch_to_aggregate_ignores_corrupt_profile_snapshot_when_live_is_valid() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex");
    std::fs::create_dir(&home).unwrap();
    std::fs::write(home.join("auth.json"), r#"{"OPENAI_API_KEY":"sk-old-api"}"#).unwrap();

    switch_api_to_aggregate_with(&home, &temp, aggregate_profile_with_auth("agg", "{oops")).unwrap();

    let live: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.join("auth.json")).unwrap())
            .expect("live auth.json 必须保持合法");
    assert_eq!(
        live.get("OPENAI_API_KEY").and_then(|item| item.as_str()),
        Some("codex-plus-aggregate")
    );
}

/// 回归（issue #1604）：Codex 刚刷新过的 live OAuth token 不能被 profile 里的旧快照覆盖。
#[test]
fn switch_to_aggregate_keeps_fresh_live_tokens_over_stale_profile_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex");
    std::fs::create_dir(&home).unwrap();
    std::fs::write(
        home.join("auth.json"),
        r#"{"auth_mode":"chatgpt","tokens":{"access_token":"fresh-token"}}"#,
    )
    .unwrap();

    let stale = r#"{"auth_mode":"chatgpt","tokens":{"access_token":"stale-token"}}"#;
    switch_api_to_aggregate_with(&home, &temp, aggregate_profile_with_auth("agg", stale)).unwrap();

    let live: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.join("auth.json")).unwrap()).unwrap();
    assert_eq!(
        live.get("tokens")
            .and_then(|tokens| tokens.get("access_token"))
            .and_then(|token| token.as_str()),
        Some("fresh-token"),
        "不能用 profile 旧快照覆盖 live 的新 token"
    );
    assert_eq!(
        live.get("OPENAI_API_KEY").and_then(|item| item.as_str()),
        Some("codex-plus-aggregate")
    );
}

/// 回归（issue #1604）：live 与 profile 快照都损坏时必须中止切换，不能改动 live。
#[test]
fn switch_to_aggregate_rejects_when_live_and_profile_auth_both_corrupt() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex");
    std::fs::create_dir(&home).unwrap();
    let corrupt_live = r#"{"OPENAI_API_KEY": "sk-broken""#;
    std::fs::write(home.join("auth.json"), corrupt_live).unwrap();

    let error = switch_api_to_aggregate_with(
        &home,
        &temp,
        aggregate_profile_with_auth("agg", "{oops"),
    )
    .unwrap_err();

    assert!(
        format!("{error:#}").contains("auth.json"),
        "错误信息应指向 auth.json：{error:#}"
    );
    assert_eq!(
        std::fs::read_to_string(home.join("auth.json")).unwrap(),
        corrupt_live,
        "两份来源都不可用时不能改动 live"
    );
}

/// 回归（issue #1604）：官方 OAuth 登录态在「官方 → 聚合 → 官方」往返后必须完好。
/// 聚合切换写入代理 key 时不能弄丢 OAuth token，否则中途重启就会退到登录页。
#[test]
fn official_oauth_survives_aggregate_round_trip() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex");
    std::fs::create_dir(&home).unwrap();
    std::fs::write(
        home.join("auth.json"),
        r#"{"auth_mode":"chatgpt","tokens":{"access_token":"access-token","refresh_token":"refresh-token"},"last_refresh":"2026-09-23T00:00:00Z"}"#,
    )
    .unwrap();
    let store = SettingsStore::new(temp.path().join("settings.json"));
    let official = RelayProfile {
        id: "official".to_string(),
        name: "官方".to_string(),
        relay_mode: RelayMode::Official,
        ..RelayProfile::default()
    };
    let aggregate = aggregate_profile("agg");
    let round_trip_settings = |active: &str| BackendSettings {
        active_relay_id: active.to_string(),
        relay_profiles: vec![official.clone(), aggregate.clone()],
        aggregate_relay_profiles: vec![AggregateRelayProfile {
            id: "agg".to_string(),
            name: "聚合".to_string(),
            session_provider: RelaySessionProvider::Custom,
            strategy: AggregateRelayStrategy::Failover,
            members: vec![AggregateRelayMember {
                relay_id: "official".to_string(),
                weight: 1,
            }],
            routes: Vec::new(),
        }],
        active_aggregate_relay_id: "agg".to_string(),
        ..BackendSettings::default()
    };
    store.save(&round_trip_settings("official")).unwrap();

    // 官方 → 聚合
    switch_relay_profile_in_home(&store, &home, round_trip_settings("agg"), "official").unwrap();
    let after_switch: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.join("auth.json")).unwrap()).unwrap();
    assert_eq!(
        after_switch
            .get("tokens")
            .and_then(|tokens| tokens.get("access_token"))
            .and_then(|token| token.as_str()),
        Some("access-token"),
        "切到聚合不能清掉官方 OAuth token"
    );
    assert_eq!(
        after_switch
            .get("OPENAI_API_KEY")
            .and_then(|item| item.as_str()),
        Some("codex-plus-aggregate")
    );

    // 聚合 → 官方
    switch_relay_profile_in_home(&store, &home, round_trip_settings("official"), "agg").unwrap();
    let final_auth: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.join("auth.json")).unwrap()).unwrap();
    assert_eq!(
        final_auth
            .get("tokens")
            .and_then(|tokens| tokens.get("access_token"))
            .and_then(|token| token.as_str()),
        Some("access-token"),
        "切回官方后 OAuth token 必须仍在"
    );
    assert!(
        final_auth.get("OPENAI_API_KEY").is_none(),
        "切回官方应清掉中转 key，实际：{final_auth}"
    );
}

fn aggregate_profile(id: &str) -> RelayProfile {
    RelayProfile {
        id: id.to_string(),
        name: "聚合供应商".to_string(),
        relay_mode: RelayMode::Aggregate,
        config_contents: String::new(),
        auth_contents: String::new(),
        ..RelayProfile::default()
    }
}

fn aggregate_profile_with_auth(id: &str, auth_contents: &str) -> RelayProfile {
    RelayProfile {
        auth_contents: auth_contents.to_string(),
        ..aggregate_profile(id)
    }
}

fn aggregate_target_settings(api: RelayProfile, aggregate: RelayProfile) -> BackendSettings {
    BackendSettings {
        active_relay_id: "agg".to_string(),
        relay_profiles: vec![api, aggregate],
        aggregate_relay_profiles: vec![AggregateRelayProfile {
            id: "agg".to_string(),
            name: "聚合供应商".to_string(),
            session_provider: RelaySessionProvider::Custom,
            strategy: AggregateRelayStrategy::Failover,
            members: vec![AggregateRelayMember {
                relay_id: "api".to_string(),
                weight: 1,
            }],
            routes: Vec::new(),
        }],
        active_aggregate_relay_id: "agg".to_string(),
        ..BackendSettings::default()
    }
}

fn switch_api_to_aggregate(home: &std::path::Path, temp: &tempfile::TempDir) -> anyhow::Result<()> {
    switch_api_to_aggregate_with(home, temp, aggregate_profile("agg"))
}

fn switch_api_to_aggregate_with(
    home: &std::path::Path,
    temp: &tempfile::TempDir,
    aggregate: RelayProfile,
) -> anyhow::Result<()> {
    let store = SettingsStore::new(temp.path().join("settings.json"));
    let api = pure_profile("api", "https://api.example/v1", "sk-api");
    let original = BackendSettings {
        active_relay_id: "api".to_string(),
        relay_profiles: vec![api.clone(), aggregate.clone()],
        ..BackendSettings::default()
    };
    store.save(&original)?;
    switch_relay_profile_in_home(
        &store,
        home,
        aggregate_target_settings(api, aggregate),
        "api",
    )?;
    Ok(())
}

/// 回归（issue #1604）：live auth.json 里已有 API Key 时切换到聚合，
/// 不能把 key 删光——否则 auth.json 变成空文件/空对象，Codex 认为未登录而弹登录页。
#[test]
fn switch_to_aggregate_keeps_api_key_usable() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex");
    std::fs::create_dir(&home).unwrap();
    std::fs::write(home.join("auth.json"), r#"{"OPENAI_API_KEY":"sk-old-api"}"#).unwrap();

    switch_api_to_aggregate(&home, &temp).unwrap();

    let auth = std::fs::read_to_string(home.join("auth.json")).unwrap();
    assert!(!auth.trim().is_empty(), "auth.json 不能写成空文件：{auth:?}");
    let value: serde_json::Value = serde_json::from_str(&auth).unwrap();
    assert_eq!(
        value.get("OPENAI_API_KEY").and_then(|item| item.as_str()),
        Some("codex-plus-aggregate")
    );
}

/// 回归（issue #1604）：auth.json 为空文件时切换到聚合，必须写入合法 JSON，
/// 不能留下 0 字节文件（Codex 解析会报 EOF while parsing）。
#[test]
fn switch_to_aggregate_from_empty_auth_writes_valid_json() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex");
    std::fs::create_dir(&home).unwrap();
    std::fs::write(home.join("auth.json"), "").unwrap();

    switch_api_to_aggregate(&home, &temp).unwrap();

    let auth = std::fs::read_to_string(home.join("auth.json")).unwrap();
    let value: serde_json::Value =
        serde_json::from_str(&auth).expect("聚合切换后 auth.json 必须是合法 JSON");
    assert_eq!(
        value.get("OPENAI_API_KEY").and_then(|item| item.as_str()),
        Some("codex-plus-aggregate")
    );
}

/// 回归（issue #1604）：官方 OAuth 登录态不能被聚合切换清掉（PR #1813 被拒的原因），
/// 同时要补上聚合代理所需的 API 模式凭据。
#[test]
fn switch_to_aggregate_preserves_chatgpt_oauth_tokens() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex");
    std::fs::create_dir(&home).unwrap();
    std::fs::write(
        home.join("auth.json"),
        r#"{"auth_mode":"chatgpt","tokens":{"access_token":"access-token","refresh_token":"refresh-token"},"last_refresh":"2026-09-23T00:00:00Z"}"#,
    )
    .unwrap();

    switch_api_to_aggregate(&home, &temp).unwrap();

    let auth = std::fs::read_to_string(home.join("auth.json")).unwrap();
    let value: serde_json::Value = serde_json::from_str(&auth).unwrap();
    assert_eq!(
        value
            .get("tokens")
            .and_then(|tokens| tokens.get("access_token"))
            .and_then(|token| token.as_str()),
        Some("access-token"),
        "聚合切换不能清掉官方 OAuth token"
    );
    assert_eq!(
        value.get("OPENAI_API_KEY").and_then(|item| item.as_str()),
        Some("codex-plus-aggregate")
    );
}

/// 回归（issue #1604）：live auth.json 非空但损坏时，切换必须失败并保留原文件，
/// 不能静默覆盖（否则用户的 OAuth/API Key 会被写没）。
#[test]
fn switch_to_aggregate_rejects_corrupt_auth_json() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex");
    std::fs::create_dir(&home).unwrap();
    let corrupt = r#"{"OPENAI_API_KEY": "sk-broken""#;
    std::fs::write(home.join("auth.json"), corrupt).unwrap();

    let error = switch_api_to_aggregate(&home, &temp).unwrap_err();
    assert!(
        format!("{error:#}").contains("auth.json"),
        "错误信息应指向 auth.json：{error:#}"
    );
    let live = std::fs::read_to_string(home.join("auth.json")).unwrap();
    assert_eq!(live, corrupt, "损坏的 auth.json 不能被静默覆盖");
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
