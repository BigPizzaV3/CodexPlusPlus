use codex_plus_core::protocol_proxy::open_responses_proxy_request_with_settings;
use codex_plus_core::settings::{BackendSettings, RelayMode, RelayProfile, RelayProtocol};
use serde_json::{Value, json};
use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::method};

async fn probe_rejection(status: u16, body: &str, should_fallback: bool) {
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
    let response = open_responses_proxy_request_with_settings(
        &json!({"model":"model", "stream":false, "input":[
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
        probe_rejection(status, body, true).await;
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
        probe_rejection(status, body, false).await;
    }
}
