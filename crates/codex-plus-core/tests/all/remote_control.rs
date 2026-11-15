//! 远程控制：版本表、IPC 客户端、会话视图、HTTP 安全检查、状态持久化，
//! 以及 P2 的配对设备、权限分级与局域网 HTTPS 入口。
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

    assert!(origin_allowed(None, "127.0.0.1:57360", "http"));
    assert!(origin_allowed(Some("http://127.0.0.1:57360"), "127.0.0.1:57360", "http"));
    assert!(!origin_allowed(Some("https://evil.example"), "127.0.0.1:57360", "http"));
    assert!(origin_allowed(Some("https://192.168.1.8:57361"), "192.168.1.8:57361", "https"));
    assert!(!origin_allowed(Some("http://192.168.1.8:57361"), "192.168.1.8:57361", "https"), "局域网入口是 https");
}

#[test]
fn lan_entry_accepts_ip_literals_and_local_names_only() {
    use codex_plus_core::remote_control::server::lan_host_allowed;
    assert!(lan_host_allowed(Some("192.168.1.8:57361"), 57361));
    assert!(lan_host_allowed(Some("100.101.102.103:57361"), 57361), "Tailscale 地址");
    assert!(lan_host_allowed(Some("[fe80::1]:57361"), 57361));
    assert!(lan_host_allowed(Some("my-mac.local:57361"), 57361));
    assert!(lan_host_allowed(Some("my-mac.tail1234.ts.net:57361"), 57361), "MagicDNS");
    assert!(!lan_host_allowed(Some("evil.example:57361"), 57361), "外部域名指向内网就是 DNS 重绑定");
    assert!(!lan_host_allowed(Some("192.168.1.8.evil.example:57361"), 57361));
    assert!(!lan_host_allowed(Some("192.168.1.8:1"), 57361));
    assert!(!lan_host_allowed(Some("192.168.1.8"), 57361));
    assert!(!lan_host_allowed(None, 57361));
}

#[test]
fn routes_require_roles_by_capability() {
    use codex_plus_core::remote_control::devices::Role;
    use codex_plus_core::remote_control::server::required_role;
    assert_eq!(required_role("GET", &["threads"]), Role::View);
    assert_eq!(required_role("GET", &["threads", "x", "stream"]), Role::View);
    assert_eq!(required_role("GET", &["threads", "x", "options"]), Role::View);
    assert_eq!(required_role("POST", &["threads"]), Role::Message, "新建会话");
    assert_eq!(required_role("POST", &["threads", "x", "messages"]), Role::Message);
    assert_eq!(required_role("POST", &["threads", "x", "interrupt"]), Role::Message);
    assert_eq!(required_role("POST", &["threads", "x", "open"]), Role::Message);
    assert_eq!(required_role("POST", &["threads", "x", "requests", "3"]), Role::Full, "审批");
    assert_eq!(required_role("POST", &["threads", "x", "settings"]), Role::Full, "改权限模式");
    assert!(Role::View < Role::Message && Role::Message < Role::Full);
}

/// 把状态文件挪到临时目录，结束时恢复。进程级状态，调用方要先拿全局锁。
struct TempState {
    dir: tempfile::TempDir,
}

impl TempState {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        codex_plus_core::remote_control::set_state_path_for_tests(Some(dir.path().join("remote-control.json")));
        Self { dir }
    }

    fn state_text(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("remote-control.json")).unwrap_or_default()
    }
}

impl Drop for TempState {
    fn drop(&mut self) {
        codex_plus_core::remote_control::stop();
        codex_plus_core::remote_control::set_state_path_for_tests(None);
    }
}

#[test]
fn pairing_issues_device_tokens_that_can_be_revoked() {
    use codex_plus_core::remote_control::devices::{self, Principal, Role};
    let _guard = crate::common::global_state_lock();
    let temp = TempState::new();

    assert!(devices::redeem("ABCD2345", "手机").is_err(), "没有进行中的配对");
    let pairing = devices::start_pairing(Role::Message);
    assert_eq!(pairing.code.len(), 8);
    assert!(devices::redeem("WRONG000", "手机").unwrap_err().to_string().contains("不对"));
    // 配对码大小写、空格、横线都不敏感
    let loose = format!("{}-{} ", pairing.code[..4].to_lowercase(), &pairing.code[4..]);
    let (device, token) = devices::redeem(&loose, "  我的\u{7}手机  ").unwrap();
    assert_eq!(device.name, "我的手机", "去掉控制字符和首尾空白");
    assert_eq!(device.role, Role::Message);
    assert_eq!(token.len(), 64);
    assert!(devices::redeem(&pairing.code, "第二台").is_err(), "配对码只能用一次");

    let saved = temp.state_text();
    assert!(!saved.contains(&token), "状态文件里不能有明文设备令牌");
    assert!(saved.contains(&devices::token_digest(&token)));

    match devices::authorize_device(&token, Some("192.168.1.20")) {
        Some(Principal::Device { id, role, .. }) => {
            assert_eq!(id, device.id);
            assert_eq!(role, Role::Message);
        }
        other => panic!("设备令牌应能认出：{other:?}"),
    }
    assert!(devices::authorize_device(&"0".repeat(64), None).is_none());
    assert_eq!(devices::list()[0].last_address.as_deref(), Some("192.168.1.20"), "记下最近来源地址（落盘有节流）");

    // 重新从文件读，设备还在
    devices::reload();
    assert!(devices::authorize_device(&token, None).is_some());
    devices::set_role(&device.id, Role::Full).unwrap();
    assert_eq!(devices::authorize_device(&token, None).unwrap().role(), Role::Full);

    devices::revoke(&device.id).unwrap();
    assert!(devices::authorize_device(&token, None).is_none(), "吊销后立刻失效");
    assert!(!devices::device_exists(&device.id));
    assert!(devices::revoke(&device.id).is_err());
}

#[test]
fn pairing_code_is_voided_after_too_many_wrong_attempts() {
    use codex_plus_core::remote_control::devices::{self, Role};
    let _guard = crate::common::global_state_lock();
    let _temp = TempState::new();
    let pairing = devices::start_pairing(Role::View);
    for _ in 0..4 {
        assert!(devices::redeem("ZZZZZZZZ", "x").is_err());
    }
    assert!(devices::redeem("ZZZZZZZZ", "x").unwrap_err().to_string().contains("过多"));
    assert!(devices::current_pairing().is_none());
    assert!(devices::redeem(&pairing.code, "x").is_err(), "作废后正确的码也不行");
}

#[test]
fn certificate_is_stable_and_grows_with_new_addresses() {
    use codex_plus_core::remote_control::tls;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tls.json");
    let home: std::net::IpAddr = "192.168.1.8".parse().unwrap();
    let office: std::net::IpAddr = "10.0.0.5".parse().unwrap();

    let first = tls::ensure_certificate_at(&path, &[home]).unwrap();
    assert!(first.names.contains(&"192.168.1.8".to_string()));
    assert!(first.names.contains(&"localhost".to_string()));
    let fingerprint = first.fingerprint();
    assert_eq!(fingerprint.split(':').count(), 32, "SHA-256 指纹");
    let days = (first.not_after - time::OffsetDateTime::now_utc().unix_timestamp()) / 86_400;
    assert!((790..=800).contains(&days), "有效期不能超过 iOS 的 825 天上限：{days}");
    assert!(tls::server_config(&first).is_ok());

    let again = tls::ensure_certificate_at(&path, &[home]).unwrap();
    assert_eq!(again.fingerprint(), fingerprint, "地址没变就不重签");

    let moved = tls::ensure_certificate_at(&path, &[office]).unwrap();
    assert_ne!(moved.fingerprint(), fingerprint, "新地址要重签");
    assert!(moved.names.contains(&"10.0.0.5".to_string()));
    assert!(moved.names.contains(&"192.168.1.8".to_string()), "保留旧地址，换回原网络不用再重签");
    assert_eq!(tls::ensure_certificate_at(&path, &[home]).unwrap().fingerprint(), moved.fingerprint());

    assert!(tls::is_tailscale(&"100.100.1.2".parse().unwrap()));
    assert!(!tls::is_tailscale(&"100.200.1.2".parse().unwrap()));
}

/// 局域网 HTTPS 入口：自签证书握手、主令牌不认、配对拿设备令牌、按权限放行、吊销即失效；
/// 本机入口也认设备令牌。只调不碰 Codex 的接口（me / 权限检查），不依赖桌面在跑。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lan_https_entry_pairs_devices_and_enforces_roles() {
    use codex_plus_core::remote_control::devices::{self, Role};
    use codex_plus_core::remote_control::{LanOptions, ListenOptions, tls};
    let _guard = crate::common::global_state_lock();
    let _temp = TempState::new();
    let free_port = || std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port();
    let (port, lan_port) = (free_port(), free_port());
    let token = generate_token();
    let certificate = tls::ensure_certificate_at(&tls::tls_path(), &[]).unwrap();
    let lan = LanOptions { port: lan_port, tls: tls::server_config(&certificate).unwrap(), fingerprint: certificate.fingerprint() };
    tokio::task::block_in_place(|| {
        codex_plus_core::remote_control::start_with(ListenOptions { port, token: token.clone(), lan: Some(lan) }).unwrap()
    });
    assert_eq!(codex_plus_core::remote_control::running_lan_port(), Some(lan_port));

    let client = reqwest::Client::builder().danger_accept_invalid_certs(true).no_proxy().build().unwrap();
    let lan_base = format!("https://127.0.0.1:{lan_port}");
    let get = |base: String, path: &str, bearer: &str| {
        client.get(format!("{base}{path}")).bearer_auth(bearer).send()
    };

    let page = client.get(format!("{lan_base}/")).send().await.unwrap();
    assert_eq!(page.status(), 200);
    assert!(page.headers()["content-security-policy"].to_str().unwrap().contains("wss://127.0.0.1"));
    assert_eq!(get(lan_base.clone(), "/api/me", &token).await.unwrap().status(), 401, "局域网不认主令牌");

    let pair = |code: String| client.post(format!("{lan_base}/api/pair")).json(&json!({ "code": code, "name": "测试手机" })).send();
    assert_eq!(pair("AAAAAAAA".into()).await.unwrap().status(), 400, "没有进行中的配对");
    let pairing = devices::start_pairing(Role::View);
    let paired: Value = pair(pairing.code.clone()).await.unwrap().json().await.unwrap();
    assert_eq!(paired["ok"], true, "{paired}");
    let device_token = paired["data"]["token"].as_str().unwrap().to_string();
    let device_id = paired["data"]["device"]["id"].as_str().unwrap().to_string();
    assert!(paired["data"]["device"].get("tokenSha256").is_none(), "不回传摘要");

    let me: Value = get(lan_base.clone(), "/api/me", &device_token).await.unwrap().json().await.unwrap();
    assert_eq!(me["data"]["role"], "view");
    assert_eq!(me["data"]["name"], "测试手机");
    let thread = uuid::Uuid::new_v4();
    let interrupt = client.post(format!("{lan_base}/api/threads/{thread}/interrupt")).bearer_auth(&device_token).send().await.unwrap();
    assert_eq!(interrupt.status(), 403, "只读设备不能停止会话");
    let cross = client
        .get(format!("{lan_base}/api/me"))
        .bearer_auth(&device_token)
        .header("Origin", "https://evil.example")
        .send()
        .await
        .unwrap();
    assert_eq!(cross.status(), 403);

    // 本机入口也认设备令牌
    let local: Value = get(format!("http://127.0.0.1:{port}"), "/api/me", &device_token).await.unwrap().json().await.unwrap();
    assert_eq!(local["data"]["kind"], "device");
    let master: Value = get(format!("http://127.0.0.1:{port}"), "/api/me", &token).await.unwrap().json().await.unwrap();
    assert_eq!(master["data"]["kind"], "master");

    devices::revoke(&device_id).unwrap();
    assert_eq!(get(lan_base.clone(), "/api/me", &device_token).await.unwrap().status(), 401, "吊销后立刻失效");

    tokio::task::block_in_place(codex_plus_core::remote_control::stop);
    assert_eq!(codex_plus_core::remote_control::running_lan_port(), None);
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

    // P1 写的旧文件没有局域网与设备字段，读出来按默认（局域网关）。
    std::fs::write(&path, br#"{"enabled":true,"port":57360,"token":"t"}"#).unwrap();
    let legacy = load_state_from(&path);
    assert!(legacy.enabled && !legacy.lan_enabled);
    assert_eq!(legacy.lan_port, 57361);
    assert!(legacy.devices.is_empty());

    let state = RemoteControlState { enabled: true, port: 57361, token: "abc".into(), ..Default::default() };
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

#[test]
fn permission_mode_is_derived_like_the_desktop_presets() {
    use codex_plus_core::remote_control::follower::{PERMISSION_PRESETS, permission_mode_of};
    // 每个预设写出去的设置，读回来要还是同一个预设。
    for preset in PERMISSION_PRESETS {
        let settings = json!({
            "activePermissionProfile": { "id": preset.profile, "extends": null },
            "approvalPolicy": preset.approval_policy,
            "approvalsReviewer": preset.reviewer,
        });
        assert_eq!(permission_mode_of(&settings), preset.id, "{}", preset.id);
    }
    // 没有 profile 时按沙箱类型判断（旧会话）
    assert_eq!(permission_mode_of(&json!({ "sandboxPolicy": { "type": "readOnly" }, "approvalPolicy": "on-request" })), "read-only");
    assert_eq!(permission_mode_of(&json!({ "sandboxPolicy": { "type": "dangerFullAccess" }, "approvalPolicy": "never" })), "full-access");
    assert_eq!(permission_mode_of(&json!({ "sandboxPolicy": { "type": "workspaceWrite" }, "approvalPolicy": "on-request", "approvalsReviewer": "auto_review" })), "guardian-approvals");
    // 细粒度审批之类对不上预设的
    assert_eq!(permission_mode_of(&json!({ "activePermissionProfile": { "id": ":workspace" }, "approvalPolicy": { "granular": {} } })), "custom");
    assert_eq!(permission_mode_of(&Value::Null), "custom");
}

#[test]
fn settings_change_parses_and_checks_reflection() {
    use codex_plus_core::remote_control::follower::SettingsChange;
    let change = SettingsChange::from_json(&json!({ "model": "gpt-6.1-sol", "effort": "high", "permissionMode": "full-access", "collaborationMode": "plan" })).unwrap();
    assert_eq!(change.model.as_deref(), Some("gpt-6.1-sol"));
    assert_eq!(change.effort, Some(Some("high".to_string())));
    let reflected = json!({ "model": "gpt-6.1-sol", "effort": "high", "permissionMode": "full-access", "collaborationMode": "plan" });
    assert!(change.is_reflected_in(&reflected));
    assert!(!change.is_reflected_in(&json!({ "model": "gpt-6.1-sol", "effort": "high", "permissionMode": "full-access", "collaborationMode": "default" })), "计划模式还没生效");

    let reset = SettingsChange::from_json(&json!({ "effort": null })).unwrap();
    assert_eq!(reset.effort, Some(None), "null 表示改回模型默认档位");
    assert!(reset.is_reflected_in(&json!({ "effort": null, "model": "x" })));
    assert!(!reset.is_reflected_in(&json!({ "effort": "high" })));

    assert_eq!(SettingsChange::from_json(&json!({})).unwrap(), SettingsChange::default());
    assert!(SettingsChange::from_json(&json!({ "model": 3 })).is_err());
}

#[test]
fn current_controls_prefer_thread_settings() {
    use codex_plus_core::remote_control::follower::current_controls;
    let state = json!({
        "latestModel": "old-model",
        "latestReasoningEffort": "low",
        "latestThreadSettings": {
            "model": "gpt-6-luna", "effort": "high",
            "activePermissionProfile": { "id": ":danger-full-access" }, "approvalPolicy": "never", "approvalsReviewer": "user"
        },
        "latestCollaborationMode": { "mode": "plan", "settings": {} }
    });
    assert_eq!(current_controls(&state), json!({ "model": "gpt-6-luna", "effort": "high", "permissionMode": "full-access", "collaborationMode": "plan" }));
    // 会话设置还没下发时退回 latest*
    let fresh = json!({ "latestModel": "mock-model", "latestReasoningEffort": null });
    assert_eq!(current_controls(&fresh)["model"], "mock-model");
    assert_eq!(current_controls(&fresh)["collaborationMode"], "default");
}

/// 管理器命令与 Codex 页面（bridge `/remote-control`）共用一份分发；bridge 只在管理器进程里放行。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn codex_page_bridge_route_shares_the_manager_dispatch() {
    use codex_plus_core::remote_control::{bridge_route, dispatch, mark_hosted_by_this_process};
    let _guard = crate::common::global_state_lock();
    let _temp = TempState::new();

    // 管理器还没标记托管时（例如 bridge 跑在别的进程）不能在这里起服务。
    // 托管标记是进程级的：同进程里别的用例先标记过就跳过这一步。
    if let Err(error) = bridge_route(&json!({ "action": "status" })) {
        assert!(error.to_string().contains("管理器未运行"), "{error}");
    }
    mark_hosted_by_this_process();

    let status = bridge_route(&json!({ "action": "status" })).unwrap();
    assert_eq!(status["status"], "ok", "bridge 回包带 status: ok");
    assert_eq!(status["enabled"], false);
    assert_eq!(status["lan"]["enabled"], false);
    assert_eq!(status["devices"], json!([]));
    assert_eq!(bridge_route(&json!({})).unwrap()["status"], "ok", "缺省动作是 status");

    assert!(dispatch("nope", &json!({})).unwrap_err().to_string().contains("未知操作"));
    assert!(dispatch("setDeviceRole", &json!({ "id": "x" })).unwrap_err().to_string().contains("role"), "缺参数要报出参数名");
    assert!(dispatch("startPairing", &json!({ "role": "full" })).unwrap_err().to_string().contains("局域网"), "没开局域网不能配对");
    assert!(dispatch("startPairing", &json!({ "role": "root" })).is_err(), "未知权限");
    assert!(dispatch("revokeDevice", &json!({ "id": "missing" })).is_err());
    let cancelled = bridge_route(&json!({ "action": "cancelPairing" })).unwrap();
    assert_eq!(cancelled["pairing"], Value::Null);
}

/// 网页的两种对话样式：Codex 样式（桌面形态）与聊天样式（独立的手机界面，照微信与 Codex 手机端做），切换按设备记住。
#[test]
fn web_page_offers_both_conversation_styles() {
    let page = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/remote-control/index.html")).unwrap();
    // 两套界面互斥显示，只切根节点的 data-style
    assert!(page.contains(r#"html[data-style="chat"] .app { display: none !important; }"#));
    assert!(page.contains(r#"html:not([data-style="chat"]) .m { display: none !important; }"#));
    assert!(page.contains("codexPlusRemoteStyle"), "按设备记住选择");
    assert!(page.contains(r#"narrow ? "chat" : "codex""#), "手机默认聊天样式");
    // 手机界面四屏 + 底部标签栏
    for id in [r#"id="mList""#, r#"id="mProject""#, r#"id="mChat""#, r#"id="mSettings""#, r#"id="mTabs""#] {
        assert!(page.contains(id), "缺少 {id}");
    }
    // 列表：主机名顶栏、搜索、九宫格项目头像（按微信群头像的排法）、未读角标
    assert!(page.contains(r#"id="mHostName""#) && page.contains(r#"id="mSearch""#));
    assert!(page.contains("{ 2: [2], 3: [1, 2], 4: [2, 2], 5: [2, 3], 6: [3, 3], 7: [1, 3, 3], 8: [2, 3, 3], 9: [3, 3, 3] }"));
    assert!(page.contains("const unread = (thread) =>"), "本机记录已读");
    // 对话：带尾巴的左右气泡、时间分隔、卡片式命令 / 审批
    assert!(page.contains(".m-msg.in .m-bubble::before") && page.contains(".m-msg.out .m-bubble::before"));
    assert!(page.contains(r#"el("div", "m-time-tip", chatTime(ms / 1000, true))"#));
    assert!(page.contains("function requestCard(request)"));
    // 输入区：胶囊输入框 + 模式 / 权限 / 模型 / 上下文用量
    assert!(page.contains(r#"class="m-pill""#) && page.contains(r#"id="mChips""#));
    assert!(page.contains("openModeSheet") && page.contains("openPermissionSheet") && page.contains("openModelSheet"));
    assert!(page.contains("state.view.tokenUsage"), "上下文用量圈");
    // 安卓返回键 / iOS 侧滑：进项目、进对话都压一层历史
    assert!(page.contains(r#"history.pushState({ m: "chat" }, "")"#) && page.contains(r#"window.addEventListener("popstate", popped)"#));
    // 加号：照片 / 拍照 / 文件；上传后在输入框上方显示，发消息时带上
    assert!(page.contains("function openAttachSheet()") && page.contains(r#"pickFiles("image/*", "environment")"#));
    assert!(page.contains(r#"$("mPlus").onclick = openAttachSheet;"#));
    assert!(page.contains(r#"await api("POST", `/threads/${state.current}/messages`, { text, attachments });"#));
    // 新建会话：列表上可浏览文件夹；项目里的 ⊕ 直接用该项目目录
    assert!(page.contains("function openFolderSheet(start, onPick)") && page.contains(r#"api("GET", `/fs/dirs"#));
    assert!(page.contains(r#"$("mProjectNew").onclick = () => openNewSheet(ui.project);"#));
}

/// 网页会话列表分两级：先按工作目录列项目，点进项目再看会话。
#[test]
fn web_page_groups_sessions_by_project() {
    let page = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/remote-control/index.html")).unwrap();
    assert!(page.contains("function projectsOf(threads)"), "按工作目录归组");
    assert!(page.contains(r#"const key = thread.cwd || "";"#), "没记目录的会话单独一组");
    assert!(page.contains("project.updatedAt = Math.max(project.updatedAt"), "项目按最近活动排序");
    assert!(page.contains("if (isRunning(thread)) project.running += 1;"), "项目上带运行中数量");
    assert!(page.contains(r#"state.project = project.cwd; renderThreads();"#), "点项目进入会话列表");
    assert!(page.contains(r#"$("projectBack").onclick"#), "能回到项目列表");
    // 桌面样式下图标栏一直可见，会话开着时点「‹ 项目」也要能退出去，不能只在列表页才生效。
    let back_start = page.find(r#"$("projectBack").onclick"#).expect("有返回按钮的处理函数");
    let back_body = &page[back_start..(back_start + 400).min(page.len())];
    for line in [
        r#"$("app").classList.remove("viewing")"#,
        "state.current = null",
        "state.project = null",
        "renderThreads();",
    ] {
        assert!(back_body.contains(line), "「‹ 项目」缺少 {line}");
    }
    assert!(!back_body.contains("contains(\"viewing\")) renderThreads"), "会话开着时不能把点击挡掉");

    assert!(page.contains(r#"const cwd = state.project !== null ? state.project : $("newCwd").value.trim();"#), "项目里新建会话用该项目的目录");
    assert!(page.contains(r#"el("span", "count", ` (${project.threads.length})`)"#), "项目后显示会话数");
}

/// 后台执行：网页回答转成 app-server 的回包（和桌面 follower 协议的字段不同）。
#[test]
fn background_answers_map_to_app_server_results() {
    use codex_plus_core::remote_control::headless::app_server_result;
    assert_eq!(
        app_server_result("item/commandExecution/requestApproval", &json!({ "decision": "accept" })).unwrap(),
        json!({ "decision": "accept" }),
        "不能带 requestId：那是桌面 follower 协议的字段"
    );
    assert_eq!(
        app_server_result("item/fileChange/requestApproval", &json!({ "decision": "decline" })).unwrap(),
        json!({ "decision": "decline" })
    );
    assert_eq!(
        app_server_result("item/tool/requestUserInput", &json!({ "answers": { "q": { "answers": ["蓝色"] } } })).unwrap(),
        json!({ "answers": { "q": { "answers": ["蓝色"] } } })
    );
    let grant = json!({ "permissions": {}, "scope": "turn" });
    assert_eq!(app_server_result("item/permissions/requestApproval", &json!({ "response": grant })).unwrap(), grant);
    assert!(app_server_result("item/commandExecution/requestApproval", &json!({})).is_err(), "缺 decision 要报错");
    assert!(app_server_result("item/tool/call", &json!({})).is_err());
}

/// 后台执行的权限预设映射到 app-server 的沙箱策略，计划模式不支持。
#[test]
fn background_permission_presets_map_to_sandbox_policies() {
    use codex_plus_core::remote_control::follower::{PERMISSION_PRESETS, permission_mode_of};
    use codex_plus_core::remote_control::headless::{headless_supports, sandbox_policy_for};
    for preset in PERMISSION_PRESETS {
        let settings = json!({
            "sandboxPolicy": sandbox_policy_for(preset.profile),
            "approvalPolicy": preset.approval_policy,
            "approvalsReviewer": preset.reviewer,
        });
        assert_eq!(permission_mode_of(&settings), preset.id, "后台发出的设置读回来要是同一个预设：{}", preset.id);
    }
    assert!(headless_supports("default"));
    assert!(!headless_supports("plan"), "turn/start 没有协作模式字段");
}

/// 后台执行的接线：没在桌面打开的会话发消息不再让桌面打开它。
#[test]
fn sending_to_unopened_thread_runs_in_background() {
    let service = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/remote_control/service.rs")).unwrap();
    // send() 转给 send_with()（带附件的那个），检查真正干活的那个。
    let send = &service[service.find("pub async fn send_with(").unwrap()..];
    let send = &send[..send.find("\n    }\n").unwrap()];
    assert!(!send.contains("open_thread"), "发消息不能让桌面打开会话");
    assert!(send.contains("ensure_headless"), "没在桌面打开的会话走后台");
    let create = &service[service.find("pub async fn create_thread(").unwrap()..];
    let create = &create[..create.find("\n    }\n").unwrap()];
    assert!(!create.contains("open_thread"), "新建会话也不让桌面打开");
    let page = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/remote-control/index.html")).unwrap();
    assert!(!page.contains("正在桌面打开会话…\");\n      await api(\"POST\", `/threads/${state.current}/messages`"), "发消息时不再提示打开桌面");
    assert!(page.contains("后台运行中"), "后台会话有自己的状态");
    assert!(page.contains("message.type === \"closed\" && message.reconnect"), "推送来源换了要重连");
}

/// 附件：图片作为 localImage，文件照桌面的「# Files mentioned by the user:」写法列路径；视图里再拆回来。
#[test]
fn attachments_become_codex_inputs_and_round_trip_in_views() {
    use codex_plus_core::remote_control::uploads::{Attachment, AttachmentKind, sanitize_name, split_message_text, user_inputs};
    use std::path::PathBuf;
    let image = Attachment { kind: AttachmentKind::Image, name: "截图.png".into(), path: PathBuf::from("/u/remote-uploads/a-截图.png") };
    let file = Attachment { kind: AttachmentKind::File, name: "error.log".into(), path: PathBuf::from("/u/remote-uploads/b-error.log") };

    assert_eq!(user_inputs("你好", &[]), vec![json!({ "type": "text", "text": "你好", "text_elements": [] })]);
    let input = user_inputs("看看", &[image.clone(), file.clone()]);
    assert_eq!(input.len(), 2, "文字一条 + 图片一条；文件写进文字里");
    let text = input[0]["text"].as_str().unwrap();
    assert!(text.starts_with("# Files mentioned by the user:"));
    assert!(text.contains("## error.log: /u/remote-uploads/b-error.log"));
    assert!(text.ends_with("## My request for Codex:\n看看"));
    assert_eq!(input[1], json!({ "type": "localImage", "path": "/u/remote-uploads/a-截图.png" }));
    let only_image = user_inputs("", &[image]);
    assert_eq!(only_image.len(), 1, "只发图不带空文字");
    assert!(user_inputs("", &[file])[0]["text"].as_str().unwrap().contains("看看这个文件"), "只发文件时补一句请求");

    let (files, request) = split_message_text(text).unwrap();
    assert_eq!(files, vec![json!({ "kind": "file", "name": "error.log", "path": "/u/remote-uploads/b-error.log" })]);
    assert_eq!(request, "看看");
    assert!(split_message_text("普通消息").is_none());

    assert_eq!(sanitize_name("../../etc/passwd"), "passwd");
    assert_eq!(sanitize_name("C:\\a\\b.txt"), "b.txt");
    assert_eq!(sanitize_name(".hidden"), "hidden");
    assert_eq!(sanitize_name("   "), "file");
}

/// 上传只认上传目录；目录浏览只列文件夹。用临时状态目录，进程级状态先拿全局锁。
#[test]
fn uploads_stay_inside_upload_dir_and_dir_listing_shows_folders() {
    use codex_plus_core::remote_control::uploads::{attachments_from, list_dirs, resolve, save};
    let _guard = crate::common::global_state_lock();
    let _temp = TempState::new();

    let saved = save("IMG_0001", "image/jpeg", b"\xff\xd8\xff").unwrap();
    assert_eq!(saved["kind"], "image");
    assert!(saved["name"].as_str().unwrap().ends_with(".jpg"), "没扩展名按类型补");
    let path = saved["path"].as_str().unwrap();
    assert!(resolve(path).is_ok());
    assert!(save("x.txt", "text/plain", b"").is_err(), "空文件被拒");

    let outside = tempfile::NamedTempFile::new().unwrap();
    assert!(resolve(&outside.path().to_string_lossy()).is_err(), "上传目录外的文件不认");
    assert!(resolve("relative/path.png").is_err());
    assert!(attachments_from(&json!({ "attachments": [{ "path": outside.path().to_string_lossy() }] })).is_err());
    let parsed = attachments_from(&json!({ "attachments": [{ "path": path }] })).unwrap();
    assert_eq!(parsed.len(), 1);
    assert!(attachments_from(&json!({ "attachments": vec![json!({ "path": path }); 10] })).is_err(), "最多 9 个");

    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("repo/.git")).unwrap();
    std::fs::create_dir_all(root.path().join("plain")).unwrap();
    std::fs::create_dir_all(root.path().join(".hidden")).unwrap();
    std::fs::write(root.path().join("file.txt"), b"x").unwrap();
    let listed = list_dirs(Some(&root.path().to_string_lossy())).unwrap();
    let names: Vec<&str> = listed["entries"].as_array().unwrap().iter().map(|entry| entry["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["plain", "repo"], "只列文件夹、不列隐藏的、按名字排");
    assert_eq!(listed["entries"][1]["git"], true);
    assert!(listed["parent"].is_string());
    assert!(list_dirs(Some("relative")).is_err());
    assert!(list_dirs(Some(&format!("{}/../x", root.path().display()))).is_err(), "不认 ..");
    assert!(list_dirs(None).unwrap()["path"].is_string(), "不给路径从主目录开始");
}

/// 上传和目录浏览的接口：读请求体前先认证；文件夹浏览要「可发消息」。
#[test]
fn upload_and_folder_routes_require_auth_and_role() {
    use codex_plus_core::remote_control::devices::Role;
    use codex_plus_core::remote_control::server::{MAX_UPLOAD_BYTES, required_role};
    assert_eq!(required_role("POST", &["uploads"]), Role::Message);
    assert_eq!(required_role("GET", &["fs", "dirs"]), Role::Message, "只读设备不能浏览电脑上的文件夹");
    assert_eq!(required_role("GET", &["uploads", "raw"]), Role::View);
    assert!(MAX_UPLOAD_BYTES >= 20 * 1024 * 1024);
    let server = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/remote_control/server.rs")).unwrap();
    let auth = server.find("let Some(principal) = authenticate(").unwrap();
    let body = server.find("read_body(&mut stream, &mut request, limit)").unwrap();
    assert!(auth < body, "认证通过后才读请求体，没配对的人不能让我们收大文件");
}
