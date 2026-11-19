//! #3084：OpenCode Go 与 Command Code 的套餐额度。
//!
//! 用本机模拟上游核对：请求的路径、顺序、鉴权头，响应映射成 5 小时 / 每周 / 每月窗口，
//! 401 / 403 / 429 / 其它状态 / 非 JSON / 网络失败各自的错误，以及认不出的地址不发请求。
//! 每个用例用自己的供应商 id，额度缓存、在途去重这些进程内状态互不干扰。

use codex_plus_core::gateway::coding_plan::{
    CodingPlan, fetch_command_code_at, fetch_opencode_go_at, plan_for_relay,
};
use codex_plus_core::gateway::quota;
use codex_plus_core::settings::{RelayMode, RelayProfile};
use serde_json::json;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY: &str = "sk-plan-0123456789abcdefghij-3084";
const NOW: u64 = 1_790_000_000_000;

fn relay(id: &str, base_url: &str) -> RelayProfile {
    RelayProfile {
        id: id.to_string(),
        name: id.to_string(),
        relay_mode: RelayMode::PureApi,
        base_url: base_url.to_string(),
        api_key: KEY.to_string(),
        // 本机模拟上游：直连，不受系统代理影响。
        gateway_proxy: Some("direct".to_string()),
        ..RelayProfile::default()
    }
}

async fn reply(server: &MockServer, route: &str, status: u16, body: &str) {
    Mock::given(method("GET"))
        .and(path(route))
        .respond_with(ResponseTemplate::new(status).set_body_string(body.to_string()))
        .mount(server)
        .await;
}

#[test]
fn only_api_key_providers_with_a_known_plan_url_are_quota_targets() {
    assert_eq!(
        plan_for_relay(&relay("p3084-a", "https://opencode.ai/zen/go/v1")),
        Some(CodingPlan::OpencodeGo)
    );
    assert_eq!(
        plan_for_relay(&relay("p3084-b", "https://api.commandcode.ai/provider/v1")),
        Some(CodingPlan::CommandCode)
    );
    assert!(quota::supports(&relay("p3084-a", "https://opencode.ai/zen/go/v1")));
    // 按量的 Zen、普通厂商：不查。
    assert!(!quota::supports(&relay("p3084-c", "https://opencode.ai/zen/v1")));
    assert!(!quota::supports(&relay("p3084-d", "https://api.deepseek.com/v1")));
}

#[tokio::test]
async fn unrecognized_base_url_never_sends_a_quota_request() {
    let server = MockServer::start().await;
    let plain = relay("p3084-plain", &format!("{}/v1", server.uri()));
    assert!(quota::refresh(&plain).await.unwrap().is_none());
    assert!(server.received_requests().await.unwrap().is_empty(), "认不出的地址不该发请求");
}

#[tokio::test]
async fn opencode_go_reads_usage_with_bearer_key() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/zen/go/v1/usage"))
        .and(header("authorization", format!("Bearer {KEY}").as_str()))
        .and(header("accept", "application/json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"usage": {
            "rolling": {"status": "ok", "percent": 37, "resetsAt": "2026-08-26T14:12:03.000Z"},
            "weekly": {"status": "ok", "percent": 0, "resetsAt": "2026-08-31T00:00:00.000Z"},
            "monthly": {"status": "ok", "percent": 5.5, "resetsAt": "2026-09-11T00:00:00.000Z"}
        }})))
        .mount(&server)
        .await;
    let relay = relay("p3084-go", "https://opencode.ai/zen/go/v1");
    let quota = fetch_opencode_go_at(&relay, &format!("{}/zen/go/v1/usage", server.uri()), NOW)
        .await
        .unwrap();
    let windows: Vec<_> = quota
        .windows
        .iter()
        .map(|window| (window.window_minutes, window.used_percent, window.resets_at_ms.is_some()))
        .collect();
    assert_eq!(
        windows,
        vec![(Some(300), 37.0, true), (Some(10080), 0.0, false), (None, 5.5, true)]
    );
    assert_eq!(quota.updated_at_ms, NOW);
}

#[tokio::test]
async fn opencode_go_errors_are_distinguishable() {
    let cases = [
        (401, "{}", "HTTP 401"),
        (403, "{}", "没有 OpenCode Go 订阅"),
        (429, "slow down", "HTTP 429"),
        (500, r#"{"error":{"code":"internal_error","message":"key sk-leak"}}"#, "internal_error"),
        (200, "not-json", "格式不认识"),
        (200, r#"{"rollingUsage":{"usagePercent":37}}"#, "格式不认识"),
    ];
    for (index, (status, body, expected)) in cases.into_iter().enumerate() {
        let server = MockServer::start().await;
        reply(&server, "/usage", status, body).await;
        let relay = relay(&format!("p3084-go-err-{index}"), "https://opencode.ai/zen/go/v1");
        let error = fetch_opencode_go_at(&relay, &format!("{}/usage", server.uri()), NOW)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains(expected), "HTTP {status}: {error}");
        assert!(!error.contains(KEY) && !error.contains("sk-leak"), "错误里不能带 Key 或响应正文：{error}");
    }
    // 403 与 401 的提示不同（Key 有效但没有订阅）。
    let server = MockServer::start().await;
    reply(&server, "/usage", 401, "{}").await;
    let unauthorized = fetch_opencode_go_at(&relay("p3084-go-401", "https://opencode.ai/zen/go"), &format!("{}/usage", server.uri()), NOW)
        .await
        .unwrap_err()
        .to_string();
    assert!(unauthorized.contains("无效") && !unauthorized.contains("订阅"), "{unauthorized}");
}

#[tokio::test]
async fn opencode_go_network_failure_is_an_error() {
    // 绑定后立即关掉：连接被拒。
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let error = fetch_opencode_go_at(&relay("p3084-go-net", "https://opencode.ai/zen/go"), &format!("http://{address}/usage"), NOW)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("连不上"), "{error}");
    assert!(!error.contains(KEY));
}

async fn command_code_server() -> MockServer {
    let server = MockServer::start().await;
    reply(&server, "/alpha/whoami", 200, r#"{"success":true,"user":{"id":"u-1"},"org":{"id":"org-1"}}"#).await;
    reply(
        &server,
        "/alpha/billing/credits",
        200,
        &json!({"credits": {"monthlyCredits": 16, "purchasedCredits": 0, "freeCredits": 0},
            "windowLimits": {"limited": true,
                "fiveHour": {"used": 7, "cap": 14, "resetAt": "2026-09-13T06:00:00.000Z"},
                "weekly": {"used": 7, "cap": 35, "resetAt": 0}}})
        .to_string(),
    )
    .await;
    reply(
        &server,
        "/alpha/billing/subscriptions",
        200,
        r#"{"success":true,"data":{"status":"active","currentPeriodStart":"2026-09-13T01:01:27.000Z","currentPeriodEnd":"2026-10-13T01:01:27.000Z","planId":"individual-goat"}}"#,
    )
    .await;
    server
}

#[tokio::test]
async fn command_code_queries_four_endpoints_in_order() {
    let server = command_code_server().await;
    reply(&server, "/alpha/usage/summary", 200, r#"{"totalCost":4,"periodBasis":"billing-period"}"#).await;
    let relay = relay("p3084-cc", "https://api.commandcode.ai/provider/v1");
    let quota = fetch_command_code_at(&relay, &server.uri(), NOW).await.unwrap();
    let windows: Vec<_> = quota
        .windows
        .iter()
        .map(|window| (window.window_minutes, window.used_percent, window.resets_at_ms.is_some()))
        .collect();
    assert_eq!(
        windows,
        vec![(Some(300), 50.0, true), (Some(10080), 20.0, false), (None, 20.0, true)]
    );

    let requests = server.received_requests().await.unwrap();
    let targets: Vec<String> = requests
        .iter()
        .map(|request| match request.url.query() {
            Some(query) => format!("{}?{query}", request.url.path()),
            None => request.url.path().to_string(),
        })
        .collect();
    assert_eq!(
        targets,
        vec![
            "/alpha/whoami?limits=1".to_string(),
            "/alpha/billing/credits?orgId=org-1".to_string(),
            "/alpha/billing/subscriptions?orgId=org-1".to_string(),
            "/alpha/usage/summary?since=2026-09-13T01%3A01%3A27.000Z&orgId=org-1".to_string(),
        ]
    );
    for request in &requests {
        assert_eq!(
            request.headers.get("authorization").and_then(|value| value.to_str().ok()),
            Some(format!("Bearer {KEY}").as_str())
        );
    }
}

#[tokio::test]
async fn command_code_failures_do_not_degrade_to_zero_usage() {
    // summary 失败：整体失败，不拿 5 小时 / 每周窗口凑数。
    let server = command_code_server().await;
    reply(&server, "/alpha/usage/summary", 500, r#"{"error":"boom"}"#).await;
    let error = fetch_command_code_at(&relay("p3084-cc-500", "https://api.commandcode.ai/provider"), &server.uri(), NOW)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("HTTP 500"), "{error}");

    // 第一个请求就被拒。
    for (status, body, expected) in [
        (401, "{}", "无效或已过期"),
        (403, r#"{"error":{"code":"upgrade_required"}}"#, "upgrade_required"),
        (429, "slow down", "HTTP 429"),
    ] {
        let server = MockServer::start().await;
        reply(&server, "/alpha/whoami", status, body).await;
        let error = fetch_command_code_at(&relay(&format!("p3084-cc-{status}"), "https://api.commandcode.ai/provider"), &server.uri(), NOW)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains(expected), "HTTP {status}: {error}");
        assert_eq!(server.received_requests().await.unwrap().len(), 1, "失败后不再往下请求");
    }

    // credits 缺字段：响应格式不认识。
    let server = MockServer::start().await;
    reply(&server, "/alpha/whoami", 200, r#"{"org":{"id":"org-9"}}"#).await;
    reply(&server, "/alpha/billing/credits", 200, "{}").await;
    reply(&server, "/alpha/billing/subscriptions", 200, r#"{"data":{}}"#).await;
    reply(&server, "/alpha/usage/summary", 200, r#"{"totalCost":0}"#).await;
    let error = fetch_command_code_at(&relay("p3084-cc-shape", "https://api.commandcode.ai/provider"), &server.uri(), NOW)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("格式不认识"), "{error}");
}
