use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use serde_json::{Value, json};

// A create command performs thread/start and turn/start serially in the renderer.
// Keep the outer CDP budget larger than the sum of the renderer request budgets.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(75);
const TARGET_RETRY_COUNT: usize = 4;
const TARGET_RETRY_DELAY: Duration = Duration::from_millis(250);

pub(super) async fn execute(request: Value) -> anyhow::Result<Value> {
    let command_type = request["commandType"]
        .as_str()
        .unwrap_or("unknown")
        .to_owned();
    let started = Instant::now();
    let result = execute_inner(request).await;
    let elapsed_ms = started.elapsed().as_millis();
    match &result {
        Ok(value) => {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "remote_mobile.command_source_completed",
                json!({
                    "command_type": &command_type,
                    "status": value["status"].as_str().unwrap_or("unknown"),
                    "error_code": value["errorCode"].as_str().unwrap_or(""),
                    "phase": renderer_failure_phase(&value["message"]),
                    "reason": renderer_failure_reason(&value["message"]),
                    "elapsed_ms": elapsed_ms,
                }),
            );
        }
        Err(error) => {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "remote_mobile.command_source_failed",
                json!({
                    "command_type": &command_type,
                    "reason": command_source_failure_reason(error),
                    "elapsed_ms": elapsed_ms,
                }),
            );
        }
    }
    result
}

async fn execute_inner(request: Value) -> anyhow::Result<Value> {
    let port = crate::status::StatusStore::default()
        .load_latest()?
        .and_then(|status| status.debug_port)
        .context("桌面命令入口暂不可用")?;
    let target = command_target(port).await?;
    let websocket = target
        .web_socket_debugger_url
        .as_deref()
        .context("桌面命令连接暂不可用")?;
    crate::cdp::validate_cdp_websocket_url(websocket, port)?;
    let request = serde_json::to_string(&request)?;
    let script = format!(
        r#"(async () => {{
          let execute = window.__codexPlusMobileRemoteCommand;
          const bridgeDeadline = Date.now() + 3000;
          while (typeof execute !== "function" && Date.now() < bridgeDeadline) {{
            await new Promise((resolve) => setTimeout(resolve, 100));
            execute = window.__codexPlusMobileRemoteCommand;
          }}
          if (typeof execute !== "function") {{
            return {{ status: "failed", errorCode: "unsupported_operation",
              message: "桌面远程命令桥尚未就绪，请重启轩++与 Codex" }};
          }}
          try {{
            return await execute({request});
          }} catch (error) {{
            return {{ status: "failed", errorCode: "internal",
              message: String(error?.message || error || "桌面命令执行失败") }};
          }}
        }})()"#
    );
    let response = crate::bridge::evaluate_script_with_await_promise_timeout(
        websocket,
        &script,
        true,
        COMMAND_TIMEOUT,
    )
    .await
    .context("桌面命令响应失败")?;
    let value = response
        .get("result")
        .and_then(|result| result.get("result"))
        .and_then(|result| result.get("value"))
        .cloned()
        .context("桌面命令未返回结果")?;
    if !value.is_object() {
        bail!("桌面命令结果格式无效");
    }
    Ok(value)
}

async fn command_target(port: u16) -> anyhow::Result<crate::cdp::CdpTarget> {
    let mut last_error = None;
    for attempt in 0..TARGET_RETRY_COUNT {
        match crate::cdp::list_targets(port).await {
            Ok(targets) => {
                if let Some(target) = select_command_target(&targets) {
                    return Ok(target);
                }
                last_error = Some(anyhow::anyhow!("桌面命令页面暂不可用"));
            }
            Err(error) => last_error = Some(error),
        }
        if attempt + 1 < TARGET_RETRY_COUNT {
            tokio::time::sleep(TARGET_RETRY_DELAY).await;
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("桌面命令页面暂不可用")))
}

fn select_command_target(targets: &[crate::cdp::CdpTarget]) -> Option<crate::cdp::CdpTarget> {
    targets
        .iter()
        .find(|target| target.url.trim().eq_ignore_ascii_case("app://-/index.html"))
        .or_else(|| {
            targets.iter().find(|target| {
                crate::cdp::is_primary_codex_page_target(target)
                    && target
                        .url
                        .trim()
                        .to_ascii_lowercase()
                        .starts_with("app://-/")
            })
        })
        .cloned()
}

fn renderer_failure_phase(message: &Value) -> &'static str {
    let message = message.as_str().unwrap_or("").to_ascii_lowercase();
    for phase in [
        "thread/start",
        "thread/resume",
        "turn/start",
        "turn/interrupt",
        "thread/name/set",
    ] {
        if message.contains(phase) {
            return phase;
        }
    }
    "unknown"
}

fn renderer_failure_reason(message: &Value) -> &'static str {
    let message = message.as_str().unwrap_or("").to_ascii_lowercase();
    if message.is_empty() {
        "none"
    } else if message.contains("timed out") || message.contains("timeout") {
        "official_timeout"
    } else if message.contains("model provider") || message.contains("configuration") {
        "provider_configuration"
    } else if message.contains("bridge") || message.contains("命令桥") {
        "bridge_unavailable"
    } else {
        "official_error"
    }
}

fn command_source_failure_reason(error: &anyhow::Error) -> &'static str {
    let message = format!("{error:#}").to_ascii_lowercase();
    if message.contains("timed out") || message.contains("timeout") {
        "cdp_timeout"
    } else if message.contains("query cdp targets") || message.contains("connect error") {
        "cdp_unavailable"
    } else if message.contains("命令页面") {
        "page_unavailable"
    } else if message.contains("命令连接") {
        "connection_unavailable"
    } else if message.contains("命令响应") || message.contains("未返回结果") {
        "response_invalid"
    } else {
        "internal"
    }
}

pub(super) fn request(
    command_type: &str,
    thread_id: &str,
    turn_id: &str,
    client_request_id: &str,
    cwd: &str,
    model: &str,
    provider: &str,
    text: &str,
    name: &str,
) -> Value {
    json!({
        "commandType": command_type,
        "threadId": thread_id,
        "turnId": turn_id,
        "clientRequestId": client_request_id,
        "cwd": cwd,
        "model": model,
        "provider": provider,
        "text": text,
        "name": name,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renderer_failures_are_classified_without_persisting_raw_messages() {
        assert_eq!(
            renderer_failure_phase(&json!("thread/start: Codex thread/start timed out")),
            "thread/start"
        );
        assert_eq!(
            renderer_failure_reason(&json!("thread/start: Codex thread/start timed out")),
            "official_timeout"
        );
        assert_eq!(
            renderer_failure_reason(&json!(
                "thread/start: failed to load configuration: Model provider missing not found"
            )),
            "provider_configuration"
        );
    }

    #[test]
    fn command_source_failures_use_public_diagnostic_categories() {
        assert_eq!(
            command_source_failure_reason(&anyhow::anyhow!(
                "timed out waiting for CDP command Runtime.evaluate"
            )),
            "cdp_timeout"
        );
        assert_eq!(
            command_source_failure_reason(&anyhow::anyhow!("桌面命令页面暂不可用")),
            "page_unavailable"
        );
    }

    #[test]
    fn command_target_prefers_the_main_window_over_avatar_overlay() {
        let target = |id: &str, url: &str| crate::cdp::CdpTarget {
            id: id.to_owned(),
            target_type: "page".to_owned(),
            title: "ChatGPT".to_owned(),
            url: url.to_owned(),
            web_socket_debugger_url: Some(format!("ws://127.0.0.1:9229/devtools/page/{id}")),
        };
        let targets = [
            target(
                "overlay",
                "app://-/index.html?initialRoute=%2Favatar-overlay",
            ),
            target("main", "app://-/index.html"),
        ];

        assert_eq!(select_command_target(&targets).unwrap().id, "main");
    }
}
