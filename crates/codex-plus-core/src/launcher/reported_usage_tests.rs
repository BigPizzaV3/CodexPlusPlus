use super::*;
use serde_json::json;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

struct RestoreSettings(Option<PathBuf>);

impl Drop for RestoreSettings {
    fn drop(&mut self) {
        crate::paths::set_settings_path_for_tests(self.0.take());
        crate::builtin_takeover::invalidate();
    }
}

fn usage() -> Value {
    json!({"input_tokens":305936,"output_tokens":1552,"total_tokens":307488,
        "input_tokens_details":{"cached_tokens":305624}})
}

fn response(streaming: bool) -> String {
    let response = json!({"id":"fixture-response","status":"completed","usage":usage(),
        "output":[{"type":"message","id":"fixture-message","role":"assistant",
            "content":[{"type":"output_text","text":"Fixture summary."}]}]});
    if streaming {
        format!(
            "data: {}\n\n",
            json!({"type":"response.completed","response":response})
        )
    } else {
        response.to_string()
    }
}

fn request(streaming: bool, compact: bool) -> Value {
    json!({"model":"fixture","stream":streaming,
    "tools":if compact {json!([])} else {json!([
        {"type":"function","name":"lookup","parameters":{"type":"object"}}
    ])},
    "client_metadata":{"x-codex-turn-metadata":json!({
        "request_kind":if compact {"compaction"} else {"sampling"},
        "compaction":{"implementation":"responses"}
    }).to_string()},
    "input":[
        {"type":"web_search_call","id":"fixture-search","status":"completed",
            "action":{"type":"search","query":"fixture"}},
        {"role":"user","content":[{"type":"input_text",
            "text":if compact {"Fixture handoff."} else {"Fixture task."}}]}
    ]})
}

async fn helper_request(body: &Value, originator: &str) -> (String, Vec<u8>) {
    let identity = format!("Originator: {originator}\r\nX-Codex-Window-Id: fixture-window\r\n");
    helper_request_with_identity(body, &identity).await
}

async fn helper_request_with_identity(body: &Value, identity: &str) -> (String, Vec<u8>) {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, remote) = listener.accept().await.unwrap();
        handle_helper_connection(stream, Some(remote))
            .await
            .unwrap();
    });
    let mut client = tokio::net::TcpStream::connect(address).await.unwrap();
    let body = body.to_string();
    let raw = format!(
        "POST /v1/responses HTTP/1.1\r\nHost: {address}\r\n{identity}Thread-Id: fixture-thread\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    client.write_all(raw.as_bytes()).await.unwrap();
    let mut received = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), client.read_to_end(&mut received))
        .await
        .unwrap()
        .unwrap();
    server.await.unwrap();
    let end = find_header_end(&received).unwrap();
    (
        String::from_utf8(received[..end].to_vec()).unwrap(),
        received[end + 4..].to_vec(),
    )
}

fn write_settings(
    path: &Path,
    url: &str,
    enabled: bool,
    master: bool,
    buffered: bool,
    local: bool,
    search: bool,
) {
    let settings = json!({
        "enhancementsEnabled":master,
        "codexAppReportedUsageCompaction":enabled,
        "protocolProxyAutoRetry":buffered,
        "protocolProxyAutoRetryMode":if buffered {"buffered"} else {"streaming"},
        "relayProfilesEnabled":true,"activeRelayId":"usage-fixture",
        "relayProfiles":[{"id":"usage-fixture","name":"Usage fixture",
            "upstreamBaseUrl":format!("{url}/v1"),"baseUrl":format!("{url}/v1"),
            "apiKey":"fixture-only","protocol":"responses","relayMode":"pureApi",
            "localCompactionPromptAtEnd":local,"webSearchHistoryCompat":search}]
    });
    std::fs::write(path, serde_json::to_vec(&settings).unwrap()).unwrap();
}

#[tokio::test]
async fn helper_reported_usage_scopes_to_codex_clients_and_preserves_genuine_markers() {
    let _log_guard = crate::request_log::request_log_test_guard();
    let _settings_guard = crate::paths::settings_path_test_guard();
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    let _restore = RestoreSettings(crate::paths::set_settings_path_for_tests(Some(
        path.clone(),
    )));
    for streaming in [false, true] {
        for buffered in [false, true] {
            if buffered && !streaming {
                continue;
            }
            for (has_marker, local_upstream) in [(false, false), (true, false), (true, true)] {
                let genuine = has_marker && !local_upstream;
                for (originator, enabled, master) in [
                    ("Codex Desktop", true, true),
                    ("codex_cli_rs", true, true),
                    ("codex_exec", true, true),
                    ("fixture-other", true, true),
                    ("Codex Desktop", false, true),
                    ("Codex Desktop", true, false),
                ] {
                    let upstream = MockServer::start().await;
                    let mut reply = ResponseTemplate::new(200)
                        .set_body_string(response(streaming))
                        .insert_header(
                            "Content-Type",
                            if streaming {
                                "text/event-stream"
                            } else {
                                "application/json"
                            },
                        );
                    if has_marker {
                        reply = reply.insert_header("X-Reasoning-Included", "true");
                    }
                    if local_upstream {
                        reply = reply.insert_header("X-CodexPlus-Compaction-Usage", "reported");
                    }
                    Mock::given(method("POST"))
                        .respond_with(reply)
                        .expect(1)
                        .mount(&upstream)
                        .await;
                    write_settings(
                        &path,
                        &upstream.uri(),
                        enabled,
                        master,
                        buffered,
                        false,
                        false,
                    );
                    let body = request(streaming, false);
                    let (headers, received) = helper_request(&body, originator).await;
                    assert!(headers.starts_with("HTTP/1.1 200 OK"), "{headers}");
                    let local = originator != "fixture-other" && enabled && master;
                    assert_eq!(
                        header_value_from_headers(&headers, "x-reasoning-included").is_some(),
                        local || genuine
                    );
                    assert_eq!(
                        header_value_from_headers(&headers, "x-codexplus-compaction-usage"),
                        (local && !genuine).then_some("reported".into())
                    );
                    let received: Value = if streaming {
                        let text = String::from_utf8(received).unwrap();
                        text.lines()
                            .filter_map(|line| line.strip_prefix("data: "))
                            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                            .find(|event| event["type"] == "response.completed")
                            .unwrap()["response"]
                            .clone()
                    } else {
                        serde_json::from_slice(&received).unwrap()
                    };
                    assert_eq!(
                        received["usage"],
                        usage(),
                        "Never rewrite reported usage or cached tokens"
                    );
                    let sent = upstream.received_requests().await.unwrap();
                    assert_eq!(sent[0].body_json::<Value>().unwrap(), body);
                    assert!(!sent[0].headers.contains_key("x-reasoning-included"));
                    assert!(!sent[0].headers.contains_key("x-codexplus-compaction-usage"));
                }
            }
        }
    }
}

#[tokio::test]
async fn helper_reported_usage_requires_codex_client_identity() {
    let _log_guard = crate::request_log::request_log_test_guard();
    let _settings_guard = crate::paths::settings_path_test_guard();
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    let _restore = RestoreSettings(crate::paths::set_settings_path_for_tests(Some(
        path.clone(),
    )));
    for (identity, enabled) in [
        ("", false),
        ("Originator: Codex Desktop\r\n", true),
        ("Originator: codex_cli_rs\r\n", true),
        ("Originator: codex_exec\r\n", true),
        ("Originator: fixture-other\r\n", false),
        ("X-Codex-Window-Id: fixture-window\r\n", false),
    ] {
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(response(false))
                    .insert_header("Content-Type", "application/json"),
            )
            .expect(1)
            .mount(&upstream)
            .await;
        write_settings(&path, &upstream.uri(), true, true, false, false, false);
        let (headers, _) = helper_request_with_identity(&request(false, false), identity).await;
        assert!(headers.starts_with("HTTP/1.1 200 OK"), "{headers}");
        assert_eq!(
            header_value_from_headers(&headers, "x-reasoning-included").is_some(),
            enabled
        );
        assert_eq!(
            header_value_from_headers(&headers, "x-codexplus-compaction-usage").is_some(),
            enabled
        );
    }
}

#[tokio::test]
async fn helper_reported_usage_coexists_with_local_cache_and_search_repair() {
    let _log_guard = crate::request_log::request_log_test_guard();
    let _settings_guard = crate::paths::settings_path_test_guard();
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    let _restore = RestoreSettings(crate::paths::set_settings_path_for_tests(Some(
        path.clone(),
    )));
    for streaming in [false, true] {
        for (local, search) in [(false, false), (false, true), (true, false), (true, true)] {
            let upstream = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_string(response(streaming))
                        .insert_header(
                            "Content-Type",
                            if streaming {
                                "text/event-stream"
                            } else {
                                "application/json"
                            },
                        ),
                )
                .expect(2)
                .mount(&upstream)
                .await;
            write_settings(&path, &upstream.uri(), true, true, false, local, search);
            helper_request(&request(streaming, false), "Codex Desktop").await;
            let compact = request(streaming, true);
            let (headers, received) = helper_request(&compact, "Codex Desktop").await;
            assert!(headers.starts_with("HTTP/1.1 200 OK"), "{headers}");
            assert_eq!(
                header_value_from_headers(&headers, "x-codexplus-compaction-usage"),
                Some("reported".into())
            );
            assert!(String::from_utf8_lossy(&received).contains("Fixture summary."));
            let sent = upstream.received_requests().await.unwrap();
            let sent: Value = sent[1].body_json().unwrap();
            assert_eq!(
                sent["input"], compact["input"],
                "Keep search history intact"
            );
            let tools = sent["tools"].as_array().unwrap();
            assert_eq!(tools.iter().any(|tool| tool["type"] == "function"), local);
            assert_eq!(
                tools.iter().any(|tool| tool["type"] == "web_search"),
                search
            );
            if local || search {
                assert_eq!(sent["tool_choice"], "none");
            }
        }
    }
}

#[tokio::test]
async fn helper_reported_usage_does_not_mask_compaction_errors_or_retry() {
    let _log_guard = crate::request_log::request_log_test_guard();
    let _settings_guard = crate::paths::settings_path_test_guard();
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("settings.json");
    let _restore = RestoreSettings(crate::paths::set_settings_path_for_tests(Some(
        path.clone(),
    )));
    for status in [400, 200] {
        let upstream = MockServer::start().await;
        let body = if status == 200 {
            "data: {\"type\":\"response.failed\",\"response\":{\"status\":\"failed\",\"error\":{\"code\":\"fixture_error\",\"message\":\"response protection is unavailable\"}}}\n\n"
        } else {
            "{\"error\":{\"code\":\"fixture_error\",\"message\":\"response protection is unavailable\"}}"
        };
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(status)
                    .set_body_string(body)
                    .insert_header(
                        "Content-Type",
                        if status == 200 {
                            "text/event-stream"
                        } else {
                            "application/json"
                        },
                    ),
            )
            .expect(1)
            .mount(&upstream)
            .await;
        write_settings(&path, &upstream.uri(), true, true, false, true, true);
        let (headers, received) =
            helper_request(&request(status == 200, true), "Codex Desktop").await;
        assert!(
            headers.starts_with(&format!("HTTP/1.1 {status}")),
            "{headers}"
        );
        let text = String::from_utf8(received).unwrap();
        assert!(
            text.contains("response protection is unavailable"),
            "{text}"
        );
        assert!(!text.contains("\"type\":\"response.completed\""));
        assert_eq!(upstream.received_requests().await.unwrap().len(), 1);
    }
}
