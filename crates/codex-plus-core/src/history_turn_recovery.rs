//! 会话索引检查完成后的前端补载阶段；不创建入口、定时任务或持久请求。

use std::time::Duration;

use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RecoveryReport {
    status: String,
    loaded_pages: usize,
    #[serde(default)]
    metadata_pages: usize,
}

impl RecoveryReport {
    fn is_valid(&self) -> bool {
        matches!(
            self.status.as_str(),
            "completed" | "partial" | "skipped" | "busy" | "unavailable"
        ) && self.loaded_pages <= 128
            && self.metadata_pages <= 64
            && self.loaded_pages + self.metadata_pages <= 128
    }
}

fn recovery_expression(request_id: &str) -> String {
    let id = serde_json::to_string(request_id).expect("request id is serializable");
    format!(
        "(async()=>JSON.stringify(await window.__codexPlusHistoryTurnRecoveryRuntime?.run({id})\
         ||{{status:\"unavailable\",loadedPages:0}}))()"
    )
}

async fn recover(debug_port: u16) -> anyhow::Result<RecoveryReport> {
    let targets = crate::cdp::list_targets(debug_port).await?;
    let targets = crate::cdp::select_extension_cdp_targets(&targets, None, Some("main"))?;
    let [target] = targets.as_slice() else {
        anyhow::bail!("expected one main Codex page");
    };
    let websocket = target
        .web_socket_debugger_url
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("main page has no websocket"))?;
    crate::cdp::validate_cdp_websocket_url(websocket, debug_port)?;
    let request_id = uuid::Uuid::new_v4().to_string();
    let response = crate::bridge::evaluate_script_with_await_promise(
        websocket,
        &recovery_expression(&request_id),
        true,
    )
    .await?;
    let raw = response
        .pointer("/result/result/value")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("missing frontend recovery result"))?;
    let report: RecoveryReport = serde_json::from_str(raw)?;
    if !report.is_valid() {
        anyhow::bail!("invalid frontend recovery result");
    }
    Ok(report)
}

/// 只接在原有成功且未中止的索引检查后；失败不反转已提交的数据库修复。
/// 超时可能意味着原生读取仍在进行，因此不自动重发，也不排队等待页面出现。
pub async fn after_index_repair(debug_port: Option<u16>, aborted: bool) {
    let Some(debug_port) = debug_port.filter(|port| *port != 0 && !aborted) else {
        return;
    };
    let report = tokio::time::timeout(Duration::from_secs(20), recover(debug_port)).await;
    let details = match report {
        Ok(Ok(report)) => json!({
            "status": report.status,
            "loaded_pages": report.loaded_pages,
            "metadata_pages": report.metadata_pages,
        }),
        _ => json!({"status": "unconfirmed"}),
    };
    let _ = crate::diagnostic_log::append_diagnostic_log("session_index_repair.frontend", details);
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn report_bounds_metadata_and_item_pages_together_without_overflow() {
        for (items, metadata, expected) in [
            (128, 0, true),
            (64, 64, true),
            (65, 64, false),
            (0, 65, false),
            (usize::MAX, 1, false),
        ] {
            assert_eq!(
                RecoveryReport {
                    status: "partial".into(),
                    loaded_pages: items,
                    metadata_pages: metadata,
                }
                .is_valid(),
                expected
            );
        }
        let legacy: RecoveryReport =
            serde_json::from_value(json!({"status":"completed","loadedPages":3})).unwrap();
        assert!(legacy.is_valid());
        assert_eq!(legacy.metadata_pages, 0);
        let current: RecoveryReport =
            serde_json::from_value(json!({"status":"completed","loadedPages":3,"metadataPages":2}))
                .unwrap();
        assert!(current.is_valid());
        assert_eq!(current.metadata_pages, 2);
    }

    #[test]
    fn notification_only_calls_existing_runtime_with_a_quoted_request_id() {
        let request_id = uuid::Uuid::new_v4().to_string();
        let expression = recovery_expression(&request_id);
        assert!(expression.contains(&format!(".run(\"{request_id}\")")));
        assert!(expression.contains("JSON.stringify(await window."));
        assert!(!expression.contains("setTimeout"));
        assert!(!expression.contains("loadTurnItems"));
        let escaped = recovery_expression("\";throw Error('fixture')");
        assert!(escaped.contains(".run(\"\\\";throw Error('fixture')\")"));
    }

    #[tokio::test]
    async fn absent_port_and_aborted_repairs_do_not_contact_the_page() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        after_index_repair(None, false).await;
        after_index_repair(Some(0), false).await;
        after_index_repair(Some(port), true).await;
        assert!(
            tokio::time::timeout(Duration::from_millis(25), listener.accept())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn notification_selects_main_page_and_waits_for_its_bounded_result() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut http, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0; 1];
                http.read_exact(&mut byte).await.unwrap();
                request.push(byte[0]);
                assert!(request.len() < 8192);
            }
            assert!(request.starts_with(b"GET /json "));
            let body = json!([
                {
                    "id": "overlay", "type": "page", "title": "Codex",
                    "url": "app://-/index.html?initialRoute=/avatar-overlay",
                    "webSocketDebuggerUrl": format!("ws://127.0.0.1:{port}/devtools/page/overlay"),
                },
                {
                    "id": "main", "type": "page", "title": "Codex",
                    "url": "app://-/index.html",
                    "webSocketDebuggerUrl": format!("ws://127.0.0.1:{port}/devtools/page/main"),
                },
            ])
            .to_string();
            http.write_all(format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len(),
            ).as_bytes()).await.unwrap();
            drop(http);
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let message = socket.next().await.unwrap().unwrap();
            let command: serde_json::Value =
                serde_json::from_str(message.to_text().unwrap()).unwrap();
            assert_eq!(command["method"], "Runtime.evaluate");
            assert_eq!(command["params"]["awaitPromise"], true);
            assert!(
                command["params"]["expression"]
                    .as_str()
                    .unwrap()
                    .contains("window.__codexPlusHistoryTurnRecoveryRuntime?.run(")
            );
            let response = json!({
                "id": command["id"],
                "result": {"result": {
                    "type": "string",
                    "value": json!({"status":"completed","loadedPages":3,"metadataPages":2}).to_string(),
                }},
            });
            socket
                .send(tokio_tungstenite::tungstenite::Message::Text(
                    response.to_string().into(),
                ))
                .await
                .unwrap();
        });
        let report = tokio::time::timeout(Duration::from_secs(3), recover(port))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(report.status, "completed");
        assert_eq!(report.loaded_pages, 3);
        assert_eq!(report.metadata_pages, 2);
        server.await.unwrap();
    }
}
