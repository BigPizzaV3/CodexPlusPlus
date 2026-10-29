use codex_plus_core::relay_config::normalize_relay_profile_for_storage;
use codex_plus_core::settings::{
    BackendSettings, RelayMode, RelayProfile, RelayProtocol, SettingsStore,
};
use serde_json::json;

#[test]
fn local_compaction_defaults_off_and_preserves_search_compatibility_independently() {
    let defaults = serde_json::to_value(RelayProfile::default()).unwrap();
    assert!(defaults.get("localCompactionPromptAtEnd").is_none());
    assert!(
        !serde_json::from_value::<RelayProfile>(defaults)
            .unwrap()
            .local_compaction_prompt_at_end_enabled()
    );
    for search in [false, true] {
        for local in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let store = SettingsStore::new(temp.path().join("settings.json"));
            let mut profile = RelayProfile {
                id: "test".into(),
                relay_mode: RelayMode::PureApi,
                base_url: "https://example.test/v1".into(),
                upstream_base_url: "https://example.test/v1".into(),
                api_key: "fixture-key".into(),
                web_search_history_compat: search,
                local_compaction_prompt_at_end: local,
                ..RelayProfile::default()
            };
            normalize_relay_profile_for_storage(&mut profile).unwrap();
            assert_eq!(
                profile
                    .config_contents
                    .contains("http://127.0.0.1:57321/v1"),
                search || local
            );
            let settings = BackendSettings {
                enhancements_enabled: false,
                active_relay_id: profile.id.clone(),
                relay_profiles: vec![profile],
                ..BackendSettings::default()
            };
            store.save(&settings).unwrap();
            let saved = store.load().unwrap();
            assert_eq!(
                saved
                    .active_relay_profile()
                    .web_search_history_compat_enabled(),
                search
            );
            assert_eq!(
                saved
                    .active_relay_profile()
                    .local_compaction_prompt_at_end_enabled(),
                local
            );
            assert_eq!(
                saved.active_relay_transport_uses_protocol_proxy(),
                search || local
            );
            let mut profiles = serde_json::to_value(&saved.relay_profiles).unwrap();
            profiles[0]["localCompactionPromptAtEnd"] = json!(!local);
            let updated = store.update(json!({"relayProfiles": profiles})).unwrap();
            assert_eq!(
                updated
                    .active_relay_profile()
                    .local_compaction_prompt_at_end,
                !local
            );
            assert_eq!(
                updated.active_relay_profile().web_search_history_compat,
                search
            );
            let reloaded = store.load().unwrap();
            assert_eq!(
                reloaded.active_relay_profile().web_search_history_compat,
                search
            );
            assert_eq!(
                reloaded
                    .active_relay_profile()
                    .local_compaction_prompt_at_end,
                !local
            );
        }
    }
}

#[test]
fn local_compaction_supports_only_eligible_api_profiles() {
    for (mode, protocol, mixed, expected) in [
        (RelayMode::Official, RelayProtocol::Responses, false, false),
        (RelayMode::Official, RelayProtocol::Responses, true, true),
        (RelayMode::PureApi, RelayProtocol::Responses, false, true),
        (
            RelayMode::PureApi,
            RelayProtocol::ChatCompletions,
            false,
            true,
        ),
        (
            RelayMode::PureApi,
            RelayProtocol::AnthropicMessages,
            false,
            false,
        ),
        (RelayMode::Aggregate, RelayProtocol::Responses, false, false),
    ] {
        let profile = RelayProfile {
            relay_mode: mode,
            protocol,
            official_mix_api_key: mixed,
            local_compaction_prompt_at_end: true,
            ..RelayProfile::default()
        };
        assert_eq!(profile.local_compaction_prompt_at_end_enabled(), expected);
    }
}
