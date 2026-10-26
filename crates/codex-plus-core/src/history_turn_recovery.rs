//! 会话索引检查完成后的前端补载阶段；不创建入口、定时任务或持久请求。

use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RecoveryReport {
    status: String,
    loaded_pages: usize,
    #[serde(default)]
    metadata_pages: usize,
    #[serde(default)]
    reason: Option<String>,
}

impl RecoveryReport {
    fn is_valid(&self) -> bool {
        matches!(
            self.status.as_str(),
            "completed" | "partial" | "skipped" | "busy" | "unavailable"
        ) && self.loaded_pages <= 128
            && self.metadata_pages <= 64
            && self.loaded_pages + self.metadata_pages <= 128
            && self.reason.as_deref().is_none_or(valid_reason)
    }
}

fn valid_reason(reason: &str) -> bool {
    matches!(
        reason,
        "disabled"
            | "disposed"
            | "duplicate_request"
            | "recovery_busy"
            | "window_hidden"
            | "unsupported_window"
            | "no_local_thread"
            | "thread_not_mounted"
            | "native_api_unavailable"
            | "runtime_unavailable"
            | "native_state_unavailable"
            | "thread_busy"
            | "context_changed"
            | "budget_exhausted"
            | "native_read_failed"
            | "incomplete"
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendRecoveryReport {
    pub checked_at_ms: i64,
    request_id: String,
    pub status: String,
    pub reason: Option<String>,
    pub loaded_pages: usize,
    pub metadata_pages: usize,
    updated_at_ms: i64,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

fn report_path(home: &Path) -> PathBuf {
    home.join("session-index-repair/frontend-report.json")
}

/// 仅合并同一轮索引检查的结果；崩溃或重启后不把遗留 pending 当成仍在运行。
pub fn load_report(home: &Path, checked_at_ms: i64) -> Option<FrontendRecoveryReport> {
    let bytes = std::fs::read(report_path(home)).ok()?;
    let mut report: FrontendRecoveryReport = serde_json::from_slice(&bytes).ok()?;
    if report.checked_at_ms != checked_at_ms {
        return None;
    }
    if report.status == "pending" && now_ms().saturating_sub(report.updated_at_ms) > 30_000 {
        report.status = "unavailable".into();
        report.reason = Some("background_interrupted".into());
    }
    Some(report)
}

fn save_report(home: &Path, report: &FrontendRecoveryReport, initial: bool) -> anyhow::Result<()> {
    let path = report_path(home);
    let dir = path.parent().expect("report has parent");
    std::fs::create_dir_all(dir)?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(dir.join("frontend-report.lock"))?;
    lock.lock_exclusive()?;
    let result = (|| {
        if let Ok(bytes) = std::fs::read(&path) {
            if let Ok(previous) = serde_json::from_slice::<FrontendRecoveryReport>(&bytes) {
                // 后完成的旧请求不能覆盖新检查；同毫秒检查也按请求身份隔离。
                if previous.checked_at_ms > report.checked_at_ms
                    || (!initial && previous.request_id != report.request_id)
                {
                    return Ok(());
                }
            }
        }
        crate::settings::atomic_write(&path, &serde_json::to_vec(report)?)
    })();
    let _ = FileExt::unlock(&lock);
    result
}

fn recovery_expression(request_id: &str) -> String {
    let id = serde_json::to_string(request_id).expect("request id is serializable");
    format!(
        "(async()=>JSON.stringify(await window.__codexPlusHistoryTurnRecoveryRuntime?.run({id})\
         ||{{status:\"unavailable\",reason:\"runtime_unavailable\",loadedPages:0}}))()"
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

/// 数据库修复完成后只安排后台补载，不延迟返回其报告或延长数据库事务生命周期。
/// 超时可能意味着原生读取仍在进行，因此不自动重发，也不排队等待页面出现。
pub fn after_index_repair(debug_port: Option<u16>, aborted: bool, checked_at_ms: i64) {
    let _ = spawn_after_index_repair(
        debug_port,
        aborted,
        crate::settings::SettingsStore::default(),
        crate::relay_config::default_codex_home_dir(),
        checked_at_ms,
    );
}

fn spawn_after_index_repair(
    debug_port: Option<u16>,
    aborted: bool,
    store: crate::settings::SettingsStore,
    home: PathBuf,
    checked_at_ms: i64,
) -> Option<tokio::task::JoinHandle<()>> {
    Some(tokio::spawn(async move {
        let mut status = FrontendRecoveryReport {
            checked_at_ms,
            request_id: uuid::Uuid::new_v4().to_string(),
            status: "pending".into(),
            reason: None,
            loaded_pages: 0,
            metadata_pages: 0,
            updated_at_ms: now_ms(),
        };
        let initial = status.clone();
        let initial_home = home.clone();
        if !matches!(
            tokio::task::spawn_blocking(move || save_report(&initial_home, &initial, true)).await,
            Ok(Ok(()))
        ) {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "session_index_repair.frontend",
                json!({"status":"unavailable","reason":"report_write_failed"}),
            );
        }
        // 设置缺失、读取失败或总增强关闭时均不访问 CDP，也不触碰原生页面。
        let enabled = tokio::task::spawn_blocking(move || store.load())
            .await
            .ok()
            .and_then(Result::ok)
            .is_some_and(|settings| {
                settings.enhancements_enabled && settings.codex_app_history_turn_recovery
            });
        let skip_reason = if !enabled {
            Some("disabled_or_settings_unavailable")
        } else if aborted {
            Some("index_repair_aborted")
        } else if debug_port.is_none_or(|port| port == 0) {
            Some("debug_port_unavailable")
        } else {
            None
        };
        if let Some(reason) = skip_reason {
            status.status = "skipped".into();
            status.reason = Some(reason.into());
        } else {
            match tokio::time::timeout(Duration::from_secs(20), recover(debug_port.unwrap())).await
            {
                Ok(Ok(report)) => {
                    let busy = report.status == "busy";
                    status.status = if busy {
                        "skipped".into()
                    } else {
                        report.status
                    };
                    status.reason = report
                        .reason
                        .or_else(|| busy.then(|| "recovery_busy".into()));
                    status.loaded_pages = report.loaded_pages;
                    status.metadata_pages = report.metadata_pages;
                }
                result => {
                    status.status = "unavailable".into();
                    status.reason = Some(
                        if result.is_err() {
                            "frontend_timeout"
                        } else {
                            "frontend_unavailable"
                        }
                        .into(),
                    );
                }
            }
        }
        status.updated_at_ms = now_ms();
        let details = json!({
            "status": status.status,
            "reason": status.reason,
            "loaded_pages": status.loaded_pages,
            "metadata_pages": status.metadata_pages,
        });
        let _ =
            crate::diagnostic_log::append_diagnostic_log("session_index_repair.frontend", details);
        if !matches!(
            tokio::task::spawn_blocking(move || save_report(&home, &status, false)).await,
            Ok(Ok(()))
        ) {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "session_index_repair.frontend",
                json!({"status":"unavailable","reason":"report_write_failed"}),
            );
        }
    }))
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
                    reason: None,
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
        let mut unexpected: RecoveryReport = serde_json::from_value(json!({
            "status":"skipped", "loadedPages":0, "reason":"private native exception",
        }))
        .unwrap();
        assert!(!unexpected.is_valid());
        unexpected.reason = Some("window_hidden".into());
        assert!(unexpected.is_valid());
    }

    fn saved_report(checked_at_ms: i64, request_id: &str) -> FrontendRecoveryReport {
        FrontendRecoveryReport {
            checked_at_ms,
            request_id: request_id.into(),
            status: "pending".into(),
            reason: None,
            loaded_pages: 0,
            metadata_pages: 0,
            updated_at_ms: now_ms(),
        }
    }

    #[test]
    fn latest_report_is_bounded_and_older_completions_cannot_replace_it() {
        let temp = tempfile::tempdir().unwrap();
        let old = saved_report(1, "old");
        let latest = saved_report(2, "latest");
        save_report(temp.path(), &old, true).unwrap();
        save_report(temp.path(), &latest, true).unwrap();
        save_report(temp.path(), &old, false).unwrap();
        save_report(temp.path(), &old, true).unwrap();
        assert!(load_report(temp.path(), 1).is_none());
        assert_eq!(load_report(temp.path(), 2).unwrap().request_id, "latest");
        let replacement = saved_report(2, "replacement");
        save_report(temp.path(), &replacement, true).unwrap();
        save_report(temp.path(), &latest, false).unwrap();
        assert_eq!(
            load_report(temp.path(), 2).unwrap().request_id,
            "replacement"
        );
        assert_eq!(
            std::fs::read_dir(temp.path().join("session-index-repair"))
                .unwrap()
                .count(),
            2
        );
    }

    #[test]
    fn completed_reports_round_trip_and_abandoned_pending_reports_expire_without_writes() {
        let temp = tempfile::tempdir().unwrap();
        let mut report = saved_report(1, "request");
        save_report(temp.path(), &report, true).unwrap();
        for status in ["completed", "partial", "unavailable", "skipped"] {
            report.status = status.into();
            report.reason = Some("incomplete".into());
            report.loaded_pages = 3;
            report.metadata_pages = 2;
            save_report(temp.path(), &report, false).unwrap();
            let loaded = load_report(temp.path(), 1).unwrap();
            assert_eq!(loaded.status, status);
            assert_eq!(loaded.loaded_pages, 3);
            assert_eq!(loaded.metadata_pages, 2);
        }
        report.status = "pending".into();
        report.updated_at_ms = now_ms() - 31_000;
        save_report(temp.path(), &report, false).unwrap();
        let before = std::fs::read(report_path(temp.path())).unwrap();
        let loaded = load_report(temp.path(), 1).unwrap();
        assert_eq!(loaded.status, "unavailable");
        assert_eq!(loaded.reason.as_deref(), Some("background_interrupted"));
        assert_eq!(std::fs::read(report_path(temp.path())).unwrap(), before);
        assert!(load_report(temp.path(), 2).is_none());
    }

    #[test]
    fn notification_only_calls_existing_runtime_with_a_quoted_request_id() {
        let request_id = uuid::Uuid::new_v4().to_string();
        let expression = recovery_expression(&request_id);
        assert!(expression.contains(&format!(".run(\"{request_id}\")")));
        assert!(expression.contains("JSON.stringify(await window."));
        assert!(expression.contains("reason:\"runtime_unavailable\""));
        assert!(!expression.contains("setTimeout"));
        assert!(!expression.contains("loadTurnItems"));
        let escaped = recovery_expression("\";throw Error('fixture')");
        assert!(escaped.contains(".run(\"\\\";throw Error('fixture')\")"));
    }

    #[tokio::test]
    async fn absent_port_and_aborted_repairs_do_not_contact_the_page() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings.json");
        std::fs::write(&path, r#"{"codexAppHistoryTurnRecovery":true}"#).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        for (index, (port, aborted, reason)) in [
            (None, false, "debug_port_unavailable"),
            (Some(0), false, "debug_port_unavailable"),
            (Some(port), true, "index_repair_aborted"),
        ]
        .into_iter()
        .enumerate()
        {
            let checked_at_ms = index as i64 + 1;
            spawn_after_index_repair(
                port,
                aborted,
                crate::settings::SettingsStore::new(path.clone()),
                temp.path().to_path_buf(),
                checked_at_ms,
            )
            .unwrap()
            .await
            .unwrap();
            let report = load_report(temp.path(), checked_at_ms).unwrap();
            assert_eq!(report.status, "skipped");
            assert_eq!(report.reason.as_deref(), Some(reason));
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(25), listener.accept())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn disabled_missing_or_unreadable_settings_do_not_contact_the_page() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings.json");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        for contents in [
            None,
            Some("{}"),
            Some(r#"{"codexAppHistoryTurnRecovery":false}"#),
            Some(r#"{"codexAppHistoryTurnRecovery":true,"enhancementsEnabled":false}"#),
            Some("{invalid"),
        ] {
            if let Some(contents) = contents {
                std::fs::write(&path, contents).unwrap();
            }
            spawn_after_index_repair(
                Some(port),
                false,
                crate::settings::SettingsStore::new(path.clone()),
                temp.path().to_path_buf(),
                1,
            )
            .unwrap()
            .await
            .unwrap();
            let report = load_report(temp.path(), 1).unwrap();
            assert_eq!(report.status, "skipped");
            assert_eq!(
                report.reason.as_deref(),
                Some("disabled_or_settings_unavailable")
            );
            assert!(
                tokio::time::timeout(Duration::from_millis(25), listener.accept())
                    .await
                    .is_err()
            );
        }
    }

    #[tokio::test]
    async fn scheduling_returns_without_waiting_for_the_frontend_result() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings.json");
        std::fs::write(&path, r#"{"codexAppHistoryTurnRecovery":true}"#).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (connected, pending) = tokio::sync::oneshot::channel();
        let (release, wait) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            let (_socket, _) = listener.accept().await.unwrap();
            connected.send(()).unwrap();
            let _ = wait.await;
        });
        let task = spawn_after_index_repair(
            Some(port),
            false,
            crate::settings::SettingsStore::new(path),
            temp.path().to_path_buf(),
            1,
        )
        .unwrap();
        tokio::time::timeout(Duration::from_secs(3), pending)
            .await
            .unwrap()
            .unwrap();
        // 触发方已拿回控制权；前端没有响应也不阻塞数据库报告返回。
        assert!(!task.is_finished());
        assert_eq!(load_report(temp.path(), 1).unwrap().status, "pending");
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        release.send(()).unwrap();
        server.await.unwrap();
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
