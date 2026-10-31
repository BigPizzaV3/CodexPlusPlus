//! 供应商池的「测试」：按这家供应商的协议、代理和 Key 发一条很短的请求，
//! 报出成功 / 失败、状态码、耗时、回复和排查分类；测试请求不碰 Key 池的冷却、
//! 限速名额、会话固定和测速。
//!
//! Key 池状态是进程内全局的：每个用例用自己的供应商名，id 不和别的测试撞。

use std::time::{Duration, Instant};

use codex_plus_core::gateway::apply::{ProviderInput, create_provider, set_provider_limits};
use codex_plus_core::gateway::key_pool::{self, KeyLimitState, KeyRef};
use codex_plus_core::gateway::probe::{probe_provider, probe_provider_with_timeout};
use codex_plus_core::gateway::speed;
use codex_plus_core::gateway::store::{GatewaySettings, GatewayStore, KeyRotationMode};
use codex_plus_core::settings::{BackendSettings, RelayProtocol};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const KEY_A: &str = "sk-alpha-0123456789abcdefghij-AAAA";
const KEY_B: &str = "sk-bravo-0123456789abcdefghij-BBBB";

/// 模拟上游收到的一条请求。
#[derive(Debug)]
struct Seen {
    request_line: String,
    head: String,
    body: Value,
}

impl Seen {
    fn header(&self, name: &str) -> Option<String> {
        self.head.lines().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.trim()
                .eq_ignore_ascii_case(name)
                .then(|| value.trim().to_string())
        })
    }
}

struct Reply {
    status: u16,
    content_type: &'static str,
    body: String,
}

fn reply(status: u16, content_type: &'static str, body: impl Into<String>) -> Reply {
    Reply {
        status,
        content_type,
        body: body.into(),
    }
}

/// 本地模拟上游：按顺序给每个连接回一条答复，收下请求后关掉连接。
async fn serve(listener: tokio::net::TcpListener, replies: Vec<Reply>) -> Vec<Seen> {
    let mut seen = Vec::new();
    for reply in replies {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buffer = Vec::new();
        let mut chunk = [0; 4096];
        let (end, length) = loop {
            let read = stream.read(&mut chunk).await.unwrap();
            assert!(read > 0, "请求没读完连接就断了");
            buffer.extend_from_slice(&chunk[..read]);
            if let Some(end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&buffer[..end]).to_ascii_lowercase();
                let length = head
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .and_then(|value| value.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                break (end + 4, length);
            }
        };
        while buffer.len() < end + length {
            let read = stream.read(&mut chunk).await.unwrap();
            buffer.extend_from_slice(&chunk[..read]);
        }
        let head = String::from_utf8_lossy(&buffer[..end]).to_string();
        seen.push(Seen {
            request_line: head.lines().next().unwrap_or_default().to_string(),
            body: serde_json::from_slice(&buffer[end..end + length]).unwrap_or(Value::Null),
            head,
        });
        let response = format!(
            "HTTP/1.1 {} X\r\ncontent-type: {}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            reply.status,
            reply.content_type,
            reply.body.len(),
            reply.body
        );
        stream.write_all(response.as_bytes()).await.unwrap();
        let _ = stream.shutdown().await;
    }
    seen
}

fn chat_stream(text: &str) -> String {
    let chunk = json!({ "choices": [{ "index": 0, "delta": { "content": text } }] });
    let stop = json!({ "choices": [{ "index": 0, "delta": {}, "finish_reason": "stop" }] });
    format!("data: {chunk}\n\ndata: {stop}\n\ndata: [DONE]\n\n")
}

fn input(name: &str, base_url: String, protocol: RelayProtocol, keys: &[&str]) -> ProviderInput {
    ProviderInput {
        name: name.into(),
        base_url,
        protocol,
        keys: keys.iter().map(|key| key.to_string()).collect(),
        model: "probe-default".into(),
        model_list: "probe-default\nprobe-other".into(),
        // 本机模拟上游：直连，不受系统代理影响。
        proxy: Some("direct".into()),
        ..ProviderInput::default()
    }
}

fn setup(input: ProviderInput) -> (tempfile::TempDir, GatewayStore, String) {
    let temp = tempfile::tempdir().unwrap();
    let store = GatewayStore::at(temp.path().join("gateway.json"));
    let (_, id) = create_provider(&store, &input, "custom", None).unwrap();
    (temp, store, id)
}

async fn listen() -> (tokio::net::TcpListener, String) {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    (listener, base)
}

fn gateway(store: &GatewayStore) -> GatewaySettings {
    store.load().unwrap()
}

/// Key 池里这家供应商没有任何冷却 / 停用 / 熔断记录。
fn assert_pool_untouched(relay_id: &str) {
    let snapshot = key_pool::snapshot(Instant::now());
    assert!(
        snapshot.keys.iter().all(|key| key.relay_id != relay_id),
        "测试请求不该记 Key 的失败：{:?}",
        snapshot.keys
    );
    assert!(
        snapshot
            .providers
            .iter()
            .all(|item| item.relay_id != relay_id),
        "测试请求不该计入熔断：{:?}",
        snapshot.providers
    );
    assert!(
        speed::snapshot()
            .iter()
            .all(|view| view.relay_id != relay_id)
            && speed::rank(relay_id, "probe-default").is_none(),
        "测试请求不该计入测速"
    );
}

#[tokio::test]
async fn success_reports_reply_and_timings_without_touching_routing_state() {
    let (listener, base) = listen().await;
    let (_temp, store, id) = setup(input(
        "Probe Ok",
        base,
        RelayProtocol::ChatCompletions,
        &[KEY_A, KEY_B],
    ));
    // 每个 Key 每分钟只许 1 次：测试请求不该占掉这个名额。
    set_provider_limits(&store, &id, Some(1), None).unwrap();
    let upstream = tokio::spawn(serve(
        listener,
        vec![reply(200, "text/event-stream", chat_stream("OK"))],
    ));

    let result = probe_provider(&BackendSettings::default(), &gateway(&store), &id, "", None)
        .await
        .unwrap();
    assert!(result.ok, "{result:?}");
    assert_eq!(result.http_status, Some(200));
    assert_eq!(result.wire_api, "chat");
    assert_eq!(result.model, "probe-default", "没选模型时用默认模型");
    assert_eq!(result.reply, "OK");
    assert!(result.error.is_empty() && result.category.is_empty());
    let first = result.first_token_ms.expect("收到了首字");
    assert!(first <= result.total_ms, "{result:?}");
    // 没指定 Key：用当前选中的 Key（第一个），结果里只有标识和末 4 位。
    assert_eq!(result.key_id, "key-1");
    assert_eq!(result.key_tail, "AAAA");
    let json = serde_json::to_string(&result).unwrap();
    assert!(!json.contains(KEY_A) && !json.contains(KEY_B), "{json}");

    let seen = upstream.await.unwrap();
    assert_eq!(seen.len(), 1);
    assert!(
        seen[0]
            .request_line
            .starts_with("POST /v1/chat/completions "),
        "按供应商协议转换成 Chat：{}",
        seen[0].request_line
    );
    assert_eq!(
        seen[0].header("authorization").as_deref(),
        Some(format!("Bearer {KEY_A}").as_str())
    );
    assert_eq!(seen[0].body["model"], "probe-default");
    assert_eq!(seen[0].body["stream"], true);
    let max_tokens = seen[0].body["max_tokens"]
        .as_u64()
        .or_else(|| seen[0].body["max_completion_tokens"].as_u64())
        .expect("带了输出上限");
    assert!(max_tokens <= 64, "输出上限要很小：{max_tokens}");

    assert_pool_untouched(&id);
    let key = KeyRef {
        relay_id: id.clone(),
        key_id: "key-1".into(),
        fingerprint: key_pool::fingerprint(KEY_A),
    };
    let limits = gateway(&store).provider(&id).unwrap().key_limits();
    assert_eq!(
        key_pool::limit_state(&key, limits, Instant::now()),
        KeyLimitState::Free,
        "测试请求不占 RPM 名额"
    );
}

#[tokio::test]
async fn auth_failure_uses_only_the_chosen_key_and_names_it() {
    let (listener, base) = listen().await;
    let (_temp, store, id) = setup(input(
        "Probe Auth",
        base,
        RelayProtocol::ChatCompletions,
        &[KEY_A, KEY_B],
    ));
    // 有个会话固定在第一个 Key 上：测试第二个 Key 失败后，固定不变、第二个 Key 也不被停用。
    let first_key = KeyRef {
        relay_id: id.clone(),
        key_id: "key-1".into(),
        fingerprint: key_pool::fingerprint(KEY_A),
    };
    key_pool::record_success(&first_key, Some("probe-conversation"));
    let upstream = tokio::spawn(serve(
        listener,
        vec![reply(
            401,
            "application/json",
            r#"{"error":{"message":"Incorrect API key provided: sk-bra****BBBB","code":"invalid_api_key"}}"#,
        )],
    ));

    let result = probe_provider(
        &BackendSettings::default(),
        &gateway(&store),
        &id,
        "probe-other",
        Some("key-2"),
    )
    .await
    .unwrap();
    assert!(!result.ok);
    assert_eq!(result.category, "auth");
    assert_eq!(result.http_status, Some(401));
    assert_eq!(result.model, "probe-other");
    assert!(result.error.contains("Incorrect API key"), "{result:?}");
    assert_eq!(result.key_id, "key-2");
    assert_eq!(result.key_tail, "BBBB");
    let saved_name = gateway(&store).provider(&id).unwrap().relay.api_keys[1]
        .name
        .clone();
    assert_eq!(result.key_name, saved_name, "多个 Key 时带上名称");

    let seen = upstream.await.unwrap();
    assert_eq!(seen.len(), 1, "失败不换 Key 重试");
    assert_eq!(
        seen[0].header("authorization").as_deref(),
        Some(format!("Bearer {KEY_B}").as_str()),
        "只用指定的 Key"
    );
    assert_eq!(seen[0].body["model"], "probe-other");

    assert_pool_untouched(&id);
    let relay = gateway(&store).provider(&id).unwrap().relay.clone();
    let (keys, _) = key_pool::order_keys_with(
        &relay,
        KeyRotationMode::Sticky,
        Some("probe-conversation"),
        Instant::now(),
        false,
    );
    assert_eq!(keys.len(), 2, "第二个 Key 没被停用");
    assert_eq!(keys[0].key.key_id, "key-1", "会话固定不变");

    // 不在了的 Key：没法测，直接报错，不发请求。
    let error = probe_provider(
        &BackendSettings::default(),
        &gateway(&store),
        &id,
        "",
        Some("key-9"),
    )
    .await
    .unwrap_err();
    assert!(format!("{error:#}").contains("不在了"), "{error:#}");
}

#[tokio::test]
async fn not_found_and_rate_limit_are_classified_without_cooling_the_key() {
    let (listener, base) = listen().await;
    let (_temp, store, id) = setup(input(
        "Probe Classify",
        base,
        RelayProtocol::ChatCompletions,
        &[KEY_A],
    ));
    let upstream = tokio::spawn(serve(
        listener,
        vec![
            reply(
                404,
                "application/json",
                r#"{"error":{"message":"not here"}}"#,
            ),
            reply(
                400,
                "application/json",
                r#"{"error":{"message":"The model `nope` does not exist","code":"model_not_found"}}"#,
            ),
            reply(
                429,
                "application/json",
                r#"{"error":{"message":"Rate limit reached, slow down"}}"#,
            ),
            reply(
                429,
                "application/json",
                r#"{"error":{"message":"You exceeded your current quota","code":"insufficient_quota"}}"#,
            ),
        ],
    ));
    let settings = BackendSettings::default();
    let mut categories = Vec::new();
    for _ in 0..4 {
        let result = probe_provider(&settings, &gateway(&store), &id, "nope", None)
            .await
            .unwrap();
        assert!(!result.ok);
        categories.push((result.http_status, result.category));
    }
    assert_eq!(
        categories,
        vec![
            (Some(404), "not_found".to_string()),
            (Some(400), "not_found".to_string()),
            (Some(429), "rate_limited".to_string()),
            (Some(429), "quota".to_string()),
        ]
    );
    assert_eq!(upstream.await.unwrap().len(), 4);
    assert_pool_untouched(&id);
}

#[tokio::test]
async fn anthropic_provider_is_converted_like_gateway_requests() {
    let (listener, base) = listen().await;
    let (_temp, store, id) = setup(input(
        "Probe Anthropic",
        base,
        RelayProtocol::AnthropicMessages,
        &[KEY_A],
    ));
    let events = [
        json!({ "type": "message_start", "message": { "id": "m", "type": "message", "role": "assistant", "content": [], "model": "probe-default", "usage": { "input_tokens": 3, "output_tokens": 0 } } }),
        json!({ "type": "content_block_start", "index": 0, "content_block": { "type": "text", "text": "" } }),
        json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "text_delta", "text": "OK" } }),
        json!({ "type": "content_block_stop", "index": 0 }),
        json!({ "type": "message_delta", "delta": { "stop_reason": "end_turn" }, "usage": { "output_tokens": 1 } }),
        json!({ "type": "message_stop" }),
    ];
    let body: String = events
        .iter()
        .map(|event| {
            format!(
                "event: {}\ndata: {event}\n\n",
                event["type"].as_str().unwrap()
            )
        })
        .collect();
    let upstream = tokio::spawn(serve(listener, vec![reply(200, "text/event-stream", body)]));
    let result = probe_provider(&BackendSettings::default(), &gateway(&store), &id, "", None)
        .await
        .unwrap();
    assert!(result.ok, "{result:?}");
    assert_eq!(result.wire_api, "anthropic");
    assert_eq!(result.reply, "OK");
    let seen = upstream.await.unwrap();
    assert!(
        seen[0].request_line.starts_with("POST /v1/messages "),
        "{}",
        seen[0].request_line
    );
    assert_eq!(seen[0].header("x-api-key").as_deref(), Some(KEY_A));
    assert!(seen[0].body["max_tokens"].as_u64().unwrap() <= 64);
    assert_pool_untouched(&id);
}

#[tokio::test]
async fn responses_upstream_answering_plain_json_is_read_too() {
    let (listener, base) = listen().await;
    let (_temp, store, id) = setup(input(
        "Probe Responses",
        base,
        RelayProtocol::Responses,
        &[KEY_A],
    ));
    let body = json!({
        "id": "resp_1", "object": "response", "status": "completed", "model": "probe-default",
        "output": [{ "type": "message", "role": "assistant", "content": [{ "type": "output_text", "text": "OK" }] }]
    });
    let upstream = tokio::spawn(serve(
        listener,
        vec![reply(200, "application/json", body.to_string())],
    ));
    let result = probe_provider(&BackendSettings::default(), &gateway(&store), &id, "", None)
        .await
        .unwrap();
    assert!(result.ok, "{result:?}");
    assert_eq!(result.wire_api, "responses");
    assert_eq!(result.reply, "OK");
    assert!(result.first_token_ms.is_some());
    let seen = upstream.await.unwrap();
    assert!(seen[0].request_line.starts_with("POST /v1/responses "));
    assert!(seen[0].body["max_output_tokens"].as_u64().unwrap() <= 64);
}

#[tokio::test]
async fn requests_go_through_the_providers_own_proxy() {
    // 本地「代理」直接扮演上游：收到绝对地址的请求行，说明请求确实交给了供应商代理。
    let (proxy, proxy_base) = listen().await;
    let proxy_url = proxy_base.trim_end_matches("/v1").to_string();
    let mut provider = input(
        "Probe Proxy",
        "http://pooltest-upstream.invalid/v1".into(),
        RelayProtocol::ChatCompletions,
        &[KEY_A],
    );
    provider.proxy = Some(proxy_url);
    let (_temp, store, id) = setup(provider);
    let upstream = tokio::spawn(serve(
        proxy,
        vec![reply(200, "text/event-stream", chat_stream("走代理"))],
    ));
    let result = probe_provider(&BackendSettings::default(), &gateway(&store), &id, "", None)
        .await
        .unwrap();
    assert!(result.ok, "{result:?}");
    assert_eq!(result.reply, "走代理");
    let seen = tokio::time::timeout(Duration::from_secs(5), upstream)
        .await
        .expect("请求没有经过供应商代理")
        .unwrap();
    assert!(
        seen[0]
            .request_line
            .starts_with("POST http://pooltest-upstream.invalid/v1/chat/completions "),
        "{}",
        seen[0].request_line
    );
}

#[tokio::test]
async fn connection_failures_and_timeouts_are_reported() {
    // 连不上：端口上没人监听。
    let (listener, base) = listen().await;
    drop(listener);
    let (_temp, store, id) = setup(input(
        "Probe Refused",
        base,
        RelayProtocol::ChatCompletions,
        &[KEY_A],
    ));
    let result = probe_provider(&BackendSettings::default(), &gateway(&store), &id, "", None)
        .await
        .unwrap();
    assert!(!result.ok);
    assert_eq!(result.category, "connect", "{result:?}");
    assert_eq!(result.http_status, None);
    assert!(!result.error.is_empty());
    assert_pool_untouched(&id);

    // 一直不回：到时间放弃，报超时。
    let (listener, base) = listen().await;
    let (_temp, store, id) = setup(input(
        "Probe Silent",
        base,
        RelayProtocol::ChatCompletions,
        &[KEY_A],
    ));
    let hold = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        tokio::time::sleep(Duration::from_secs(5)).await;
        drop(stream);
    });
    let started = Instant::now();
    let result = probe_provider_with_timeout(
        &BackendSettings::default(),
        &gateway(&store),
        &id,
        "",
        None,
        Duration::from_millis(400),
    )
    .await
    .unwrap();
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(!result.ok);
    assert_eq!(result.category, "timeout");
    hold.abort();
    assert_pool_untouched(&id);
}
