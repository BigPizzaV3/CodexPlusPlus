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
