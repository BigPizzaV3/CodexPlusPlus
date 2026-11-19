use super::*;

async fn reject_browser_origins(gateway: bool, authenticated: bool) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let relay = profile(listener.local_addr().unwrap());
    let fixture = Fixture::new(relay.clone());
    if gateway {
        fixture.gateway(relay);
    }
    let origins = [
        vec!["https://evil.example"],
        vec!["null"],
        vec!["http://localhost"],
        vec!["", "https://evil.example"],
    ];
    let (url, task) = helper(origins.len()).await;
    let path = if gateway {
        "/gw/codex/v1/responses"
    } else {
        "/v1/responses"
    };
    for values in origins {
        let mut request = format!("{url}{path}").into_client_request().unwrap();
        if authenticated {
            request
                .headers_mut()
                .insert("authorization", "Bearer local-test".parse().unwrap());
        }
        for origin in &values {
            request
                .headers_mut()
                .append("origin", origin.parse().unwrap());
        }
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            tokio_tungstenite::connect_async(request),
        )
        .await
        .unwrap();
        match result {
            Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
                assert_eq!(response.status(), 403, "Origin {values:?} on {path}");
                assert!(!response.headers().contains_key("sec-websocket-accept"));
            }
            Ok((mut ws, response)) => {
                ws.close(None).await.unwrap();
                panic!(
                    "Browser Origin {values:?} upgraded on {path}: {}",
                    response.status()
                );
            }
            Err(error) => panic!("Expected HTTP 403, got {error}"),
        }
    }
    finish(task).await;
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock,
        "Rejected upgrade contacted the upstream"
    );
}

#[tokio::test]
async fn helper_ws_rejects_browser_origins_before_upgrade() {
    if isolated("origin::helper_ws_rejects_browser_origins_before_upgrade").await {
        return;
    }
    reject_browser_origins(false, false).await;
}

#[tokio::test]
async fn gateway_ws_rejects_browser_origins_with_valid_local_token() {
    if isolated("origin::gateway_ws_rejects_browser_origins_with_valid_local_token").await {
        return;
    }
    reject_browser_origins(true, true).await;
}

#[tokio::test]
async fn gateway_ws_rejects_browser_origins_before_local_authentication() {
    if isolated("origin::gateway_ws_rejects_browser_origins_before_local_authentication").await {
        return;
    }
    reject_browser_origins(true, false).await;
}

#[tokio::test]
async fn helper_and_gateway_ws_allow_absent_or_empty_origin() {
    if isolated("origin::helper_and_gateway_ws_allow_absent_or_empty_origin").await {
        return;
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let relay = profile(listener.local_addr().unwrap());
    let fixture = Fixture::new(relay.clone());
    fixture.gateway(relay);
    let (url, task) = helper(6).await;
    for path in ["/v1/responses", "/gw/codex/v1/responses"] {
        for origin in [None, Some(""), Some(" \t ")] {
            let mut headers = vec![("authorization", "Bearer local-test".into())];
            if let Some(origin) = origin {
                headers.push(("origin", origin.into()));
            }
            let mut ws = client(&format!("{url}{path}"), &headers).await;
            ws.close(None).await.unwrap();
        }
    }
    finish(task).await;
}
