//! 网关的主动限速（#1130 / #2726）：按 Key 的 RPM / 并发上限，满了先换 Key、都满了排队。
//!
//! 每个用例用自己的供应商 id，Key 池、限速计数、会话固定这些进程内状态互不干扰。

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use codex_plus_core::gateway::key_pool::{self, KeyLimitState, KeyLimits};
use codex_plus_core::gateway::pipeline::{
    self, GatewayOutcome, GatewayRequest, LocalRateLimited, QUEUE_REASON_PREFIX,
};
use codex_plus_core::gateway::route::{self, RoutePlan};
use codex_plus_core::gateway::store::{AgentBinding, GatewayProvider, GatewaySettings};
use codex_plus_core::protocol_proxy::ProxySessionHeaders;
use codex_plus_core::settings::{
    BackendSettings, RelayApiKey, RelayMode, RelayProfile, RelayProtocol,
};
use codex_plus_core::tools::ToolId;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// 假上游：每个请求回一段 Chat Completions 答复，记下用的是哪个 Key。
async fn mock_upstream() -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = seen.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let record = record.clone();
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
                record.lock().unwrap().push(key);
                let body = json!({
                    "id": "c", "object": "chat.completion", "model": "m",
                    "choices": [{ "index": 0, "message": { "role": "assistant", "content": "ok" }, "finish_reason": "stop" }]
                })
                .to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });
    (base, seen)
}

fn provider(id: &str, base_url: &str, keys: &[&str], limits: KeyLimits) -> GatewayProvider {
    GatewayProvider {
        enabled: true,
        source: String::new(),
        price: None,
        model_prices: Default::default(),
        model_windows: Default::default(),
        context_window: None,
        proxy: String::new(),
        key_rpm: limits.rpm,
        key_concurrency: limits.concurrency,
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
            model_list: "limited-model".to_string(),
            ..RelayProfile::default()
        },
        extra: Default::default(),
    }
}

fn gateway(providers: Vec<GatewayProvider>) -> GatewaySettings {
    let mut gateway = GatewaySettings {
        providers,
        // 多家都有同一个模型时按池顺序依次试，不走自动组（不牵扯组的会话固定）。
        auto_models: false,
        ..GatewaySettings::default()
    };
    gateway.bindings.insert(
        "claude".to_string(),
        AgentBinding {
            enabled: true,
            model: "limited-model".to_string(),
            ..AgentBinding::default()
        },
    );
    gateway
}

fn plan(gateway: &GatewaySettings, conversation: Option<&str>) -> RoutePlan {
    route::resolve_plan(gateway, &ToolId::Claude, "limited-model", conversation).unwrap()
}

async fn send(
    plan: &RoutePlan,
    conversation: Option<&str>,
    queue_timeout: Duration,
) -> anyhow::Result<GatewayOutcome> {
    pipeline::open(
        &BackendSettings::default(),
        plan,
        GatewayRequest {
            responses_body: json!({
                "model": "limited-model",
                "input": [{ "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "hi" }] }]
            }),
            anthropic: None,
            user_agent: None,
            session_headers: ProxySessionHeaders {
                session_id: conversation,
                thread_id: None,
                opencode_session: None,
                client_identity: &[],
            },
            live_request_id: None,
            auto_retry: false,
            is_stream: false,
            upstream_path: "/responses",
            client: None,
            queue_timeout,
        },
    )
    .await
}

fn limits(rpm: Option<u32>, concurrency: Option<u32>) -> KeyLimits {
    KeyLimits { rpm, concurrency }
}

fn queued(outcome: &GatewayOutcome) -> bool {
    outcome
        .trace
        .retry_reasons
        .iter()
        .any(|reason| reason.starts_with(QUEUE_REASON_PREFIX))
}

#[tokio::test]
async fn rpm_full_key_switches_to_the_next_key_without_cooling() {
    let (base, seen) = mock_upstream().await;
    let gateway = gateway(vec![provider(
        "kl-rpm",
        &base,
        &["a", "b"],
        limits(Some(1), None),
    )]);
    for _ in 0..2 {
        let outcome = send(&plan(&gateway, None), None, Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(outcome.trace.attempts, 1, "跳过满了的 Key 不算一次尝试");
        assert!(!queued(&outcome), "还有别的 Key 可用时不排队");
    }
    assert_eq!(
        *seen.lock().unwrap(),
        vec!["sk-kl-rpm-a".to_string(), "sk-kl-rpm-b".to_string()]
    );
    let health = key_pool::snapshot(Instant::now());
    assert!(
        health.keys.iter().all(|key| key.relay_id != "kl-rpm"),
        "限速满了不冷却：{health:?}"
    );
    assert!(
        health
            .providers
            .iter()
            .all(|item| item.relay_id != "kl-rpm")
    );
}

#[tokio::test]
async fn full_provider_falls_through_to_the_next_provider() {
    let (base, seen) = mock_upstream().await;
    let gateway = gateway(vec![
        provider("kl-first", &base, &["a"], limits(None, Some(1))),
        provider("kl-second", &base, &["a"], KeyLimits::default()),
    ]);
    let plan = plan(&gateway, None);
    let held = key_pool::try_acquire(
        &plan.candidates[0].key,
        plan.candidates[0].limits,
        Instant::now(),
    )
    .unwrap();
    let outcome = send(&plan, None, Duration::from_secs(5)).await.unwrap();
    assert_eq!(outcome.trace.relay_id, "kl-second");
    assert_eq!(*seen.lock().unwrap(), vec!["sk-kl-second-a".to_string()]);
    drop(held);
}

#[tokio::test]
async fn concurrency_slot_is_held_until_the_outcome_is_dropped() {
    let (base, _seen) = mock_upstream().await;
    let gateway = gateway(vec![provider(
        "kl-hold",
        &base,
        &["a"],
        limits(None, Some(1)),
    )]);
    let plan = plan(&gateway, None);
    let key = plan.candidates[0].key.clone();
    let outcome = send(&plan, None, Duration::from_secs(5)).await.unwrap();
    assert!(outcome.slot.is_some());
    assert_eq!(
        key_pool::limit_state(&key, plan.candidates[0].limits, Instant::now()),
        KeyLimitState::Full { retry_in: None },
        "响应还没转发完，名额一直占着"
    );
    drop(outcome);
    assert_eq!(
        key_pool::limit_state(&key, plan.candidates[0].limits, Instant::now()),
        KeyLimitState::Free
    );
}

#[tokio::test]
async fn all_full_waits_for_a_released_slot_then_continues() {
    let (base, seen) = mock_upstream().await;
    let gateway = gateway(vec![provider(
        "kl-wait",
        &base,
        &["a"],
        limits(None, Some(1)),
    )]);
    let plan = plan(&gateway, None);
    let held = key_pool::try_acquire(
        &plan.candidates[0].key,
        plan.candidates[0].limits,
        Instant::now(),
    )
    .unwrap();
    let started = Instant::now();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        drop(held);
    });
    let outcome = send(&plan, None, Duration::from_secs(10)).await.unwrap();
    assert!(started.elapsed() >= Duration::from_millis(250));
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "释放后马上被唤醒"
    );
    assert!(
        queued(&outcome),
        "请求日志标出限速排队：{:?}",
        outcome.trace.retry_reasons
    );
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn rpm_queue_wakes_when_the_window_slides() {
    let (base, seen) = mock_upstream().await;
    let gateway = gateway(vec![provider(
        "kl-rpm-wait",
        &base,
        &["a"],
        limits(Some(1), None),
    )]);
    let plan = plan(&gateway, None);
    // 59.5 秒前发过一次：再过约半秒滑出窗口。
    let past = Instant::now() - Duration::from_millis(59_500);
    drop(key_pool::try_acquire(&plan.candidates[0].key, plan.candidates[0].limits, past).unwrap());
    let started = Instant::now();
    let outcome = send(&plan, None, Duration::from_secs(10)).await.unwrap();
    assert!(started.elapsed() >= Duration::from_millis(300));
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(queued(&outcome));
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn queue_times_out_with_a_local_rate_limit_error() {
    let (base, seen) = mock_upstream().await;
    let gateway = gateway(vec![provider(
        "kl-timeout",
        &base,
        &["a", "b"],
        limits(Some(5), Some(1)),
    )]);
    let plan = plan(&gateway, None);
    let held: Vec<_> = plan
        .candidates
        .iter()
        .map(|candidate| {
            key_pool::try_acquire(&candidate.key, candidate.limits, Instant::now()).unwrap()
        })
        .collect();
    let started = Instant::now();
    let error = match send(&plan, None, Duration::from_millis(200)).await {
        Ok(_) => panic!("名额一直不释放，应该超时"),
        Err(error) => error,
    };
    assert!(started.elapsed() >= Duration::from_millis(150));
    let limited = error
        .downcast_ref::<LocalRateLimited>()
        .expect("超时返回本地限速错误");
    assert!(
        limited.message.contains("Codex++ 本地限速"),
        "{}",
        limited.message
    );
    assert!(
        limited.message.contains("「kl-timeout」"),
        "{}",
        limited.message
    );
    assert!(
        limited.message.contains("每分钟 5 次"),
        "{}",
        limited.message
    );
    assert!(limited.message.contains("调高"), "{}", limited.message);
    assert!(seen.lock().unwrap().is_empty(), "一个请求都没发出去");
    let health = key_pool::snapshot(Instant::now());
    assert!(health.keys.iter().all(|key| key.relay_id != "kl-timeout"));
    drop(held);
}

#[tokio::test]
async fn full_sticky_key_is_replaced_temporarily_without_changing_the_pin() {
    let (base, seen) = mock_upstream().await;
    let gateway = gateway(vec![provider(
        "kl-sticky",
        &base,
        &["a", "b"],
        limits(None, Some(1)),
    )]);
    let conversation = Some("kl-sticky-conv");
    // 先让会话固定到 Key b：a 满着时 b 回答不算固定，所以手动记一次成功。
    let key_b = key_pool::expand_keys(&gateway.providers[0].relay)[1]
        .key
        .clone();
    key_pool::record_success(&key_b, conversation);
    let plan_b = plan(&gateway, conversation);
    assert_eq!(plan_b.candidates[0].key.key_id, "b", "会话固定在 b");
    let held = key_pool::try_acquire(
        &plan_b.candidates[0].key,
        plan_b.candidates[0].limits,
        Instant::now(),
    )
    .unwrap();
    // b 满了：临时用 a。
    let outcome = send(&plan_b, conversation, Duration::from_secs(5))
        .await
        .unwrap();
    drop(outcome);
    drop(held);
    // 记住的还是 b。
    let again = plan(&gateway, conversation);
    assert_eq!(
        again.candidates[0].key.key_id, "b",
        "临时顶替不改记住的 Key"
    );
    let outcome = send(&again, conversation, Duration::from_secs(5))
        .await
        .unwrap();
    drop(outcome);
    assert_eq!(
        *seen.lock().unwrap(),
        vec!["sk-kl-sticky-a".to_string(), "sk-kl-sticky-b".to_string()]
    );
}

#[tokio::test]
async fn unlimited_providers_behave_as_before() {
    let (base, seen) = mock_upstream().await;
    let gateway = gateway(vec![provider(
        "kl-free",
        &base,
        &["a", "b"],
        KeyLimits::default(),
    )]);
    let mut outcomes = Vec::new();
    for _ in 0..5 {
        let outcome = send(&plan(&gateway, None), None, Duration::ZERO)
            .await
            .unwrap();
        assert!(!queued(&outcome));
        // 同时持有多个响应也不会被拦：没设上限就不计数。
        outcomes.push(outcome);
    }
    assert_eq!(seen.lock().unwrap().len(), 5);
    assert!(
        seen.lock().unwrap().iter().all(|key| key == "sk-kl-free-a"),
        "默认会话粘性：一直用第一个 Key"
    );
}
