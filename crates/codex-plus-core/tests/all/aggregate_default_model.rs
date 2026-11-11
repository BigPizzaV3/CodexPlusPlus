//! 聚合切换时 config.toml 没有 `model` 的补值（#2958）。
//!
//! 目录按成员顺序输出模型，默认成员不一定排第一。补默认模型时必须取默认成员的
//! 模型（裸名），不能取目录第一项（别家成员的 `cpx-<relay>/<model>`）。

use codex_plus_core::paths::set_settings_path_for_tests;
use codex_plus_core::relay_config::apply_relay_profile_to_home_with_switch_rules;
use codex_plus_core::settings::{
    AggregateRelayMember, AggregateRelayProfile, AggregateRelayStrategy, BackendSettings, RelayMode,
    RelayProfile, RelaySessionProvider, SettingsStore,
};

fn api_member(id: &str, model: &str) -> RelayProfile {
    RelayProfile {
        id: id.to_string(),
        name: id.to_uppercase(),
        relay_mode: RelayMode::PureApi,
        base_url: format!("https://{id}.example.test/v1"),
        api_key: format!("sk-{id}"),
        model: model.to_string(),
        model_list: model.to_string(),
        ..RelayProfile::default()
    }
}

fn aggregate_relay() -> RelayProfile {
    RelayProfile {
        id: "agg".to_string(),
        name: "聚合供应商".to_string(),
        relay_mode: RelayMode::Aggregate,
        api_key: "codex-plus-aggregate".to_string(),
        ..RelayProfile::default()
    }
}

#[test]
fn missing_model_defaults_to_default_member_not_first_member() {
    let _serial = crate::common::global_state_lock();
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("codex-home");
    std::fs::create_dir_all(&home).unwrap();
    let settings_path = dir.path().join("settings.json");
    let _guard = set_settings_path_for_tests(Some(settings_path.clone()));

    let mut settings = BackendSettings::default();
    settings.relay_profiles = vec![
        api_member("relay-a", "model-a"),
        api_member("relay-c", "model-c"),
        aggregate_relay(),
    ];
    settings.active_relay_id = "agg".to_string();
    settings.active_aggregate_relay_id = "agg".to_string();
    settings.aggregate_relay_profiles = vec![AggregateRelayProfile {
        id: "agg".to_string(),
        name: "聚合供应商".to_string(),
        session_provider: RelaySessionProvider::Custom,
        model_aggregation_enabled: true,
        strategy: AggregateRelayStrategy::Failover,
        // A 排第一，C 才是默认成员。
        members: ["relay-a", "relay-c"]
            .into_iter()
            .map(|relay_id| AggregateRelayMember {
                relay_id: relay_id.to_string(),
                weight: 1,
            })
            .collect(),
        routes: Vec::new(),
        default_relay_id: "relay-c".to_string(),
        sync_default_member_login: false,
        auto_review_relay_id: String::new(),
    }];
    SettingsStore::new(settings_path.clone()).save(&settings).unwrap();

    apply_relay_profile_to_home_with_switch_rules(&home, &aggregate_relay(), "").unwrap();
    let config = std::fs::read_to_string(home.join("config.toml")).unwrap();
    let doc = config.parse::<toml_edit::DocumentMut>().unwrap();
    assert_eq!(doc["model"].as_str(), Some("model-c"), "{config}");
}
