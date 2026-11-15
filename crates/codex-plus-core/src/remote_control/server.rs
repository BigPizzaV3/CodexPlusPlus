//! 远程控制的 HTTP / WebSocket 入口。
//!
//! P1 只监听 127.0.0.1。所有 `/api/*` 都要带令牌（`Authorization: Bearer`，WebSocket 用 `?token=`）；
//! Host 只认本机回环地址（防 DNS 重绑定），带 Origin 的请求必须与自身同源（防跨站请求）。
//! 网页本身（`/`）不含任何数据，令牌放在 URL 片段 `#token=` 里，不会出现在请求和日志里。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, bail};
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use sha1::{Digest, Sha1};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::Role;

use super::service::RemoteService;

const MAX_HEADER_BYTES: usize = 16 * 1024;
const MAX_BODY_BYTES: usize = 1024 * 1024;
const PAGE: &str = include_str!("../../assets/remote-control/index.html");
/// 推送合并窗口：流式输出时状态每秒变几十次，合并后再推。
const PUSH_COALESCE: Duration = Duration::from_millis(80);

pub struct ServerContext {
    pub service: Arc<RemoteService>,
    pub token: String,
    pub port: u16,
}

pub async fn serve(listener: TcpListener, context: Arc<ServerContext>) {
    loop {
        let Ok((stream, _)) = listener.accept().await else { continue };
        let context = context.clone();
        tokio::spawn(async move {
            let _ = handle_connection(stream, context).await;
        });
    }
}

#[derive(Debug)]
pub struct HttpRequest {
    pub method: String,
    pub path: String,
    pub query: HashMap<String, String>,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl HttpRequest {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }
}

async fn read_request(stream: &mut TcpStream) -> anyhow::Result<HttpRequest> {
    let mut buffer = Vec::with_capacity(2048);
    let header_end = loop {
        let mut chunk = [0u8; 2048];
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            bail!("连接已关闭");
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(position) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break position;
        }
        if buffer.len() > MAX_HEADER_BYTES {
            bail!("请求头过大");
        }
    };
    let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let mut body = buffer[header_end + 4..].to_vec();
    let request = parse_head(&head)?;
    let length = request
        .headers
        .get("content-length")
        .map(|value| value.parse::<usize>())
        .transpose()
        .context("Content-Length 无效")?
        .unwrap_or(0);
    if length > MAX_BODY_BYTES {
        bail!("请求体过大");
    }
    while body.len() < length {
        let mut chunk = vec![0u8; (length - body.len()).min(64 * 1024)];
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            bail!("请求体不完整");
        }
        body.extend_from_slice(&chunk[..read]);
    }
    body.truncate(length);
    Ok(HttpRequest { body, ..request })
}

pub fn parse_head(head: &str) -> anyhow::Result<HttpRequest> {
    let mut lines = head.split("\r\n");
    let request_line = lines.next().context("缺少请求行")?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().context("缺少方法")?.to_string();
    let target = parts.next().context("缺少路径")?;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let query = query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .filter_map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            Some((percent_decode(key)?, percent_decode(value)?))
        })
        .collect();
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_string()))
        .collect();
    Ok(HttpRequest { method, path: path.to_string(), query, headers, body: Vec::new() })
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok()?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                index += 3;
            }
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

/// 只认本机回环 Host：DNS 重绑定的网页拿到的 Host 是它自己的域名。
pub fn host_allowed(host: Option<&str>, port: u16) -> bool {
    let Some(host) = host else { return false };
    [format!("127.0.0.1:{port}"), format!("localhost:{port}"), format!("[::1]:{port}")]
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(host))
}

/// 带 Origin 时必须与自身同源。
pub fn origin_allowed(origin: Option<&str>, host: &str) -> bool {
    match origin {
        None => true,
        Some(origin) => origin.eq_ignore_ascii_case(&format!("http://{host}")),
    }
}

/// 常量时间比较，避免按响应时间猜令牌。
pub fn token_matches(expected: &str, provided: &str) -> bool {
    if expected.is_empty() || expected.len() != provided.len() {
        return false;
    }
    expected.bytes().zip(provided.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

fn provided_token(request: &HttpRequest) -> Option<String> {
    if let Some(value) = request.header("authorization") {
        return value.strip_prefix("Bearer ").map(str::to_string);
    }
    request.query.get("token").cloned()
}

async fn handle_connection(mut stream: TcpStream, context: Arc<ServerContext>) -> anyhow::Result<()> {
    let request = match read_request(&mut stream).await {
        Ok(request) => request,
        Err(error) => return write_json(&mut stream, 400, &json!({ "ok": false, "error": error.to_string() })).await,
    };
    let host = request.header("host").map(str::to_string);
    if !host_allowed(host.as_deref(), context.port) {
        return write_json(&mut stream, 421, &json!({ "ok": false, "error": "Host 不被允许" })).await;
    }
    let host = host.unwrap_or_default();
    if request.method == "GET" && (request.path == "/" || request.path == "/index.html") {
        return write_page(&mut stream, &host).await;
    }
    if !request.path.starts_with("/api/") {
        return write_json(&mut stream, 404, &json!({ "ok": false, "error": "not found" })).await;
    }
    if !origin_allowed(request.header("origin"), &host) {
        return write_json(&mut stream, 403, &json!({ "ok": false, "error": "Origin 不被允许" })).await;
    }
    if !provided_token(&request).is_some_and(|token| token_matches(&context.token, &token)) {
        return write_json(&mut stream, 401, &json!({ "ok": false, "error": "令牌无效，请从 Codex++ 重新打开远程控制页面" })).await;
    }
    let segments: Vec<&str> = request.path.trim_start_matches("/api/").split('/').filter(|s| !s.is_empty()).collect();
    if let ["threads", thread_id, "stream"] = segments.as_slice() {
        if request.header("upgrade").is_some_and(|value| value.eq_ignore_ascii_case("websocket")) {
            let thread_id = valid_thread_id(thread_id)?;
            return stream_thread(stream, &request, context, thread_id).await;
        }
    }
    let (status, body) = match route(&request, &segments, &context).await {
        Ok(value) => (200, json!({ "ok": true, "data": value })),
        Err(error) => (400, json!({ "ok": false, "error": error.to_string() })),
    };
    if request.method == "POST" {
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "remote_control.action",
            json!({ "path": request.path, "ok": status == 200, "error": body.get("error") }),
        );
    }
    write_json(&mut stream, status, &body).await
}

fn valid_thread_id(text: &str) -> anyhow::Result<String> {
    Ok(uuid::Uuid::parse_str(text).context("会话 ID 无效")?.to_string())
}

fn json_body(request: &HttpRequest) -> anyhow::Result<Value> {
    if request.body.is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_slice(&request.body).context("请求体不是 JSON")
}

fn text_field(body: &Value, key: &str) -> anyhow::Result<String> {
    let text = body.get(key).and_then(Value::as_str).unwrap_or_default().trim().to_string();
    if text.is_empty() {
        bail!("缺少 {key}");
    }
    Ok(text)
}

async fn route(request: &HttpRequest, segments: &[&str], context: &ServerContext) -> anyhow::Result<Value> {
    let service = &context.service;
    match (request.method.as_str(), segments) {
        ("GET", ["status"]) => Ok(service.status().await),
        ("GET", ["threads"]) => service.list_threads(request.query.get("cursor").map(String::as_str)).await,
        ("POST", ["threads"]) => {
            let body = json_body(request)?;
            let cwd = text_field(&body, "cwd")?;
            let name = body.get("name").and_then(Value::as_str).filter(|name| !name.trim().is_empty()).unwrap_or("远程会话");
            let thread_id = service.create_thread(&cwd, name).await?;
            if let Some(text) = body.get("text").and_then(Value::as_str).filter(|text| !text.trim().is_empty()) {
                service.send(&thread_id, text).await?;
            }
            Ok(json!({ "threadId": thread_id }))
        }
        ("GET", ["threads", id]) => service.thread_view(&valid_thread_id(id)?).await,
        ("POST", ["threads", id, "open"]) => Ok(service.open_thread(&valid_thread_id(id)?).await?.view()),
        ("POST", ["threads", id, "messages"]) => {
            let text = text_field(&json_body(request)?, "text")?;
            service.send(&valid_thread_id(id)?, &text).await
        }
        ("POST", ["threads", id, "interrupt"]) => service.interrupt(&valid_thread_id(id)?).await,
        ("POST", ["threads", id, "requests", request_id]) => {
            let request_id: Value = serde_json::from_str(request_id).unwrap_or_else(|_| json!(request_id));
            service.respond(&valid_thread_id(id)?, &request_id, &json_body(request)?).await
        }
        _ => bail!("不支持的接口：{} {}", request.method, request.path),
    }
}

pub fn websocket_accept(key: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(key.as_bytes());
    hasher.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    base64::engine::general_purpose::STANDARD.encode(hasher.finalize())
}

async fn stream_thread(
    mut stream: TcpStream,
    request: &HttpRequest,
    context: Arc<ServerContext>,
    thread_id: String,
) -> anyhow::Result<()> {
    let key = request.header("sec-websocket-key").context("缺少 Sec-WebSocket-Key")?;
    let handshake = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
        websocket_accept(key)
    );
    stream.write_all(handshake.as_bytes()).await?;
    let mut socket = WebSocketStream::from_raw_socket(stream, Role::Server, None).await;
    let follower = match context.service.follower(&thread_id).await {
        Ok(Some(follower)) => follower,
        Ok(None) => {
            let _ = socket.send(Message::Text(json!({ "type": "closed", "reason": "会话没有在桌面打开" }).to_string().into())).await;
            return Ok(());
        }
        Err(error) => {
            let _ = socket.send(Message::Text(json!({ "type": "error", "error": error.to_string() }).to_string().into())).await;
            return Ok(());
        }
    };
    let mut updates = follower.subscribe();
    socket.send(Message::Text(json!({ "type": "view", "view": follower.view() }).to_string().into())).await?;
    loop {
        tokio::select! {
            changed = updates.changed() => {
                if changed.is_err() {
                    break;
                }
                tokio::time::sleep(PUSH_COALESCE).await;
                updates.borrow_and_update();
                if !follower.is_alive() {
                    let _ = socket.send(Message::Text(json!({ "type": "closed", "reason": "桌面不再持有这个会话" }).to_string().into())).await;
                    break;
                }
                socket.send(Message::Text(json!({ "type": "view", "view": follower.view() }).to_string().into())).await?;
            }
            incoming = socket.next() => {
                match incoming {
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    _ => {}
                }
            }
        }
    }
    let _ = socket.close(None).await;
    Ok(())
}

fn status_text(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        421 => "Misdirected Request",
        _ => "Error",
    }
}

async fn write_json(stream: &mut TcpStream, status: u16, body: &Value) -> anyhow::Result<()> {
    let body = serde_json::to_vec(body)?;
    let head = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        status_text(status),
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(&body).await?;
    stream.shutdown().await?;
    Ok(())
}

async fn write_page(stream: &mut TcpStream, host: &str) -> anyhow::Result<()> {
    let csp = format!(
        "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data:; connect-src 'self' ws://{host}; base-uri 'none'; form-action 'none'; frame-ancestors 'none'"
    );
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nContent-Security-Policy: {csp}\r\nReferrer-Policy: no-referrer\r\nX-Frame-Options: DENY\r\nConnection: close\r\n\r\n",
        PAGE.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(PAGE.as_bytes()).await?;
    stream.shutdown().await?;
    Ok(())
}
