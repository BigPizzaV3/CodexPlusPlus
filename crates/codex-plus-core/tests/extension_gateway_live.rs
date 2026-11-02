//! 拓展网关能力走真实请求链路：经协议代理打到本机模拟上游，验证请求中间件加请求头、
//! 失败中间件换模型降级、拓展供应商从上游 /models 拉模型列表。
//!
//! 单独成一个测试程序：要改平台配置目录环境变量，而网关中间件清单是进程级缓存，
//! 和 tests/all 里其它跑代理请求的用例同进程会互相读到对方的缓存。

use codex_plus_core::gateway::store::GatewaySettings;
use codex_plus_core::protocol_proxy::open_responses_proxy_request_with_settings;
use codex_plus_core::settings::{BackendSettings, RelayMode, RelayProfile, RelayProtocol};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

type Seen = Arc<Mutex<Vec<(String, String)>>>;

/// 模拟上游：记下每个请求（请求头、请求体）。model 为 bad-model 时回 404，GET /v1/models 回模型列表，其余回一个完成的 Response。
async fn serve(listener: tokio::net::TcpListener, seen: Seen) {
    loop {
        let Ok((mut stream, _)) = listener.accept().await else { return };
        let seen = seen.clone();
        tokio::spawn(async move {
            let mut buffer = Vec::new();
            let mut chunk = [0; 8192];
            let (head, body) = loop {
                let read = stream.read(&mut chunk).await.unwrap_or(0);
                if read == 0 {
                    return;
                }
                buffer.extend_from_slice(&chunk[..read]);
                let Some(end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") else { continue };
                let head = String::from_utf8_lossy(&buffer[..end]).to_string();
                let length = head
                    .lines()
                    .find_map(|line| line.to_ascii_lowercase().strip_prefix("content-length:").map(|value| value.trim().parse::<usize>().unwrap_or(0)))
                    .unwrap_or(0);
                while buffer.len() < end + 4 + length {
                    let read = stream.read(&mut chunk).await.unwrap_or(0);
                    if read == 0 {
                        break;
                    }
                    buffer.extend_from_slice(&chunk[..read]);
                }
                break (head, String::from_utf8_lossy(&buffer[end + 4..]).to_string());
            };
            seen.lock().unwrap().push((head.clone(), body.clone()));
            let (status, payload) = if head.starts_with("GET /v1/models") {
                ("200 OK", json!({ "object": "list", "data": [{ "id": "found-a" }, { "id": "found-b" }] }))
            } else if body.contains("\"bad-model\"") {
                ("404 Not Found", json!({ "error": { "message": "model not found" } }))
            } else {
                ("200 OK", json!({ "id": "resp_live", "object": "response", "status": "completed", "output": [] }))
            };
            let text = payload.to_string();
            let response = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}", text.len());
            let _ = stream.write_all(response.as_bytes()).await;
        });
    }
}

#[tokio::test]
async fn gateway_extensions_on_the_real_request_path() {
    if codex_plus_core::dsh_plugins::locate_node().is_none() {
        eprintln!("跳过：本机没有 Node");
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let config_home_env = if cfg!(windows) { "APPDATA" } else { "XDG_CONFIG_HOME" };
    // SAFETY: 本测试程序只有这一个用例，没有别的线程在读环境变量。
    unsafe {
        std::env::set_var(config_home_env, temp.path());
        std::env::set_var(codex_plus_core::paths::STATE_DIR_ENV, temp.path().join("state"));
    }
    let root = temp.path().join("Codex++");
    assert_eq!(
        codex_plus_core::user_scripts::default_user_scripts_config_dir(),
        root,
        "拓展必须从测试临时目录加载",
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let seen: Seen = Arc::default();
    tokio::spawn(serve(listener, seen.clone()));

    let dir = root.join("user_scripts/live");
    std::fs::create_dir_all(dir.join("gateway")).unwrap();
    std::fs::write(root.join("user_scripts.json"), json!({ "enabled": true, "scripts": {} }).to_string()).unwrap();
    std::fs::write(dir.join("codexpp.json"), json!({
        "id": "live", "name": "网关实测",
        "gateway": { "request": [{ "script": "gateway/request.mjs" }], "error": [{ "script": "gateway/error.mjs" }] },
        "providers": [{ "id": "found", "protocol": "responses", "baseUrl": format!("http://{address}/v1"), "apiKey": "sk-found", "discoverModels": true }]
    }).to_string()).unwrap();
    std::fs::write(dir.join("gateway/request.mjs"), r#"
export default (body, context) => {
  context.headers["x-codexpp-trace"] = `step-${context.step.index}`;
  context.headers["authorization"] = "Bearer stolen";
  return null;
};"#).unwrap();
    std::fs::write(dir.join("gateway/error.mjs"), r#"
export default (error) => error.status === 404 ? { kind: "fallback", model: "good-model" } : null;"#).unwrap();

    let settings = BackendSettings {
        active_relay_id: "live".into(),
        relay_profiles: vec![RelayProfile {
            id: "live".into(),
            name: "live".into(),
            protocol: RelayProtocol::Responses,
            base_url: format!("http://{address}/v1"),
            api_key: "sk-live".into(),
            relay_mode: RelayMode::PureApi,
            model: "bad-model".into(),
            model_list: "bad-model\ngood-model".into(),
            ..RelayProfile::default()
        }],
        ..BackendSettings::default()
    };
    let request = json!({ "model": "bad-model", "stream": false, "input": [{ "type": "message", "role": "user", "content": "hi" }] });
    let response = open_responses_proxy_request_with_settings(&request.to_string(), settings).await.unwrap();
    assert_eq!(response.status_code, 200, "降级到 good-model 后成功");

    let requests = seen.lock().unwrap().clone();
    let posts = requests.iter().filter(|(head, _)| head.starts_with("POST")).collect::<Vec<_>>();
    assert_eq!(posts.len(), 2, "原请求 404 一次，换模型重发一次：{requests:?}");
    let model = |body: &str| serde_json::from_str::<Value>(body).unwrap()["model"].as_str().unwrap_or_default().to_string();
    assert_eq!(model(&posts[0].1), "bad-model");
    assert_eq!(model(&posts[1].1), "good-model");
    for (head, _) in &posts {
        let lower = head.to_ascii_lowercase();
        assert!(lower.contains("x-codexpp-trace: step-1"), "中间件加的请求头带到上游（重发也带）：{head}");
        assert!(lower.contains("authorization: bearer sk-live") && !lower.contains("stolen"), "鉴权头不让拓展改：{head}");
    }

    // 拓展供应商从上游 /models 拉模型：第一次在后台拉，之后合并进模型列表。
    let _ = GatewaySettings::default().with_extension_providers();
    let mut models = String::new();
    for _ in 0..40 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let merged = GatewaySettings::default().with_extension_providers();
        if let Some(provider) = merged.providers.iter().find(|provider| provider.relay.id == "found") {
            models = provider.relay.model_list.clone();
            if models.contains("found-b") {
                break;
            }
        }
    }
    assert_eq!(models, "found-a\nfound-b");
    assert!(seen.lock().unwrap().iter().any(|(head, _)| head.starts_with("GET /v1/models") && head.to_ascii_lowercase().contains("bearer sk-found")));
}
