use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::thread;

use codex_plus_core::model_catalog::{
    read_codex_model_catalog, read_codex_model_catalog_from_home,
};
use codex_plus_core::settings::{
    BackendSettings, RelayMode, RelayProfile, RelayProtocol, SettingsStore,
};
use serde_json::json;

#[tokio::test]
async fn model_catalog_fetches_models_from_codex_config_provider() {
    let temp = tempfile::tempdir().unwrap();
    let server = spawn_models_server(json!({
        "data": [
            {"id": "qwen3-coder"},
            {"id": "deepseek-coder"}
        ]
    }));
    write_config(
        temp.path(),
        &format!(
            r#"
model = "qwen3-coder"
model_provider = "relay"

[model_providers.relay]
name = "Relay"
base_url = "{}"
experimental_bearer_token = "relay-key"
"#,
            server.base_url
        ),
    );

    let result = read_codex_model_catalog_from_home(
        temp.path(),
        &HashMap::new(),
        reqwest::Client::builder().no_proxy().build().unwrap(),
    )
    .await;

    assert_eq!(result["status"], "ok");
    assert_eq!(result["model_provider"], "relay");
    assert_eq!(result["provider_name"], "Relay");
    assert_eq!(result["default_model"], "qwen3-coder");
    assert_eq!(result["models"], json!(["qwen3-coder", "deepseek-coder"]));
    assert_eq!(
        result["sources"][0]["endpoint"],
        format!("{}/v1/models", server.base_url)
    );
    assert_eq!(
        result["responses_api"],
        json!({
            "status": "unknown",
            "endpoint": "",
            "message": ""
        })
    );
    assert_eq!(result["sources"][0]["responses_api"]["status"], "unknown");
    let requests = server.finish();
    assert_eq!(requests[0].path, "/v1/models");
    assert_eq!(requests[0].authorization, "Bearer relay-key");
}

#[tokio::test]
async fn model_catalog_appends_models_to_versioned_base_url() {
    // Volcano Engine ARK (and other providers) expose a versioned base URL such
    // as `.../api/coding/v3`. The model list must be fetched from
    // `<base>/models`, not `<base>/v1/models` (which 404s). Regression for #1349.
    let temp = tempfile::tempdir().unwrap();
    let server = spawn_models_server(json!({
        "data": [
            {"id": "doubao-seed-code-preview"}
        ]
    }));
    let versioned_base = format!("{}/api/coding/v3", server.base_url);
    write_config(
        temp.path(),
        &format!(
            r#"
model = "doubao-seed-code-preview"
model_provider = "ark"

[model_providers.ark]
name = "ARK"
base_url = "{versioned_base}"
experimental_bearer_token = "ark-key"
"#
        ),
    );

    let result = read_codex_model_catalog_from_home(
        temp.path(),
        &HashMap::new(),
        reqwest::Client::builder().no_proxy().build().unwrap(),
    )
    .await;

    assert_eq!(result["status"], "ok");
    assert_eq!(result["models"], json!(["doubao-seed-code-preview"]));
    assert_eq!(
        result["sources"][0]["endpoint"],
        format!("{versioned_base}/models")
    );
    let requests = server.finish();
    assert_eq!(requests[0].path, "/api/coding/v3/models");
    assert_eq!(requests[0].authorization, "Bearer ark-key");
}

#[tokio::test]
async fn model_catalog_reports_effective_service_tier_from_config() {
    let temp = tempfile::tempdir().unwrap();
    let server = spawn_models_server(json!({
        "data": [
            {"id": "qwen3-coder"}
        ]
    }));
    write_config(
        temp.path(),
        &format!(
            r#"
model = "qwen3-coder"
model_provider = "relay"
service_tier = "fast"

[model_providers.relay]
name = "Relay"
base_url = "{}"
experimental_bearer_token = "relay-key"
"#,
            server.base_url
        ),
    );

    let result = read_codex_model_catalog_from_home(
        temp.path(),
        &HashMap::new(),
        reqwest::Client::builder().no_proxy().build().unwrap(),
    )
    .await;

    assert_eq!(result["status"], "ok");
    assert_eq!(result["service_tier"], "fast");
    server.finish();
}

#[tokio::test]
async fn model_catalog_service_tier_is_null_when_config_does_not_set_it() {
    let temp = tempfile::tempdir().unwrap();
    let server = spawn_models_server(json!({
        "data": [
            {"id": "qwen3-coder"}
        ]
    }));
    write_config(
        temp.path(),
        &format!(
            r#"
model = "qwen3-coder"
model_provider = "relay"

[model_providers.relay]
name = "Relay"
base_url = "{}"
experimental_bearer_token = "relay-key"
"#,
            server.base_url
        ),
    );

    let result = read_codex_model_catalog_from_home(
        temp.path(),
        &HashMap::new(),
        reqwest::Client::builder().no_proxy().build().unwrap(),
    )
    .await;

    assert_eq!(result["status"], "ok");
    assert_eq!(result["service_tier"], serde_json::Value::Null);
    server.finish();
}

#[tokio::test]
async fn model_catalog_uses_active_relay_profile_model_list_for_display() {
    let _lock = codex_home_test_lock();
    let temp = tempfile::tempdir().unwrap();
    let codex_home = temp.path().join("codex-home");
    std::fs::create_dir_all(&codex_home).unwrap();
    let settings_path = temp.path().join("settings.json");
    let previous_codex_home = std::env::var_os("CODEX_HOME");
    let previous_settings_path =
        codex_plus_core::paths::set_settings_path_for_tests(Some(settings_path.clone()));
    unsafe {
        std::env::set_var("CODEX_HOME", &codex_home);
    }

    let result = async {
        SettingsStore::new(settings_path)
            .save(&BackendSettings {
                active_relay_id: "relay-a".to_string(),
                relay_profiles: vec![RelayProfile {
                    id: "relay-a".to_string(),
                    name: "Relay A".to_string(),
                    model: "qwen3-coder".to_string(),
                    base_url: "https://example.test/v1".to_string(),
                    protocol: RelayProtocol::Responses,
                    relay_mode: RelayMode::MixedApi,
                    model_list: "deepseek-coder\nqwen3-coder\nclaude-compatible\ngpt-5.6-sol"
                        .to_string(),
                    config_contents: "model = \"qwen3-coder\"\n".to_string(),
                    ..RelayProfile::default()
                }],
                ..BackendSettings::default()
            })
            .unwrap();

        read_codex_model_catalog().await
    }
    .await;

    match previous_codex_home {
        Some(value) => unsafe {
            std::env::set_var("CODEX_HOME", value);
        },
        None => unsafe {
            std::env::remove_var("CODEX_HOME");
        },
    }
    codex_plus_core::paths::set_settings_path_for_tests(previous_settings_path);

    assert_eq!(result["status"], "ok");
    assert_eq!(result["model_provider"], "relay-a");
    assert_eq!(result["provider_name"], "Relay A");
    assert_eq!(result["default_model"], "qwen3-coder");
    assert_eq!(
        result["models"],
        json!([
            "qwen3-coder",
            "deepseek-coder",
            "claude-compatible",
            "gpt-5.6-sol"
        ])
    );
    assert_eq!(
        result["modelMetadata"]["gpt-5.6-sol"]["defaultReasoningEffort"],
        "low"
    );
    assert_eq!(
        result["modelMetadata"]["gpt-5.6-sol"]["supportedReasoningEfforts"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|entry| entry["reasoningEffort"].as_str())
            .collect::<Vec<_>>(),
        ["low", "medium", "high", "xhigh", "max", "ultra"]
    );
    assert_eq!(result["sources"][0]["type"], "relay_profile_model_list");
}

#[tokio::test]
async fn model_catalog_uses_single_provider_when_root_model_provider_is_absent() {
    let temp = tempfile::tempdir().unwrap();
    let server = spawn_models_server(json!({
        "models": ["moonshot-v1", "mimo-v2.5-pro"]
    }));
    write_config(
        temp.path(),
        &format!(
            r#"
[model_providers.only]
name = "Only Provider"
base_url = "{}/v1"
"#,
            server.base_url
        ),
    );

    let result = read_codex_model_catalog_from_home(
        temp.path(),
        &HashMap::new(),
        reqwest::Client::builder().no_proxy().build().unwrap(),
    )
    .await;

    assert_eq!(result["status"], "ok");
    assert_eq!(result["model_provider"], "only");
    assert_eq!(result["models"], json!(["moonshot-v1", "mimo-v2.5-pro"]));
    let requests = server.finish();
    assert_eq!(requests[0].path, "/v1/models");
    assert_eq!(result["responses_api"]["status"], "unknown");
}

#[tokio::test]
async fn model_catalog_merges_models_from_config_model_catalog_json() {
    let temp = tempfile::tempdir().unwrap();
    let server = spawn_models_server(json!({
        "data": [
            {"id": "qwen3-coder"}
        ]
    }));
    let catalog_path = temp.path().join("custom-models.json");
    std::fs::write(
        &catalog_path,
        json!({
            "models": [
                {
                    "slug": "gpt-5.6-sol",
                    "display_name": "GPT-5.6-Sol",
                    "visibility": "list",
                    "supported_in_api": true
                }
            ]
        })
        .to_string(),
    )
    .unwrap();
    write_config(
        temp.path(),
        &format!(
            r#"
 model = "gpt-5.6-sol"
model_provider = "relay"
model_catalog_json = "{}"

[model_providers.relay]
name = "Relay"
base_url = "{}"
experimental_bearer_token = "relay-key"
"#,
            catalog_path.display().to_string().replace('\\', "\\\\"),
            server.base_url
        ),
    );

    let result = read_codex_model_catalog_from_home(
        temp.path(),
        &HashMap::new(),
        reqwest::Client::builder().no_proxy().build().unwrap(),
    )
    .await;

    assert_eq!(result["status"], "ok");
    assert_eq!(result["default_model"], "gpt-5.6-sol");
    assert_eq!(result["models"], json!(["qwen3-coder", "gpt-5.6-sol"]));
    assert_eq!(
        result["modelMetadata"]["gpt-5.6-sol"]["supportedReasoningEfforts"][5]["reasoningEffort"],
        "ultra"
    );
    server.finish();
}

#[tokio::test]
async fn model_catalog_reads_single_quoted_config_model_catalog_json_path() {
    let temp = tempfile::tempdir().unwrap();
    let catalog_path = temp.path().join("literal-path-models.json");
    std::fs::write(
        &catalog_path,
        json!({
            "models": [
                {
                    "slug": "gpt-5.6",
                    "visibility": "list",
                    "supported_in_api": true
                },
                {
                    "slug": "hidden-test-model",
                    "visibility": "hidden",
                    "supported_in_api": true
                },
                {
                    "slug": "chatgpt-only-test-model",
                    "visibility": "list",
                    "supported_in_api": false
                }
            ]
        })
        .to_string(),
    )
    .unwrap();
    write_config(
        temp.path(),
        &format!(
            r#"
model = "gpt-5.6"
model_catalog_json = '{}'
"#,
            catalog_path.display()
        ),
    );

    let result = read_codex_model_catalog_from_home(
        temp.path(),
        &HashMap::new(),
        reqwest::Client::builder().no_proxy().build().unwrap(),
    )
    .await;

    assert_eq!(result["status"], "ok");
    assert_eq!(result["default_model"], "gpt-5.6");
    assert_eq!(result["models"], json!(["gpt-5.6"]));
    assert_eq!(result["sources"][0]["status"], "ok");
    assert_eq!(result["sources"][0]["models"], 1);
}

#[tokio::test]
async fn model_catalog_leaves_responses_api_unknown_without_probe() {
    let temp = tempfile::tempdir().unwrap();
    let server = spawn_models_server(json!({
        "data": [
            {"id": "legacy-model"}
        ]
    }));
    write_config(
        temp.path(),
        &format!(
            r#"
model = "legacy-model"

[model_providers.legacy]
name = "Legacy"
base_url = "{}"
"#,
            server.base_url
        ),
    );

    let result = read_codex_model_catalog_from_home(
        temp.path(),
        &HashMap::new(),
        reqwest::Client::builder().no_proxy().build().unwrap(),
    )
    .await;

    assert_eq!(result["status"], "ok");
    assert_eq!(result["responses_api"]["status"], "unknown");
    assert_eq!(result["responses_api"]["endpoint"], "");
    assert_eq!(result["sources"][0]["responses_api"]["status"], "unknown");
    let requests = server.finish();
    assert_eq!(requests[0].path, "/v1/models");
}

fn write_config(home: &Path, contents: &str) {
    std::fs::write(home.join("config.toml"), contents.trim_start()).unwrap();
}

/// Serializes the tests that repoint `CODEX_HOME` and the settings path, both of
/// which are process-wide.
fn codex_home_test_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct CodexHomeGuard {
    previous: Option<std::ffi::OsString>,
}

impl CodexHomeGuard {
    fn set(home: &Path) -> Self {
        let previous = std::env::var_os("CODEX_HOME");
        unsafe {
            std::env::set_var("CODEX_HOME", home);
        }
        Self { previous }
    }
}

impl Drop for CodexHomeGuard {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => unsafe { std::env::set_var("CODEX_HOME", value) },
            None => unsafe { std::env::remove_var("CODEX_HOME") },
        }
    }
}

struct SettingsPathGuard {
    previous: Option<PathBuf>,
}

impl SettingsPathGuard {
    fn set(path: PathBuf) -> Self {
        Self {
            previous: codex_plus_core::paths::set_settings_path_for_tests(Some(path)),
        }
    }
}

impl Drop for SettingsPathGuard {
    fn drop(&mut self) {
        codex_plus_core::paths::set_settings_path_for_tests(self.previous.take());
    }
}

/// The three profiles the routing tests share: a plain official profile plus two
/// third-party providers, one of which pins its context window with a suffix.
fn model_routing_profiles() -> Vec<RelayProfile> {
    vec![
        RelayProfile {
            id: "official".to_string(),
            name: "ChatGPT".to_string(),
            relay_mode: RelayMode::Official,
            ..RelayProfile::default()
        },
        RelayProfile {
            id: "mimo".to_string(),
            name: "MiMo".to_string(),
            relay_mode: RelayMode::PureApi,
            base_url: "https://mimo.test/v1".to_string(),
            model_list: "mimo-v2.5-pro[1M]".to_string(),
            ..RelayProfile::default()
        },
        RelayProfile {
            id: "claude".to_string(),
            name: "Claude API".to_string(),
            relay_mode: RelayMode::PureApi,
            base_url: "https://claude.test/v1".to_string(),
            model_list: "claude-sonnet-4-6".to_string(),
            ..RelayProfile::default()
        },
    ]
}

/// Writes the catalog the launcher generates for the router: Codex's official
/// models merged with every routed model.
fn write_router_catalog(codex_home: &Path) {
    std::fs::create_dir_all(codex_home.join("model-catalogs")).unwrap();
    std::fs::write(
        codex_home
            .join("model-catalogs")
            .join("codex-plus-router.json"),
        r#"{"models":[
            {"slug":"gpt-5.6-codex","context_window":272000},
            {"slug":"gpt-5.6","context_window":272000},
            {"slug":"claude-sonnet-4-6","context_window":200000},
            {"slug":"mimo-v2.5-pro","context_window":1000000}
        ]}"#,
    )
    .unwrap();
}

#[tokio::test]
async fn model_catalog_lists_official_and_third_party_models_when_routing_is_enabled() {
    let _lock = codex_home_test_lock();
    let temp = tempfile::tempdir().unwrap();
    let codex_home = temp.path().join("codex-home");
    std::fs::create_dir_all(&codex_home).unwrap();
    write_router_catalog(&codex_home);
    let settings_path = temp.path().join("settings.json");
    let _settings_guard = SettingsPathGuard::set(settings_path.clone());
    let _home_guard = CodexHomeGuard::set(&codex_home);

    SettingsStore::new(settings_path)
        .save(&BackendSettings {
            model_routing_enabled: true,
            active_relay_id: "official".to_string(),
            relay_profiles: model_routing_profiles(),
            ..BackendSettings::default()
        })
        .unwrap();
    let result = read_codex_model_catalog().await;

    assert_eq!(result["status"], "ok");
    assert_eq!(result["model_provider"], "codex-plus-router");
    assert_eq!(result["provider_name"], "Codex++ Model Router");
    // Official models first, then each provider's declared models. `mimo-v2.5-pro`
    // and `claude-sonnet-4-6` are in the generated catalog too, but the routed
    // profile owns them, so they appear once each rather than twice.
    assert_eq!(
        result["models"],
        json!([
            "gpt-5.6-codex",
            "gpt-5.6",
            "mimo-v2.5-pro",
            "claude-sonnet-4-6"
        ])
    );
    assert_eq!(result["sources"][0]["type"], "official_model_catalog");
    assert_eq!(result["sources"][0]["models"], 2);
    assert_eq!(result["sources"][1]["type"], "model_route");
    assert_eq!(result["sources"][1]["name"], "MiMo");
    assert_eq!(result["sources"][2]["name"], "Claude API");
    // The plain official profile is not a route of its own.
    assert_eq!(result["sources"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn model_catalog_keeps_per_profile_behaviour_when_routing_is_disabled() {
    let _lock = codex_home_test_lock();
    let temp = tempfile::tempdir().unwrap();
    let codex_home = temp.path().join("codex-home");
    std::fs::create_dir_all(&codex_home).unwrap();
    // Left over from a previous routing session; it must not leak into the
    // per-profile catalog once the switch is off again.
    write_router_catalog(&codex_home);
    let settings_path = temp.path().join("settings.json");
    let _settings_guard = SettingsPathGuard::set(settings_path.clone());
    let _home_guard = CodexHomeGuard::set(&codex_home);

    SettingsStore::new(settings_path)
        .save(&BackendSettings {
            model_routing_enabled: false,
            active_relay_id: "mimo".to_string(),
            relay_profiles: model_routing_profiles(),
            ..BackendSettings::default()
        })
        .unwrap();
    let result = read_codex_model_catalog().await;

    assert_eq!(result["status"], "ok");
    assert_eq!(result["model_provider"], "mimo");
    assert_eq!(result["provider_name"], "MiMo");
    assert_eq!(result["models"], json!(["mimo-v2.5-pro"]));
    assert_eq!(result["sources"][0]["type"], "relay_profile_model_list");
    assert_eq!(result["sources"].as_array().unwrap().len(), 1);
}

struct ModelsServer {
    base_url: String,
    handle: thread::JoinHandle<Vec<ModelsRequest>>,
}

impl ModelsServer {
    fn finish(self) -> Vec<ModelsRequest> {
        self.handle.join().unwrap()
    }
}

struct ModelsRequest {
    path: String,
    authorization: String,
}

fn spawn_models_server(payload: serde_json::Value) -> ModelsServer {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let base_url = format!("http://{address}");
    listener
        .set_nonblocking(true)
        .expect("listener should switch to nonblocking mode");
    let models_body = payload.to_string();
    let handle = thread::spawn(move || {
        let started = std::time::Instant::now();
        let mut requests = Vec::new();
        while requests.is_empty() && started.elapsed() < std::time::Duration::from_secs(2) {
            let Ok((mut stream, _)) = listener.accept() else {
                std::thread::sleep(std::time::Duration::from_millis(10));
                continue;
            };
            let mut buffer = [0u8; 4096];
            let mut read = 0;
            let read_started = std::time::Instant::now();
            while read == 0 && read_started.elapsed() < std::time::Duration::from_secs(2) {
                match stream.read(&mut buffer) {
                    Ok(0) => std::thread::sleep(std::time::Duration::from_millis(10)),
                    Ok(bytes) => read = bytes,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    Err(error) => panic!("failed to read test request: {error}"),
                }
            }
            if read == 0 {
                continue;
            }
            let request = String::from_utf8_lossy(&buffer[..read]).to_string();
            let request_path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or_default()
                .to_string();
            let authorization = request
                .lines()
                .find_map(|line| line.strip_prefix("authorization: "))
                .unwrap_or_default()
                .to_string();
            let (status, body) = (200, models_body.as_str());
            let response = format!(
                "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
            requests.push(ModelsRequest {
                path: request_path,
                authorization,
            });
        }
        requests
    });
    ModelsServer { base_url, handle }
}
