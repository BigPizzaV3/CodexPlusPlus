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
        // Deliberately omit output_item.done: the proxy must make this consumable.
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
    let mut requests = Vec::new();
    for (status, content_type, reply) in replies {
        let (mut stream, _) =
            tokio::time::timeout(std::time::Duration::from_secs(10), listener.accept())
                .await
                .unwrap()
                .unwrap();
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
        requests.push(serde_json::from_slice(&bytes[start..start + length]).unwrap());
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
            if streaming && protocol == RelayProtocol::Responses {
                let events = response
                    .split("\n\n")
                    .filter_map(|frame| {
                        frame
                            .lines()
                            .find_map(|line| line.strip_prefix("data: "))
                            .and_then(|data| serde_json::from_str::<Value>(data).ok())
                    })
                    .collect::<Vec<_>>();
                assert!(
                    events
                        .iter()
                        .any(|event| event["type"] == "response.output_item.done"
                            && event["item"]["content"][0]["text"] == "Summary.")
                );
                assert!(
                    events
                        .iter()
                        .any(|event| event["type"] == "response.completed"
                            && event["response"]["usage"]["total_tokens"] == 1203)
                );
            }
            let requests = server.await.unwrap();
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
async fn local_compaction_disabled_missing_session_and_changed_history_use_original() {
    for case in ["disabled", "session", "instructions", "trimmed"] {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let settings = settings(
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
        let mut compact = compact(&normal);
        if case == "instructions" {
            compact["instructions"] = json!("New system instructions");
        }
        if case == "trimmed" {
            compact["input"].as_array_mut().unwrap().remove(0);
        }
        send(&compact, &settings, thread)
            .await
            .read_body()
            .await
            .unwrap();
        assert_eq!(server.await.unwrap()[1], compact);
    }
}

#[tokio::test]
async fn local_compaction_inherits_only_entry_or_target_opt_in_and_keeps_target_model() {
    for (entry, target) in [(true, false), (false, true), (false, false)] {
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
    }
}

#[tokio::test]
async fn local_compaction_invalid_summary_falls_back_once_but_rate_limits_do_not() {
    for status in [200, 400, 422, 429, 500] {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let settings = settings(
            format!("http://{}/v1", listener.local_addr().unwrap()),
            RelayProtocol::Responses,
            true,
        );
        let reply = success(RelayProtocol::Responses, false);
        let fallback = matches!(status, 200 | 400 | 422);
        let invalid = json!({"id":"resp_bad","status":"completed",
            "output":[{"type":"function_call","name":"lookup"}]})
        .to_string();
        let mut replies = vec![reply.clone(), (status, "application/json", invalid)];
        if fallback {
            replies.push(reply);
        }
        let server = tokio::spawn(capture(listener, replies));
        let normal = normal(false);
        send(&normal, &settings, Some("fallback"))
            .await
            .read_body()
            .await
            .unwrap();
        let compact = compact(&normal);
        let mut result = send(&compact, &settings, Some("fallback")).await;
        assert_eq!(result.status_code, if fallback { 200 } else { status });
        result.read_body().await.unwrap();
        let requests = server.await.unwrap();
        assert_eq!(requests.len(), if fallback { 3 } else { 2 });
        if fallback {
            assert_eq!(requests[2], compact);
        }
    }
}

#[tokio::test]
async fn local_compaction_exec_history_is_projected_for_candidate_and_fallback() {
    for fallback in [false, true] {
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
        let mut replies = vec![rejection, reply.clone()];
        if fallback {
            replies.push((
                400,
                "application/json",
                json!({"error":{"message":"candidate rejected"}}).to_string(),
            ));
        }
        replies.push(reply);
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
        send(&compact, &settings, Some("exec"))
            .await
            .read_body()
            .await
            .unwrap();
        let requests = server.await.unwrap();
        assert_eq!(requests.len(), if fallback { 4 } else { 3 });
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
        if fallback {
            assert_eq!(requests[3]["tools"], json!([]));
        }
    }
}
