use codex_plus_core::relay_switch::switch_relay_profile_in_home;
use codex_plus_core::settings::{
    AggregateRelayMember, AggregateRelayProfile, AggregateRelayStrategy, BackendSettings,
    BedrockAuthMode, BedrockConfig, LaunchMode, RelayMode, RelayProfile, RelaySessionProvider,
    SettingsStore, default_bedrock_iam_key_validity_days,
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

#[test]
fn switch_to_bedrock_bearer_token_writes_expected_config() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex");
    std::fs::create_dir(&home).unwrap();
    // 用一份合法的初始 config 作种子，方便 backfill 有内容可读
    std::fs::write(
        home.join("config.toml"),
        "model_provider = \"custom\"\n\n[model_providers.custom]\nname = \"custom\"\nwire_api = \"responses\"\nrequires_openai_auth = true\nbase_url = \"https://example.com/v1\"\n",
    )
    .unwrap();
    std::fs::write(home.join("auth.json"), r#"{"OPENAI_API_KEY":"sk-init"}"#).unwrap();

    let store = SettingsStore::new(temp.path().join("settings.json"));
    let seed_profile = pure_profile("seed", "https://example.com/v1", "sk-init");
    let bedrock_profile = RelayProfile {
        id: "bedrock".to_string(),
        name: "Bedrock (Bearer)".to_string(),
        relay_mode: RelayMode::PureApi,
        model: "openai.gpt-oss-120b-1:0".to_string(),
        api_key: "brk-test-key-12345".to_string(),
        bedrock: Some(BedrockConfig {
            auth_mode: BedrockAuthMode::BearerToken,
            provider_id: "my-bedrock".to_string(),
            region: "us-east-2".to_string(),
            aws_profile: String::new(),
            iam_user_name: String::new(),
            iam_key_validity_days: default_bedrock_iam_key_validity_days(),
        }),
        ..RelayProfile::default()
    };
    let original = BackendSettings {
        active_relay_id: "seed".to_string(),
        relay_profiles: vec![seed_profile.clone(), bedrock_profile.clone()],
        ..BackendSettings::default()
    };
    store.save(&original).unwrap();

    let next = BackendSettings {
        active_relay_id: "bedrock".to_string(),
        relay_profiles: vec![seed_profile, bedrock_profile],
        ..BackendSettings::default()
    };
    switch_relay_profile_in_home(&store, &home, next, "seed").unwrap();

    let live = std::fs::read_to_string(home.join("config.toml")).unwrap();
    // Property 1 关键片段
    assert!(
        live.contains("model_provider = \"my-bedrock\""),
        "expected model_provider = \"my-bedrock\", got:\n{live}"
    );
    assert!(
        live.contains("base_url = \"https://bedrock-mantle.us-east-2.api.aws/openai/v1\""),
        "expected bedrock-mantle base_url, got:\n{live}"
    );
    assert!(
        live.contains("experimental_bearer_token = \"brk-test-key-12345\""),
        "expected experimental_bearer_token, got:\n{live}"
    );
    // `requires_openai_auth` 表示"走 OpenAI 鉴权语义（ChatGPT 登录态）"，
    // 上游仅在混合 / 官方模式下写入。Bedrock Bearer Token 自带 Bedrock API Key，
    // 属纯 API 形态，因此切换后的 live config 不应含该字段——种子 config 里
    // 那份 `requires_openai_auth = true` 也必须被整表替换清掉。
    assert!(
        !live.contains("requires_openai_auth"),
        "requires_openai_auth should not be written for Bedrock, got:\n{live}"
    );
    assert!(
        live.contains("web_search = \"disabled\""),
        "expected web_search = \"disabled\", got:\n{live}"
    );
}

#[test]
fn switch_to_bedrock_aws_profile_writes_expected_config() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex");
    std::fs::create_dir(&home).unwrap();
    // 用一份合法的初始 config 作种子
    std::fs::write(
        home.join("config.toml"),
        "model_provider = \"custom\"\n\n[model_providers.custom]\nname = \"custom\"\nwire_api = \"responses\"\nrequires_openai_auth = true\nbase_url = \"https://example.com/v1\"\n",
    )
    .unwrap();
    std::fs::write(home.join("auth.json"), r#"{"OPENAI_API_KEY":"sk-init"}"#).unwrap();

    let store = SettingsStore::new(temp.path().join("settings.json"));
    let seed_profile = pure_profile("seed", "https://example.com/v1", "sk-init");
    let bedrock_profile = RelayProfile {
        id: "bedrock-aws".to_string(),
        name: "Bedrock (AWS Profile)".to_string(),
        relay_mode: RelayMode::PureApi,
        model: "openai.gpt-oss-120b-1:0".to_string(),
        bedrock: Some(BedrockConfig {
            auth_mode: BedrockAuthMode::AwsProfile,
            provider_id: String::new(),
            region: "us-west-2".to_string(),
            aws_profile: "my-dev".to_string(),
            iam_user_name: String::new(),
            iam_key_validity_days: default_bedrock_iam_key_validity_days(),
        }),
        ..RelayProfile::default()
    };
    let original = BackendSettings {
        active_relay_id: "seed".to_string(),
        relay_profiles: vec![seed_profile.clone(), bedrock_profile.clone()],
        ..BackendSettings::default()
    };
    store.save(&original).unwrap();

    let next = BackendSettings {
        active_relay_id: "bedrock-aws".to_string(),
        relay_profiles: vec![seed_profile, bedrock_profile],
        ..BackendSettings::default()
    };
    switch_relay_profile_in_home(&store, &home, next, "seed").unwrap();

    let live = std::fs::read_to_string(home.join("config.toml")).unwrap();

    // Property 2 关键片段
    assert!(
        live.contains("model_provider = \"amazon-bedrock\""),
        "expected model_provider = \"amazon-bedrock\", got:\n{live}"
    );
    assert!(
        live.contains("[model_providers.amazon-bedrock.aws]"),
        "expected [model_providers.amazon-bedrock.aws] table header, got:\n{live}"
    );
    assert!(
        live.contains("region = \"us-west-2\""),
        "expected region = \"us-west-2\", got:\n{live}"
    );
    assert!(
        live.contains("profile = \"my-dev\""),
        "expected profile = \"my-dev\", got:\n{live}"
    );

    // Requirement 4.2：顶层 model_provider 所在行号 < 第一个 [...] 表头所在行号
    let lines: Vec<&str> = live.lines().collect();
    let model_provider_line = lines
        .iter()
        .position(|line| line.trim_start().starts_with("model_provider ="))
        .expect("expected a model_provider line in generated config");
    let first_table_line = lines
        .iter()
        .position(|line| {
            let trimmed = line.trim_start();
            trimmed.starts_with('[') && trimmed.contains(']')
        })
        .expect("expected at least one [...] table header in generated config");
    assert!(
        model_provider_line < first_table_line,
        "model_provider (line {model_provider_line}) must precede first table (line {first_table_line}); got:\n{live}"
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
