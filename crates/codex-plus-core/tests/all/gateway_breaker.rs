//! 网关的供应商熔断（#2761）：上游连续 5xx 后熔断，有别家时让给别家；
//! 路由里没有别家可换时不再本地回 429，照常发给熔断中的供应商。
//!
//! 每个用例用自己的供应商 id，Key 池、熔断这些进程内状态互不干扰。

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use codex_plus_core::gateway::key_pool::{self, FailureClass};
use codex_plus_core::gateway::pipeline::{self, GatewayOutcome, GatewayRequest, GatewayUpstream};
use codex_plus_core::gateway::route::{self, RoutePlan};
use codex_plus_core::gateway::store::{AgentBinding, GatewayProvider, GatewaySettings};
use codex_plus_core::protocol_proxy::ProxySessionHeaders;
use codex_plus_core::settings::{BackendSettings, RelayMode, RelayProfile, RelayProtocol};
use codex_plus_core::tools::ToolId;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const MODEL: &str = "breaker-model";

/// 假上游：前 `failures` 个请求回 Cloudflare 风格的纯文本 502，之后回流式 Chat Completions。
/// 返回 (base_url, 收到的请求数)。
async fn mock_upstream(failures: usize) -> (String, Arc<AtomicUsize>) {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let seen = Arc::new(AtomicUsize::new(0));
    let counter = seen.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let counter = counter.clone();
            tokio::spawn(async move {
                let mut buffer = Vec::new();
                let mut chunk = [0; 4096];
                let (end, length) = loop {
                    let read = stream.read(&mut chunk).await.unwrap_or(0);
                    if read == 0 {
                        return;
                    }
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
                    let read = stream.read(&mut chunk).await.unwrap_or(0);
                    if read == 0 {
                        return;
                    }
                    buffer.extend_from_slice(&chunk[..read]);
                }
                let index = counter.fetch_add(1, Ordering::SeqCst);
                let response = if index < failures {
                    let body = "error code: 502";
                    format!(
                        "HTTP/1.1 502 Bad Gateway\r\ncontent-type: text/plain\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    )
                } else {
                    let chunks = [
                        json!({"id":"c","object":"chat.completion.chunk","model":"m",
                            "choices":[{"index":0,"delta":{"role":"assistant","content":"ok"},"finish_reason":null}]}),
                        json!({"id":"c","object":"chat.completion.chunk","model":"m",
                            "choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}),
                    ];
                    let mut body = String::new();
                    for chunk in chunks {
                        body.push_str(&format!("data: {chunk}\n\n"));
                    }
                    body.push_str("data: [DONE]\n\n");
                    format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n{body}"
                    )
                };
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            });
        }
    });
    (base, seen)
}

fn provider(id: &str, base_url: &str) -> GatewayProvider {
    GatewayProvider {
        enabled: true,
        source: String::new(),
        price: None,
        model_prices: Default::default(),
        model_windows: Default::default(),
        context_window: None,
        proxy: String::new(),
        key_rpm: None,
        key_concurrency: None,
        upstream_new: Vec::new(),
        upstream_gone: Vec::new(),
        models_checked_at_ms: 0,
        newapi_access: None,
        upstream_meta: Default::default(),
        relay: RelayProfile {
            id: id.to_string(),
            name: id.to_string(),
            relay_mode: RelayMode::PureApi,
            protocol: RelayProtocol::ChatCompletions,
            base_url: base_url.to_string(),
            api_key: format!("sk-{id}"),
            model_list: MODEL.to_string(),
            ..RelayProfile::default()
        },
        extra: Default::default(),
    }
}

fn gateway(providers: Vec<GatewayProvider>) -> GatewaySettings {
    let mut gateway = GatewaySettings {
        providers,
        // 多家都有同一个模型时按池顺序依次试，不走自动组。
        auto_models: false,
        ..GatewaySettings::default()
    };
    gateway.bindings.insert(
        "claude".to_string(),
        AgentBinding {
            enabled: true,
            model: MODEL.to_string(),
            ..AgentBinding::default()
        },
    );
    gateway
}

fn plan(gateway: &GatewaySettings) -> RoutePlan {
    route::resolve_plan(gateway, &ToolId::Claude, MODEL, None).unwrap()
}

async fn send(plan: &RoutePlan) -> anyhow::Result<GatewayOutcome> {
    pipeline::open(
        &BackendSettings::default(),
        plan,
        GatewayRequest {
            responses_body: json!({
                "model": MODEL,
                "stream": true,
                "input": [{ "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "hi" }] }]
            }),
            anthropic: None,
            user_agent: None,
            session_headers: ProxySessionHeaders {
                session_id: None,
                thread_id: None,
                opencode_session: None,
                client_identity: &[],
            },
            live_request_id: None,
            auto_retry: false,
            is_stream: true,
            upstream_path: "/responses",
            client: None,
            queue_timeout: Duration::from_secs(5),
        },
    )
    .await
}

fn status(outcome: &GatewayOutcome) -> Option<u16> {
    match &outcome.upstream {
        GatewayUpstream::Pipeline(response) => Some(response.status_code),
        GatewayUpstream::Native(response) => Some(response.status_code),
        GatewayUpstream::Cooling(_) => None,
    }
}

/// 只有一家时：连续 3 次 502 打开熔断，但下一个请求照常发到上游（上游已恢复就成功），
/// 不再由网关本地回 429（gateway_keys_cooling）。
#[tokio::test]
async fn sole_provider_is_tried_through_an_open_breaker() {
    let (base, seen) = mock_upstream(3).await;
    let gateway = gateway(vec![provider("br-sole", &base)]);
    for attempt in 0..3 {
        let outcome = send(&plan(&gateway)).await.unwrap();
        assert_eq!(
            status(&outcome),
            Some(502),
            "第 {attempt} 次应原样拿到上游 502"
        );
    }
    assert!(
        key_pool::breaker_open("br-sole", Instant::now()),
        "连续 3 次 5xx 应打开熔断"
    );
    let plan = plan(&gateway);
    assert_eq!(
        plan.candidates.len(),
        1,
        "没有别家可换时熔断中的供应商仍是候选"
    );
    assert_eq!(plan.cooling, None);
    let outcome = send(&plan).await.unwrap();
    assert_eq!(status(&outcome), Some(200), "上游恢复后第 4 个请求直接成功");
    match &outcome.upstream {
        GatewayUpstream::Pipeline(response) => assert!(response.is_stream),
        _ => panic!("应走代理链路"),
    }
    assert_eq!(seen.load(Ordering::SeqCst), 4, "4 个请求都发到了上游");
    assert!(
        !key_pool::breaker_open("br-sole", Instant::now()),
        "成功后关闭熔断"
    );
}

/// 熔断中的供应商仍然失败时，客户端拿到的是上游原样的 5xx，而不是网关编出来的 429。
#[tokio::test]
async fn sole_provider_still_failing_returns_the_upstream_error() {
    let (base, seen) = mock_upstream(usize::MAX).await;
    let gateway = gateway(vec![provider("br-down", &base)]);
    for _ in 0..4 {
        let outcome = send(&plan(&gateway)).await.unwrap();
        assert_eq!(status(&outcome), Some(502));
    }
    assert_eq!(seen.load(Ordering::SeqCst), 4);
    assert!(key_pool::breaker_open("br-down", Instant::now()));
}

/// 有别家可换时熔断照旧生效：熔断中的供应商不进候选，请求直接给别家。
#[tokio::test]
async fn open_breaker_still_yields_to_other_providers() {
    let (bad, bad_seen) = mock_upstream(usize::MAX).await;
    let (good, good_seen) = mock_upstream(0).await;
    let gateway = gateway(vec![
        provider("br-first", &bad),
        provider("br-second", &good),
    ]);
    let key = plan(&gateway).candidates[0].key.clone();
    assert_eq!(key.relay_id, "br-first");
    for _ in 0..3 {
        key_pool::record_failure(&key, &FailureClass::ServerError, Instant::now());
    }
    let plan = plan(&gateway);
    assert_eq!(
        plan.candidates
            .iter()
            .map(|candidate| candidate.relay.id.as_str())
            .collect::<Vec<_>>(),
        vec!["br-second"]
    );
    let outcome = send(&plan).await.unwrap();
    assert_eq!(status(&outcome), Some(200));
    assert_eq!(outcome.trace.relay_id, "br-second");
    assert_eq!(bad_seen.load(Ordering::SeqCst), 0);
    assert_eq!(good_seen.load(Ordering::SeqCst), 1);
}
