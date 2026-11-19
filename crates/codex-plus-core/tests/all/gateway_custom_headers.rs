//! #3075：网关供应商池的自定义请求头。
//!
//! 保存往返（敏感头不回显、没改就沿用、换站点不沿用凭据），以及测试连接、拉模型列表、
//! 实际转发三处都把头带给上游。Key 池是进程内全局状态：每个用例用自己的供应商名。

use codex_plus_core::gateway::apply::{ProviderInput, create_provider, update_provider};
use codex_plus_core::gateway::pipeline::{self, GatewayRequest};
use codex_plus_core::gateway::probe::probe_provider;
use codex_plus_core::gateway::provider_headers::{self, ProviderHeaderInput};
use codex_plus_core::gateway::store::{AgentBinding, GatewayStore};
use codex_plus_core::gateway::{model_sync, route};
use codex_plus_core::protocol_proxy::ProxySessionHeaders;
use codex_plus_core::settings::{BackendSettings, RelayHeaderKeyValue, RelayProfile, RelayProtocol};
use codex_plus_core::tools::ToolId;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY: &str = "sk-headers-0123456789abcdefghij-3075";
const SECRET: &str = "relay-secret-0123456789abcdef";

fn row(key: &str, value: &str, keep: bool) -> ProviderHeaderInput {
    ProviderHeaderInput {
        key: key.into(),
        value: value.into(),
        keep,
    }
}

fn input(name: &str, base_url: &str, headers: Option<Vec<ProviderHeaderInput>>) -> ProviderInput {
    ProviderInput {
        name: name.into(),
        base_url: base_url.into(),
        protocol: RelayProtocol::ChatCompletions,
        keys: vec![KEY.into()],
        model: "h3075-model".into(),
        model_list: "h3075-model".into(),
        // 本机模拟上游：直连，不受系统代理影响。
        proxy: Some("direct".into()),
        custom_headers: headers,
        ..ProviderInput::default()
    }
}

fn setup(name: &str, base_url: &str) -> (tempfile::TempDir, GatewayStore, String) {
    let temp = tempfile::tempdir().unwrap();
    let store = GatewayStore::at(temp.path().join("gateway.json"));
    let headers = vec![row("X-Tenant", "acme", false), row("X-Relay-Key", SECRET, false)];
    let (_, id) = create_provider(&store, &input(name, base_url, Some(headers)), "custom", None).unwrap();
    (temp, store, id)
}

fn relay(store: &GatewayStore, id: &str) -> RelayProfile {
    store.load().unwrap().provider(id).unwrap().relay.clone()
}

fn header(key: &str, value: &str) -> RelayHeaderKeyValue {
    RelayHeaderKeyValue {
        key: key.into(),
        value: value.into(),
    }
}

#[test]
fn saved_headers_round_trip_without_exposing_secrets() {
    let (_temp, store, id) = setup("Headers Save 3075", "https://relay-3075.test/v1");
    let saved = relay(&store, &id);
    assert_eq!(
        saved.custom_headers,
        vec![header("X-Tenant", "acme"), header("X-Relay-Key", SECRET)]
    );
    // 给界面的数据：普通头原样，敏感头只有打码后的样子。
    let view = provider_headers::view(&saved);
    assert_eq!(view[0].value, "acme");
    assert_eq!(view[1].value, "");
    assert!(view[1].masked.is_some());
    assert!(!serde_json::to_string(&view).unwrap().contains(SECRET));

    // 改普通头、敏感头没改（keep）：沿用已保存的值。
    let edit = vec![row("X-Tenant", "acme2", false), row("X-Relay-Key", "", true)];
    update_provider(&store, &id, &input("Headers Save 3075", "https://relay-3075.test/v2", Some(edit))).unwrap();
    assert_eq!(
        relay(&store, &id).custom_headers,
        vec![header("X-Tenant", "acme2"), header("X-Relay-Key", SECRET)]
    );

    // 不传请求头（旧版界面）：原样保留。
    update_provider(&store, &id, &input("Headers Save 3075", "https://relay-3075.test/v2", None)).unwrap();
    assert_eq!(relay(&store, &id).custom_headers.len(), 2);

    // 地址换到别的站点还想沿用敏感头：拒绝，免得把这个站点的凭据发到新站点；原配置不动。
    let moved = vec![row("X-Tenant", "acme2", false), row("X-Relay-Key", "", true)];
    let error = update_provider(&store, &id, &input("Headers Save 3075", "https://elsewhere-3075.test/v1", Some(moved)))
        .unwrap_err()
        .to_string();
    assert!(error.contains("X-Relay-Key"), "{error}");
    assert!(!error.contains(SECRET));
    assert_eq!(relay(&store, &id).base_url, "https://relay-3075.test/v2");
    assert_eq!(relay(&store, &id).custom_headers[1].value, SECRET);

    // 传输头照直连模式的规则拒绝。
    assert!(update_provider(&store, &id, &input("Headers Save 3075", "https://relay-3075.test/v2", Some(vec![row("Host", "evil.test", false)]))).is_err());

    // 清空。
    update_provider(&store, &id, &input("Headers Save 3075", "https://relay-3075.test/v2", Some(Vec::new()))).unwrap();
    assert!(relay(&store, &id).custom_headers.is_empty());
}

#[test]
fn new_provider_cannot_keep_a_header_it_never_saved() {
    let temp = tempfile::tempdir().unwrap();
    let store = GatewayStore::at(temp.path().join("gateway.json"));
    let headers = vec![row("Authorization", "", true)];
    assert!(create_provider(&store, &input("Headers New 3075", "https://relay-3075.test/v1", Some(headers)), "custom", None).is_err());
}

fn received_header(request: &wiremock::Request, name: &str) -> Option<String> {
    request
        .headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

fn assert_carries_headers(request: &wiremock::Request) {
    assert_eq!(received_header(request, "x-tenant").as_deref(), Some("acme"));
    assert_eq!(received_header(request, "x-relay-key").as_deref(), Some(SECRET));
    assert_eq!(
        received_header(request, "authorization").as_deref(),
        Some(format!("Bearer {KEY}").as_str())
    );
}

#[tokio::test]
async fn probe_and_model_list_send_custom_headers() {
    let server = MockServer::start().await;
    let chunk = json!({ "choices": [{ "index": 0, "delta": { "content": "OK" }, "finish_reason": null }] });
    let stop = json!({ "choices": [{ "index": 0, "delta": {}, "finish_reason": "stop" }] });
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(format!("data: {chunk}\n\ndata: {stop}\n\ndata: [DONE]\n\n")),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "list",
            "data": [{ "id": "h3075-model", "object": "model" }, { "id": "h3075-new", "object": "model" }]
        })))
        .mount(&server)
        .await;
    let (_temp, store, id) = setup("Headers Probe 3075", &format!("{}/v1", server.uri()));

    let result = probe_provider(&BackendSettings::default(), &store.load().unwrap(), &id, "", None)
        .await
        .unwrap();
    assert!(result.ok, "{result:?}");
    let preview = model_sync::preview_provider_models(&store, &id).await.unwrap();
    let text = serde_json::to_string(&preview).unwrap();
    assert!(text.contains("h3075-new"), "{text}");

    let requests = server.received_requests().await.unwrap();
    let probe = requests.iter().find(|request| request.url.path() == "/v1/chat/completions").expect("测试连接没发出请求");
    assert_carries_headers(probe);
    let models = requests.iter().find(|request| request.url.path() == "/v1/models").expect("没拉模型列表");
    assert_carries_headers(models);
}

#[tokio::test]
async fn gateway_requests_send_custom_headers() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "c", "object": "chat.completion", "model": "h3075-model",
            "choices": [{ "index": 0, "message": { "role": "assistant", "content": "ok" }, "finish_reason": "stop" }]
        })))
        .mount(&server)
        .await;
    let (_temp, store, id) = setup("Headers Route 3075", &format!("{}/v1", server.uri()));
    let mut gateway = store.load().unwrap();
    gateway.auto_models = false;
    let model = format!("{id}/h3075-model");
    gateway.bindings.insert(
        "claude".to_string(),
        AgentBinding {
            enabled: true,
            model: model.clone(),
            ..AgentBinding::default()
        },
    );
    let plan = route::resolve_plan(&gateway, &ToolId::Claude, &model, None).unwrap();
    pipeline::open(
        &BackendSettings::default(),
        &plan,
        GatewayRequest {
            responses_body: json!({
                "model": model,
                "input": [{ "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "hi" }] }]
            }),
            anthropic: None,
            user_agent: Some("h3075-test"),
            session_headers: ProxySessionHeaders::default(),
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
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_carries_headers(&requests[0]);
}
