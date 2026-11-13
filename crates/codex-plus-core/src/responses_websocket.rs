//! 标准 Responses WS 传输；请求准备仍走 protocol_proxy / gateway。
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, bail};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{Message, protocol::Role},
};

use crate::gateway::{
    key_pool::KeyRef,
    route::{GatewayCandidate, RoutePlan},
};

type UpstreamSocket = WebSocketStream<reqwest::Upgraded>;

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub(crate) struct Unsupported(pub(crate) &'static str);

#[derive(Clone)]
struct Affinity {
    connection: String,
    relay: String,
    model: String,
    key: Option<KeyRef>,
}

#[derive(Default)]
struct Registry {
    // 指纹只用于复用；响应亲和绑定具体连接实例，重连不能恢复 connection-local cache。
    connections: HashMap<String, String>,
    locks: HashMap<String, Arc<tokio::sync::Mutex<()>>>,
    responses: HashMap<String, Affinity>,
}

pub(crate) struct RequestContext {
    registry: Arc<Mutex<Registry>>,
    previous: Option<Affinity>,
}

tokio::task_local! {
    pub(crate) static REQUEST: RequestContext;
}

pub(crate) fn active() -> bool {
    REQUEST.try_with(|_| ()).is_ok()
}

pub(crate) fn pinned_key() -> Option<KeyRef> {
    REQUEST
        .try_with(|ctx| ctx.previous.as_ref().and_then(|p| p.key.clone()))
        .ok()
        .flatten()
}

pub(crate) fn pinned_relay() -> Option<String> {
    REQUEST
        .try_with(|ctx| ctx.previous.as_ref().map(|p| p.relay.clone()))
        .ok()
        .flatten()
}

/// 原 Key 必须在路由收集阶段筛选，保留它的冷却候选，再由既有队列等名额。
pub(crate) fn restrict_plan(plan: &mut RoutePlan) -> anyhow::Result<()> {
    if pinned_key().is_some() && plan.candidates.is_empty() && plan.waiting.is_empty() {
        return Err(Unsupported(
            "WS 续接的原供应商或 Key 已不在当前模型路由中；请使用完整上下文开始新响应",
        )
        .into());
    }
    Ok(())
}

pub(crate) struct PreparedWebsocket {
    connection: String,
    fingerprint: String,
    relay: String,
    body: Value,
    socket: Option<UpstreamSocket>,
    // 同指纹首次握手串行，其他指纹及 Key 排队互不阻塞；注册连接之后释放。
    registration: Option<tokio::sync::OwnedMutexGuard<()>>,
    pub(crate) headers: reqwest::header::HeaderMap,
    pub(crate) gateway: Option<(RoutePlan, GatewayCandidate)>,
    pub(crate) rotation: Option<crate::settings::BackendSettings>,
    pub(crate) recorder: Option<crate::gateway::activity::RouteRecorder>,
    pub(crate) busy: Option<crate::gateway::activity::InFlight>,
}

pub(crate) enum PreparedUpstream {
    Websocket(Box<PreparedWebsocket>),
    Rejected(reqwest::Response),
}

fn connection_id(relay_id: &str, request: &reqwest::Request) -> String {
    let mut hash = Sha256::new();
    hash.update(relay_id.as_bytes());
    hash.update(request.url().as_str().as_bytes());
    let mut headers: Vec<_> = request
        .headers()
        .iter()
        .filter(|(name, _)| {
            !matches!(
                name.as_str(),
                "content-type"
                    | "content-length"
                    | "accept"
                    | "cache-control"
                    | "connection"
                    | "upgrade"
                    | "sec-websocket-key"
                    | "sec-websocket-version"
                    | "sec-websocket-protocol"
                    | "sec-websocket-extensions"
            ) && !crate::protocol_proxy::is_volatile_client_identity_header(name.as_str())
        })
        .collect();
    headers.sort_by_key(|(name, _)| name.as_str());
    for (name, value) in headers {
        hash.update(name.as_str().as_bytes());
        hash.update([0]);
        hash.update(value.as_bytes());
        hash.update([0]);
    }
    format!("{:x}", hash.finalize())
}

/// reqwest 的升级沿用现有供应商代理、系统 CA 与自定义认证，不另建一套 TLS 栈。
pub(crate) async fn prepare(
    relay_id: &str,
    mut builder: reqwest::RequestBuilder,
    body: Value,
    timeout: Duration,
) -> anyhow::Result<PreparedUpstream> {
    for (name, value) in crate::extension_gateway::extension_headers() {
        builder = builder.header(name, value);
    }
    let (client, request) = builder.build_split();
    let mut request = request?;
    let fingerprint = connection_id(relay_id, &request);
    let registry = REQUEST.with(|ctx| ctx.registry.clone());
    let lock = {
        let mut registry = registry.lock().unwrap_or_else(|p| p.into_inner());
        if registry.locks.len() >= 8 && !registry.locks.contains_key(&fingerprint) {
            return Err(Unsupported("一个本地 WS 会话最多连接 8 个上游，请新建连接").into());
        }
        registry
            .locks
            .entry(fingerprint.clone())
            .or_default()
            .clone()
    };
    let registration = lock.lock_owned().await;
    let reuse = {
        let registry = registry.lock().unwrap_or_else(|p| p.into_inner());
        let current = registry.connections.get(&fingerprint).cloned();
        REQUEST.with(|ctx| -> anyhow::Result<()> {
            if let Some(previous) = &ctx.previous {
                if current.as_deref() != Some(&previous.connection)
                    || body.get("model").and_then(Value::as_str) != Some(&previous.model)
                {
                    return Err(Unsupported(
                        "WS 续接的原连接已关闭或模型已变更；请使用完整上下文开始新响应",
                    )
                    .into());
                }
            }
            Ok(())
        })?;
        current
    };
    let mut response_headers = reqwest::header::HeaderMap::new();
    let socket = if reuse.is_some() {
        None
    } else {
        let key = tokio_tungstenite::tungstenite::handshake::client::generate_key();
        *request.method_mut() = reqwest::Method::GET;
        *request.version_mut() = reqwest::Version::HTTP_11;
        *request.body_mut() = None;
        for name in [
            "content-type",
            "content-length",
            "transfer-encoding",
            "accept",
            "cache-control",
            "sec-websocket-extensions",
            "sec-websocket-protocol",
        ] {
            request.headers_mut().remove(name);
        }
        for (name, value) in [
            ("connection", "Upgrade"),
            ("upgrade", "websocket"),
            ("sec-websocket-version", "13"),
            ("sec-websocket-key", key.as_str()),
        ] {
            request.headers_mut().insert(
                reqwest::header::HeaderName::from_bytes(name.as_bytes())?,
                value.parse()?,
            );
        }
        let response = tokio::time::timeout(timeout, client.execute(request))
            .await
            .context("WS 上游握手超时")??;
        if response.status() != reqwest::StatusCode::SWITCHING_PROTOCOLS {
            return Ok(PreparedUpstream::Rejected(response));
        }
        let header = |name| {
            response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
        };
        let accept = tokio_tungstenite::tungstenite::handshake::derive_accept_key(key.as_bytes());
        if !header("upgrade").eq_ignore_ascii_case("websocket")
            || !header("connection")
                .split(',')
                .any(|v| v.trim().eq_ignore_ascii_case("upgrade"))
            || header("sec-websocket-accept") != accept
            || response.headers().contains_key("sec-websocket-extensions")
            || response.headers().contains_key("sec-websocket-protocol")
        {
            bail!("WS 上游返回无效的升级握手");
        }
        response_headers = response.headers().clone();
        Some(
            WebSocketStream::from_raw_socket(
                response.upgrade().await?,
                Role::Client,
                Some(ws_config()),
            )
            .await,
        )
    };
    static NEXT_CONNECTION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let connection = reuse.unwrap_or_else(|| {
        format!(
            "{}:{}",
            fingerprint,
            NEXT_CONNECTION.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        )
    });
    Ok(PreparedUpstream::Websocket(Box::new(PreparedWebsocket {
        connection,
        fingerprint,
        relay: relay_id.into(),
        body,
        socket,
        registration: Some(registration),
        headers: response_headers,
        gateway: None,
        rotation: None,
        recorder: None,
        busy: None,
    })))
}

fn ws_config() -> tokio_tungstenite::tungstenite::protocol::WebSocketConfig {
    tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
        .max_message_size(Some(16 * 1024 * 1024))
        .max_frame_size(Some(16 * 1024 * 1024))
}

#[derive(Clone)]
struct ClientHeaders {
    user_agent: Option<String>,
    beta: Option<String>,
    session: Option<String>,
    thread: Option<String>,
    opencode: Option<String>,
    identity: Vec<(String, String)>,
}

impl ClientHeaders {
    fn read(headers: &str) -> Self {
        let get = |name| crate::launcher::header_value_from_headers(headers, name);
        Self {
            user_agent: get("user-agent"),
            beta: get("x-codex-beta-features"),
            session: get("session-id"),
            thread: get("thread-id"),
            opencode: get("x-opencode-session"),
            identity: crate::protocol_proxy::codex_client_identity_headers(headers),
        }
    }

    fn for_request(&self, body: &Value) -> Self {
        let mut headers = self.clone();
        // Codex 在长连接的 client_metadata 中携带逐回合身份，不能一直沿用握手那一回合。
        if let Some(metadata) = body.get("client_metadata").and_then(Value::as_object) {
            for (name, value) in metadata {
                if crate::protocol_proxy::CODEX_CLIENT_IDENTITY_HEADERS
                    .iter()
                    .any(|n| name.eq_ignore_ascii_case(n))
                    && let Some(value) = value
                        .as_str()
                        .filter(|v| v.len() <= 4096 && !v.contains(['\r', '\n']))
                {
                    headers
                        .identity
                        .retain(|(n, _)| !n.eq_ignore_ascii_case(name));
                    headers.identity.push((name.clone(), value.to_string()));
                }
            }
        }
        if let Some(metadata) = body.get("client_metadata") {
            let turn = metadata
                .get("x-codex-turn-metadata")
                .and_then(Value::as_str)
                .and_then(|text| serde_json::from_str::<Value>(text).ok());
            if let Some(thread) = metadata
                .get("thread_id")
                .and_then(Value::as_str)
                .or_else(|| {
                    turn.as_ref()
                        .and_then(|turn| turn.get("thread_id"))
                        .and_then(Value::as_str)
                })
                .filter(|id| id.len() <= 4096 && !id.contains(['\r', '\n']))
            {
                headers.thread = Some(thread.into());
                headers.session = Some(thread.into());
            }
            if let Some(session) = metadata
                .get("session_id")
                .and_then(Value::as_str)
                .filter(|id| id.len() <= 4096 && !id.contains(['\r', '\n']))
            {
                headers.session = Some(session.into());
            }
        }
        headers
    }
}

struct Opened {
    lane: String,
    response: ActiveResponse,
}

struct Reservation {
    slot: Option<crate::gateway::key_pool::KeySlot>,
    submitted: bool,
}
impl Drop for Reservation {
    fn drop(&mut self) {
        if !self.submitted
            && let Some(slot) = self.slot.take()
        {
            slot.cancel();
        }
    }
}

struct ActiveResponse {
    ready: Box<PreparedWebsocket>,
    log: crate::launcher::ProxyRequestLog,
    reservation: Reservation,
    transformer: crate::protocol_proxy::ResponsesSseTransformer,
    filter: Option<crate::extension_gateway::ResponseFilter>,
    started: std::time::Instant,
    response_id: Option<String>,
}

impl Drop for ActiveResponse {
    fn drop(&mut self) {
        if self.log.entry.stream_outcome.is_none() {
            self.log.record_stream(
                Some(if self.reservation.submitted {
                    "disconnected"
                } else {
                    "cancelled"
                }),
                None,
                None,
            );
        }
        if let Some(mut recorder) = self.ready.recorder.take() {
            let success =
                self.log.entry.success && self.log.entry.stream_outcome.as_deref() == Some("ok");
            if let Some((_, candidate)) = &self.ready.gateway {
                recorder.step(
                    candidate,
                    if success { "answered" } else { "failed" },
                    None,
                    Some(self.log.entry.status_code),
                );
            }
            recorder.finish(success);
        }
    }
}

fn error_status(event: &Value) -> u16 {
    if let Some(status) = event
        .get("status")
        .and_then(Value::as_u64)
        .filter(|status| (400..600).contains(status))
    {
        return status as u16;
    }
    let error = event.pointer("/response/error").unwrap_or(&event["error"]);
    let code = error["code"].as_str().unwrap_or("");
    let kind = error["type"].as_str().unwrap_or("");
    if code.contains("rate_limit") || code.contains("quota") {
        429
    } else if code == "invalid_api_key" || kind == "authentication_error" {
        401
    } else if kind == "permission_error" {
        403
    } else if kind == "invalid_request_error"
        || code.starts_with("invalid_")
        || code == "previous_response_not_found"
    {
        400
    } else {
        502
    }
}

fn record_failure(response: &ActiveResponse, status: u16, body: &[u8]) {
    if let Some((_, candidate)) = &response.ready.gateway {
        let class = crate::gateway::key_pool::classify_failure(
            status,
            &reqwest::header::HeaderMap::new(),
            body,
        );
        crate::gateway::pipeline::record_key_failure(&candidate.key, &class);
        crate::gateway::speed::record_failure(&candidate.relay.id, &candidate.upstream_model);
    }
    if let Some(settings) = &response.ready.rotation {
        crate::relay_rotation::record_relay_request_failure(settings);
    }
}

enum UpstreamEvent {
    Frame(String, Message),
    Closed(String),
}

struct Connection {
    sender: tokio::sync::mpsc::Sender<Message>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn start_connection(
    id: String,
    mut socket: UpstreamSocket,
    events: tokio::sync::mpsc::Sender<UpstreamEvent>,
) -> Connection {
    let (sender, mut commands) = tokio::sync::mpsc::channel::<Message>(16);
    let task = tokio::spawn(async move {
        loop {
            tokio::select! {
                command = commands.recv() => match command {
                    Some(message) => {
                        if !matches!(tokio::time::timeout(Duration::from_secs(30), socket.send(message)).await, Ok(Ok(()))) { break; }
                    }
                    None => break,
                },
                event = socket.next() => match event {
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => {},
                    Some(Ok(message)) => if events.send(UpstreamEvent::Frame(id.clone(), message)).await.is_err() { break; },
                }
            }
        }
        let _ = events.send(UpstreamEvent::Closed(id)).await;
    });
    Connection { sender, task }
}

fn error_event(lane: &str, status: u16, message: &str) -> Message {
    let mut value = json!({"type":"error","status":status,"error":{"type":"invalid_request_error","code":"local_responses_websocket_error","message":message}});
    if !lane.is_empty() {
        value["stream_id"] = json!(lane);
    }
    Message::Text(value.to_string().into())
}

fn parse_create(text: &str) -> anyhow::Result<(String, Value)> {
    let mut body: Value = serde_json::from_str(text)?;
    if body["type"] != "response.create" {
        bail!("此入口接受 response.create 事件");
    }
    let lane = match body.get("stream_id") {
        None => String::new(),
        Some(Value::String(id))
            if !id.is_empty()
                && id.len() <= 256
                && id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')) =>
        {
            id.clone()
        }
        _ => bail!("stream_id 格式无效"),
    };
    let object = body
        .as_object_mut()
        .context("response.create 必须是 JSON 对象")?;
    object.remove("type");
    object.remove("stream_id");
    object.remove("background");
    // 现有准备链按流式请求配置路由；发给 WS 上游前再去掉 HTTP 字段。
    object.insert("stream".into(), json!(true));
    Ok((lane, body))
}

async fn open_request(
    lane: String,
    body: Value,
    headers: ClientHeaders,
    path: String,
    opener: crate::launcher::ProxyOpener,
    registry: Arc<Mutex<Registry>>,
) -> Result<Opened, (String, u16, String)> {
    if registry
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .responses
        .len()
        >= 4096
    {
        return Err((
            lane,
            429,
            "本地 WS 响应亲和记录已达上限，请重新连接并发送完整上下文".into(),
        ));
    }
    let previous = body
        .get("previous_response_id")
        .and_then(Value::as_str)
        .and_then(|id| {
            registry
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .responses
                .get(id)
                .cloned()
        });
    let context = RequestContext { registry, previous };
    REQUEST.scope(context, crate::extension_gateway::with_codex_response_plan(async {
        let mut log = crate::launcher::ProxyRequestLog::begin("GET", &path, Some(&body), headers.thread.clone().or(headers.session.clone()));
        log.entry.stream = true;
        let session = crate::protocol_proxy::ProxySessionHeaders { session_id: headers.session.as_deref(), thread_id: headers.thread.as_deref(),
            opencode_session: headers.opencode.as_deref(), client_identity: &headers.identity };
        let body_text = body.to_string();
        let result = match &opener {
            crate::launcher::ProxyOpener::Legacy => crate::protocol_proxy::open_responses_proxy_request_for_path_with_session_headers_and_live_id(
                &body_text, headers.user_agent.as_deref(), &path, headers.beta.as_deref(), session, Some(&log.entry.id)).await.map(|r| (r, None)),
            crate::launcher::ProxyOpener::Gateway(agent) => {
                log.set_agent(agent);
                crate::gateway::server::open_responses_for_agent(agent, &path, &body_text, headers.user_agent.as_deref(), session, Some(&log.entry.id), None).await
            }
        };
        let (mut upstream, slot) = match result {
            Ok(result) => result,
            Err(error) => {
                let (status, message) = match error.downcast_ref::<Unsupported>() {
                    Some(error) => (400, error.to_string()),
                    None => (502, "WS 上游连接或请求准备失败".to_string()),
                };
                log.record_error(&status.to_string(), message.clone());
                return Err((lane, status, message));
            }
        };
        let reservation = Reservation { slot, submitted: false };
        log.record_upstream(&upstream);
        log.entry.success = false;
        let Some(ready) = upstream.websocket.take() else {
            let status = if upstream.status_code == 200 { 502 } else { upstream.status_code };
            let message = "选中的上游未接受标准 Responses WebSocket，请为此配置使用 HTTP".to_string();
            log.record_error(&status.to_string(), message.clone());
            return Err((lane, status, message));
        };
        let phase = crate::settings::SettingsStore::default().load().unwrap_or_default().codex_app_third_party_reasoning_collapse;
        let transformer = crate::protocol_proxy::ResponsesSseTransformer::for_request(Some(&body))
            .filling_message_phase(phase && !crate::extension_gateway::builtin_replaced("builtin:reasoning-collapse"));
        let mut response = ActiveResponse { ready, log, reservation, transformer, filter: None,
            started: std::time::Instant::now(), response_id: None };
        response.filter = crate::extension_gateway::ResponseFilter::start(&path, Some(&body)).await;
        Ok(Opened { lane, response })
    })).await
}

async fn send_message<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    ws: &mut WebSocketStream<S>,
    message: Message,
) -> anyhow::Result<()> {
    tokio::time::timeout(Duration::from_secs(30), ws.send(message))
        .await
        .context("WS 下游写入超时")??;
    Ok(())
}

async fn send_sse<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    ws: &mut WebSocketStream<S>,
    lane: &str,
    bytes: &[u8],
) -> anyhow::Result<()> {
    for block in String::from_utf8_lossy(bytes).split("\n\n") {
        let data = block
            .lines()
            .filter_map(|line| line.strip_prefix("data:").map(str::trim))
            .collect::<Vec<_>>()
            .join("\n");
        if !data.is_empty() && data != "[DONE]" {
            let mut value: Value =
                serde_json::from_str(&data).context("WS 响应中间件返回非 JSON 事件")?;
            if !lane.is_empty() {
                value["stream_id"] = json!(lane);
            }
            send_message(ws, Message::Text(value.to_string().into())).await?;
        }
    }
    Ok(())
}

/// 下游握手完成后仍通过既有准备链为每个 response.create 选模型、认证和 Key。
pub(crate) async fn handle(
    stream: &mut tokio::net::TcpStream,
    headers: &str,
    read_ahead: Vec<u8>,
    path: &str,
    mut opener: crate::launcher::ProxyOpener,
) -> anyhow::Result<()> {
    let request_line = headers.lines().next().unwrap_or_default();
    let mut request = http::Request::builder()
        .method("GET")
        .uri(request_line.split_whitespace().nth(1).unwrap_or("/"));
    for line in headers.lines().skip(1) {
        if let Some((name, value)) = line.split_once(':') {
            request = request.header(name.trim(), value.trim());
        }
    }
    let response = request.body(()).ok().and_then(|r| {
        use base64::Engine;
        let valid_key = r
            .headers()
            .get("sec-websocket-key")
            .and_then(|key| {
                base64::engine::general_purpose::STANDARD
                    .decode(key.as_bytes())
                    .ok()
            })
            .is_some_and(|key| key.len() == 16);
        valid_key
            .then(|| tokio_tungstenite::tungstenite::handshake::server::create_response(&r).ok())
            .flatten()
    });
    let Some(response) = response else {
        crate::launcher::write_http_response(
            stream,
            "400 Bad Request",
            "application/json",
            br#"{"error":"invalid WebSocket upgrade"}"#,
        )
        .await?;
        return Ok(());
    };
    use tokio::io::AsyncWriteExt;
    let mut wire = "HTTP/1.1 101 Switching Protocols\r\n".to_string();
    for (name, value) in response.headers() {
        wire.push_str(&format!("{}: {}\r\n", name, value.to_str()?));
    }
    wire.push_str("\r\n");
    stream.write_all(wire.as_bytes()).await?;
    let mut ws =
        WebSocketStream::from_partially_read(stream, read_ahead, Role::Server, Some(ws_config()))
            .await;
    // 官方身份的本地代理原本再跳一次网关；WS 直接调用同一网关准备入口，避免双重 hooks 与日志。
    if matches!(opener, crate::launcher::ProxyOpener::Legacy)
        && crate::gateway::apply::codex_hops_through_gateway()
    {
        opener = crate::launcher::ProxyOpener::Gateway(crate::tools::ToolId::Codex);
    }
    let headers = ClientHeaders::read(headers);
    let registry = Arc::new(Mutex::new(Registry::default()));
    let (events_tx, mut events) = tokio::sync::mpsc::channel(64);
    let (opened_tx, mut opened_rx) = tokio::sync::mpsc::channel(16);
    let mut connections: HashMap<String, Connection> = HashMap::new();
    let mut active: HashMap<String, ActiveResponse> = HashMap::new();
    let mut queue: std::collections::VecDeque<(String, Value, usize)> =
        std::collections::VecDeque::new();
    let mut queued_bytes = 0usize;
    let mut lanes = HashSet::new();
    struct PreparationTasks(HashMap<String, tokio::task::JoinHandle<()>>);
    impl Drop for PreparationTasks {
        fn drop(&mut self) {
            for task in self.0.values() {
                task.abort();
            }
        }
    }
    let mut preparing = PreparationTasks(HashMap::new());
    loop {
        while active.len() + preparing.0.len() < 16 {
            let Some(index) = queue.iter().position(|(lane, _, _)| {
                !active.contains_key(lane) && !preparing.0.contains_key(lane)
            }) else {
                break;
            };
            let (lane, body, size) = queue.remove(index).unwrap();
            queued_bytes -= size;
            let headers = headers.for_request(&body);
            let (opener, registry, path, sender) = (
                opener.clone(),
                registry.clone(),
                path.to_string(),
                opened_tx.clone(),
            );
            let task_lane = lane.clone();
            preparing.0.insert(
                lane,
                tokio::spawn(async move {
                    let result =
                        open_request(task_lane, body, headers, path, opener, registry).await;
                    let _ = sender.send(result).await;
                }),
            );
        }
        tokio::select! {
            message = ws.next() => match message {
                Some(Ok(Message::Text(text))) => match parse_create(&text) {
                    Ok((lane, body)) => {
                        if (!lane.is_empty() && !lanes.contains(&lane) && lanes.len() >= 32)
                            || queue.len() >= 64 || queued_bytes.saturating_add(text.len()) > 32 * 1024 * 1024
                        {
                            send_message(&mut ws, error_event(&lane, 429, "本地 WS 流或待处理请求数量已达上限")).await?;
                        } else {
                            if !lane.is_empty() { lanes.insert(lane.clone()); }
                            queued_bytes += text.len();
                            queue.push_back((lane, body, text.len()));
                        }
                    }
                    Err(_) => send_message(&mut ws, error_event("", 400, "此入口支持 response.create；事件或 stream_id 无效")).await?,
                },
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {},
                Some(Ok(Message::Close(_))) | None => return Ok(()),
                Some(Err(error)) => return Err(anyhow::Error::from(error)),
                _ => send_message(&mut ws, error_event("", 400, "Responses WS 请求必须是 JSON 文本事件")).await?,
            },
            result = opened_rx.recv(), if !preparing.0.is_empty() => {
                match result.context("WS 请求准备任务退出")? {
                    Err((lane, status, message)) => {
                        preparing.0.remove(&lane);
                        send_message(&mut ws, error_event(&lane, status, &message)).await?;
                    }
                    Ok(opened) => {
                        let Opened { lane, mut response } = opened;
                        preparing.0.remove(&lane);
                        let id = response.ready.connection.clone();
                        if let Some(socket) = response.ready.socket.take() {
                            connections.insert(id.clone(), start_connection(id.clone(), socket, events_tx.clone()));
                            registry.lock().unwrap_or_else(|p| p.into_inner()).connections.insert(response.ready.fingerprint.clone(), id.clone());
                        }
                        response.ready.registration.take();
                        let mut body = response.ready.body.clone();
                        let object = body.as_object_mut().context("WS 上游请求不是 JSON 对象")?;
                        object.remove("stream"); object.remove("background");
                        object.insert("type".into(), json!("response.create"));
                        if !lane.is_empty() { object.insert("stream_id".into(), json!(lane)); }
                        let Some(connection) = connections.get(&id) else {
                            response.log.record_error("502", "WS 上游连接已关闭".into());
                            send_message(&mut ws, error_event(&lane, 502, "WS 上游连接已关闭")).await?;
                            continue;
                        };
                        if connection.sender.send(Message::Text(body.to_string().into())).await.is_err() {
                            response.log.record_error("502", "WS 上游连接已关闭".into());
                            send_message(&mut ws, error_event(&lane, 502, "WS 上游连接已关闭")).await?;
                            continue;
                        }
                        // 入上游写队列后可能已生成；取消不再退 RPM，更不能自动重发。
                        response.reservation.submitted = true;
                        active.insert(lane, response);
                    }
                }
            },
            event = events.recv() => match event.context("WS 上游事件通道退出")? {
                UpstreamEvent::Closed(id) => {
                    connections.remove(&id);
                    registry.lock().unwrap_or_else(|p| p.into_inner()).connections.retain(|_, current| current != &id);
                    let failed: Vec<_> = active.iter().filter(|(_, r)| r.ready.connection == id).map(|(lane, _)| lane.clone()).collect();
                    for lane in failed {
                        if let Some(mut response) = active.remove(&lane) {
                            response.log.entry.status_code = 502;
                            record_failure(&response, 502, b"");
                        }
                        send_message(&mut ws, error_event(&lane, 502, "WS 上游连接已关闭；生成结果可能不完整，未自动重发")).await?;
                    }
                }
                UpstreamEvent::Frame(id, Message::Text(text)) => {
                    let event: Value = serde_json::from_str(&text).context("WS 上游返回无效 JSON")?;
                    let lane = event.get("stream_id").and_then(Value::as_str).unwrap_or("").to_string();
                    let kind = event["type"].as_str().unwrap_or("");
                    // 不带 lane 的连接级错误影响该连接上的全部活动响应。
                    if kind == "error" && event.get("stream_id").is_none()
                        && !active.get("").is_some_and(|response| response.ready.connection == id)
                    {
                        let failed: Vec<_> = active.iter().filter(|(_, r)| r.ready.connection == id).map(|(lane, _)| lane.clone()).collect();
                        for lane in failed {
                            if let Some(mut response) = active.remove(&lane) {
                                let status = error_status(&event);
                                response.log.entry.status_code = status;
                                response.log.record_stream(Some("failed"), None, None);
                                record_failure(&response, status, text.as_bytes());
                            }
                            send_sse(&mut ws, &lane, format!("data: {text}\n\n").as_bytes()).await?;
                        }
                        continue;
                    }
                    let Some(response) = active.get_mut(&lane).filter(|r| r.ready.connection == id) else {
                        // 服务端控制事件可发生在响应终态之后，保留其原始协议形状。
                        send_message(&mut ws, Message::Text(text)).await?;
                        continue;
                    };
                    if response.response_id.is_none() && let Some(response_id) = event.pointer("/response/id").and_then(Value::as_str) {
                        response.response_id = Some(response_id.into());
                        registry.lock().unwrap_or_else(|p| p.into_inner()).responses.insert(response_id.into(), Affinity {
                            connection: id.clone(), relay: response.ready.relay.clone(),
                            model: response.ready.body["model"].as_str().unwrap_or("").into(),
                            key: response.ready.gateway.as_ref().map(|(_, c)| c.key.clone()),
                        });
                    }
                    if kind.ends_with(".delta") { response.log.mark_first_token_at(crate::launcher::current_time_ms()); }
                    let bytes = response.transformer.push_bytes(format!("data: {text}\n\n").as_bytes());
                    let bytes = if let Some(filter) = response.filter.as_mut() { filter.push(&bytes).await } else { bytes };
                    send_sse(&mut ws, &lane, &bytes).await?;
                    if matches!(kind, "response.completed" | "response.failed" | "response.incomplete" | "error") {
                        if let Some(filter) = response.filter.as_mut() { send_sse(&mut ws, &lane, &filter.finish().await).await?; }
                        let success = kind == "response.completed" && event.pointer("/response/status").and_then(Value::as_str).is_none_or(|s| s == "completed");
                        let error = event.pointer("/response/error").unwrap_or(&event["error"]);
                        let status = error_status(&event);
                        response.log.entry.status_code = if success || kind == "response.incomplete" { 200 } else { status };
                        response.log.record_usage_json(text.as_bytes());
                        response.log.record_stream(if success { None } else if kind == "response.incomplete" { Some("incomplete") } else { Some("failed") }, None, None);
                        if success { response.log.record_final_response(response.transformer.final_response_usage()); }
                        if success {
                            if let Some((plan, candidate)) = &response.ready.gateway {
                                crate::gateway::key_pool::record_success(&candidate.key, plan.conversation.as_deref());
                                crate::gateway::route::remember_answer(plan, candidate);
                                crate::gateway::speed::record(&candidate.relay.id, &candidate.upstream_model, response.started.elapsed());
                            }
                            if let Some(settings) = &response.ready.rotation {
                                crate::relay_rotation::record_relay_request_event(settings, crate::relay_rotation::RotationEvent::Success);
                            }
                        } else if kind != "response.incomplete" {
                            record_failure(response, status, json!({"error":error}).to_string().as_bytes());
                        }
                        active.remove(&lane);
                    }
                }
                UpstreamEvent::Frame(_, _) => bail!("WS 上游返回非 JSON 文本事件"),
            },
        }
    }
}

#[cfg(test)]
mod tests;
