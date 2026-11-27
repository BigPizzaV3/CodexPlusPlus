use super::*;
use crate::extension_gateway as extensions;

fn log_fixture() -> ProxyRequestLog {
    ProxyRequestLog {
        started: std::time::Instant::now(),
        entry: serde_json::from_value(serde_json::json!({
            "id":"forwarded-fixture","timestampMs":0,"startedAtMs":0,"durationMs":0,
            "method":"POST","path":"/gw/codex/v1/responses","statusCode":0,
            "success":false,"stream":true,"model":"group/model",
            "upstreamModel":"","relayId":"","relayName":"","wireApi":"",
            "attempts":0,"autoRetries":0,"agent":"codex"
        }))
        .unwrap(),
        written: true,
        silent: true,
        final_response_eligible: false,
    }
}

fn final_upstream() -> crate::protocol_proxy::UpstreamProxyResponse {
    let mut trace = crate::request_log::RequestTrace {
        relay_id: "final-supplier".into(),
        relay_name: "Final supplier".into(),
        attempts: 2,
        auto_retries: 1,
        ..Default::default()
    };
    trace.bind_upstream_model(&serde_json::json!({"model":"final-upstream-model"}));
    crate::protocol_proxy::UpstreamProxyResponse {
        status_code: 200,
        is_stream: true,
        content_type: "text/event-stream".into(),
        wire_api: crate::protocol_proxy::UpstreamWireApi::Responses,
        compaction: false,
        native_compaction_passthrough: false,
        upstream_reasoning_included: false,
        buffered_body: None,
        stream_prefix: Vec::new(),
        request_trace: Some(trace),
        exec_adapted: false,
        exec_outcome: None,
        websocket: None,
        response: None,
    }
}

fn complete_log(
    log: &mut ProxyRequestLog,
    upstream: &crate::protocol_proxy::UpstreamProxyResponse,
) {
    let mut transformer = crate::protocol_proxy::ResponsesSseTransformer::default();
    transformer.push_bytes(b"data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"usage\":{\"input_tokens\":12,\"output_tokens\":3,\"input_tokens_details\":{\"cached_tokens\":0}}}}\n\n");
    let final_usage = transformer.final_response_usage();
    let usage = transformer.take_usage();
    let failure = transformer.finish();
    log.record_upstream(upstream);
    log.record_stream(failure.as_deref(), usage, None);
    log.record_final_response(final_usage);
}

#[tokio::test]
async fn forwarded_codex_retry_requires_claim_and_keeps_final_route() {
    extensions::with_codex_response_plan_for_tests(Some(Vec::new()), async {
        let mut request = reqwest::Request::new(
            reqwest::Method::POST,
            format!(
                "http://127.0.0.1:{}/gw/codex/v1/responses",
                crate::protocol_proxy::protocol_proxy_port()
            )
            .parse()
            .unwrap(),
        );
        request
            .headers_mut()
            .insert(extensions::EXTENSIONS_APPLIED_HEADER, "1".parse().unwrap());
        extensions::refresh_response_passthrough(&mut request);
        let mut headers = "POST /gw/codex/v1/responses HTTP/1.1\r\n".to_string();
        for (name, value) in request.headers() {
            headers.push_str(&format!("{name}: {}\r\n", value.to_str().unwrap()));
        }
        // 独立任务模拟 HTTP 内层；外层快照不会自动跨任务继承，只能凭一次性请求头证明。
        tokio::spawn(async move {
            let verified =
                extensions::claim_response_passthrough("POST", "/gw/codex/v1/responses", &headers);
            assert!(verified);
            extensions::EXTENSIONS_ALREADY_APPLIED
                .scope(
                    true,
                    extensions::VERIFIED_RESPONSE_PASSTHROUGH.scope(verified, async {
                        let upstream = final_upstream();
                        let mut log = log_fixture();
                        complete_log(&mut log, &upstream);
                        assert_eq!(
                            log.entry.final_response_evidence.as_deref(),
                            Some("final-response-v1")
                        );
                        assert_eq!(log.entry.relay_id, "final-supplier");
                        assert_eq!(log.entry.upstream_model, "final-upstream-model");
                        assert_eq!(log.entry.attempts, 2);
                        assert_eq!(log.entry.auto_retries, 1);
                        assert_eq!(log.entry.usage.as_ref().unwrap().input_tokens, 12);
                        for agent in ["claude", "grok"] {
                            log.entry.agent = Some(agent.into());
                            complete_log(&mut log, &upstream);
                            assert!(log.entry.final_response_evidence.is_none());
                        }
                        log.entry.agent = Some("codex".into());
                        for case in ["unbound", "gateway-relay", "exec", "compaction"] {
                            let mut upstream = final_upstream();
                            match case {
                                "unbound" => {
                                    upstream
                                        .request_trace
                                        .as_mut()
                                        .unwrap()
                                        .upstream_model_bound = false
                                }
                                "gateway-relay" => {
                                    upstream.request_trace.as_mut().unwrap().relay_id =
                                        crate::gateway::apply::CODEX_GATEWAY_RELAY_ID.into()
                                }
                                "exec" => upstream.exec_adapted = true,
                                _ => upstream.compaction = true,
                            }
                            complete_log(&mut log, &upstream);
                            assert!(log.entry.final_response_evidence.is_none(), "{case}");
                        }
                    }),
                )
                .await;
        })
        .await
        .unwrap();
    })
    .await;
}

#[tokio::test]
async fn applied_header_without_verified_outer_plan_does_not_mark_retry() {
    extensions::EXTENSIONS_ALREADY_APPLIED
        .scope(true, async {
            let mut log = log_fixture();
            complete_log(&mut log, &final_upstream());
            assert!(log.entry.final_response_evidence.is_none());
        })
        .await;
}

#[tokio::test]
async fn responses_supplier_receipt_requires_delivered_unchanged_terminal() {
    extensions::with_codex_response_plan_for_tests(Some(Vec::new()), async {
        for case in ["delivered", "truncated", "failed", "changed", "write-error"] {
            let terminal = "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_supplier\",\"status\":\"completed\",\"usage\":{\"input_tokens\":12,\"output_tokens\":3}}}\n\n";
            let mut transformer = crate::protocol_proxy::ResponsesSseTransformer::default();
            transformer.push_bytes(if case == "truncated" {
                terminal[..terminal.len() - 2].as_bytes()
            } else {
                terminal.as_bytes()
            });
            if case == "failed" {
                transformer.push_bytes(b"data: {\"type\":\"response.failed\"}\n\n");
            }
            let receipt = transformer.final_response_usage();
            let mut usage = transformer.take_usage();
            if case == "changed" {
                usage.as_mut().unwrap().input_tokens = 999;
            }
            let failure = transformer.finish();
            let mut log = log_fixture();
            log.record_upstream(&final_upstream());
            log.record_stream(failure.as_deref(), usage, None);
            if case == "write-error" {
                log.record_error("502 Bad Gateway", "downstream closed".to_owned());
            }
            log.record_final_response(receipt);
            if case == "delivered" {
                assert_eq!(log.entry.final_response_evidence.as_deref(), Some("final-response-supplier-v1"));
                assert_eq!(log.entry.final_response_id.as_deref(), Some("resp_supplier"));
                assert_eq!(log.entry.relay_id, "final-supplier");
                assert_eq!(log.entry.upstream_model, "final-upstream-model");
                assert!(!log.entry.usage.as_ref().unwrap().cached_tokens_known);
            } else {
                assert!(log.entry.final_response_evidence.is_none(), "{case}");
                assert!(log.entry.final_response_id.is_none(), "{case}");
            }
        }
    }).await;
}

#[tokio::test]
async fn chat_conversion_records_delivered_final_id_and_preserves_cache_knowledge() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    extensions::with_codex_response_plan_for_tests(Some(Vec::new()), async {
        for case in ["cached", "zero", "unknown", "incomplete", "broken", "late-usage", "failed"] {
            let cached = match case { "zero" => Some(0), "unknown" => None, _ => Some(90) };
            let mut usage = serde_json::json!({"prompt_tokens":120,"completion_tokens":30});
            if let Some(cached) = cached {
                usage["prompt_tokens_details"] = serde_json::json!({"cached_tokens":cached});
            }
            let finish = if case == "incomplete" { "length" } else { "stop" };
            let body = format!("data: {{\"id\":\"chatcmpl-returned\",\"choices\":[{{\"delta\":{{\"content\":\"hi\"}},\"finish_reason\":\"{finish}\"}}]}}\n\ndata: {{\"choices\":[],\"usage\":{usage}}}\n\ndata: [DONE]\n\n");
            let body = match case {
                "late-usage" => format!("{body}data: {{\"choices\":[],\"usage\":{{\"prompt_tokens\":999,\"completion_tokens\":30}}}}\n\n"),
                "failed" => format!("{body}data: {{\"error\":{{\"message\":\"failed\"}}}}\n\n"),
                _ => body,
            };
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") { request.push(socket.read_u8().await.unwrap()); }
                socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n").await.unwrap();
                socket.write_all(format!("{:X}\r\n{body}\r\n", body.len()).as_bytes()).await.unwrap();
                if case != "broken" { socket.write_all(b"0\r\n\r\n").await.unwrap(); }
                socket.shutdown().await.unwrap();
            });
            let response = reqwest::Client::builder().no_proxy().build().unwrap()
                .get(format!("http://{address}")).send().await.unwrap();
            #[derive(Default)]
            struct Sink(Vec<u8>);
            impl SseSink for Sink {
                async fn send(&mut self, bytes: &[u8]) -> std::io::Result<()> {
                    self.0.extend_from_slice(bytes);
                    Ok(())
                }
            }
            let mut sink = Sink::default();
            let mut terminal = None;
            let (failure, logged, detail, _) = forward_protocol_proxy_stream_for_client_with_live_request(
                &mut sink, &[], response, crate::protocol_proxy::UpstreamWireApi::ChatCompletions,
                None, None, false, None, &mut terminal,
            ).await.unwrap();
            server.await.unwrap();
            let mut upstream = final_upstream();
            upstream.wire_api = crate::protocol_proxy::UpstreamWireApi::ChatCompletions;
            let mut log = log_fixture();
            log.record_upstream(&upstream);
            log.record_stream(failure.as_deref(), logged, detail);
            log.record_final_response(terminal);
            if ["incomplete", "broken", "late-usage", "failed"].contains(&case) {
                assert!(log.entry.final_response_id.is_none(), "{case}");
                assert!(log.entry.final_response_evidence.is_none(), "{case}");
                continue;
            }
            let output = String::from_utf8(sink.0).unwrap();
            let delivered: Value = output.lines().filter_map(|line| line.strip_prefix("data: "))
                .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                .find(|row| row["type"] == "response.completed").unwrap();
            assert_eq!(log.entry.final_response_id.as_deref(), delivered["response"]["id"].as_str());
            assert_eq!(
                log.entry.final_response_evidence.as_deref(),
                Some(if cached.is_some() { "final-response-v1" } else { "final-response-supplier-v1" }),
            );
            let counts = log.entry.usage.as_ref().unwrap();
            assert_eq!(counts.input_tokens, 120);
            assert_eq!(counts.cached_tokens, cached.unwrap_or(0));
            assert_eq!(counts.cached_tokens_known, cached.is_some());
            assert_eq!(log.entry.relay_id, "final-supplier");
            assert_eq!(log.entry.upstream_model, "final-upstream-model");
            assert_eq!(delivered["response"]["usage"]["input_tokens"], 120 - cached.unwrap_or(0));

            let chat = serde_json::json!({"id":"chatcmpl-returned","usage":usage,
                "choices":[{"message":{"content":"hi"},"finish_reason":"stop"}]});
            let bytes = serde_json::to_vec(&chat).unwrap();
            let (response, _) = non_stream_body_to_responses(upstream.wire_api, &bytes, None).unwrap();
            let terminal = crate::request_log::converted_final_response_usage(
                &response, crate::request_log::usage_from_body_bytes(&bytes)).unwrap();
            assert_eq!(terminal.response_id.as_deref(), response["id"].as_str());
            assert_eq!(terminal.usage, *counts);
        }
    }).await;
}
