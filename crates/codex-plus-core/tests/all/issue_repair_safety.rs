use codex_plus_core::settings::{
    BackendSettings, RelayMode, RelayProfile, RelayProtocol, SettingsStore,
};
use codex_plus_core::{ccs_import::*, relay_config::*};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};

#[test]
fn stale_full_save_cannot_overwrite_patch_or_unknown_fields() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    std::fs::write(&path, r#"{"futureSetting":{"keep":true},"relayProfiles":[{"id":"default","name":"default","futureProfile":7}]}"#).unwrap();
    let store = SettingsStore::new(path.clone());
    let stale = store.load().unwrap();
    store.update(json!({"providerSyncEnabled":true})).unwrap();
    let before = std::fs::read(&path).unwrap();
    assert!(
        store
            .save(&stale)
            .unwrap_err()
            .to_string()
            .contains("revision_conflict")
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let mut latest = store.load().unwrap();
    latest.relay_test_model = "new-model".into();
    store.save(&latest).unwrap();
    let disk: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(disk["futureSetting"]["keep"], true);
    assert_eq!(disk["relayProfiles"][0]["futureProfile"], 7);
    assert!(disk.get("revision").is_none());
}

#[test]
fn nested_unknown_fields_survive_but_removed_import_origins_do_not_reappear() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    std::fs::write(&path,json!({"relayProfiles":[{"id":"p","name":"p","futureProfile":7}],
        "ccsImportSources":{"p":{"kind":"cc-switch","database":"fixture","sourceId":"source"}},
        "tools":{"codex":{"futureTool":{"keep":true},"relayProfiles":[{"id":"p","name":"p","nestedFuture":9}]}},
        "aggregateRelayProfiles":[{"id":"agg","name":"agg","futureAggregate":5}]}).to_string()).unwrap();
    let store = SettingsStore::new(path.clone());
    store.update(json!({"providerSyncEnabled":true})).unwrap();
    let mut latest = store.load().unwrap();
    latest.ccs_import_sources.clear();
    store.save(&latest).unwrap();
    let disk: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert!(disk.get("ccsImportSources").is_none());
    assert_eq!(disk["relayProfiles"][0]["futureProfile"], 7);
    assert_eq!(disk["tools"]["codex"]["futureTool"]["keep"], true);
    assert_eq!(
        disk["tools"]["codex"]["relayProfiles"][0]["nestedFuture"],
        9
    );
    assert_eq!(disk["aggregateRelayProfiles"][0]["futureAggregate"], 5);
}

#[test]
fn explicit_stale_patch_revision_is_rejected_and_initial_import_returns_current_revision() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    let store = SettingsStore::new(path.clone());
    store.save(&BackendSettings::default()).unwrap();
    let first = store.load().unwrap();
    store.update(json!({"providerSyncEnabled":true})).unwrap();
    let before = std::fs::read(&path).unwrap();
    assert!(
        store
            .update(json!({"revision":first.settings_revision,"relayTestModel":"stale"}))
            .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let home = dir.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let imported =
        codex_plus_core::provider_import::initialize_local_config_provider(&store, &home).unwrap();
    assert_eq!(imported.settings_revision, store.revision().unwrap());
}

#[test]
fn manual_lookalike_is_not_claimed_and_same_source_id_in_another_database_is_distinct() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("one.db");
    std::fs::write(&db, "fixture").unwrap();
    let provider = CcsProviderImport {
        source_id: "s".into(),
        name: "Same".into(),
        base_url: "https://fixture.invalid/Case".into(),
        api_key: "fixture".into(),
        protocol: RelayProtocol::Responses,
        config_contents: String::new(),
        auth_contents: String::new(),
    };
    let mut source = CcsProviderSource {
        configured_db_path: String::new(),
        db_path: db,
        fallback_reason: None,
        providers: vec![provider],
    };
    let mut settings = BackendSettings::default();
    settings.relay_profiles.push(RelayProfile {
        id: "manual-copy".into(),
        name: "Same（ccswitch）".into(),
        upstream_base_url: "https://fixture.invalid/Case".into(),
        ..RelayProfile::default()
    });
    assert_eq!(
        import_providers_with_origins(&mut settings, &source, &BTreeMap::new()).unwrap(),
        (1, 0)
    );
    assert!(!settings.ccs_import_sources.contains_key("manual-copy"));
    source.db_path = dir.path().join("two.db");
    std::fs::write(&source.db_path, "fixture").unwrap();
    assert_eq!(
        import_providers_with_origins(&mut settings, &source, &BTreeMap::new()).unwrap(),
        (1, 0)
    );
    assert_eq!(settings.ccs_import_sources.len(), 2);
}

#[test]
fn legacy_origin_backfill_requires_unique_match_on_both_sides() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("source.db");
    std::fs::write(&database, "fixture").unwrap();
    let first = CcsProviderImport {
        source_id: "source/path".into(),
        name: "Same".into(),
        base_url: "https://fixture.invalid/Case".into(),
        api_key: "fixture".into(),
        protocol: RelayProtocol::Responses,
        config_contents: String::new(),
        auth_contents: String::new(),
    };
    let mut second = first.clone();
    second.source_id = "source-path".into();
    let legacy = relay_profile_from_ccs(&first, &[]);
    let mut settings = BackendSettings::default();
    settings.relay_profiles.push(legacy);
    let before = settings.clone();
    let source = CcsProviderSource {
        configured_db_path: String::new(),
        db_path: database,
        fallback_reason: None,
        providers: vec![first, second],
    };
    assert!(import_providers_with_origins(&mut settings, &source, &BTreeMap::new()).is_err());
    assert_eq!(
        settings, before,
        "source ID清洗后撞名不能根据迭代顺序猜来源"
    );
}

fn publication_fixture() -> (
    tempfile::TempDir,
    SettingsStore,
    std::path::PathBuf,
    codex_plus_core::settings::AggregateRelayProfile,
) {
    use codex_plus_core::settings::{AggregateRelayMember, AggregateRelayProfile};
    let dir = tempfile::tempdir().unwrap();
    let store = SettingsStore::new(dir.path().join("settings.json"));
    let home = dir.path().join("home");
    std::fs::create_dir_all(home.join("model-catalogs")).unwrap();
    let aggregate = AggregateRelayProfile {
        id: "agg".into(),
        name: "Aggregate".into(),
        model_aggregation_enabled: true,
        members: vec![AggregateRelayMember {
            relay_id: "member".into(),
            weight: 1,
        }],
        session_provider: Default::default(),
        strategy: Default::default(),
        routes: Vec::new(),
        default_relay_id: String::new(),
        sync_default_member_login: false,
        auto_review_relay_id: String::new(),
    };
    let mut settings = BackendSettings::default();
    settings.active_relay_id = "agg".into();
    settings.active_aggregate_relay_id = "agg".into();
    settings.aggregate_relay_profiles = vec![aggregate.clone()];
    settings.relay_profiles=vec![RelayProfile{id:"member".into(),name:"Member".into(),relay_mode:RelayMode::PureApi,no_auth:true,config_contents:"model='alpha'\nmodel_provider='custom'\n[model_providers.custom]\nbase_url='http://127.0.0.1:1/v1'\nwire_api='responses'\n".into(),model_list:"alpha".into(),..RelayProfile::default()},RelayProfile{id:"agg".into(),name:"Aggregate".into(),relay_mode:RelayMode::Aggregate,..RelayProfile::default()}];
    store.save(&settings).unwrap();
    let settings = store.load().unwrap();
    let catalog =
        codex_plus_core::aggregate_catalog::build_catalog_json(&settings, &aggregate).unwrap();
    let path = home.join("model-catalogs/agg.json");
    std::fs::write(&path, catalog).unwrap();
    std::fs::write(
        home.join("config.toml"),
        format!("model_catalog_json = {:?}\n", path.to_str().unwrap()),
    )
    .unwrap();
    (dir, store, home, aggregate)
}

#[test]
fn active_member_save_publishes_and_repairs_a_drifted_projection() {
    let (_dir, store, home, aggregate) = publication_fixture();
    let mut next = store.load().unwrap();
    next.relay_profiles[0].model_list = "alpha\nbeta".into();
    codex_plus_core::aggregate_catalog::save_with_active_catalog_refresh(&store, &home, &next)
        .unwrap();
    let latest = store.load().unwrap();
    let summary =
        codex_plus_core::aggregate_catalog::catalog_summary(&home, &latest, &aggregate).unwrap();
    assert_eq!(summary["models"].as_array().unwrap().len(), 2);
    assert_eq!(summary["applied"], true);
    assert_eq!(summary["pending_changes"], false);
    let config: toml_edit::DocumentMut = std::fs::read_to_string(home.join("config.toml"))
        .unwrap()
        .parse()
        .unwrap();
    let published = std::path::PathBuf::from(config["model_catalog_json"].as_str().unwrap());
    // 模拟 settings 已经更新、发布目录仍旧：无新编辑的保存也必须修复实际投影。
    let mut drifted: Value = serde_json::from_slice(&std::fs::read(&published).unwrap()).unwrap();
    drifted["models"].as_array_mut().unwrap().pop();
    std::fs::write(&published, drifted.to_string()).unwrap();
    // 内容寻址文件损坏应拒绝覆盖；切回旧的可变 managed 指针再重新发布。
    std::fs::write(
        home.join("config.toml"),
        format!(
            "model_catalog_json = {:?}\n",
            home.join("model-catalogs/agg.json").to_str().unwrap()
        ),
    )
    .unwrap();
    assert!(
        codex_plus_core::aggregate_catalog::save_with_active_catalog_refresh(
            &store, &home, &latest
        )
        .is_err()
    );
    // 换一个合法新投影验证不会因 previous == next 而直接跳过。
    let mut repaired = store.load().unwrap();
    repaired.relay_profiles[0].model_list = "alpha\nbeta\ngamma".into();
    codex_plus_core::aggregate_catalog::save_with_active_catalog_refresh(&store, &home, &repaired)
        .unwrap();
    let summary = codex_plus_core::aggregate_catalog::catalog_summary(
        &home,
        &store.load().unwrap(),
        &aggregate,
    )
    .unwrap();
    assert_eq!(summary["models"].as_array().unwrap().len(), 3);
    let latest = store.load().unwrap();
    std::fs::write(
        home.join("config.toml"),
        format!(
            "model_catalog_json = {:?}\n",
            home.join("model-catalogs/agg.json").to_str().unwrap()
        ),
    )
    .unwrap();
    codex_plus_core::aggregate_catalog::save_with_active_catalog_refresh(&store, &home, &latest)
        .unwrap();
    let summary = codex_plus_core::aggregate_catalog::catalog_summary(
        &home,
        &store.load().unwrap(),
        &aggregate,
    )
    .unwrap();
    assert_eq!(summary["models"].as_array().unwrap().len(), 3);
}

#[test]
fn summary_uses_owned_published_snapshot_when_candidate_is_invalid_and_ignores_foreign_pointer() {
    let (dir, store, home, aggregate) = publication_fixture();
    let mut invalid = store.load().unwrap();
    invalid.relay_profiles[0].model_metadata = r#"{"alpha":{"base_instructions":null}}"#.into();
    let summary =
        codex_plus_core::aggregate_catalog::catalog_summary(&home, &invalid, &aggregate).unwrap();
    assert_eq!(summary["applied"], true);
    assert_eq!(summary["pending_changes"], true);
    let foreign = dir.path().join("foreign.json");
    std::fs::write(&foreign, r#"{"models":[]}"#).unwrap();
    std::fs::write(
        home.join("config.toml"),
        format!("model_catalog_json = {:?}\n", foreign.to_str().unwrap()),
    )
    .unwrap();
    let summary = codex_plus_core::aggregate_catalog::catalog_summary(
        &home,
        &store.load().unwrap(),
        &aggregate,
    )
    .unwrap();
    assert_eq!(summary["applied"], false);
}

#[test]
#[ignore = "由跨进程 CAS 父测试显式启动的人工子进程"]
fn cas_child_process_worker() {
    let directory = std::path::PathBuf::from(std::env::var("CODEX_ISSUE_CAS_DIR").unwrap());
    let label = std::env::var("CODEX_ISSUE_CAS_WORKER").unwrap();
    let store = SettingsStore::new(directory.join("settings.json"));
    let mut settings = store.load().unwrap();
    std::fs::write(directory.join(format!("ready-{label}")), "ready").unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !directory.join("go").exists() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    settings.relay_test_model = label.clone();
    let outcome = match store.save(&settings) {
        Ok(()) => "ok",
        Err(error) => {
            assert!(error.to_string().contains("revision_conflict"));
            "conflict"
        }
    };
    std::fs::write(directory.join(format!("result-{label}")), outcome).unwrap();
}

#[test]
fn cross_process_full_saves_have_exactly_one_winner() {
    let directory = tempfile::tempdir().unwrap();
    let store = SettingsStore::new(directory.path().join("settings.json"));
    store.save(&BackendSettings::default()).unwrap();
    let mut children = Vec::new();
    for label in ["writer-a", "writer-b"] {
        children.push(
            std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    // 合并测试程序后用例名带模块前缀。
                    "issue_repair_safety::cas_child_process_worker",
                    "--ignored",
                    "--test-threads=1",
                ])
                .env("CODEX_ISSUE_CAS_DIR", directory.path())
                .env("CODEX_ISSUE_CAS_WORKER", label)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        );
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !["writer-a", "writer-b"]
        .iter()
        .all(|label| directory.path().join(format!("ready-{label}")).is_file())
    {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    std::fs::write(directory.path().join("go"), "go").unwrap();
    for mut child in children {
        assert!(child.wait().unwrap().success());
    }
    let mut outcomes = ["writer-a", "writer-b"].map(|label| {
        std::fs::read_to_string(directory.path().join(format!("result-{label}"))).unwrap()
    });
    outcomes.sort();
    assert_eq!(outcomes, ["conflict", "ok"]);
}

#[test]
fn failed_publication_preserves_external_newer_settings() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    let store = SettingsStore::new(path.clone());
    store.save(&BackendSettings::default()).unwrap();
    let mut next = store.load().unwrap();
    next.provider_sync_enabled = true;
    assert!(
        store
            .save_with_commit(&next, |_| {
                std::fs::write(&path, r#"{"external":"new"}"#)?;
                anyhow::bail!("injected publication failure")
            })
            .unwrap_err()
            .to_string()
            .contains("外部修改")
    );
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        r#"{"external":"new"}"#
    );
}

#[test]
fn unsupported_auth_is_preserved_but_edit_and_apply_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    let auth = r#"{"personal_access_token":"artificial-token"}"#;
    std::fs::write(&path, json!({"relayProfiles":[{"id":"legacy","name":"Legacy","relayMode":"official","authContents":auth}],"activeRelayId":"legacy"}).to_string()).unwrap();
    let store = SettingsStore::new(path.clone());
    let mut settings = store.load().unwrap();
    assert_eq!(settings.relay_profiles[0].auth_contents, auth);
    settings.relay_test_model = "unrelated-edit".into();
    store.save(&settings).unwrap();
    settings = store.load().unwrap();
    let before = std::fs::read(&path).unwrap();
    settings.relay_profiles[0].auth_contents = r#"{"unknown_auth":"new"}"#.into();
    assert!(store.save(&settings).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let home = dir.path().join("codex");
    assert!(
        apply_relay_profile_to_home_with_switch_rules(&home, &settings.relay_profiles[0], "")
            .is_err()
    );
    assert!(!home.join("auth.json").exists());
}

#[test]
fn invalid_catalog_overlay_keeps_previous_live_files() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    std::fs::create_dir(home.join("model-catalogs")).unwrap();
    let catalog = home.join("model-catalogs/custom.json");
    std::fs::write(&catalog, "original-catalog").unwrap();
    std::fs::write(home.join("config.toml"), "model = 'old'\n").unwrap();
    let profile = RelayProfile { id:"custom".into(), relay_mode:RelayMode::PureApi, no_auth:true,
        config_contents:"model='new'\nmodel_provider='custom'\n[model_providers.custom]\nbase_url='http://127.0.0.1:1/v1'\nwire_api='responses'\n".into(),
        model_list:"custom-model[1M]".into(), model_metadata:r#"{"custom-model":{"base_instructions":null}}"#.into(), ..RelayProfile::default() };
    assert!(apply_relay_profile_to_home_with_switch_rules(home, &profile, "").is_err());
    assert_eq!(
        std::fs::read_to_string(catalog).unwrap(),
        "original-catalog"
    );
    assert_eq!(
        std::fs::read_to_string(home.join("config.toml")).unwrap(),
        "model = 'old'\n"
    );
    let entries = codex_plus_core::model_suffix::collect_catalog_entries(
        "unknown-model[1M]",
        &HashMap::new(),
        &HashMap::new(),
        "",
    );
    let result: Value = serde_json::from_str(
        &codex_plus_core::model_suffix::build_model_catalog_json(&entries, None),
    )
    .unwrap();
    assert_eq!(result["models"][0]["supports_parallel_tool_calls"], false);
}

#[test]
fn derived_external_catalog_copy_is_validated_before_publication() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    std::fs::create_dir(home.join("model-catalogs")).unwrap();
    let external = home.join("external.json");
    let source=json!({"models":[{"slug":"custom-model","base_instructions":null,"supports_parallel_tool_calls":false,"context_window":1000,"use_responses_lite":true}]}).to_string();
    std::fs::write(&external, &source).unwrap();
    std::fs::write(home.join("config.toml"), "model='old'\n").unwrap();
    std::fs::write(home.join("model-catalogs/custom.json"), "old-catalog").unwrap();
    let profile = RelayProfile {
        id: "custom".into(),
        relay_mode: RelayMode::PureApi,
        no_auth: true,
        model_list: "custom-model".into(),
        context_window: "1000000".into(),
        standard_openai_protocol: true,
        config_contents: format!(
            "model='custom-model'\nmodel_provider='custom'\nmodel_catalog_json={:?}\n[model_providers.custom]\nbase_url='http://127.0.0.1:1/v1'\nwire_api='responses'\n",
            external.to_str().unwrap()
        ),
        ..RelayProfile::default()
    };
    assert!(apply_relay_profile_to_home_with_switch_rules(home, &profile, "").is_err());
    assert_eq!(
        std::fs::read_to_string(home.join("config.toml")).unwrap(),
        "model='old'\n"
    );
    assert_eq!(
        std::fs::read_to_string(home.join("model-catalogs/custom.json")).unwrap(),
        "old-catalog"
    );
    assert_eq!(std::fs::read_to_string(external).unwrap(), source);
}

#[test]
fn opted_in_preservation_only_fills_missing_project_and_notify() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.toml"), "notify=['old-command']\nmodel='old-model'\n[projects.'/旧项目']\ntrust_level='trusted'\n[projects.'/existing']\ntrust_level='trusted'\n").unwrap();
    let result = preserve_live_nonprovider_settings(
        dir.path(),
        "notify=[]\n[projects.'/existing']\ntrust_level='untrusted'\n",
    )
    .unwrap();
    let value: toml_edit::DocumentMut = result.parse().unwrap();
    assert_eq!(value["notify"].as_array().unwrap().len(), 0);
    assert_eq!(
        value["projects"]["/existing"]["trust_level"].as_str(),
        Some("untrusted")
    );
    assert_eq!(
        value["projects"]["/旧项目"]["trust_level"].as_str(),
        Some("trusted")
    );
    assert!(value.get("model").is_none());
}

#[test]
fn ccs_stable_source_rename_does_not_duplicate_or_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cc-switch.db");
    std::fs::write(&path, "artificial-source-identity").unwrap();
    let provider = CcsProviderImport {
        source_id: "stable-source".into(),
        name: "First".into(),
        base_url: "https://example.invalid/CasePath".into(),
        api_key: "artificial-key".into(),
        protocol: RelayProtocol::Responses,
        config_contents: String::new(),
        auth_contents: String::new(),
    };
    let mut source = CcsProviderSource {
        configured_db_path: String::new(),
        db_path: path,
        fallback_reason: None,
        providers: vec![provider],
    };
    let mut settings = BackendSettings::default();
    assert_eq!(
        import_providers_with_origins(&mut settings, &source, &BTreeMap::new()).unwrap(),
        (1, 0)
    );
    let original = settings.relay_profiles.last().unwrap().clone();
    source.providers[0].name = "Renamed".into();
    source.providers[0].api_key = "rotated-artificial-key".into();
    assert_eq!(
        import_providers_with_origins(&mut settings, &source, &BTreeMap::new()).unwrap(),
        (0, 0)
    );
    assert_eq!(settings.relay_profiles.last().unwrap(), &original);
    source.providers[0].source_id = "different-source".into();
    assert_eq!(
        import_providers_with_origins(&mut settings, &source, &BTreeMap::new()).unwrap(),
        (1, 0)
    );
}

#[test]
fn protocol_validation_distinguishes_transport_incomplete_and_complete() {
    for body in [
        "<html>homepage</html>",
        "",
        r#"{"error":{"message":"failed"}}"#,
        "{}",
    ] {
        assert_eq!(
            validate_relay_test_response(RelayProtocol::Responses, 200, body).0,
            false
        );
    }
    assert_eq!(
        validate_relay_test_response(
            RelayProtocol::Responses,
            200,
            r#"{"status":"completed","output":[]}"#
        ),
        (true, "completed".into())
    );
    assert_eq!(
        validate_relay_test_response(
            RelayProtocol::Responses,
            200,
            r#"{"status":"incomplete","output":[{"type":"reasoning"}]}"#
        ),
        (true, "incomplete".into())
    );
    // 被测试请求自身的 16 token 上限截断属正常（#2638）
    assert_eq!(
        validate_relay_test_response(
            RelayProtocol::Responses,
            200,
            r#"{"status":"incomplete","incomplete_details":{"reason":"max_output_tokens"},"output":[{"type":"reasoning"}]}"#
        ),
        (true, "completed".into())
    );
    assert_eq!(
        validate_relay_test_response(
            RelayProtocol::ChatCompletions,
            200,
            r#"{"choices":[{"message":{"content":"h"},"finish_reason":"length"}]}"#
        ),
        (true, "completed".into())
    );
    assert_eq!(
        validate_relay_test_response(
            RelayProtocol::AnthropicMessages,
            200,
            r#"{"type":"message","content":[],"stop_reason":"max_tokens"}"#
        ),
        (true, "completed".into())
    );
    assert_eq!(
        validate_relay_test_response(
            RelayProtocol::ChatCompletions,
            200,
            r#"{"choices":[{"message":{"content":""},"finish_reason":"content_filter"}]}"#
        ),
        (true, "incomplete".into())
    );
    assert!(
        validate_relay_test_response(
            RelayProtocol::ChatCompletions,
            200,
            r#"{"choices":[{"message":{"tool_calls":[]},"finish_reason":"tool_calls"}]}"#
        )
        .0
    );
    assert!(
        validate_relay_test_response(
            RelayProtocol::AnthropicMessages,
            200,
            r#"{"type":"message","content":[],"stop_reason":"end_turn"}"#
        )
        .0
    );
}

#[tokio::test]
async fn relay_test_preserves_complete_endpoint_and_explicit_user_agent() {
    let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = server.local_addr().unwrap();
    let thread = std::thread::spawn(move || {
        let (mut stream, _) = server.accept().unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let mut bytes = Vec::new();
        let mut chunk = [0; 4096];
        while !bytes.windows(4).any(|v| v == b"\r\n\r\n") {
            let n = stream.read(&mut chunk).unwrap();
            if n == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..n]);
        }
        let header_end = bytes.windows(4).position(|v| v == b"\r\n\r\n").unwrap() + 4;
        let length = String::from_utf8_lossy(&bytes[..header_end])
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .map(|value| value.trim().parse::<usize>().unwrap())
            })
            .unwrap_or(0);
        while bytes.len() < header_end + length {
            let n = stream.read(&mut chunk).unwrap();
            if n == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..n]);
        }
        let body = r#"{"status":"completed","output":[]}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .unwrap();
        String::from_utf8(bytes).unwrap()
    });
    let endpoint = format!("http://{address}/api/v3/responses");
    let profile = RelayProfile {
        relay_mode: RelayMode::PureApi,
        upstream_base_url: endpoint.clone(),
        base_url: endpoint.clone(),
        no_auth: true,
        user_agent: "ArtificialExplicitUA/1".into(),
        ..RelayProfile::default()
    };
    let result = test_relay_profile(&profile, "artificial-model")
        .await
        .unwrap();
    assert_eq!(result.endpoint, endpoint);
    assert!(result.protocol_valid, "{result:?}");
    let captured = thread.join().unwrap();
    assert!(captured.starts_with("POST /api/v3/responses HTTP/1.1"));
    assert!(
        captured
            .to_ascii_lowercase()
            .contains("user-agent: artificialexplicitua/1")
    );
}
