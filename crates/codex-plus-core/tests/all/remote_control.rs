//! 远程控制（P1）：版本表、IPC 客户端、会话视图、HTTP 安全检查与状态持久化。
//! 端到端链路（真实 Codex 桌面）由 tools/remote-control-p0/run-p0.mjs 覆盖。

use std::io::Write;

use codex_plus_core::remote_control::follower::{build_offline_view, build_view, response_for, turns_of};
use codex_plus_core::remote_control::ipc::encode_frame;
use codex_plus_core::remote_control::server::{host_allowed, origin_allowed, parse_head, token_matches, websocket_accept};
use codex_plus_core::remote_control::versions::{default_versions, extract_versions, read_asar_file};
use codex_plus_core::remote_control::{RemoteControlState, generate_token, load_state_from, page_url, save_state_to};
use serde_json::{Value, json};

/// 按 asar 格式拼一个只含若干文件的包：16 字节头 + JSON 目录 + 文件数据（数据从 8 + headerSize 开始）。
fn write_asar(files: &[(&str, &[u8])]) -> tempfile::NamedTempFile {
    let mut tree = json!({ "files": {} });
    let mut data = Vec::new();
    for (path, bytes) in files {
        let mut node = &mut tree;
        let parts: Vec<&str> = path.split('/').collect();
        for part in &parts[..parts.len() - 1] {
            node = node["files"].as_object_mut().unwrap().entry(part.to_string()).or_insert(json!({ "files": {} }));
        }
        node["files"][parts[parts.len() - 1]] = json!({ "size": bytes.len(), "offset": data.len().to_string() });
        data.extend_from_slice(bytes);
    }
    let json_bytes = serde_json::to_vec(&tree).unwrap();
    let header_size = (8 + json_bytes.len()) as u32;
    let mut out = Vec::new();
    out.extend_from_slice(&4u32.to_le_bytes());
    out.extend_from_slice(&header_size.to_le_bytes());
    out.extend_from_slice(&((4 + json_bytes.len()) as u32).to_le_bytes());
    out.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(&json_bytes);
    out.extend_from_slice(&data);
    let mut file = tempfile::NamedTempFile::new().unwrap();
    file.write_all(&out).unwrap();
    file
}

#[test]
fn extracts_ipc_versions_from_main_entry() {
    let source = r#"var x={a:1};let iI={"thread-stream-state-changed":13,"thread-stream-following-changed":1,"ipc-connection-reset":1,"thread-read-state-changed":3,"thread-archived":2,"thread-unarchived":1,"thread-owner-discovery":1,"thread-follower-start-turn":2,"thread-follower-steer-turn":1,"thread-follower-interrupt-turn":4,"thread-follower-command-approval-decision":1};function iS(e){}"#;
    let versions = extract_versions(source).unwrap();
    assert_eq!(versions["thread-stream-state-changed"], 13);
    assert_eq!(versions["thread-follower-interrupt-turn"], 4);
    assert!(extract_versions("no table here").is_err());
}

#[test]
fn reads_file_out_of_asar_archive() {
    let asar = write_asar(&[
        ("package.json", b"{\"version\":\"26.1111\"}"),
        (".vite/build/early-bootstrap.js", b"console.log(1)"),
    ]);
    assert_eq!(read_asar_file(asar.path(), ".vite/build/early-bootstrap.js").unwrap(), b"console.log(1)");
    assert_eq!(read_asar_file(asar.path(), "package.json").unwrap(), b"{\"version\":\"26.1111\"}");
    assert!(read_asar_file(asar.path(), "missing.js").is_err());
}

#[test]
fn default_versions_cover_every_follower_method_we_send() {
    let versions = default_versions();
    for method in [
        "thread-owner-discovery",
        "thread-stream-following-changed",
        "thread-follower-load-complete-history",
        "thread-follower-start-turn",
        "thread-follower-steer-turn",
        "thread-follower-interrupt-turn",
        "thread-follower-command-approval-decision",
        "thread-follower-file-approval-decision",
        "thread-follower-submit-user-input",
        "thread-follower-permissions-request-approval-response",
        "thread-follower-submit-mcp-server-elicitation-response",
    ] {
        assert!(versions.contains_key(method), "缺少 {method}");
    }
}

#[test]
fn ipc_frames_are_little_endian_length_prefixed_json() {
    let frame = encode_frame(&json!({ "type": "broadcast" })).unwrap();
    let body = br#"{"type":"broadcast"}"#;
    assert_eq!(&frame[..4], &(body.len() as u32).to_le_bytes());
    assert_eq!(&frame[4..], body);
}

#[cfg(unix)]
#[tokio::test]
async fn ipc_client_handshakes_requests_and_ignores_discovery() {
    use codex_plus_core::remote_control::ipc::{IpcConnection, IpcEndpoint};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn read_message(stream: &mut tokio::net::UnixStream) -> Value {
        let mut head = [0u8; 4];
        stream.read_exact(&mut head).await.unwrap();
        let mut body = vec![0u8; u32::from_le_bytes(head) as usize];
        stream.read_exact(&mut body).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }
    async fn write_message(stream: &mut tokio::net::UnixStream, message: Value) {
        stream.write_all(&encode_frame(&message).unwrap()).await.unwrap();
    }

    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("ipc.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    // 假路由器：initialize → 分配 clientId；先发一条 discovery 看客户端是否回 canHandle:false，
    // 再回答 owner-discovery，最后推一条广播。
    let router = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let init = read_message(&mut stream).await;
        assert_eq!(init["method"], "initialize");
        assert_eq!(init["params"]["clientType"], "test-client");
        write_message(&mut stream, json!({ "type": "response", "requestId": init["requestId"], "resultType": "success", "result": { "clientId": "client-1" } })).await;

        write_message(&mut stream, json!({ "type": "client-discovery-request", "requestId": "disc-1", "request": { "method": "x" } })).await;
        let discovery = read_message(&mut stream).await;
        assert_eq!(discovery, json!({ "type": "client-discovery-response", "requestId": "disc-1", "response": { "canHandle": false } }));

        let request = read_message(&mut stream).await;
        assert_eq!(request["method"], "thread-owner-discovery");
        assert_eq!(request["sourceClientId"], "client-1");
        assert_eq!(request["version"], 1);
        write_message(&mut stream, json!({ "type": "response", "requestId": request["requestId"], "resultType": "success", "handledByClientId": "owner-9", "result": {} })).await;

        write_message(&mut stream, json!({ "type": "broadcast", "method": "thread-stream-state-changed", "sourceClientId": "owner-9", "params": { "conversationId": "t" } })).await;
        // 保持连接直到客户端读完
        let _ = read_message(&mut stream).await;
    });

    let conn = IpcConnection::connect(&IpcEndpoint::Socket(socket), "test-client", default_versions()).await.unwrap();
    assert_eq!(conn.client_id(), "client-1");
    let mut events = conn.subscribe();
    let response = conn
        .request("thread-owner-discovery", json!({ "hostId": "local", "conversationId": "t" }), None, std::time::Duration::from_secs(5))
        .await
        .unwrap();
    assert!(response.ok());
    assert_eq!(response.handled_by(), Some("owner-9"));
    let event = tokio::time::timeout(std::time::Duration::from_secs(5), events.recv()).await.unwrap().unwrap();
    assert_eq!(event["method"], "thread-stream-state-changed");
    conn.broadcast("thread-stream-following-changed", json!({ "following": false })).await.unwrap();
    router.await.unwrap();
}

#[test]
fn response_for_maps_request_methods_to_follower_methods() {
    let id = json!(3);
    let (method, params) = response_for("item/commandExecution/requestApproval", &id, &json!({ "decision": "accept" })).unwrap();
    assert_eq!(method, "thread-follower-command-approval-decision");
    assert_eq!(params, json!({ "requestId": 3, "decision": "accept" }));

    let (method, params) = response_for("item/tool/requestUserInput", &id, &json!({ "answers": { "color": { "answers": ["蓝色"] } } })).unwrap();
    assert_eq!(method, "thread-follower-submit-user-input");
    assert_eq!(params["response"]["answers"]["color"]["answers"][0], "蓝色");

    assert_eq!(response_for("item/fileChange/requestApproval", &id, &json!({ "decision": "decline" })).unwrap().0, "thread-follower-file-approval-decision");
    assert!(response_for("item/commandExecution/requestApproval", &id, &json!({})).is_err(), "缺 decision 要报错");
    assert!(response_for("something/else", &id, &json!({})).is_err());
}

fn sample_state() -> Value {
    json!({
        "title": "远程测试",
        "cwd": "/work",
        "latestModel": "mock-model",
        "threadRuntimeStatus": { "type": "active" },
        "requests": [{ "method": "item/commandExecution/requestApproval", "id": 0, "params": { "command": "ls" } }],
        "turns": [],
        "turnHistory": { "kind": "canonical", "history": {
            "islands": [{ "entries": [{ "key": "a", "value": "a" }, { "key": "b", "value": "b" }] }],
            "entitiesByKey": {
                "a": { "turnId": "t1", "status": "completed", "items": [
                    { "type": "userMessage", "content": [{ "type": "text", "text": "你好" }] },
                    { "type": "agentMessage", "text": "收到" }
                ] },
                "b": { "turnId": "t2", "status": "inProgress", "items": [
                    { "type": "commandExecution", "command": "ls", "status": "inProgress", "aggregatedOutput": "x".repeat(20000) },
                    { "type": "steeringUserMessage", "input": [{ "type": "text", "text": "插一句" }] }
                ] }
            }
        } }
    })
}

#[test]
fn view_flattens_turn_history_in_display_order() {
    let state = sample_state();
    let turns = turns_of(&state);
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[1]["turnId"], "t2");

    let view = build_view("thread-1", &state, Some(7));
    assert_eq!(view["live"], true);
    assert_eq!(view["revision"], 7);
    assert_eq!(view["runtime"], "active");
    assert_eq!(view["turns"][0]["items"][0]["text"], "你好", "用户消息补 text");
    assert_eq!(view["turns"][1]["items"][1]["text"], "插一句", "插话消息补 text");
    let output = view["turns"][1]["items"][0]["aggregatedOutput"].as_str().unwrap();
    assert!(output.len() < 9000 && output.starts_with('…'), "过长的命令输出要截断");
    assert_eq!(view["requests"][0]["id"], 0);
}

#[test]
fn offline_view_reads_sidecar_thread() {
    let thread = json!({
        "id": "thread-2", "name": null, "preview": "第一句话", "cwd": "/w", "model": "m",
        "turns": [{ "id": "t1", "status": "completed", "items": [{ "type": "userMessage", "content": [{ "type": "text", "text": "第一句话" }] }] }]
    });
    let view = build_offline_view(&thread);
    assert_eq!(view["live"], false);
    assert_eq!(view["title"], "第一句话", "没有名字时用 preview");
    assert_eq!(view["turns"][0]["items"][0]["text"], "第一句话");
    assert_eq!(view["requests"], json!([]));
}

#[test]
fn only_loopback_hosts_and_same_origin_are_allowed() {
    assert!(host_allowed(Some("127.0.0.1:57360"), 57360));
    assert!(host_allowed(Some("localhost:57360"), 57360));
    assert!(!host_allowed(Some("evil.example:57360"), 57360), "DNS 重绑定");
    assert!(!host_allowed(Some("127.0.0.1:1"), 57360));
    assert!(!host_allowed(None, 57360));

    assert!(origin_allowed(None, "127.0.0.1:57360"));
    assert!(origin_allowed(Some("http://127.0.0.1:57360"), "127.0.0.1:57360"));
    assert!(!origin_allowed(Some("https://evil.example"), "127.0.0.1:57360"));
}

#[test]
fn token_comparison_rejects_empty_and_mismatched_tokens() {
    let token = generate_token();
    assert_eq!(token.len(), 64);
    assert!(token_matches(&token, &token));
    assert!(!token_matches(&token, &generate_token()));
    assert!(!token_matches(&token, &token[..63]));
    assert!(!token_matches("", ""), "没有令牌时一律拒绝");
}

#[test]
fn parses_request_head_and_query() {
    let request = parse_head("GET /api/threads/abc/stream?token=a%2Bb+c&x=1 HTTP/1.1\r\nHost: 127.0.0.1:57360\r\nUpgrade: websocket\r\nAuthorization: Bearer t").unwrap();
    assert_eq!(request.method, "GET");
    assert_eq!(request.path, "/api/threads/abc/stream");
    assert_eq!(request.query["token"], "a+b c");
    assert_eq!(request.headers["host"], "127.0.0.1:57360");
    assert_eq!(request.headers["upgrade"], "websocket");
}

#[test]
fn websocket_accept_matches_rfc_example() {
    // RFC 6455 第 1.3 节的示例。
    assert_eq!(websocket_accept("dGhlIHNhbXBsZSBub25jZQ=="), "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
}

#[test]
fn state_round_trips_and_defaults_to_disabled() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("remote-control.json");
    let missing = load_state_from(&path);
    assert!(!missing.enabled, "默认关闭");
    assert!(missing.token.is_empty());

    let state = RemoteControlState { enabled: true, port: 57361, token: "abc".into() };
    save_state_to(&path, &state).unwrap();
    assert_eq!(load_state_from(&path), state);
    assert_eq!(page_url(&state), "http://127.0.0.1:57361/#token=abc", "令牌放在 URL 片段里，不随请求发出");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600, "令牌文件只给本人读");
    }

    std::fs::write(&path, b"not json").unwrap();
    assert_eq!(load_state_from(&path), RemoteControlState::default(), "坏文件按默认处理");
}

/// 服务启停：起在空闲端口上，网页可访问、`/api` 没令牌被拒、Host 不对被拒，停掉后端口释放。
/// 用的是进程级单例，先拿全局锁。
#[test]
fn server_starts_serves_page_and_stops() {
    use std::io::{Read, Write as _};
    let _guard = crate::common::global_state_lock();
    let port = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port();
    let token = generate_token();
    codex_plus_core::remote_control::start(port, &token).unwrap();
    assert_eq!(codex_plus_core::remote_control::running_port(), Some(port));

    let get = |path: &str, host: &str, auth: Option<&str>| -> String {
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        let auth = auth.map(|token| format!("Authorization: Bearer {token}\r\n")).unwrap_or_default();
        write!(stream, "GET {path} HTTP/1.1\r\nHost: {host}\r\n{auth}Connection: close\r\n\r\n").unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    };
    let host = format!("127.0.0.1:{port}");
    let page = get("/", &host, None);
    assert!(page.starts_with("HTTP/1.1 200"), "{page}");
    assert!(page.contains("Codex++ 远程控制"));
    assert!(page.contains("frame-ancestors 'none'"));
    assert!(get("/api/status", &host, None).starts_with("HTTP/1.1 401"));
    assert!(get("/api/status", "evil.example", Some(&token)).starts_with("HTTP/1.1 421"));
    assert!(get("/api/nothing", &host, Some(&token)).starts_with("HTTP/1.1 400"));

    codex_plus_core::remote_control::stop();
    assert_eq!(codex_plus_core::remote_control::running_port(), None);
    assert!(std::net::TcpStream::connect(("127.0.0.1", port)).is_err(), "停掉后端口要释放");
}
