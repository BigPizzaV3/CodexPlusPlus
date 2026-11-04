use super::*;

const CODEX_PATH: &str = "/gw/codex/v1/responses";

fn forwarded_headers() -> String {
    let port = CODEX_RESPONSE_PLAN.with(|plan| plan.port);
    let mut request = reqwest::Request::new(
        reqwest::Method::POST,
        format!("http://127.0.0.1:{port}{CODEX_PATH}")
            .parse()
            .unwrap(),
    );
    request
        .headers_mut()
        .insert(EXTENSIONS_APPLIED_HEADER, "1".parse().unwrap());
    refresh_response_passthrough(&mut request);
    let mut raw = format!("POST {CODEX_PATH} HTTP/1.1\r\n");
    for (name, value) in request.headers() {
        raw.push_str(&format!("{name}: {}\r\n", value.to_str().unwrap()));
    }
    raw.push_str("\r\n");
    raw
}

fn configured_response_middleware() -> MiddlewareList {
    vec![(
        "fixture".to_string(),
        ExtensionGatewayMiddleware {
            script: "unused-fixture.mjs".into(),
            timeout_secs: 1,
            config: json!({}),
            stage: "response".to_string(),
            order: 0,
        },
    )]
}

#[tokio::test]
async fn passthrough_requires_live_single_use_codex_grant() {
    assert!(!claim_response_passthrough(
        "POST",
        CODEX_PATH,
        &format!(
            "POST {CODEX_PATH} HTTP/1.1\r\n{EXTENSIONS_APPLIED_HEADER}: 1\r\n{RESPONSE_PASSTHROUGH_HEADER}: invented\r\n\r\n"
        )
    ));
    let stale = with_codex_response_plan_for_tests(Some(Vec::new()), async {
        let headers = forwarded_headers();
        for (method, path) in [
            ("GET", CODEX_PATH),
            ("POST", "/gw/grok/v1/responses"),
            ("POST", "/gw/claude/v1/messages"),
            ("POST", "/gw/codex/v1/responses/compact"),
        ] {
            assert!(!claim_response_passthrough(method, path, &headers));
        }
        let missing_applied = headers.replace(&format!("{EXTENSIONS_APPLIED_HEADER}: 1\r\n"), "");
        assert!(!claim_response_passthrough(
            "POST",
            CODEX_PATH,
            &missing_applied
        ));
        let token = CODEX_RESPONSE_PLAN.with(|plan| plan.token.borrow().clone().unwrap());
        let duplicate = format!("{headers}{RESPONSE_PASSTHROUGH_HEADER}: {token}\r\n");
        assert!(!claim_response_passthrough("POST", CODEX_PATH, &duplicate));
        assert!(claim_response_passthrough("POST", CODEX_PATH, &headers));
        assert!(!claim_response_passthrough("POST", CODEX_PATH, &headers));
        let retry = forwarded_headers();
        assert_ne!(retry, headers);
        assert!(claim_response_passthrough("POST", CODEX_PATH, &retry));
        assert!(!claim_response_passthrough("POST", CODEX_PATH, &retry));
        // 另签一张未消费的凭据，验证正常离开 scope 也会回收。
        with_codex_response_plan_for_tests(Some(Vec::new()), async { forwarded_headers() }).await
    })
    .await;
    assert!(!claim_response_passthrough("POST", CODEX_PATH, &stale));
}

#[tokio::test]
async fn response_middleware_or_unknown_plan_never_grants_passthrough() {
    for plan in [None, Some(configured_response_middleware())] {
        with_codex_response_plan_for_tests(plan, async {
            assert!(
                extension_headers()
                    .iter()
                    .all(|(name, _)| name != RESPONSE_PASSTHROUGH_HEADER)
            );
            assert!(!claim_response_passthrough(
                "POST",
                CODEX_PATH,
                &forwarded_headers()
            ));
        })
        .await;
    }
}

#[tokio::test]
async fn middleware_header_merging_cannot_duplicate_or_forge_internal_grants() {
    let attempted =
        json!({(RESPONSE_PASSTHROUGH_HEADER): "forged", (EXTENSIONS_APPLIED_HEADER): "1"});
    assert!(allowed_headers(&attempted).is_empty());
    assert!(allowed_auth_headers(&attempted).is_empty());
    with_codex_response_plan_for_tests(Some(Vec::new()), async {
        EXTENSION_HEADERS
            .scope(std::cell::RefCell::new(Vec::new()), async {
                set_extension_headers(extension_headers());
                set_extension_headers(extension_headers());
                let headers = extension_headers();
                assert_eq!(
                    headers
                        .iter()
                        .filter(|(name, _)| name == RESPONSE_PASSTHROUGH_HEADER)
                        .count(),
                    0
                );
                assert!(claim_response_passthrough(
                    "POST",
                    CODEX_PATH,
                    &forwarded_headers()
                ));
            })
            .await;
    })
    .await;
    assert!(
        extension_headers()
            .iter()
            .all(|(name, _)| name != RESPONSE_PASSTHROUGH_HEADER)
    );
}

#[tokio::test]
async fn final_response_grants_only_target_the_pinned_codex_gateway() {
    with_codex_response_plan_on(Some(Vec::new()), 45678, async {
        for (method, url, applied) in [
            (
                reqwest::Method::POST,
                "https://127.0.0.1:45678/gw/codex/v1/responses",
                true,
            ),
            (
                reqwest::Method::POST,
                "http://example.invalid:45678/gw/codex/v1/responses",
                true,
            ),
            (
                reqwest::Method::POST,
                "http://127.0.0.1:45679/gw/codex/v1/responses",
                true,
            ),
            (
                reqwest::Method::POST,
                "http://127.0.0.1:45678/gw/grok/v1/responses",
                true,
            ),
            (
                reqwest::Method::POST,
                "http://127.0.0.1:45678/v1/responses",
                true,
            ),
            (
                reqwest::Method::POST,
                "http://127.0.0.1:45678/gw/codex/v1/responses/compact",
                true,
            ),
            (
                reqwest::Method::GET,
                "http://127.0.0.1:45678/gw/codex/v1/responses",
                true,
            ),
            (
                reqwest::Method::POST,
                "http://127.0.0.1:45678/gw/codex/v1/responses",
                false,
            ),
        ] {
            let mut request = reqwest::Request::new(method, url.parse().unwrap());
            request
                .headers_mut()
                .insert(RESPONSE_PASSTHROUGH_HEADER, "forged".parse().unwrap());
            if applied {
                request
                    .headers_mut()
                    .insert(EXTENSIONS_APPLIED_HEADER, "1".parse().unwrap());
            }
            refresh_response_passthrough(&mut request);
            assert!(
                !request.headers().contains_key(RESPONSE_PASSTHROUGH_HEADER),
                "{url}"
            );
            assert!(CODEX_RESPONSE_PLAN.with(|plan| plan.token.borrow().is_none()));
        }
        assert!(claim_response_passthrough(
            "POST",
            CODEX_PATH,
            &forwarded_headers()
        ));
        assert!(extension_headers().is_empty());
    })
    .await;
}

#[tokio::test]
async fn existing_transport_refreshes_consumed_grant_on_transparent_retry() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let mut previous_headers = None;
        for attempt in 0..2 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut chunk = [0; 1024];
            while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                let length = stream.read(&mut chunk).await.unwrap();
                assert_ne!(length, 0);
                request.extend_from_slice(&chunk[..length]);
            }
            let headers = String::from_utf8(request).unwrap();
            let verified = claim_response_passthrough("POST", CODEX_PATH, &headers);
            assert!(verified);
            if attempt == 0 {
                previous_headers = Some(headers);
                // 内层已消费凭据，但尚未返回响应头时断开，触发实际传输层透明重发。
                drop(stream);
                continue;
            }
            assert_ne!(previous_headers.as_ref().unwrap(), &headers);
            EXTENSIONS_ALREADY_APPLIED
                .scope(
                    true,
                    VERIFIED_RESPONSE_PASSTHROUGH.scope(verified, async {
                        assert!(final_response_path_verified(true));
                        assert!(!final_response_path_verified(false));
                        assert!(
                            extension_headers().is_empty(),
                            "内层不得把凭据转发给真实上游"
                        );
                    }),
                )
                .await;
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
            stream.shutdown().await.unwrap();
        }
    });
    with_codex_response_plan_on(Some(Vec::new()), address.port(), async {
        let request = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .post(format!("http://{address}{CODEX_PATH}"))
            .header(EXTENSIONS_APPLIED_HEADER, "1");
        let response = crate::protocol_proxy::send_upstream_request_with_header_timeout(
            request,
            Duration::from_secs(5),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), 200);
    })
    .await;
    server.await.unwrap();
}

#[tokio::test]
async fn cancellation_revokes_unconsumed_grant() {
    let (sent, received) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(with_codex_response_plan_for_tests(
        Some(Vec::new()),
        async {
            sent.send(forwarded_headers()).unwrap();
            std::future::pending::<()>().await;
        },
    ));
    let headers = received.await.unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(!claim_response_passthrough("POST", CODEX_PATH, &headers));
}

#[tokio::test]
async fn empty_snapshot_pins_actual_filter_even_if_configuration_changes() {
    let root = tempfile::tempdir().unwrap();
    let manager = UserScriptManager::new(
        root.path().join("builtin"),
        root.path().join("user"),
        root.path().join("scripts.json"),
    );
    let snapshot = checked_response_middlewares(&manager).unwrap();
    assert!(snapshot.is_empty());
    let package = manager.user_dir().join("response-fixture");
    std::fs::create_dir_all(package.join("gateway")).unwrap();
    std::fs::write(
        package.join("gateway/response.mjs"),
        "export default async function* (_, next) { yield* next(); }",
    )
    .unwrap();
    std::fs::write(
        package.join("codexpp.json"),
        json!({
            "id":"response-fixture", "name":"fixture",
            "gateway":{"response":[{"script":"gateway/response.mjs"}]}
        })
        .to_string(),
    )
    .unwrap();
    assert_eq!(checked_response_middlewares(&manager).unwrap().len(), 1);
    with_codex_response_plan_for_tests(Some(snapshot), async {
        // 不启动 Node，也不读取默认正式配置；实际 filter 必须使用请求开始时的空快照。
        assert!(ResponseFilter::start("/v1/responses", None).await.is_none());
        assert!(claim_response_passthrough(
            "POST",
            CODEX_PATH,
            &forwarded_headers()
        ));
    })
    .await;
}

#[test]
fn unreadable_or_malformed_configuration_is_not_an_empty_snapshot() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("scripts.json");
    let manager =
        UserScriptManager::new(root.path().join("builtin"), root.path().join("user"), &path);
    for bytes in ["{", "null", "[]"] {
        std::fs::write(&path, bytes).unwrap();
        assert!(checked_response_middlewares(&manager).is_err());
    }
    let unreadable = UserScriptManager::new(
        root.path().join("builtin"),
        root.path().join("user"),
        root.path(),
    );
    assert!(checked_response_middlewares(&unreadable).is_err());
}
