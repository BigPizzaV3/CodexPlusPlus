use super::*;

#[tokio::test]
async fn gateway_ws_temporary_key_failover_preserves_sticky_key() {
    if isolated("lifecycle::gateway_ws_temporary_key_failover_preserves_sticky_key").await {
        return;
    }
    use crate::gateway::key_pool::{self, KeyLimits};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut relay = profile(listener.local_addr().unwrap());
    relay["apiKeys"] = json!([{"id":"review-sticky-a","name":"A","apiKey":"sk-review-a"},{"id":"review-sticky-b","name":"B","apiKey":"sk-review-b"}]);
    relay["activeApiKeyId"] = json!("review-sticky-a");
    let fixture = Fixture::new(relay.clone());
    fixture.gateway(relay);
    let gateway = GatewayStore::default().load().unwrap();
    let plan = crate::gateway::route::resolve_plan(
        &gateway,
        &crate::tools::ToolId::Codex,
        "ws-model",
        Some("review-sticky-conversation"),
    )
    .unwrap();
    let key_a = plan
        .candidates
        .iter()
        .find(|candidate| candidate.key.key_id == "review-sticky-a")
        .unwrap()
        .key
        .clone();
    key_pool::record_success(&key_a, Some("review-sticky-conversation"));
    let held = key_pool::try_acquire(
        &key_a,
        KeyLimits {
            concurrency: Some(1),
            ..KeyLimits::default()
        },
        std::time::Instant::now(),
    )
    .unwrap();
    let upstream = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws =
            tokio_tungstenite::accept_hdr_async(socket, |request: &http::Request<()>, response| {
                assert_eq!(request.headers()["authorization"], "Bearer sk-review-b");
                Ok(response)
            })
            .await
            .unwrap();
        next(&mut ws).await;
        send(&mut ws, completed("review-temporary", "")).await;
        let _ = ws.next().await;
    });
    let (url, task) = helper(1).await;
    let mut ws = client(
        &format!("{url}/gw/codex/v1/responses"),
        &[("authorization", "Bearer local-test".into())],
    )
    .await;
    let mut event = create("");
    event["prompt_cache_key"] = json!("review-sticky-conversation");
    send(&mut ws, event).await;
    assert_eq!(next(&mut ws).await["type"], "response.completed");
    ws.close(None).await.unwrap();
    drop(ws);
    finish(task).await;
    finish(upstream).await;
    drop(held);
    let after = crate::gateway::route::resolve_plan(
        &gateway,
        &crate::tools::ToolId::Codex,
        "ws-model",
        Some("review-sticky-conversation"),
    )
    .unwrap();
    assert_eq!(
        after.candidates[0].key.key_id, "review-sticky-a",
        "Temporary concurrency failover overwrote the original sticky key"
    );
}

async fn pending_message_on_failure(lane: &str, connection_error: bool) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let fixture = Fixture::new(profile(listener.local_addr().unwrap()));
    let path = fixture.temp.path().join("settings.json");
    let mut settings: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    settings["codexAppThirdPartyReasoningCollapse"] = json!(true);
    std::fs::write(&path, settings.to_string()).unwrap();
    let upstream_lane = lane.to_string();
    let upstream = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();
        next(&mut ws).await;
        let mut created =
            json!({"type":"response.created","response":{"id":"partial","status":"in_progress"}});
        let mut done = json!({"type":"response.output_item.done","output_index":0,"item":{"id":"msg-partial","type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":"finished answer","annotations":[]}]}});
        if !upstream_lane.is_empty() {
            created["stream_id"] = json!(upstream_lane);
            done["stream_id"] = json!(upstream_lane);
        }
        send(&mut ws, created).await;
        send(&mut ws, done).await;
        if connection_error {
            send(&mut ws, json!({"type":"error","status":429,"error":{"code":"rate_limit_exceeded","message":"limited"}})).await;
            let _ = ws.next().await;
        } else {
            ws.close(None).await.unwrap();
        }
    });
    let (url, task) = helper(1).await;
    let mut ws = client(&format!("{url}/v1/responses"), &[]).await;
    send(&mut ws, create(lane)).await;
    let mut observed = Vec::new();
    loop {
        let event = next(&mut ws).await;
        let done = event["type"] == "error";
        observed.push(event);
        if done {
            break;
        }
    }
    ws.close(None).await.unwrap();
    drop(ws);
    finish(task).await;
    finish(upstream).await;
    assert_eq!(
        observed.len(),
        3,
        "Pending message must precede the failure: {observed:?}"
    );
    assert_eq!(observed[1]["type"], "response.output_item.done");
    assert_eq!(observed[1]["item"]["content"][0]["text"], "finished answer");
    assert!(
        observed[1]["item"].get("phase").is_none(),
        "Failure must not guess the final phase"
    );
    if !lane.is_empty() {
        assert!(observed.iter().all(|event| event["stream_id"] == lane));
    }
}

#[tokio::test]
async fn helper_ws_disconnect_flushes_pending_message() {
    if isolated("lifecycle::helper_ws_disconnect_flushes_pending_message").await {
        return;
    }
    pending_message_on_failure("", false).await;
}

#[tokio::test]
async fn helper_ws_connection_error_flushes_pending_named_message() {
    if isolated("lifecycle::helper_ws_connection_error_flushes_pending_named_message").await {
        return;
    }
    pending_message_on_failure("worker", true).await;
}

async fn continuation_after_auth_change(
    old_account: Option<&str>,
    new_account: Option<&str>,
) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1/responses", listener.local_addr().unwrap());
    let upstream = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws =
            tokio_tungstenite::accept_hdr_async(socket, |request: &http::Request<()>, response| {
                assert_eq!(request.headers()["authorization"], "Bearer old-token");
                Ok(response)
            })
            .await
            .unwrap();
        let _ = ws.next().await;
    });
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let registry = Arc::new(Mutex::new(Registry::default()));
    let context = RequestContext {
        registry: registry.clone(),
        previous: None,
    };
    let mut ready = REQUEST
        .scope(context, async {
            let builder = client
                .post(&url)
                .bearer_auth("old-token")
                .json(&json!({"model":"test"}));
            match prepare(
                "auth-test",
                builder,
                json!({"model":"test"}),
                Duration::from_secs(5),
                old_account,
            )
            .await
            .unwrap()
            {
                PreparedUpstream::Websocket(ready) => ready,
                _ => panic!("Initial upgrade was rejected"),
            }
        })
        .await;
    registry
        .lock()
        .unwrap()
        .connections
        .insert(ready.fingerprint.clone(), ready.connection.clone());
    ready.registration.take();
    let context = RequestContext {
        registry,
        previous: Some(Affinity {
            connection: ready.connection.clone(),
            relay: "auth-test".into(),
            model: "test".into(),
            key: None,
        }),
    };
    let result = REQUEST
        .scope(context, async {
            let builder = client
                .post(&url)
                .bearer_auth("new-token")
                .json(&json!({"model":"test"}));
            prepare(
                "auth-test",
                builder,
                json!({"model":"test","previous_response_id":"cached-id"}),
                Duration::from_secs(1),
                new_account,
            )
            .await
        })
        .await;
    let connection = ready.connection.clone();
    drop(ready);
    finish(upstream).await;
    match result? {
        PreparedUpstream::Websocket(reused) => {
            assert_eq!(reused.connection, connection);
            assert!(
                reused.socket.is_none(),
                "Continuation must reuse the live socket"
            );
            assert_eq!(reused.body["previous_response_id"], "cached-id");
            Ok(())
        }
        _ => panic!("Continuation was not a WebSocket"),
    }
}

#[tokio::test]
async fn refreshed_oauth_reuses_live_continuation_for_the_same_account() {
    continuation_after_auth_change(Some("account-a"), Some("account-a"))
        .await
        .unwrap();
}

#[tokio::test]
async fn changed_oauth_account_cannot_reuse_cached_continuation() {
    let error = continuation_after_auth_change(Some("account-a"), Some("account-b"))
        .await
        .unwrap_err();
    assert!(error.is::<Unsupported>());
}

#[tokio::test]
async fn changed_api_key_cannot_reuse_cached_continuation() {
    let error = continuation_after_auth_change(None, None)
        .await
        .unwrap_err();
    assert!(error.is::<Unsupported>());
}

#[tokio::test]
async fn unknown_oauth_account_keeps_credentials_in_connection_identity() {
    let error = continuation_after_auth_change(Some(""), Some(""))
        .await
        .unwrap_err();
    assert!(error.is::<Unsupported>());
}

#[tokio::test]
async fn gateway_ws_temporary_member_failover_preserves_group_sticky() {
    if isolated("lifecycle::gateway_ws_temporary_member_failover_preserves_group_sticky").await {
        return;
    }
    use crate::gateway::{
        key_pool::{self, KeyLimits},
        route,
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let relay_a = profile(listener.local_addr().unwrap());
    let mut relay_b = relay_a.clone();
    relay_b["id"] = json!("group-b");
    relay_b["apiKey"] = json!("sk-group-b");
    relay_b["apiKeys"] = json!([{"id":"group-b-key","name":"B","apiKey":"sk-group-b"}]);
    relay_b["activeApiKeyId"] = json!("group-b-key");
    let _fixture = Fixture::new(relay_a.clone());
    let gateway: GatewaySettings = serde_json::from_value(json!({
        "version":3,
        "bindings":{"codex":{"enabled":true,"model":"group/ws-sticky-group","token":"local-test"}},
        "providers":[{"relay":relay_a,"keyConcurrency":1},{"relay":relay_b,"keyConcurrency":1}],
        "groups":[{"id":"ws-sticky-group","name":"Sticky group","mode":"rotate","sticky":"session","members":[{"model":"ws-test/ws-model"},{"model":"group-b/ws-model"}]}],
        "autoModels":false
    })).unwrap();
    GatewayStore::default().save(&gateway).unwrap();
    let plan = route::resolve_plan(
        &gateway,
        &crate::tools::ToolId::Codex,
        "group/ws-sticky-group",
        Some("group-conversation"),
    )
    .unwrap();
    let candidate_a = plan
        .candidates
        .iter()
        .find(|candidate| candidate.relay.id == "ws-test")
        .unwrap();
    route::remember_answer(&plan, candidate_a);
    let held = key_pool::try_acquire(
        &candidate_a.key,
        KeyLimits {
            concurrency: Some(1),
            ..KeyLimits::default()
        },
        std::time::Instant::now(),
    )
    .unwrap();
    let upstream = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws =
            tokio_tungstenite::accept_hdr_async(socket, |request: &http::Request<()>, response| {
                assert_eq!(request.headers()["authorization"], "Bearer sk-group-b");
                Ok(response)
            })
            .await
            .unwrap();
        next(&mut ws).await;
        send(&mut ws, completed("group-temporary", "")).await;
        let _ = ws.next().await;
    });
    let (url, task) = helper(1).await;
    let mut ws = client(
        &format!("{url}/gw/codex/v1/responses"),
        &[("authorization", "Bearer local-test".into())],
    )
    .await;
    let mut event = create("");
    event["model"] = json!("group/ws-sticky-group");
    event["prompt_cache_key"] = json!("group-conversation");
    send(&mut ws, event).await;
    assert_eq!(next(&mut ws).await["type"], "response.completed");
    ws.close(None).await.unwrap();
    drop(ws);
    finish(task).await;
    finish(upstream).await;
    drop(held);
    let after = route::resolve_plan(
        &gateway,
        &crate::tools::ToolId::Codex,
        "group/ws-sticky-group",
        Some("group-conversation"),
    )
    .unwrap();
    assert_eq!(
        after.candidates[0].relay.id, "ws-test",
        "Temporary member failover must preserve group sticky"
    );
}

fn oauth_auth(subject: Option<&str>, issuer: &str, access: &str) -> String {
    use base64::Engine;
    let claims = match subject {
        Some(subject) => json!({"sub":subject,"iss":issuer}),
        None => json!({"iss":issuer}),
    };
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(claims.to_string());
    json!({"auth_mode":"chatgpt","codex_plus_gateway":true,"tokens":{
        "account_id":"shared-workspace","access_token":access,"refresh_token":"test-refresh",
        "id_token":format!("e30.{payload}.test")
    }})
    .to_string()
}

async fn official_identity_continuation(subject: Option<&str>, issuer: &str, allowed: bool) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    // 每个 fixture 由独立子进程运行，环境覆盖不与其他测试共享。
    unsafe { std::env::set_var("CODEX_PLUS_OFFICIAL_BASE_URL", &base) };
    let mut relay = profile(listener.local_addr().unwrap());
    relay["relayMode"] = json!("official");
    relay["upstreamBaseUrl"] = json!("");
    relay["apiKey"] = json!("");
    relay["apiKeys"] = json!([]);
    relay["activeApiKeyId"] = json!("");
    relay["authContents"] = json!(oauth_auth(
        Some("alice"),
        "https://auth.openai.com",
        "old-token"
    ));
    let _fixture = Fixture::new(relay);
    let upstream = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws =
            tokio_tungstenite::accept_hdr_async(socket, |request: &http::Request<()>, response| {
                assert_eq!(request.headers()["authorization"], "Bearer old-token");
                assert_eq!(request.headers()["chatgpt-account-id"], "shared-workspace");
                Ok(response)
            })
            .await
            .unwrap();
        next(&mut ws).await;
        send(&mut ws, completed("official-cached", "")).await;
        while let Some(Ok(Message::Text(text))) = ws.next().await {
            let event: Value = serde_json::from_str(&text).unwrap();
            assert_eq!(event["previous_response_id"], "official-cached");
            send(&mut ws, completed("official-continuation", "")).await;
        }
    });
    let (url, task) = helper(1).await;
    let mut ws = client(&format!("{url}/v1/responses"), &[]).await;
    send(&mut ws, create("")).await;
    assert_eq!(next(&mut ws).await["type"], "response.completed");
    let store = crate::settings::SettingsStore::default();
    let mut settings = store.load().unwrap();
    settings.relay_profiles[0].auth_contents = oauth_auth(subject, issuer, "new-token");
    store.save(&settings).unwrap();
    let mut continuation = create("");
    continuation["previous_response_id"] = json!("official-cached");
    send(&mut ws, continuation).await;
    let result = next(&mut ws).await;
    assert_eq!(
        result["type"],
        if allowed {
            "response.completed"
        } else {
            "error"
        },
        "{result}"
    );
    ws.close(None).await.unwrap();
    drop(ws);
    finish(task).await;
    finish(upstream).await;
    unsafe { std::env::remove_var("CODEX_PLUS_OFFICIAL_BASE_URL") };
}

#[tokio::test]
async fn helper_ws_oauth_refresh_preserves_id_token_identity() {
    if isolated("lifecycle::helper_ws_oauth_refresh_preserves_id_token_identity").await {
        return;
    }
    official_identity_continuation(Some("alice"), "https://auth.openai.com", true).await;
}

#[tokio::test]
async fn helper_ws_oauth_different_users_in_one_workspace_are_isolated() {
    if isolated("lifecycle::helper_ws_oauth_different_users_in_one_workspace_are_isolated").await {
        return;
    }
    official_identity_continuation(Some("bob"), "https://auth.openai.com", false).await;
}

#[tokio::test]
async fn helper_ws_oauth_changed_issuer_is_isolated() {
    if isolated("lifecycle::helper_ws_oauth_changed_issuer_is_isolated").await {
        return;
    }
    official_identity_continuation(Some("alice"), "https://other.example", false).await;
}

#[tokio::test]
async fn helper_ws_oauth_missing_subject_keeps_token_isolation() {
    if isolated("lifecycle::helper_ws_oauth_missing_subject_keeps_token_isolation").await {
        return;
    }
    official_identity_continuation(None, "https://auth.openai.com", false).await;
}
