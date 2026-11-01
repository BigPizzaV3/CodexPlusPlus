//! #2783：Codex 经网关时，上游要收到 Codex 自己的客户端标识。
//!
//! 开了「仅允许 Codex 官方客户端」的中转站按 UA、`originator` 和 `x-codex-*` 头认客户端，
//! 网关只转 UA 时会被 403 拒绝。这里用假上游核对网关真正发出去的请求头。
//!
//! 每个用例用自己的供应商 id 与模型名，Key 池、会话固定这些进程内状态互不干扰。

use codex_plus_core::gateway::pipeline::{self, GatewayRequest};
use codex_plus_core::gateway::route;
use codex_plus_core::gateway::server::client_identity_headers;
use codex_plus_core::gateway::store::{AgentBinding, GatewayProvider, GatewaySettings};
use codex_plus_core::protocol_proxy::{ProxySessionHeaders, codex_client_identity_headers};
use codex_plus_core::settings::{
    BackendSettings, RelayHeaderKeyValue, RelayMode, RelayProfile, RelayProtocol,
};
use codex_plus_core::tools::ToolId;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const CODEX_UA: &str =
    "codex_exec/0.165.0 (Mac OS 26.5.2; arm64) Apple_Terminal/470.2 (codex_exec; 0.165.0)";

/// Codex 实测发出的请求头（另加几条不该透传的：鉴权、Cookie、Host、长度、粘性路由令牌）。
fn codex_raw_headers() -> String {
    [
        "POST /gw/codex/v1/responses HTTP/1.1",
        "host: 127.0.0.1:63130",
        "content-length: 123",
        "authorization: Bearer gateway-token-from-codex",
        "cookie: session=secret",
        &format!("user-agent: {CODEX_UA}"),
        "originator: codex_exec",
        "x-codex-beta-features: remote_compaction_v2",
        "x-codex-window-id: 01a188b8-86ec-7480-afef-515fd89327ba:0",
        r#"x-codex-turn-metadata: {"session_id":"01a188b8","turn_id":"t1"}"#,
        "x-client-request-id: 01a188b8-86ec-7480-afef-515fd89327ba",
        "x-codex-turn-state: sticky-token-from-another-upstream",
        "session-id: 01a188b8-86ec-7480-afef-515fd89327ba",
        "thread-id: 01a188b8-86ec-7480-afef-515fd89327ba",
        "accept: text/event-stream",
    ]
    .join("\r\n")
}

fn provider(id: &str, base_url: &str, model: &str, protocol: RelayProtocol) -> GatewayProvider {
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
            protocol,
            base_url: base_url.to_string(),
            api_key: format!("sk-{id}"),
            model_list: model.to_string(),
            ..RelayProfile::default()
        },
        extra: Default::default(),
    }
}

fn gateway(agent: &str, model: &str, provider: GatewayProvider) -> GatewaySettings {
    let mut gateway = GatewaySettings {
        providers: vec![provider],
        auto_models: false,
        ..GatewaySettings::default()
    };
    gateway.bindings.insert(
        agent.to_string(),
        AgentBinding {
            enabled: true,
            model: model.to_string(),
            ..AgentBinding::default()
        },
    );
    gateway
}

async fn responses_upstream() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "resp_1", "object": "response", "status": "completed", "model": "m",
            "output": [{ "type": "message", "role": "assistant", "content": [{ "type": "output_text", "text": "ok" }] }]
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "c", "object": "chat.completion", "model": "m",
            "choices": [{ "index": 0, "message": { "role": "assistant", "content": "ok" }, "finish_reason": "stop" }]
        })))
        .mount(&server)
        .await;
    server
}

/// 按网关入口的做法从原始请求头取 UA 与身份头，经网关流水线发一次，返回上游收到的请求头。
async fn send_through_gateway(
    agent: ToolId,
    gateway: &GatewaySettings,
    model: &str,
    server: &MockServer,
) -> reqwest::header::HeaderMap {
    let raw = codex_raw_headers();
    let identity = client_identity_headers(&agent, &raw);
    let plan = route::resolve_plan(gateway, &agent, model, None).unwrap();
    let outcome = pipeline::open(
        &BackendSettings::default(),
        &plan,
        GatewayRequest {
            responses_body: json!({
                "model": model,
                "input": [{ "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "hi" }] }]
            }),
            anthropic: None,
            user_agent: Some(CODEX_UA),
            session_headers: ProxySessionHeaders {
                session_id: Some("01a188b8-86ec-7480-afef-515fd89327ba"),
                thread_id: Some("01a188b8-86ec-7480-afef-515fd89327ba"),
                opencode_session: None,
                client_identity: &identity,
            },
            live_request_id: None,
            auto_retry: false,
            is_stream: false,
            upstream_path: "/responses",
            client: None,
            queue_timeout: std::time::Duration::from_secs(5),
        },
    )
    .await
    .unwrap();
    assert_eq!(outcome.trace.attempts, 1, "假上游一次就该答复成功");
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1, "只该发出一次上游请求");
    let mut headers = reqwest::header::HeaderMap::new();
    for (name, value) in requests[0].headers.iter() {
        headers.append(
            reqwest::header::HeaderName::from_bytes(name.as_str().as_bytes()).unwrap(),
            reqwest::header::HeaderValue::from_bytes(value.as_bytes()).unwrap(),
        );
    }
    headers
}

fn header<'a>(headers: &'a reqwest::header::HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

#[test]
fn identity_whitelist_keeps_codex_identity_and_drops_credentials() {
    let identity = codex_client_identity_headers(&codex_raw_headers());
    let names: Vec<&str> = identity.iter().map(|(name, _)| name.as_str()).collect();
    for expected in [
        "originator",
        "x-codex-beta-features",
        "x-codex-window-id",
        "x-codex-turn-metadata",
        "x-client-request-id",
    ] {
        assert!(names.contains(&expected), "应当透传 {expected}：{names:?}");
    }
    for dropped in [
        "authorization",
        "cookie",
        "host",
        "content-length",
        "user-agent",
        "x-codex-turn-state",
        "session-id",
        "accept",
    ] {
        assert!(!names.contains(&dropped), "不该透传 {dropped}：{names:?}");
    }
}

#[test]
fn only_codex_agent_gets_identity_headers() {
    let raw = codex_raw_headers();
    assert!(!client_identity_headers(&ToolId::Codex, &raw).is_empty());
    assert!(client_identity_headers(&ToolId::Claude, &raw).is_empty());
    assert!(client_identity_headers(&ToolId::Grok, &raw).is_empty());
}

#[tokio::test]
async fn codex_identity_reaches_upstream_through_gateway() {
    let server = responses_upstream().await;
    let base = format!("{}/v1", server.uri());
    let model = "id2783-model-a";
    let gateway = gateway(
        "codex",
        model,
        provider("id2783-a", &base, model, RelayProtocol::Responses),
    );
    let headers = send_through_gateway(ToolId::Codex, &gateway, model, &server).await;
    assert_eq!(header(&headers, "user-agent"), Some(CODEX_UA));
    assert_eq!(header(&headers, "originator"), Some("codex_exec"));
    assert_eq!(
        header(&headers, "x-codex-window-id"),
        Some("01a188b8-86ec-7480-afef-515fd89327ba:0")
    );
    assert_eq!(
        header(&headers, "x-codex-turn-metadata"),
        Some(r#"{"session_id":"01a188b8","turn_id":"t1"}"#)
    );
    assert_eq!(
        header(&headers, "x-client-request-id"),
        Some("01a188b8-86ec-7480-afef-515fd89327ba")
    );
    assert_eq!(
        header(&headers, "x-codex-beta-features"),
        Some("remote_compaction_v2")
    );
    assert_eq!(
        header(&headers, "session-id"),
        Some("01a188b8-86ec-7480-afef-515fd89327ba")
    );
    // 鉴权用供应商自己的 Key，不是 Codex 发给网关的 token；Cookie 与粘性路由令牌不透传。
    assert_eq!(
        header(&headers, "authorization"),
        Some("Bearer sk-id2783-a")
    );
    assert!(headers.get("cookie").is_none());
    assert!(headers.get("x-codex-turn-state").is_none());
    for name in ["originator", "x-codex-window-id", "user-agent"] {
        assert_eq!(headers.get_all(name).iter().count(), 1, "{name} 只能有一个");
    }
}

#[tokio::test]
async fn configured_user_agent_and_custom_headers_win_over_codex_identity() {
    let server = responses_upstream().await;
    let base = format!("{}/v1", server.uri());
    let model = "id2783-model-b";
    let mut member = provider("id2783-b", &base, model, RelayProtocol::Responses);
    member.relay.user_agent = "MyRelayClient/1.0".to_string();
    member.relay.custom_headers.push(RelayHeaderKeyValue {
        key: "originator".to_string(),
        value: "configured-originator".to_string(),
    });
    let gateway = gateway("codex", model, member);
    let headers = send_through_gateway(ToolId::Codex, &gateway, model, &server).await;
    assert_eq!(header(&headers, "user-agent"), Some("MyRelayClient/1.0"));
    assert_eq!(
        header(&headers, "originator"),
        Some("configured-originator")
    );
    assert_eq!(headers.get_all("originator").iter().count(), 1);
    // 供应商没配的身份头照常透传。
    assert_eq!(
        header(&headers, "x-codex-window-id"),
        Some("01a188b8-86ec-7480-afef-515fd89327ba:0")
    );
    assert_eq!(
        header(&headers, "authorization"),
        Some("Bearer sk-id2783-b")
    );
}

#[tokio::test]
async fn claude_code_through_gateway_gets_no_codex_identity() {
    let server = responses_upstream().await;
    let base = format!("{}/v1", server.uri());
    let model = "id2783-model-c";
    let gateway = gateway(
        "claude",
        model,
        provider("id2783-c", &base, model, RelayProtocol::Responses),
    );
    let headers = send_through_gateway(ToolId::Claude, &gateway, model, &server).await;
    assert!(headers.get("originator").is_none());
    assert!(headers.get("x-codex-window-id").is_none());
    assert!(headers.get("x-codex-turn-metadata").is_none());
    assert!(headers.get("x-client-request-id").is_none());
    assert_eq!(
        header(&headers, "authorization"),
        Some("Bearer sk-id2783-c")
    );
}

#[tokio::test]
async fn chat_completions_upstream_does_not_get_codex_identity() {
    let server = responses_upstream().await;
    let base = format!("{}/v1", server.uri());
    let model = "id2783-model-d";
    let gateway = gateway(
        "codex",
        model,
        provider("id2783-d", &base, model, RelayProtocol::ChatCompletions),
    );
    let headers = send_through_gateway(ToolId::Codex, &gateway, model, &server).await;
    // Chat 上游走协议转换，和以前一样只带 UA 与会话头。
    assert_eq!(header(&headers, "user-agent"), Some(CODEX_UA));
    assert!(headers.get("originator").is_none());
    assert!(headers.get("x-codex-window-id").is_none());
}
