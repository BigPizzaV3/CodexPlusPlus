//! 桌面镜像：把 Codex 桌面窗口的画面经 CDP 投屏（`Page.startScreencast`）推给网页，
//! 网页的鼠标 / 滚轮 / 按键 / 文字再经 `Input.*` 发回去。效果像只投 Codex 一个窗口的远程桌面。
//!
//! - 调试端口不对外：网页只连我们已认证的 `/api/mirror`，这里替它连本机的 CDP，只转发画面和白名单内的输入。
//! - 输入法用的是网页那一端设备的：网页把「已确定的文字」发来，这里用 `Input.insertText` 写进 Codex 当前聚焦处，
//!   不经过 Mac 的输入法，中文 / 语音输入都能用；回车、退格、方向键等单独作为按键发。
//! - 画面是 JPEG 二进制帧；尺寸等元数据变化时另发一条 JSON。Chromium 出一帧就确认，往网页只发最新一帧：
//!   网络慢时丢掉没来得及发的旧帧，画面不会越积越落后。
//! - 端到端流控：网页每画完一帧回 `frameAck`，路上最多 [`MAX_FRAMES_IN_FLIGHT`] 帧。光看「写进 socket」不够：
//!   系统的发送缓冲能攒下好几帧，慢网下画面会落后零点几秒；等网页确认才发下一帧，网快就快、网慢就少发。
//! - 它就是桌面上那个窗口：网页上的操作桌面同步可见。
//! - 镜像期间打开 `Emulation.setFocusEmulationEnabled`：Codex 窗口失焦、被挡住、最小化时，页面仍当自己可见且有焦点，
//!   照常出画面、收键盘（Chromium 会给页面加一个「正在被捕获」的计数，不再按隐藏处理）。连接断开后 Chromium 自动撤销。
//!   万一仍然没画面（推 `visibility: false`），网页给出提示。

use std::time::Duration;

use anyhow::{Context, bail};
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::{Bytes, Message};

const MAX_TEXT_CHARS: usize = 20_000;
const MAX_KEY_FIELD_CHARS: usize = 32;
const MAX_FRAME_SIDE: u32 = 2560;
const MIN_FRAME_SIDE: u32 = 200;
const JPEG_QUALITY: u32 = 60;
const REVALIDATE_EVERY: Duration = Duration::from_secs(3);
const REVOKED: &str = "revoked";
/// 往网页发画面的最短间隔（约 60 帧/秒）：再快肉眼看不出，白占带宽；慢网由下面的流控自然降帧。
const MIN_FRAME_INTERVAL: Duration = Duration::from_millis(16);
/// 网页还没确认的帧最多几帧：2 帧让传输和网页解码能重叠，又不会攒出延迟。
pub const MAX_FRAMES_IN_FLIGHT: usize = 2;
/// 等网页确认的最长时间：老页面不回确认、或者确认丢了，也不至于停住。
const ACK_TIMEOUT: Duration = Duration::from_millis(800);

/// macOS 上 CDP 按键事件触发编辑命令要带 `commands`（Cmd+A 全选之类）；只放这些。
const ALLOWED_COMMANDS: &[&str] = &[
    "selectAll",
    "undo",
    "redo",
    "copy",
    "cut",
    "moveToBeginningOfLine",
    "moveToEndOfLine",
    "moveToBeginningOfLineAndModifySelection",
    "moveToEndOfLineAndModifySelection",
    "moveWordLeft",
    "moveWordRight",
    "moveWordLeftAndModifySelection",
    "moveWordRightAndModifySelection",
    "moveToBeginningOfDocument",
    "moveToEndOfDocument",
    "deleteWordBackward",
    "deleteWordForward",
    "deleteToBeginningOfLine",
];

/// 网页发来的一条输入，校验后转成 CDP 命令 `(method, params)`。不认识或越界的返回错误。
pub fn input_command(message: &Value) -> anyhow::Result<(&'static str, Value)> {
    let number = |key: &str| -> anyhow::Result<f64> {
        let value = message.get(key).and_then(Value::as_f64).with_context(|| format!("缺少 {key}"))?;
        if !value.is_finite() {
            bail!("{key} 无效");
        }
        Ok(value)
    };
    let coordinate = |key: &str| -> anyhow::Result<f64> { Ok(number(key)?.clamp(0.0, 20_000.0)) };
    let modifiers = message.get("modifiers").and_then(Value::as_u64).unwrap_or(0) & 0b1111;
    let short = |key: &str| -> anyhow::Result<Option<String>> {
        match message.get(key).and_then(Value::as_str) {
            Some(text) if text.chars().count() > MAX_KEY_FIELD_CHARS => bail!("{key} 过长"),
            Some(text) => Ok(Some(text.to_string())),
            None => Ok(None),
        }
    };
    match message.get("type").and_then(Value::as_str).unwrap_or_default() {
        "mouse" => {
            let event = match message.get("event").and_then(Value::as_str) {
                Some(event @ ("mousePressed" | "mouseReleased" | "mouseMoved")) => event,
                _ => bail!("鼠标事件类型无效"),
            };
            let button = match message.get("button").and_then(Value::as_str).unwrap_or("none") {
                button @ ("none" | "left" | "middle" | "right") => button,
                _ => bail!("鼠标按键无效"),
            };
            Ok((
                "Input.dispatchMouseEvent",
                json!({
                    "type": event,
                    "x": coordinate("x")?,
                    "y": coordinate("y")?,
                    "button": button,
                    "buttons": message.get("buttons").and_then(Value::as_u64).unwrap_or(0) & 0b111,
                    "clickCount": message.get("clickCount").and_then(Value::as_u64).unwrap_or(0).min(3),
                    "modifiers": modifiers,
                }),
            ))
        }
        "wheel" => Ok((
            "Input.dispatchMouseEvent",
            json!({
                "type": "mouseWheel",
                "x": coordinate("x")?,
                "y": coordinate("y")?,
                "deltaX": number("deltaX")?.clamp(-5000.0, 5000.0),
                "deltaY": number("deltaY")?.clamp(-5000.0, 5000.0),
                "modifiers": modifiers,
            }),
        )),
        "text" => {
            let text = message.get("text").and_then(Value::as_str).unwrap_or_default();
            if text.is_empty() {
                bail!("文字为空");
            }
            if text.chars().count() > MAX_TEXT_CHARS {
                bail!("一次最多输入 {MAX_TEXT_CHARS} 个字");
            }
            Ok(("Input.insertText", json!({ "text": text })))
        }
        "key" => {
            let event = match message.get("event").and_then(Value::as_str) {
                Some(event @ ("keyDown" | "rawKeyDown" | "keyUp")) => event,
                _ => bail!("按键事件类型无效"),
            };
            let mut params = json!({ "type": event, "modifiers": modifiers });
            for field in ["key", "code", "text"] {
                if let Some(value) = short(field)? {
                    params[field] = json!(value);
                }
            }
            if let Some(code) = message.get("keyCode").and_then(Value::as_u64) {
                params["windowsVirtualKeyCode"] = json!(code.min(255));
                params["nativeVirtualKeyCode"] = json!(code.min(255));
            }
            if let Some(commands) = message.get("commands").and_then(Value::as_array) {
                let commands: Vec<&str> = commands.iter().filter_map(Value::as_str).collect();
                if commands.iter().any(|command| !ALLOWED_COMMANDS.contains(command)) {
                    bail!("不支持的编辑命令");
                }
                params["commands"] = json!(commands);
            }
            Ok(("Input.dispatchKeyEvent", params))
        }
        other => bail!("不支持的输入：{other}"),
    }
}

/// 投屏参数：按网页画布大小要画面，限制在合理范围。
pub fn screencast_params(message: &Value) -> Value {
    let side = |key: &str, default: u32| {
        message
            .get(key)
            .and_then(Value::as_u64)
            .map(|value| (value as u32).clamp(MIN_FRAME_SIDE, MAX_FRAME_SIDE))
            .unwrap_or(default)
    };
    json!({
        "format": "jpeg",
        "quality": JPEG_QUALITY,
        "maxWidth": side("width", 1600),
        "maxHeight": side("height", 1000),
        "everyNthFrame": 1,
    })
}

/// 端到端流控：记着网页还没确认的帧数。网页在 `start` 里声明 `acks: true` 才启用（老页面不回确认，照旧发）。
#[derive(Default)]
pub struct FrameFlow {
    enabled: std::sync::atomic::AtomicBool,
    in_flight: std::sync::atomic::AtomicUsize,
    room: tokio::sync::Notify,
}

impl FrameFlow {
    pub fn reset(&self, enabled: bool) {
        use std::sync::atomic::Ordering;
        self.enabled.store(enabled, Ordering::SeqCst);
        self.in_flight.store(0, Ordering::SeqCst);
        self.room.notify_waiters();
    }

    pub fn acked(&self) {
        use std::sync::atomic::Ordering;
        let _ = self.in_flight.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |count| count.checked_sub(1));
        self.room.notify_waiters();
    }

    /// 有空位就占一个返回；没有就等确认，最多等 [`ACK_TIMEOUT`]（超时也发，并把计数清零，免得一直卡住）。
    pub async fn wait_for_room(&self) {
        use std::sync::atomic::Ordering;
        let deadline = tokio::time::Instant::now() + ACK_TIMEOUT;
        loop {
            let notified = self.room.notified();
            if !self.enabled.load(Ordering::SeqCst) || self.in_flight.load(Ordering::SeqCst) < MAX_FRAMES_IN_FLIGHT {
                break;
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                self.in_flight.store(0, Ordering::SeqCst);
                break;
            }
        }
        if self.enabled.load(Ordering::SeqCst) {
            self.in_flight.fetch_add(1, Ordering::SeqCst);
        }
    }
}

/// 跑一个镜像会话，直到网页断开、Codex 页面关闭或设备被吊销。
pub async fn run<S, F>(mut socket: WebSocketStream<S>, debug_port: u16, still_authorized: F) -> anyhow::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
    F: Fn() -> bool,
{
    let send = |value: Value| Message::Text(value.to_string().into());
    let target = match crate::cdp::list_targets(debug_port).await.and_then(|targets| crate::cdp::pick_injectable_codex_page_target(&targets)) {
        Ok(target) => target,
        Err(_) => {
            let _ = socket
                .send(send(json!({ "type": "error", "error": "连不上 Codex 桌面：请从 Codex++ 启动 Codex（需要它的调试端口），并保持窗口打开" })))
                .await;
            return Ok(());
        }
    };
    let websocket_url = target.web_socket_debugger_url.context("Codex 页面没有调试地址")?;
    let mut cdp = crate::bridge::connect_cdp_websocket(&websocket_url).await?;
    let mut next_id: u64 = 1;
    let mut command = |method: &str, params: Value| {
        let id = next_id;
        next_id += 1;
        Message::Text(json!({ "id": id, "method": method, "params": params }).to_string().into())
    };
    socket.send(send(json!({ "type": "ready", "title": target.title }))).await?;
    // 窗口失焦 / 最小化也照常出画面、接收输入。
    cdp.send(command("Emulation.setFocusEmulationEnabled", json!({ "enabled": true }))).await?;
    // 读写分开：画面只留最新一帧（watch 会盖掉还没发出去的旧帧），网络慢时丢帧而不是越积越多、越来越卡。
    let (mut sink, mut stream) = socket.split();
    let (text_tx, mut text_rx) = tokio::sync::mpsc::unbounded_channel::<Value>();
    let (frame_tx, mut frame_rx) = tokio::sync::watch::channel(Bytes::new());
    let flow = FrameFlow::default();
    let writer = async {
        let mut next_frame_at = tokio::time::Instant::now();
        loop {
            let message = tokio::select! {
                // 文字消息（尺寸、提示）先发，保证画面帧之前对端已拿到新尺寸。
                biased;
                text = text_rx.recv() => match text {
                    Some(value) => send(value),
                    None => break,
                },
                changed = frame_rx.changed() => match changed {
                    Ok(()) => {
                        // 没到间隔先等一下，等完取那时最新的一帧（期间来的旧帧直接跳过）。
                        tokio::time::sleep_until(next_frame_at).await;
                        // 按节拍排下一帧，而不是从现在起算：等新帧到来的那点时间不累加进去，平均才到得了 30 帧。
                        next_frame_at = (next_frame_at + MIN_FRAME_INTERVAL).max(tokio::time::Instant::now());
                        // 网页还有两帧没画完：等它确认（期间新来的帧继续覆盖，等完发的是最新那帧）。
                        flow.wait_for_room().await;
                        Message::Binary(frame_rx.borrow_and_update().clone())
                    }
                    Err(_) => break,
                },
            };
            if sink.send(message).await.is_err() {
                break;
            }
        }
    };
    let reader = async {
        let mut meta: Option<Value> = None;
        let mut revalidate = tokio::time::interval(REVALIDATE_EVERY);
        loop {
            tokio::select! {
                incoming = stream.next() => {
                    let text = match incoming {
                        Some(Ok(Message::Text(text))) => text,
                        Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break None,
                        _ => continue,
                    };
                    let Ok(message) = serde_json::from_str::<Value>(&text) else { continue };
                    let sent = match message.get("type").and_then(Value::as_str) {
                        Some("frameAck") => {
                            flow.acked();
                            Ok(())
                        }
                        Some("start") => {
                            flow.reset(message.get("acks").and_then(Value::as_bool).unwrap_or(false));
                            let stopped = cdp.send(command("Page.stopScreencast", json!({}))).await;
                            stopped.and(cdp.send(command("Page.startScreencast", screencast_params(&message))).await)
                        }
                        Some("stop") => cdp.send(command("Page.stopScreencast", json!({}))).await,
                        _ => match input_command(&message) {
                            Ok((method, params)) => cdp.send(command(method, params)).await,
                            Err(error) => {
                                let _ = text_tx.send(json!({ "type": "warning", "error": error.to_string() }));
                                Ok(())
                            }
                        },
                    };
                    if sent.is_err() {
                        break Some("Codex 页面关闭或刷新了");
                    }
                }
                event = cdp.next() => {
                    let text = match event {
                        Some(Ok(Message::Text(text))) => text,
                        Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break Some("Codex 页面关闭或刷新了"),
                        _ => continue,
                    };
                    let Ok(event) = serde_json::from_str::<Value>(&text) else { continue };
                    match event.get("method").and_then(Value::as_str) {
                        Some("Page.screencastFrame") => {
                            let params = event.get("params").cloned().unwrap_or(Value::Null);
                            // 收到就确认：节奏由 Chromium 出帧决定，发不过去的旧帧在 writer 那里被新帧盖掉。
                            if let Some(session_id) = params.get("sessionId") {
                                if cdp.send(command("Page.screencastFrameAck", json!({ "sessionId": session_id }))).await.is_err() {
                                    break Some("Codex 页面关闭或刷新了");
                                }
                            }
                            let frame_meta = params.get("metadata").cloned().unwrap_or(Value::Null);
                            let size = json!({
                                "deviceWidth": frame_meta.get("deviceWidth"),
                                "deviceHeight": frame_meta.get("deviceHeight"),
                                "offsetTop": frame_meta.get("offsetTop"),
                                "pageScaleFactor": frame_meta.get("pageScaleFactor"),
                            });
                            if meta.as_ref() != Some(&size) {
                                let _ = text_tx.send(json!({ "type": "meta", "meta": size }));
                                meta = Some(size);
                            }
                            if let Some(data) = params.get("data").and_then(Value::as_str) {
                                if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(data) {
                                    let _ = frame_tx.send(Bytes::from(bytes));
                                }
                            }
                        }
                        Some("Page.screencastVisibilityChanged") => {
                            let visible = event.pointer("/params/visible").and_then(Value::as_bool).unwrap_or(true);
                            let _ = text_tx.send(json!({ "type": "visibility", "visible": visible }));
                        }
                        _ => {}
                    }
                }
                _ = revalidate.tick() => {
                    if !still_authorized() {
                        break Some(REVOKED);
                    }
                }
            }
        }
    };
    // 任一边结束（网页断开、写不出去、Codex 页面没了、设备被吊销）就收尾。
    let reason = tokio::select! {
        reason = reader => reason,
        _ = writer => None,
    };
    let _ = cdp.send(command("Page.stopScreencast", json!({}))).await;
    let _ = cdp.send(command("Emulation.setFocusEmulationEnabled", json!({ "enabled": false }))).await;
    let _ = cdp.close(None).await;
    let mut socket = sink.reunite(stream).map_err(|_| anyhow::anyhow!("镜像连接收尾失败"))?;
    match reason {
        Some(REVOKED) => {
            let _ = socket.send(send(json!({ "type": "revoked", "reason": "这台设备已被移除" }))).await;
        }
        Some(reason) => {
            let _ = socket.send(send(json!({ "type": "closed", "reason": reason, "reconnect": true }))).await;
        }
        None => {}
    }
    let _ = socket.close(None).await;
    Ok(())
}
