use codex_plus_core::relay_config::{
    RelayBackfillPolicy, apply_relay_profile_to_home_with_switch_rules,
    backfill_relay_profile_from_home_with_common,
    backfill_relay_profile_from_home_with_common_and_policy,
    ensure_active_protocol_proxy_config_in_home, normalize_relay_profile_for_storage,
    relay_profile_api_key, relay_profile_base_url, sanitize_official_profile_config,
};
use codex_plus_core::relay_switch::switch_relay_profile_in_home;
use codex_plus_core::settings::{
    BackendSettings, RelayMode, RelayProfile, RelayProtocol, SettingsStore,
};

fn api(id: &str, url: &str, key: &str) -> RelayProfile {
    RelayProfile {
        id: id.to_string(),
        name: id.to_string(),
        relay_mode: RelayMode::PureApi,
        base_url: url.to_string(),
        api_key: key.to_string(),
        config_contents: format!(
            "model_provider = \"custom\"\n[model_providers.custom]\nname = \"custom\"\nwire_api = \"responses\"\nrequires_openai_auth = true\nbase_url = \"{url}\"\n"
        ),
        auth_contents: serde_json::json!({"OPENAI_API_KEY":key}).to_string(),
        ..RelayProfile::default()
    }
}

fn write_live(home: &std::path::Path, profile: &RelayProfile) {
    std::fs::write(home.join("config.toml"), &profile.config_contents).unwrap();
    std::fs::write(home.join("auth.json"), &profile.auth_contents).unwrap();
}

#[test]
fn disabled_supplier_management_never_repairs_or_routes_live_config() {
    for (protocol, mixed, no_auth) in [
        (RelayProtocol::ChatCompletions, false, false),
        (RelayProtocol::Responses, true, false),
        (RelayProtocol::Responses, false, true),
    ] {
        let home = tempfile::tempdir().unwrap();
        let external = api(
            "external",
            "https://external.example/v1",
            "fixture-external",
        );
        write_live(home.path(), &external);
        std::fs::write(home.path().join("catalog.json"), "external-catalog").unwrap();
        let mut stale = api("stale", "https://stale.example/v1", "fixture-stale");
        stale.protocol = protocol;
        stale.official_mix_api_key = mixed;
        stale.relay_mode = if mixed {
            RelayMode::Official
        } else {
            RelayMode::PureApi
        };
        stale.no_auth = no_auth;
        let settings = BackendSettings {
            relay_profiles_enabled: false,
            active_relay_id: "stale".to_string(),
            relay_profiles: vec![stale],
            ..BackendSettings::default()
        };
        assert!(!ensure_active_protocol_proxy_config_in_home(home.path(), &settings).unwrap());
        assert_eq!(
            std::fs::read_to_string(home.path().join("config.toml")).unwrap(),
            external.config_contents
        );
        assert_eq!(
            std::fs::read_to_string(home.path().join("auth.json")).unwrap(),
            external.auth_contents
        );
        assert_eq!(
            std::fs::read_to_string(home.path().join("catalog.json")).unwrap(),
            "external-catalog"
        );
        // 关闭时不应连外部语法都强制检查。
        std::fs::write(home.path().join("config.toml"), "external = [").unwrap();
        assert!(!ensure_active_protocol_proxy_config_in_home(home.path(), &settings).unwrap());
    }
}

#[test]
fn unknown_endpoint_drift_refuses_backfill_without_mutating_any_snapshot() {
    for live_key in ["fixture-a", "fixture-b"] {
        let home = tempfile::tempdir().unwrap();
        let foreign = api("foreign", "https://unknown.example/v1", live_key);
        write_live(home.path(), &foreign);
        let mut previous = api("a", "https://a.example/v1", "fixture-a");
        let before = previous.clone();
        let mut common = "approval_policy = \"never\"\n".to_string();
        let error =
            backfill_relay_profile_from_home_with_common(home.path(), &mut previous, &mut common)
                .unwrap_err();
        assert!(error.to_string().contains("显式导入"));
        assert!(!error.to_string().contains(live_key));
        assert_eq!(previous, before);
        assert_eq!(common, "approval_policy = \"never\"\n");
        assert_eq!(
            std::fs::read_to_string(home.path().join("config.toml")).unwrap(),
            foreign.config_contents
        );
        assert_eq!(
            std::fs::read_to_string(home.path().join("auth.json")).unwrap(),
            foreign.auth_contents
        );
    }
}

#[test]
fn bound_api_missing_or_invalid_live_endpoint_preserves_all_state() {
    for config in [
        "model = \"gpt-5.6-sol\"\n",
        "model_provider = \"custom\"\n[model_providers.custom]\nbase_url = \"not-a-url\"\n",
    ] {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(home.path().join("config.toml"), config).unwrap();
        std::fs::write(home.path().join("auth.json"), "{}").unwrap();
        let original = api("a", "https://a.example/v1", "fixture-a");
        let mut profile = original.clone();
        let mut common = "approval_policy = \"never\"\n".to_string();
        assert!(
            backfill_relay_profile_from_home_with_common(home.path(), &mut profile, &mut common)
                .is_err()
        );
        assert_eq!(profile, original);
        assert_eq!(common, "approval_policy = \"never\"\n");
        assert_eq!(
            std::fs::read_to_string(home.path().join("config.toml")).unwrap(),
            config
        );
        assert_eq!(
            std::fs::read_to_string(home.path().join("auth.json")).unwrap(),
            "{}"
        );
    }
}

#[test]
fn explicit_api_adoption_needs_valid_url_even_when_live_has_key_or_no_auth_mode() {
    for no_auth in [false, true] {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(home.path().join("config.toml"), "model = \"gpt-5.6-sol\"\n").unwrap();
        std::fs::write(
            home.path().join("auth.json"),
            r#"{"OPENAI_API_KEY":"fixture-live"}"#,
        )
        .unwrap();
        let mut original = api("a", "https://a.example/v1", "fixture-a");
        original.no_auth = no_auth;
        let mut profile = original.clone();
        let mut common = "approval_policy = \"never\"\n".to_string();
        assert!(
            backfill_relay_profile_from_home_with_common_and_policy(
                home.path(),
                &mut profile,
                &mut common,
                RelayBackfillPolicy::AdoptLiveIdentity
            )
            .is_err()
        );
        assert_eq!(profile, original);
        assert_eq!(common, "approval_policy = \"never\"\n");
        assert_eq!(
            std::fs::read_to_string(home.path().join("auth.json")).unwrap(),
            r#"{"OPENAI_API_KEY":"fixture-live"}"#
        );
    }
}

#[test]
fn explicit_chat_adoption_does_not_reuse_a_different_legacy_upstream_marker() {
    let home = tempfile::tempdir().unwrap();
    let mut live = api("b", "https://b.example/v1", "fixture-b");
    live.config_contents = format!(
        "codex_plus_chat_base_url = \"https://a.example/v1\"\n{}",
        live.config_contents
    );
    write_live(home.path(), &live);
    let mut profile = api("a", "https://a.example/v1", "fixture-a");
    profile.protocol = RelayProtocol::ChatCompletions;
    backfill_relay_profile_from_home_with_common_and_policy(
        home.path(),
        &mut profile,
        &mut String::new(),
        RelayBackfillPolicy::AdoptLiveIdentity,
    )
    .unwrap();
    assert_eq!(relay_profile_base_url(&profile), "https://b.example/v1");
    assert_eq!(relay_profile_api_key(&profile), "fixture-b");
    assert!(!profile.config_contents.contains("codex_plus_chat_base_url"));
    normalize_relay_profile_for_storage(&mut profile).unwrap();
    assert_eq!(relay_profile_base_url(&profile), "https://b.example/v1");
    assert_eq!(relay_profile_api_key(&profile), "fixture-b");
    let store = SettingsStore::new(home.path().join("settings.json"));
    store
        .save(&BackendSettings {
            active_relay_id: profile.id.clone(),
            relay_profiles: vec![profile],
            ..BackendSettings::default()
        })
        .unwrap();
    let saved = store.load().unwrap().active_relay_profile();
    assert_eq!(relay_profile_base_url(&saved), "https://b.example/v1");
    assert_eq!(relay_profile_api_key(&saved), "fixture-b");
}

#[test]
fn explicit_adoption_does_not_forward_old_custom_credentials_to_the_new_endpoint() {
    use codex_plus_core::settings::RelayHeaderKeyValue;
    let home = tempfile::tempdir().unwrap();
    write_live(home.path(), &api("b", "https://b.example/v1", "fixture-b"));
    let mut profile = api("a", "https://a.example/v1", "fixture-a");
    profile.custom_headers = vec![
        RelayHeaderKeyValue {
            key: "Authorization".to_string(),
            value: "Bearer fixture-old-override".to_string(),
        },
        RelayHeaderKeyValue {
            key: "Cookie".to_string(),
            value: "fixture-cookie".to_string(),
        },
        RelayHeaderKeyValue {
            key: "X-API-Key".to_string(),
            value: "fixture-old-api".to_string(),
        },
        RelayHeaderKeyValue {
            key: "X-Client-Version".to_string(),
            value: "1".to_string(),
        },
        RelayHeaderKeyValue {
            key: "X-Company-Access-Code".to_string(),
            value: "fixture-unknown-credential".to_string(),
        },
    ];
    backfill_relay_profile_from_home_with_common_and_policy(
        home.path(),
        &mut profile,
        &mut String::new(),
        RelayBackfillPolicy::AdoptLiveIdentity,
    )
    .unwrap();
    normalize_relay_profile_for_storage(&mut profile).unwrap();
    let request = codex_plus_core::relay_headers::apply(
        reqwest::Client::new().post(format!("{}/responses", relay_profile_base_url(&profile))),
        &profile,
    )
    .build()
    .unwrap();
    assert_eq!(request.url().host_str(), Some("b.example"));
    assert_eq!(
        request.headers().get("authorization").unwrap(),
        "Bearer fixture-b"
    );
    assert!(request.headers().get("cookie").is_none());
    assert!(request.headers().get("x-api-key").is_none());
    assert!(request.headers().get("x-client-version").is_none());
    assert!(request.headers().get("x-company-access-code").is_none());
}

#[test]
fn old_custom_headers_are_not_mistaken_for_a_completely_unbound_profile() {
    use codex_plus_core::settings::RelayHeaderKeyValue;
    let home = tempfile::tempdir().unwrap();
    write_live(home.path(), &api("b", "https://b.example/v1", "fixture-b"));
    let mut profile = RelayProfile {
        relay_mode: RelayMode::PureApi,
        custom_headers: vec![RelayHeaderKeyValue {
            key: "X-Company-Access-Code".to_string(),
            value: "fixture-old".to_string(),
        }],
        ..RelayProfile::default()
    };
    let before = profile.clone();
    assert!(
        backfill_relay_profile_from_home_with_common(home.path(), &mut profile, &mut String::new())
            .is_err()
    );
    assert_eq!(profile, before);
}

#[test]
fn explicit_no_auth_adoption_keeps_its_api_mode_without_inventing_a_key() {
    let home = tempfile::tempdir().unwrap();
    let mut live = api("b", "https://b.example/v1", "");
    live.auth_contents = "{}".to_string();
    write_live(home.path(), &live);
    let mut profile = api("a", "https://a.example/v1", "");
    profile.no_auth = true;
    profile.auth_contents = "{}".to_string();
    backfill_relay_profile_from_home_with_common_and_policy(
        home.path(),
        &mut profile,
        &mut String::new(),
        RelayBackfillPolicy::AdoptLiveIdentity,
    )
    .unwrap();
    assert_eq!(profile.relay_mode, RelayMode::PureApi);
    assert!(profile.no_auth);
    assert_eq!(relay_profile_base_url(&profile), "https://b.example/v1");
    assert!(relay_profile_api_key(&profile).is_empty());
    normalize_relay_profile_for_storage(&mut profile).unwrap();
    assert_eq!(profile.relay_mode, RelayMode::PureApi);
    assert!(profile.no_auth);
    assert_eq!(profile.upstream_base_url, "https://b.example/v1");
    normalize_relay_profile_for_storage(&mut profile).unwrap();
    assert_eq!(relay_profile_base_url(&profile), "https://b.example/v1");
    let store = SettingsStore::new(home.path().join("settings.json"));
    store
        .save(&BackendSettings {
            active_relay_id: profile.id.clone(),
            relay_profiles: vec![profile],
            ..BackendSettings::default()
        })
        .unwrap();
    let saved = store.load().unwrap().active_relay_profile();
    assert_eq!(relay_profile_base_url(&saved), "https://b.example/v1");
    assert!(saved.no_auth);
    assert!(relay_profile_api_key(&saved).is_empty());
}

#[test]
fn no_config_source_keeps_bound_api_snapshot_and_allows_first_explicit_switch() {
    for source in [
        None,
        Some(""),
        Some(" \n"),
        Some("# configuration not initialized\n"),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        std::fs::create_dir(&home).unwrap();
        if let Some(source) = source {
            std::fs::write(home.join("config.toml"), source).unwrap();
        }
        std::fs::write(
            home.join("auth.json"),
            r#"{"OPENAI_API_KEY":"fixture-live"}"#,
        )
        .unwrap();
        let mut profile = api("a", "https://a.example/v1", "fixture-a");
        let before = profile.clone();
        let mut common = "approval_policy = \"never\"\n".to_string();
        backfill_relay_profile_from_home_with_common(&home, &mut profile, &mut common).unwrap();
        assert_eq!(profile, before);
        assert_eq!(common, "approval_policy = \"never\"\n");
        assert_eq!(
            std::fs::read_to_string(home.join("auth.json")).unwrap(),
            r#"{"OPENAI_API_KEY":"fixture-live"}"#
        );
        let store = SettingsStore::new(temp.path().join("settings.json"));
        let original = BackendSettings {
            active_relay_id: "a".to_string(),
            relay_profiles: vec![profile, api("b", "https://b.example/v1", "fixture-b")],
            ..BackendSettings::default()
        };
        store.save(&original).unwrap();
        let mut next = store.load().unwrap();
        let archived = next.relay_profiles[0].clone();
        next.active_relay_id = "b".to_string();
        switch_relay_profile_in_home(&store, &home, next, "a").unwrap();
        assert_eq!(store.load().unwrap().relay_profiles[0], archived);
        assert!(
            std::fs::read_to_string(home.join("config.toml"))
                .unwrap()
                .contains("https://b.example/v1")
        );
    }
}

#[test]
fn official_auth_only_source_refreshes_oauth_without_clearing_its_config_snapshot() {
    let home = tempfile::tempdir().unwrap();
    std::fs::write(home.path().join("auth.json"),r#"{"auth_mode":"chatgpt","tokens":{"access_token":"fixture-new"},"OPENAI_API_KEY":"fixture-api"}"#).unwrap();
    let config = "approval_policy = \"never\"\n";
    let mut profile = RelayProfile {
        relay_mode: RelayMode::Official,
        config_contents: config.to_string(),
        auth_contents: r#"{"auth_mode":"chatgpt","tokens":{"access_token":"fixture-old"}}"#
            .to_string(),
        ..RelayProfile::default()
    };
    backfill_relay_profile_from_home_with_common(home.path(), &mut profile, &mut String::new())
        .unwrap();
    assert_eq!(profile.config_contents, config);
    let auth: serde_json::Value = serde_json::from_str(&profile.auth_contents).unwrap();
    assert_eq!(auth["tokens"]["access_token"], "fixture-new");
    assert!(auth.get("OPENAI_API_KEY").is_none());
}

#[test]
fn failed_switch_restores_default_single_profile_and_unknown_fields_as_exact_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex");
    std::fs::create_dir(&home).unwrap();
    std::fs::write(home.join("auth.json"), "{}").unwrap();
    let path = temp.path().join("settings.json");
    let store = SettingsStore::new(path.clone());
    store.save(&BackendSettings::default()).unwrap();
    let mut raw: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    raw["futureUserField"] = serde_json::json!({"preserve":true});
    let before = serde_json::to_vec_pretty(&raw).unwrap();
    std::fs::write(&path, &before).unwrap();
    let mut next = store.load().unwrap();
    let mut bad = api("bad", "https://bad.example/v1", "fixture-bad");
    bad.model_list = "gpt-5.6-sol".to_string();
    // 保存两条设置以后，应用阶段遇到目录占据catalog文件路径；不能绕过回滚断言。
    std::fs::create_dir_all(home.join("model-catalogs/bad.json")).unwrap();
    next.relay_profiles.push(bad);
    next.active_relay_id = "bad".to_string();
    assert!(switch_relay_profile_in_home(&store, &home, next, "default").is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(store.load().unwrap().relay_profiles.len(), 1);
    assert_eq!(
        std::fs::read_to_string(home.join("auth.json")).unwrap(),
        "{}"
    );
    assert!(!home.join("config.toml").exists());
}

#[test]
fn explicit_live_adoption_uses_the_live_endpoint_and_key_together() {
    let home = tempfile::tempdir().unwrap();
    let foreign = api("foreign", "https://manual.example/v1", "fixture-new");
    write_live(home.path(), &foreign);
    let mut profile = api("a", "https://a.example/v1", "fixture-old");
    backfill_relay_profile_from_home_with_common_and_policy(
        home.path(),
        &mut profile,
        &mut String::new(),
        RelayBackfillPolicy::AdoptLiveIdentity,
    )
    .unwrap();
    normalize_relay_profile_for_storage(&mut profile).unwrap();
    assert_eq!(
        relay_profile_base_url(&profile),
        "https://manual.example/v1"
    );
    assert_eq!(relay_profile_api_key(&profile), "fixture-new");
    assert!(!profile.auth_contents.contains("fixture-old"));
}

#[test]
fn explicit_adoption_prefers_provider_bearer_and_refuses_missing_credentials() {
    let home = tempfile::tempdir().unwrap();
    let mut live = api("manual", "https://manual.example/v1", "fixture-login");
    live.config_contents
        .push_str("experimental_bearer_token = \"fixture-provider\"\n");
    write_live(home.path(), &live);
    let original = api("a", "https://a.example/v1", "fixture-old");
    let mut profile = original.clone();
    backfill_relay_profile_from_home_with_common_and_policy(
        home.path(),
        &mut profile,
        &mut String::new(),
        RelayBackfillPolicy::AdoptLiveIdentity,
    )
    .unwrap();
    assert_eq!(relay_profile_api_key(&profile), "fixture-provider");
    live.config_contents = live
        .config_contents
        .replace("experimental_bearer_token = \"fixture-provider\"\n", "");
    live.auth_contents = "{}".to_string();
    write_live(home.path(), &live);
    profile = original.clone();
    let error = backfill_relay_profile_from_home_with_common_and_policy(
        home.path(),
        &mut profile,
        &mut String::new(),
        RelayBackfillPolicy::AdoptLiveIdentity,
    )
    .unwrap_err();
    assert!(error.to_string().contains("完整 URL"));
    assert_eq!(profile, original);
}

#[test]
fn equivalent_provider_rename_and_key_change_do_not_block_automatic_backfill() {
    let home = tempfile::tempdir().unwrap();
    let mut live = api("live", "https://a.example/v1/", "fixture-rotated");
    live.config_contents = live.config_contents.replace("custom", "renamed");
    write_live(home.path(), &live);
    let mut profile = api("a", "https://a.example/v1", "fixture-archived");
    backfill_relay_profile_from_home_with_common(home.path(), &mut profile, &mut String::new())
        .unwrap();
    assert!(profile.config_contents.contains("renamed"));
    // 默认保留归档Key，以兼容Codex登录覆盖live认证；显式采纳可轮换。
    assert_eq!(relay_profile_api_key(&profile), "fixture-archived");
    backfill_relay_profile_from_home_with_common_and_policy(
        home.path(),
        &mut profile,
        &mut String::new(),
        RelayBackfillPolicy::AdoptLiveIdentity,
    )
    .unwrap();
    assert_eq!(relay_profile_api_key(&profile), "fixture-rotated");
}

#[test]
fn saved_managed_proxy_port_and_legacy_marker_survive_port_changes() {
    let current = codex_plus_core::protocol_proxy::protocol_proxy_port();
    let old_port = if current == 65431 { 65430 } else { 65431 };
    let old_url = format!("http://127.0.0.1:{old_port}/v1");
    for protocol in [RelayProtocol::Responses, RelayProtocol::ChatCompletions] {
        let home = tempfile::tempdir().unwrap();
        let mut profile = api("a", "https://a.example/v1", "fixture-a");
        profile.protocol = protocol;
        profile.upstream_base_url = "https://a.example/v1".to_string();
        profile.config_contents = profile
            .config_contents
            .replace("https://a.example/v1", &old_url);
        write_live(home.path(), &profile);
        let mut saved = profile.clone();
        backfill_relay_profile_from_home_with_common(home.path(), &mut saved, &mut String::new())
            .unwrap();
        assert_eq!(relay_profile_base_url(&saved), "https://a.example/v1");
        assert_eq!(relay_profile_api_key(&saved), "fixture-a");
    }
    let home = tempfile::tempdir().unwrap();
    let mut profile = api("a", "https://a.example/v1", "fixture-a");
    profile.protocol = RelayProtocol::ChatCompletions;
    profile.upstream_base_url = "https://a.example/v1".to_string();
    let mut live = api("a", &old_url, "fixture-a");
    live.config_contents = format!(
        "codex_plus_chat_base_url = \"https://a.example/v1\"\n{}",
        live.config_contents
    );
    write_live(home.path(), &live);
    backfill_relay_profile_from_home_with_common(home.path(), &mut profile, &mut String::new())
        .unwrap();
    assert_eq!(relay_profile_base_url(&profile), "https://a.example/v1");
}

#[test]
fn unrelated_loopback_proxy_without_saved_identity_or_marker_is_not_trusted() {
    let current = codex_plus_core::protocol_proxy::protocol_proxy_port();
    let other = if current == 65429 { 65428 } else { 65429 };
    let home = tempfile::tempdir().unwrap();
    let live = api(
        "third-party",
        &format!("http://127.0.0.1:{other}/v1"),
        "fixture-local",
    );
    write_live(home.path(), &live);
    let mut profile = api("a", "https://a.example/v1", "fixture-a");
    profile.protocol = RelayProtocol::ChatCompletions;
    let original = profile.clone();
    assert!(
        backfill_relay_profile_from_home_with_common(home.path(), &mut profile, &mut String::new())
            .is_err()
    );
    assert_eq!(profile, original);
}

#[test]
fn known_external_supplier_with_shared_key_does_not_pollute_the_previous_profile() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex");
    std::fs::create_dir(&home).unwrap();
    let a = api("a", "https://a.example/v1", "fixture-shared");
    let mut b = api("b", "https://b.example/v1", "fixture-shared");
    b.config_contents = b.config_contents.replace("custom", "external-renamed");
    let c = api("c", "https://c.example/v1", "fixture-c");
    write_live(&home, &b);
    let store = SettingsStore::new(temp.path().join("settings.json"));
    let original = BackendSettings {
        active_relay_id: "a".to_string(),
        relay_profiles: vec![a, b, c],
        ..BackendSettings::default()
    };
    store.save(&original).unwrap();
    let before = store.load().unwrap().relay_profiles[0].clone();
    let mut next = store.load().unwrap();
    next.active_relay_id = "c".to_string();
    switch_relay_profile_in_home(&store, &home, next, "a").unwrap();
    assert_eq!(store.load().unwrap().relay_profiles[0], before);
    assert!(
        std::fs::read_to_string(home.join("config.toml"))
            .unwrap()
            .contains("https://c.example/v1")
    );
}

#[test]
fn known_supplier_on_same_endpoint_is_disambiguated_by_its_saved_key() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex");
    std::fs::create_dir(&home).unwrap();
    let a = api("a", "https://shared.example/v1", "fixture-a");
    let mut b = api("b", "https://shared.example/v1", "fixture-b");
    b.config_contents = format!("model_reasoning_effort = \"low\"\n{}", b.config_contents);
    write_live(&home, &b);
    let store = SettingsStore::new(temp.path().join("settings.json"));
    let original = BackendSettings {
        active_relay_id: "a".to_string(),
        relay_profiles: vec![a, b, api("c", "https://c.example/v1", "fixture-c")],
        ..BackendSettings::default()
    };
    store.save(&original).unwrap();
    let mut next = store.load().unwrap();
    let previous = next.relay_profiles[0].clone();
    next.active_relay_id = "c".to_string();
    switch_relay_profile_in_home(&store, &home, next, "a").unwrap();
    assert_eq!(store.load().unwrap().relay_profiles[0], previous);
}

#[test]
fn ambiguous_manual_switch_keeps_settings_live_auth_and_catalog_unchanged() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex");
    std::fs::create_dir(&home).unwrap();
    let live = api("manual", "https://manual.example/v1", "fixture-manual");
    write_live(&home, &live);
    std::fs::write(home.join("catalog.json"), "original-catalog").unwrap();
    let store = SettingsStore::new(temp.path().join("settings.json"));
    let original = BackendSettings {
        active_relay_id: "a".to_string(),
        relay_profiles: vec![
            api("a", "https://a.example/v1", "fixture-a"),
            api("b", "https://b.example/v1", "fixture-b"),
        ],
        ..BackendSettings::default()
    };
    store.save(&original).unwrap();
    let settings_before = std::fs::read(temp.path().join("settings.json")).unwrap();
    let mut next = store.load().unwrap();
    next.active_relay_id = "b".to_string();
    assert!(
        switch_relay_profile_in_home(&store, &home, next, "a")
            .unwrap_err()
            .to_string()
            .contains("endpoint")
    );
    assert_eq!(
        std::fs::read(temp.path().join("settings.json")).unwrap(),
        settings_before
    );
    assert_eq!(
        std::fs::read_to_string(home.join("config.toml")).unwrap(),
        live.config_contents
    );
    assert_eq!(
        std::fs::read_to_string(home.join("auth.json")).unwrap(),
        live.auth_contents
    );
    assert_eq!(
        std::fs::read_to_string(home.join("catalog.json")).unwrap(),
        "original-catalog"
    );
}

#[test]
fn official_config_sanitization_keeps_settings_and_removes_nested_provider_credentials() {
    let config = "model_provider = \"custom\"\nmodel = \"third-party\"\napproval_policy = \"never\"\nmodel_catalog_json = \"external.json\"\n[model_providers.custom]\nbase_url = \"https://foreign.example/v1\"\nexperimental_bearer_token = \"fixture-private\"\n[profiles.work]\nmodel_provider = \"custom\"\nmodel = \"third-party\"\napi_key = \"fixture-nested\"\nsandbox_mode = \"workspace-write\"\n";
    let result = sanitize_official_profile_config(config).unwrap();
    assert!(result.contains("approval_policy"));
    assert!(result.contains("sandbox_mode"));
    for removed in [
        "model_provider",
        "model_providers",
        "model_catalog_json",
        "fixture-private",
        "fixture-nested",
        "third-party",
    ] {
        assert!(!result.contains(removed), "{removed}");
    }
}

#[test]
fn official_api_official_roundtrip_restores_non_auth_profile_and_common_contract() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex");
    std::fs::create_dir(&home).unwrap();
    let official_config = "approval_policy = \"never\"\nsandbox_mode = \"workspace-write\"\nmodel_reasoning_effort = \"high\"\n[features]\ngoals = false\nofficial_only = true\n";
    let official = RelayProfile {
        id: "official".to_string(),
        relay_mode: RelayMode::Official,
        config_contents: official_config.to_string(),
        auth_contents: r#"{"auth_mode":"chatgpt","tokens":{"access_token":"fixture-oauth"}}"#
            .to_string(),
        ..RelayProfile::default()
    };
    write_live(&home, &official);
    let store = SettingsStore::new(temp.path().join("settings.json"));
    let settings = BackendSettings {
        active_relay_id: "official".to_string(),
        relay_common_config_contents:
            "verbosity = \"common\"\n[features]\ngoals = true\ncommon_only = true\n".to_string(),
        relay_profiles: vec![official, api("api", "https://a.example/v1", "fixture-a")],
        ..BackendSettings::default()
    };
    store.save(&settings).unwrap();
    let mut api_settings = store.load().unwrap();
    api_settings.active_relay_id = "api".to_string();
    switch_relay_profile_in_home(&store, &home, api_settings, "official").unwrap();
    let mut official_settings = store.load().unwrap();
    official_settings.active_relay_id = "official".to_string();
    switch_relay_profile_in_home(&store, &home, official_settings, "api").unwrap();
    let text = std::fs::read_to_string(home.join("config.toml")).unwrap();
    let parsed: toml::Value = text.parse().unwrap();
    assert_eq!(parsed["approval_policy"].as_str(), Some("never"));
    assert_eq!(parsed["model_reasoning_effort"].as_str(), Some("high"));
    assert_eq!(parsed["features"]["goals"].as_bool(), Some(false));
    assert_eq!(parsed["features"]["common_only"].as_bool(), Some(true));
    assert_eq!(parsed["verbosity"].as_str(), Some("common"));
    assert!(!text.contains("model_provider"));
    assert!(!text.contains("https://a.example"));
    let auth: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.join("auth.json")).unwrap()).unwrap();
    assert!(auth.get("OPENAI_API_KEY").is_none());
    assert_eq!(auth["tokens"]["access_token"], "fixture-oauth");
    let mut profile = store.load().unwrap().relay_profiles[0].clone();
    profile.use_common_config = false;
    apply_relay_profile_to_home_with_switch_rules(
        &home,
        &profile,
        "approval_policy = \"on-request\"\n",
    )
    .unwrap();
    assert!(
        std::fs::read_to_string(home.join("config.toml"))
            .unwrap()
            .contains("approval_policy = \"never\"")
    );
}

#[test]
fn official_restore_rebuilds_its_catalog_and_keeps_per_model_limits() {
    let home = tempfile::tempdir().unwrap();
    let mut previous_api = api("api", "https://a.example/v1", "fixture-a");
    previous_api.config_contents = format!(
        "model_catalog_json = \"external-api.json\"\n{}",
        previous_api.config_contents
    );
    write_live(home.path(), &previous_api);
    std::fs::write(
        home.path().join("external-api.json"),
        r#"{"models":[{"slug":"wrong-api-model"}]}"#,
    )
    .unwrap();
    let profile = RelayProfile {
        id: "official".to_string(),
        relay_mode: RelayMode::Official,
        config_contents: "approval_policy = \"never\"\n".to_string(),
        model: "gpt-5.6-sol".to_string(),
        model_list: "gpt-5.6-sol".to_string(),
        model_windows: r#"{"gpt-5.6-sol":"200000"}"#.to_string(),
        model_auto_compact: r#"{"gpt-5.6-sol":"80"}"#.to_string(),
        context_window: "1000000".to_string(),
        auto_compact_limit: "900000".to_string(),
        auth_contents: "{}".to_string(),
        ..RelayProfile::default()
    };
    apply_relay_profile_to_home_with_switch_rules(home.path(), &profile, "").unwrap();
    let config: toml::Value = std::fs::read_to_string(home.path().join("config.toml"))
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(config["model_context_window"].as_integer(), Some(1000000));
    assert_eq!(
        config["model_auto_compact_token_limit"].as_integer(),
        Some(900000)
    );
    assert_eq!(
        config["model_catalog_json"].as_str(),
        Some("model-catalogs/official.json")
    );
    let catalog: serde_json::Value = serde_json::from_slice(
        &std::fs::read(home.path().join("model-catalogs/official.json")).unwrap(),
    )
    .unwrap();
    let entry = catalog["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["slug"] == "gpt-5.6-sol")
        .unwrap();
    assert_eq!(entry["context_window"], 200000);
    assert_eq!(entry["auto_compact_token_limit"], 160000);
    assert_eq!(entry["use_responses_lite"], false);
    assert!(config.get("model_providers").is_none());

    // 生成目录后发生认证错误，旧live与目录均需恢复。
    let config_before = std::fs::read(home.path().join("config.toml")).unwrap();
    let catalog_before = std::fs::read(home.path().join("model-catalogs/official.json")).unwrap();
    let mut invalid = profile.clone();
    invalid.auth_contents = "{bad".to_string();
    invalid.model_windows = r#"{"gpt-5.6-sol":"300000"}"#.to_string();
    assert!(apply_relay_profile_to_home_with_switch_rules(home.path(), &invalid, "").is_err());
    assert_eq!(
        std::fs::read(home.path().join("config.toml")).unwrap(),
        config_before
    );
    assert_eq!(
        std::fs::read(home.path().join("model-catalogs/official.json")).unwrap(),
        catalog_before
    );
}

#[test]
fn pure_official_snapshot_that_sanitizes_to_empty_is_valid() {
    let home = tempfile::tempdir().unwrap();
    let profile = RelayProfile { relay_mode:RelayMode::Official,
        config_contents:"model_provider = \"custom\"\n[model_providers.custom]\nbase_url = \"https://a.example/v1\"\n".to_string(),
        auth_contents:"{}".to_string(), ..RelayProfile::default() };
    assert!(
        sanitize_official_profile_config(&profile.config_contents)
            .unwrap()
            .trim()
            .is_empty()
    );
    apply_relay_profile_to_home_with_switch_rules(home.path(), &profile, "").unwrap();
    let config: toml::Value = std::fs::read_to_string(home.path().join("config.toml"))
        .unwrap()
        .parse()
        .unwrap();
    assert!(config.get("model_provider").is_none());
}
