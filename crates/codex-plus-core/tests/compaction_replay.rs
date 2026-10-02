use codex_plus_core::protocol_proxy::{
    CompactionSseConverter, open_responses_proxy_request_with_settings,
};
use codex_plus_core::settings::{BackendSettings, RelayMode, RelayProfile, RelayProtocol};
use serde_json::{Value, json};
use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::method};

const SUMMARY: &str = "Remember the project decision: use Rust.\n已确认：保留回归测试。";

fn settings(url: String, protocol: RelayProtocol, id: &str) -> BackendSettings {
    BackendSettings {
        active_relay_id: id.to_string(),
        relay_profiles: vec![RelayProfile {
            id: id.to_string(),
            name: id.to_string(),
            base_url: url,
            api_key: "compaction-replay-test-key".to_string(),
            protocol,
            relay_mode: RelayMode::Official,
            official_mix_api_key: true,
            ..RelayProfile::default()
        }],
        ..BackendSettings::default()
    }
}

fn persisted_local_summary() -> Value {
    let mut converter = CompactionSseConverter::new("replay-model");
    converter.push_summary_text(SUMMARY);
    let output = String::from_utf8(converter.finish()).unwrap();
    let item = output
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter_map(|data| serde_json::from_str::<Value>(data).ok())
        .find(|event| event["type"] == "response.output_item.done")
        .expect("the summary converter must emit a completed output item")["item"]
        .clone();
    assert_eq!(item["type"], "compaction");
    assert!(item["encrypted_content"].is_string());
    // 模拟客户端持久化后的历史；额外字段（包括 id）不能作为本地摘要的识别依据。
    json!({
        "type": item["type"],
        "encrypted_content": item["encrypted_content"],
    })
}

fn replay_request(compact: bool) -> String {
    let mut input = vec![
        persisted_local_summary(),
        json!({"type": "message", "role": "user", "content": "Continue the project."}),
    ];
    if compact {
        input.push(json!({"type": "compaction_trigger"}));
    }
    json!({"model": "replay-model", "stream": false, "input": input}).to_string()
}

fn chat_reply() -> Value {
    json!({
        "id": "chatcmpl_replay",
        "choices": [{
            "message": {"role": "assistant", "content": "The Rust project continues."},
            "finish_reason": "stop",
        }],
    })
}

fn responses_reply() -> Value {
    json!({
        "id": "resp_replay",
        "object": "response",
        "status": "completed",
        "output": [{
            "type": "message",
            "role": "assistant",
            "content": [{"type": "output_text", "text": "The Rust project continues."}],
        }],
    })
}

fn has_trigger(request: &Request) -> bool {
    serde_json::from_slice::<Value>(&request.body).unwrap()["input"]
        .as_array()
        .is_some_and(|input| {
            input
                .iter()
                .any(|item| item["type"] == "compaction_trigger")
        })
}

fn assert_chat_summary(body: &Value) {
    assert!(
        body["messages"].as_array().unwrap().iter().any(|message| {
            message["role"] == "user"
                && message["content"]
                    .as_str()
                    .is_some_and(|text| text.contains(SUMMARY))
        }),
        "the upstream chat history must contain the original summary as user text: {body}"
    );
}

fn assert_responses_summary(body: &Value) {
    let input = body["input"].as_array().unwrap();
    assert!(
        input.iter().all(|item| item["type"] != "compaction"),
        "a local summary must not be sent as an opaque upstream checkpoint: {body}"
    );
    assert!(
        input.iter().any(|item| {
            item["role"] == "user"
                && item["content"].as_array().is_some_and(|content| {
                    content.iter().any(|part| {
                        part["type"] == "input_text"
                            && part["text"]
                                .as_str()
                                .is_some_and(|text| text.contains(SUMMARY))
                    })
                })
        }),
        "the upstream Responses history must contain the original summary as user/input_text: {body}"
    );
}

#[tokio::test]
async fn persisted_local_summary_can_continue_chat() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(chat_reply()))
        .expect(1)
        .mount(&server)
        .await;

    let result = open_responses_proxy_request_with_settings(
        &replay_request(false),
        settings(server.uri(), RelayProtocol::ChatCompletions, "replay-chat"),
    )
    .await
    .unwrap();

    assert_eq!(result.status_code, 200);
    assert!(!result.compaction);
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_chat_summary(&serde_json::from_slice(&requests[0].body).unwrap());
}

#[tokio::test]
async fn persisted_local_summary_can_be_compacted_again_in_chat() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(chat_reply()))
        .expect(1)
        .mount(&server)
        .await;

    let result = open_responses_proxy_request_with_settings(
        &replay_request(true),
        settings(
            server.uri(),
            RelayProtocol::ChatCompletions,
            "replay-chat-compact",
        ),
    )
    .await
    .unwrap();

    assert_eq!(result.status_code, 200);
    assert!(result.compaction);
    assert!(!result.native_compaction_passthrough);
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_chat_summary(&serde_json::from_slice(&requests[0].body).unwrap());
}

#[tokio::test]
async fn persisted_local_summary_is_plain_text_when_continuing_responses() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(responses_reply()))
        .expect(1)
        .mount(&server)
        .await;

    let result = open_responses_proxy_request_with_settings(
        &replay_request(false),
        settings(server.uri(), RelayProtocol::Responses, "replay-responses"),
    )
    .await
    .unwrap();

    assert_eq!(result.status_code, 200);
    assert!(!result.compaction);
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_responses_summary(&serde_json::from_slice(&requests[0].body).unwrap());
}

#[tokio::test]
async fn persisted_local_summary_survives_responses_compaction_fallback() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(has_trigger)
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": {
                "type": "invalid_request_error",
                "message": "Unsupported input type: compaction_trigger",
                "param": "input",
            },
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(|request: &Request| !has_trigger(request))
        .respond_with(ResponseTemplate::new(200).set_body_json(responses_reply()))
        .expect(1)
        .mount(&server)
        .await;

    let result = open_responses_proxy_request_with_settings(
        &replay_request(true),
        settings(
            server.uri(),
            RelayProtocol::Responses,
            "replay-responses-compact",
        ),
    )
    .await
    .unwrap();

    assert_eq!(result.status_code, 200);
    assert!(result.compaction);
    assert!(!result.native_compaction_passthrough);
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    assert!(has_trigger(&requests[0]));
    assert!(!has_trigger(&requests[1]));
    for request in &requests {
        assert_responses_summary(&serde_json::from_slice(&request.body).unwrap());
    }
}

#[tokio::test]
async fn local_summary_does_not_make_opaque_history_safe_for_chat() {
    let server = MockServer::start().await;
    let config = settings(
        server.uri(),
        RelayProtocol::ChatCompletions,
        "replay-mixed-native-history",
    );
    // 普通续聊和再次压缩都必须保护同一历史中的原生 opaque checkpoint。
    for compact in [false, true] {
        let mut request: Value = serde_json::from_str(&replay_request(compact)).unwrap();
        request["input"].as_array_mut().unwrap().insert(
            1,
            json!({"type": "compaction", "encrypted_content": "opaque-native-checkpoint"}),
        );

        let result =
            open_responses_proxy_request_with_settings(&request.to_string(), config.clone())
                .await
                .unwrap();

        assert_eq!(result.status_code, 400);
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}
