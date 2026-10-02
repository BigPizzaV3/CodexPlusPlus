use codex_plus_core::protocol_proxy::{
    open_responses_proxy_request_with_settings, responses_to_chat_completions,
    wrap_non_stream_response_as_compaction,
};
use codex_plus_core::settings::{BackendSettings, RelayMode, RelayProfile, RelayProtocol};
use serde_json::{Value, json};
use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::method};

async fn probe_response(
    status: u16,
    body: &str,
    should_fallback: bool,
    stream: bool,
) -> (bool, Vec<u8>) {
    let server = MockServer::start().await;
    let error_body = body.to_string();
    Mock::given(method("POST"))
        .respond_with(move |request: &Request| {
            let request: Value = serde_json::from_slice(&request.body).unwrap();
            let has_trigger = request["input"].as_array().unwrap().iter()
                .any(|item| item["type"] == "compaction_trigger");
            if has_trigger {
                ResponseTemplate::new(status).set_body_string(error_body.clone())
            } else {
                ResponseTemplate::new(200).set_body_json(json!({
                    "output": [{"type": "message", "content": [{"type": "output_text", "text": "Summary"}]}]
                }))
            }
        })
        .mount(&server)
        .await;
    let config = BackendSettings {
        active_relay_id: "rejection-probe".to_string(),
        relay_profiles: vec![RelayProfile {
            id: "rejection-probe".to_string(),
            base_url: server.uri(),
            api_key: "test-key".to_string(),
            protocol: RelayProtocol::Responses,
            relay_mode: RelayMode::Official,
            official_mix_api_key: true,
            ..RelayProfile::default()
        }],
        ..BackendSettings::default()
    };
    let mut response = open_responses_proxy_request_with_settings(
        &json!({"model":"model", "stream":stream, "input":[
            {"type":"message", "role":"user", "content":"history"},
            {"type":"compaction_trigger"}
        ]})
        .to_string(),
        config,
    )
    .await
    .unwrap();
    assert_eq!(
        response.status_code,
        if should_fallback { 200 } else { status },
        "{body}"
    );
    assert_eq!(response.compaction, should_fallback, "{body}");
    assert_eq!(
        server.received_requests().await.unwrap().len(),
        if should_fallback { 2 } else { 1 },
        "{body}"
    );
    (response.is_stream, response.read_body().await.unwrap())
}

#[tokio::test]
async fn only_explicit_trigger_rejections_allow_summary_fallback() {
    for (status, body) in [
        (
            400,
            r#"{"error":{"message":"Unsupported input type: compaction_trigger"}}"#,
        ),
        (
            422,
            r#"{"error":{"message":"Unknown field","param":"compaction_trigger"}}"#,
        ),
        (400, "compaction_trigger is not supported"),
    ] {
        probe_response(status, body, true, false).await;
    }
}

#[tokio::test]
async fn unrelated_validation_and_operational_errors_do_not_fallback() {
    for (status, body) in [
        (400, r#"{"error":{"message":"invalid input type"}}"#),
        (
            422,
            r#"{"error":{"message":"unsupported parameter: temperature"}}"#,
        ),
        (
            400,
            r#"{"error":{"message":"compaction_trigger requires nonempty input"}}"#,
        ),
        (
            400,
            r#"{"error":{"message":"unsupported parameter: temperature"},"request":{"input":[{"type":"compaction_trigger"}]}}"#,
        ),
        (
            401,
            r#"{"error":{"message":"unsupported compaction_trigger"}}"#,
        ),
        (
            403,
            r#"{"error":{"message":"unsupported compaction_trigger"}}"#,
        ),
        (
            429,
            r#"{"error":{"message":"unsupported compaction_trigger"}}"#,
        ),
        (
            500,
            r#"{"error":{"message":"unsupported compaction_trigger"}}"#,
        ),
        (
            503,
            r#"{"error":{"message":"unsupported compaction_trigger"}}"#,
        ),
    ] {
        probe_response(status, body, false, false).await;
    }
}

#[tokio::test]
async fn json_native_compaction_requires_one_usable_checkpoint() {
    for item in [
        json!({"type": "compaction"}),
        json!({"type": "compaction", "encrypted_content": null}),
        json!({"type": "compaction", "encrypted_content": 123}),
        json!({"type": "compaction", "encrypted_content": ""}),
    ] {
        let body = json!({"id":"resp_native", "status":"completed", "output":[item]}).to_string();
        probe_response(200, &body, true, false).await;
    }
}

#[tokio::test]
async fn streaming_request_with_json_summary_uses_json_wrapper() {
    let (is_stream, body) = probe_response(
        400,
        r#"{"error":{"message":"unsupported compaction_trigger"}}"#,
        true,
        true,
    )
    .await;
    assert!(
        !is_stream,
        "the launcher must select the JSON summary wrapper"
    );
    let sse = wrap_non_stream_response_as_compaction(&body, "model").unwrap();
    let events: Vec<Value> = std::str::from_utf8(&sse)
        .unwrap()
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter_map(|data| serde_json::from_str(data).ok())
        .collect();
    assert!(
        events
            .iter()
            .any(|event| event["type"] == "response.completed")
    );
    let item = &events
        .iter()
        .find(|event| event["type"] == "response.output_item.done")
        .unwrap()["item"];
    let replayed = responses_to_chat_completions(json!({"input":[item]})).unwrap();
    assert!(
        replayed["messages"][0]["content"]
            .as_str()
            .unwrap()
            .ends_with("Summary")
    );
}
