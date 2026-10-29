use codex_plus_core::protocol_proxy::{
    CompactionSseConverter, chat_completion_to_response_with_request,
    chat_sse_to_responses_sse_with_request, open_responses_proxy_request_with_settings,
    open_responses_proxy_request_with_settings_for_path, responses_to_chat_completions,
    wrap_non_stream_response_as_compaction,
};
use codex_plus_core::settings::{BackendSettings, RelayMode, RelayProfile, RelayProtocol};
use serde_json::{Value, json};
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

fn settings(protocol: RelayProtocol, url: &str) -> BackendSettings {
    BackendSettings {
        relay_profiles_enabled: true,
        active_relay_id: "fixture".into(),
        relay_profiles: vec![RelayProfile {
            id: "fixture".into(),
            base_url: format!("{url}/v1"),
            api_key: "fixture-only".into(),
            protocol,
            relay_mode: RelayMode::PureApi,
            ..RelayProfile::default()
        }],
        ..BackendSettings::default()
    }
}

fn events(bytes: &[u8]) -> Vec<Value> {
    std::str::from_utf8(bytes)
        .unwrap()
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter(|data| *data != "[DONE]")
        .map(|data| serde_json::from_str(data).unwrap())
        .collect()
}

fn assert_failed(bytes: &[u8]) {
    let values = events(bytes);
    assert!(
        values.iter().any(|v| v["type"] == "response.failed"),
        "{values:?}"
    );
    assert!(
        !values
            .iter()
            .any(|v| v["type"] == "response.completed" || v["type"] == "response.output_item.done")
    );
}

#[tokio::test]
async fn unsupported_files_never_send_a_partial_chat_or_anthropic_request() {
    for protocol in [
        RelayProtocol::ChatCompletions,
        RelayProtocol::AnthropicMessages,
    ] {
        for file in [
            json!({"type":"input_file","file_data":"private-fixture"}),
            json!({"type":"input_file","file_id":"file-fixture","filename":"private.txt"}),
        ] {
            let upstream = MockServer::start().await;
            let error = open_responses_proxy_request_with_settings(
                &json!({"model":"fixture","input":[
                    {"role":"user","content":[{"type":"input_text","text":"read"},file]}
                ]})
                .to_string(),
                settings(protocol, &upstream.uri()),
            )
            .await
            .err()
            .unwrap();
            assert!(error.to_string().contains("input_file"));
            assert!(!error.to_string().contains("private-fixture"));
            assert!(upstream.received_requests().await.unwrap().is_empty());
        }
    }
    let request = json!({"input":[{"type":"function_call","name":"inspect","call_id":"c",
        "arguments":"{\"type\":\"input_file\"}"},{"type":"function_call_output","call_id":"c","output":"input_file"}]});
    assert!(
        responses_to_chat_completions(request).is_ok(),
        "工具的任意字符串不是附件协议位置"
    );
}

#[tokio::test]
async fn vlm_failure_or_missing_configuration_never_contacts_primary() {
    for configured in [false, true] {
        let primary = MockServer::start().await;
        let vlm = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&vlm)
            .await;
        let mut configured_settings = settings(RelayProtocol::ChatCompletions, &primary.uri());
        let relay = &mut configured_settings.relay_profiles[0];
        relay.model_vlm = json!({"fixture":"vlm"}).to_string();
        if configured {
            relay.vlm_base_url = vlm.uri();
            relay.vlm_api_key = "fixture".into();
            relay.vlm_model = "vision".into();
        }
        let error = open_responses_proxy_request_with_settings(&json!({"model":"fixture","input":[
            {"role":"user","content":[{"type":"input_image","image_url":format!("https://fixture.invalid/{}.png",uuid::Uuid::new_v4())}]}
        ]}).to_string(), configured_settings).await.err().unwrap();
        assert!(
            error
                .downcast_ref::<codex_plus_core::vision::VlmPreprocessError>()
                .is_some()
        );
        assert!(primary.received_requests().await.unwrap().is_empty());
        assert_eq!(
            vlm.received_requests().await.unwrap().is_empty(),
            !configured
        );
    }
}

#[tokio::test]
async fn vlm_success_sends_description_without_images() {
    let primary = MockServer::start().await;
    let vlm = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"choices":[{"message":{"content":"fixture picture"}}]})),
        )
        .mount(&vlm)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"choices":[{"message":{"content":"ok"},"finish_reason":"stop"}]}),
        ))
        .mount(&primary)
        .await;
    let mut configured = settings(RelayProtocol::ChatCompletions, &primary.uri());
    let relay = &mut configured.relay_profiles[0];
    relay.model_vlm = json!({"fixture":"vlm"}).to_string();
    relay.vlm_base_url = vlm.uri();
    relay.vlm_api_key = "fixture".into();
    relay.vlm_model = "vision".into();
    open_responses_proxy_request_with_settings(&json!({"model":"fixture","input":[{"role":"user","content":[
        {"type":"input_image","image_url":format!("https://fixture.invalid/{}.png",uuid::Uuid::new_v4())}]}]}).to_string(),configured).await.unwrap();
    let sent = primary.received_requests().await.unwrap();
    assert_eq!(sent.len(), 1);
    let body: Value = serde_json::from_slice(&sent[0].body).unwrap();
    assert!(body.to_string().contains("fixture picture"));
    assert!(!body.to_string().contains("image_url"));
}

#[tokio::test]
async fn legacy_compact_json_requires_one_valid_checkpoint_without_retry() {
    for (body, expected) in [
        (
            json!({"object":"response.compaction","output":[{"type":"compaction","encrypted_content":"opaque"}]}),
            200,
        ),
        (
            json!({"output":[{"type":"message","content":[{"type":"output_text","text":"not checkpoint"}]}]}),
            502,
        ),
        (json!({"output":[]}), 502),
        (
            json!({"output":[{"type":"compaction","encrypted_content":" "}]}),
            502,
        ),
        (
            json!({"output":[{"type":"compaction","encrypted_content":"a"},{"type":"compaction","encrypted_content":"b"}]}),
            502,
        ),
        (
            json!({"status":"in_progress","output":[{"type":"compaction","encrypted_content":"opaque"}]}),
            502,
        ),
        (
            json!({"status":"failed","error":{"message":"fixture failure"}}),
            200,
        ),
    ] {
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&upstream)
            .await;
        let mut result = open_responses_proxy_request_with_settings_for_path(
            &json!({"model":"fixture","stream":false,"input":"compact"}).to_string(),
            settings(RelayProtocol::Responses, &upstream.uri()),
            "/v1/responses/compact",
        )
        .await
        .unwrap();
        assert_eq!(result.status_code, expected);
        let payload = result.read_body().await.unwrap();
        if expected == 502 {
            assert!(String::from_utf8_lossy(&payload).contains("compaction_invalid_response"));
        }
        let sent = upstream.received_requests().await.unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].url.path(), "/v1/responses/compact");
    }
}

#[tokio::test]
async fn legacy_compact_sse_requires_complete_checkpoint_stream() {
    let item = "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"compaction\",\"encrypted_content\":\"opaque\"}}\n\n";
    let done = "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"fixture\",\"status\":\"completed\"}}\n\n";
    for (body, expected) in [
        (format!("{item}{done}"), 200),
        (done.to_string(), 502),
        (format!("{done}{item}"), 502),
        (format!("{item}{item}{done}"), 502),
        (item.to_string(), 502),
        (format!("{item}{done}data: {{"), 502),
        (format!("{item}{done}data: invalid\n\n"), 502),
        (
            format!(
                "{item}{done}data: {{\"type\":\"response.failed\",\"response\":{{\"status\":\"failed\"}}}}\n\n"
            ),
            200,
        ),
    ] {
        let failed = body.contains("response.failed");
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Content-Type", "text/event-stream")
                    .set_body_string(body),
            )
            .mount(&upstream)
            .await;
        let mut result = open_responses_proxy_request_with_settings_for_path(
            &json!({"model":"fixture","stream":true,"input":"compact"}).to_string(),
            settings(RelayProtocol::Responses, &upstream.uri()),
            "/responses/compact",
        )
        .await
        .unwrap();
        assert_eq!(result.status_code, expected);
        if failed {
            assert_failed(&result.read_body().await.unwrap());
        }
        assert_eq!(upstream.received_requests().await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn native_v2_incomplete_checkpoint_is_not_delivered_before_failure() {
    let upstream = MockServer::start().await;
    Mock::given(method("POST")).respond_with(ResponseTemplate::new(200).insert_header("Content-Type","text/event-stream")
        .set_body_string("data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"compaction\",\"encrypted_content\":\"not-completed\"}}\n\n"))
        .mount(&upstream).await;
    let mut result = open_responses_proxy_request_with_settings(
        &json!({"model":"fixture","stream":true,"input":[{"type":"compaction_trigger"}]})
            .to_string(),
        settings(RelayProtocol::Responses, &upstream.uri()),
    )
    .await
    .unwrap();
    assert_failed(&result.read_body().await.unwrap());
    assert_eq!(upstream.received_requests().await.unwrap().len(), 1);
}

#[test]
fn synthetic_compaction_requires_wire_completion_and_complete_events() {
    for chat in [false, true] {
        let delta = if chat {
            "data: {\"choices\":[{\"delta\":{\"content\":\"摘要\"}}]}\n\n"
        } else {
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"摘要\"}\n\n"
        };
        for suffix in [
            "",
            "data: [DONE]\n\n",
            "data: {",
            "data: {\"error\":{\"message\":\"failed\"}}\n\n",
        ] {
            let mut converter = CompactionSseConverter::new("fixture");
            if chat {
                converter = converter.with_chat_upstream();
            }
            converter.push_upstream_bytes(format!("{delta}{suffix}").as_bytes());
            assert_failed(&converter.finish());
        }
        let terminal = if chat {
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n"
        } else {
            "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\"}}\n\n"
        };
        let mut converter = CompactionSseConverter::new("fixture");
        if chat {
            converter = converter.with_chat_upstream();
        }
        for chunk in format!("{delta}{terminal}").as_bytes().chunks(1) {
            converter.push_upstream_bytes(chunk);
        }
        let result = converter.finish_with_outcome();
        assert!(result.succeeded);
        assert_eq!(
            events(&result.payload)
                .iter()
                .filter(|v| v["type"] == "response.output_item.done")
                .count(),
            1
        );
        let mut converter = CompactionSseConverter::new("fixture");
        if chat {
            converter = converter.with_chat_upstream();
        }
        converter.push_upstream_bytes(
            format!("{delta}{terminal}data: {{\"error\":{{\"message\":\"late failure\"}}}}\n\n")
                .as_bytes(),
        );
        assert_failed(&converter.finish());
    }
    for body in [
        json!({"output":[{"type":"message","content":[{"type":"output_text","text":"partial"}]}]}),
        json!({"choices":[{"message":{"content":"partial"},"finish_reason":"length"}]}),
        json!({"choices":[{"message":{"content":"partial"}}]}),
    ] {
        assert_failed(
            &wrap_non_stream_response_as_compaction(body.to_string().as_bytes(), "fixture")
                .unwrap(),
        );
    }
    for body in [
        json!({"status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"summary"}]}]}),
        json!({"choices":[{"message":{"content":"summary"},"finish_reason":"stop"}]}),
    ] {
        assert!(
            events(
                &wrap_non_stream_response_as_compaction(body.to_string().as_bytes(), "fixture")
                    .unwrap()
            )
            .iter()
            .any(|v| v["type"] == "response.completed")
        );
    }
}

#[test]
fn namespace_projection_merges_children_and_uses_current_definition() {
    let child = |name: &str, description: &str| json!({"type":"function","name":name,"description":description,"parameters":{"type":"object"}});
    let request = json!({"model":"fixture","tools":[{"type":"namespace","name":"repo","tools":[child("search","current")]}],"input":[
        {"type":"tool_search_output","call_id":"old","tools":[{"type":"namespace","name":"repo","tools":[child("search","old")]}]},
        {"type":"tool_search_output","call_id":"new","tools":[{"type":"namespace","name":"repo","tools":[child("search","recent"),child("new_tool","new")]}]}]});
    let sent = responses_to_chat_completions(request.clone()).unwrap();
    let tools = sent["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 2);
    assert!(
        tools
            .iter()
            .any(|v| v["function"]["name"] == "repo__new_tool"
                || v["function"]["name"] == "mcp__repo__new_tool")
    );
    assert!(tools.iter().any(|v| {
        v["function"]["description"]
            .as_str()
            .unwrap_or("")
            .contains("current")
    }));
    let name = tools
        .iter()
        .find(|v| {
            v["function"]["name"]
                .as_str()
                .unwrap()
                .ends_with("new_tool")
        })
        .unwrap()["function"]["name"]
        .as_str()
        .unwrap();
    let response=chat_completion_to_response_with_request(json!({"choices":[{"message":{"tool_calls":[{"id":"call_new","type":"function","function":{"name":name,"arguments":"{}"}}]},"finish_reason":"tool_calls"}]}),&request).unwrap();
    assert_eq!(response["output"][0]["namespace"], "repo");
    assert_eq!(response["output"][0]["name"], "new_tool");
    let mut distinct = request.clone();
    distinct["tools"].as_array_mut().unwrap().push(
        json!({"type":"namespace","name":"another","tools":[child("search","other namespace")]}),
    );
    let projected = responses_to_chat_completions(distinct).unwrap();
    assert_eq!(projected["tools"].as_array().unwrap().len(), 3);
}

#[test]
fn same_named_client_search_survives_json_and_sse_round_trip() {
    let request = json!({"tools":[{"type":"web_search"},{"type":"function","name":"web_search","parameters":{"type":"object"}}],"input":"search"});
    let projected = responses_to_chat_completions(request.clone()).unwrap();
    assert_eq!(projected["tools"].as_array().unwrap().len(), 1);
    let response = json!({"choices":[{"message":{"tool_calls":[{"id":"search_call","type":"function","function":{"name":"web_search","arguments":"{}"}}]},"finish_reason":"tool_calls"}]});
    let restored = chat_completion_to_response_with_request(response, &request).unwrap();
    assert_eq!(restored["output"][0]["type"], "function_call");
    assert_eq!(restored["output"][0]["call_id"], "search_call");
    let sse = "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"search_call\",\"function\":{\"name\":\"web_search\",\"arguments\":\"{}\"}}]}}]}\n\ndata: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\ndata: [DONE]\n\n";
    let restored = chat_sse_to_responses_sse_with_request(sse, &request);
    let values = events(restored.as_bytes());
    assert!(
        values
            .iter()
            .any(|v| v["type"] == "response.output_item.done"
                && v["item"]["type"] == "function_call")
    );
    assert!(!restored.contains("web_search_call"));
}
