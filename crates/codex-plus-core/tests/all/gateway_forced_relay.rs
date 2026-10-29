//! 网关用的「强制上游」钩子：绕过当前供应商、聚合与模型映射，只发往指定供应商。

use codex_plus_core::protocol_proxy::{
    ForcedRelay, ProxySessionHeaders, open_responses_proxy_request_for_relay,
};
use codex_plus_core::settings::{BackendSettings, RelayMode, RelayProfile, RelayProtocol};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn capture_once(listener: tokio::net::TcpListener, status: u16, body: String) -> Value {
    let (mut stream, _) = listener.accept().await.unwrap();
    let mut buffer = Vec::new();
    let mut chunk = [0; 4096];
    let (end, length) = loop {
        let read = stream.read(&mut chunk).await.unwrap();
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buffer[..end]).to_ascii_lowercase();
            let length = head
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .and_then(|value| value.trim().parse::<usize>().ok())
                .unwrap_or(0);
            break (end + 4, length);
        }
    };
    while buffer.len() < end + length {
        let read = stream.read(&mut chunk).await.unwrap();
        buffer.extend_from_slice(&chunk[..read]);
    }
    let response = format!(
        "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await.unwrap();
    serde_json::from_slice(&buffer[end..end + length]).unwrap()
}

fn relay(id: &str, base_url: String) -> RelayProfile {
    RelayProfile {
        id: id.into(),
        name: id.into(),
        relay_mode: RelayMode::PureApi,
        protocol: RelayProtocol::ChatCompletions,
        base_url,
        api_key: format!("sk-{id}"),
        model_list: "listed-model".into(),
        ..RelayProfile::default()
    }
}

fn chat_completion() -> String {
    json!({
        "id": "c", "object": "chat.completion", "model": "x",
        "choices": [{ "index": 0, "message": { "role": "assistant", "content": "ok" }, "finish_reason": "stop" }]
    })
    .to_string()
}

#[tokio::test]
async fn forced_relay_ignores_active_relay_and_applies_model_override() {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    // 当前供应商指向一个不可达地址：强制上游不能读它。
    let settings = BackendSettings {
        active_relay_id: "active".into(),
        relay_profiles: vec![relay("active", "http://127.0.0.1:9/v1".into())],
        protocol_proxy_map_unlisted_models: true,
        ..BackendSettings::default()
    };
    let capture = tokio::spawn(capture_once(listener, 200, chat_completion()));
    let body = json!({
        "model": "client-model-not-listed",
        "input": [{ "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "hi" }] }]
    })
    .to_string();
    let response = open_responses_proxy_request_for_relay(
        &body,
        settings,
        ForcedRelay {
            relay: relay("forced", base),
            upstream_model: Some("forced-model".into()),
            auto_retry: false,
        },
        None,
        "/responses",
        ProxySessionHeaders::default(),
        None,
    )
    .await
    .unwrap();
    assert_eq!(response.status_code, 200);
    let upstream = capture.await.unwrap();
    assert_eq!(upstream["model"], "forced-model");
}

#[tokio::test]
async fn forced_relay_without_override_keeps_client_model_and_skips_unlisted_mapping() {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let forced = relay("forced", base);
    let settings = BackendSettings {
        active_relay_id: "forced".into(),
        relay_profiles: vec![forced.clone()],
        protocol_proxy_map_unlisted_models: true,
        ..BackendSettings::default()
    };
    let capture = tokio::spawn(capture_once(
        listener,
        503,
        "{\"error\":{\"message\":\"busy\"}}".into(),
    ));
    let body = json!({ "model": "unlisted", "input": "hi" }).to_string();
    let response = open_responses_proxy_request_for_relay(
        &body,
        settings,
        ForcedRelay {
            relay: forced,
            upstream_model: None,
            auto_retry: false,
        },
        None,
        "/responses",
        ProxySessionHeaders::default(),
        None,
    )
    .await
    .unwrap();
    // 关掉自动重试时 503 直接交回，由网关自己决定是否换上游。
    assert_eq!(response.status_code, 503);
    assert_eq!(capture.await.unwrap()["model"], "unlisted");
}
