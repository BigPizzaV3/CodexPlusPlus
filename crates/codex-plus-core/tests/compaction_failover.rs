use codex_plus_core::protocol_proxy::open_responses_proxy_request_with_settings;
use codex_plus_core::settings::{
    AggregateRelayMember, AggregateRelayProfile, AggregateRelayStrategy, BackendSettings,
    RelayMode, RelayProfile, RelayProtocol, RelaySessionProvider,
};
use serde_json::{Value, json};
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

fn aggregate_settings(
    first: &MockServer,
    first_protocol: RelayProtocol,
    second: &MockServer,
    second_protocol: RelayProtocol,
) -> BackendSettings {
    let id = format!("compaction-failover-{}", uuid::Uuid::new_v4().simple());
    let first_id = format!("{id}-first");
    let second_id = format!("{id}-second");
    let profiles = [
        (first, first_protocol, &first_id),
        (second, second_protocol, &second_id),
    ]
    .into_iter()
    .map(|(server, protocol, member_id)| RelayProfile {
        id: member_id.clone(),
        name: member_id.clone(),
        base_url: server.uri(),
        api_key: "compaction-failover-test-key".to_string(),
        protocol,
        relay_mode: RelayMode::Official,
        official_mix_api_key: true,
        ..RelayProfile::default()
    })
    .chain(std::iter::once(RelayProfile {
        id: id.clone(),
        name: id.clone(),
        relay_mode: RelayMode::Aggregate,
        ..RelayProfile::default()
    }))
    .collect();
    BackendSettings {
        relay_profiles: profiles,
        active_relay_id: id.clone(),
        active_aggregate_relay_id: id.clone(),
        aggregate_relay_profiles: vec![AggregateRelayProfile {
            id: id.clone(),
            name: id,
            session_provider: RelaySessionProvider::Custom,
            strategy: AggregateRelayStrategy::RequestRoundRobin,
            members: vec![
                AggregateRelayMember {
                    relay_id: first_id,
                    weight: 1,
                },
                AggregateRelayMember {
                    relay_id: second_id,
                    weight: 1,
                },
            ],
            routes: Vec::new(),
        }],
        ..BackendSettings::default()
    }
}

fn native_history_request(compact: bool) -> Value {
    let mut input = vec![
        json!({
            "id": "cmp_native_history",
            "type": "compaction",
            "encrypted_content": "opaque-state+/=\n不能作为明文摘要",
            "future_state": {"version": 7, "entries": ["keep", 42]},
        }),
        json!({
            "type": "message",
            "role": "user",
            "content": [{"type": "input_text", "text": "Continue the project."}],
        }),
    ];
    if compact {
        input.push(json!({"type": "compaction_trigger"}));
    }
    json!({"model": "test-model", "stream": false, "input": input})
}

fn native_compaction_reply() -> Value {
    json!({
        "id": "resp_native_result",
        "object": "response",
        "status": "completed",
        "output": [{
            "id": "cmp_native_result",
            "type": "compaction",
            "encrypted_content": "opaque-next-state",
            "future_state": {"preserved": true},
        }],
    })
}

async fn reject_trigger(server: &MockServer) {
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": {"message": "Unsupported input type: compaction_trigger"},
        })))
        .expect(1)
        .mount(server)
        .await;
}

async fn assert_native_request_preserved(server: &MockServer, expected: &Value) {
    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests.len(),
        1,
        "an incompatible native history must not be retried as a summary"
    );
    assert_eq!(requests[0].url.path(), "/v1/responses");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(
        body["input"], expected["input"],
        "native checkpoint fields and trigger must survive failover"
    );
    assert!(!String::from_utf8_lossy(&requests[0].body).contains("CONTEXT CHECKPOINT COMPACTION"));
}

#[tokio::test]
async fn native_history_skips_chat_member_and_continues_on_responses() {
    let chat = MockServer::start().await;
    let responses = MockServer::start().await;
    let reply = json!({
        "id": "resp_continued",
        "object": "response",
        "status": "completed",
        "output": [{
            "type": "message", "role": "assistant",
            "content": [{"type": "output_text", "text": "Continuing the project."}],
        }],
    });
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(reply.clone()))
        .expect(1)
        .mount(&responses)
        .await;
    let request = native_history_request(false);

    let mut result = open_responses_proxy_request_with_settings(
        &request.to_string(),
        aggregate_settings(
            &chat,
            RelayProtocol::ChatCompletions,
            &responses,
            RelayProtocol::Responses,
        ),
    )
    .await
    .unwrap();

    assert_eq!(result.status_code, 200);
    assert!(!result.compaction);
    assert_eq!(
        serde_json::from_slice::<Value>(&result.read_body().await.unwrap()).unwrap(),
        reply
    );
    assert!(chat.received_requests().await.unwrap().is_empty());
    assert_native_request_preserved(&responses, &request).await;
}

#[tokio::test]
async fn rejected_native_compaction_tries_next_responses_member_without_summary() {
    let first = MockServer::start().await;
    let second = MockServer::start().await;
    reject_trigger(&first).await;
    let reply = native_compaction_reply();
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(reply.clone()))
        .expect(1)
        .mount(&second)
        .await;
    let request = native_history_request(true);

    let mut result = open_responses_proxy_request_with_settings(
        &request.to_string(),
        aggregate_settings(
            &first,
            RelayProtocol::Responses,
            &second,
            RelayProtocol::Responses,
        ),
    )
    .await
    .unwrap();

    assert_eq!(result.status_code, 200);
    assert!(result.native_compaction_passthrough);
    assert!(!result.compaction);
    assert_eq!(
        serde_json::from_slice::<Value>(&result.read_body().await.unwrap()).unwrap(),
        reply
    );
    assert_native_request_preserved(&first, &request).await;
    assert_native_request_preserved(&second, &request).await;
}

#[tokio::test]
async fn exhausted_incompatible_members_return_error_without_plain_text_fallback() {
    let first = MockServer::start().await;
    let second = MockServer::start().await;
    reject_trigger(&first).await;
    reject_trigger(&second).await;
    let request = native_history_request(true);

    let mut result = open_responses_proxy_request_with_settings(
        &request.to_string(),
        aggregate_settings(
            &first,
            RelayProtocol::Responses,
            &second,
            RelayProtocol::Responses,
        ),
    )
    .await
    .unwrap();

    assert_eq!(result.status_code, 400);
    assert!(!result.compaction);
    assert!(!result.native_compaction_passthrough);
    let error: Value = serde_json::from_slice(&result.read_body().await.unwrap()).unwrap();
    assert_eq!(error["error"]["code"], "native_compaction_unsupported");
    assert_native_request_preserved(&first, &request).await;
    assert_native_request_preserved(&second, &request).await;
}
