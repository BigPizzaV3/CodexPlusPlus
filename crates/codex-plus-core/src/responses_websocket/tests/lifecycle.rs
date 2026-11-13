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
