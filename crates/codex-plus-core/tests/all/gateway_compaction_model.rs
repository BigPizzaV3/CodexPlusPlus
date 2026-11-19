//! #3142：Codex 的上下文压缩请求改发到绑定里单独指定的压缩模型。
//!
//! 用两个假上游（主模型一家、压缩模型一家）核对请求真正发给了谁、发的哪个模型；
//! 每个用例用自己的供应商 id 与模型名，Key 池、会话固定这些进程内状态互不干扰。

use codex_plus_core::gateway::compaction_model;
use codex_plus_core::gateway::pipeline::GatewayRequest;
use codex_plus_core::gateway::route;
use codex_plus_core::gateway::store::{AgentBinding, GatewayProvider, GatewaySettings};
use codex_plus_core::protocol_proxy::{COMPACTION_SUMMARY_INSTRUCTION, ProxySessionHeaders};
use codex_plus_core::settings::{BackendSettings, RelayMode, RelayProfile, RelayProtocol};
use codex_plus_core::tools::ToolId;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

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
        subscription_policy: None,
        upstream_meta: Default::default(),
        auto_compact: Default::default(),
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

fn gateway(
    main: GatewayProvider,
    cheap: GatewayProvider,
    main_model: &str,
    compaction: &str,
) -> GatewaySettings {
    let mut gateway = GatewaySettings {
        providers: vec![main, cheap],
        auto_models: false,
        ..GatewaySettings::default()
    };
    gateway.bindings.insert(
        "codex".to_string(),
        AgentBinding {
            enabled: true,
            model: main_model.to_string(),
            compaction_model: compaction.to_string(),
            ..AgentBinding::default()
        },
    );
    gateway
}

/// 一家假上游：Responses 与 Chat 两个端点都按 `status` 答复。
async fn upstream(status: u16) -> MockServer {
    let server = MockServer::start().await;
    let responses = if status == 200 {
        ResponseTemplate::new(200).set_body_json(json!({
            "id": "resp_1", "object": "response", "status": "completed", "model": "m",
            "output": [{ "type": "message", "role": "assistant", "content": [{ "type": "output_text", "text": "summary" }] }]
        }))
    } else {
        ResponseTemplate::new(status).set_body_json(json!({
            "error": { "type": "invalid_request_error", "code": "model_not_found", "message": "no such model" }
        }))
    };
    let chat = if status == 200 {
        ResponseTemplate::new(200).set_body_json(json!({
            "id": "c", "object": "chat.completion", "model": "m",
            "choices": [{ "index": 0, "message": { "role": "assistant", "content": "summary" }, "finish_reason": "stop" }]
        }))
    } else {
        responses.clone()
    };
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(responses)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(chat)
        .mount(&server)
        .await;
    server
}

fn base(server: &MockServer) -> String {
    format!("{}/v1", server.uri())
}

/// Codex 的内联压缩请求：末尾是压缩提示词，不带工具。
fn inline_compaction(model: &str) -> Value {
    json!({
        "model": model,
        "tools": [],
        "reasoning": { "effort": "xhigh" },
        "input": [
            { "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "修一下登录" }] },
            { "type": "message", "role": "assistant", "content": [{ "type": "output_text", "text": "改好了" }] },
            { "type": "message", "role": "user", "content": [{ "type": "input_text", "text": COMPACTION_SUMMARY_INSTRUCTION }] }
        ]
    })
}

fn remote_v2_compaction(model: &str) -> Value {
    json!({
        "model": model,
        "input": [
            { "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "修一下登录" }] },
            { "type": "compaction_trigger" }
        ]
    })
}

fn ordinary(model: &str) -> Value {
    json!({
        "model": model,
        "input": [{ "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "修一下登录" }] }]
    })
}

/// 按网关入口的做法经压缩模型那一层发一次，返回要记进请求日志的说明。
async fn send(gateway: &GatewaySettings, body: Value) -> Vec<String> {
    let model = body["model"].as_str().unwrap().to_string();
    let plan = route::resolve_plan(gateway, &ToolId::Codex, &model, None).unwrap();
    let (outcome, notes) = compaction_model::open(
        &BackendSettings::default(),
        gateway,
        &ToolId::Codex,
        &plan,
        GatewayRequest {
            responses_body: body,
            anthropic: None,
            user_agent: None,
            session_headers: ProxySessionHeaders::default(),
            live_request_id: None,
            auto_retry: false,
            is_stream: false,
            upstream_path: "/responses",
            client: None,
            queue_timeout: std::time::Duration::from_secs(5),
        },
        None,
    )
    .await
    .unwrap();
    assert!(outcome.trace.attempts >= 1);
    notes
}

async fn received(server: &MockServer) -> Vec<Value> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|request| serde_json::from_slice(&request.body).unwrap())
        .collect()
}

#[tokio::test]
async fn inline_compaction_goes_to_compaction_model_with_its_own_effort() {
    let (main, cheap) = (upstream(200).await, upstream(200).await);
    let mut gateway = gateway(
        provider(
            "id3142-a-main",
            &base(&main),
            "id3142-a-big",
            RelayProtocol::Responses,
        ),
        provider(
            "id3142-a-cheap",
            &base(&cheap),
            "id3142-a-flash",
            RelayProtocol::Responses,
        ),
        "id3142-a-big",
        "id3142-a-flash",
    );
    gateway
        .bindings
        .get_mut("codex")
        .unwrap()
        .compaction_reasoning_effort = "low".into();
    let notes = send(&gateway, inline_compaction("id3142-a-big")).await;
    assert_eq!(notes, vec!["compaction_model:id3142-a-flash".to_string()]);
    assert!(
        received(&main).await.is_empty(),
        "主模型那家不该收到压缩请求"
    );
    let sent = received(&cheap).await;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["model"], "id3142-a-flash");
    assert_eq!(sent[0]["reasoning"]["effort"], "low");
}

#[tokio::test]
async fn ordinary_requests_keep_the_main_model() {
    let (main, cheap) = (upstream(200).await, upstream(200).await);
    let gateway = gateway(
        provider(
            "id3142-b-main",
            &base(&main),
            "id3142-b-big",
            RelayProtocol::Responses,
        ),
        provider(
            "id3142-b-cheap",
            &base(&cheap),
            "id3142-b-flash",
            RelayProtocol::Responses,
        ),
        "id3142-b-big",
        "id3142-b-flash",
    );
    let notes = send(&gateway, ordinary("id3142-b-big")).await;
    assert!(notes.is_empty());
    assert!(received(&cheap).await.is_empty());
    let sent = received(&main).await;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["model"], "id3142-b-big");
}

#[tokio::test]
async fn unconfigured_binding_keeps_compaction_on_the_main_model() {
    let (main, cheap) = (upstream(200).await, upstream(200).await);
    let gateway = gateway(
        provider(
            "id3142-c-main",
            &base(&main),
            "id3142-c-big",
            RelayProtocol::Responses,
        ),
        provider(
            "id3142-c-cheap",
            &base(&cheap),
            "id3142-c-flash",
            RelayProtocol::Responses,
        ),
        "id3142-c-big",
        "",
    );
    let notes = send(&gateway, inline_compaction("id3142-c-big")).await;
    assert!(notes.is_empty());
    assert!(received(&cheap).await.is_empty());
    let sent = received(&main).await;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["model"], "id3142-c-big");
    assert_eq!(
        sent[0]["reasoning"]["effort"], "xhigh",
        "没配置时请求原样不动"
    );
}

#[tokio::test]
async fn rejected_compaction_model_falls_back_to_main_model_once() {
    let (main, cheap) = (upstream(200).await, upstream(400).await);
    let gateway = gateway(
        provider(
            "id3142-d-main",
            &base(&main),
            "id3142-d-big",
            RelayProtocol::Responses,
        ),
        provider(
            "id3142-d-cheap",
            &base(&cheap),
            "id3142-d-flash",
            RelayProtocol::Responses,
        ),
        "id3142-d-big",
        "id3142-d-flash",
    );
    let notes = send(&gateway, inline_compaction("id3142-d-big")).await;
    assert_eq!(
        notes,
        vec!["compaction_fallback:failed:id3142-d-flash".to_string()]
    );
    assert_eq!(received(&cheap).await.len(), 1, "压缩模型只试一次");
    let sent = received(&main).await;
    assert_eq!(sent.len(), 1, "回退到原模型重发一次");
    assert_eq!(sent[0]["model"], "id3142-d-big");
}

#[tokio::test]
async fn compaction_model_with_too_small_window_is_skipped() {
    let (main, cheap) = (upstream(200).await, upstream(200).await);
    let mut small = provider(
        "id3142-e-cheap",
        &base(&cheap),
        "id3142-e-flash",
        RelayProtocol::Responses,
    );
    small.model_windows.insert("id3142-e-flash".into(), 1_000);
    let gateway = gateway(
        provider(
            "id3142-e-main",
            &base(&main),
            "id3142-e-big",
            RelayProtocol::Responses,
        ),
        small,
        "id3142-e-big",
        "id3142-e-flash",
    );
    let mut body = inline_compaction("id3142-e-big");
    body["input"][1]["content"][0]["text"] = json!("很长的历史 ".repeat(2_000));
    let notes = send(&gateway, body).await;
    assert_eq!(
        notes,
        vec!["compaction_fallback:window:id3142-e-flash".to_string()]
    );
    assert!(received(&cheap).await.is_empty());
    assert_eq!(received(&main).await.len(), 1);
}

#[tokio::test]
async fn unknown_compaction_model_falls_back() {
    let (main, cheap) = (upstream(200).await, upstream(200).await);
    let gateway = gateway(
        provider(
            "id3142-f-main",
            &base(&main),
            "id3142-f-big",
            RelayProtocol::Responses,
        ),
        provider(
            "id3142-f-cheap",
            &base(&cheap),
            "id3142-f-flash",
            RelayProtocol::Responses,
        ),
        "id3142-f-big",
        "id3142-f-gone",
    );
    let notes = send(&gateway, inline_compaction("id3142-f-big")).await;
    assert_eq!(
        notes,
        vec!["compaction_fallback:unavailable:id3142-f-gone".to_string()]
    );
    assert!(received(&cheap).await.is_empty());
    assert_eq!(received(&main).await[0]["model"], "id3142-f-big");
}

/// v2 远程压缩：Responses 原生上游签发的加密压缩项只有它自己读得懂，不能改发到别家 Responses 上游。
#[tokio::test]
async fn remote_compaction_does_not_move_to_another_responses_provider() {
    let (main, cheap) = (upstream(200).await, upstream(200).await);
    let gateway = gateway(
        provider(
            "id3142-g-main",
            &base(&main),
            "id3142-g-big",
            RelayProtocol::Responses,
        ),
        provider(
            "id3142-g-cheap",
            &base(&cheap),
            "id3142-g-flash",
            RelayProtocol::Responses,
        ),
        "id3142-g-big",
        "id3142-g-flash",
    );
    let notes = send(&gateway, remote_v2_compaction("id3142-g-big")).await;
    assert_eq!(
        notes,
        vec!["compaction_fallback:incompatible:id3142-g-flash".to_string()]
    );
    assert!(received(&cheap).await.is_empty());
    assert!(!received(&main).await.is_empty());
}

/// v2 远程压缩落到 Chat 上游时由 Codex++ 合成明文摘要，和模型无关，可以改发。
#[tokio::test]
async fn remote_compaction_can_move_to_a_chat_provider() {
    let (main, cheap) = (upstream(200).await, upstream(200).await);
    let gateway = gateway(
        provider(
            "id3142-h-main",
            &base(&main),
            "id3142-h-big",
            RelayProtocol::Responses,
        ),
        provider(
            "id3142-h-cheap",
            &base(&cheap),
            "id3142-h-flash",
            RelayProtocol::ChatCompletions,
        ),
        "id3142-h-big",
        "id3142-h-flash",
    );
    let notes = send(&gateway, remote_v2_compaction("id3142-h-big")).await;
    assert_eq!(notes, vec!["compaction_model:id3142-h-flash".to_string()]);
    assert!(received(&main).await.is_empty());
    let sent = received(&cheap).await;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["model"], "id3142-h-flash");
}

/// 历史里已有主模型那家签发的原生压缩项：别家读不了，不改发。
#[tokio::test]
async fn upstream_compaction_state_stays_with_its_provider() {
    let (main, cheap) = (upstream(200).await, upstream(200).await);
    let gateway = gateway(
        provider(
            "id3142-i-main",
            &base(&main),
            "id3142-i-big",
            RelayProtocol::Responses,
        ),
        provider(
            "id3142-i-cheap",
            &base(&cheap),
            "id3142-i-flash",
            RelayProtocol::ChatCompletions,
        ),
        "id3142-i-big",
        "id3142-i-flash",
    );
    let mut body = inline_compaction("id3142-i-big");
    body["input"].as_array_mut().unwrap().insert(
        0,
        json!({ "type": "compaction", "encrypted_content": "gAAAAABnative" }),
    );
    let notes = send(&gateway, body).await;
    assert_eq!(
        notes,
        vec!["compaction_fallback:incompatible:id3142-i-flash".to_string()]
    );
    assert!(received(&cheap).await.is_empty());
    assert_eq!(received(&main).await.len(), 1);
}

#[test]
fn old_bindings_read_and_write_without_compaction_fields() {
    let binding: AgentBinding = serde_json::from_value(json!({
        "enabled": true, "model": "gpt-5.5", "titleModel": "kimi/kimi-k3", "token": "t"
    }))
    .unwrap();
    assert_eq!(binding.compaction_model, "");
    assert_eq!(binding.compaction_reasoning_effort, "");
    let written = serde_json::to_value(&binding).unwrap();
    assert!(
        written.get("compactionModel").is_none(),
        "没配置时不写这个字段"
    );
    assert!(written.get("compactionReasoningEffort").is_none());

    let configured = AgentBinding {
        compaction_model: "deepseek/deepseek-v4-flash".into(),
        compaction_reasoning_effort: "low".into(),
        ..AgentBinding::default()
    };
    let written = serde_json::to_value(&configured).unwrap();
    assert_eq!(written["compactionModel"], "deepseek/deepseek-v4-flash");
    assert_eq!(written["compactionReasoningEffort"], "low");
    let read: AgentBinding = serde_json::from_value(written).unwrap();
    assert_eq!(read, configured);
}
