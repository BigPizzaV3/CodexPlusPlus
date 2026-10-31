//! 别家签发的加密内容（#2777）：同一对话里先用中转、再换到官方账号（或反过来、或换到 xAI）时，
//! 新上游校验不过历史里 reasoning / compaction 项的 `encrypted_content`，会整条请求失败——
//! 直接回 4xx，或先回 200 再在流里发 `response.failed` / `error`。
//!
//! 守住这几条：报了这个错时（含 xAI / Grok 的措辞、流内拒收）去掉一批加密项重发并成功，
//! 之后同一批旧项提前去掉；每个请求最多重发两次；已经把内容转发给 Codex 后不再重发；
//! 正常请求原样转发、不碰 reasoning / compaction 项；别的 400 不重试。

use codex_plus_core::protocol_proxy::{
    ForcedRelay, ProxySessionHeaders, open_responses_proxy_request_for_relay,
    open_responses_proxy_request_with_settings_for_path,
};
use codex_plus_core::settings::{
    BackendSettings, RelayMode, RelayProfile, RelayProtocol, SettingsStore,
};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicUsize, Ordering};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

/// 中转签发、官方校验不过的推理内容。
const FOREIGN: &str = "relay-signed-reasoning";
/// 中转压缩上下文得到、官方校验不过的压缩结果。
const FOREIGN_COMPACTION: &str = "relay-signed-compaction";
/// xAI 拒收别家加密内容时的原文。
const XAI_REJECTION: &str = "Could not decrypt the provided encrypted_content. Ensure the value is the unmodified encrypted_content from a previous response.";

/// 每个用例用自己的供应商 id：被拒推理项按「供应商 + 内容」记在进程里。
fn relay(server: &MockServer, label: &str) -> RelayProfile {
    static FIXTURE_ID: AtomicUsize = AtomicUsize::new(0);
    let id = format!(
        "foreign-reasoning-{label}-{}",
        FIXTURE_ID.fetch_add(1, Ordering::Relaxed)
    );
    RelayProfile {
        relay_mode: RelayMode::PureApi,
        protocol: RelayProtocol::Responses,
        base_url: format!("{}/v1", server.uri()),
        model: "gpt-5.5".to_string(),
        model_list: "gpt-5.5".to_string(),
        api_key: "local-fixture-key".to_string(),
        // 上游 client 按 UA 等缓存并复用连接（#2761）；每个 tokio 测试是独立运行时，wiremock 又会把
        // 同一端口的假上游留给后面的用例，共用 client 会拿到上一个运行时留下的连接
        // （dispatch task is gone）。每个用例用自己的 UA，就用自己的 client。
        user_agent: format!("{id}-fixture"),
        id: id.clone(),
        name: id,
        ..RelayProfile::default()
    }
}

fn reasoning(id: &str, content: &str) -> Value {
    json!({"type": "reasoning", "id": id, "summary": [], "encrypted_content": content})
}

fn request(input: Vec<Value>) -> Value {
    json!({"model": "gpt-5.5", "stream": false, "store": false, "input": input})
}

fn completed() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "id": "resp_fixture", "object": "response", "status": "completed",
        "output": [{"type": "message", "role": "assistant",
                    "content": [{"type": "output_text", "text": "ok"}]}],
        "usage": {"input_tokens": 1, "output_tokens": 1, "total_tokens": 2}
    }))
}

/// OpenAI 对别家加密推理的原样报错。
fn unverifiable() -> ResponseTemplate {
    ResponseTemplate::new(400).set_body_json(json!({"error": {
        "message": "The encrypted content for item rs_relay could not be verified. Reason: Encrypted content item_id did not match the target item id.",
        "type": "invalid_request_error", "param": null, "code": "invalid_encrypted_content"
    }}))
}

/// 模拟官方：只要请求里还带着中转签发的推理就拒收。
async fn official_upstream() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(|incoming: &Request| {
            if String::from_utf8_lossy(&incoming.body).contains(FOREIGN) {
                unverifiable()
            } else {
                completed()
            }
        })
        .mount(&server)
        .await;
    server
}

async fn send_forced(relay: &RelayProfile, body: &Value) -> (u16, Vec<u8>) {
    send_forced_with(relay, body, AutoRetry::Off).await
}

/// 「请求避震」的几种状态：关着时只为 #2777 预读生命周期事件；开着时走它的首包前预读
/// （默认的流式预读，或整段缓冲）。
#[derive(Debug, Clone, Copy)]
enum AutoRetry {
    Off,
    Streaming,
    Buffered,
}

async fn send_forced_with(
    relay: &RelayProfile,
    body: &Value,
    auto_retry: AutoRetry,
) -> (u16, Vec<u8>) {
    let mut settings = BackendSettings::default();
    if matches!(auto_retry, AutoRetry::Buffered) {
        settings.protocol_proxy_auto_retry_mode = "buffered".to_string();
    }
    let auto_retry = !matches!(auto_retry, AutoRetry::Off);
    let mut response = open_responses_proxy_request_for_relay(
        &body.to_string(),
        settings,
        ForcedRelay {
            relay: relay.clone(),
            upstream_model: None,
            auto_retry,
        },
        None,
        "/responses",
        ProxySessionHeaders::default(),
        None,
    )
    .await
    .unwrap();
    let status = response.status_code;
    (status, response.read_body().await.unwrap())
}

async fn received(server: &MockServer) -> Vec<Value> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|request| request.body_json::<Value>().unwrap())
        .collect()
}

#[tokio::test]
async fn unverifiable_foreign_reasoning_is_stripped_and_retried_once() {
    let server = official_upstream().await;
    let relay = relay(&server, "gateway");
    let user = json!({"type": "message", "role": "user", "content": "继续"});
    let call =
        json!({"type": "function_call", "call_id": "call_1", "name": "shell", "arguments": "{}"});
    let output = json!({"type": "function_call_output", "call_id": "call_1", "output": "done"});
    let first = request(vec![
        user.clone(),
        reasoning("rs_relay", FOREIGN),
        call.clone(),
        output.clone(),
    ]);

    let (status, body) = send_forced(&relay, &first).await;
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
    let sent = received(&server).await;
    assert_eq!(sent.len(), 2, "只重发一次");
    assert_eq!(sent[0], first, "第一次原样转发");
    // 重发只去掉加密推理项，消息与工具调用历史原样保留。
    assert_eq!(sent[1]["input"], json!([user, call, output]));

    // 下一轮：旧的中转推理还在历史里，官方新签发的推理也在。旧的提前去掉，不再先吃一次 400。
    let second = request(vec![
        user.clone(),
        reasoning("rs_relay", FOREIGN),
        reasoning("rs_official", "official-signed-reasoning"),
        json!({"type": "message", "role": "user", "content": "再来"}),
    ]);
    let (status, _) = send_forced(&relay, &second).await;
    assert_eq!(status, 200);
    let sent = received(&server).await;
    assert_eq!(sent.len(), 3, "已知被拒的推理项提前去掉，一次就成功");
    let kept: Vec<&Value> = sent[2]["input"].as_array().unwrap().iter().collect();
    assert_eq!(kept.len(), 3);
    assert_eq!(kept[1]["id"], "rs_official", "官方自己签发的推理保留");
}

#[tokio::test]
async fn ordinary_requests_keep_reasoning_items_untouched() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(|incoming: &Request| {
            let body: Value = serde_json::from_slice(&incoming.body).unwrap();
            if body["stream"] == true {
                sse(&[created(), text_delta("ok"), stream_completed()])
            } else {
                completed()
            }
        })
        .mount(&server)
        .await;
    let relay = relay(&server, "ordinary");
    let body = request(vec![
        json!({"type": "message", "role": "user", "content": "hi"}),
        reasoning("rs_same", "same-provider-reasoning"),
        compaction("cmp_same", "compaction", "same-provider-compaction"),
        compaction("cmps_same", "compaction_summary", "same-provider-summary"),
        json!({"type": "message", "role": "user", "content": "again"}),
    ]);

    let (status, _) = send_forced(&relay, &body).await;
    assert_eq!(status, 200);
    let mut streamed = body.clone();
    streamed["stream"] = json!(true);
    for auto_retry in [AutoRetry::Off, AutoRetry::Streaming, AutoRetry::Buffered] {
        let (status, output) = send_forced_with(&relay, &streamed, auto_retry).await;
        assert_eq!(status, 200);
        assert!(String::from_utf8_lossy(&output).contains("response.completed"));
    }
    let sent = received(&server).await;
    assert_eq!(sent.len(), 4, "正常请求各发一次");
    assert_eq!(
        sent[0], body,
        "没切换供应商的请求不改动 reasoning / compaction 项"
    );
    assert!(sent[1..].iter().all(|sent| *sent == streamed));
}

#[tokio::test]
async fn other_bad_requests_are_not_retried() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"error": {
            "message": "Unsupported parameter: 'temperature'", "type": "invalid_request_error",
            "param": "temperature", "code": "unsupported_parameter"
        }})))
        .mount(&server)
        .await;
    let relay = relay(&server, "other-400");
    let body = request(vec![
        json!({"type": "message", "role": "user", "content": "hi"}),
        reasoning("rs_keep", "kept-reasoning"),
    ]);

    let (status, error) = send_forced(&relay, &body).await;
    assert_eq!(status, 400);
    let error: Value = serde_json::from_slice(&error).unwrap();
    assert_eq!(
        error["error"]["code"], "unsupported_parameter",
        "错误体原样交回"
    );
    let sent = received(&server).await;
    assert_eq!(sent.len(), 1, "别的 400 不重试");
    assert_eq!(sent[0], body);
}

#[tokio::test]
async fn direct_local_proxy_path_also_retries_without_foreign_reasoning() {
    // 直连模式经本地协议代理（聚合、需要转换的供应商）走的是同一条链路。
    let _guard = crate::common::global_state_lock();
    let server = official_upstream().await;
    let relay = relay(&server, "direct");
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    let settings = BackendSettings {
        active_relay_id: relay.id.clone(),
        relay_profiles: vec![relay],
        protocol_proxy_auto_retry: false,
        ..BackendSettings::default()
    };
    SettingsStore::new(path.clone()).save(&settings).unwrap();
    let previous = codex_plus_core::paths::set_settings_path_for_tests(Some(path));
    let body = request(vec![
        json!({"type": "message", "role": "user", "content": "hi"}),
        reasoning("rs_relay", FOREIGN),
    ]);

    let response = open_responses_proxy_request_with_settings_for_path(
        &body.to_string(),
        settings,
        "/v1/responses",
    )
    .await;
    codex_plus_core::paths::set_settings_path_for_tests(previous);
    let response = response.unwrap();
    assert_eq!(response.status_code, 200);
    let trace = response.request_trace.expect("请求日志轨迹");
    assert_eq!(
        trace.retry_reasons,
        vec!["invalid_encrypted_content".to_string()]
    );
    let sent = received(&server).await;
    assert_eq!(sent.len(), 2);
    assert_eq!(
        sent[1]["input"],
        json!([{"type": "message", "role": "user", "content": "hi"}])
    );
}

fn compaction(id: &str, kind: &str, content: &str) -> Value {
    json!({"type": kind, "id": id, "encrypted_content": content})
}

fn sse(events: &[Value]) -> ResponseTemplate {
    let body: String = events
        .iter()
        .map(|event| {
            format!(
                "event: {}\ndata: {event}\n\n",
                event["type"].as_str().unwrap()
            )
        })
        .collect();
    ResponseTemplate::new(200).set_body_raw(body, "text/event-stream")
}

fn created() -> Value {
    json!({"type": "response.created", "response": {"id": "resp_fixture", "status": "in_progress"}})
}

fn text_delta(text: &str) -> Value {
    json!({"type": "response.output_text.delta", "item_id": "msg_1", "output_index": 0,
           "content_index": 0, "delta": text})
}

fn stream_completed() -> Value {
    json!({"type": "response.completed", "response": {
        "id": "resp_fixture", "object": "response", "status": "completed",
        "output": [{"type": "message", "role": "assistant",
                    "content": [{"type": "output_text", "text": "ok"}]}],
        "usage": {"input_tokens": 1, "output_tokens": 1, "total_tokens": 2}
    }})
}

/// ChatGPT 后端 / Grok 的流内拒收：先 200 + response.created，再 response.failed。
fn stream_failed(message: &str) -> Value {
    json!({"type": "response.failed", "response": {
        "id": "resp_fixture", "status": "failed",
        "error": {"code": "invalid_request_error", "message": message}
    }})
}

fn contains_any(incoming: &Request, needles: &[&str]) -> bool {
    let body = String::from_utf8_lossy(&incoming.body);
    needles.iter().any(|needle| body.contains(needle))
}

#[tokio::test]
async fn xai_wording_rejection_is_stripped_and_retried() {
    // 原样的 xAI 报错（400），以及被包成「Grok (SuperGrok): …」的 422。
    for (status, message) in [
        (400, XAI_REJECTION.to_string()),
        (
            422,
            "Grok (SuperGrok): Could not decrypt the provided encrypted_content.".to_string(),
        ),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(move |incoming: &Request| {
                if contains_any(incoming, &[FOREIGN]) {
                    ResponseTemplate::new(status).set_body_json(json!({
                        "code": "Client specified an invalid argument",
                        "error": message.clone()
                    }))
                } else {
                    completed()
                }
            })
            .mount(&server)
            .await;
        let relay = relay(&server, "xai");
        let user = json!({"type": "message", "role": "user", "content": "继续"});
        let body = request(vec![user.clone(), reasoning("rs_relay", FOREIGN)]);

        let (code, output) = send_forced(&relay, &body).await;
        assert_eq!(code, 200, "{status}: {}", String::from_utf8_lossy(&output));
        let sent = received(&server).await;
        assert_eq!(sent.len(), 2, "{status}: 只重发一次");
        assert_eq!(sent[1]["input"], json!([user]));
    }
}

#[tokio::test]
async fn rejected_compaction_items_are_stripped_and_remembered() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(|incoming: &Request| {
            if contains_any(incoming, &[FOREIGN_COMPACTION]) {
                unverifiable()
            } else {
                completed()
            }
        })
        .mount(&server)
        .await;
    let relay = relay(&server, "compaction");
    let user = json!({"type": "message", "role": "user", "content": "继续"});
    let first = request(vec![
        compaction("cmp_relay", "compaction", FOREIGN_COMPACTION),
        user.clone(),
    ]);

    let (status, _) = send_forced(&relay, &first).await;
    assert_eq!(status, 200);
    let sent = received(&server).await;
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[1]["input"], json!([user]));

    // 下一轮：同一个旧压缩结果（这次以 compaction_summary 形式出现也一样）提前去掉，官方自己的推理保留。
    let second = request(vec![
        compaction("cmp_relay", "compaction", FOREIGN_COMPACTION),
        compaction("cmps_relay", "compaction_summary", FOREIGN_COMPACTION),
        user.clone(),
        reasoning("rs_official", "official-signed-reasoning"),
    ]);
    let (status, _) = send_forced(&relay, &second).await;
    assert_eq!(status, 200);
    let sent = received(&server).await;
    assert_eq!(sent.len(), 3, "已知被拒的压缩项提前去掉，一次就成功");
    assert_eq!(
        sent[2]["input"],
        json!([user, reasoning("rs_official", "official-signed-reasoning")])
    );
}

#[tokio::test]
async fn local_compaction_summary_is_kept_when_foreign_reasoning_is_stripped() {
    // Codex++ 自己生成的本地压缩摘要进代理时就展开成 user 消息，拒收重发时不会被当成别家内容去掉。
    let server = official_upstream().await;
    let relay = relay(&server, "local-summary");
    let local = json!({
        "type": "compaction", "id": "cmp_local",
        "encrypted_content": format!("codexplusplus:summary:v1:{}", json!("本地摘要内容"))
    });
    let body = request(vec![
        local,
        reasoning("rs_relay", FOREIGN),
        json!({"type": "message", "role": "user", "content": "继续"}),
    ]);

    let (status, _) = send_forced(&relay, &body).await;
    assert_eq!(status, 200);
    let sent = received(&server).await;
    assert_eq!(sent.len(), 2);
    let input = sent[1]["input"].as_array().unwrap();
    assert_eq!(input.len(), 2, "只去掉别家推理");
    assert_eq!(input[0]["type"], "message");
    assert!(input[0].to_string().contains("本地摘要内容"));
}

#[tokio::test]
async fn in_stream_rejection_before_any_content_is_retried() {
    // 没开「请求避震」（只按住生命周期事件）与开着（走它的首包前预读 / 整段缓冲）几种情况；
    // 拒收以 response.failed 或 error 事件出现。
    let error_event = json!({"type": "error", "error": {"code": "invalid_encrypted_content",
        "message": "The encrypted content for item rs_relay could not be verified."}});
    for (auto_retry, rejection) in [
        (AutoRetry::Off, stream_failed(XAI_REJECTION)),
        (AutoRetry::Off, error_event.clone()),
        (AutoRetry::Streaming, error_event),
        (
            AutoRetry::Buffered,
            stream_failed("Grok (SuperGrok): Could not decrypt the provided encrypted_content."),
        ),
    ] {
        let server = MockServer::start().await;
        let failure = rejection.clone();
        Mock::given(method("POST"))
            .respond_with(move |incoming: &Request| {
                if contains_any(incoming, &[FOREIGN]) {
                    sse(&[
                        created(),
                        json!({"type": "response.in_progress", "response": {"id": "resp_fixture"}}),
                        failure.clone(),
                    ])
                } else {
                    sse(&[created(), text_delta("ok"), stream_completed()])
                }
            })
            .mount(&server)
            .await;
        let relay = relay(&server, "stream");
        let user = json!({"type": "message", "role": "user", "content": "继续"});
        let mut body = request(vec![user.clone(), reasoning("rs_relay", FOREIGN)]);
        body["stream"] = json!(true);

        let (status, output) = send_forced_with(&relay, &body, auto_retry).await;
        let output = String::from_utf8_lossy(&output);
        assert_eq!(status, 200);
        assert!(
            output.contains("response.completed"),
            "{auto_retry:?}: {output}"
        );
        assert!(
            !output.contains("response.failed"),
            "{auto_retry:?}: 被拒的那次不转发"
        );
        assert!(
            !output.contains("encrypted_content"),
            "{auto_retry:?}: {output}"
        );
        let sent = received(&server).await;
        assert_eq!(sent.len(), 2, "{auto_retry:?}: 只重发一次");
        assert_eq!(sent[1]["input"], json!([user]));
    }
}

#[tokio::test]
async fn in_stream_rejection_after_forwarded_content_is_not_retried() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse(&[
            created(),
            text_delta("partial"),
            stream_failed(XAI_REJECTION),
        ]))
        .mount(&server)
        .await;
    let relay = relay(&server, "stream-forwarded");
    let mut body = request(vec![
        json!({"type": "message", "role": "user", "content": "继续"}),
        reasoning("rs_relay", FOREIGN),
    ]);
    body["stream"] = json!(true);

    let (status, output) = send_forced(&relay, &body).await;
    let output = String::from_utf8_lossy(&output);
    assert_eq!(status, 200);
    assert!(output.contains("partial"), "已转发的内容原样交给客户端");
    assert!(output.contains("Could not decrypt"), "错误原样交给客户端");
    let sent = received(&server).await;
    assert_eq!(sent.len(), 1, "已经转发了内容就不再重发");
    assert_eq!(sent[0], body);

    // 没重发，也没记住：下一轮同样的推理照常发出。
    let (_, _) = send_forced(&relay, &body).await;
    let sent = received(&server).await;
    assert_eq!(sent[1], body);
}

#[tokio::test]
async fn second_rejection_strips_next_batch_and_third_is_not_retried() {
    // 推理和压缩结果都是别家签发的：第一次去掉推理后仍被拒，第二次再去掉压缩项后成功。
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(|incoming: &Request| {
            if contains_any(incoming, &[FOREIGN, FOREIGN_COMPACTION]) {
                unverifiable()
            } else {
                completed()
            }
        })
        .mount(&server)
        .await;
    let switched = relay(&server, "two-batches");
    let user = json!({"type": "message", "role": "user", "content": "继续"});
    let cmp = compaction("cmp_relay", "compaction", FOREIGN_COMPACTION);
    let body = request(vec![
        cmp.clone(),
        user.clone(),
        reasoning("rs_relay", FOREIGN),
    ]);

    let (status, output) = send_forced(&switched, &body).await;
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&output));
    let sent = received(&server).await;
    assert_eq!(sent.len(), 3);
    assert_eq!(
        sent[1]["input"],
        json!([cmp, user]),
        "先去推理，压缩项留到最后"
    );
    assert_eq!(sent[2]["input"], json!([user]));

    // 上游一直拒收：两次重发之后不再重发，错误原样交回。
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(unverifiable())
        .mount(&server)
        .await;
    let rejecting = relay(&server, "always-rejects");
    let body = request(vec![
        compaction("cmp_other", "compaction", "other-compaction"),
        user.clone(),
        reasoning("rs_other", "other-reasoning"),
    ]);
    let (status, error) = send_forced(&rejecting, &body).await;
    assert_eq!(status, 400);
    let error: Value = serde_json::from_slice(&error).unwrap();
    assert_eq!(error["error"]["code"], "invalid_encrypted_content");
    let sent = received(&server).await;
    assert_eq!(sent.len(), 3, "最多重发两次");
    assert_eq!(sent[2]["input"], json!([user]));
}

/// 同 [`send_forced`]，另外交回请求日志轨迹（重试原因、自动重试次数）。
async fn send_forced_traced(
    relay: &RelayProfile,
    body: &Value,
) -> (u16, codex_plus_core::request_log::RequestTrace) {
    let mut response = open_responses_proxy_request_for_relay(
        &body.to_string(),
        BackendSettings::default(),
        ForcedRelay {
            relay: relay.clone(),
            upstream_model: None,
            auto_retry: false,
        },
        None,
        "/responses",
        ProxySessionHeaders::default(),
        None,
    )
    .await
    .unwrap();
    let status = response.status_code;
    let trace = response.request_trace.take().expect("请求日志轨迹");
    response.read_body().await.unwrap();
    (status, trace)
}

#[tokio::test]
async fn dropping_foreign_compaction_is_flagged_in_request_log() {
    // 推理和压缩结果都是别家签发的：第二批去掉压缩结果时，请求日志单独标出「已去掉别家压缩上下文」，
    // 它不算一次重试；之后按记忆提前去掉同一压缩结果的请求也照样标出。
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(|incoming: &Request| {
            if contains_any(incoming, &[FOREIGN, FOREIGN_COMPACTION]) {
                unverifiable()
            } else {
                completed()
            }
        })
        .mount(&server)
        .await;
    let relay = relay(&server, "compaction-flag");
    let user = json!({"type": "message", "role": "user", "content": "继续"});
    let body = request(vec![
        compaction("cmp_relay", "compaction", FOREIGN_COMPACTION),
        user.clone(),
        reasoning("rs_relay", FOREIGN),
    ]);

    let (status, trace) = send_forced_traced(&relay, &body).await;
    assert_eq!(status, 200);
    assert_eq!(trace.auto_retries, 2);
    assert_eq!(
        trace.retry_reasons,
        vec![
            "invalid_encrypted_content".to_string(),
            "invalid_encrypted_content".to_string(),
            "foreign_compaction_dropped".to_string(),
        ]
    );

    // 下一轮：压缩结果按记忆提前去掉，一次就成功，但仍要标出来。
    let (status, trace) = send_forced_traced(&relay, &body).await;
    assert_eq!(status, 200);
    assert_eq!(trace.auto_retries, 0);
    assert_eq!(
        trace.retry_reasons,
        vec!["foreign_compaction_dropped".to_string()]
    );
    assert_eq!(received(&server).await.len(), 4);

    // 只去掉推理的请求不带这个标记。
    let reasoning_only = request(vec![user.clone(), reasoning("rs_relay", FOREIGN)]);
    let (status, trace) = send_forced_traced(&relay, &reasoning_only).await;
    assert_eq!(status, 200);
    assert!(
        !trace
            .retry_reasons
            .iter()
            .any(|reason| reason == "foreign_compaction_dropped"),
        "{:?}",
        trace.retry_reasons
    );
}

#[tokio::test]
async fn rejection_by_one_key_does_not_strip_for_another_key_of_same_relay() {
    // 同一供应商配了两个 Key：只有 Key B 背后的账号校验不过这段推理（它是 Key A 签发的）。
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(|incoming: &Request| {
            let key_b = incoming
                .headers
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.contains("key-b"));
            if key_b && contains_any(incoming, &[FOREIGN]) {
                unverifiable()
            } else {
                completed()
            }
        })
        .mount(&server)
        .await;
    let key_a = RelayProfile {
        api_key: "sk-fixture-key-a".to_string(),
        ..relay(&server, "two-keys")
    };
    let key_b = RelayProfile {
        api_key: "sk-fixture-key-b".to_string(),
        ..key_a.clone()
    };
    let user = json!({"type": "message", "role": "user", "content": "继续"});
    let body = request(vec![user.clone(), reasoning("rs_a", FOREIGN)]);

    // Key B 拒收：去掉推理重发。
    let (status, _) = send_forced(&key_b, &body).await;
    assert_eq!(status, 200);
    let sent = received(&server).await;
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[1]["input"], json!([user]));

    // 轮回 Key A：它自己签发的推理原样发出，不被提前去掉。
    let (status, _) = send_forced(&key_a, &body).await;
    assert_eq!(status, 200);
    let sent = received(&server).await;
    assert_eq!(sent.len(), 3);
    assert_eq!(sent[2], body, "发给 Key A 的请求原样转发");

    // 再发给 Key B：按记忆提前去掉，一次就成功。
    let (status, _) = send_forced(&key_b, &body).await;
    assert_eq!(status, 200);
    let sent = received(&server).await;
    assert_eq!(sent.len(), 4);
    assert_eq!(sent[3]["input"], json!([user]));
}
