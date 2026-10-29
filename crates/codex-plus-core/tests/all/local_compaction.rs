use codex_plus_core::protocol_proxy::{
    ProxySessionHeaders, UpstreamProxyResponse,
    open_responses_proxy_request_with_settings_for_path_and_session_headers,
};
use codex_plus_core::settings::{
    BackendSettings, RelayMode, RelayModelRoute, RelayProfile, RelayProtocol,
};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const PROMPT: &str = "You are performing a CONTEXT CHECKPOINT COMPACTION. Create a handoff summary for another LLM that will resume the task.\n\nInclude:\n- Current progress and key decisions made\n- Important context, constraints, or user preferences\n- What remains to be done (clear next steps)\n- Any critical data, examples, or references needed to continue\n\nBe concise, structured, and focused on helping the next LLM seamlessly continue the work.";

fn normal(stream: bool) -> Value {
    json!({
        "model":"fixture", "stream":stream, "store":false,
        "instructions":"Keep the current system instructions.",
        "prompt_cache_key":"fixture-cache",
        "tools":[{"type":"function","name":"lookup","parameters":{"type":"object"}}],
        "tool_choice":"required", "parallel_tool_calls":true,
        "input":[{"type":"message","role":"user","content":"Current task"}]
    })
}

fn compact(normal: &Value) -> Value {
    let mut body = normal.clone();
    body["tools"] = json!([]);
    body["tool_choice"] = json!("none");
    body["parallel_tool_calls"] = json!(false);
    body["input"].as_array_mut().unwrap().extend([
        json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"Progress."}]}),
        json!({"type":"message","role":"user","content":[{"type":"input_text","text":PROMPT}]}),
    ]);
    body
}

fn settings(url: String, protocol: RelayProtocol, enabled: bool) -> BackendSettings {
    BackendSettings {
        // HTTP 失败用例只检查实验本身不重发；自动重试另有显式开启的交互用例。
        protocol_proxy_auto_retry: false,
        active_relay_id: "fixture".into(),
        relay_profiles: vec![RelayProfile {
            id: "fixture".into(),
            relay_mode: RelayMode::PureApi,
            base_url: url,
            api_key: "fixture-key".into(),
            protocol,
            local_compaction_prompt_at_end: enabled,
            ..RelayProfile::default()
        }],
        ..BackendSettings::default()
    }
}

fn success(protocol: RelayProtocol, streaming: bool) -> (u16, &'static str, String) {
    let response = if protocol == RelayProtocol::ChatCompletions {
        json!({"choices":[{"finish_reason":"stop","message":{"role":"assistant","content":"Summary."}}]})
    } else {
        json!({"id":"resp_summary","status":"completed","usage":{"input_tokens":1200,"output_tokens":3,"total_tokens":1203},
            "output":[{"type":"message","id":"msg_summary","role":"assistant",
                "content":[{"type":"output_text","text":"Summary."}]}]})
    };
    if streaming && protocol == RelayProtocol::Responses {
        // 供应商事件保持原样，由既有响应路径处理。
        (
            200,
            "text/event-stream",
            format!(
                "data: {}\n\n",
                json!({"type":"response.completed","response":response})
            ),
        )
    } else if streaming {
        (
            200,
            "text/event-stream",
            format!(
                "data: {}\n\ndata: [DONE]\n\n",
                json!({"choices":[{"delta":{"content":"Summary."},"finish_reason":"stop"}]})
            ),
        )
    } else {
        (200, "application/json", response.to_string())
    }
}

async fn capture(
    listener: tokio::net::TcpListener,
    replies: Vec<(u16, &'static str, String)>,
) -> Vec<Value> {
    capture_requests(&listener, replies).await
}

async fn read_request(stream: &mut tokio::net::TcpStream) -> Value {
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    let (start, length) = loop {
        let read = stream.read(&mut buffer).await.unwrap();
        assert!(read > 0);
        bytes.extend_from_slice(&buffer[..read]);
        let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") else {
            continue;
        };
        let length = String::from_utf8_lossy(&bytes[..end])
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().unwrap())
            })
            .unwrap();
        break (end + 4, length);
    };
    while bytes.len() < start + length {
        let read = stream.read(&mut buffer).await.unwrap();
        assert!(read > 0);
        bytes.extend_from_slice(&buffer[..read]);
    }
    serde_json::from_slice(&bytes[start..start + length]).unwrap()
}

async fn capture_requests(
    listener: &tokio::net::TcpListener,
    replies: Vec<(u16, &'static str, String)>,
) -> Vec<Value> {
    let mut requests = Vec::new();
    for (status, content_type, reply) in replies {
        let (mut stream, _) =
            tokio::time::timeout(std::time::Duration::from_secs(10), listener.accept())
                .await
                .unwrap()
                .unwrap();
        requests.push(read_request(&mut stream).await);
        let headers = format!(
            "HTTP/1.1 {status} Result\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            reply.len(),
        );
        stream.write_all(headers.as_bytes()).await.unwrap();
        stream.write_all(reply.as_bytes()).await.unwrap();
    }
    requests
}

async fn send(
    body: &Value,
    settings: &BackendSettings,
    thread: Option<&str>,
) -> UpstreamProxyResponse {
    open_responses_proxy_request_with_settings_for_path_and_session_headers(
        &body.to_string(),
        settings.clone(),
        "/v1/responses",
        None,
        ProxySessionHeaders {
            thread_id: thread,
            ..ProxySessionHeaders::default()
        },
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn local_compaction_preserves_tools_without_enabling_calls_for_json_and_sse() {
    for protocol in [RelayProtocol::Responses, RelayProtocol::ChatCompletions] {
        for streaming in [false, true] {
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                .await
                .unwrap();
            let settings = settings(
                format!("http://{}/v1", listener.local_addr().unwrap()),
                protocol,
                true,
            );
            let reply = success(protocol, streaming);
            let expected_response = reply.2.clone();
            let server = tokio::spawn(capture(listener, vec![reply.clone(), reply]));
            let normal = normal(streaming);
            send(&normal, &settings, Some("matrix"))
                .await
                .read_body()
                .await
                .unwrap();
            let mut compact = compact(&normal);
            compact["max_output_tokens"] = json!(4096);
            compact["client_metadata"] = json!({"turn":"current"});
            let mut result = send(&compact, &settings, Some("matrix")).await;
            assert!(!result.compaction);
            let response = String::from_utf8(result.read_body().await.unwrap()).unwrap();
            assert_eq!(response, expected_response);
            let requests = server.await.unwrap();
            assert_eq!(requests.len(), 2);
            assert_eq!(requests[1]["tools"], requests[0]["tools"]);
            assert_eq!(requests[1]["tool_choice"], "none");
            assert_eq!(requests[1]["parallel_tool_calls"], false);
            if protocol == RelayProtocol::Responses {
                assert_eq!(requests[1]["input"], compact["input"]);
                assert_eq!(requests[1]["max_output_tokens"], 4096);
                assert_eq!(requests[1]["client_metadata"], compact["client_metadata"]);
            }
        }
    }
}

#[tokio::test]
async fn local_compaction_and_search_history_compatibility_remain_independent() {
    for (local, search) in [(false, false), (false, true), (true, false), (true, true)] {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let mut settings = settings(
            format!("http://{}/v1", listener.local_addr().unwrap()),
            RelayProtocol::Responses,
            local,
        );
        settings.relay_profiles[0].web_search_history_compat = search;
        let before = serde_json::to_value(&settings).unwrap();
        let reply = success(RelayProtocol::Responses, false);
        let server = tokio::spawn(capture(listener, vec![reply.clone(), reply]));
        let mut normal = normal(false);
        normal["input"].as_array_mut().unwrap().push(json!({
            "type":"web_search_call","id":"ws_prior","status":"completed",
            "action":{"type":"search","query":"fixture"}
        }));
        send(&normal, &settings, Some("search-interaction"))
            .await
            .read_body()
            .await
            .unwrap();
        let compact = compact(&normal);
        send(&compact, &settings, Some("search-interaction"))
            .await
            .read_body()
            .await
            .unwrap();
        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 2);
        let mut expected_normal = normal.clone();
        let mut expected_compact = compact.clone();
        if local {
            expected_compact["tools"] = normal["tools"].clone();
        }
        if search {
            let tool = json!({"type":"web_search","external_web_access":false});
            expected_normal["tools"]
                .as_array_mut()
                .unwrap()
                .push(tool.clone());
            expected_compact["tools"].as_array_mut().unwrap().push(tool);
        }
        assert_eq!(
            requests[0], expected_normal,
            "ordinary search repair must not depend on the local compaction switch"
        );
        assert_eq!(
            requests[1], expected_compact,
            "local compaction must retain the existing independent search repair"
        );
        assert_eq!(serde_json::to_value(&settings).unwrap(), before);
    }
}

#[tokio::test]
async fn local_compaction_disabled_missing_session_and_cold_cache_use_original() {
    for case in ["disabled", "session", "cold", "no_auth"] {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let mut settings = settings(
            format!("http://{}/v1", listener.local_addr().unwrap()),
            RelayProtocol::Responses,
            case != "disabled",
        );
        let reply = success(RelayProtocol::Responses, false);
        let server = tokio::spawn(capture(listener, vec![reply.clone(), reply]));
        let normal = normal(false);
        let thread = (case != "session").then_some("isolation");
        send(&normal, &settings, thread)
            .await
            .read_body()
            .await
            .unwrap();
        if case == "no_auth" {
            settings.relay_profiles[0].no_auth = true;
        }
        let compact = compact(&normal);
        let compact_thread = if case == "cold" {
            Some("unseen")
        } else {
            thread
        };
        send(&compact, &settings, compact_thread)
            .await
            .read_body()
            .await
            .unwrap();
        assert_eq!(server.await.unwrap()[1], compact);
    }
}

#[tokio::test]
async fn local_compaction_keeps_current_context_when_history_instructions_and_cache_key_change() {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let settings = settings(
        format!("http://{}/v1", listener.local_addr().unwrap()),
        RelayProtocol::Responses,
        true,
    );
    let reply = success(RelayProtocol::Responses, false);
    let server = tokio::spawn(capture(listener, vec![reply.clone(), reply]));
    let normal = normal(false);
    send(&normal, &settings, Some("changed-context"))
        .await
        .read_body()
        .await
        .unwrap();
    let mut compact = compact(&normal);
    compact["input"].as_array_mut().unwrap().remove(0);
    compact["instructions"] = json!("Current system instructions, not the old ones.");
    compact["prompt_cache_key"] = json!("current-cache");
    compact["previous_response_id"] = json!("current-response");
    compact["client_metadata"] = json!({"current":true});
    send(&compact, &settings, Some("changed-context"))
        .await
        .read_body()
        .await
        .unwrap();
    let requests = server.await.unwrap();
    let mut expected = compact;
    expected["tools"] = requests[0]["tools"].clone();
    assert_eq!(requests[1], expected);
}

#[tokio::test]
async fn local_compaction_fills_missing_envelope_before_responses_or_chat_conversion() {
    for protocol in [RelayProtocol::Responses, RelayProtocol::ChatCompletions] {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let settings = settings(
            format!("http://{}/v1", listener.local_addr().unwrap()),
            protocol,
            true,
        );
        let reply = success(protocol, false);
        let server = tokio::spawn(capture(listener, vec![reply.clone(), reply]));
        let mut normal = normal(false);
        normal["reasoning"] = json!({"effort":"high"});
        normal["text"] =
            json!({"format":{"type":"json_schema","name":"old","schema":{"type":"object"}}});
        send(&normal, &settings, Some("missing-envelope"))
            .await
            .read_body()
            .await
            .unwrap();
        let mut compact = compact(&normal);
        for name in ["instructions", "reasoning", "prompt_cache_key"] {
            compact.as_object_mut().unwrap().remove(name);
        }
        // 当前显式格式优先，旧 JSON schema 不能约束本次摘要。
        compact["text"] = json!({"format":{"type":"text"}});
        compact["max_output_tokens"] = json!(1024);
        send(&compact, &settings, Some("missing-envelope"))
            .await
            .read_body()
            .await
            .unwrap();
        let requests = server.await.unwrap();
        assert_eq!(requests[1]["tools"], requests[0]["tools"]);
        assert_eq!(requests[1]["tool_choice"], "none");
        if protocol == RelayProtocol::Responses {
            assert_eq!(requests[1]["instructions"], normal["instructions"]);
            assert_eq!(requests[1]["reasoning"], normal["reasoning"]);
            assert_eq!(requests[1]["prompt_cache_key"], normal["prompt_cache_key"]);
            assert_eq!(requests[1]["text"], compact["text"]);
            assert_eq!(requests[1]["input"], compact["input"]);
            assert_eq!(requests[1]["max_output_tokens"], 1024);
        } else {
            assert_eq!(requests[1]["messages"][0], requests[0]["messages"][0]);
            assert_eq!(requests[1]["messages"][0]["role"], "system");
            let messages = requests[1]["messages"].as_array().unwrap();
            assert_eq!(messages.last().unwrap()["content"], PROMPT);
            assert!(messages.iter().any(|item| item["content"] == "Progress."));
        }
    }
}

#[tokio::test]
async fn local_compaction_restores_tool_context_before_chat_namespace_history_projection() {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let settings = settings(
        format!("http://{}/v1", listener.local_addr().unwrap()),
        RelayProtocol::ChatCompletions,
        true,
    );
    let reply = success(RelayProtocol::ChatCompletions, false);
    let server = tokio::spawn(capture(listener, vec![reply.clone(), reply]));
    let mut normal = normal(false);
    normal["tool_choice"] = json!("auto");
    normal["tools"] = json!([{
        "type":"namespace", "name":"mcp__fixture__",
        "tools":[{"type":"function","name":"lookup","parameters":{"type":"object"}}]
    }]);
    send(&normal, &settings, Some("namespace"))
        .await
        .read_body()
        .await
        .unwrap();
    let mut compact = compact(&normal);
    let items = compact["input"].as_array_mut().unwrap();
    items.insert(
        1,
        json!({
            "type":"function_call", "call_id":"latest", "namespace":"mcp__fixture__",
            "name":"lookup", "arguments":"{}"
        }),
    );
    items.insert(
        2,
        json!({
            "type":"function_call_output", "call_id":"latest", "output":"Newest result."
        }),
    );
    send(&compact, &settings, Some("namespace"))
        .await
        .read_body()
        .await
        .unwrap();
    let requests = server.await.unwrap();
    assert_eq!(requests[1]["tools"], requests[0]["tools"]);
    let messages = requests[1]["messages"].as_array().unwrap();
    let call = messages
        .iter()
        .find(|item| item.get("tool_calls").is_some())
        .unwrap();
    assert_eq!(
        call["tool_calls"][0]["function"]["name"],
        "mcp__fixture__lookup"
    );
    assert!(
        messages
            .iter()
            .any(|item| item["role"] == "tool" && item["content"] == "Newest result.")
    );
    assert_eq!(messages.last().unwrap()["content"], PROMPT);
}

#[tokio::test]
async fn full_tools_compaction_metadata_preserves_exec_negotiation_when_experiment_is_disabled() {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let settings = settings(
        format!("http://{}/v1", listener.local_addr().unwrap()),
        RelayProtocol::Responses,
        false,
    );
    let rejection = (
        400,
        "application/json",
        json!({"error":{
            "type":"invalid_request_error",
            "message":"Unsupported custom tool: 'exec'. Only 'apply_patch' is supported.",
            "param":"tools[0]"
        }})
        .to_string(),
    );
    let server = tokio::spawn(capture(
        listener,
        vec![rejection, success(RelayProtocol::Responses, false)],
    ));
    let mut current = normal(false);
    current["tools"] = json!([{"type":"custom","name":"exec","description":"Current."}]);
    current["client_metadata"] = json!({"x-codex-turn-metadata":
        json!({"request_kind":"compaction","compaction":{"implementation":"responses"}}).to_string()});
    current["input"].as_array_mut().unwrap().push(json!({
        "type":"message","role":"user",
        "content":[{"type":"input_text","text":PROMPT}]
    }));
    current["tool_choice"] = json!("none");
    current["parallel_tool_calls"] = json!(false);
    send(&current, &settings, Some("disabled-full-tools"))
        .await
        .read_body()
        .await
        .unwrap();
    let requests = server.await.unwrap();
    assert_eq!(
        requests.len(),
        2,
        "default-off retains the existing explicit exec negotiation"
    );
    assert_eq!(requests[0], current);
    assert_eq!(requests[1]["tools"][0]["type"], "function");
    assert_eq!(requests[1]["input"], current["input"]);
}

#[tokio::test]
async fn local_compaction_uses_latest_tools_and_clears_them_after_an_empty_tools_request() {
    for empty in [false, true] {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let settings = settings(
            format!("http://{}/v1", listener.local_addr().unwrap()),
            RelayProtocol::Responses,
            true,
        );
        let reply = success(RelayProtocol::Responses, false);
        let server = tokio::spawn(capture(listener, vec![reply.clone(), reply.clone(), reply]));
        let normal = normal(false);
        send(&normal, &settings, Some("latest-tools"))
            .await
            .read_body()
            .await
            .unwrap();
        let mut latest = normal.clone();
        latest["tools"] = if empty {
            json!([])
        } else {
            json!([{"type":"function","name":"current","parameters":{"type":"object"}}])
        };
        send(&latest, &settings, Some("latest-tools"))
            .await
            .read_body()
            .await
            .unwrap();
        send(&compact(&normal), &settings, Some("latest-tools"))
            .await
            .read_body()
            .await
            .unwrap();
        let requests = server.await.unwrap();
        assert_eq!(requests[2]["tools"], latest["tools"]);
        assert_eq!(requests[2]["input"], compact(&normal)["input"]);
    }
}

#[tokio::test]
async fn local_compaction_inherits_only_entry_or_target_opt_in_and_keeps_target_model() {
    for (entry, target) in [(true, false), (false, true), (true, true), (false, false)] {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let mut settings = settings(
            format!("http://{}/v1", listener.local_addr().unwrap()),
            RelayProtocol::Responses,
            target,
        );
        settings.relay_profiles.insert(
            0,
            RelayProfile {
                id: "entry".into(),
                relay_mode: RelayMode::PureApi,
                api_key: "entry-key".into(),
                base_url: "http://127.0.0.1:9/v1".into(),
                local_compaction_prompt_at_end: entry,
                model_routes: vec![RelayModelRoute {
                    model: "fixture".into(),
                    target_relay_id: "fixture".into(),
                    target_model: "target-model".into(),
                }],
                ..RelayProfile::default()
            },
        );
        settings.active_relay_id = "entry".into();
        let before = serde_json::to_value(&settings).unwrap();
        let reply = success(RelayProtocol::Responses, false);
        let server = tokio::spawn(capture(listener, vec![reply.clone(), reply]));
        let normal = normal(false);
        send(&normal, &settings, Some("route"))
            .await
            .read_body()
            .await
            .unwrap();
        send(&compact(&normal), &settings, Some("route"))
            .await
            .read_body()
            .await
            .unwrap();
        let requests = server.await.unwrap();
        assert_eq!(requests[1]["model"], "target-model");
        assert_eq!(
            requests[1]["tools"].as_array().unwrap().is_empty(),
            !(entry || target)
        );
        assert_eq!(requests[1]["tool_choice"], "none");
        assert_eq!(
            serde_json::to_value(&settings).unwrap(),
            before,
            "entry opt-in applies only to the request's target clone"
        );
    }
}

#[tokio::test]
async fn local_compaction_does_not_validate_or_retry_summary_responses() {
    for (status, content_type, body) in [
        (200, "application/json", json!({"id":"resp_variant","status":"completed",
            "output":[{"type":"function_call","name":"lookup"}]}).to_string()),
        (200, "text/event-stream",
            "event: provider.extension\ndata: {\"type\":\"provider.extension\",\"value\":1}\n\ndata: [DONE]\n\n".into()),
        (400, "application/json", json!({"error":{"message":"bad request"}}).to_string()),
        (422, "application/json", json!({"error":{"message":"unprocessable"}}).to_string()),
        (429, "application/json", json!({"error":{"message":"rate limit"}}).to_string()),
        (500, "application/json", json!({"error":{"message":"server error"}}).to_string()),
    ] {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let settings = settings(
            format!("http://{}/v1", listener.local_addr().unwrap()),
            RelayProtocol::Responses,
            true,
        );
        let reply = success(RelayProtocol::Responses, false);
        let server = tokio::spawn(capture(listener, vec![reply, (status, content_type, body.clone())]));
        let normal = normal(false);
        send(&normal, &settings, Some("single-send"))
            .await
            .read_body()
            .await
            .unwrap();
        let compact = compact(&normal);
        let mut result = send(&compact, &settings, Some("single-send")).await;
        assert_eq!(result.status_code, status);
        assert_eq!(result.read_body().await.unwrap().as_slice(), body.as_bytes());
        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1]["tools"], normal["tools"]);
        assert_eq!(requests[1]["tool_choice"], "none");
    }
}

#[tokio::test]
async fn local_compaction_forwards_tool_calls_from_upstreams_ignoring_none_without_retry() {
    let tool_call = json!({
        "type":"function_call", "id":"fc_ignored_none", "call_id":"call_ignored_none",
        "name":"lookup", "arguments":"{}", "status":"completed"
    });
    let response = json!({
        "id":"resp_ignored_none", "object":"response", "status":"completed",
        "output":[tool_call.clone()]
    });
    for mode in ["streaming", "buffered"] {
        for streaming in [false, true] {
            let body = if streaming {
                format!(
                    "event: response.output_item.done\ndata: {}\n\nevent: response.completed\ndata: {}\n\ndata: [DONE]\n\n",
                    json!({"type":"response.output_item.done","output_index":0,"item":tool_call}),
                    json!({"type":"response.completed","response":response}),
                )
            } else {
                response.to_string()
            };
            let content_type = if streaming {
                "text/event-stream"
            } else {
                "application/json"
            };
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                .await
                .unwrap();
            let mut settings = settings(
                format!("http://{}/v1", listener.local_addr().unwrap()),
                RelayProtocol::Responses,
                true,
            );
            settings.protocol_proxy_auto_retry = true;
            settings.protocol_proxy_auto_retry_mode = mode.into();
            let server = tokio::spawn(capture(
                listener,
                vec![
                    success(RelayProtocol::Responses, false),
                    (200, content_type, body.clone()),
                ],
            ));
            let normal = normal(false);
            send(&normal, &settings, Some("ignored-none"))
                .await
                .read_body()
                .await
                .unwrap();
            let mut compact = compact(&normal);
            compact["stream"] = json!(streaming);
            let mut result = send(&compact, &settings, Some("ignored-none")).await;
            assert_eq!(result.status_code, 200);
            assert!(result.buffered_body.is_none());
            assert_eq!(result.read_body().await.unwrap(), body.as_bytes());
            let requests = server.await.unwrap();
            assert_eq!(requests.len(), 2, "no second summary request");
            assert_eq!(requests[1]["tools"], normal["tools"]);
            assert_eq!(requests[1]["tool_choice"], "none");
            assert_eq!(requests[1]["parallel_tool_calls"], false);
            assert_eq!(requests[1]["input"], compact["input"]);
        }
    }
}

#[tokio::test]
async fn local_compaction_returns_stream_headers_without_waiting_for_the_summary() {
    for mode in ["streaming", "buffered"] {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let mut settings = settings(
            format!("http://{}/v1", listener.local_addr().unwrap()),
            RelayProtocol::Responses,
            true,
        );
        settings.protocol_proxy_auto_retry = true;
        settings.protocol_proxy_auto_retry_mode = mode.into();
        let (release, released) = tokio::sync::oneshot::channel::<()>();
        let body = success(RelayProtocol::Responses, true).2;
        let expected = body.clone();
        let server = tokio::spawn(async move {
            capture_requests(&listener, vec![success(RelayProtocol::Responses, false)]).await;
            let (mut stream, _) = listener.accept().await.unwrap();
            let request = read_request(&mut stream).await;
            let headers = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(headers.as_bytes()).await.unwrap();
            released.await.unwrap();
            stream.write_all(body.as_bytes()).await.unwrap();
            request
        });
        let normal = normal(false);
        send(&normal, &settings, Some("no-buffer"))
            .await
            .read_body()
            .await
            .unwrap();
        let mut compact = compact(&normal);
        compact["stream"] = json!(true);
        let mut result = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            send(&compact, &settings, Some("no-buffer")),
        )
        .await
        .expect("the experiment must not wait for the summary body");
        release.send(()).unwrap();
        assert_eq!(
            result.read_body().await.unwrap().as_slice(),
            expected.as_bytes()
        );
        assert_eq!(server.await.unwrap()["tools"], normal["tools"]);
    }
}

#[tokio::test]
async fn local_compaction_forwards_summary_streams_once_with_auto_retry_enabled() {
    for mode in ["streaming", "buffered"] {
        for body in [
            // 旧校验会拒绝扩展事件或 delta/final 差异，进而重复请求摘要。
            concat!(
                "data: {\"type\":\"provider.extension\"}\n\n",
                "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Summary\"}\n\n",
                "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_summary\",",
                "\"status\":\"completed\",\"output\":[{\"type\":\"message\",\"role\":\"assistant\",",
                "\"content\":[{\"type\":\"output_text\",\"text\":\"Summary.\"}]}]}}\n\n"
            ),
            "data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"code\":\"server_overloaded\",\"message\":\"overloaded\"}}}\n\n",
        ] {
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                .await
                .unwrap();
            let mut settings = settings(
                format!("http://{}/v1", listener.local_addr().unwrap()),
                RelayProtocol::Responses,
                true,
            );
            settings.protocol_proxy_auto_retry = true;
            settings.protocol_proxy_auto_retry_mode = mode.into();
            settings.protocol_proxy_auto_retry_interval_seconds = 1;
            let server = tokio::spawn(capture(
                listener,
                vec![
                    success(RelayProtocol::Responses, false),
                    (200, "text/event-stream", body.into()),
                ],
            ));
            let normal = normal(false);
            send(&normal, &settings, Some("single-summary-stream"))
                .await
                .read_body()
                .await
                .unwrap();
            let mut compact = compact(&normal);
            compact["stream"] = json!(true);
            let mut result = send(&compact, &settings, Some("single-summary-stream")).await;
            assert_eq!(result.status_code, 200);
            assert!(
                result.buffered_body.is_none(),
                "local summaries must not be prevalidated"
            );
            assert!(result.stream_prefix.is_empty());
            assert_eq!(result.read_body().await.unwrap(), body.as_bytes());
            let requests = server.await.unwrap();
            assert_eq!(
                requests.len(),
                2,
                "one ordinary request and exactly one summary request"
            );
            assert_eq!(requests[1]["tools"], requests[0]["tools"]);
            assert_eq!(requests[1]["input"], compact["input"]);
        }
    }
}

#[tokio::test]
async fn local_compaction_disabled_preserves_upstream_stream_auto_retry() {
    for mode in ["streaming", "buffered"] {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let mut settings = settings(
            format!("http://{}/v1", listener.local_addr().unwrap()),
            RelayProtocol::Responses,
            false,
        );
        settings.protocol_proxy_auto_retry = true;
        settings.protocol_proxy_auto_retry_mode = mode.into();
        settings.protocol_proxy_auto_retry_max_attempts = 1;
        settings.protocol_proxy_auto_retry_interval_seconds = 1;
        let reply = success(RelayProtocol::Responses, true);
        let expected = reply.2.clone();
        let server = tokio::spawn(capture(listener, vec![
            (200, "text/event-stream",
                "data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"code\":\"server_overloaded\",\"message\":\"overloaded\"}}}\n\n".into()),
            reply,
        ]));
        let compact = compact(&normal(true));
        let mut result = send(&compact, &settings, Some("experiment-disabled")).await;
        assert_eq!(result.read_body().await.unwrap(), expected.as_bytes());
        let requests = server.await.unwrap();
        assert_eq!(
            requests.len(),
            2,
            "disabled experiment must preserve upstream overload retries"
        );
        assert_eq!(requests[0], compact);
        assert_eq!(requests[1], compact);
    }
}

#[tokio::test]
async fn local_compaction_remembers_tools_after_a_buffered_ordinary_response() {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let mut settings = settings(
        format!("http://{}/v1", listener.local_addr().unwrap()),
        RelayProtocol::Responses,
        true,
    );
    settings.protocol_proxy_auto_retry = true;
    settings.protocol_proxy_auto_retry_mode = "buffered".into();
    let reply = success(RelayProtocol::Responses, true);
    let server = tokio::spawn(capture(listener, vec![reply.clone(), reply]));
    let normal = normal(true);
    let mut result = send(&normal, &settings, Some("buffered-tools")).await;
    assert!(result.buffered_body.is_some());
    result.read_body().await.unwrap();
    let mut result = send(&compact(&normal), &settings, Some("buffered-tools")).await;
    assert!(result.buffered_body.is_none());
    result.read_body().await.unwrap();
    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1]["tools"], requests[0]["tools"]);
    assert_eq!(requests[1]["tool_choice"], "none");
}

#[tokio::test]
async fn local_compaction_preserves_exec_projection_without_an_experimental_retry() {
    for status in [200, 400] {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let settings = settings(
            format!("http://{}/v1", listener.local_addr().unwrap()),
            RelayProtocol::Responses,
            true,
        );
        let reply = success(RelayProtocol::Responses, false);
        let rejection = (
            400,
            "application/json",
            json!({"error":{
                "type":"invalid_request_error",
                "message":"Unsupported custom tool: 'exec'. Only 'apply_patch' is supported.",
                "param":"tools[0]"
            }})
            .to_string(),
        );
        let replies = vec![rejection, reply.clone(), (status, reply.1, reply.2)];
        let server = tokio::spawn(capture(listener, replies));
        let mut normal = normal(false);
        normal["tools"] = json!([{"type":"custom","name":"exec","description":"fixture"}]);
        normal["tool_choice"] = json!("auto");
        normal["input"].as_array_mut().unwrap().extend([
            json!({"type":"custom_tool_call","id":"ctc_prior","call_id":"prior","name":"exec","input":"echo prior"}),
            json!({"type":"custom_tool_call_output","id":"ctco_prior","call_id":"prior","output":"prior"}),
        ]);
        send(&normal, &settings, Some("exec"))
            .await
            .read_body()
            .await
            .unwrap();
        let mut compact = compact(&normal);
        let items = compact["input"].as_array_mut().unwrap();
        items.insert(3, json!({"type":"custom_tool_call","id":"ctc_new","call_id":"new","name":"exec","input":"echo new"}));
        items.insert(4, json!({"type":"custom_tool_call_output","id":"ctco_new","call_id":"new","output":"new"}));
        let mut result = send(&compact, &settings, Some("exec")).await;
        assert_eq!(result.status_code, status);
        result.read_body().await.unwrap();
        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[2]["tools"][0]["type"], "function");
        assert_eq!(requests[2]["tool_choice"], "none");
        for request in &requests[2..] {
            assert!(
                !request["input"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|item| matches!(
                        item["type"].as_str(),
                        Some("custom_tool_call" | "custom_tool_call_output")
                    ))
            );
            assert_eq!(request["input"][1]["type"], "function_call");
        }
    }
}
