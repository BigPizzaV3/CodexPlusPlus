//! 网关遇到上游 429 / 5xx 时的冷却与熔断（#2802）。
//!
//! 只有一个 Key、一家供应商时，网关不能比直连更糟：
//! - 429 冷却按上游的 Retry-After / 限流重置头，没给时第一次只冷却 5 秒；
//! - 所有 Key 都在冷却时，最早的那个很快恢复就等一等再试，而不是马上回本地 429；
//! - 熔断只在有别的候选可换时拦请求；
//! - 「请求避震」不对 429 重发同一个 Key，一次请求打到上游的次数有数。
//!
//! 多 Key / 多供应商时 429 仍然立即换下一个。每个用例用自己的供应商 id，进程内的 Key 池状态互不干扰。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use codex_plus_core::gateway::key_pool;
use codex_plus_core::gateway::pipeline::{self, GatewayOutcome, GatewayRequest, GatewayUpstream};
use codex_plus_core::gateway::route::{self, RoutePlan};
use codex_plus_core::gateway::store::{AgentBinding, GatewayProvider, GatewaySettings};
use codex_plus_core::protocol_proxy::ProxySessionHeaders;
use codex_plus_core::settings::{
    BackendSettings, RelayApiKey, RelayMode, RelayProfile, RelayProtocol,
};
use codex_plus_core::tools::ToolId;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// 假上游的一次答复：状态码、额外响应头、响应体。
struct Reply {
    status: u16,
    headers: Vec<(&'static str, String)>,
    body: String,
}

fn ok() -> Reply {
    Reply {
        status: 200,
        headers: Vec::new(),
        body: json!({
            "id": "c", "object": "chat.completion", "model": "m",
            "choices": [{ "index": 0, "message": { "role": "assistant", "content": "ok" }, "finish_reason": "stop" }]
        })
        .to_string(),
    }
}

fn rate_limited(headers: &[(&'static str, &str)]) -> Reply {
    Reply {
        status: 429,
        headers: headers
            .iter()
            .map(|(name, value)| (*name, value.to_string()))
            .collect(),
        body:
            json!({ "error": { "type": "rate_limit_error", "message": "upstream says slow down" } })
                .to_string(),
    }
}

fn server_error() -> Reply {
    Reply {
        status: 500,
        headers: Vec::new(),
        body: json!({ "error": { "message": "boom" } }).to_string(),
    }
}

type Responder = Arc<dyn Fn(&str, usize) -> Reply + Send + Sync>;

/// 假上游：按 (Key, 这个 Key 第几次被打到) 决定怎么答，记下每次用的 Key。
async fn mock_upstream(responder: Responder) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let counts = Arc::new(Mutex::new(HashMap::<String, usize>::new()));
    let record = seen.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let record = record.clone();
            let counts = counts.clone();
            let responder = responder.clone();
            tokio::spawn(async move {
                let mut buffer = Vec::new();
                let mut chunk = [0; 4096];
                let (end, length, head) = loop {
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
                        break (end + 4, length, head);
                    }
                };
                while buffer.len() < end + length {
                    let read = stream.read(&mut chunk).await.unwrap_or(0);
                    if read == 0 {
                        return;
                    }
                    buffer.extend_from_slice(&chunk[..read]);
                }
                let key = head
                    .lines()
                    .find_map(|line| line.strip_prefix("authorization:"))
                    .map(|value| value.trim().trim_start_matches("bearer ").to_string())
                    .unwrap_or_default();
                record.lock().unwrap().push(key.clone());
                let hit = {
                    let mut counts = counts.lock().unwrap();
                    let count = counts.entry(key.clone()).or_default();
                    *count += 1;
                    *count
                };
                let reply = responder(&key, hit);
                let mut extra = String::new();
                for (name, value) in &reply.headers {
                    extra.push_str(&format!("{name}: {value}\r\n"));
                }
                let response = format!(
                    "HTTP/1.1 {} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\n{extra}connection: close\r\n\r\n{}",
                    reply.status,
                    reply.body.len(),
                    reply.body
                );
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });
    (base, seen)
}

fn provider(id: &str, base_url: &str, keys: &[&str]) -> GatewayProvider {
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
            api_key: format!("sk-{id}-{}", keys[0]),
            api_keys: keys
                .iter()
                .map(|key| RelayApiKey {
                    id: key.to_string(),
                    name: key.to_string(),
                    api_key: format!("sk-{id}-{key}"),
                })
                .collect(),
            active_api_key_id: keys[0].to_string(),
            model_list: "rl-model".to_string(),
            ..RelayProfile::default()
        },
        extra: Default::default(),
    }
}

fn gateway(providers: Vec<GatewayProvider>) -> GatewaySettings {
    let mut gateway = GatewaySettings {
        providers,
        auto_models: false,
        ..GatewaySettings::default()
    };
    gateway.bindings.insert(
        "claude".to_string(),
        AgentBinding {
            enabled: true,
            model: "rl-model".to_string(),
            ..AgentBinding::default()
        },
    );
    gateway
}

fn plan(gateway: &GatewaySettings) -> RoutePlan {
    route::resolve_plan(gateway, &ToolId::Claude, "rl-model", None).unwrap()
}

/// 「请求避震」开着、而且勾了 429：用来确认网关模式下它不再对 429 重发同一个 Key。
fn retrying_settings() -> BackendSettings {
    let mut settings = BackendSettings::default();
    settings.protocol_proxy_auto_retry = true;
    settings.protocol_proxy_auto_retry_max_attempts = 5;
    settings.protocol_proxy_auto_retry_interval_seconds = 0;
    settings.protocol_proxy_auto_retry_status_codes = vec![429, 500, 502, 503];
    settings
}

async fn send(settings: &BackendSettings, plan: &RoutePlan) -> GatewayOutcome {
    pipeline::open(
        settings,
        plan,
        GatewayRequest {
            responses_body: json!({
                "model": "rl-model",
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
            auto_retry: true,
            is_stream: false,
            upstream_path: "/responses",
            client: None,
            queue_timeout: Duration::from_secs(5),
        },
    )
    .await
    .unwrap()
}

fn status(outcome: &GatewayOutcome) -> Option<u16> {
    match &outcome.upstream {
        GatewayUpstream::Pipeline(response) => Some(response.status_code),
        GatewayUpstream::Native(response) => Some(response.status_code),
        GatewayUpstream::Cooling(_) => None,
    }
}

fn cooling_ms(relay_id: &str) -> Option<u64> {
    key_pool::snapshot(Instant::now())
        .keys
        .into_iter()
        .find(|key| key.relay_id == relay_id)
        .and_then(|key| key.remaining_ms)
}

/// 单 Key 429 带 Retry-After：Key 冷却时长照上游说的来；客户端拿到的是上游的 429 原文；
/// 「请求避震」勾了 429 也只打上游一次（以前会对同一个 Key 连发 6 次）。
#[tokio::test]
async fn single_key_429_cools_for_retry_after_and_hits_upstream_once() {
    let (base, seen) = mock_upstream(Arc::new(|_, _| rate_limited(&[("retry-after", "7")]))).await;
    let gateway = gateway(vec![provider("rl-retry-after", &base, &["a"])]);
    let outcome = send(&retrying_settings(), &plan(&gateway)).await;
    assert_eq!(status(&outcome), Some(429), "上游的 429 原样交回");
    let GatewayUpstream::Pipeline(response) = &outcome.upstream else {
        panic!("应是上游响应");
    };
    let body = String::from_utf8_lossy(response.buffered_body.as_deref().unwrap_or_default());
    assert!(body.contains("upstream says slow down"), "{body}");
    assert_eq!(
        seen.lock().unwrap().len(),
        1,
        "429 不在网关里对同一个 Key 重发"
    );
    let remaining = cooling_ms("rl-retry-after").expect("Key 在冷却");
    assert!(
        (6_000..=7_000).contains(&remaining),
        "按 Retry-After 冷却 7 秒：{remaining}ms"
    );
}

/// 无提示的 429：第一次只冷却 5 秒；限流重置头给了就按它。
#[tokio::test]
async fn first_429_without_retry_after_cools_briefly() {
    let (base, _seen) = mock_upstream(Arc::new(|_, _| rate_limited(&[]))).await;
    let gateway = gateway(vec![provider("rl-no-hint", &base, &["a"])]);
    let outcome = send(&BackendSettings::default(), &plan(&gateway)).await;
    assert_eq!(status(&outcome), Some(429));
    let remaining = cooling_ms("rl-no-hint").expect("Key 在冷却");
    assert!((4_000..=5_000).contains(&remaining), "{remaining}ms");

    let (base, _seen) = mock_upstream(Arc::new(|_, _| {
        rate_limited(&[
            ("x-ratelimit-remaining-requests", "0"),
            ("x-ratelimit-reset-requests", "2s"),
        ])
    }))
    .await;
    let gateway = self::gateway(vec![provider("rl-reset-header", &base, &["a"])]);
    send(&BackendSettings::default(), &plan(&gateway)).await;
    let remaining = cooling_ms("rl-reset-header").expect("Key 在冷却");
    assert!((1_000..=2_000).contains(&remaining), "{remaining}ms");
}

/// 唯一的 Key 在冷却、很快就恢复：网关等它恢复再发，而不是马上回本地 429（gateway_keys_cooling）。
#[tokio::test]
async fn all_keys_cooling_waits_for_the_soonest_key_then_retries() {
    let (base, seen) = mock_upstream(Arc::new(|_, hit| {
        if hit == 1 {
            rate_limited(&[("retry-after", "1")])
        } else {
            ok()
        }
    }))
    .await;
    let gateway = gateway(vec![provider("rl-wait", &base, &["a"])]);
    let first = send(&BackendSettings::default(), &plan(&gateway)).await;
    assert_eq!(status(&first), Some(429));

    let plan = plan(&gateway);
    assert!(plan.candidates.is_empty(), "唯一的 Key 在冷却");
    assert_eq!(plan.waiting.len(), 1);
    let started = Instant::now();
    let second = send(&BackendSettings::default(), &plan).await;
    assert_eq!(status(&second), Some(200), "等 Key 恢复后成功");
    assert!(
        started.elapsed() >= Duration::from_millis(800),
        "确实等到了冷却结束：{:?}",
        started.elapsed()
    );
    assert_eq!(seen.lock().unwrap().len(), 2, "冷却期间不打上游");
}

/// 冷却比等待上限还久（上游要求等两分钟）：照旧马上回本地 429，不把请求挂着，也不打上游。
#[tokio::test]
async fn long_cooling_still_answers_locally_without_hitting_upstream() {
    let (base, seen) =
        mock_upstream(Arc::new(|_, _| rate_limited(&[("retry-after", "120")]))).await;
    let gateway = gateway(vec![provider("rl-long", &base, &["a"])]);
    send(&BackendSettings::default(), &plan(&gateway)).await;
    let started = Instant::now();
    let outcome = send(&BackendSettings::default(), &plan(&gateway)).await;
    assert!(
        matches!(outcome.upstream, GatewayUpstream::Cooling(wait) if wait > pipeline::COOLING_WAIT_LIMIT)
    );
    assert!(started.elapsed() < Duration::from_secs(2), "不等");
    assert_eq!(seen.lock().unwrap().len(), 1);
}

/// 只有一家供应商时熔断不拦请求：连续 5xx 后下一次照样发给它（半开），成功即恢复。
#[tokio::test]
async fn breaker_does_not_block_the_only_provider() {
    let (base, seen) = mock_upstream(Arc::new(
        |_, hit| {
            if hit <= 3 { server_error() } else { ok() }
        },
    ))
    .await;
    let gateway = gateway(vec![provider("rl-breaker-only", &base, &["a"])]);
    let settings = BackendSettings::default();
    for _ in 0..3 {
        assert_eq!(status(&send(&settings, &plan(&gateway)).await), Some(500));
    }
    let breaker_open = key_pool::snapshot(Instant::now())
        .providers
        .into_iter()
        .any(|item| item.relay_id == "rl-breaker-only" && item.breaker_open);
    assert!(breaker_open, "连续 3 次 5xx 熔断");
    let plan = plan(&gateway);
    assert_eq!(
        plan.candidates.len(),
        1,
        "没有别的候选：熔断中的供应商照常放行"
    );
    assert_eq!(status(&send(&settings, &plan).await), Some(200));
    assert_eq!(seen.lock().unwrap().len(), 4);
    let still_open = key_pool::snapshot(Instant::now())
        .providers
        .into_iter()
        .any(|item| item.relay_id == "rl-breaker-only" && item.breaker_open);
    assert!(!still_open, "成功一次关闭熔断");
}

/// 有别家可换时熔断照旧生效：请求直接去第二家，不再打熔断中的第一家。
#[tokio::test]
async fn breaker_still_routes_around_when_another_provider_exists() {
    let (bad, bad_seen) = mock_upstream(Arc::new(|_, _| server_error())).await;
    let (good, good_seen) = mock_upstream(Arc::new(|_, _| ok())).await;
    let gateway = gateway(vec![
        provider("rl-breaker-a", &bad, &["a"]),
        provider("rl-breaker-b", &good, &["a"]),
    ]);
    let settings = BackendSettings::default();
    for _ in 0..3 {
        assert_eq!(status(&send(&settings, &plan(&gateway)).await), Some(200));
    }
    assert_eq!(bad_seen.lock().unwrap().len(), 3);
    let plan = plan(&gateway);
    assert_eq!(
        plan.candidates
            .iter()
            .map(|candidate| candidate.relay.id.as_str())
            .collect::<Vec<_>>(),
        vec!["rl-breaker-b"],
        "第一家熔断后不在候选里"
    );
    assert_eq!(status(&send(&settings, &plan).await), Some(200));
    assert_eq!(bad_seen.lock().unwrap().len(), 3, "熔断中的第一家不再被打");
    assert_eq!(good_seen.lock().unwrap().len(), 4);
}

/// 多 Key：第一个 Key 429 立即换第二个，同一个 Key 不重发；一次请求只打上游两次。
#[tokio::test]
async fn multi_key_429_still_switches_to_the_next_key() {
    let (base, seen) = mock_upstream(Arc::new(|key, _| {
        if key.ends_with("-a") {
            rate_limited(&[("retry-after", "30")])
        } else {
            ok()
        }
    }))
    .await;
    let gateway = gateway(vec![provider("rl-multi", &base, &["a", "b"])]);
    let started = Instant::now();
    let outcome = send(&retrying_settings(), &plan(&gateway)).await;
    assert_eq!(status(&outcome), Some(200));
    assert!(started.elapsed() < Duration::from_secs(5), "换 Key 不等待");
    assert_eq!(
        *seen.lock().unwrap(),
        vec!["sk-rl-multi-a".to_string(), "sk-rl-multi-b".to_string()]
    );
    // 第二次请求：a 还在冷却，直接用 b。
    let outcome = send(&retrying_settings(), &plan(&gateway)).await;
    assert_eq!(status(&outcome), Some(200));
    assert_eq!(seen.lock().unwrap().last().unwrap(), "sk-rl-multi-b");
    assert_eq!(seen.lock().unwrap().len(), 3);
}
