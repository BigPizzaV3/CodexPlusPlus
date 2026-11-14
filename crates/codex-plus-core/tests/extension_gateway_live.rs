//! 拓展网关能力走真实请求链路：经协议代理打到本机模拟上游，验证请求中间件加请求头、
//! 失败中间件换模型降级、拓展供应商从上游 /models 拉模型列表。
//!
//! 单独成一个测试程序：要改平台配置目录环境变量，而网关中间件清单是进程级缓存，
//! 和 tests/all 里其它跑代理请求的用例同进程会互相读到对方的缓存。

use codex_plus_core::gateway::apply::{ApplyContext, ProviderInput, create_provider, enable_binding};
use codex_plus_core::gateway::store::{AgentBinding, GatewaySettings, GatewayStore};
use codex_plus_core::protocol_proxy::open_responses_proxy_request_with_settings;
use codex_plus_core::routes::{BridgeRuntimeService, CoreRuntimeService};
use codex_plus_core::settings::{BackendSettings, RelayMode, RelayProfile, RelayProtocol, SettingsStore};
use codex_plus_core::status::StatusStore;
use codex_plus_core::tools::{ToolId, claude::ClaudeContext};
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
            let (status, payload) = if head.starts_with("GET /late/models") {
                ("200 OK", json!({ "object": "list", "data": [{ "id": "late-model" }] }))
            } else if head.starts_with("GET /v1/models") {
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

async fn wait_for_catalog_model(path: &std::path::Path, model: &str, available: bool) {
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            let catalog: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
            if catalog["models"].as_array().unwrap().iter().any(|entry| entry["slug"] == model) == available {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }).await.expect("拓展模型目录没有及时更新");
}

#[tokio::test]
async fn gateway_extensions_on_the_real_request_path() {
    if codex_plus_core::dsh_plugins::locate_node().is_none() {
        eprintln!("跳过：本机没有 Node");
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let codex_home = temp.path().join("codex");
    let sqlite_home = temp.path().join("sqlite");
    std::fs::create_dir_all(&codex_home).unwrap();
    std::fs::create_dir_all(&sqlite_home).unwrap();
    // SAFETY: 本测试程序只有这一个用例，没有别的线程在读环境变量。
    unsafe {
        std::env::set_var("APPDATA", temp.path());
        std::env::set_var("XDG_CONFIG_HOME", temp.path());
        std::env::set_var(codex_plus_core::paths::STATE_DIR_ENV, temp.path().join("state"));
        std::env::set_var("CODEX_HOME", &codex_home);
        std::env::set_var("CODEX_SQLITE_HOME", &sqlite_home);
        std::env::set_var("HOME", temp.path());
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

    // #3072：拓展停用后，已发布给 Codex 的模型不能静默改发到默认供应商。
    let port = {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.local_addr().unwrap().port()
    };
    let state = temp.path().join("state");
    let ctx = ApplyContext {
        gateway: GatewayStore::default(),
        claude: ClaudeContext {
            store_path: state.join("claude-providers.json"),
            claude_home: temp.path().join("claude"),
            backup_root: state.join("backups"),
        },
        grok_home: temp.path().join("grok"),
        codex_home: codex_home.clone(),
        settings: SettingsStore::default(),
        backup_root: state.join("backups"),
        port,
        agent_env: codex_plus_core::gateway::agents::AgentEnv::at(temp.path().join("home")),
    };
    create_provider(
        &ctx.gateway,
        &ProviderInput {
            name: "Default".into(),
            base_url: format!("http://{address}/v1"),
            protocol: RelayProtocol::Responses,
            keys: vec!["sk-default".into()],
            model: "default-model".into(),
            model_list: "default-model".into(),
            ..ProviderInput::default()
        },
        "test:default",
        None,
    )
    .unwrap();
    let mut direct = codex_plus_core::provider_import::pool_relay_profile(
        "直连",
        "https://api.direct.test/v1",
        "sk-direct",
        RelayProtocol::Responses,
        "direct-model",
        "direct-model",
        &[],
    )
    .unwrap();
    direct.id = "direct".into();
    direct.relay_mode = RelayMode::PureApi;
    codex_plus_core::relay_config::normalize_relay_profile_for_storage(&mut direct).unwrap();
    let settings = BackendSettings {
        relay_profiles_enabled: true,
        relay_profiles: vec![direct],
        active_relay_id: "direct".into(),
        ..BackendSettings::default()
    };
    codex_plus_core::relay_switch::switch_relay_profile_in_home(
        &ctx.settings,
        &ctx.codex_home,
        settings,
        "direct",
    )
    .unwrap();
    let gateway = enable_binding(
        &ctx,
        &ToolId::Codex,
        AgentBinding {
            model: "default-model".into(),
            ..AgentBinding::default()
        },
    )
    .unwrap();
    let token = &gateway.bindings["codex"].token;
    let config = std::fs::read_to_string(codex_home.join("config.toml")).unwrap();
    let config: toml::Value = toml::from_str(&config).unwrap();
    let pointer = std::path::PathBuf::from(config["model_catalog_json"].as_str().unwrap());
    let catalog_path = if pointer.is_absolute() { pointer } else { codex_home.join(pointer) };
    let catalog: Value = serde_json::from_str(&std::fs::read_to_string(&catalog_path).unwrap()).unwrap();
    assert!(
        catalog["models"].as_array().unwrap().iter().any(|entry| entry["slug"] == "found-a"),
        "发现的拓展模型必须已经公布给 Codex：{catalog}"
    );

    codex_plus_core::resident_helper::ensure_resident_helper(port).unwrap();
    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{port}/gw/codex/v1/responses");
    let request = |model| json!({
        "model": model, "stream": false,
        "input": [{ "type": "message", "role": "user", "content": "hi" }]
    });
    let response = client.post(&url).bearer_auth(token).json(&request("found-a")).send().await.unwrap();
    let status = response.status();
    let text = response.text().await.unwrap();
    assert_eq!(status.as_u16(), 200, "{text}");
    let count_posts = |key: &str| {
        seen.lock().unwrap().iter().filter(|(head, _)| {
            head.starts_with("POST") && head.to_ascii_lowercase().contains(&format!("authorization: bearer {key}"))
        }).count()
    };
    assert_eq!(count_posts("sk-found"), 1, "拓展模型应发到拓展供应商");
    let default_count = count_posts("sk-default");

    // 等长改写且修改时间相同时，拓展缓存和运行中的 helper 都必须发现开关变化。
    codex_plus_core::resident_helper::stop_resident_helper();
    let scripts_config = root.join("user_scripts.json");
    let enabled_config = br#"{"enabled":true ,"scripts":{"user:live":true}}"#;
    let disabled_config = br#"{"enabled":false,"scripts":{"user:live":true}}"#;
    assert_eq!(enabled_config.len(), disabled_config.len());
    std::fs::write(&scripts_config, enabled_config).unwrap();
    let metadata = std::fs::metadata(&scripts_config).unwrap();
    let modified = metadata.modified().unwrap();
    assert!(GatewaySettings::default().with_extension_providers().providers.iter().any(|provider| provider.relay.id == "found"));
    std::fs::write(&scripts_config, disabled_config).unwrap();
    std::fs::OpenOptions::new().write(true).open(&scripts_config).unwrap().set_modified(modified).unwrap();
    let updated = std::fs::metadata(&scripts_config).unwrap();
    assert_eq!(updated.modified().unwrap(), modified);
    assert_eq!(updated.len(), metadata.len());
    assert!(
        !GatewaySettings::default().with_extension_providers().providers.iter().any(|provider| provider.relay.id == "found"),
        "等长且修改时间相同的配置变化也应立即刷新拓展供应商缓存"
    );
    std::fs::write(&scripts_config, enabled_config).unwrap();
    std::fs::OpenOptions::new().write(true).open(&scripts_config).unwrap().set_modified(modified).unwrap();
    assert!(GatewaySettings::default().with_extension_providers().providers.iter().any(|provider| provider.relay.id == "found"));
    codex_plus_core::resident_helper::ensure_resident_helper(port).unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
    std::fs::write(&scripts_config, disabled_config).unwrap();
    std::fs::OpenOptions::new().write(true).open(&scripts_config).unwrap().set_modified(modified).unwrap();
    wait_for_catalog_model(&catalog_path, "found-a", false).await;
    let response = client.post(&url).bearer_auth(token).json(&request("found-a")).send().await.unwrap();
    assert_eq!(response.status().as_u16(), 400);
    assert_eq!(count_posts("sk-default"), default_count);
    assert_eq!(count_posts("sk-found"), 1);
    std::fs::write(&scripts_config, enabled_config).unwrap();
    std::fs::OpenOptions::new().write(true).open(&scripts_config).unwrap().set_modified(modified).unwrap();
    wait_for_catalog_model(&catalog_path, "found-a", true).await;

    let runtime = CoreRuntimeService::new(0, StatusStore::new(temp.path().join("status.json")))
        .with_user_scripts(codex_plus_core::user_scripts::default_user_script_manager());
    runtime.set_user_script_enabled("user:live".into(), false).await.unwrap();
    assert!(
        !GatewaySettings::default().with_extension_providers().providers.iter().any(|provider| provider.relay.id == "found"),
        "拓展供应商缓存应在停用后立即刷新"
    );
    let response = client.post(&url).bearer_auth(token).json(&request("found-a")).send().await.unwrap();
    let status = response.status();
    let text = response.text().await.unwrap();
    assert_eq!(count_posts("sk-default"), default_count, "旧模型不可改发默认供应商");
    assert_eq!(count_posts("sk-found"), 1, "停用后的拓展也不可收到请求");
    assert_eq!(status.as_u16(), 400, "{text}");
    let error: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(error["error"]["code"], "model_not_found");
    assert!(text.contains("请重新选择模型"), "{text}");
    wait_for_catalog_model(&catalog_path, "found-a", false).await;

    let response = client.post(&url).bearer_auth(token).json(&request("default-model")).send().await.unwrap();
    let status = response.status();
    let text = response.text().await.unwrap();
    assert_eq!(status.as_u16(), 200, "{text}");
    assert_eq!(count_posts("sk-default"), default_count + 1, "默认模型应仍可用");

    runtime.set_user_script_enabled("user:live".into(), true).await.unwrap();
    wait_for_catalog_model(&catalog_path, "found-a", true).await;
    assert!(
        GatewaySettings::default().with_extension_providers().providers.iter().any(|provider| provider.relay.id == "found"),
        "拓展供应商缓存应在重新启用后立即刷新"
    );
    let response = client.post(&url).bearer_auth(token).json(&request("found-a")).send().await.unwrap();
    let status = response.status();
    let text = response.text().await.unwrap();
    assert_eq!(status.as_u16(), 200, "{text}");
    assert_eq!(count_posts("sk-found"), 2, "重新启用后应继续使用原拓展供应商");

    // 在 helper 已运行时新增异步发现模型的拓展，发现完成后也要发布。
    let late = root.join("user_scripts/late");
    std::fs::create_dir_all(&late).unwrap();
    std::fs::write(late.join("codexpp.json"), json!({
        "id": "late", "name": "后添加的拓展",
        "providers": [{ "id": "late", "protocol": "responses",
            "baseUrl": format!("http://{address}/late"), "apiKey": "sk-late", "discoverModels": true }]
    }).to_string()).unwrap();
    runtime.set_user_script_enabled("user:late".into(), true).await.unwrap();
    wait_for_catalog_model(&catalog_path, "late-model", true).await;
    let response = client.post(&url).bearer_auth(token).json(&request("late-model")).send().await.unwrap();
    assert_eq!(response.status().as_u16(), 200);
    assert_eq!(count_posts("sk-late"), 1);

    runtime.delete_user_script("user:late".into()).await.unwrap();
    wait_for_catalog_model(&catalog_path, "late-model", false).await;
    let response = client.post(&url).bearer_auth(token).json(&request("late-model")).send().await.unwrap();
    assert_eq!(response.status().as_u16(), 400);
    assert_eq!(count_posts("sk-default"), default_count + 1);
    assert_eq!(count_posts("sk-late"), 1);

    // 常驻 helper 停止期间改了拓展开关，重新启动后也要追上最新目录。
    codex_plus_core::resident_helper::stop_resident_helper();
    runtime.set_user_script_enabled("user:live".into(), false).await.unwrap();
    codex_plus_core::resident_helper::ensure_resident_helper(port).unwrap();
    wait_for_catalog_model(&catalog_path, "found-a", false).await;
    let response = client.post(&url).bearer_auth(token).json(&request("found-a")).send().await.unwrap();
    assert_eq!(response.status().as_u16(), 400);
    assert_eq!(count_posts("sk-default"), default_count + 1);
    assert_eq!(count_posts("sk-found"), 2);

    codex_plus_core::resident_helper::stop_resident_helper();
    runtime.set_user_script_enabled("user:live".into(), true).await.unwrap();
    codex_plus_core::resident_helper::ensure_resident_helper(port).unwrap();
    wait_for_catalog_model(&catalog_path, "found-a", true).await;
    let response = client.post(&url).bearer_auth(token).json(&request("found-a")).send().await.unwrap();
    assert_eq!(response.status().as_u16(), 200);
    assert_eq!(count_posts("sk-found"), 3);
    codex_plus_core::resident_helper::stop_resident_helper();
}
