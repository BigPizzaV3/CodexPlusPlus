use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::thread;

use codex_plus_core::model_catalog::read_codex_model_catalog_from_home;
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
            "status": "supported",
            "endpoint": format!("{}/v1/responses", server.base_url),
            "message": ""
        })
    );
    assert_eq!(result["sources"][0]["responses_api"]["status"], "supported");
    let requests = server.finish();
    assert_eq!(requests[0].path, "/v1/models");
    assert_eq!(requests[0].authorization, "Bearer relay-key");
    assert_eq!(requests[1].path, "/v1/responses");
    assert_eq!(requests[1].authorization, "Bearer relay-key");
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
    assert_eq!(requests[1].path, "/v1/responses");
}

#[tokio::test]
async fn model_catalog_marks_responses_api_unsupported_when_probe_returns_404() {
    let temp = tempfile::tempdir().unwrap();
    let server = spawn_models_server_with_responses(
        json!({
            "data": [
                {"id": "legacy-model"}
            ]
        }),
        404,
        json!({"error": {"message": "Not Found"}}),
    );
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
    assert_eq!(result["responses_api"]["status"], "unsupported");
    assert_eq!(
        result["responses_api"]["endpoint"],
        format!("{}/v1/responses", server.base_url)
    );
    assert!(
        result["responses_api"]["message"]
            .as_str()
            .unwrap()
            .contains("404")
    );
    assert_eq!(
        result["sources"][0]["responses_api"]["status"],
        "unsupported"
    );
    let requests = server.finish();
    assert_eq!(requests[0].path, "/v1/models");
    assert_eq!(requests[1].path, "/v1/responses");
}

fn write_config(home: &Path, contents: &str) {
    std::fs::write(home.join("config.toml"), contents.trim_start()).unwrap();
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
    spawn_models_server_with_responses(payload, 200, json!({"id": "resp_1"}))
}

fn spawn_models_server_with_responses(
    payload: serde_json::Value,
    responses_status: u16,
    responses_payload: serde_json::Value,
) -> ModelsServer {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let base_url = format!("http://{address}");
    listener
        .set_nonblocking(true)
        .expect("listener should switch to nonblocking mode");
    let models_body = payload.to_string();
    let responses_body = responses_payload.to_string();
    let handle = thread::spawn(move || {
        let started = std::time::Instant::now();
        let mut requests = Vec::new();
        while requests.len() < 2 && started.elapsed() < std::time::Duration::from_secs(2) {
            let Ok((mut stream, _)) = listener.accept() else {
                std::thread::sleep(std::time::Duration::from_millis(10));
                continue;
            };
            let mut buffer = [0u8; 4096];
            let read = stream.read(&mut buffer).unwrap();
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
            let (status, body) = if request_path == "/v1/responses" {
                (responses_status, responses_body.as_str())
            } else {
                (200, models_body.as_str())
            };
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
