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
