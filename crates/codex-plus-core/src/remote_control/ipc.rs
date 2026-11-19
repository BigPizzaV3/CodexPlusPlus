//! Codex 桌面 IPC 路由的客户端。
//!
//! 传输：macOS / Linux 为 `<CODEX_HOME>/ipc/ipc.sock`（路径过长时桌面会改用
//! `/tmp/codex-ipc-*/<sha256(ipc 目录真实路径)>/ipc.sock`），Windows 为命名管道 `\\.\pipe\codex-ipc`。
//! 帧：`[u32 LE 长度][UTF-8 JSON]`。消息类型：request / response / broadcast /
//! client-discovery-request / client-discovery-response。
//!
//! 我们注册成普通客户端：对路由器的 discovery 一律回 `canHandle: false`，不接别人的请求，
//! 只发 follower 请求和跟随广播。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, bail};
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{Mutex, broadcast, oneshot};

use super::versions::IpcVersions;

/// 桌面那边单帧上限 256 MiB。
const MAX_FRAME_BYTES: usize = 256 * 1024 * 1024;
const INITIALIZING_CLIENT: &str = "initializing-client";

type Writer = Box<dyn AsyncWrite + Send + Unpin>;
type Reader = Box<dyn AsyncRead + Send + Unpin>;

/// 一条 IPC 回复：`resultType` 为 success 时有 `result` 与 `handledByClientId`。
#[derive(Debug, Clone)]
pub struct IpcResponse {
    pub raw: Value,
}

impl IpcResponse {
    pub fn ok(&self) -> bool {
        self.raw.get("resultType").and_then(Value::as_str) == Some("success")
    }

    pub fn handled_by(&self) -> Option<&str> {
        self.raw.get("handledByClientId").and_then(Value::as_str)
    }

    pub fn result(&self) -> &Value {
        self.raw.get("result").unwrap_or(&Value::Null)
    }

    pub fn error(&self) -> String {
        match self.raw.get("error") {
            Some(Value::String(text)) => text.clone(),
            Some(other) => other.to_string(),
            None => "unknown-error".to_string(),
        }
    }

    /// 失败时转成错误。
    pub fn into_result(self) -> anyhow::Result<Value> {
        if self.ok() {
            Ok(self.result().clone())
        } else {
            bail!("{}", self.error())
        }
    }
}

pub struct IpcConnection {
    writer: Mutex<Writer>,
    pending: std::sync::Mutex<HashMap<String, oneshot::Sender<Value>>>,
    events: broadcast::Sender<Value>,
    client_id: std::sync::Mutex<String>,
    versions: IpcVersions,
    closed: AtomicBool,
}

impl IpcConnection {
    /// 连上并完成 `initialize`。
    pub async fn connect(endpoint: &IpcEndpoint, client_type: &str, versions: IpcVersions) -> anyhow::Result<Arc<Self>> {
        let (reader, writer) = open_transport(endpoint).await?;
        let (events, _) = broadcast::channel(1024);
        let connection = Arc::new(Self {
            writer: Mutex::new(writer),
            pending: std::sync::Mutex::new(HashMap::new()),
            events,
            client_id: std::sync::Mutex::new(INITIALIZING_CLIENT.to_string()),
            versions,
            closed: AtomicBool::new(false),
        });
        tokio::spawn(read_loop(Arc::downgrade(&connection), reader));
        let init = connection
            .request("initialize", json!({ "clientType": client_type }), None, Duration::from_secs(5))
            .await?;
        let client_id = init
            .result()
            .get("clientId")
            .and_then(Value::as_str)
            .with_context(|| format!("IPC initialize 失败：{}", init.raw))?
            .to_string();
        *connection.client_id.lock().unwrap_or_else(|e| e.into_inner()) = client_id;
        Ok(connection)
    }

    pub fn client_id(&self) -> String {
        self.client_id.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// 订阅广播。连接断开时接收端会收到 `Closed`。
    pub fn subscribe(&self) -> broadcast::Receiver<Value> {
        self.events.subscribe()
    }

    fn version(&self, method: &str) -> u64 {
        self.versions.get(method).copied().unwrap_or(0)
    }

    /// 发请求并等回复。`target` 缺省时由路由器向所有客户端做 discovery。
    /// 路由器在某个客户端不回 discovery 时会等满 10 秒，所以本地超时要按调用场景给。
    pub async fn request(
        &self,
        method: &str,
        params: Value,
        target: Option<&str>,
        timeout: Duration,
    ) -> anyhow::Result<IpcResponse> {
        if self.is_closed() {
            bail!("Codex IPC 已断开");
        }
        let request_id = uuid::Uuid::new_v4().to_string();
        let (sender, receiver) = oneshot::channel();
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(request_id.clone(), sender);
        let mut message = json!({
            "type": "request",
            "requestId": request_id,
            "sourceClientId": self.client_id(),
            "version": self.version(method),
            "method": method,
            "params": params,
            "timeoutMs": timeout.as_millis() as u64,
        });
        if let Some(target) = target {
            message["targetClientId"] = json!(target);
        }
        if let Err(error) = self.write(&message).await {
            self.pending.lock().unwrap_or_else(|e| e.into_inner()).remove(&request_id);
            return Err(error);
        }
        // 留一点余量，让路由器自己的超时回复先到。
        match tokio::time::timeout(timeout + Duration::from_millis(500), receiver).await {
            Ok(Ok(raw)) => Ok(IpcResponse { raw }),
            Ok(Err(_)) => bail!("Codex IPC 已断开"),
            Err(_) => {
                self.pending.lock().unwrap_or_else(|e| e.into_inner()).remove(&request_id);
                Ok(IpcResponse { raw: json!({ "resultType": "error", "error": "local-timeout" }) })
            }
        }
    }

    pub async fn broadcast(&self, method: &str, params: Value) -> anyhow::Result<()> {
        let message = json!({
            "type": "broadcast",
            "method": method,
            "sourceClientId": self.client_id(),
            "version": self.version(method),
            "params": params,
        });
        self.write(&message).await
    }

    async fn write(&self, message: &Value) -> anyhow::Result<()> {
        let frame = encode_frame(message)?;
        let mut writer = self.writer.lock().await;
        writer.write_all(&frame).await.context("写 Codex IPC 失败")?;
        writer.flush().await?;
        Ok(())
    }

    fn mark_closed(&self) {
        self.closed.store(true, Ordering::SeqCst);
        // 丢掉所有等待中的回复发送端，request 会立刻得到「已断开」。
        self.pending.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
}

pub fn encode_frame(message: &Value) -> anyhow::Result<Vec<u8>> {
    let body = serde_json::to_vec(message)?;
    if body.len() > MAX_FRAME_BYTES {
        bail!("IPC 消息过大：{} 字节", body.len());
    }
    let mut frame = Vec::with_capacity(4 + body.len());
    frame.extend_from_slice(&(body.len() as u32).to_le_bytes());
    frame.extend_from_slice(&body);
    Ok(frame)
}

async fn read_frame(reader: &mut Reader) -> anyhow::Result<Value> {
    let mut head = [0u8; 4];
    reader.read_exact(&mut head).await?;
    let size = u32::from_le_bytes(head) as usize;
    if size > MAX_FRAME_BYTES {
        bail!("IPC 帧过大：{size}");
    }
    let mut body = vec![0u8; size];
    reader.read_exact(&mut body).await?;
    Ok(serde_json::from_slice(&body)?)
}

async fn read_loop(connection: std::sync::Weak<IpcConnection>, mut reader: Reader) {
    loop {
        let message = match read_frame(&mut reader).await {
            Ok(message) => message,
            Err(_) => break,
        };
        let Some(connection) = connection.upgrade() else { return };
        match message.get("type").and_then(Value::as_str) {
            Some("response") => {
                let id = message.get("requestId").and_then(Value::as_str).unwrap_or_default();
                let sender = connection.pending.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
                if let Some(sender) = sender {
                    let _ = sender.send(message);
                }
            }
            Some("client-discovery-request") => {
                let reply = json!({
                    "type": "client-discovery-response",
                    "requestId": message.get("requestId"),
                    "response": { "canHandle": false },
                });
                let _ = connection.write(&reply).await;
            }
            Some("request") => {
                let reply = json!({
                    "type": "response",
                    "requestId": message.get("requestId"),
                    "resultType": "error",
                    "error": "unsupported",
                });
                let _ = connection.write(&reply).await;
            }
            Some("broadcast") => {
                let _ = connection.events.send(message);
            }
            _ => {}
        }
    }
    if let Some(connection) = connection.upgrade() {
        connection.mark_closed();
    }
}

/// IPC 端点：Unix socket 路径或 Windows 命名管道名。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpcEndpoint {
    Socket(PathBuf),
    Pipe(String),
}

impl std::fmt::Display for IpcEndpoint {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Socket(path) => write!(formatter, "{}", path.display()),
            Self::Pipe(name) => write!(formatter, "{name}"),
        }
    }
}

/// 与桌面主进程相同的端点规则。
pub fn resolve_endpoint(codex_home: &Path) -> IpcEndpoint {
    if cfg!(windows) {
        return IpcEndpoint::Pipe(r"\\.\pipe\codex-ipc".to_string());
    }
    let ipc_dir = codex_home.join("ipc");
    let primary = ipc_dir.join("ipc.sock");
    if primary.exists() {
        return IpcEndpoint::Socket(primary);
    }
    // 路径太长时桌面把 socket 放到 /tmp/codex-ipc-*/<sha256(ipc 目录真实路径)>/ipc.sock。
    if let Ok(real) = std::fs::canonicalize(&ipc_dir) {
        use sha2::Digest;
        let hash = sha2::Sha256::digest(real.to_string_lossy().as_bytes());
        let hash = hash.iter().map(|byte| format!("{byte:02x}")).collect::<String>();
        if let Ok(entries) = std::fs::read_dir("/tmp") {
            for entry in entries.flatten() {
                if entry.file_name().to_string_lossy().starts_with("codex-ipc-") {
                    let candidate = entry.path().join(&hash).join("ipc.sock");
                    if candidate.exists() {
                        return IpcEndpoint::Socket(candidate);
                    }
                }
            }
        }
    }
    IpcEndpoint::Socket(primary)
}

#[cfg(unix)]
async fn open_transport(endpoint: &IpcEndpoint) -> anyhow::Result<(Reader, Writer)> {
    let IpcEndpoint::Socket(path) = endpoint else { bail!("此平台只支持 Unix socket") };
    let stream = tokio::net::UnixStream::connect(path)
        .await
        .with_context(|| format!("连不上 Codex IPC（{}），Codex 桌面在运行吗？", path.display()))?;
    let (reader, writer) = stream.into_split();
    Ok((Box::new(reader), Box::new(writer)))
}

#[cfg(windows)]
async fn open_transport(endpoint: &IpcEndpoint) -> anyhow::Result<(Reader, Writer)> {
    let IpcEndpoint::Pipe(name) = endpoint else { bail!("此平台只支持命名管道") };
    let pipe = tokio::net::windows::named_pipe::ClientOptions::new()
        .open(name)
        .with_context(|| format!("连不上 Codex IPC（{name}），Codex 桌面在运行吗？"))?;
    let (reader, writer) = tokio::io::split(pipe);
    Ok((Box::new(reader), Box::new(writer)))
}
