use std::collections::HashMap;
use std::collections::VecDeque;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Context, bail};
use base64::Engine;
use futures_util::stream::FuturesUnordered;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

pub const BRIDGE_BINDING_NAME: &str = "codexSessionDeleteV2";
const CDP_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const CDP_COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
/// 陈旧会话的 generation 轮询间隔。旧会话只会在"socket 再收到消息"时走到循环顶部的
/// generation 检查；bridge 失效场景下旧 socket 不会再有任何消息，没有这个轮询，
/// 被顶替的会话会带着 Runtime.enable 订阅和脚本注册无限期滞留。
pub const BRIDGE_GENERATION_POLL_INTERVAL: Duration = Duration::from_secs(1);

pub type BridgeHandler = Arc<
    dyn Fn(String, Value) -> Pin<Box<dyn Future<Output = anyhow::Result<Value>> + Send>>
        + Send
        + Sync,
>;

static NEXT_MESSAGE_ID: AtomicU64 = AtomicU64::new(100);

/// Bridge 会话按注入目标分代。
///
/// 同一目标再次安装 Bridge 时，旧会话会在下一次消息循环中退出并关闭 socket，
/// 避免多份 CDP 会话同时应答同一个页面请求。不同目标互不影响。
static NEXT_BRIDGE_GENERATION: AtomicU64 = AtomicU64::new(1);
static CURRENT_BRIDGE_GENERATIONS: std::sync::LazyLock<std::sync::Mutex<HashMap<String, u64>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

#[derive(Clone)]
struct BridgeGeneration {
    target: String,
    id: u64,
}

type PendingBridgeCall = Pin<Box<dyn Future<Output = CompletedBridgeCall> + Send>>;

struct CompletedBridgeCall {
    request_id: String,
    generation: Option<BridgeGeneration>,
    response: Result<Value, String>,
}

fn next_bridge_generation(target: &str) -> BridgeGeneration {
    BridgeGeneration {
        target: target.to_string(),
        id: NEXT_BRIDGE_GENERATION.fetch_add(1, Ordering::SeqCst),
    }
}

fn publish_bridge_generation(generation: &BridgeGeneration) -> bool {
    let mut generations = CURRENT_BRIDGE_GENERATIONS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if generations
        .get(&generation.target)
        .is_some_and(|current| *current > generation.id)
    {
        return false;
    }
    generations.insert(generation.target.clone(), generation.id);
    true
}

fn bridge_generation_is_current(generation: &BridgeGeneration) -> bool {
    CURRENT_BRIDGE_GENERATIONS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&generation.target)
        .is_some_and(|current| *current == generation.id)
}

fn release_bridge_generation(generation: &BridgeGeneration) {
    let mut generations = CURRENT_BRIDGE_GENERATIONS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if generations
        .get(&generation.target)
        .is_some_and(|current| *current == generation.id)
    {
        generations.remove(&generation.target);
    }
}

pub fn build_bridge_script(binding_name: &str) -> String {
    format!(
        r#"
(() => {{
  // Bridge 可能在请求进行中被重新注入。不要静默丢弃旧 resolver，
  // 否则调用方的 Promise 会永久 pending（服务模式会一直显示“正在读取”）。
  const previousCallbacks = window.__codexSessionDeleteCallbacks;
  if (previousCallbacks && typeof previousCallbacks.forEach === "function") {{
    previousCallbacks.forEach((callback) => {{
      try {{ callback.resolve({{ status: "failed", message: "桥接已重新连接" }}); }} catch {{}}
    }});
  }}
  window.__codexSessionDeleteCallbacks = new Map();
  window.__codexSessionDeleteSeq = Number.isFinite(window.__codexSessionDeleteSeq)
    ? window.__codexSessionDeleteSeq
    : 0;
  window.__codexPlusBridgeHealth = window.__codexPlusBridgeHealth || {{}};
  window.__codexPlusBridgeHealth.lastInjectionAt = Date.now();
  window.__codexSessionDeleteResolve = (id, result) => {{
    const callback = window.__codexSessionDeleteCallbacks.get(id);
    if (!callback) return;
    window.__codexSessionDeleteCallbacks.delete(id);
    callback.resolve(result);
  }};
  window.__codexSessionDeleteReject = (id, message) => {{
    const callback = window.__codexSessionDeleteCallbacks.get(id);
    if (!callback) return;
    window.__codexSessionDeleteCallbacks.delete(id);
    callback.resolve({{ status: "failed", message }});
  }};
  window.__codexSessionDeleteBridge = (path, payload) => new Promise((resolve) => {{
    const id = String(++window.__codexSessionDeleteSeq);
    window.__codexSessionDeleteCallbacks.set(id, {{ resolve }});
    window.{binding_name}(JSON.stringify({{ id, path, payload }}));
  }});
}})();
"#
    )
}

pub fn bridge_health_check_script() -> &'static str {
    r#"
(() => {
  // The renderer heartbeat records real bridge results. Reading this state
  // keeps the watchdog probe synchronous and cannot create a duplicate call.
  const bridge = window.__codexSessionDeleteBridge;
  const health = window.__codexPlusBridgeHealth;
  if (typeof bridge !== "function" || !health) return false;
  const now = Date.now();
  const lastSuccessAt = Number(health.lastSuccessAt) || 0;
  const lastInjectionAt = Number(health.lastInjectionAt) || 0;
  const lastAttemptAt = Number(health.lastAttemptAt) || 0;
  if (lastInjectionAt > 0 && now - lastInjectionAt <= 5000) return true;
  if (lastSuccessAt > 0 && now - lastSuccessAt <= 15000) return true;
  // 页面忙碌时状态请求会超时，但心跳仍在调用桥接。最近一次尝试也算活着，
  // 避免看门狗把整份脚本反复注入并触发整页刷新。
  return lastAttemptAt > 0 && now - lastAttemptAt <= 15000;
})()
"#
}

pub fn bridge_active_health_probe_script() -> &'static str {
    r#"
(async () => {
  const bridge = window.__codexSessionDeleteBridge;
  if (typeof bridge !== "function") return false;
  let timer;
  try {
    const timeout = new Promise((resolve) => {
      timer = setTimeout(() => resolve(null), 1500);
    });
    const result = await Promise.race([bridge("/backend/status", {}), timeout]);
    if (result?.status !== "ok") return false;
    const health = window.__codexPlusBridgeHealth || (window.__codexPlusBridgeHealth = {});
    health.lastSuccessAt = Date.now();
    return true;
  } catch {
    return false;
  } finally {
    clearTimeout(timer);
  }
})()
"#
}

pub async fn evaluate_script(websocket_url: &str, script: &str) -> anyhow::Result<Value> {
    evaluate_script_with_await_promise(websocket_url, script, false).await
}

pub async fn evaluate_script_with_await_promise(
    websocket_url: &str,
    script: &str,
    await_promise: bool,
) -> anyhow::Result<Value> {
    let socket = connect_cdp_websocket(websocket_url).await?;
    let mut session = CdpSession::new(socket);
    let response = session
        .send_command(
            1,
            "Runtime.evaluate",
            runtime_evaluate_params_with_await_promise(script, await_promise),
        )
        .await?;
    ensure_runtime_evaluate_succeeded(response)
}

pub fn capture_screenshot_params() -> Value {
    json!({
        "format": "png",
        "fromSurface": true,
        "captureBeyondViewport": false,
    })
}

pub async fn send_cdp_command(
    websocket_url: &str,
    method: &str,
    params: Value,
) -> anyhow::Result<Value> {
    let socket = connect_cdp_websocket(websocket_url).await?;
    let mut session = CdpSession::new(socket);
    session
        .send_command(next_message_id(), method, params)
        .await
}

pub async fn capture_page_screenshot(
    websocket_url: &str,
    output_path: &Path,
) -> anyhow::Result<u64> {
    let response = send_cdp_command(
        websocket_url,
        "Page.captureScreenshot",
        capture_screenshot_params(),
    )
    .await?;
    let encoded = response
        .get("result")
        .and_then(|result| result.get("data"))
        .and_then(Value::as_str)
        .filter(|data| !data.is_empty())
        .ok_or_else(|| anyhow::anyhow!("Page.captureScreenshot returned no image data"))?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .context("failed to decode screenshot PNG")?;
    if !bytes.starts_with(&[137, 80, 78, 71, 13, 10, 26, 10]) {
        bail!("Page.captureScreenshot returned invalid PNG data");
    }
    crate::settings::atomic_write(output_path, &bytes)
        .with_context(|| format!("failed to save screenshot {}", output_path.display()))?;
    Ok(bytes.len() as u64)
}

pub async fn run_periodic_evaluations<F>(
    websocket_url: &str,
    period: Duration,
    mut next_expression: F,
) -> anyhow::Result<()>
where
    F: FnMut() -> anyhow::Result<Option<String>>,
{
    let socket = connect_cdp_websocket(websocket_url).await?;
    let mut session = CdpSession::new(socket);
    let mut interval = tokio::time::interval(period);
    loop {
        interval.tick().await;
        let Some(expression) = next_expression()? else {
            return Ok(());
        };
        let response = session
            .send_command(
                next_message_id(),
                "Runtime.evaluate",
                runtime_evaluate_params(&expression),
            )
            .await?;
        let response = ensure_runtime_evaluate_succeeded(response)?;
        if runtime_evaluate_result_is_false(&response) {
            bail!("periodic Runtime.evaluate reported unavailable capability");
        }
    }
}

pub async fn add_script_to_new_documents(
    websocket_url: &str,
    script: &str,
) -> anyhow::Result<Value> {
    let socket = connect_cdp_websocket(websocket_url).await?;
    let mut session = CdpSession::new(socket);
    session
        .send_command(
            1,
            "Page.addScriptToEvaluateOnNewDocument",
            json!({ "source": script }),
        )
        .await
}

/// issue #2177：AppServerRequestClient 在 Codex 26.908+ 被藏进模块闭包且不再导出，
/// 渲染层扫描无法触达，直接改写 dispatcher 又会撞上不可写的 RPC stub。
/// 分两段接管：渲染层用纯文本定位算出 sendRequest 的断点坐标并放到
/// `window.__codexPlusAppServerClientCapture`；这里用独立 CDP 会话按坐标下
/// 条件断点（`!window.__codexPlusAppServerClientClass`），命中时把类构造器挂到
/// `window.__codexPlusAppServerClientClass`，渲染层再对原型套用与旧版实例补丁
/// 一致的请求逻辑。条件断点保证页面重载后自动重新捕获、正常路径零暂停。
pub fn app_server_client_capture_condition() -> &'static str {
    "!window.__codexPlusAppServerClientClass"
}

pub fn app_server_client_capture_probe_script() -> &'static str {
    "JSON.stringify(window.__codexPlusAppServerClientCapture || null)"
}

pub fn parse_app_server_client_capture_location(value: &Value) -> Option<(String, u32, u32)> {
    let text = value.as_str()?;
    let parsed: Value = serde_json::from_str(text).ok()?;
    let url_regex = parsed.get("urlRegex")?.as_str()?.to_string();
    if url_regex.is_empty() {
        return None;
    }
    let line_number = parsed.get("lineNumber")?.as_u64()?;
    let column_number = parsed.get("columnNumber")?.as_u64()?;
    if line_number > u32::MAX as u64 || column_number > u32::MAX as u64 {
        return None;
    }
    Some((url_regex, line_number as u32, column_number as u32))
}

const APP_SERVER_CLIENT_CAPTURE_LOCATION_TIMEOUT: Duration = Duration::from_secs(90);
const APP_SERVER_CLIENT_CAPTURE_LOCATION_POLL: Duration = Duration::from_secs(2);
const APP_SERVER_CLIENT_CAPTURE_WATCH_CAP: Duration = Duration::from_secs(8 * 60 * 60);
const APP_SERVER_CLIENT_CAPTURE_ON_CALL_FRAME: &str =
    "try{window.__codexPlusAppServerClientClass=this.constructor}catch(e){};1";

fn spawn_app_server_client_capture(websocket_url: &str, generation: BridgeGeneration) {
    let websocket_url = websocket_url.to_string();
    tokio::spawn(async move {
        if let Err(error) = run_app_server_client_capture(&websocket_url, generation).await {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "bridge.app_server_client_capture_failed",
                json!({ "message": error.to_string() }),
            );
        }
    });
}

pub fn external_api_quota_breakpoint_condition(value: &Value) -> Option<String> {
    let quota = value.get("quotaVariable")?.as_str()?;
    let host = value.get("hostVariable")?.as_str()?;
    let identifier = |text: &str| {
        !text.is_empty()
            && text.chars().enumerate().all(|(index, ch)| {
                ch == '_'
                    || ch == '$'
                    || ch.is_ascii_alphabetic()
                    || (index > 0 && ch.is_ascii_digit())
            })
    };
    if !identifier(quota) || !identifier(host) {
        return None;
    }
    Some(format!(
        "({quota}&&window.__codexPlusExternalApiQuotaAllowed?.({host})===true&&({quota}=false),false)"
    ))
}

fn spawn_external_api_quota_gate(websocket_url: &str, generation: BridgeGeneration) {
    let websocket_url = websocket_url.to_string();
    tokio::spawn(async move {
        supervise_external_api_quota_gate(&websocket_url, generation).await;
    });
}

async fn supervise_external_api_quota_gate(websocket_url: &str, generation: BridgeGeneration) {
    let mut retry_seconds = 2;
    while bridge_generation_is_current(&generation) {
        let started = std::time::Instant::now();
        match run_external_api_quota_gate(websocket_url, generation.clone()).await {
            Ok(()) => break,
            Err(error) => {
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "bridge.external_api_quota_gate_failed",
                    json!({ "message": error.to_string(), "retry_seconds": retry_seconds }),
                );
            }
        }
        if !bridge_generation_is_current(&generation) {
            break;
        }
        if started.elapsed() > Duration::from_secs(30) {
            retry_seconds = 2;
        }
        tokio::time::sleep(Duration::from_secs(retry_seconds)).await;
        retry_seconds = (retry_seconds * 2).min(30);
    }
}

#[cfg(test)]
mod external_api_quota_tests {
    use super::*;

    #[tokio::test]
    async fn quota_gate_reconnects_after_timeout_and_stops_when_superseded() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "ws://{}/devtools/page/quota-test",
            listener.local_addr().unwrap()
        );
        let generation = next_bridge_generation(&url);
        assert!(publish_bridge_generation(&generation));
        let task_generation = generation.clone();
        let task = tokio::spawn(async move {
            supervise_external_api_quota_gate(&url, task_generation).await;
        });
        let (stream, _) = listener.accept().await.unwrap();
        let mut first = tokio_tungstenite::accept_async(stream).await.unwrap();
        let command = tokio::time::timeout(Duration::from_secs(5), first.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let command: Value = serde_json::from_str(command.to_text().unwrap()).unwrap();
        assert_eq!(command["method"], "Runtime.evaluate");
        // 模拟启动繁忙：连接保持，但首个 CDP 命令不返回。
        let (stream, _) = tokio::time::timeout(Duration::from_secs(12), listener.accept())
            .await
            .expect("quota gate should reconnect after command timeout")
            .unwrap();
        let _second = tokio_tungstenite::accept_async(stream).await.unwrap();
        release_bridge_generation(&generation);
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("superseded supervisor must stop")
            .unwrap();
    }
}

async fn run_external_api_quota_gate(
    websocket_url: &str,
    generation: BridgeGeneration,
) -> anyhow::Result<()> {
    let socket = connect_cdp_websocket(websocket_url).await?;
    let mut session = CdpSession::new(socket);
    let mut installed: Option<(Value, String)> = None;
    let mut debugger_enabled = false;
    while bridge_generation_is_current(&generation) {
        tokio::time::sleep(Duration::from_secs(2)).await;
        if !bridge_generation_is_current(&generation) {
            break;
        }
        let expression = if installed.is_some() {
            "window.__codexPlusApiQuotaGate?.refreshComposers?.(window.__codexPlusApiQuotaBreakpoint); JSON.stringify(window.__codexPlusApiQuotaBreakpoint || null)"
        } else {
            "JSON.stringify(window.__codexPlusApiQuotaBreakpoint || null)"
        };
        let response = session
            .send_command(
                next_message_id(),
                "Runtime.evaluate",
                runtime_evaluate_params(expression),
            )
            .await?;
        let Some(text) = response
            .pointer("/result/result/value")
            .and_then(Value::as_str)
        else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(text) else {
            continue;
        };
        if installed
            .as_ref()
            .is_some_and(|(previous, _)| previous == &value)
        {
            continue;
        }
        let Some(condition) = external_api_quota_breakpoint_condition(&value) else {
            continue;
        };
        let Some((url_regex, line_number, column_number)) =
            parse_app_server_client_capture_location(&Value::String(text.to_string()))
        else {
            continue;
        };
        if !debugger_enabled {
            session
                .send_command(next_message_id(), "Debugger.enable", json!({}))
                .await?;
            debugger_enabled = true;
        }
        if let Some((_, id)) = installed.take() {
            session
                .send_command(
                    next_message_id(),
                    "Debugger.removeBreakpoint",
                    json!({ "breakpointId": id }),
                )
                .await?;
        }
        let result = session
            .send_command(
                next_message_id(),
                "Debugger.setBreakpointByUrl",
                json!({
                    "urlRegex": url_regex,
                    "lineNumber": line_number,
                    "columnNumber": column_number,
                    "condition": condition,
                }),
            )
            .await?;
        let Some(id) = result.pointer("/result/breakpointId").and_then(Value::as_str) else {
            bail!("external API quota gate breakpoint was not installed");
        };
        installed = Some((value, id.to_string()));
        // 断点不会重跑已经完成的 render。安装后必须触发等价状态重绘。
        session
            .send_command(
                next_message_id(),
                "Runtime.evaluate",
                runtime_evaluate_params(
                    "window.__codexPlusApiQuotaGate?.refreshComposers?.(window.__codexPlusApiQuotaBreakpoint, true)",
                ),
            )
            .await?;
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "bridge.external_api_quota_gate_armed",
            json!({}),
        );
    }
    if let Some((_, id)) = installed {
        let _ = session
            .send_command(
                next_message_id(),
                "Debugger.removeBreakpoint",
                json!({ "breakpointId": id }),
            )
            .await;
    }
    session.close().await;
    Ok(())
}

async fn run_app_server_client_capture(
    websocket_url: &str,
    generation: BridgeGeneration,
) -> anyhow::Result<()> {
    let settings = crate::settings::SettingsStore::default()
        .load()
        .unwrap_or_default();
    if !settings.codex_app_service_tier_controls && !settings.codex_app_model_whitelist_unlock {
        return Ok(());
    }
    let socket = connect_cdp_websocket(websocket_url).await?;
    let mut session = CdpSession::new(socket);
    let probe_script = app_server_client_capture_probe_script();
    let location_wait = async {
        loop {
            tokio::time::sleep(APP_SERVER_CLIENT_CAPTURE_LOCATION_POLL).await;
            let Ok(response) = session
                .send_command(
                    next_message_id(),
                    "Runtime.evaluate",
                    runtime_evaluate_params(probe_script),
                )
                .await
            else {
                continue;
            };
            let Some(text) = response.pointer("/result/result/value").and_then(Value::as_str)
            else {
                continue;
            };
            if let Some(location) =
                parse_app_server_client_capture_location(&Value::String(text.to_string()))
            {
                return Some(location);
            }
        }
    };
    let Some((url_regex, line_number, column_number)) = tokio::time::timeout(
        APP_SERVER_CLIENT_CAPTURE_LOCATION_TIMEOUT,
        location_wait,
    )
    .await
    .unwrap_or(None)
    else {
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "bridge.app_server_client_capture_skipped",
            json!({ "reason": "capture_location_unavailable" }),
        );
        return Ok(());
    };
    session
        .send_command(next_message_id(), "Debugger.enable", json!({}))
        .await?;
    let breakpoint = session
        .send_command(
            next_message_id(),
            "Debugger.setBreakpointByUrl",
            json!({
                "lineNumber": line_number,
                "columnNumber": column_number,
                "urlRegex": url_regex,
                "condition": app_server_client_capture_condition(),
            }),
        )
        .await?;
    let Some(breakpoint_id) = breakpoint
        .pointer("/result/breakpointId")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "bridge.app_server_client_capture_skipped",
            json!({ "reason": "breakpoint_unresolved" }),
        );
        return Ok(());
    };
    let _ = crate::diagnostic_log::append_diagnostic_log(
        "bridge.app_server_client_capture_armed",
        json!({
            "lineNumber": line_number,
            "columnNumber": column_number,
        }),
    );
    let watch_deadline = tokio::time::Instant::now() + APP_SERVER_CLIENT_CAPTURE_WATCH_CAP;
    let mut captured_once = false;
    loop {
        if !bridge_generation_is_current(&generation) {
            break;
        }
        let message = tokio::time::timeout_at(watch_deadline, session.next_message()).await;
        let Ok(message) = message else {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "bridge.app_server_client_capture_watch_expired",
                json!({}),
            );
            break;
        };
        let Ok(Some(message)) = message else {
            break;
        };
        if message.get("method").and_then(Value::as_str) != Some("Debugger.paused") {
            continue;
        }
        if let Some(call_frame_id) = message
            .pointer("/params/callFrames/0/callFrameId")
            .and_then(Value::as_str)
            .map(str::to_string)
        {
            let _ = session
                .send_command(
                    next_message_id(),
                    "Debugger.evaluateOnCallFrame",
                    json!({
                        "callFrameId": call_frame_id,
                        "expression": APP_SERVER_CLIENT_CAPTURE_ON_CALL_FRAME,
                        "returnByValue": true,
                    }),
                )
                .await;
        }
        let _ = session
            .send_command(next_message_id(), "Debugger.resume", json!({}))
            .await;
        if !captured_once {
            captured_once = true;
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "bridge.app_server_client_captured",
                json!({}),
            );
        }
    }
    let _ = session
        .send_command(
            next_message_id(),
            "Debugger.removeBreakpoint",
            json!({ "breakpointId": breakpoint_id }),
        )
        .await;
    Ok(())
}

pub async fn install_bridge(
    websocket_url: &str,
    binding_name: &str,
    handler: BridgeHandler,
    new_document_scripts: &[String],
) -> anyhow::Result<()> {
    let socket = connect_cdp_websocket(websocket_url).await?;
    let mut session = CdpSession::new(socket).with_handler(handler);
    let generation = next_bridge_generation(websocket_url);
    session = session.with_generation(generation.clone());
    let mut registered_script_ids = Vec::new();

    let install_result: anyhow::Result<()> = async {
        session.send_command(1, "Runtime.enable", json!({})).await?;
        session
            .send_command(2, "Runtime.removeBinding", json!({ "name": binding_name }))
            .await?;
        session
            .send_command(3, "Runtime.addBinding", json!({ "name": binding_name }))
            .await?;

        let bridge_script = build_bridge_script(binding_name);
        let response = session
            .send_command(
                4,
                "Page.addScriptToEvaluateOnNewDocument",
                json!({ "source": bridge_script }),
            )
            .await?;
        collect_script_identifier(&response, &mut registered_script_ids);
        session
            .send_command(
                5,
                "Runtime.evaluate",
                runtime_evaluate_params(&bridge_script),
            )
            .await?;

        for script in new_document_scripts {
            let message_id = next_message_id();
            let response = session
                .send_command(
                    message_id,
                    "Page.addScriptToEvaluateOnNewDocument",
                    json!({ "source": script }),
                )
                .await?;
            collect_script_identifier(&response, &mut registered_script_ids);
            let message_id = next_message_id();
            session
                .send_command(
                    message_id,
                    "Runtime.evaluate",
                    runtime_evaluate_params(script),
                )
                .await?;
        }
        Ok(())
    }
    .await;
    if let Err(error) = install_result {
        session
            .remove_registered_scripts(&registered_script_ids)
            .await;
        session.close().await;
        return Err(error);
    }

    if !publish_bridge_generation(&generation) {
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "bridge.generation_superseded_before_publish",
            json!({ "generation": generation.id }),
        );
        session
            .remove_registered_scripts(&registered_script_ids)
            .await;
        session.close().await;
        return Ok(());
    }
    let _ = crate::diagnostic_log::append_diagnostic_log(
        "bridge.generation_published",
        json!({ "generation": generation.id }),
    );

    spawn_app_server_client_capture(websocket_url, generation.clone());
    spawn_external_api_quota_gate(websocket_url, generation.clone());

    let mut pending_calls = FuturesUnordered::new();
    session.enqueue_binding_calls(&mut pending_calls);
    tokio::spawn(async move {
        loop {
            if !bridge_generation_is_current(&generation) {
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "bridge.generation_superseded",
                    json!({ "generation": generation.id }),
                );
                break;
            }

            session.enqueue_binding_calls(&mut pending_calls);
            tokio::select! {
                completed = pending_calls.next(), if !pending_calls.is_empty() => {
                    let Some(completed) = completed else {
                        continue;
                    };
                    if session.finish_binding_call(completed).await.is_err() {
                        break;
                    }
                }
                message = session.next_message() => {
                    match message {
                        Ok(Some(_)) => {}
                        Ok(None) | Err(_) => break,
                    }
                }
                // 不依赖 socket 消息也能发现 generation 过期：旧会话最多
                // 一个轮询间隔内主动退出，避免失效场景下会话无限堆积。
                _ = tokio::time::sleep(BRIDGE_GENERATION_POLL_INTERVAL) => {}
            }
        }
        session
            .remove_registered_scripts(&registered_script_ids)
            .await;
        session.close().await;
        release_bridge_generation(&generation);
    });

    Ok(())
}

fn collect_script_identifier(response: &Value, identifiers: &mut Vec<String>) {
    if let Some(identifier) = response
        .get("result")
        .and_then(|result| result.get("identifier"))
        .and_then(Value::as_str)
        && !identifier.is_empty()
    {
        identifiers.push(identifier.to_string());
    }
}

pub fn runtime_evaluate_params(script: &str) -> Value {
    runtime_evaluate_params_with_await_promise(script, false)
}

pub fn runtime_evaluate_params_with_await_promise(script: &str, await_promise: bool) -> Value {
    json!({
        "expression": script,
        "awaitPromise": await_promise,
        "allowUnsafeEvalBlockedByCSP": true,
    })
}

pub fn resolve_bridge_expression(request_id: &str, result: &Value) -> anyhow::Result<String> {
    Ok(format!(
        "window.__codexSessionDeleteResolve({}, {})",
        serde_json::to_string(request_id)?,
        serde_json::to_string(result)?,
    ))
}

pub fn reject_bridge_expression(request_id: &str, message: &str) -> anyhow::Result<String> {
    Ok(format!(
        "window.__codexSessionDeleteReject({}, {})",
        serde_json::to_string(request_id)?,
        serde_json::to_string(message)?,
    ))
}

async fn connect_cdp_websocket(
    websocket_url: &str,
) -> anyhow::Result<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
> {
    let parsed = reqwest::Url::parse(websocket_url).context("invalid CDP WebSocket URL")?;
    let port = parsed
        .port()
        .ok_or_else(|| anyhow::anyhow!("CDP WebSocket URL must include an explicit port"))?;
    crate::cdp::validate_cdp_websocket_url(websocket_url, port)?;
    let (socket, _) = tokio::time::timeout(CDP_CONNECT_TIMEOUT, connect_async(websocket_url))
        .await
        .with_context(|| {
            format!(
                "timed out connecting CDP websocket after {}s",
                CDP_CONNECT_TIMEOUT.as_secs()
            )
        })?
        .context("failed to connect CDP websocket")?;

    Ok(socket)
}

struct CdpSession<S> {
    socket: S,
    responses: HashMap<u64, Value>,
    binding_calls: VecDeque<Value>,
    handler: Option<BridgeHandler>,
    generation: Option<BridgeGeneration>,
}

impl<S> CdpSession<S>
where
    S: SinkExt<Message>
        + StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
        + Unpin
        + Send,
    <S as futures_util::Sink<Message>>::Error: std::error::Error + Send + Sync + 'static,
{
    fn new(socket: S) -> Self {
        Self {
            socket,
            responses: HashMap::new(),
            binding_calls: VecDeque::new(),
            handler: None,
            generation: None,
        }
    }

    fn with_handler(mut self, handler: BridgeHandler) -> Self {
        self.handler = Some(handler);
        self
    }

    fn with_generation(mut self, generation: BridgeGeneration) -> Self {
        self.generation = Some(generation);
        self
    }

    fn is_current(&self) -> bool {
        self.generation
            .as_ref()
            .is_none_or(bridge_generation_is_current)
    }

    async fn close(&mut self) {
        let _ = self.socket.send(Message::Close(None)).await;
        let _ = self.socket.close().await;
    }

    async fn remove_registered_scripts(&mut self, identifiers: &[String]) {
        for identifier in identifiers {
            let _ = self
                .send_command_without_wait(
                    next_message_id(),
                    "Page.removeScriptToEvaluateOnNewDocument",
                    json!({ "identifier": identifier }),
                )
                .await;
        }
    }

    async fn send_command(
        &mut self,
        message_id: u64,
        method: &str,
        params: Value,
    ) -> anyhow::Result<Value> {
        self.socket
            .send(Message::Text(
                json!({
                    "id": message_id,
                    "method": method,
                    "params": params,
                })
                .to_string()
                .into(),
            ))
            .await
            .with_context(|| format!("failed to send CDP command {method} id {message_id}"))?;

        tokio::time::timeout(
            CDP_COMMAND_TIMEOUT,
            self.wait_for_id(message_id, method.to_string()),
        )
        .await
        .with_context(|| {
            format!(
                "timed out waiting for CDP command {method} id {message_id} response after {}s",
                CDP_COMMAND_TIMEOUT.as_secs()
            )
        })?
    }

    async fn send_command_without_wait(
        &mut self,
        message_id: u64,
        method: &str,
        params: Value,
    ) -> anyhow::Result<()> {
        self.socket
            .send(Message::Text(
                json!({
                    "id": message_id,
                    "method": method,
                    "params": params,
                })
                .to_string()
                .into(),
            ))
            .await
            .with_context(|| format!("failed to send CDP command {method} id {message_id}"))?;
        Ok(())
    }

    async fn wait_for_id(&mut self, message_id: u64, method: String) -> anyhow::Result<Value> {
        loop {
            if let Some(response) = self.responses.remove(&message_id) {
                return command_result(response, &method, message_id);
            }

            let Some(message) = self.next_message().await? else {
                bail!("CDP websocket closed before response for {method} id {message_id}");
            };

            if let Some(response_id) = message.get("id").and_then(Value::as_u64) {
                if response_id == message_id {
                    return command_result(message, &method, message_id);
                }
                self.responses.insert(response_id, message);
            }
        }
    }

    async fn next_message(&mut self) -> anyhow::Result<Option<Value>> {
        let Some(message) = self.socket.next().await else {
            return Ok(None);
        };
        let message = message.context("failed to read CDP websocket message")?;
        let Message::Text(text) = message else {
            return Ok(Some(json!({})));
        };
        let value: Value = serde_json::from_str(&text).context("failed to parse CDP message")?;

        if value.get("method").and_then(Value::as_str) == Some("Runtime.bindingCalled") {
            self.binding_calls.push_back(value.clone());
        }

        Ok(Some(value))
    }

    fn enqueue_binding_calls(&mut self, pending_calls: &mut FuturesUnordered<PendingBridgeCall>) {
        while let Some(message) = self.binding_calls.pop_front() {
            self.enqueue_binding_call(message, pending_calls);
        }
    }

    fn enqueue_binding_call(
        &mut self,
        message: Value,
        pending_calls: &mut FuturesUnordered<PendingBridgeCall>,
    ) {
        let Some(handler) = self.handler.clone() else {
            return;
        };

        let Some(payload_text) = message
            .get("params")
            .and_then(|params| params.get("payload"))
            .and_then(Value::as_str)
        else {
            return;
        };

        let parsed: Value = match serde_json::from_str(payload_text) {
            Ok(parsed) => parsed,
            Err(error) => {
                let Some(request_id) = extract_string_field(payload_text, "id") else {
                    return;
                };
                self.enqueue_completed_binding_call(
                    request_id,
                    Err(format!("failed to parse bridge payload: {error}")),
                    pending_calls,
                );
                return;
            }
        };
        let Some(request_id) = parsed.get("id").and_then(Value::as_str).map(str::to_string) else {
            return;
        };
        if !self.is_current() {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "bridge.stale_request_dropped",
                json!({
                    "request_id": request_id,
                    "generation": self.generation.as_ref().map(|generation| generation.id)
                }),
            );
            return;
        }
        let path = parsed
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let payload = parsed.get("payload").cloned().unwrap_or_else(|| json!({}));
        let generation = self.generation.clone();

        pending_calls.push(Box::pin(async move {
            CompletedBridgeCall {
                request_id,
                generation,
                response: handler(path, payload)
                    .await
                    .map_err(|error| error.to_string()),
            }
        }));
    }

    fn enqueue_completed_binding_call(
        &self,
        request_id: String,
        response: Result<Value, String>,
        pending_calls: &mut FuturesUnordered<PendingBridgeCall>,
    ) {
        let generation = self.generation.clone();
        pending_calls.push(Box::pin(async move {
            CompletedBridgeCall {
                request_id,
                generation,
                response,
            }
        }));
    }

    async fn finish_binding_call(&mut self, completed: CompletedBridgeCall) -> anyhow::Result<()> {
        if completed
            .generation
            .as_ref()
            .is_some_and(|generation| !bridge_generation_is_current(generation))
        {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "bridge.stale_response_dropped",
                json!({
                    "request_id": completed.request_id,
                    "generation": completed.generation.as_ref().map(|generation| generation.id)
                }),
            );
            return Ok(());
        }

        match completed.response {
            Ok(result) => {
                self.resolve_bridge_request(&completed.request_id, &result)
                    .await
            }
            Err(message) => {
                self.reject_bridge_request(&completed.request_id, &message)
                    .await
            }
        }
    }

    async fn resolve_bridge_request(
        &mut self,
        request_id: &str,
        result: &Value,
    ) -> anyhow::Result<()> {
        let expression = resolve_bridge_expression(request_id, result)?;
        let message_id = next_message_id();
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "bridge.resolve_start",
            json!({
                "request_id": request_id,
                "message_id": message_id,
                "result_status": result.get("status").and_then(Value::as_str).unwrap_or("")
            }),
        );
        let sent = self
            .send_command_without_wait(
                message_id,
                "Runtime.evaluate",
                runtime_evaluate_params(&expression),
            )
            .await;
        match &sent {
            Ok(_) => {
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "bridge.resolve_ok",
                    json!({
                        "request_id": request_id,
                        "message_id": message_id
                    }),
                );
            }
            Err(error) => {
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "bridge.resolve_failed",
                    json!({
                        "request_id": request_id,
                        "message_id": message_id,
                        "message": error.to_string()
                    }),
                );
            }
        }
        sent.map(|_| ())
    }

    async fn reject_bridge_request(
        &mut self,
        request_id: &str,
        message: &str,
    ) -> anyhow::Result<()> {
        let expression = reject_bridge_expression(request_id, message)?;
        let message_id = next_message_id();
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "bridge.reject_start",
            json!({
                "request_id": request_id,
                "message_id": message_id,
                "message": message
            }),
        );
        let sent = self
            .send_command_without_wait(
                message_id,
                "Runtime.evaluate",
                runtime_evaluate_params(&expression),
            )
            .await;
        match &sent {
            Ok(_) => {
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "bridge.reject_ok",
                    json!({
                        "request_id": request_id,
                        "message_id": message_id
                    }),
                );
            }
            Err(error) => {
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "bridge.reject_failed",
                    json!({
                        "request_id": request_id,
                        "message_id": message_id,
                        "error": error.to_string()
                    }),
                );
            }
        }
        sent.map(|_| ())
    }
}

fn command_result(response: Value, method: &str, message_id: u64) -> anyhow::Result<Value> {
    if let Some(error) = response.get("error") {
        bail!("CDP command {method} id {message_id} failed: {error}");
    }
    Ok(response)
}

fn ensure_runtime_evaluate_succeeded(response: Value) -> anyhow::Result<Value> {
    if let Some(exception) = response
        .get("result")
        .and_then(|result| result.get("exceptionDetails"))
    {
        bail!("Runtime.evaluate raised an exception: {exception}");
    }
    Ok(response)
}

fn runtime_evaluate_result_is_false(response: &Value) -> bool {
    response
        .get("result")
        .and_then(|result| result.get("result"))
        .and_then(|result| result.get("value"))
        .is_some_and(|value| value == false)
}

fn extract_string_field(input: &str, field: &str) -> Option<String> {
    let needle = format!("\"{field}\"");
    let mut index = input.find(&needle)? + needle.len();
    let bytes = input.as_bytes();

    while matches!(bytes.get(index), Some(b' ' | b'\n' | b'\r' | b'\t')) {
        index += 1;
    }
    if bytes.get(index) != Some(&b':') {
        return None;
    }
    index += 1;
    while matches!(bytes.get(index), Some(b' ' | b'\n' | b'\r' | b'\t')) {
        index += 1;
    }
    if bytes.get(index) != Some(&b'"') {
        return None;
    }
    index += 1;

    let mut output = String::new();
    let mut escaped = false;
    for ch in input[index..].chars() {
        if escaped {
            output.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '"' => return Some(output),
            _ => output.push(ch),
        }
    }

    None
}

fn next_message_id() -> u64 {
    NEXT_MESSAGE_ID.fetch_add(1, Ordering::Relaxed) + 1
}
