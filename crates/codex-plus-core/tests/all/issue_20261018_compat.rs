use base64::Engine as _;
use codex_plus_core::protocol_proxy::open_responses_proxy_request_with_settings;
use codex_plus_core::relay_config::apply_relay_profile_to_home_with_switch_rules;
use codex_plus_core::settings::{BackendSettings, RelayMode, RelayProfile, RelayProtocol};
use serde_json::{Value, json};
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

fn settings(url: &str, protocol: RelayProtocol, mode: &str) -> BackendSettings {
    BackendSettings {
        relay_profiles_enabled: true,
        active_relay_id: "compat-fixture".into(),
        relay_profiles: vec![RelayProfile {
            id: "compat-fixture".into(),
            relay_mode: RelayMode::PureApi,
            base_url: format!("{url}/v1"),
            api_key: "fixture-only".into(),
            protocol,
            model_metadata: json!({"private-alias": {
                "codex_plus_reasoning_parameters": mode,
                "supported_reasoning_levels": [{"effort":"high","description":"fixture"}]
            }})
            .to_string(),
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[tokio::test]
async fn reasoning_policy_controls_real_outgoing_fields_and_does_not_change_other_models() {
    for protocol in [
        RelayProtocol::Responses,
        RelayProtocol::ChatCompletions,
        RelayProtocol::AnthropicMessages,
    ] {
        for mode in ["auto", "omit", "passthrough"] {
            let upstream = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(json!({"id":"fixture","status":"completed","output":[]})),
                )
                .mount(&upstream)
                .await;
            let config = settings(&upstream.uri(), protocol, mode);
            let request = json!({"model":"private-alias","input":[{"role":"user","content":"hello"}],"reasoning":{"effort":"high"}});
            open_responses_proxy_request_with_settings(&request.to_string(), config.clone())
                .await
                .unwrap();
            let outgoing: Value = upstream
                .received_requests()
                .await
                .unwrap()
                .last()
                .unwrap()
                .body_json()
                .unwrap();
            if mode == "omit" {
                for key in [
                    "reasoning",
                    "reasoning_effort",
                    "thinking",
                    "enable_thinking",
                ] {
                    assert!(outgoing.get(key).is_none(), "{protocol:?} {key}");
                }
            } else if mode == "passthrough" && protocol == RelayProtocol::ChatCompletions {
                assert_eq!(outgoing["reasoning_effort"], "high");
            }
            let mut another = request;
            another["model"] = json!("gpt-5.4");
            open_responses_proxy_request_with_settings(&another.to_string(), config.clone())
                .await
                .unwrap();
            let outgoing: Value = upstream
                .received_requests()
                .await
                .unwrap()
                .last()
                .unwrap()
                .body_json()
                .unwrap();
            let mut baseline = config;
            baseline.relay_profiles[0].model_metadata.clear();
            open_responses_proxy_request_with_settings(&another.to_string(), baseline)
                .await
                .unwrap();
            let expected: Value = upstream
                .received_requests()
                .await
                .unwrap()
                .last()
                .unwrap()
                .body_json()
                .unwrap();
            assert_eq!(outgoing, expected, "另一模型不应受到 {mode} 影响");
        }
    }
}

#[tokio::test]
async fn strict_history_is_opt_in_and_never_discards_unrecognized_ciphertext() {
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"id":"fixture","status":"completed","output":[]})),
        )
        .mount(&upstream)
        .await;
    let mut config = settings(&upstream.uri(), RelayProtocol::Responses, "auto");
    let original = json!({"model":"private-alias","input":[{
        "type":"reasoning","content":[{"type":"reasoning_text","text":"private-sentinel"}],
        "encrypted_content":"placeholder-uuid-0","summary":[]
    }]});
    open_responses_proxy_request_with_settings(&original.to_string(), config.clone())
        .await
        .unwrap();
    let outgoing: Value = upstream.received_requests().await.unwrap()[0]
        .body_json()
        .unwrap();
    assert_eq!(outgoing["input"], original["input"]);
    config.relay_profiles[0].strict_responses_reasoning = true;
    let error = open_responses_proxy_request_with_settings(&original.to_string(), config.clone())
        .await
        .err()
        .expect("占位密文必须拒绝")
        .to_string();
    assert!(error.contains("incompatible_reasoning_history"));
    assert!(!error.contains("private-sentinel"));
    assert_eq!(upstream.received_requests().await.unwrap().len(), 1);

    // 人工信封只验证格式投影；账号认证/解密仍必须由真实上游完成。
    let mut bytes = vec![0; 73];
    bytes[0] = 0x80;
    let cipher = base64::engine::general_purpose::URL_SAFE.encode(bytes);
    let mut recognized = original.clone();
    recognized["input"][0]["encrypted_content"] = json!(cipher);
    open_responses_proxy_request_with_settings(&recognized.to_string(), config)
        .await
        .unwrap();
    let outgoing: Value = upstream
        .received_requests()
        .await
        .unwrap()
        .last()
        .unwrap()
        .body_json()
        .unwrap();
    assert_eq!(outgoing["input"][0]["content"], json!([]));
    assert_eq!(outgoing["input"][0]["encrypted_content"], cipher);
    assert_eq!(
        recognized["input"][0]["content"],
        original["input"][0]["content"]
    );
}

#[test]
fn compatibility_profiles_route_through_helper_and_omit_levels_only_in_projection() {
    let temp = tempfile::tempdir().unwrap();
    let mut profile = settings("https://upstream.test", RelayProtocol::Responses, "omit")
        .relay_profiles
        .remove(0);
    profile.model = "private-alias".into();
    profile.model_list = "private-alias[1M]".into();
    assert!(profile.request_compatibility_enabled());
    apply_relay_profile_to_home_with_switch_rules(temp.path(), &profile, "").unwrap();
    let text = std::fs::read_to_string(temp.path().join("config.toml")).unwrap();
    assert!(text.contains("127.0.0.1:57321"));
    let doc: toml_edit::DocumentMut = text.parse().unwrap();
    let catalog: Value = serde_json::from_str(
        &std::fs::read_to_string(doc["model_catalog_json"].as_str().unwrap()).unwrap(),
    )
    .unwrap();
    assert_eq!(
        catalog["models"][0]["supported_reasoning_levels"],
        json!([])
    );
    assert_eq!(
        serde_json::from_str::<Value>(&profile.model_metadata).unwrap()["private-alias"]["supported_reasoning_levels"]
            [0]["effort"],
        "high"
    );
    let saved = serde_json::to_value(&profile).unwrap();
    assert!(saved.get("strictResponsesReasoning").is_none());
    assert!(saved.get("trustUpstreamCatalogLite").is_none());
}

#[test]
fn explicit_catalog_trust_preserves_lite_without_enabling_other_models() {
    for trusted in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let external = temp.path().join("external.json");
        let original = json!({"models":[
            {"slug":"one","base_instructions":"","supports_parallel_tool_calls":false,"use_responses_lite":true},
            {"slug":"two","base_instructions":"","supports_parallel_tool_calls":false,"use_responses_lite":false}
        ]}).to_string();
        std::fs::write(&external, &original).unwrap();
        let mut doc = toml_edit::DocumentMut::new();
        doc["model_catalog_json"] = toml_edit::value(external.to_string_lossy().as_ref());
        let profile = RelayProfile {
            id: "lite-fixture".into(),
            relay_mode: RelayMode::PureApi,
            base_url: "https://upstream.test/v1".into(),
            api_key: "fixture-only".into(),
            model: "one".into(),
            model_list: "one\ntwo".into(),
            context_window: "1000000".into(),
            config_contents: doc.to_string(),
            trust_upstream_catalog_lite: trusted,
            ..Default::default()
        };
        apply_relay_profile_to_home_with_switch_rules(temp.path(), &profile, "").unwrap();
        let config: toml_edit::DocumentMut =
            std::fs::read_to_string(temp.path().join("config.toml"))
                .unwrap()
                .parse()
                .unwrap();
        let catalog: Value = serde_json::from_str(
            &std::fs::read_to_string(config["model_catalog_json"].as_str().unwrap()).unwrap(),
        )
        .unwrap();
        assert_eq!(catalog["models"][0]["use_responses_lite"], trusted);
        assert_eq!(catalog["models"][1]["use_responses_lite"], false);
        assert_eq!(std::fs::read_to_string(&external).unwrap(), original);
    }
}

#[test]
fn mcp_path_inspection_only_suggests_verified_cli_files_and_preserves_configuration() {
    use codex_plus_core::mcp_config::{McpServerForm, inspect_mcp_startup};
    let temp = tempfile::tempdir().unwrap();
    let cli = temp.path().join("codex.exe");
    std::fs::write(&cli, b"fixture is deliberately not an executable").unwrap();
    let mut form = McpServerForm {
        command: r"C:\Program Files\WindowsApps\OpenAI.Codex_26.1007_x64__2p2nqsd0c76g0\app\resources\codex.exe".into(),
        args: vec!["mcp-server".into()],
        extra_toml: "custom = 'kept'\n".into(),
        ..Default::default()
    };
    let original = form.clone();
    let result = inspect_mcp_startup(&form, Some(&cli));
    assert_eq!(result.code, "replacement_available");
    assert_eq!(result.replacement_command.as_deref(), cli.to_str());
    assert_eq!(form, original);
    for unknown in [
        original
            .command
            .replace("OpenAI.Codex_", "OtherPublisher.Codex_"),
        original.command.replace("2p2nqsd0c76g0", "otherpublisher"),
        original.command.replace(
            r"app\resources\codex.exe",
            r"app\resources\..\other\codex.exe",
        ),
    ] {
        let untrusted = McpServerForm {
            command: unknown,
            ..original.clone()
        };
        assert!(
            inspect_mcp_startup(&untrusted, Some(&cli))
                .replacement_command
                .is_none()
        );
    }
    assert_eq!(
        std::fs::read(&cli).unwrap(),
        b"fixture is deliberately not an executable"
    );
    form.command =
        r"C:\Program Files\WindowsApps\OpenAI.Codex_26.1007_x64__publisher\app\ChatGPT.exe".into();
    form.args.clear();
    let gui = inspect_mcp_startup(&form, Some(&cli));
    assert_eq!(gui.code, "desktop_executable");
    assert!(gui.replacement_command.is_none());
    form.command = "node".into();
    assert_eq!(inspect_mcp_startup(&form, Some(&cli)).code, "unverified");
    form.command = cli.with_file_name("missing").to_string_lossy().into();
    assert_eq!(inspect_mcp_startup(&form, None).code, "missing");
}

#[test]
fn disabling_new_flags_does_not_resurrect_true_from_preserved_unknown_fields() {
    use codex_plus_core::settings::SettingsStore;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    let store = SettingsStore::new(path.clone());
    let mut initial = settings("https://upstream.test", RelayProtocol::Responses, "auto");
    initial.relay_profiles[0].trust_upstream_catalog_lite = true;
    initial.relay_profiles[0].strict_responses_reasoning = true;
    store.save(&initial).unwrap();
    let mut current = store.load().unwrap();
    assert!(current.relay_profiles[0].strict_responses_reasoning);
    current.relay_profiles[0].strict_responses_reasoning = false;
    current.relay_profiles[0].trust_upstream_catalog_lite = false;
    store.save(&current).unwrap();
    let reloaded = store.load().unwrap();
    assert!(!reloaded.relay_profiles[0].strict_responses_reasoning);
    assert!(!reloaded.relay_profiles[0].trust_upstream_catalog_lite);
    let disk: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert!(
        disk["relayProfiles"][0]
            .get("strictResponsesReasoning")
            .is_none()
    );
    assert!(
        disk["relayProfiles"][0]
            .get("trustUpstreamCatalogLite")
            .is_none()
    );
}
