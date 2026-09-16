use std::path::Path;

use codex_plus_core::protocol_proxy::{local_responses_proxy_base_url, protocol_proxy_port};
use codex_plus_core::relay_config::{
    RelayApplyResult, apply_relay_profile_config_to_home_with_context,
    apply_relay_profile_files_to_home_with_context, apply_relay_profile_to_home_with_switch_rules,
    clear_relay_config_to_home_with_auth,
};
use codex_plus_core::settings::{RelayMode, RelayModelRoute, RelayProfile, RelayProtocol};
use serde_json::{Value, json};

type ApplyProfile = fn(&Path, &RelayProfile, &str) -> anyhow::Result<RelayApplyResult>;

const APPLY_PATHS: [(&str, ApplyProfile); 3] = [
    ("files", apply_relay_profile_files_to_home_with_context),
    ("switch", apply_relay_profile_to_home_with_switch_rules),
    ("config", apply_relay_profile_config_to_home_with_context),
];
const API_MODES: [RelayMode; 3] = [RelayMode::PureApi, RelayMode::MixedApi, RelayMode::Official];
const OFFICIAL_AUTH: &str =
    r#"{"auth_mode":"chatgpt","tokens":{"access_token":"test-official-token"}}"#;

fn profile(mode: RelayMode, identity: &str) -> RelayProfile {
    let transport = if identity == "openai" {
        "custom"
    } else {
        identity
    };
    RelayProfile {
        id: "identity-catalog".to_string(),
        relay_mode: mode,
        official_mix_api_key: mode == RelayMode::Official,
        protocol: RelayProtocol::Responses,
        model: "gpt-5.6-sol".to_string(),
        model_list: "gpt-5.6-sol,gpt-5.6-terra".to_string(),
        base_url: "https://relay.example/v1".to_string(),
        api_key: "test-api-key".to_string(),
        config_contents: format!(
            r#"model_provider = "{identity}"
[model_providers.{transport}]
wire_api = "responses"
base_url = "https://relay.example/v1"
requires_openai_auth = true
"#
        ),
        auth_contents: if mode == RelayMode::PureApi {
            r#"{"OPENAI_API_KEY":"test-api-key"}"#.to_string()
        } else {
            OFFICIAL_AUTH.to_string()
        },
        ..RelayProfile::default()
    }
}

fn config(home: &Path) -> toml::Value {
    toml::from_str(&std::fs::read_to_string(home.join("config.toml")).unwrap()).unwrap()
}

fn catalog(home: &Path) -> Value {
    let config = config(home);
    let path = config["model_catalog_json"].as_str().unwrap();
    serde_json::from_slice(&std::fs::read(home.join(path)).unwrap()).unwrap()
}

fn assert_auth_and_identity(home: &Path, profile: &RelayProfile, identity: &str) {
    let config = config(home);
    assert_eq!(config["model_provider"].as_str(), Some(identity));
    let transport = if identity == "openai" {
        "custom"
    } else {
        identity
    };
    let provider = &config["model_providers"][transport];
    assert_eq!(provider["wire_api"].as_str(), Some("responses"));
    assert_eq!(
        provider["base_url"].as_str(),
        Some("https://relay.example/v1")
    );
    let auth: Value =
        serde_json::from_slice(&std::fs::read(home.join("auth.json")).unwrap()).unwrap();
    assert_eq!(
        auth,
        serde_json::from_str::<Value>(&profile.auth_contents).unwrap()
    );
    if profile.relay_mode == RelayMode::PureApi {
        assert!(provider.get("experimental_bearer_token").is_none());
    } else {
        assert_eq!(
            provider["experimental_bearer_token"].as_str(),
            Some("test-api-key")
        );
    }
}

#[test]
fn managed_responses_defaults_are_independent_of_session_identity() {
    for mode in API_MODES {
        for identity in ["custom", "openai", "vendor"] {
            for (path_name, apply) in APPLY_PATHS {
                let temp = tempfile::tempdir().unwrap();
                let profile = profile(mode, identity);
                std::fs::write(temp.path().join("auth.json"), &profile.auth_contents).unwrap();
                apply(temp.path(), &profile, "").unwrap();
                let catalog = catalog(temp.path());
                assert_eq!(catalog["models"].as_array().unwrap().len(), 2);
                for model in catalog["models"].as_array().unwrap() {
                    assert_eq!(
                        model["use_responses_lite"], false,
                        "{mode:?}, {identity}, {path_name}"
                    );
                    assert_eq!(model["supports_search_tool"], true);
                    assert_eq!(model["web_search_tool_type"], "text_and_image");
                    assert_eq!(model["tool_mode"], "code_mode_only");
                    assert_eq!(model["multi_agent_version"], "v2");
                    assert_eq!(model["context_window"], 272_000);
                    assert_eq!(model["max_context_window"], 872_000);
                }
                assert_auth_and_identity(temp.path(), &profile, identity);
            }
        }
    }
}

#[test]
fn managed_responses_copy_external_catalogs_for_both_identities() {
    let original = json!({
        "models": [{
            "slug": "gpt-5.6-sol",
            "context_window": 272000,
            "supports_search_tool": true,
            "web_search_tool_type": "text_and_image",
            "tool_mode": "code_mode_only",
            "use_responses_lite": true,
            "vendor_extension": {"preserve": true}
        }]
    });
    let original_bytes = serde_json::to_vec_pretty(&original).unwrap();
    let mut expected = original.clone();
    expected["models"][0]["use_responses_lite"] = json!(false);
    for mode in API_MODES {
        for identity in ["custom", "openai"] {
            for (path_name, apply) in APPLY_PATHS {
                for live_pointer in [false, true] {
                    let temp = tempfile::tempdir().unwrap();
                    let external = temp.path().join("external.json");
                    std::fs::write(&external, &original_bytes).unwrap();
                    let pointer = format!("model_catalog_json = '{}'\n", external.display());
                    let mut profile = profile(mode, identity);
                    if live_pointer {
                        std::fs::write(temp.path().join("config.toml"), &pointer).unwrap();
                    } else {
                        profile.config_contents = pointer + &profile.config_contents;
                    }
                    std::fs::write(temp.path().join("auth.json"), &profile.auth_contents).unwrap();
                    apply(temp.path(), &profile, "").unwrap();
                    assert_eq!(
                        config(temp.path())["model_catalog_json"].as_str(),
                        Some("model-catalogs/identity-catalog.json"),
                        "{mode:?}, {identity}, {path_name}, live={live_pointer}"
                    );
                    assert_eq!(catalog(temp.path()), expected);
                    assert_eq!(std::fs::read(&external).unwrap(), original_bytes);
                    assert_auth_and_identity(temp.path(), &profile, identity);
                }
            }
        }
    }
}

#[test]
fn explicit_lite_metadata_still_overrides_managed_defaults() {
    for mode in API_MODES {
        for identity in ["custom", "openai"] {
            for (_, apply) in APPLY_PATHS {
                let temp = tempfile::tempdir().unwrap();
                let mut profile = profile(mode, identity);
                profile.model_metadata =
                    json!({"gpt-5.6-sol": {"use_responses_lite": true}}).to_string();
                std::fs::write(temp.path().join("auth.json"), &profile.auth_contents).unwrap();
                apply(temp.path(), &profile, "").unwrap();
                let catalog = catalog(temp.path());
                let models = catalog["models"].as_array().unwrap();
                let sol = models.iter().find(|m| m["slug"] == "gpt-5.6-sol").unwrap();
                let terra = models
                    .iter()
                    .find(|m| m["slug"] == "gpt-5.6-terra")
                    .unwrap();
                assert_eq!(sol["use_responses_lite"], true);
                assert_eq!(terra["use_responses_lite"], false);
            }
        }
    }
}

#[test]
fn native_legacy_and_aggregate_catalog_defaults_are_not_broadened() {
    for (_, apply) in APPLY_PATHS {
        for (mode, identity, expected_lite) in [
            (RelayMode::Official, "openai", true),
            (RelayMode::Official, "custom", false),
            (RelayMode::Aggregate, "openai", true),
            (RelayMode::Aggregate, "custom", false),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let mut profile = profile(mode, identity);
            profile.official_mix_api_key = false;
            std::fs::write(temp.path().join("auth.json"), OFFICIAL_AUTH).unwrap();
            apply(temp.path(), &profile, "").unwrap();
            for model in catalog(temp.path())["models"].as_array().unwrap() {
                assert_eq!(model["use_responses_lite"], expected_lite);
            }
        }
    }
}

#[test]
fn responses_chat_responses_roundtrip_uses_real_upstream_protocol() {
    for mode in API_MODES {
        for (_, apply) in APPLY_PATHS {
            let temp = tempfile::tempdir().unwrap();
            let mut profile = profile(mode, "custom");
            std::fs::write(temp.path().join("auth.json"), &profile.auth_contents).unwrap();
            for protocol in [
                RelayProtocol::Responses,
                RelayProtocol::ChatCompletions,
                RelayProtocol::Responses,
            ] {
                profile.protocol = protocol;
                apply(temp.path(), &profile, "").unwrap();
                let config = config(temp.path());
                let provider = &config["model_providers"]["custom"];
                assert_eq!(provider["wire_api"].as_str(), Some("responses"));
                let expected_url = if protocol == RelayProtocol::ChatCompletions {
                    local_responses_proxy_base_url(protocol_proxy_port())
                } else {
                    "https://relay.example/v1".to_string()
                };
                assert_eq!(provider["base_url"].as_str(), Some(expected_url.as_str()));
                for model in catalog(temp.path())["models"].as_array().unwrap() {
                    assert_eq!(
                        model["use_responses_lite"],
                        protocol == RelayProtocol::ChatCompletions
                    );
                }
            }
        }
    }
}

#[test]
fn routed_unbundled_models_get_catalogs_for_managed_responses_only() {
    for mode in API_MODES {
        for identity in ["custom", "openai", "vendor"] {
            for (_, apply) in APPLY_PATHS {
                for routed in [false, true] {
                    let temp = tempfile::tempdir().unwrap();
                    let mut profile = profile(mode, identity);
                    profile.model = "qwen3-coder".to_string();
                    profile.model_list = profile.model.clone();
                    if routed {
                        profile.model_routes = vec![RelayModelRoute {
                            model: profile.model.clone(),
                            target_relay_id: "target".to_string(),
                            target_model: "target-model".to_string(),
                        }];
                    }
                    std::fs::write(temp.path().join("auth.json"), &profile.auth_contents).unwrap();
                    apply(temp.path(), &profile, "").unwrap();
                    let config = config(temp.path());
                    assert_eq!(config["model_provider"].as_str(), Some(identity));
                    assert_eq!(config.get("model_catalog_json").is_some(), routed);
                    if routed {
                        let catalog = catalog(temp.path());
                        let models = catalog["models"].as_array().unwrap();
                        assert_eq!(models.len(), 1);
                        assert_eq!(models[0]["slug"], "qwen3-coder");
                        assert_eq!(models[0]["use_responses_lite"], false);
                        assert_eq!(models[0]["multi_agent_version"], "v2");
                    } else {
                        assert!(!temp.path().join("model-catalogs").exists());
                    }
                }
            }
        }
    }
}

#[test]
fn explicit_windows_keep_upstream_clamp_and_metadata_precedence() {
    for identity in ["custom", "openai"] {
        let temp = tempfile::tempdir().unwrap();
        let mut profile = profile(RelayMode::PureApi, identity);
        profile.model_windows = json!({"gpt-5.6-sol": "400000"}).to_string();
        profile.model_metadata = json!({
            "gpt-5.6-sol": {
                "max_context_window": 1234,
                "multi_agent_version": "custom-contract",
                "use_responses_lite": true
            }
        })
        .to_string();
        apply_relay_profile_files_to_home_with_context(temp.path(), &profile, "").unwrap();
        let catalog = catalog(temp.path());
        let sol = catalog["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|model| model["slug"] == "gpt-5.6-sol")
            .unwrap();
        assert_eq!(sol["context_window"], 400_000);
        assert_eq!(sol["max_context_window"], 400_000);
        assert_eq!(sol["multi_agent_version"], "custom-contract");
        assert_eq!(sol["use_responses_lite"], true);
    }
}

#[test]
fn openai_chat_rejection_leaves_live_files_unchanged() {
    for mode in API_MODES {
        for (_, apply) in APPLY_PATHS {
            let temp = tempfile::tempdir().unwrap();
            let original_config = b"model = 'untouched'\n";
            std::fs::write(temp.path().join("config.toml"), original_config).unwrap();
            std::fs::write(temp.path().join("auth.json"), OFFICIAL_AUTH).unwrap();
            let mut profile = profile(mode, "openai");
            profile.protocol = RelayProtocol::ChatCompletions;
            let error = apply(temp.path(), &profile, "").unwrap_err();
            assert!(error.to_string().contains("Responses API"));
            assert_eq!(
                std::fs::read(temp.path().join("config.toml")).unwrap(),
                original_config
            );
            assert_eq!(
                std::fs::read(temp.path().join("auth.json")).unwrap(),
                OFFICIAL_AUTH.as_bytes()
            );
            assert!(!temp.path().join("model-catalogs").exists());
        }
    }
}

#[test]
fn api_to_official_roundtrip_removes_managed_catalog_pointer() {
    for identity in ["custom", "openai"] {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("auth.json"), OFFICIAL_AUTH).unwrap();
        std::fs::write(
            temp.path().join("config.toml"),
            "model = 'gpt-5.6-sol'\n[features]\nunified_exec = true\n",
        )
        .unwrap();
        let mut profile = profile(RelayMode::PureApi, identity);
        profile.config_contents += "\n[features]\nunified_exec = true\n";
        apply_relay_profile_to_home_with_switch_rules(temp.path(), &profile, "").unwrap();
        assert!(config(temp.path()).get("model_catalog_json").is_some());
        clear_relay_config_to_home_with_auth(temp.path(), Some(OFFICIAL_AUTH)).unwrap();
        let config = config(temp.path());
        assert!(config.get("model_catalog_json").is_none());
        assert!(config.get("model_provider").is_none());
        if let Some(provider) = config
            .get("model_providers")
            .and_then(|providers| providers.get("custom"))
        {
            for key in [
                "experimental_bearer_token",
                "env_key",
                "requires_openai_auth",
            ] {
                assert!(provider.get(key).is_none());
            }
        }
        assert_eq!(config["features"]["unified_exec"].as_bool(), Some(true));
        let auth: Value =
            serde_json::from_slice(&std::fs::read(temp.path().join("auth.json")).unwrap()).unwrap();
        assert_eq!(auth, serde_json::from_str::<Value>(OFFICIAL_AUTH).unwrap());
    }
}
