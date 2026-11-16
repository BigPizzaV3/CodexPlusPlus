use codex_plus_core::relay_config::apply_relay_profile_to_home_with_switch_rules;
use codex_plus_core::settings::{BackendSettings, RelayMode, RelayProfile, RelayProtocol, SettingsStore};
use codex_plus_core::session_tuning::{read, write};
use serde_json::{Value, json};

// 「上下文与压缩」后端：把压缩 / 精简系统提示等键同步进通用配置。只碰临时目录。

fn profile(id: &str, model: &str, extra: &str) -> RelayProfile {
    RelayProfile {
        id: id.to_string(),
        name: id.to_string(),
        relay_mode: RelayMode::PureApi,
        protocol: RelayProtocol::Responses,
        config_contents: format!(
            "model = \"{model}\"\nmodel_provider = \"custom\"\n{extra}\n[model_providers.custom]\nname = \"custom\"\nwire_api = \"responses\"\nrequires_openai_auth = true\nbase_url = \"https://responses.example.test/v1\"\n"
        ),
        auth_contents: r#"{"OPENAI_API_KEY":"sk-test-redacted"}"#.to_string(),
        ..RelayProfile::default()
    }
}

fn store_with(common: &str, profiles: Vec<RelayProfile>) -> (tempfile::TempDir, SettingsStore) {
    let temp = tempfile::tempdir().unwrap();
    let store = SettingsStore::new(temp.path().join("settings.json"));
    let mut settings = BackendSettings::default();
    settings.relay_profiles_enabled = true;
    settings.active_relay_id = profiles[0].id.clone();
    settings.relay_profiles = profiles;
    settings.relay_common_config_contents = common.to_string();
    store.save(&settings).unwrap();
    (temp, store)
}

fn edits(items: Value) -> Value {
    json!({ "edits": items })
}

#[test]
fn writes_and_removes_whitelisted_keys_in_common_config() {
    let (_temp, store) = store_with("model_reasoning_effort = \"high\"\n", vec![profile("a", "glm-5.2", "")]);
    let state = write(
        &store,
        &edits(json!([
            { "key": "compact_prompt", "value": "保留决定与待办" },
            { "key": "model_post_turn_compact_threshold_percent", "value": 80 },
            { "key": "include_environment_context", "value": false },
        ])),
    )
    .unwrap();
    assert_eq!(state["values"]["compact_prompt"], "保留决定与待办");
    assert_eq!(state["values"]["model_post_turn_compact_threshold_percent"], 80);
    assert_eq!(state["values"]["include_environment_context"], false);
    assert_eq!(state["values"]["tool_output_token_limit"], Value::Null);
    assert_eq!(state["commonConfigApplies"], true);
    let common = store.load().unwrap().relay_common_config_contents;
    assert!(common.contains("model_reasoning_effort = \"high\""), "原有通用配置不动：{common}");
    assert!(common.contains("model_post_turn_compact_threshold_percent = 80"), "{common}");

    // null 与空文本都是删除。
    let state = write(
        &store,
        &edits(json!([
            { "key": "compact_prompt", "value": "  " },
            { "key": "include_environment_context", "value": null },
        ])),
    )
    .unwrap();
    assert_eq!(state["values"]["compact_prompt"], Value::Null);
    assert_eq!(state["values"]["include_environment_context"], Value::Null);
    let common = store.load().unwrap().relay_common_config_contents;
    assert!(!common.contains("compact_prompt") && !common.contains("include_environment_context"), "{common}");
    assert_eq!(read(&store).unwrap()["values"]["model_post_turn_compact_threshold_percent"], 80);
}

#[test]
fn rejects_unknown_keys_and_out_of_range_values_without_partial_writes() {
    let (_temp, store) = store_with("", vec![profile("a", "glm-5.2", "")]);
    for bad in [
        json!([{ "key": "compact_prompt", "value": "ok" }, { "key": "model_provider", "value": "x" }]),
        json!([{ "key": "model_post_turn_compact_threshold_percent", "value": 150 }]),
        json!([{ "key": "model_post_turn_compact_threshold_percent", "value": 0 }]),
        json!([{ "key": "tool_output_token_limit", "value": "200" }]),
        json!([{ "key": "include_apps_instructions", "value": "no" }]),
    ] {
        assert!(write(&store, &edits(bad.clone())).is_err(), "{bad}");
    }
    assert!(store.load().unwrap().relay_common_config_contents.trim().is_empty(), "拒绝时一个键都不写");
}

#[test]
fn reports_when_active_profile_skips_common_config() {
    let mut own = profile("own", "glm-5.2", "");
    own.use_common_config = false;
    let (_temp, store) = store_with("", vec![own]);
    let state = read(&store).unwrap();
    assert_eq!(state["commonConfigApplies"], false);
    assert_eq!(state["activeProfileName"], "own");
}

/// 写进通用配置的键在切换供应商后仍在；改掉 / 删掉时，各快照里和旧值相同的副本一并清掉，
/// 不同的（给单个供应商专门设的）保留。
#[test]
fn edits_survive_switching_and_clear_stale_snapshot_copies() {
    let (_temp, store) = store_with(
        "",
        vec![
            profile("a", "glm-5.2", ""),
            profile("b", "kimi-k3", "tool_output_token_limit = 4000\n"),
            profile("c", "qwen-4", "tool_output_token_limit = 9000\n"),
        ],
    );
    write(&store, &edits(json!([{ "key": "tool_output_token_limit", "value": 4000 }]))).unwrap();
    let settings = store.load().unwrap();
    let home = tempfile::tempdir().unwrap();
    for id in ["a", "b", "c"] {
        let target = settings.relay_profiles.iter().find(|p| p.id == id).unwrap();
        apply_relay_profile_to_home_with_switch_rules(home.path(), target, &settings.relay_common_config_contents)
            .unwrap();
        let config = std::fs::read_to_string(home.path().join("config.toml")).unwrap();
        assert!(config.contains("tool_output_token_limit = 4000"), "{id}: {config}");
    }

    write(&store, &edits(json!([{ "key": "tool_output_token_limit", "value": null }]))).unwrap();
    let settings = store.load().unwrap();
    let snapshot = |id: &str| settings.relay_profiles.iter().find(|p| p.id == id).unwrap().config_contents.clone();
    assert!(!snapshot("b").contains("tool_output_token_limit"), "和旧通用值相同的副本清掉：{}", snapshot("b"));
    assert!(snapshot("c").contains("tool_output_token_limit = 9000"), "单独设的保留：{}", snapshot("c"));
    let b = settings.relay_profiles.iter().find(|p| p.id == "b").unwrap();
    apply_relay_profile_to_home_with_switch_rules(home.path(), b, &settings.relay_common_config_contents).unwrap();
    let config = std::fs::read_to_string(home.path().join("config.toml")).unwrap();
    assert!(!config.contains("tool_output_token_limit"), "{config}");
}
