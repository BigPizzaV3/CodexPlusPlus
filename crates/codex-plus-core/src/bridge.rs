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
use tokio_tungstenite::connect_async_with_config;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

pub const BRIDGE_BINDING_NAME: &str = "codexSessionDeleteV2";
const CDP_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const CDP_COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
// 25MiB 音频的 base64 与 JSON 封装约 33.4MiB；限制单 frame 和完整 message 的内存上限。
const CDP_MAX_MESSAGE_BYTES: usize = 40 * 1024 * 1024;
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
  // 窗口隐藏时 Chromium 会把后台定时器钳到约每分钟一次，心跳时间戳必然过期。
  // 此时桥接对象仍在，不判定失效，避免看门狗按失败阈值每分钟重注入整份脚本
  // （issue #2330）。
  // 必须放在 bridge/health 存在性检查之后：页面重载后真丢桥时仍要能修复。
  if (typeof document !== "undefined" && document.visibilityState === "hidden") return true;
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
// Debugger.enable 冷启动时要枚举大型 bundle，仅放宽抓取会话，不改变普通 CDP 命令。
const APP_SERVER_CLIENT_CAPTURE_ENABLE_TIMEOUT: Duration = Duration::from_secs(15);
const APP_SERVER_CLIENT_CAPTURE_ENABLE_ATTEMPTS: usize = 3;
const APP_SERVER_CLIENT_CAPTURE_ENABLE_RETRY_DELAY: Duration = Duration::from_secs(2);
const APP_SERVER_CLIENT_CAPTURE_ON_CALL_FRAME: &str =
    "(()=>{try{const ctor=this.constructor;if(typeof ctor!=='function'||ctor===Object||ctor===Function)return false;if(typeof ctor.prototype?.sendRequest!=='function'&&typeof this.sendRequest!=='function')return false;window.__codexPlusAppServerClientClass=ctor;return window.__codexPlusAppServerClientClass===ctor}catch{return false}})()";
const APP_SERVER_CLIENT_CAPTURE_COMMAND_TIMEOUT: Duration = Duration::from_secs(1);
const APP_SERVER_CLIENT_CAPTURE_CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);

async fn wait_for_stale_bridge_generation(generation: &BridgeGeneration) {
    while bridge_generation_is_current(generation) {
        tokio::time::sleep(BRIDGE_GENERATION_POLL_INTERVAL).await;
    }
}

async fn enable_app_server_client_capture_debugger<S>(
    session: &mut CdpSession<S>,
    generation: &BridgeGeneration,
    timeout: Duration,
    retry_delay: Duration,
) -> anyhow::Result<bool>
where
    S: SinkExt<Message>
        + StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
        + Unpin
        + Send,
    <S as futures_util::Sink<Message>>::Error: std::error::Error + Send + Sync + 'static,
{
    for attempt in 1..=APP_SERVER_CLIENT_CAPTURE_ENABLE_ATTEMPTS {
        if !bridge_generation_is_current(generation) {
            return Ok(false);
        }
        let result = tokio::select! {
            result = session.send_command_with_timeout(
                next_message_id(), "Debugger.enable", json!({}), timeout
            ) => result,
            _ = wait_for_stale_bridge_generation(generation) => return Ok(false),
        };
        match result {
            Ok(_) => return Ok(bridge_generation_is_current(generation)),
            Err(error) => {
                // 协议错误/断连不能靠重复 enable 修复，只重试迟到或缺失的回包。
                if attempt == APP_SERVER_CLIENT_CAPTURE_ENABLE_ATTEMPTS
                    || !error.is::<tokio::time::error::Elapsed>()
                {
                    return Err(error);
                }
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "bridge.app_server_client_capture_enable_retry",
                    json!({ "attempt": attempt, "timeout_ms": timeout.as_millis() }),
                );
                tokio::select! {
                    _ = tokio::time::sleep(retry_delay) => {},
                    _ = wait_for_stale_bridge_generation(generation) => return Ok(false),
                }
            }
        }
    }
    Ok(false)
}

fn spawn_app_server_client_capture(websocket_url: &str, generation: BridgeGeneration) {
    let websocket_url = websocket_url.to_string();
    tokio::spawn(async move {
        if let Err(error) = run_app_server_client_capture(&websocket_url, generation).await {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "bridge.app_server_client_capture_failed",
                json!({ "message": format!("{error:#}") }),
            );
        }
    });
}

async fn run_app_server_client_capture(
    websocket_url: &str,
    generation: BridgeGeneration,
) -> anyhow::Result<()> {
    let settings = crate::settings::SettingsStore::default()
        .load()
        .unwrap_or_default();
    if !settings.codex_app_service_tier_controls
        && !settings.codex_app_model_whitelist_unlock
        && !settings.codex_app_session_delete
    {
        return Ok(());
    }
    let socket = connect_cdp_websocket(websocket_url).await?;
    let mut session = CdpSession::new(socket);
    let probe_script = app_server_client_capture_probe_script();
    let location_wait = async {
        loop {
            if !bridge_generation_is_current(&generation) {
                return None;
            }
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
            let Some(text) = response
                .pointer("/result/result/value")
                .and_then(Value::as_str)
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
    let Some((url_regex, line_number, column_number)) =
        tokio::time::timeout(APP_SERVER_CLIENT_CAPTURE_LOCATION_TIMEOUT, location_wait)
            .await
            .unwrap_or(None)
    else {
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "bridge.app_server_client_capture_skipped",
            json!({ "reason": "capture_location_unavailable" }),
        );
        return Ok(());
    };
    capture_app_server_client_at_location(
        &mut session, &generation, (url_regex, line_number, column_number),
        APP_SERVER_CLIENT_CAPTURE_COMMAND_TIMEOUT, APP_SERVER_CLIENT_CAPTURE_WATCH_CAP,
        APP_SERVER_CLIENT_CAPTURE_CLEANUP_TIMEOUT,
    ).await.map(|_| ())
}

#[derive(Default)]
struct CaptureDebuggerState {
    breakpoint_id: Option<String>,
}

fn capture_owns_pause(message: &Value, breakpoint_id: Option<&str>) -> bool {
    let Some(breakpoint_id) = breakpoint_id else {
        return false;
    };
    if message.get("method").and_then(Value::as_str) != Some("Debugger.paused") {
        return false;
    }
    let Some(hits) = message
        .pointer("/params/hitBreakpoints")
        .and_then(Value::as_array)
    else {
        return false;
    };
    // 同时命中用户/其它 debugger 的断点时，不能替它们恢复页面。
    !hits.is_empty() && hits.iter().all(|hit| hit.as_str() == Some(breakpoint_id))
}

fn breakpoint_fingerprint(id: &str) -> [u8; 32] {
    use sha2::Digest;
    sha2::Sha256::digest(id.as_bytes()).into()
}

fn paused_hit_fingerprint(message: &Value) -> Option<[u8; 32]> {
    let hits = message.pointer("/params/hitBreakpoints")?.as_array()?;
    let first = hits.first()?.as_str()?;
    hits.iter()
        .all(|hit| hit.as_str() == Some(first))
        .then(|| breakpoint_fingerprint(first))
}

async fn capture_app_server_client_at_location<S>(
    session: &mut CdpSession<S>,
    generation: &BridgeGeneration,
    location: (String, u32, u32),
    command_timeout: Duration,
    watch_cap: Duration,
    cleanup_timeout: Duration,
) -> anyhow::Result<bool>
where
    S: SinkExt<Message>
        + StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
        + Unpin
        + Send,
    <S as futures_util::Sink<Message>>::Error: std::error::Error + Send + Sync + 'static,
{
    let mut state = CaptureDebuggerState::default();
    let result: anyhow::Result<bool> = async {
        if !enable_app_server_client_capture_debugger(
            session, generation, APP_SERVER_CLIENT_CAPTURE_ENABLE_TIMEOUT,
            APP_SERVER_CLIENT_CAPTURE_ENABLE_RETRY_DELAY,
        ).await? { return Ok(false); }
        let (url_regex, line_number, column_number) = location;
        let breakpoint = session.send_command_with_timeout(
            next_message_id(), "Debugger.setBreakpointByUrl",
            json!({ "lineNumber": line_number, "columnNumber": column_number, "urlRegex": url_regex,
                "condition": app_server_client_capture_condition() }), command_timeout,
        ).await?;
        state.breakpoint_id = breakpoint.pointer("/result/breakpointId")
            .and_then(Value::as_str).map(str::to_string);
        anyhow::ensure!(state.breakpoint_id.is_some(), "capture breakpoint response did not contain an id");
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "bridge.app_server_client_capture_armed", json!({ "lineNumber": line_number, "columnNumber": column_number }),
        );
        let watch_deadline = tokio::time::Instant::now() + watch_cap;
        let mut captured_once = false;
        loop {
            if !bridge_generation_is_current(generation) { break; }
            let message = tokio::select! {
                message = tokio::time::timeout_at(watch_deadline, session.next_message()) => message,
                _ = wait_for_stale_bridge_generation(generation) => break,
            };
            let Ok(message) = message else { break; };
            let Some(message) = message? else { break; };
            if !capture_owns_pause(&message, state.breakpoint_id.as_deref())
                || !session.current_pause_is_owned(state.breakpoint_id.as_deref()) { continue; }
            let evaluation: anyhow::Result<()> = async {
                let frame = message.pointer("/params/callFrames/0/callFrameId")
                    .and_then(Value::as_str).context("owned capture pause has no call frame")?;
                let reply = session.send_command_with_timeout(
                    next_message_id(), "Debugger.evaluateOnCallFrame",
                    json!({ "callFrameId": frame, "expression": APP_SERVER_CLIENT_CAPTURE_ON_CALL_FRAME,
                        "returnByValue": true }), command_timeout,
                ).await?;
                anyhow::ensure!(reply.pointer("/result/exceptionDetails").is_none(), "capture frame evaluation raised an exception");
                anyhow::ensure!(reply.pointer("/result/result/value").and_then(Value::as_bool) == Some(true),
                    "capture frame evaluation did not publish the app-server client class");
                Ok(())
            }.await;
            // 求值等待期间其它 debugger 可能已恢复并再次暂停；不能恢复新用户暂停。
            anyhow::ensure!(session.current_pause_is_owned(state.breakpoint_id.as_deref()),
                "owned capture pause was resumed or replaced by another debugger");
            let resume_epoch = session.pause_epoch;
            // 求值失败也要先恢复；失败结果不可冒充 captured，且不能留着页面等下一次 RPC。
            session.send_command_with_timeout(
                next_message_id(), "Debugger.resume", json!({}), command_timeout,
            ).await.context("failed to resume owned app-server capture pause")?;
            session.clear_pause_if_epoch(resume_epoch);
            evaluation?;
            if !captured_once {
                captured_once = true;
                let _ = crate::diagnostic_log::append_diagnostic_log("bridge.app_server_client_captured", json!({}));
            }
        }
        Ok(captured_once)
    }.await;
    let cleanup = tokio::time::timeout(
        cleanup_timeout,
        cleanup_capture_debugger(
            session,
            &mut state,
            command_timeout.min(cleanup_timeout / 4),
        ),
    )
    .await
    .context("capture debugger cleanup timed out")
    .and_then(|result| result);
    // 即便安装超时尚未拿到 id，disable 与关闭本会话也会移除本会话的 debugger 状态。
    let closed = tokio::time::timeout(cleanup_timeout, session.socket.close())
        .await
        .context("capture debugger socket close timed out")
        .and_then(|result| result.context("failed to close capture debugger socket"));
    let cleanup = match (cleanup, closed) {
        (Err(cleanup), Err(closed)) => {
            Err(cleanup.context(format!("socket close also failed: {closed:#}")))
        }
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    };
    match (result, cleanup) {
        (Err(error), Err(cleanup)) => {
            Err(error.context(format!("capture cleanup also failed: {cleanup:#}")))
        }
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Ok(captured), Ok(())) => Ok(captured),
    }
}

async fn cleanup_capture_debugger<S>(
    session: &mut CdpSession<S>,
    state: &mut CaptureDebuggerState,
    timeout: Duration,
) -> anyhow::Result<()>
where
    S: SinkExt<Message>
        + StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
        + Unpin
        + Send,
    <S as futures_util::Sink<Message>>::Error: std::error::Error + Send + Sync + 'static,
{
    let mut errors = Vec::new();
    if let Some(id) = state.breakpoint_id.as_deref() {
        if let Err(error) = session
            .send_command_with_timeout(
                next_message_id(),
                "Debugger.removeBreakpoint",
                json!({ "breakpointId": id }),
                timeout,
            )
            .await
        {
            errors.push(format!("{error:#}"));
        }
    }
    if session.current_pause_is_owned(state.breakpoint_id.as_deref()) {
        let resume_epoch = session.pause_epoch;
        if let Err(error) = session
            .send_command_with_timeout(next_message_id(), "Debugger.resume", json!({}), timeout)
            .await
        {
            errors.push(format!("{error:#}"));
        } else {
            session.clear_pause_if_epoch(resume_epoch);
        }
    }
    // 只 disable 自己的 CDP debugger session；未知 id 时不猜测其它会话的暂停归属。
    if let Err(error) = session
        .send_command_with_timeout(next_message_id(), "Debugger.disable", json!({}), timeout)
        .await
    {
        errors.push(format!("{error:#}"));
    }
    anyhow::ensure!(
        errors.is_empty(),
        "capture debugger cleanup failed: {}",
        errors.join("; ")
    );
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

fn cdp_websocket_config() -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(CDP_MAX_MESSAGE_BYTES))
        .max_frame_size(Some(CDP_MAX_MESSAGE_BYTES))
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
    let (socket, _) = tokio::time::timeout(
        CDP_CONNECT_TIMEOUT,
        connect_async_with_config(websocket_url, Some(cdp_websocket_config()), false),
    )
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
    paused_events: VecDeque<(Value, usize, u64)>,
    paused_event_bytes: usize,
    pause_epoch: u64,
    current_pause_hit: Option<[u8; 32]>,
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
            paused_events: VecDeque::new(),
            paused_event_bytes: 0,
            pause_epoch: 0,
            current_pause_hit: None,
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

    fn current_pause_is_owned(&self, breakpoint_id: Option<&str>) -> bool {
        breakpoint_id.is_some_and(|id| self.current_pause_hit == Some(breakpoint_fingerprint(id)))
    }

    fn clear_pause_if_epoch(&mut self, epoch: u64) {
        if self.pause_epoch == epoch {
            self.pause_epoch = self.pause_epoch.wrapping_add(1);
            self.current_pause_hit = None;
            self.paused_events.clear();
            self.paused_event_bytes = 0;
        }
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
        self.send_command_with_timeout(message_id, method, params, CDP_COMMAND_TIMEOUT)
            .await
    }

    async fn send_command_with_timeout(
        &mut self,
        message_id: u64,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> anyhow::Result<Value> {
        // 发送本身也受截止时间约束，否则暂停中的页面可能卡在写 socket 阶段。
        tokio::time::timeout(timeout, async {
            self.socket
                .send(Message::Text(
                    json!({ "id": message_id, "method": method, "params": params })
                        .to_string()
                        .into(),
                ))
                .await
                .with_context(|| format!("failed to send CDP command {method} id {message_id}"))?;
            self.wait_for_id(message_id, method.to_string()).await
        })
        .await
        .with_context(|| {
            format!(
                "timed out waiting for CDP command {method} id {message_id} response after {}s",
                timeout.as_secs()
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

            // 这里只读 socket，不能重新消费 deferred queue 再放回，造成自喂循环。
            let Some(message) = self.read_socket_message().await? else {
                bail!("CDP websocket closed before response for {method} id {message_id}");
            };

            if let Some(response_id) = message.get("id").and_then(Value::as_u64) {
                if response_id == message_id {
                    return command_result(message, &method, message_id);
                }
                self.responses.insert(response_id, message);
            } else if message.get("method").and_then(Value::as_str) == Some("Debugger.paused") {
                let bytes = message.to_string().len();
                anyhow::ensure!(
                    self.paused_events.len() < 8 && bytes <= 1024 * 1024 - self.paused_event_bytes,
                    "CDP paused event queue exceeded its bounded capacity"
                );
                self.paused_event_bytes += bytes;
                self.paused_events
                    .push_back((message, bytes, self.pause_epoch));
            }
        }
    }

    async fn next_message(&mut self) -> anyhow::Result<Option<Value>> {
        while let Some((message, bytes, epoch)) = self.paused_events.pop_front() {
            self.paused_event_bytes -= bytes;
            if epoch == self.pause_epoch {
                return Ok(Some(message));
            }
        }
        self.read_socket_message().await
    }

    async fn read_socket_message(&mut self) -> anyhow::Result<Option<Value>> {
        let Some(message) = self.socket.next().await else {
            return Ok(None);
        };
        let message = message.context("failed to read CDP websocket message")?;
        let Message::Text(text) = message else {
            return Ok(Some(json!({})));
        };
        let value: Value = serde_json::from_str(&text).context("failed to parse CDP message")?;

        match value.get("method").and_then(Value::as_str) {
            Some("Debugger.paused") => {
                self.pause_epoch = self.pause_epoch.wrapping_add(1);
                // 只保留断点归属的固定尺寸摘要，不保存 scope/源码或复制巨大事件。
                self.current_pause_hit = paused_hit_fingerprint(&value);
            }
            Some("Debugger.resumed") => self.clear_pause_if_epoch(self.pause_epoch),
            _ => {}
        }

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
        // error.data 可能含 frame/scope/源码；诊断只保留协议 code 和 message。
        let code = error.get("code").and_then(Value::as_i64);
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unspecified protocol error");
        bail!("CDP command {method} id {message_id} failed (code {code:?}): {message}");
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

#[cfg(test)]
mod app_server_capture_tests {
    use super::*;

    #[derive(Clone, Copy)]
    enum CaptureCase {
        Success,
        FrameError,
        FrameException,
        ResumeError,
        Stale,
        WatchTimeout,
        InstallTimeout,
        ForeignPause,
        MixedPause,
        RemoveError,
        QueueOverflow,
        HistoricalOwnedThenForeign,
        ForeignDuringFrameEvaluation,
    }

    async fn run_capture_case(case: CaptureCase) -> (anyhow::Result<bool>, Vec<String>, bool) {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let generation = next_bridge_generation(&format!("capture-lifecycle-{address}"));
        assert!(publish_bridge_generation(&generation));
        let generation_for_server = generation.clone();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let mut commands = Vec::new();
            let mut owned_pause = false;
            let mut replacement = None;
            while let Some(message) = socket.next().await {
                let message = message.unwrap();
                if matches!(message, Message::Close(_)) {
                    break;
                }
                let Message::Text(text) = message else {
                    continue;
                };
                let command: Value = serde_json::from_str(&text).unwrap();
                let method = command["method"].as_str().unwrap().to_string();
                commands.push(method.clone());
                let mut result = json!({});
                let mut error = None;
                match method.as_str() {
                    "Debugger.setBreakpointByUrl" => {
                        assert_eq!(
                            command["params"]["condition"],
                            app_server_client_capture_condition()
                        );
                        if matches!(case, CaptureCase::HistoricalOwnedThenForeign) {
                            for event in [
                                json!({ "method": "Debugger.paused", "params": { "hitBreakpoints": ["capture-owned"], "callFrames": [{ "callFrameId": "old-frame" }] } }),
                                json!({ "method": "Debugger.resumed", "params": {} }),
                                json!({ "method": "Debugger.paused", "params": { "hitBreakpoints": ["user-breakpoint"], "callFrames": [{ "callFrameId": "user-frame" }] } }),
                            ] {
                                socket
                                    .send(Message::Text(event.to_string().into()))
                                    .await
                                    .unwrap();
                            }
                        } else if !matches!(
                            case,
                            CaptureCase::WatchTimeout | CaptureCase::RemoveError
                        ) {
                            let hits = match case {
                                CaptureCase::ForeignPause => json!(["user-breakpoint"]),
                                CaptureCase::MixedPause => {
                                    json!(["capture-owned", "user-breakpoint"])
                                }
                                _ => json!(["capture-owned"]),
                            };
                            owned_pause = !matches!(
                                case,
                                CaptureCase::ForeignPause | CaptureCase::MixedPause
                            );
                            let pause = json!({ "method": "Debugger.paused", "params": {
                                "hitBreakpoints": hits, "callFrames": [{ "callFrameId": "capture-frame" }]
                            }});
                            for _ in 0..if matches!(case, CaptureCase::QueueOverflow) {
                                9
                            } else {
                                1
                            } {
                                socket
                                    .send(Message::Text(pause.to_string().into()))
                                    .await
                                    .unwrap();
                            }
                        }
                        if matches!(case, CaptureCase::Stale) {
                            let next = next_bridge_generation(&generation_for_server.target);
                            assert!(publish_bridge_generation(&next));
                            replacement = Some(next);
                        }
                        if matches!(case, CaptureCase::InstallTimeout) {
                            continue;
                        }
                        result = json!({ "breakpointId": "capture-owned" });
                    }
                    "Debugger.evaluateOnCallFrame" => {
                        if matches!(case, CaptureCase::ForeignDuringFrameEvaluation) {
                            for event in [
                                json!({ "method": "Debugger.resumed", "params": {} }),
                                json!({ "method": "Debugger.paused", "params": { "hitBreakpoints": ["user-breakpoint"], "callFrames": [{ "callFrameId": "user-frame" }] } }),
                            ] {
                                socket
                                    .send(Message::Text(event.to_string().into()))
                                    .await
                                    .unwrap();
                            }
                        }
                        if !matches!(case, CaptureCase::HistoricalOwnedThenForeign) {
                            assert_eq!(command["params"]["callFrameId"], "capture-frame");
                        }
                        if matches!(
                            case,
                            CaptureCase::FrameError
                                | CaptureCase::HistoricalOwnedThenForeign
                                | CaptureCase::ForeignDuringFrameEvaluation
                        ) {
                            error = Some(
                                json!({ "code": -32000, "message": "frame unavailable", "data": {
                                "scopeChain": [{ "description": "PRIVATE_SOURCE_SENTINEL" }]
                            } }),
                            );
                        } else if matches!(case, CaptureCase::FrameException) {
                            result = json!({ "exceptionDetails": { "text": "PRIVATE_SOURCE_SENTINEL", "stackTrace": { "callFrames": [] } } });
                        } else {
                            result = json!({ "result": { "type": "boolean", "value": true } });
                        }
                    }
                    "Debugger.resume" => {
                        if matches!(case, CaptureCase::ResumeError) {
                            error = Some(json!({ "code": -32000, "message": "resume refused" }));
                        } else {
                            owned_pause = false;
                        }
                    }
                    "Debugger.removeBreakpoint" => {
                        assert_eq!(command["params"]["breakpointId"], "capture-owned");
                        if matches!(case, CaptureCase::RemoveError) {
                            error = Some(json!({ "code": -32000, "message": "remove refused" }));
                        }
                    }
                    "Debugger.disable" => {
                        owned_pause = false;
                    }
                    "Debugger.enable" => {}
                    other => panic!("unexpected capture command {other}"),
                }
                let reply = if let Some(error) = error {
                    json!({ "id": command["id"], "error": error })
                } else {
                    json!({ "id": command["id"], "result": result })
                };
                socket
                    .send(Message::Text(reply.to_string().into()))
                    .await
                    .unwrap();
            }
            if let Some(replacement) = replacement {
                release_bridge_generation(&replacement);
            }
            (commands, owned_pause)
        });
        let socket =
            connect_cdp_websocket(&format!("ws://{address}/devtools/page/capture-lifecycle"))
                .await
                .unwrap();
        let mut session = CdpSession::new(socket);
        let result = capture_app_server_client_at_location(
            &mut session,
            &generation,
            ("app-bundle".to_string(), 1, 24),
            Duration::from_millis(200),
            Duration::from_millis(250),
            Duration::from_millis(1000),
        )
        .await;
        let (commands, owned_pause) = server.await.unwrap();
        release_bridge_generation(&generation);
        (result, commands, owned_pause)
    }

    #[tokio::test]
    async fn early_owned_pause_is_captured_resumed_and_cleaned_on_watch_expiry() {
        let (result, commands, paused) = run_capture_case(CaptureCase::Success).await;
        assert!(result.unwrap());
        assert!(!paused);
        assert_eq!(
            commands,
            [
                "Debugger.enable",
                "Debugger.setBreakpointByUrl",
                "Debugger.evaluateOnCallFrame",
                "Debugger.resume",
                "Debugger.removeBreakpoint",
                "Debugger.disable"
            ]
        );
    }

    #[tokio::test]
    async fn failed_frame_evaluation_still_resumes_and_does_not_echo_exception_source() {
        for case in [CaptureCase::FrameError, CaptureCase::FrameException] {
            let (result, commands, paused) = run_capture_case(case).await;
            let error = format!("{:#}", result.unwrap_err());
            assert!(!error.contains("PRIVATE_SOURCE_SENTINEL"));
            assert!(commands.contains(&"Debugger.resume".to_string()));
            assert_eq!(commands.last().unwrap(), "Debugger.disable");
            assert!(!paused);
        }
    }

    #[tokio::test]
    async fn resume_failure_is_visible_and_cleanup_disables_owned_debugger() {
        let (result, commands, paused) = run_capture_case(CaptureCase::ResumeError).await;
        let error = format!("{:#}", result.unwrap_err());
        assert!(error.contains("Debugger.resume"));
        assert!(error.contains("resume refused"));
        assert_eq!(
            commands
                .iter()
                .filter(|method| method.as_str() == "Debugger.resume")
                .count(),
            2
        );
        assert_eq!(commands.last().unwrap(), "Debugger.disable");
        assert!(!paused);
    }

    #[tokio::test]
    async fn stale_and_timed_out_capture_sessions_cleanup_including_unknown_breakpoint_id() {
        for case in [
            CaptureCase::Stale,
            CaptureCase::WatchTimeout,
            CaptureCase::InstallTimeout,
        ] {
            let (result, commands, paused) = run_capture_case(case).await;
            assert!(!paused);
            assert_eq!(commands.last().unwrap(), "Debugger.disable");
            if matches!(case, CaptureCase::InstallTimeout) {
                assert!(
                    result
                        .unwrap_err()
                        .to_string()
                        .contains("setBreakpointByUrl")
                );
                assert!(!commands.contains(&"Debugger.removeBreakpoint".to_string()));
                assert!(!commands.contains(&"Debugger.resume".to_string()));
            } else {
                assert!(!result.unwrap());
                assert!(commands.contains(&"Debugger.removeBreakpoint".to_string()));
                assert_eq!(
                    commands.contains(&"Debugger.resume".to_string()),
                    matches!(case, CaptureCase::Stale)
                );
            }
        }
    }

    #[tokio::test]
    async fn user_or_mixed_breakpoints_are_not_captured_or_resumed() {
        for case in [CaptureCase::ForeignPause, CaptureCase::MixedPause] {
            let (result, commands, _) = run_capture_case(case).await;
            assert!(!result.unwrap());
            assert!(!commands.contains(&"Debugger.evaluateOnCallFrame".to_string()));
            assert!(!commands.contains(&"Debugger.resume".to_string()));
        }
    }

    #[tokio::test]
    async fn resumed_historical_owned_pause_cannot_capture_or_resume_current_foreign_pause() {
        let (result, commands, _) = run_capture_case(CaptureCase::HistoricalOwnedThenForeign).await;
        assert!(!result.expect("已恢复的历史暂停不能触碰当前用户暂停"));
        assert!(!commands.contains(&"Debugger.evaluateOnCallFrame".to_string()));
        assert!(!commands.contains(&"Debugger.resume".to_string()));
    }

    #[tokio::test]
    async fn foreign_pause_during_frame_reply_is_not_resumed_by_handler_or_cleanup() {
        let (result, commands, _) =
            run_capture_case(CaptureCase::ForeignDuringFrameEvaluation).await;
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("replaced by another debugger")
        );
        assert!(commands.contains(&"Debugger.evaluateOnCallFrame".to_string()));
        assert!(!commands.contains(&"Debugger.resume".to_string()));
        assert_eq!(commands.last().unwrap(), "Debugger.disable");
    }

    #[tokio::test]
    async fn failed_breakpoint_removal_is_visible_but_does_not_skip_debugger_disable() {
        let (result, commands, paused) = run_capture_case(CaptureCase::RemoveError).await;
        assert!(format!("{:#}", result.unwrap_err()).contains("Debugger.removeBreakpoint"));
        assert_eq!(commands.last().unwrap(), "Debugger.disable");
        assert!(!paused);
    }

    #[tokio::test]
    async fn overflowing_paused_event_queue_fails_boundedly_and_cleans_our_debugger() {
        let (result, commands, paused) = run_capture_case(CaptureCase::QueueOverflow).await;
        assert!(format!("{:#}", result.unwrap_err()).contains("bounded capacity"));
        assert_eq!(commands.last().unwrap(), "Debugger.disable");
        assert!(!paused);
    }

    #[test]
    fn frame_capture_expression_has_real_javascript_success_and_exception_semantics() {
        let expression = serde_json::to_string(APP_SERVER_CLIENT_CAPTURE_ON_CALL_FRAME).unwrap();
        let script = format!(
            r#"
const assert = require('node:assert/strict');
const expression = {expression};
function run(receiver, target) {{
  globalThis.window = target;
  return function() {{ return eval(expression); }}.call(receiver);
}}
class AppServerClient {{sendRequest() {{}}}}
const published = {{}};
assert.equal(run(new AppServerClient(), published), true);
assert.equal(published.__codexPlusAppServerClientClass, AppServerClient);
const ordinary = {{}};
assert.equal(run({{sendRequest() {{}}}}, ordinary), false);
assert.equal(ordinary.__codexPlusAppServerClientClass, undefined);
class NoRequestClient {{}}
const wrongClass = {{}};
assert.equal(run(new NoRequestClient(), wrongClass), false);
assert.equal(wrongClass.__codexPlusAppServerClientClass, undefined);
class OwnRequestClient {{constructor() {{this.sendRequest = () => {{}};}}}}
const ownMethod = {{}};
assert.equal(run(new OwnRequestClient(), ownMethod), true);
assert.equal(ownMethod.__codexPlusAppServerClientClass, OwnRequestClient);
const missing = {{}};
assert.equal(run(Object.create(null), missing), false);
assert.equal(missing.__codexPlusAppServerClientClass, undefined);
const throwingConstructor = Object.defineProperty({{}}, 'constructor', {{get() {{throw Error('unavailable');}}}});
assert.equal(run(throwingConstructor, {{}}), false);
const throwingSetter = Object.defineProperty({{}}, '__codexPlusAppServerClientClass', {{set() {{throw Error('readonly');}}}});
assert.equal(run(new AppServerClient(), throwingSetter), false);
assert.equal(run(new AppServerClient(), null), false);
"#
        );
        let output = std::process::Command::new("node")
            .arg("--eval")
            .arg(script)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[tokio::test]
    async fn paused_event_before_breakpoint_reply_is_preserved_for_capture() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let command: Value =
                serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap())
                    .unwrap();
            assert_eq!(command["method"], "Debugger.setBreakpointByUrl");
            for message in [
                json!({ "method": "Debugger.paused", "params": { "hitBreakpoints": ["capture-1"], "callFrames": [{ "callFrameId": "frame-1" }] } }),
                json!({ "id": command["id"], "result": { "breakpointId": "capture-1" } }),
            ] {
                socket
                    .send(Message::Text(message.to_string().into()))
                    .await
                    .unwrap();
            }
            let _ = socket.next().await;
        });
        let socket = connect_cdp_websocket(&format!("ws://{address}/devtools/page/early-pause"))
            .await
            .unwrap();
        let mut session = CdpSession::new(socket);
        let reply = session
            .send_command(1, "Debugger.setBreakpointByUrl", json!({}))
            .await
            .unwrap();
        assert_eq!(
            reply.pointer("/result/breakpointId"),
            Some(&json!("capture-1"))
        );
        let paused = tokio::time::timeout(Duration::from_millis(200), session.next_message())
            .await
            .expect("等待断点回包不能丢失已经暂停的事件")
            .unwrap()
            .unwrap();
        assert_eq!(paused["method"], "Debugger.paused");
        assert_eq!(
            paused.pointer("/params/callFrames/0/callFrameId"),
            Some(&json!("frame-1"))
        );
        session.close().await;
        server.await.unwrap();
    }

    #[tokio::test]
    async fn capture_enable_retries_timeout_and_accepts_late_response() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let generation = next_bridge_generation(&format!("capture-retry-{address}"));
        assert!(publish_bridge_generation(&generation));
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let first: Value =
                serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap())
                    .unwrap();
            let second: Value =
                serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap())
                    .unwrap();
            assert_eq!(first["method"], "Debugger.enable");
            assert_eq!(second["method"], "Debugger.enable");
            assert_ne!(first["id"], second["id"]);
            // 第一条回包迟到，不能误认为第二条完成，也不能阻止真正的第二条响应。
            for command in [first, second] {
                socket
                    .send(Message::Text(
                        json!({ "id": command["id"], "result": {"debuggerId": "mock"} })
                            .to_string()
                            .into(),
                    ))
                    .await
                    .unwrap();
            }
        });
        let socket = connect_cdp_websocket(&format!("ws://{address}/devtools/page/capture-retry"))
            .await
            .unwrap();
        let mut session = CdpSession::new(socket);
        assert!(
            enable_app_server_client_capture_debugger(
                &mut session,
                &generation,
                Duration::from_millis(40),
                Duration::from_millis(5),
            )
            .await
            .unwrap()
        );
        server.await.unwrap();
        release_bridge_generation(&generation);
    }

    #[tokio::test]
    async fn capture_enable_stops_after_bounded_attempts() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let generation = next_bridge_generation(&format!("capture-bounded-{address}"));
        assert!(publish_bridge_generation(&generation));
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            for _ in 0..APP_SERVER_CLIENT_CAPTURE_ENABLE_ATTEMPTS {
                let command: Value =
                    serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap())
                        .unwrap();
                assert_eq!(command["method"], "Debugger.enable");
            }
            // 保持连接，让最后一轮以超时结束，而不是断连。
            assert!(
                tokio::time::timeout(Duration::from_millis(100), socket.next())
                    .await
                    .is_err()
            );
        });
        let socket =
            connect_cdp_websocket(&format!("ws://{address}/devtools/page/capture-bounded"))
                .await
                .unwrap();
        let mut session = CdpSession::new(socket);
        let error = enable_app_server_client_capture_debugger(
            &mut session,
            &generation,
            Duration::from_millis(20),
            Duration::from_millis(5),
        )
        .await
        .unwrap_err();
        assert!(error.is::<tokio::time::error::Elapsed>());
        server.await.unwrap();
        release_bridge_generation(&generation);
    }

    #[tokio::test]
    async fn capture_enable_does_not_retry_protocol_error() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let generation = next_bridge_generation(&format!("capture-error-{address}"));
        assert!(publish_bridge_generation(&generation));
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let command: Value =
                serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap())
                    .unwrap();
            socket
                .send(Message::Text(
                    json!({ "id": command["id"], "error": { "code": -32000, "message": "unavailable" } })
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
            assert!(
                tokio::time::timeout(Duration::from_millis(100), socket.next())
                    .await
                    .is_err()
            );
        });
        let socket = connect_cdp_websocket(&format!("ws://{address}/devtools/page/capture-error"))
            .await
            .unwrap();
        let mut session = CdpSession::new(socket);
        let error = enable_app_server_client_capture_debugger(
            &mut session,
            &generation,
            Duration::from_secs(5),
            Duration::from_millis(5),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("unavailable"));
        server.await.unwrap();
        release_bridge_generation(&generation);
    }

    #[tokio::test]
    async fn capture_enable_cancels_when_generation_is_replaced() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let generation = next_bridge_generation(&format!("capture-stale-{address}"));
        assert!(publish_bridge_generation(&generation));
        let replacement = next_bridge_generation(&generation.target);
        let replacement_for_server = replacement.clone();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let command: Value =
                serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap())
                    .unwrap();
            assert_eq!(command["method"], "Debugger.enable");
            assert!(publish_bridge_generation(&replacement_for_server));
            // 旧会话退出前不得重发 enable 或安装断点。
            assert!(
                tokio::time::timeout(Duration::from_millis(1500), socket.next())
                    .await
                    .is_err()
            );
        });
        let socket = connect_cdp_websocket(&format!("ws://{address}/devtools/page/capture-stale"))
            .await
            .unwrap();
        let mut session = CdpSession::new(socket);
        assert!(
            !enable_app_server_client_capture_debugger(
                &mut session,
                &generation,
                Duration::from_secs(5),
                Duration::from_millis(5),
            )
            .await
            .unwrap()
        );
        server.await.unwrap();
        release_bridge_generation(&replacement);
    }
}

#[cfg(test)]
mod websocket_size_tests {
    use super::*;

    #[test]
    fn cdp_websocket_limits_fit_maximum_dictation_payload_and_remain_bounded() {
        let config = cdp_websocket_config();
        let base64_bytes = crate::dictation::MAX_AUDIO_BODY_BYTES.div_ceil(3) * 4;
        assert_eq!(config.max_message_size, Some(40 * 1024 * 1024));
        assert_eq!(config.max_frame_size, Some(40 * 1024 * 1024));
        assert!(base64_bytes + 64 * 1024 < config.max_frame_size.unwrap());
    }

    #[tokio::test]
    async fn cdp_websocket_receives_maximum_dictation_base64_in_one_frame() {
        // 仅连接本地模拟服务器，不操作真实 app 或页面。
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let base64_bytes = crate::dictation::MAX_AUDIO_BODY_BYTES.div_ceil(3) * 4;
        let payload = json!({
            "method": "Runtime.bindingCalled",
            "params": {
                "payload": json!({
                    "id": "dictation-smoke",
                    "path": "/dictation/transcribe",
                    "payload": {"audioBase64": "A".repeat(base64_bytes)}
                }).to_string()
            }
        })
        .to_string();
        let payload_bytes = payload.len();
        assert!(payload_bytes > 16 * 1024 * 1024);
        assert!(payload_bytes < CDP_MAX_MESSAGE_BYTES);
        let sender = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            socket.send(Message::Text(payload.into())).await.unwrap();
        });
        let mut socket =
            connect_cdp_websocket(&format!("ws://{address}/devtools/page/dictation-smoke"))
                .await
                .unwrap();
        let received = tokio::time::timeout(Duration::from_secs(15), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(received.is_text());
        assert_eq!(received.len(), payload_bytes);
        sender.await.unwrap();
    }
}
