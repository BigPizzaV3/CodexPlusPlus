use std::time::Duration;

use anyhow::{Context, bail};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;

pub async fn evaluate_script_with_await_promise_timeout(
    websocket_url: &str,
    script: &str,
    await_promise: bool,
    timeout: Duration,
) -> anyhow::Result<Value> {
    let future = async {
        let (mut socket, _) = tokio_tungstenite::connect_async(websocket_url)
            .await
            .context("failed to connect to CDP WebSocket")?;
        socket
            .send(Message::Text(
                json!({
                    "id": 1,
                    "method": "Runtime.evaluate",
                    "params": {
                        "expression": script,
                        "returnByValue": true,
                        "awaitPromise": await_promise,
                    }
                })
                .to_string()
                .into(),
            ))
            .await?;
        while let Some(message) = socket.next().await {
            match message? {
                Message::Text(text) => {
                    let value: Value = serde_json::from_str(&text)?;
                    if value["id"] != 1 {
                        continue;
                    }
                    if value["error"].is_object() {
                        bail!("CDP Runtime.evaluate returned an error");
                    }
                    let _ = socket.close(None).await;
                    return Ok(value);
                }
                Message::Ping(bytes) => socket.send(Message::Pong(bytes)).await?,
                Message::Close(_) => break,
                _ => {}
            }
        }
        bail!("CDP WebSocket closed before Runtime.evaluate completed")
    };
    tokio::time::timeout(timeout, future)
        .await
        .context("timed out waiting for CDP Runtime.evaluate")?
}
