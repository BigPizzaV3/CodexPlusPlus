use super::*;

#[tokio::test]
async fn native_upgrade_preserves_auth_and_does_not_generate_during_prepare() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1/responses", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws =
            tokio_tungstenite::accept_hdr_async(stream, |request: &http::Request<()>, response| {
                assert_eq!(request.method(), "GET");
                assert_eq!(request.headers()["authorization"], "Bearer test-only");
                assert!(!request.headers().contains_key("content-length"));
                Ok(response)
            })
            .await
            .unwrap();
        let frame = ws.next().await.unwrap().unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(frame.to_text().unwrap()).unwrap()["type"],
            "response.create"
        );
    });
    let context = RequestContext {
        registry: Arc::new(Mutex::new(Registry::default())),
        previous: None,
    };
    REQUEST
        .scope(context, async {
            let builder = reqwest::Client::builder()
                .no_proxy()
                .build()
                .unwrap()
                .post(url)
                .bearer_auth("test-only")
                .json(&json!({"model":"test"}));
            let PreparedUpstream::Websocket(mut ready) = prepare(
                "test",
                builder,
                json!({"model":"test"}),
                Duration::from_secs(5),
            )
            .await
            .unwrap() else {
                panic!("upgrade rejected")
            };
            ready
                .socket
                .as_mut()
                .unwrap()
                .send(Message::Text(
                    json!({"type":"response.create","model":"test"})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
        })
        .await;
    server.await.unwrap();
}

use crate::gateway::store::{GatewayPathTestScope, GatewaySettings, GatewayStore};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
type Client = WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

struct Fixture {
    previous: Option<std::path::PathBuf>,
    _gateway: GatewayPathTestScope,
    temp: tempfile::TempDir,
    _guard: std::sync::MutexGuard<'static, ()>,
}
impl Fixture {
    fn new(profile: Value) -> Self {
        let guard = crate::paths::settings_path_test_guard();
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings.json");
        let previous = crate::paths::set_settings_path_for_tests(Some(path.clone()));
        let gateway = GatewayPathTestScope::new(temp.path());
        std::fs::write(
            path,
            json!({"relayProfiles":[profile],"activeRelayId":"ws-test"}).to_string(),
        )
        .unwrap();
        Self {
            previous,
            _gateway: gateway,
            temp,
            _guard: guard,
        }
    }
    fn gateway(&self, relay: Value) {
        let settings: GatewaySettings = serde_json::from_value(json!({
            "version":3,"bindings":{"codex":{"enabled":true,"model":"ws-model","token":"local-test",
                "autoReviewModel":"review-model","extraModels":["review-model"]}},
            "providers":[{"relay":relay,"keyConcurrency":1}],"autoModels":false
        }))
        .unwrap();
        GatewayStore::default().save(&settings).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        crate::paths::set_settings_path_for_tests(self.previous.take());
    }
}
fn profile(address: std::net::SocketAddr) -> Value {
    json!({"id":"ws-test","name":"WS test","apiKey":"sk-test",
        "apiKeys":[{"id":"ws-test-key","name":"test","apiKey":"sk-test"}],"activeApiKeyId":"ws-test-key","protocol":"responses",
        "relayMode":"pureApi","model":"ws-model","modelList":"ws-model\nreview-model",
        "upstreamBaseUrl":format!("http://{address}/v1"),"useCommonConfig":false,
        "configContents":"model = \"ws-model\"","autoReviewModel":"review-model"})
}
async fn helper(count: usize) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..count {
            let (stream, addr) = listener.accept().await.unwrap();
            tasks.spawn(crate::launcher::handle_helper_connection_on(
                stream,
                Some(addr),
                crate::launcher::HelperListener::Loopback,
            ));
        }
        while tasks.join_next().await.is_some() {}
    });
    (url, task)
}
async fn client(url: &str, headers: &[(&str, String)]) -> Client {
    let mut request = url.into_client_request().unwrap();
    for (name, value) in headers {
        request.headers_mut().insert(
            http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
            value.parse().unwrap(),
        );
    }
    let (ws, response) = tokio_tungstenite::connect_async(request).await.unwrap();
    assert_eq!(response.status(), 101);
    ws
}
async fn send<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    ws: &mut WebSocketStream<S>,
    event: Value,
) {
    ws.send(Message::Text(event.to_string().into()))
        .await
        .unwrap();
}
async fn next<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    ws: &mut WebSocketStream<S>,
) -> Value {
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("WS fixture timed out")
            .unwrap()
            .unwrap();
        if let Message::Text(text) = frame {
            return serde_json::from_str(&text).unwrap();
        }
    }
}
fn create(lane: &str) -> Value {
    let mut event =
        json!({"type":"response.create","model":"ws-model","input":"hello","store":false});
    if !lane.is_empty() {
        event["stream_id"] = json!(lane);
    }
    event
}
fn completed(id: &str, lane: &str) -> Value {
    let mut event = json!({"type":"response.completed","response":{"id":id,"status":"completed","output":[],
        "usage":{"input_tokens":3,"output_tokens":1,"total_tokens":4}}});
    if !lane.is_empty() {
        event["stream_id"] = json!(lane);
    }
    event
}
async fn finish(task: tokio::task::JoinHandle<()>) {
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn helper_ws_preserves_warmup_tools_and_rewrites_review_model() {
    if isolated("helper_ws_preserves_warmup_tools_and_rewrites_review_model").await {
        return;
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let _fixture = Fixture::new(profile(listener.local_addr().unwrap()));
    let upstream = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws =
            tokio_tungstenite::accept_hdr_async(socket, |req: &http::Request<()>, response| {
                assert_eq!(req.uri(), "/v1/responses");
                assert_eq!(req.headers()["authorization"], "Bearer sk-test");
                Ok(response)
            })
            .await
            .unwrap();
        let warmup = next(&mut ws).await;
        assert_eq!(warmup["generate"], false);
        assert!(warmup.get("stream").is_none());
        send(&mut ws, completed("warmup", "")).await;
        let continuation = next(&mut ws).await;
        assert_eq!(continuation["previous_response_id"], "warmup");
        assert_eq!(continuation["input"][0]["type"], "function_call_output");
        assert_eq!(continuation["input"][0]["call_id"], "call-test");
        send(&mut ws, completed("followup", "")).await;
        let review = next(&mut ws).await;
        assert_eq!(review["model"], "review-model");
        send(&mut ws, completed("review", "")).await;
        let _ = ws.next().await;
    });
    let (url, task) = helper(1).await;
    let mut ws = client(&format!("{url}/v1/responses"), &[]).await;
    let mut warmup = create("");
    warmup["generate"] = json!(false);
    send(&mut ws, warmup).await;
    assert_eq!(next(&mut ws).await["response"]["id"], "warmup");
    let mut continuation = create("");
    continuation["previous_response_id"] = json!("warmup");
    continuation["input"] =
        json!([{"type":"function_call_output","call_id":"call-test","output":"ok"}]);
    send(&mut ws, continuation).await;
    assert_eq!(next(&mut ws).await["response"]["id"], "followup");
    let mut review = create("");
    review["model"] = json!("codex-auto-review");
    send(&mut ws, review).await;
    assert_eq!(next(&mut ws).await["response"]["id"], "review");
    ws.close(None).await.unwrap();
    drop(ws);
    finish(task).await;
    finish(upstream).await;
}

#[tokio::test]
async fn helper_ws_named_lanes_share_connection_run_in_parallel_and_keep_fifo() {
    if isolated("helper_ws_named_lanes_share_connection_run_in_parallel_and_keep_fifo").await {
        return;
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let _fixture = Fixture::new(profile(listener.local_addr().unwrap()));
    let upstream = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();
        let first = next(&mut ws).await;
        let second = next(&mut ws).await;
        assert_ne!(first["stream_id"], second["stream_id"]);
        assert!(
            tokio::time::timeout(Duration::from_millis(100), ws.next())
                .await
                .is_err(),
            "same lane must wait for its terminal"
        );
        send(&mut ws, completed("b", "b")).await;
        send(&mut ws, completed("a1", "a")).await;
        let third = next(&mut ws).await;
        assert_eq!(third["stream_id"], "a");
        assert_eq!(third["input"], "second a");
        send(&mut ws, completed("a2", "a")).await;
        assert!(
            tokio::time::timeout(Duration::from_millis(50), listener.accept())
                .await
                .is_err(),
            "concurrent first requests must deduplicate the upgrade"
        );
        let _ = ws.next().await;
    });
    let (url, task) = helper(1).await;
    let mut ws = client(&format!("{url}/responses"), &[]).await;
    send(&mut ws, create("a")).await;
    let mut second = create("a");
    second["input"] = json!("second a");
    send(&mut ws, second).await;
    send(&mut ws, create("b")).await;
    for lane in ["b", "a", "a"] {
        assert_eq!(next(&mut ws).await["stream_id"], lane);
    }
    ws.close(None).await.unwrap();
    drop(ws);
    finish(task).await;
    finish(upstream).await;
}

#[tokio::test]
async fn helper_ws_dead_connection_rejects_cached_continuation_without_replay() {
    if isolated("helper_ws_dead_connection_rejects_cached_continuation_without_replay").await {
        return;
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let _fixture = Fixture::new(profile(listener.local_addr().unwrap()));
    let (closed_tx, closed_rx) = tokio::sync::oneshot::channel();
    let upstream = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();
        next(&mut ws).await;
        send(&mut ws, completed("cached", "")).await;
        ws.close(None).await.unwrap();
        drop(ws);
        let _ = closed_tx.send(());
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();
        let event = next(&mut ws).await;
        assert!(event.get("previous_response_id").is_none());
        send(&mut ws, completed("fresh", "")).await;
        let _ = ws.next().await;
    });
    let (url, task) = helper(1).await;
    let mut ws = client(&format!("{url}/v1/responses"), &[]).await;
    send(&mut ws, create("")).await;
    next(&mut ws).await;
    closed_rx.await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut continuation = create("");
    continuation["previous_response_id"] = json!("cached");
    send(&mut ws, continuation).await;
    assert_eq!(next(&mut ws).await["status"], 400);
    send(&mut ws, create("")).await;
    assert_eq!(next(&mut ws).await["response"]["id"], "fresh");
    ws.close(None).await.unwrap();
    drop(ws);
    finish(task).await;
    finish(upstream).await;
}

#[tokio::test]
async fn helper_ws_http_200_is_rejected_and_never_posts_a_generation() {
    if isolated("helper_ws_http_200_is_rejected_and_never_posts_a_generation").await {
        return;
    }
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let _fixture = Fixture::new(profile(listener.local_addr().unwrap()));
    let upstream = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        while !bytes.ends_with(b"\r\n\r\n") {
            bytes.push(socket.read_u8().await.unwrap());
        }
        assert!(bytes.starts_with(b"GET /v1/responses HTTP/1.1"));
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}")
            .await
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(100), listener.accept())
                .await
                .is_err()
        );
    });
    let (url, task) = helper(1).await;
    let mut ws = client(&format!("{url}/v1/responses"), &[]).await;
    send(&mut ws, create("")).await;
    assert_eq!(next(&mut ws).await["status"], 502);
    ws.close(None).await.unwrap();
    drop(ws);
    finish(task).await;
    finish(upstream).await;
}

#[tokio::test]
async fn helper_ws_chat_compatibility_route_is_explicitly_rejected() {
    if isolated("helper_ws_chat_compatibility_route_is_explicitly_rejected").await {
        return;
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut relay = profile(listener.local_addr().unwrap());
    relay["protocol"] = json!("chatCompletions");
    let _fixture = Fixture::new(relay);
    let (url, task) = helper(1).await;
    let mut ws = client(&format!("{url}/v1/responses"), &[]).await;
    send(&mut ws, create("")).await;
    assert_eq!(next(&mut ws).await["status"], 400);
    assert!(
        tokio::time::timeout(Duration::from_millis(50), listener.accept())
            .await
            .is_err()
    );
    ws.close(None).await.unwrap();
    drop(ws);
    finish(task).await;
}

#[tokio::test]
async fn gateway_ws_authenticates_inherits_child_model_and_releases_key_slots() {
    if isolated("gateway_ws_authenticates_inherits_child_model_and_releases_key_slots").await {
        return;
    }
    use crate::gateway::key_pool::{self, KeyLimitState, KeyLimits};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let relay = profile(listener.local_addr().unwrap());
    let fixture = Fixture::new(relay.clone());
    fixture.gateway(relay);
    let (observed_tx, mut observed_rx) = tokio::sync::mpsc::channel(4);
    let upstream = tokio::spawn(async move {
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..2 {
            let (socket, _) = listener.accept().await.unwrap();
            let observed_tx = observed_tx.clone();
            tasks.spawn(async move {
                let mut ws = tokio_tungstenite::accept_hdr_async(
                    socket,
                    |request: &http::Request<()>, response| {
                        assert_eq!(request.headers()["authorization"], "Bearer sk-test");
                        Ok(response)
                    },
                )
                .await
                .unwrap();
                let event = next(&mut ws).await;
                observed_tx.send(event.clone()).await.unwrap();
                let id = if event["input"] == "parent" {
                    "parent"
                } else {
                    "child"
                };
                send(&mut ws, completed(id, "")).await;
                let _ = ws.next().await;
            });
        }
        while let Some(result) = tasks.join_next().await {
            result.unwrap();
        }
    });
    let (url, task) = helper(3).await;
    let endpoint = format!("{url}/gw/codex/v1/responses");
    let err = tokio_tungstenite::connect_async(&endpoint)
        .await
        .unwrap_err();
    assert!(
        matches!(err, tokio_tungstenite::tungstenite::Error::Http(response) if response.status() == 401)
    );
    let mut parent = client(
        &endpoint,
        &[
            ("authorization", "Bearer local-test".into()),
            ("thread-id", "ws-parent".into()),
        ],
    )
    .await;
    let mut event = create("");
    event["input"] = json!("parent");
    event["model"] = json!("ws-test/ws-model");
    event["client_metadata"] = json!({"x-codex-turn-metadata":json!({"thread_id":"ws-parent","turn_id":"ws-parent-turn"}).to_string()});
    send(&mut parent, event).await;
    let result = next(&mut parent).await;
    assert_eq!(result["type"], "response.completed", "{result}");
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), observed_rx.recv())
            .await
            .unwrap()
            .unwrap()["model"],
        "ws-model"
    );
    // 子线程通过同一网关入口；逐请求 metadata 必须覆盖握手时的 thread-id。
    let mut child = client(
        &endpoint,
        &[
            ("authorization", "Bearer local-test".into()),
            ("thread-id", "handshake-thread".into()),
        ],
    )
    .await;
    let mut event = create("");
    event["input"] = json!("child");
    event["model"] = json!("review-model");
    event["client_metadata"] = json!({"x-openai-subagent":"collab_spawn","x-codex-parent-thread-id":"ws-parent",
        "x-codex-turn-metadata":json!({"thread_id":"ws-child","turn_id":"ws-child-turn","parent_thread_id":"ws-parent",
            "parent_turn_id":"ws-parent-turn","subagent_kind":"thread_spawn"}).to_string()});
    send(&mut child, event).await;
    let result = next(&mut child).await;
    assert_eq!(result["type"], "response.completed", "{result}");
    let observed = tokio::time::timeout(Duration::from_secs(5), observed_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(observed["model"], "ws-model");
    assert_eq!(
        observed["client_metadata"]["x-openai-subagent"],
        "collab_spawn"
    );
    let gateway = GatewayStore::default().load().unwrap();
    let plan = crate::gateway::route::resolve_plan(
        &gateway,
        &crate::tools::ToolId::Codex,
        "ws-model",
        None,
    )
    .unwrap();
    assert_eq!(
        key_pool::limit_state(
            &plan.candidates[0].key,
            KeyLimits {
                concurrency: Some(1),
                rpm: None
            },
            std::time::Instant::now()
        ),
        KeyLimitState::Free
    );
    parent.close(None).await.unwrap();
    child.close(None).await.unwrap();
    drop(parent);
    drop(child);
    finish(task).await;
    finish(upstream).await;
    assert!(fixture.temp.path().join("gateway.json").is_file());
}

#[tokio::test]
async fn gateway_ws_cached_response_keeps_its_key_when_another_key_is_available() {
    if isolated("gateway_ws_cached_response_keeps_its_key_when_another_key_is_available").await {
        return;
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut relay = profile(listener.local_addr().unwrap());
    relay["apiKeys"] = json!([{"id":"ws-affinity-a","name":"A","apiKey":"sk-test-a"},{"id":"ws-affinity-b","name":"B","apiKey":"sk-test-b"}]);
    relay["activeApiKeyId"] = json!("ws-affinity-a");
    let fixture = Fixture::new(relay.clone());
    fixture.gateway(relay);
    let upstream = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();
        next(&mut ws).await;
        send(&mut ws, completed("pinned", "")).await;
        let event = next(&mut ws).await;
        assert_eq!(event["previous_response_id"], "pinned");
        send(&mut ws, completed("pinned-next", "")).await;
        let _ = ws.next().await;
    });
    let (url, task) = helper(1).await;
    let mut ws = client(
        &format!("{url}/gw/codex/v1/responses"),
        &[("authorization", "Bearer local-test".into())],
    )
    .await;
    send(&mut ws, create("")).await;
    next(&mut ws).await;
    let gateway = GatewayStore::default().load().unwrap();
    let plan = crate::gateway::route::resolve_plan(
        &gateway,
        &crate::tools::ToolId::Codex,
        "ws-model",
        None,
    )
    .unwrap();
    let key = plan
        .candidates
        .iter()
        .find(|candidate| candidate.key.key_id == "ws-affinity-a")
        .unwrap()
        .key
        .clone();
    let mut response = create("");
    response["previous_response_id"] = json!("pinned");
    // 路由提前保留冷却中的原 Key，不把可用的 B 当成续接候选。
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("retry-after", "1".parse().unwrap());
    let class = crate::gateway::key_pool::classify_failure(429, &headers, b"rate limited");
    crate::gateway::key_pool::record_failure(&key, &class, std::time::Instant::now());
    send(&mut ws, response).await;
    assert_eq!(next(&mut ws).await["response"]["id"], "pinned-next");
    ws.close(None).await.unwrap();
    drop(ws);
    finish(task).await;
    finish(upstream).await;
}

#[tokio::test]
async fn helper_ws_connection_error_reaches_all_named_lanes_and_releases_them() {
    if isolated("helper_ws_connection_error_reaches_all_named_lanes_and_releases_them").await {
        return;
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let _fixture = Fixture::new(profile(listener.local_addr().unwrap()));
    let upstream = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();
        next(&mut ws).await;
        next(&mut ws).await;
        send(&mut ws, json!({"type":"error","status":429,"error":{"code":"rate_limit_exceeded","message":"limited"}})).await;
        let event = next(&mut ws).await;
        let lane = event["stream_id"].as_str().unwrap();
        send(&mut ws, completed("after-error", lane)).await;
        let _ = ws.next().await;
    });
    let (url, task) = helper(1).await;
    let mut ws = client(&format!("{url}/v1/responses"), &[]).await;
    send(&mut ws, create("a")).await;
    send(&mut ws, create("b")).await;
    let a = next(&mut ws).await;
    let b = next(&mut ws).await;
    assert_eq!(a["type"], "error");
    assert_eq!(b["type"], "error");
    assert_ne!(a["stream_id"], b["stream_id"]);
    send(&mut ws, create("a")).await;
    assert_eq!(next(&mut ws).await["response"]["id"], "after-error");
    ws.close(None).await.unwrap();
    drop(ws);
    finish(task).await;
    finish(upstream).await;
}

// 每个端到端用例在独立测试子进程中覆盖状态目录和 CODEX_HOME。
// 不改父进程环境，单独 cargo test 也不会把 fixture 写入用户日志或子代理 registry。
async fn isolated(name: &str) -> bool {
    if std::env::var_os("CODEX_PLUS_WS_TEST_CHILD").is_some() {
        return false;
    }
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("home")).unwrap();
    let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            &format!("responses_websocket::tests::{name}"),
            "--nocapture",
            "--test-threads=1",
        ])
        .env("CODEX_PLUS_WS_TEST_CHILD", "1")
        .env(crate::paths::STATE_DIR_ENV, temp.path().join("state"))
        .env("CODEX_HOME", temp.path().join("home"))
        .kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(25), command.output())
        .await
        .expect("isolated WS test timed out")
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    true
}

#[tokio::test]
async fn helper_ws_preserves_first_frame_arriving_with_upgrade_headers() {
    if isolated("helper_ws_preserves_first_frame_arriving_with_upgrade_headers").await {
        return;
    }
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let _fixture = Fixture::new(profile(listener.local_addr().unwrap()));
    let upstream = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();
        assert_eq!(next(&mut ws).await["input"], "hello");
        send(&mut ws, completed("early", "")).await;
        let _ = ws.next().await;
    });
    let (url, task) = helper(1).await;
    let address = url.strip_prefix("ws://").unwrap();
    let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
    let payload = create("").to_string().into_bytes();
    assert!(payload.len() < 126);
    let mut wire = format!("GET /v1/responses HTTP/1.1\r\nHost: {address}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n").into_bytes();
    let mask = [1u8, 2, 3, 4];
    wire.extend([0x81, 0x80 | payload.len() as u8]);
    wire.extend(mask);
    wire.extend(
        payload
            .iter()
            .enumerate()
            .map(|(i, byte)| byte ^ mask[i % 4]),
    );
    socket.write_all(&wire).await.unwrap();
    let mut headers = Vec::new();
    while !headers.ends_with(b"\r\n\r\n") {
        headers.push(socket.read_u8().await.unwrap());
    }
    assert!(headers.starts_with(b"HTTP/1.1 101"));
    let mut ws = WebSocketStream::from_raw_socket(socket, Role::Client, None).await;
    assert_eq!(next(&mut ws).await["response"]["id"], "early");
    ws.close(None).await.unwrap();
    drop(ws);
    finish(task).await;
    finish(upstream).await;
}

#[tokio::test]
async fn gateway_ws_cancelled_handshake_refunds_rpm_and_never_records_success() {
    if isolated("gateway_ws_cancelled_handshake_refunds_rpm_and_never_records_success").await {
        return;
    }
    use crate::gateway::key_pool::{self, KeyLimitState, KeyLimits};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let relay = profile(listener.local_addr().unwrap());
    let fixture = Fixture::new(relay.clone());
    fixture.gateway(relay);
    GatewayStore::default()
        .update(|settings| {
            settings.providers[0].key_rpm = Some(1);
            Ok(())
        })
        .unwrap();
    crate::gateway::activity::reset();
    let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
    let upstream = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let _ = accepted_tx.send(());
        use tokio::io::AsyncReadExt;
        let mut bytes = Vec::new();
        let _ = socket.read_to_end(&mut bytes).await;
        assert!(bytes.starts_with(b"GET "));
        assert!(!bytes.windows(15).any(|slice| slice == b"response.create"));
    });
    let (url, task) = helper(1).await;
    let mut ws = client(
        &format!("{url}/gw/codex/v1/responses"),
        &[("authorization", "Bearer local-test".into())],
    )
    .await;
    send(&mut ws, create("")).await;
    tokio::time::timeout(Duration::from_secs(5), accepted_rx)
        .await
        .unwrap()
        .unwrap();
    ws.close(None).await.unwrap();
    drop(ws);
    finish(task).await;
    finish(upstream).await;
    let gateway = GatewayStore::default().load().unwrap();
    let plan = crate::gateway::route::resolve_plan(
        &gateway,
        &crate::tools::ToolId::Codex,
        "ws-model",
        None,
    )
    .unwrap();
    assert_eq!(
        key_pool::limit_state(
            &plan.candidates[0].key,
            KeyLimits {
                concurrency: Some(1),
                rpm: Some(1)
            },
            std::time::Instant::now()
        ),
        KeyLimitState::Free
    );
    assert!(
        crate::gateway::activity::snapshot()
            .agents
            .iter()
            .all(|agent| agent
                .last
                .as_ref()
                .is_none_or(|record| record.result != "answered"))
    );
}

#[tokio::test]
async fn helper_ws_upstream_invalid_accept_is_rejected_before_generation() {
    if isolated("helper_ws_upstream_invalid_accept_is_rejected_before_generation").await {
        return;
    }
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let _fixture = Fixture::new(profile(listener.local_addr().unwrap()));
    let upstream = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        while !bytes.ends_with(b"\r\n\r\n") {
            bytes.push(socket.read_u8().await.unwrap());
        }
        socket.write_all(b"HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Accept: invalid\r\n\r\n").await.unwrap();
        let mut rest = Vec::new();
        let _ = socket.read_to_end(&mut rest).await;
        assert!(rest.is_empty());
    });
    let (url, task) = helper(1).await;
    let mut ws = client(&format!("{url}/v1/responses"), &[]).await;
    send(&mut ws, create("")).await;
    assert_eq!(next(&mut ws).await["status"], 502);
    ws.close(None).await.unwrap();
    drop(ws);
    finish(task).await;
    finish(upstream).await;
}

#[tokio::test]
async fn helper_ws_default_lane_error_does_not_end_a_named_response() {
    if isolated("helper_ws_default_lane_error_does_not_end_a_named_response").await {
        return;
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let _fixture = Fixture::new(profile(listener.local_addr().unwrap()));
    let upstream = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();
        next(&mut ws).await;
        next(&mut ws).await;
        send(&mut ws, json!({"type":"error","status":400,"error":{"code":"invalid_request","message":"default request error"}})).await;
        send(&mut ws, completed("named-still-running", "named")).await;
        let _ = ws.next().await;
    });
    let (url, task) = helper(1).await;
    let mut ws = client(&format!("{url}/v1/responses"), &[]).await;
    send(&mut ws, create("")).await;
    send(&mut ws, create("named")).await;
    let error = next(&mut ws).await;
    assert_eq!(error["type"], "error");
    assert!(error.get("stream_id").is_none());
    let event = next(&mut ws).await;
    assert_eq!(event["type"], "response.completed");
    assert_eq!(event["stream_id"], "named");
    ws.close(None).await.unwrap();
    drop(ws);
    finish(task).await;
    finish(upstream).await;
}

#[tokio::test]
async fn gateway_ws_incomplete_output_does_not_punish_a_healthy_provider() {
    if isolated("gateway_ws_incomplete_output_does_not_punish_a_healthy_provider").await {
        return;
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let relay = profile(listener.local_addr().unwrap());
    let fixture = Fixture::new(relay.clone());
    fixture.gateway(relay);
    crate::gateway::activity::reset();
    let upstream = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();
        for _ in 0..4 {
            next(&mut ws).await;
            send(&mut ws,json!({"type":"response.incomplete","response":{"id":"incomplete","status":"incomplete","error":null,
                "incomplete_details":{"reason":"max_output_tokens"},"usage":{"input_tokens":3,"output_tokens":1}}})).await;
        }
        next(&mut ws).await;
        send(&mut ws, completed("healthy", "")).await;
        let _ = ws.next().await;
    });
    let (url, task) = helper(1).await;
    let mut ws = client(
        &format!("{url}/gw/codex/v1/responses"),
        &[("authorization", "Bearer local-test".into())],
    )
    .await;
    for _ in 0..4 {
        send(&mut ws, create("")).await;
        assert_eq!(next(&mut ws).await["type"], "response.incomplete");
    }
    assert!(!crate::gateway::key_pool::breaker_open(
        "ws-test",
        std::time::Instant::now()
    ));
    send(&mut ws, create("")).await;
    assert_eq!(next(&mut ws).await["response"]["id"], "healthy");
    ws.close(None).await.unwrap();
    drop(ws);
    finish(task).await;
    finish(upstream).await;
    let activity = crate::gateway::activity::snapshot();
    let agent = activity
        .agents
        .iter()
        .find(|agent| agent.agent == "codex")
        .unwrap();
    assert_eq!(agent.requests, 5);
    assert_eq!(agent.errors, 4);
    assert_eq!(agent.last.as_ref().unwrap().result, "answered");
    assert!(agent.in_flight.is_empty());
}

#[test]
fn websocket_error_without_http_status_preserves_failure_classification() {
    use crate::gateway::key_pool::{FailureClass, classify_failure};
    for (error, expected) in [
        (
            json!({"type":"invalid_request_error","code":"invalid_previous_response_id"}),
            FailureClass::ClientError,
        ),
        (
            json!({"type":"authentication_error","code":"invalid_api_key"}),
            FailureClass::AuthInvalid,
        ),
        (
            json!({"code":"rate_limit_exceeded"}),
            FailureClass::RateLimited { retry_after: None },
        ),
        (json!({"code":"server_error"}), FailureClass::ServerError),
    ] {
        let event = json!({"type":"error","error":error});
        assert_eq!(
            classify_failure(
                error_status(&event),
                &reqwest::header::HeaderMap::new(),
                event.to_string().as_bytes()
            ),
            expected
        );
    }
}
