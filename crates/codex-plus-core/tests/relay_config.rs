use codex_plus_core::relay_config::{
    LOCAL_RELAY_PROXY_PORT, RelayApplyOptions, apply_pure_api_config_to_home,
    apply_relay_config_to_home, chatgpt_auth_status_from_home, clear_relay_config_to_home,
    relay_config_status_from_home,
};

#[test]
fn detects_chatgpt_login_from_auth_json_and_config_provider() {
    let temp = tempfile::tempdir().unwrap();
    let id_token = format!(
        "header.{}.signature",
        base64_url_no_pad(r#"{"email":"user@example.test","name":"Codex User"}"#)
    );
    std::fs::write(
        temp.path().join("auth.json"),
        format!(
            r#"{{"auth_mode":"chatgpt","tokens":{{"id_token":"{id_token}","access_token":"access-token","refresh_token":"refresh-token"}}}}"#
        ),
    )
    .unwrap();
    std::fs::write(
        temp.path().join("config.toml"),
        r#"model_provider = "chatgpt"
"#,
    )
    .unwrap();

    let status = chatgpt_auth_status_from_home(temp.path());

    assert!(status.authenticated);
    assert!(status.source.contains("auth.json"));
    assert_eq!(status.account_label.as_deref(), Some("user@example.test"));
}

#[test]
fn detects_chatgpt_login_when_config_exists_without_model_provider() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("auth.json"),
        r#"{"auth_mode":"chatgpt","tokens":{"access_token":"access-token"}}"#,
    )
    .unwrap();
    std::fs::write(temp.path().join("config.toml"), r#"model = "gpt-5""#).unwrap();

    let status = chatgpt_auth_status_from_home(temp.path());

    assert!(status.authenticated);
    assert!(status.source.contains("auth.json"));
}

#[test]
fn rejects_auth_json_tokens_without_chatgpt_auth_mode() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("auth.json"),
        r#"{"auth_mode":"apikey","tokens":{"access_token":"access-token"}}"#,
    )
    .unwrap();
    std::fs::write(
        temp.path().join("config.toml"),
        r#"model_provider = "chatgpt""#,
    )
    .unwrap();

    let status = chatgpt_auth_status_from_home(temp.path());

    assert!(!status.authenticated);
}

#[test]
fn detects_chatgpt_login_from_auth_json_without_config_toml() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("auth.json"),
        r#"{"auth_mode":"chatgpt","tokens":{"access_token":"access-token"}}"#,
    )
    .unwrap();

    let status = chatgpt_auth_status_from_home(temp.path());

    assert!(status.authenticated);
    assert!(status.source.contains("auth.json"));
}

#[test]
fn reports_relay_configured_when_required_keys_exist() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("config.toml"),
        r#"model = "gpt-5"
model_provider = "CodexPlusPlus"
OPENAI_API_KEY = "sk-should-be-removed"
[model_providers.CodexPlusPlus]
name = "CodexPlusPlus"
wire_api = "responses"
requires_openai_auth = true
base_url = "http://192.168.188.245:3001/v1"
experimental_bearer_token = "sk-test-redacted"
"#,
    )
    .unwrap();

    let status = relay_config_status_from_home(temp.path());

    assert!(status.configured);
    assert!(status.requires_openai_auth);
    assert!(status.has_bearer_token);
}

#[test]
fn apply_relay_config_updates_provider_table_and_preserves_other_config() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("config.toml"),
        r#"model = "gpt-5"
model_provider = "custom1"
[model_providers.custom1]
name = "custom1"
wire_api = "responses"
requires_openai_auth = true
base_url = "http://192.168.188.245:3001/v1"
[profiles.default]
model = "gpt-5-mini"
"#,
    )
    .unwrap();

    let result = apply_relay_config_to_home(
        temp.path(),
        "https://relay.example.test/v1",
        "sk-test-redacted",
    )
    .unwrap();
    let updated = std::fs::read_to_string(temp.path().join("config.toml")).unwrap();

    assert!(result.configured);
    assert!(updated.contains(r#"model = "gpt-5""#));
    assert!(updated.contains(r#"model_provider = "CodexPlusPlus""#));
    assert!(updated.contains("[model_providers.CodexPlusPlus]"));
    assert!(updated.contains(r#"name = "CodexPlusPlus""#));
    assert!(updated.contains(r#"wire_api = "responses""#));
    assert!(updated.contains("requires_openai_auth = true"));
    assert!(updated.contains(&format!(
        r#"base_url = "http://127.0.0.1:{LOCAL_RELAY_PROXY_PORT}/v1""#
    )));
    assert!(updated.contains(r#"codex_plus_text_base_url = "https://relay.example.test/v1""#));
    assert!(updated.contains(r#"disabled_tools = ["image_generation"]"#));
    assert!(updated.contains(r#"experimental_bearer_token = "sk-test-redacted""#));
    assert!(updated.contains("[profiles.default]"));
}

#[test]
fn apply_relay_config_normalizes_root_base_url_to_v1() {
    let temp = tempfile::tempdir().unwrap();

    apply_relay_config_to_home(
        temp.path(),
        "https://api.86gamestore.com",
        "sk-test-redacted",
    )
    .unwrap();
    let updated = std::fs::read_to_string(temp.path().join("config.toml")).unwrap();

    assert!(updated.contains(&format!(
        r#"base_url = "http://127.0.0.1:{LOCAL_RELAY_PROXY_PORT}/v1""#
    )));
    assert!(updated.contains(r#"codex_plus_text_base_url = "https://api.86gamestore.com/v1""#));
    assert!(updated.contains(r#"disabled_tools = ["image_generation"]"#));
}

#[test]
fn apply_relay_config_preserves_custom_base_url_path() {
    let temp = tempfile::tempdir().unwrap();

    apply_relay_config_to_home(
        temp.path(),
        "https://relay.example.test/openai",
        "sk-test-redacted",
    )
    .unwrap();
    let updated = std::fs::read_to_string(temp.path().join("config.toml")).unwrap();

    assert!(updated.contains(&format!(
        r#"base_url = "http://127.0.0.1:{LOCAL_RELAY_PROXY_PORT}/v1""#
    )));
    assert!(updated.contains(r#"codex_plus_text_base_url = "https://relay.example.test/openai""#));
}

#[test]
fn apply_relay_config_allows_image_generation_on_primary_relay() {
    let temp = tempfile::tempdir().unwrap();
    let mut options = RelayApplyOptions::new("https://relay.example.test/v1", "sk-test-redacted");
    options.image_generation_enabled = true;

    codex_plus_core::relay_config::apply_relay_config_to_home_with_options(temp.path(), &options)
        .unwrap();
    let updated = std::fs::read_to_string(temp.path().join("config.toml")).unwrap();

    assert!(updated.contains(r#"base_url = "https://relay.example.test/v1""#));
    assert!(!updated.contains(r#"disabled_tools = ["image_generation"]"#));
}

#[test]
fn apply_relay_config_routes_separate_image_generation_api_through_local_proxy() {
    let temp = tempfile::tempdir().unwrap();
    let mut options = RelayApplyOptions::new("https://relay.example.test/v1", "sk-test-redacted");
    options.image_generation_enabled = true;
    options.image_generation_use_separate_api = true;
    options.image_generation_base_url = "https://image.example.test".to_string();
    options.image_generation_bearer_token = "sk-image-redacted".to_string();

    codex_plus_core::relay_config::apply_relay_config_to_home_with_options(temp.path(), &options)
        .unwrap();
    let updated = std::fs::read_to_string(temp.path().join("config.toml")).unwrap();

    assert!(updated.contains(&format!(
        r#"base_url = "http://127.0.0.1:{LOCAL_RELAY_PROXY_PORT}/v1""#
    )));
    assert!(updated.contains(r#"codex_plus_text_base_url = "https://relay.example.test/v1""#));
    assert!(updated.contains(r#"codex_plus_image_base_url = "https://image.example.test/v1""#));
    assert!(!updated.contains(r#"disabled_tools = ["image_generation"]"#));
    assert!(updated.contains(r#"experimental_bearer_token = "sk-test-redacted""#));
    assert!(!updated.contains("sk-image-redacted"));
}

#[test]
fn apply_relay_config_rejects_incomplete_separate_image_generation_api() {
    let temp = tempfile::tempdir().unwrap();
    let mut options = RelayApplyOptions::new("https://relay.example.test/v1", "sk-test-redacted");
    options.image_generation_enabled = true;
    options.image_generation_use_separate_api = true;
    options.image_generation_bearer_token = "sk-image-redacted".to_string();

    let err = codex_plus_core::relay_config::apply_relay_config_to_home_with_options(
        temp.path(),
        &options,
    )
    .unwrap_err();

    assert!(err.to_string().contains("图片 Base URL 不能为空"));
    assert!(!temp.path().join("config.toml").exists());
}

#[test]
fn apply_pure_api_config_writes_openai_api_key_auth_json_and_provider() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("auth.json"),
        r#"{"auth_mode":"chatgpt","tokens":{"access_token":"old"}}"#,
    )
    .unwrap();
    std::fs::write(temp.path().join("config.toml"), r#"model = "gpt-5""#).unwrap();

    let result = apply_pure_api_config_to_home(
        temp.path(),
        "http://192.168.188.245:3001/v1",
        "sk-test-redacted",
    )
    .unwrap();

    let auth: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(temp.path().join("auth.json")).unwrap())
            .unwrap();
    let config = std::fs::read_to_string(temp.path().join("config.toml")).unwrap();
    assert!(result.configured);
    assert_eq!(
        auth,
        serde_json::json!({"OPENAI_API_KEY": "sk-test-redacted"})
    );
    assert!(config.contains(r#"model_provider = "CodexPlusPlus""#));
    assert!(config.contains("[model_providers.CodexPlusPlus]"));
    assert!(config.contains(r#"name = "CodexPlusPlus""#));
    assert!(config.contains(r#"wire_api = "responses""#));
    assert!(config.contains("requires_openai_auth = true"));
    assert!(config.contains(r#"base_url = "http://192.168.188.245:3001/v1""#));
    assert!(config.contains(r#"experimental_bearer_token = "sk-test-redacted""#));
}

#[test]
fn apply_relay_config_points_model_provider_to_codexpp_before_tables() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("config.toml"),
        r#"model = "gpt-5"
[profiles.default]
model = "gpt-5-mini"
"#,
    )
    .unwrap();

    apply_relay_config_to_home(
        temp.path(),
        "https://relay.example.test/v1",
        "sk-test-redacted",
    )
    .unwrap();
    let updated = std::fs::read_to_string(temp.path().join("config.toml")).unwrap();
    let provider_index = updated.find(r#"model_provider = "CodexPlusPlus""#).unwrap();
    let codexpp_index = updated.find("[model_providers.CodexPlusPlus]").unwrap();
    let table_index = updated.find("[profiles.default]").unwrap();

    assert!(provider_index < table_index);
    assert!(codexpp_index < table_index);
}

#[test]
fn apply_relay_config_removes_legacy_codexpp_provider_table() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("config.toml"),
        r#"model_provider = "CodexPP"
[model_providers.CodexPP]
name = "CodexPP"
base_url = "https://old.example.test/v1"
"#,
    )
    .unwrap();

    apply_relay_config_to_home(
        temp.path(),
        "https://relay.example.test/v1",
        "sk-test-redacted",
    )
    .unwrap();
    let updated = std::fs::read_to_string(temp.path().join("config.toml")).unwrap();

    assert!(updated.contains(r#"model_provider = "CodexPlusPlus""#));
    assert!(updated.contains("[model_providers.CodexPlusPlus]"));
    assert!(!updated.contains("[model_providers.CodexPP]"));
}

#[test]
fn clear_relay_config_removes_model_provider_and_preserves_other_config() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("config.toml"),
        r#"model = "gpt-5"
model_provider = "CodexPlusPlus"
[model_providers.CodexPlusPlus]
name = "CodexPlusPlus"
wire_api = "responses"
requires_openai_auth = true
base_url = "https://relay.example.test/v1"
experimental_bearer_token = "sk-test-redacted"

[model_providers.CodexPP]
name = "CodexPP"
base_url = "https://old.example.test/v1"

[model_providers.custom1]
name = "custom1"
wire_api = "responses"
base_url = "https://keep.example.test/v1"

[profiles.default]
model = "gpt-5-mini"
"#,
    )
    .unwrap();

    let result = clear_relay_config_to_home(temp.path()).unwrap();
    let updated = std::fs::read_to_string(temp.path().join("config.toml")).unwrap();

    assert!(!result.configured);
    assert!(result.backup_path.is_some());
    assert!(updated.contains(r#"model = "gpt-5""#));
    assert!(!updated.contains("model_provider ="));
    assert!(!updated.contains("OPENAI_API_KEY"));
    assert!(!updated.contains("[model_providers.CodexPlusPlus]"));
    assert!(!updated.contains("[model_providers.CodexPP]"));
    assert!(!updated.contains("experimental_bearer_token"));
    assert!(updated.contains("[model_providers.custom1]"));
    assert!(updated.contains(r#"base_url = "https://keep.example.test/v1""#));
    assert!(updated.contains("[profiles.default]"));
}

#[test]
fn clear_relay_config_removes_pure_api_auth_json_key_and_preserves_other_auth_fields() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("auth.json"),
        r#"{"OPENAI_API_KEY":"sk-test-redacted","auth_mode":"chatgpt","tokens":{"access_token":"keep"}}"#,
    )
    .unwrap();
    std::fs::write(
        temp.path().join("config.toml"),
        r#"model = "gpt-5"
model_provider = "CodexPlusPlus"
[model_providers.CodexPlusPlus]
name = "custom"
wire_api = "responses"
requires_openai_auth = true
base_url = "https://relay.example.test/v1"
experimental_bearer_token = "sk-test-redacted"
"#,
    )
    .unwrap();

    clear_relay_config_to_home(temp.path()).unwrap();

    let auth: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(temp.path().join("auth.json")).unwrap())
            .unwrap();
    let auth_object = auth.as_object().unwrap();
    assert!(!auth_object.contains_key("OPENAI_API_KEY"));
    assert_eq!(auth["auth_mode"], "chatgpt");
    assert_eq!(auth["tokens"]["access_token"], "keep");
}

#[test]
fn clear_relay_config_removes_openai_api_key_when_auth_json_only_contains_pure_api_key() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("auth.json"),
        r#"{"OPENAI_API_KEY":"sk-test-redacted"}"#,
    )
    .unwrap();

    clear_relay_config_to_home(temp.path()).unwrap();

    let auth: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(temp.path().join("auth.json")).unwrap())
            .unwrap();
    let auth_object = auth.as_object().unwrap();
    assert!(!auth_object.contains_key("OPENAI_API_KEY"));
    assert!(auth_object.is_empty());
}

fn base64_url_no_pad(value: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value.as_bytes())
}
