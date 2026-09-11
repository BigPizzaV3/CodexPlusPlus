mod command_source;
mod live_source;
pub mod official_tasks;
mod store;

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, Utc};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};

use official_tasks::TaskReader;
use store::Store;

const SERVICE_CONFIG: &str = include_str!("../../../environment/shared-remote-service.json");
const MAX_SYNCED_TASKS: usize = 20;
type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelConfigSummary {
    id: String,
    model: String,
    provider: String,
}

fn model_configs(home: &std::path::Path) -> Vec<ModelConfigSummary> {
    crate::config::load_remote_models(home)
        .into_iter()
        .map(|config| ModelConfigSummary {
            id: config.id,
            model: config.model,
            provider: config.provider,
        })
        .collect()
}

#[derive(Default)]
struct ReplyStreamState {
    stream_id: String,
    seq: u64,
    text: String,
    ended: bool,
}

impl ReplyStreamState {
    fn updates(&mut self, text: &str, outcome: &str, reset: bool) -> Vec<Value> {
        let mut frames = Vec::new();
        // 重连和正文校正使用完整 reset，正常增长只发后缀，不重新传输全部历史。
        if reset
            || self.stream_id.is_empty()
            || (self.ended && self.text != text)
            || !text.starts_with(&self.text)
        {
            self.stream_id = id();
            self.seq = 1;
            self.ended = false;
            frames.push(json!({"messageType": "reply-stream/reset", "text": text}));
        } else if text.len() > self.text.len() {
            self.seq += 1;
            frames.push(
                json!({"messageType": "reply-stream/append", "text": &text[self.text.len()..]}),
            );
        }
        if let Some(frame) = frames.last_mut() {
            frame["streamSeq"] = self.seq.into();
        }
        self.text = text.to_owned();
        if !self.ended && matches!(outcome, "completed" | "failed" | "interrupted" | "stopped") {
            self.seq += 1;
            self.ended = true;
            frames.push(json!({"messageType": "reply-stream/end", "streamSeq": self.seq, "outcome": outcome}));
        }
        for frame in &mut frames {
            frame["streamId"] = self.stream_id.clone().into();
        }
        frames
    }
}

pub(super) fn id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}
pub(super) fn now() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
pub(super) fn opaque_id(value: &str) -> bool {
    (16..=128).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ServiceConfig {
    enabled: bool,
    protocol_environment: String,
    api_base_url: String,
    wss_url: String,
}

fn service_config() -> anyhow::Result<ServiceConfig> {
    let config: ServiceConfig = serde_json::from_str(SERVICE_CONFIG)?;
    let api = reqwest::Url::parse(&config.api_base_url)?;
    let gateway = reqwest::Url::parse(&config.wss_url)?;
    if !config.enabled
        || config.protocol_environment != "dev"
        || api.scheme() != "https"
        || gateway.scheme() != "wss"
        || api.host_str() != gateway.host_str()
        || !api.username().is_empty()
        || api.password().is_some()
        || !gateway.username().is_empty()
        || gateway.password().is_some()
    {
        bail!("远程服务配置不可用");
    }
    Ok(config)
}

fn desktop_display_name() -> String {
    let computer = std::env::var("COMPUTERNAME").unwrap_or_default();
    let computer: String = computer
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .collect();
    if computer.is_empty() {
        "轩++桌面".into()
    } else {
        official_tasks::clip(&format!("轩++桌面（{computer}）"), 50)
    }
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MobileStatus {
    pub enabled: bool,
    pub connected: bool,
    pub bound: bool,
    pub message: String,
    pub qr_image: Option<String>,
    pub qr_expires_at: Option<String>,
    pub pending: Option<PendingBinding>,
    pub auto_sync: bool,
    pub selected: BTreeSet<String>,
    pub last_synced_at: Option<String>,
    pub sync_error: Option<String>,
    pub sync_issues: Vec<SyncIssue>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncIssue {
    pub task_id: String,
    pub task_name: String,
    pub workspace_name: String,
    pub reason: String,
}

impl SyncIssue {
    fn from_task_error(task: &official_tasks::OfficialTask, error: &anyhow::Error) -> Self {
        Self {
            task_id: task.id.clone(),
            task_name: task.name.clone(),
            workspace_name: task.workspace_name.clone(),
            reason: task_sync_issue_reason(error).into(),
        }
    }
}

fn task_sync_issue_reason(error: &anyhow::Error) -> &'static str {
    let detail = format!("{error:#}");
    if detail.contains("任务历史正在分批读取") {
        "任务历史较长，正在分批读取"
    } else if detail.contains("任务记录单条内容过大") {
        "单条记录过大，已暂停同步"
    } else if detail.contains("任务记录格式暂不支持") {
        "任务记录格式不兼容"
    } else if detail.contains("任务记录尚未就绪") {
        "任务记录正在写入，等待完整写入"
    } else if detail.contains("任务记录暂不可用") {
        "任务记录文件不可访问"
    } else if detail.contains("任务记录不在官方会话目录内") {
        "任务记录不是可同步的官方会话文件"
    } else if detail.contains("任务身份不匹配或属于子任务") {
        "任务身份不匹配或属于子任务"
    } else {
        "读取任务记录失败"
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingBinding {
    pub request_id: String,
    pub phone_name: String,
    pub safety_phrase: String,
    pub expires_at: String,
}

enum Action {
    Pair,
    Enable(bool),
    Confirm(String, bool),
    AutoSync(bool),
    Select(BTreeSet<String>),
}
struct Request {
    action: Action,
    reply: oneshot::Sender<Result<(), String>>,
}

pub struct MobileRemote {
    path: PathBuf,
    home: PathBuf,
    status: Arc<Mutex<MobileStatus>>,
    sender: tokio::sync::Mutex<Option<mpsc::Sender<Request>>>,
}

impl Default for MobileRemote {
    fn default() -> Self {
        Self::new(
            crate::paths::default_remote_state_dir().join("mobile-remote.sqlite"),
            crate::codex_home::default_codex_home_dir(),
        )
    }
}

impl MobileRemote {
    pub fn new(path: PathBuf, home: PathBuf) -> Self {
        Self {
            path,
            home,
            status: Arc::new(Mutex::new(MobileStatus {
                message: "手机连接未启用".into(),
                ..Default::default()
            })),
            sender: tokio::sync::Mutex::new(None),
        }
    }

    pub fn status(&self) -> MobileStatus {
        self.status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn database_path(&self) -> &std::path::Path {
        &self.path
    }

    pub fn home_path(&self) -> &std::path::Path {
        &self.home
    }

    pub fn initialize_state(&self) -> anyhow::Result<()> {
        Store::open(&self.path).map(drop)
    }

    pub async fn send_input(
        &self,
        thread_id: String,
        client_request_id: String,
        text: String,
    ) -> Result<Value, String> {
        validate_direct_command(&thread_id, &client_request_id, &text)?;
        command_source::execute(command_source::request(
            "send_input",
            &thread_id,
            "",
            &client_request_id,
            "",
            "",
            "",
            &text,
            "",
        ))
        .await
        .map_err(|error| format!("{error:#}"))
    }

    pub async fn stop(&self, thread_id: String, turn_id: String) -> Result<Value, String> {
        if !opaque_id(&thread_id) || !opaque_id(&turn_id) {
            return Err("任务或回合标识无效".into());
        }
        command_source::execute(command_source::request(
            "stop_task",
            &thread_id,
            &turn_id,
            "",
            "",
            "",
            "",
            "",
            "",
        ))
        .await
        .map_err(|error| format!("{error:#}"))
    }

    pub async fn restore(&self) {
        if self.path.is_file() && self.ensure_started().await.is_err() {
            self.status
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .message = "无法恢复手机连接，请重新启用".into();
        }
    }

    async fn ensure_started(&self) -> anyhow::Result<mpsc::Sender<Request>> {
        let mut sender = self.sender.lock().await;
        if let Some(tx) = sender.as_ref().filter(|tx| !tx.is_closed()) {
            return Ok(tx.clone());
        }
        let config = service_config()?;
        let (tx, rx) = mpsc::channel(16);
        let path = self.path.clone();
        let home = self.home.clone();
        let status = Arc::clone(&self.status);
        let (ready, initialized) = oneshot::channel();
        // 身份库、官方索引和会话解析均为同步 I/O，不能占用主功能的异步执行线程。
        std::thread::Builder::new()
            .name("mobile-remote".into())
            .spawn(move || {
                let setup = (|| -> anyhow::Result<_> {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()?;
                    let worker = Worker {
                        store: Store::open(&path)?,
                        config,
                        home,
                        status,
                        pairing_id: None,
                        confirmation: None,
                        confirmation_id: None,
                    };
                    worker.update(|s| {
                        s.enabled = worker.store.state.enabled;
                        s.auto_sync = worker.store.state.auto_sync;
                        s.selected = worker.store.state.selected.clone();
                    });
                    Ok((runtime, worker))
                })();
                match setup {
                    Ok((runtime, mut worker)) => {
                        if ready.send(Ok(())).is_ok() {
                            runtime.block_on(worker.run(rx));
                        }
                    }
                    Err(error) => {
                        let _ = ready.send(Err(error));
                    }
                }
            })?;
        initialized.await??;
        *sender = Some(tx.clone());
        Ok(tx)
    }

    async fn action(&self, action: Action) -> Result<MobileStatus, String> {
        let tx = self
            .ensure_started()
            .await
            .map_err(|_| "无法读取或保护本机设备身份".to_owned())?;
        let (reply, result) = oneshot::channel();
        tx.send(Request { action, reply })
            .await
            .map_err(|_| "手机连接服务已停止")?;
        result.await.map_err(|_| "手机连接服务已停止")??;
        Ok(self.status())
    }

    pub async fn pair(&self) -> Result<MobileStatus, String> {
        self.action(Action::Pair).await
    }
    pub async fn enable(&self, enabled: bool) -> Result<MobileStatus, String> {
        self.action(Action::Enable(enabled)).await
    }
    pub async fn confirm(
        &self,
        request_id: String,
        confirmed: bool,
    ) -> Result<MobileStatus, String> {
        self.action(Action::Confirm(request_id, confirmed)).await
    }
    pub async fn auto_sync(&self, enabled: bool) -> Result<MobileStatus, String> {
        self.action(Action::AutoSync(enabled)).await
    }
    pub async fn select(&self, selected: BTreeSet<String>) -> Result<MobileStatus, String> {
        if selected.len() > MAX_SYNCED_TASKS || selected.iter().any(|id| !opaque_id(id)) {
            return Err(format!("最多可同步 {MAX_SYNCED_TASKS} 个有效任务"));
        }
        self.action(Action::Select(selected)).await
    }
}

fn validate_direct_command(
    thread_id: &str,
    client_request_id: &str,
    text: &str,
) -> Result<(), String> {
    if !opaque_id(thread_id) || !opaque_id(client_request_id) {
        return Err("任务或请求标识无效".into());
    }
    if text.trim().is_empty() || text.len() > 64 * 1024 || text.contains('\0') {
        return Err("手机输入为空或超出 64 KiB 限制".into());
    }
    Ok(())
}

struct Worker {
    store: Store,
    config: ServiceConfig,
    home: PathBuf,
    status: Arc<Mutex<MobileStatus>>,
    pairing_id: Option<String>,
    confirmation: Option<Value>,
    confirmation_id: Option<String>,
}

impl Worker {
    fn update(&self, update: impl FnOnce(&mut MobileStatus)) {
        update(&mut self.status.lock().unwrap_or_else(|e| e.into_inner()));
    }

    async fn run(&mut self, mut rx: mpsc::Receiver<Request>) {
        let mut retry = 1_u64;
        let mut readers = HashMap::new();
        loop {
            if self.store.state.enabled && self.store.state.enrolled {
                self.update(|s| {
                    s.message = "正在连接现有云服务".into();
                    s.connected = false;
                    s.bound = false;
                });
                let _result = self.session(&mut rx, &mut readers).await;
                self.confirmation = None;
                self.update(|s| {
                    s.connected = false;
                    s.bound = false;
                    s.pending = None;
                    if self.store.state.enabled {
                        s.message = "连接暂时中断，正在重试".into();
                    }
                });
                if rx.is_closed() {
                    break;
                }
                if !self.store.state.enabled {
                    continue;
                }
                tokio::select! {
                    request = rx.recv() => {
                        let Some(request) = request else { break; };
                        self.handle(request, None).await;
                    }
                    _ = tokio::time::sleep(Duration::from_secs(retry)) => {}
                }
                retry = (retry * 2).min(30);
            } else {
                let Some(request) = rx.recv().await else {
                    break;
                };
                self.handle(request, None).await;
                retry = 1;
            }
        }
    }

    async fn handle(&mut self, request: Request, socket: Option<&mut Socket>) {
        let result = self.apply_action(request.action, socket).await;
        let _ = request
            .reply
            .send(result.map_err(|error| error.to_string()));
    }

    async fn apply_action(
        &mut self,
        action: Action,
        socket: Option<&mut Socket>,
    ) -> anyhow::Result<()> {
        match action {
            Action::Pair => {
                self.register_pairing().await?;
            }
            Action::Enable(enabled) => {
                if enabled && !self.store.state.enrolled {
                    bail!("请先生成绑定二维码");
                }
                self.store.state.enabled = enabled;
                self.store.save().context("无法保存连接状态")?;
                self.update(|s| {
                    s.enabled = enabled;
                    if !enabled {
                        s.connected = false;
                        s.bound = false;
                        s.pending = None;
                        s.qr_image = None;
                        s.qr_expires_at = None;
                        s.message = "手机连接已暂停".into();
                    }
                });
                if !enabled {
                    self.confirmation = None;
                    self.pairing_id = None;
                    self.confirmation_id = None;
                }
            }
            Action::Confirm(request_id, confirmed) => {
                let socket = socket.context("连接已断开，请等待重连后确认")?;
                let pending = self.confirmation.as_ref().context("绑定确认已过期")?;
                if pending["messageId"] != request_id || !future(&pending["confirmationExpiresAt"])
                {
                    bail!("绑定确认已过期，请重新扫码");
                }
                if !confirmed {
                    // 现有云端仅接受肯定确认；本机拒绝后不再接受该二维码的重送。
                    self.pairing_id = None;
                    self.confirmation = None;
                    self.update(|s| {
                        s.pending = None;
                        s.qr_image = None;
                        s.qr_expires_at = None;
                        s.message = "已拒绝本次绑定".into();
                    });
                    return Ok(());
                }
                let mut message = self.base("binding/local-confirm");
                for key in ["bindingId", "confirmationNonce"] {
                    message[key] = pending[key].clone();
                }
                message["summaryDigest"] = pending["bindingSummary"]["summaryDigest"].clone();
                message["confirmed"] = json!(confirmed);
                send(socket, &message)
                    .await
                    .context("绑定确认发送失败，请重试")?;
                self.confirmation_id = message["messageId"].as_str().map(str::to_owned);
                self.confirmation = None;
                self.update(|s| {
                    s.pending = None;
                    s.qr_image = None;
                    s.qr_expires_at = None;
                    s.message = if confirmed {
                        "正在等待云端确认绑定"
                    } else {
                        "已拒绝本次绑定"
                    }
                    .into();
                });
            }
            Action::AutoSync(enabled) => {
                let previous = self.store.state.auto_sync;
                self.store.state.auto_sync = enabled;
                if enabled {
                    if let Err(error) = official_tasks::list_tasks(&self.home)
                        .and_then(|tasks| self.refresh_auto_selection(&tasks))
                    {
                        self.store.state.auto_sync = previous;
                        return Err(error);
                    }
                    self.store.save().context("无法保存自动同步设置")?;
                    self.update(|s| s.auto_sync = true);
                } else {
                    self.store.save().context("无法保存自动同步设置")?;
                    self.update(|s| s.auto_sync = false);
                }
            }
            Action::Select(selected) => {
                let added = selected
                    .difference(&self.store.state.selected)
                    .cloned()
                    .collect();
                let tasks = official_tasks::find_tasks(&self.home, &added)?;
                if tasks.len() != added.len() {
                    bail!("所选任务已归档或不可用，请刷新列表");
                }
                for task in tasks {
                    TaskReader::validate(&self.home, &task)
                        .context("所选任务记录暂不可用，未启用同步")?;
                }
                self.store
                    .state
                    .removed
                    .extend(self.store.state.selected.difference(&selected).cloned());
                self.store.state.removed.retain(|id| !selected.contains(id));
                self.store.state.auto_sync = false;
                self.store.state.selected = selected.clone();
                self.store.save().context("无法保存任务同步选择")?;
                self.update(|s| {
                    s.auto_sync = false;
                    s.selected = selected;
                    s.sync_error = None;
                    s.sync_issues.clear();
                });
            }
        }
        Ok(())
    }

    fn refresh_auto_selection(
        &mut self,
        tasks: &[official_tasks::OfficialTask],
    ) -> anyhow::Result<()> {
        if !self.store.state.auto_sync {
            return Ok(());
        }
        let selected: BTreeSet<String> = tasks
            .iter()
            .take(MAX_SYNCED_TASKS)
            .map(|task| task.id.clone())
            .collect();
        if selected == self.store.state.selected {
            self.update(|s| s.auto_sync = true);
            return Ok(());
        }
        self.store
            .state
            .removed
            .extend(self.store.state.selected.difference(&selected).cloned());
        self.store.state.removed.retain(|id| !selected.contains(id));
        self.store.state.selected = selected.clone();
        self.store.save().context("无法保存自动同步任务")?;
        self.update(|s| {
            s.auto_sync = true;
            s.selected = selected;
        });
        Ok(())
    }

    fn base(&self, kind: &str) -> Value {
        json!({
            "schemaVersion":"1.5", "messageType":kind, "messageId":id(),
            "environment":"dev", "sentAt":now(),
            "pcDeviceId":self.store.state.pc_id, "installationId":self.store.state.installation_id,
        })
    }

    fn event(&mut self, kind: &str) -> anyhow::Result<Value> {
        let version = self.store.version()?;
        let mut value = self.base(kind);
        value["eventId"] = json!(id());
        value["causationId"] = Value::Null;
        value["bindingEpoch"] = json!(self.store.state.epoch);
        value["stateVersion"] = json!(version);
        Ok(value)
    }

    async fn register_pairing(&mut self) -> anyhow::Result<Value> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| anyhow::anyhow!("无法初始化远程连接"))?;
        let pairing = json!({"pairingQrVersion":"2", "environment":"dev", "pairingHandle":id()});
        let mut value = self.base("pairing/register");
        value["pairing"] = pairing.clone();
        value["pcDisplayName"] = json!(desktop_display_name());
        value["expiresAt"] = json!((Utc::now() + chrono::Duration::minutes(5)).to_rfc3339());
        let body = serde_json::to_vec(&value)?;
        let mut request = client
            .post(format!("{}/v1/gateway/pairings", self.config.api_base_url))
            .header("content-type", "application/json")
            .body(body.clone());
        // 签名使用云端规范路径，不包含反向代理的外部前缀。
        for (name, value) in self.store.headers("POST", "/v1/gateway/pairings", &body) {
            request = request.header(name, value);
        }
        let response = request
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("无法连接现有云服务，请检查网络"))?;
        if !response.status().is_success() {
            bail!("云端暂未接受绑定请求，请检查网络和系统时间");
        }
        let result: Value = response
            .json()
            .await
            .map_err(|_| anyhow::anyhow!("云端绑定响应无效"))?;
        if result["schemaVersion"] != "1.5"
            || result["environment"] != "dev"
            || result["messageType"] != "pairing/registered"
            || result["requestMessageId"] != value["messageId"]
            || result["pairingHandle"] != pairing["pairingHandle"]
            || result["registrationState"] != "ready"
            || !future(&result["expiresAt"])
        {
            bail!("云端绑定响应不匹配");
        }
        let svg = qrcode::QrCode::new(serde_json::to_vec(&pairing)?)?
            .render::<qrcode::render::svg::Color>()
            .min_dimensions(256, 256)
            .build();
        self.store.state.enrolled = true;
        self.store.state.enabled = true;
        self.store.save().context("无法保存本机绑定状态")?;
        self.pairing_id = value["messageId"].as_str().map(str::to_owned);
        self.confirmation = None;
        self.confirmation_id = None;
        self.update(|s| {
            s.enabled = true;
            s.pending = None;
            s.qr_image = Some(format!(
                "data:image/svg+xml;base64,{}",
                STANDARD.encode(svg)
            ));
            s.qr_expires_at = result["expiresAt"].as_str().map(str::to_owned);
            s.message = "等待手机扫码".into();
        });
        Ok(pairing)
    }

    async fn connect_gateway(&mut self) -> anyhow::Result<Socket> {
        let mut request = self.config.wss_url.clone().into_client_request()?;
        for (name, value) in self.store.headers("GET", "/v1/gateway/connect", &[]) {
            request.headers_mut().insert(
                tokio_tungstenite::tungstenite::http::HeaderName::from_static(name),
                value.parse()?,
            );
        }
        let (mut socket, _) = tokio::time::timeout(
            Duration::from_secs(15),
            tokio_tungstenite::connect_async(request),
        )
        .await??;
        let mut hello = self.event("pc/hello")?;
        hello["supportedSchemaVersions"] = json!(["1.5"]);
        hello["lastAckEventId"] = Value::Null;
        hello["lastAckStateVersion"] = json!(0);
        send(&mut socket, &hello).await?;
        Ok(socket)
    }

    async fn binding_is_active(&mut self) -> anyhow::Result<bool> {
        // 协议 1.5 没有主动解绑通知；短连接只复核绑定，不上传任何任务内容。
        let mut probe = self.connect_gateway().await?;
        let incoming = tokio::time::timeout(Duration::from_secs(3), probe.next()).await;
        let _ = tokio::time::timeout(Duration::from_secs(2), probe.close(None)).await;
        let Ok(Some(Ok(Message::Text(text)))) = incoming else {
            return Ok(false);
        };
        let message: Value = serde_json::from_str(&text)?;
        if message["schemaVersion"] != "1.5"
            || message["environment"] != "dev"
            || message["messageType"] != "binding/active"
        {
            return Ok(false);
        }
        self.validate_identity(&message)?;
        Ok(
            message["bindingState"] == "active"
                && message["bindingEpoch"] == self.store.state.epoch,
        )
    }

    async fn session(
        &mut self,
        rx: &mut mpsc::Receiver<Request>,
        readers: &mut HashMap<String, TaskReader>,
    ) -> anyhow::Result<()> {
        let mut socket = self.connect_gateway().await?;
        self.update(|s| {
            s.connected = true;
            s.message = "云服务已连接，等待绑定".into();
        });
        let mut bound = false;
        let mut sent: HashMap<String, Value> = HashMap::new();
        let mut outstanding: HashMap<String, (String, u64, bool, Instant)> = HashMap::new();
        let mut subscriptions = BTreeSet::new();
        let mut streams: HashMap<String, ReplyStreamState> = HashMap::new();
        let mut overlays: HashMap<String, live_source::ReplyOverlay> = HashMap::new();
        let (source_scope, scope_receiver) = tokio::sync::watch::channel(BTreeSet::new());
        let (source_output, mut source_events) = mpsc::channel(8);
        let source = live_source::run(scope_receiver, source_output);
        tokio::pin!(source);
        let mut tick = tokio::time::interval(Duration::from_secs(2));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut heartbeat = tokio::time::interval_at(
            tokio::time::Instant::now() + Duration::from_secs(15),
            Duration::from_secs(15),
        );
        let mut binding_refresh = tokio::time::interval_at(
            tokio::time::Instant::now() + Duration::from_secs(30),
            Duration::from_secs(30),
        );
        loop {
            tokio::select! {
                _ = &mut source => break,
                event = source_events.recv() => {
                    let Some(event) = event else { continue; };
                    let task_id = event.thread_id.clone();
                    if !bound || !subscriptions.contains(&task_id)
                        || !self.store.state.selected.contains(&task_id)
                        || readers.get(&task_id).is_some_and(|reader| reader.last_item_id == event.item_id)
                    {
                        continue;
                    }
                    let Some(snapshot) = sent.get(&task_id) else { continue; };
                    let history = snapshot["lastReply"]["text"].as_str().unwrap_or("");
                    let overlay = overlays.entry(task_id.clone()).or_default();
                    if !overlay.apply(event, history) { continue; }
                    let mut projected = snapshot.clone();
                    projected["lastReply"] = json!({"text": overlay.render(history)});
                    projected["lastTurnOutcome"] = json!("none");
                    self.live_reply(&mut socket, &task_id, &projected,
                        streams.entry(task_id.clone()).or_default(), false).await?;
                }
                _ = binding_refresh.tick(), if bound => {
                    if !self.binding_is_active().await? {
                        self.update(|s| {
                            s.bound = false;
                            s.message = "手机绑定已解除或暂时无法确认，已暂停上传".into();
                        });
                        bail!("绑定复核未通过");
                    }
                }
                request = rx.recv() => {
                    let Some(request) = request else { break; };
                    self.handle(request, Some(&mut socket)).await;
                    if !self.store.state.enabled { break; }
                    readers.retain(|id, _| self.store.state.selected.contains(id));
                    sent.retain(|id, _| self.store.state.selected.contains(id));
                    subscriptions.retain(|id| self.store.state.selected.contains(id));
                    streams.retain(|id, _| subscriptions.contains(id));
                    overlays.retain(|id, _| subscriptions.contains(id));
                    live_source::set_scope(&source_scope, &subscriptions);
                }
                _ = heartbeat.tick() => {
                    let mut message = self.event("pc/heartbeat")?;
                    message["pcObservedAt"] = json!(now());
                    message["modelConfigs"] = serde_json::to_value(model_configs(&self.home))?;
                    send(&mut socket, &message).await?;
                }
                _ = tick.tick() => {
                    self.expire_pairing();
                    if outstanding.values().any(|(_, _, _, at)| at.elapsed() > Duration::from_secs(30)) {
                        bail!("同步确认超时");
                    }
                    if !bound { continue; }
                    let mut sync_issues = Vec::new();
                    let tasks = if self.store.state.auto_sync {
                        official_tasks::list_tasks(&self.home)
                    } else {
                        official_tasks::find_tasks(&self.home, &self.store.state.selected)
                    };
                    // 索引繁忙不等于任务被删除；保留原选择，等待下一轮只读同步。
                    let tasks = match tasks {
                        Ok(tasks) => tasks,
                        Err(_) => {
                            self.update(|s| {
                                s.sync_error = Some("任务索引暂忙，已延后同步，不影响本机使用".into());
                                s.sync_issues.clear();
                            });
                            continue;
                        }
                    };
                    self.refresh_auto_selection(&tasks)?;
                    let tasks: HashMap<_, _> = tasks.into_iter().map(|task| (task.id.clone(), task)).collect();
                    readers.retain(|id, _| self.store.state.selected.contains(id));
                    sent.retain(|id, _| self.store.state.selected.contains(id));
                    subscriptions.retain(|id| self.store.state.selected.contains(id));
                    streams.retain(|id, _| subscriptions.contains(id));
                    overlays.retain(|id, _| subscriptions.contains(id));
                    live_source::set_scope(&source_scope, &subscriptions);
                    for task_id in self.store.state.removed.clone() {
                        if outstanding.values().any(|(task, _, tombstone, _)| task == &task_id && *tombstone) { continue; }
                        let mut message = self.event("snapshot/tombstone")?;
                        message["remoteTaskId"] = json!(task_id);
                        message["reason"] = json!("remote_disabled");
                        track(&mut outstanding, &message, &task_id, true);
                        send(&mut socket, &message).await?;
                    }
                    for task_id in self.store.state.selected.clone() {
                        if outstanding.values().any(|(task, _, _, _)| task == &task_id) { continue; }
                        let task = match tasks.get(&task_id) {
                            Some(task) => task,
                            None => {
                                self.store.state.selected.remove(&task_id);
                                self.store.state.removed.insert(task_id.clone());
                                self.store.save()?;
                                self.update(|s| { s.selected.remove(&task_id); });
                                continue;
                            }
                        };
                        let snapshot = match readers.entry(task_id.clone()).or_default().read(&self.home, task) {
                            Ok(snapshot) => snapshot,
                            Err(error) => {
                                sync_issues.push(SyncIssue::from_task_error(task, &error));
                                continue;
                            }
                        };
                        if overlays.get(&task_id).is_some_and(|overlay|
                            overlay.persisted(&readers[&task_id].last_item_id))
                        {
                            overlays.remove(&task_id);
                        }
                        if sent.get(&task_id) == Some(&snapshot) { continue; }
                        let terminal = sent.get(&task_id).is_some_and(|previous| previous["turnStatus"] == "running")
                            && snapshot["turnStatus"] == "completed";
                        let mut message = self.event("snapshot/upsert")?;
                        let mut content = snapshot.clone();
                        for key in ["pcDeviceId", "installationId", "bindingEpoch", "stateVersion"] {
                            content[key] = message[key].clone();
                        }
                        content["remoteTaskId"] = json!(task_id);
                        content["pcObservedAt"] = message["sentAt"].clone();
                        content["lastReplyVersion"] = if content["lastReply"].is_null() { Value::Null } else { message["stateVersion"].clone() };
                        message["snapshot"] = content;
                        message["terminalPushEligible"] = json!(terminal);
                        track(&mut outstanding, &message, &task_id, false);
                        send(&mut socket, &message).await?;
                        if subscriptions.contains(&task_id) {
                            let mut projected = snapshot.clone();
                            let history = snapshot["lastReply"]["text"].as_str().unwrap_or("");
                            if let Some(overlay) = overlays.get(&task_id)
                                && overlay.pending(history)
                            {
                                projected["lastReply"] = json!({"text": overlay.render(history)});
                                projected["lastTurnOutcome"] = json!("none");
                            }
                            self.live_reply(&mut socket, &task_id, &projected,
                                streams.entry(task_id.clone()).or_default(), false).await?;
                        }
                        sent.insert(task_id, snapshot);
                    }
                    self.update(|s| {
                        s.sync_error = (!sync_issues.is_empty())
                            .then(|| "部分任务记录暂不可读，正在等待恢复".into());
                        s.sync_issues = sync_issues;
                    });
                }
                incoming = socket.next() => {
                    let Some(incoming) = incoming else { break; };
                    match incoming? {
                        Message::Text(text) => {
                            let message: Value = serde_json::from_str(&text)?;
                            if message["schemaVersion"] != "1.5" || message["environment"] != "dev" {
                                bail!("远程协议不兼容");
                            }
                            match message["messageType"].as_str().unwrap_or("") {
                                "binding/confirmation-request" => self.pending_confirmation(message)?,
                                "binding/active" => {
                                    self.validate_identity(&message)?;
                                    let epoch = message["bindingEpoch"].as_u64()
                                        .filter(|e| *e >= self.store.state.epoch && *e <= 9_007_199_254_740_991)
                                        .context("绑定代次无效")?;
                                    if message["bindingState"] != "active" { bail!("绑定状态无效"); }
                                    let pairing_completed = self.confirmation_id.as_deref().is_some_and(|id| message["requestMessageId"] == id)
                                        || (self.confirmation_id.is_some() && epoch > self.store.state.epoch);
                                    self.store.state.epoch = epoch;
                                    self.store.save()?;
                                    bound = true;
                                    sent.clear(); outstanding.clear(); subscriptions.clear();
                                    streams.clear();
                                    overlays.clear();
                                    live_source::set_scope(&source_scope, &subscriptions);
                                    if pairing_completed {
                                        self.confirmation = None;
                                        self.confirmation_id = None;
                                        self.pairing_id = None;
                                    }
                                    self.update(|s| {
                                        s.bound = true;
                                        if pairing_completed {
                                            s.pending = None; s.qr_image = None; s.qr_expires_at = None;
                                        }
                                        if s.qr_image.is_none() && s.pending.is_none() { s.message = "手机已绑定".into(); }
                                    });
                                }
                                "sync/ack" => {
                                    self.validate_identity(&message)?;
                                    if message["bindingEpoch"] != self.store.state.epoch { bail!("同步确认不匹配"); }
                                    let event = message["ackEventId"].as_str().context("同步确认无效")?;
                                    if let Some((task, version, removed, _)) = outstanding.get(event) {
                                        if message["ackStateVersion"] != *version { bail!("同步确认版本不匹配"); }
                                        if *removed {
                                            self.store.state.removed.remove(task);
                                            self.store.save()?;
                                        }
                                        outstanding.remove(event);
                                        self.update(|s| { s.last_synced_at = Some(now()); });
                                    }
                                }
                                "reply-stream/subscription" => {
                                    self.validate_identity(&message)?;
                                    if !bound || message["bindingEpoch"] != self.store.state.epoch { bail!("回复订阅不匹配"); }
                                    let task = message["remoteTaskId"].as_str().context("回复订阅无效")?;
                                    if message["active"] == true && self.store.state.selected.contains(task) {
                                        subscriptions.insert(task.to_owned());
                                        if let Some(snapshot) = sent.get(task) {
                                            let mut projected = snapshot.clone();
                                            let history = snapshot["lastReply"]["text"].as_str().unwrap_or("");
                                            if let Some(overlay) = overlays.get(task)
                                                && overlay.pending(history)
                                            {
                                                projected["lastReply"] = json!({"text": overlay.render(history)});
                                                projected["lastTurnOutcome"] = json!("none");
                                            }
                                            self.live_reply(&mut socket, task, &projected,
                                                streams.entry(task.to_owned()).or_default(), true).await?;
                                        }
                                    } else {
                                        subscriptions.remove(task);
                                        streams.remove(task);
                                        overlays.remove(task);
                                    }
                                    live_source::set_scope(&source_scope, &subscriptions);
                                }
                                "command/dispatch" => {
                                    self.validate_identity(&message)?;
                                    if !bound || message["bindingEpoch"] != self.store.state.epoch { bail!("命令身份不匹配"); }
                                    let command_id = message["command"]["commandId"]
                                        .as_str()
                                        .filter(|value| opaque_id(value))
                                        .context("命令标识无效")?;
                                    let payload_digest = message["command"]["payloadDigest"]
                                        .as_str()
                                        .filter(|value| lowercase_sha256(value))
                                        .context("命令摘要无效")?;
                                    let receipt = self.store.command_receipt(command_id)?;
                                    let (status, error_code) = if let Some(receipt) = receipt {
                                        if receipt.payload_digest == payload_digest {
                                            (receipt.status, receipt.error_code)
                                        } else {
                                            ("rejected".into(), Some("invalid_request".into()))
                                        }
                                    } else {
                                        let (status, error_code) = self
                                            .execute_command(&message, &sent, readers)
                                            .await;
                                        self.store.save_command_receipt(
                                            command_id,
                                            payload_digest,
                                            status,
                                            error_code,
                                        )?;
                                        (status.into(), error_code.map(str::to_owned))
                                    };
                                    let mut result = self.event("command/result")?;
                                    result["causationId"] = message["eventId"].clone();
                                    result["command"] = message["command"].clone();
                                    result["command"]["status"] = json!(status);
                                    if let Some(error_code) = error_code.as_deref() {
                                        result["command"]["errorCode"] = json!(error_code);
                                    } else if let Some(command) = result["command"].as_object_mut() {
                                        command.remove("errorCode");
                                    }
                                    result["command"]["appliedStateVersion"] = result["stateVersion"].clone();
                                    send(&mut socket, &result).await?;
                                }
                                _ => bail!("不支持的远程消息"),
                            }
                        }
                        Message::Ping(bytes) => socket.send(Message::Pong(bytes)).await?,
                        Message::Close(_) => break,
                        _ => {}
                    }
                }
            }
        }
        let _ = tokio::time::timeout(Duration::from_secs(2), socket.close(None)).await;
        Ok(())
    }

    fn validate_identity(&self, value: &Value) -> anyhow::Result<()> {
        if value["pcDeviceId"] != self.store.state.pc_id
            || value["installationId"] != self.store.state.installation_id
        {
            bail!("远程设备身份不匹配");
        }
        Ok(())
    }

    async fn execute_command(
        &mut self,
        message: &Value,
        sent: &HashMap<String, Value>,
        readers: &mut HashMap<String, TaskReader>,
    ) -> (&'static str, Option<&'static str>) {
        match self.command_request(message, sent, readers) {
            Ok(request) => match command_source::execute(request).await {
                Ok(result) if result["status"] == "completed" => ("completed", None),
                Ok(result) if result["status"] == "rejected" => {
                    ("rejected", Some(public_command_error(&result["errorCode"])))
                }
                Ok(result) => ("failed", Some(public_command_error(&result["errorCode"]))),
                Err(_) => ("failed", Some("internal")),
            },
            Err(code) => ("rejected", Some(code)),
        }
    }

    fn command_request(
        &self,
        message: &Value,
        sent: &HashMap<String, Value>,
        readers: &mut HashMap<String, TaskReader>,
    ) -> Result<Value, &'static str> {
        self.command_request_with_configs(message, sent, readers, &model_configs(&self.home))
    }

    fn command_request_with_configs(
        &self,
        message: &Value,
        sent: &HashMap<String, Value>,
        readers: &mut HashMap<String, TaskReader>,
        model_configs: &[ModelConfigSummary],
    ) -> Result<Value, &'static str> {
        let command = message.get("command").ok_or("invalid_request")?;
        let command_type = command["commandType"].as_str().ok_or("invalid_request")?;
        let task_id = command["remoteTaskId"].as_str().ok_or("invalid_request")?;
        let client_request_id = command["clientRequestId"]
            .as_str()
            .ok_or("invalid_request")?;
        let expected_version = command["expectedStateVersion"]
            .as_u64()
            .ok_or("invalid_request")?;
        if !opaque_id(task_id)
            || !opaque_id(client_request_id)
            || message["stateVersion"].as_u64() != Some(expected_version)
            || !future(&command["expiresAt"])
        {
            return Err("invalid_request");
        }
        if !self.store.state.selected.contains(task_id) {
            return Err("not_found");
        }
        let task = official_tasks::find_tasks(&self.home, &BTreeSet::from([task_id.to_owned()]))
            .map_err(|_| "internal")?
            .into_iter()
            .next()
            .ok_or("not_found")?;
        // 新建任务只借用来源任务的工作区，不需要解析其可能很大的历史文件。
        let current = if command_type == "create_task" {
            None
        } else {
            Some(
                readers
                    .entry(task_id.to_owned())
                    .or_default()
                    .read(&self.home, &task)
                    .map_err(|_| "internal")?,
            )
        };
        if current.as_ref().is_some_and(|current| {
            sent.get(task_id)
                .is_some_and(|snapshot| snapshot != current)
        }) {
            return Err("state_conflict");
        }
        let status = current
            .as_ref()
            .and_then(|current| current["taskStatus"].as_str())
            .unwrap_or("");
        let allowed = match command_type {
            "create_task" => true,
            "start_task" => matches!(status, "created" | "stopped" | "failed"),
            "send_input" => matches!(status, "running" | "stopped" | "failed"),
            "stop_task" => matches!(
                status,
                "queued" | "starting" | "running" | "pausing" | "paused"
            ),
            _ => return Err("unsupported_operation"),
        };
        if !allowed {
            return Err("invalid_command_state");
        }
        let payload = &message["payload"];
        let text = match command_type {
            "create_task" => payload["initialText"].as_str().ok_or("invalid_request")?,
            "start_task" | "send_input" => payload["text"].as_str().ok_or("invalid_request")?,
            "stop_task" => "",
            _ => unreachable!(),
        };
        if command_type != "stop_task" && (text.trim().is_empty() || text.len() > 8_192) {
            return Err("invalid_request");
        }
        let (model, provider, name) = if command_type == "create_task" {
            let model_id = payload["modelConfigId"].as_str().ok_or("invalid_request")?;
            let config = model_configs
                .iter()
                .find(|config| config.id == model_id)
                .ok_or("invalid_command_state")?;
            let name = payload["text"].as_str().ok_or("invalid_request")?;
            if name.trim().is_empty() || name.chars().count() > 60 {
                return Err("invalid_request");
            }
            (config.model.clone(), config.provider.clone(), name)
        } else {
            (String::new(), String::new(), "")
        };
        let turn_id = readers.get(task_id).map(TaskReader::turn_id).unwrap_or("");
        if command_type == "stop_task" && !opaque_id(turn_id) {
            return Err("invalid_command_state");
        }
        Ok(command_source::request(
            command_type,
            task_id,
            turn_id,
            client_request_id,
            &task.cwd.to_string_lossy(),
            &model,
            &provider,
            text,
            name,
        ))
    }

    fn pending_confirmation(&mut self, value: Value) -> anyhow::Result<()> {
        if self.pairing_id.as_deref() != value["pcPairingMessageId"].as_str()
            || !future(&value["confirmationExpiresAt"])
        {
            return Ok(());
        }
        let summary = &value["bindingSummary"];
        if summary["environment"] != "dev" {
            bail!("绑定环境不匹配");
        }
        let digest = format!(
            "{:x}",
            Sha256::digest(
                format!(
                    "workagents-binding-summary-v1\ndev\n{}\n{}\n{}",
                    summary["pcDisplayName"].as_str().context("电脑名称无效")?,
                    summary["appDisplayName"].as_str().context("手机名称无效")?,
                    summary["safetyPhrase"].as_str().context("核对短语无效")?,
                )
                .as_bytes()
            )
        );
        if summary["summaryDigest"] != digest {
            bail!("绑定核对摘要不匹配");
        }
        let pending = PendingBinding {
            request_id: value["messageId"]
                .as_str()
                .filter(|s| opaque_id(s))
                .context("绑定请求无效")?
                .into(),
            phone_name: summary["appDisplayName"]
                .as_str()
                .context("手机名称无效")?
                .into(),
            safety_phrase: summary["safetyPhrase"]
                .as_str()
                .context("核对短语无效")?
                .into(),
            expires_at: value["confirmationExpiresAt"]
                .as_str()
                .context("绑定有效期无效")?
                .into(),
        };
        self.update(|s| {
            s.pending = Some(pending);
            s.qr_image = None;
            s.message = "手机已扫码，等待本机确认".into();
        });
        self.confirmation = Some(value);
        Ok(())
    }

    fn expire_pairing(&mut self) {
        if self
            .confirmation
            .as_ref()
            .is_some_and(|v| !future(&v["confirmationExpiresAt"]))
        {
            self.confirmation = None;
            self.update(|s| {
                s.pending = None;
                s.message = "绑定确认已过期，请重新扫码".into();
            });
        }
        self.update(|s| {
            if s.qr_expires_at.as_ref().is_some_and(|v| !future(&json!(v))) {
                s.qr_image = None;
                s.qr_expires_at = None;
                if s.pending.is_none() {
                    s.message = "二维码已过期，请重新生成".into();
                }
            }
        });
    }

    async fn live_reply(
        &mut self,
        socket: &mut Socket,
        task: &str,
        snapshot: &Value,
        state: &mut ReplyStreamState,
        reset: bool,
    ) -> anyhow::Result<()> {
        for update in state.updates(
            snapshot["lastReply"]["text"].as_str().unwrap_or(""),
            snapshot["lastTurnOutcome"].as_str().unwrap_or("none"),
            reset,
        ) {
            let mut frame = self.base(update["messageType"].as_str().unwrap());
            frame["bindingEpoch"] = json!(self.store.state.epoch);
            frame["remoteTaskId"] = json!(task);
            frame
                .as_object_mut()
                .unwrap()
                .extend(update.as_object().unwrap().clone());
            send(socket, &frame).await?;
        }
        Ok(())
    }
}

fn future(value: &Value) -> bool {
    value
        .as_str()
        .and_then(|v| DateTime::parse_from_rfc3339(v).ok())
        .is_some_and(|v| v > Utc::now())
}

fn lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn track(
    pending: &mut HashMap<String, (String, u64, bool, Instant)>,
    value: &Value,
    task: &str,
    removed: bool,
) {
    pending.insert(
        value["eventId"].as_str().unwrap().into(),
        (
            task.into(),
            value["stateVersion"].as_u64().unwrap(),
            removed,
            Instant::now(),
        ),
    );
}

async fn send(socket: &mut Socket, value: &Value) -> anyhow::Result<()> {
    tokio::time::timeout(
        Duration::from_secs(10),
        socket.send(Message::Text(serde_json::to_string(value)?.into())),
    )
    .await??;
    Ok(())
}

fn public_command_error(value: &Value) -> &'static str {
    match value.as_str().unwrap_or("") {
        "invalid_request" => "invalid_request",
        "not_found" => "not_found",
        "state_conflict" => "state_conflict",
        "invalid_command_state" => "invalid_command_state",
        "command_expired" => "command_expired",
        "unsupported_operation" => "unsupported_operation",
        _ => "internal",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_sync_issue_reason_is_actionable_and_does_not_expose_raw_errors() {
        for (error, expected) in [
            (
                "任务历史正在分批读取，稍后继续同步",
                "任务历史较长，正在分批读取",
            ),
            (
                "任务记录单条内容过大，已暂停同步",
                "单条记录过大，已暂停同步",
            ),
            ("任务记录格式暂不支持", "任务记录格式不兼容"),
            ("任务记录尚未就绪", "任务记录正在写入，等待完整写入"),
            ("任务记录暂不可用: access denied", "任务记录文件不可访问"),
            (
                "未知 I/O 错误: C:/private/session.jsonl",
                "读取任务记录失败",
            ),
        ] {
            assert_eq!(task_sync_issue_reason(&anyhow::anyhow!(error)), expected);
        }
    }

    #[test]
    fn command_digest_requires_lowercase_sha256_hex() {
        assert!(lowercase_sha256(&"0123456789abcdef".repeat(4)));
        assert!(!lowercase_sha256(&"A".repeat(64)));
        assert!(!lowercase_sha256(&"g".repeat(64)));
        assert!(!lowercase_sha256(&"0".repeat(63)));
    }

    #[cfg(windows)]
    #[test]
    fn auto_sync_keeps_latest_twenty_and_removes_previous_selection() {
        let dir = tempfile::tempdir().unwrap();
        let state_path = dir.path().join("mobile.sqlite");
        let db = rusqlite::Connection::open(dir.path().join("state_5.sqlite")).unwrap();
        db.execute_batch(
            "CREATE TABLE threads(id TEXT, title TEXT, cwd TEXT, rollout_path TEXT,
             archived INTEGER, updated_at INTEGER);",
        )
        .unwrap();
        for index in 0..50 {
            db.execute(
                "INSERT INTO threads VALUES (?1, '测试任务', 'D:/workspace', 'sessions/task.jsonl', 0, ?2)",
                rusqlite::params![format!("fixture_task_{index:04}"), index],
            )
            .unwrap();
        }
        let mut worker = Worker {
            store: Store::open(&state_path).unwrap(),
            config: service_config().unwrap(),
            home: dir.path().to_path_buf(),
            status: Arc::new(Mutex::new(MobileStatus::default())),
            pairing_id: None,
            confirmation: None,
            confirmation_id: None,
        };
        // 模拟升级前已经同步 50 个任务，收窄后需通知云端移除旧任务。
        worker.store.state.selected = (0..50)
            .map(|index| format!("fixture_task_{index:04}"))
            .collect();
        worker
            .store
            .state
            .removed
            .insert("fixture_task_0049".into());
        let tasks = official_tasks::list_tasks(dir.path()).unwrap();
        worker.refresh_auto_selection(&tasks).unwrap();
        let expected: BTreeSet<String> = (30..50)
            .map(|index| format!("fixture_task_{index:04}"))
            .collect();
        assert_eq!(worker.store.state.selected, expected);
        assert_eq!(worker.store.state.removed.len(), 30);
        assert_eq!(worker.status.lock().unwrap().selected, expected);
        assert_eq!(Store::open(&state_path).unwrap().state.selected, expected);

        // 较旧任务重新活跃后进入最近 20 个，被挤出的任务进入移除队列。
        db.execute(
            "UPDATE threads SET updated_at=100 WHERE id='fixture_task_0000'",
            [],
        )
        .unwrap();
        worker
            .refresh_auto_selection(&official_tasks::list_tasks(dir.path()).unwrap())
            .unwrap();
        assert_eq!(worker.store.state.selected.len(), 20);
        assert!(worker.store.state.selected.contains("fixture_task_0000"));
        assert!(!worker.store.state.selected.contains("fixture_task_0030"));
        assert!(!worker.store.state.removed.contains("fixture_task_0000"));
        assert!(worker.store.state.removed.contains("fixture_task_0030"));

        // 手动选择不受自动收窄影响；不足 20 项时不补入旧任务。
        worker.store.state.auto_sync = false;
        let manual = BTreeSet::from(["fixture_task_0001".to_owned()]);
        worker.store.state.selected = manual.clone();
        worker.refresh_auto_selection(&tasks).unwrap();
        assert_eq!(worker.store.state.selected, manual);
        worker.store.state.auto_sync = true;
        worker.refresh_auto_selection(&tasks[..3]).unwrap();
        assert_eq!(worker.store.state.selected.len(), 3);
        worker.refresh_auto_selection(&[]).unwrap();
        assert!(worker.store.state.selected.is_empty());
    }

    #[tokio::test]
    async fn manual_sync_rejects_more_than_twenty_tasks_before_startup() {
        let dir = tempfile::tempdir().unwrap();
        let state_path = dir.path().join("mobile.sqlite");
        let remote = MobileRemote::new(state_path.clone(), dir.path().to_path_buf());
        let selected = (0..21)
            .map(|index| format!("fixture_task_{index:04}"))
            .collect();
        assert_eq!(
            remote.select(selected).await.err().unwrap(),
            "最多可同步 20 个有效任务"
        );
        assert!(!state_path.exists());
    }

    #[cfg(windows)]
    #[test]
    fn authorized_mobile_commands_build_scoped_desktop_requests() {
        use std::io::Write;

        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let workspace = home.join("workspace");
        let sessions = home.join("sessions");
        std::fs::create_dir(&workspace).unwrap();
        std::fs::create_dir(&sessions).unwrap();
        let task_id = "fixture_task_command_0001";
        let turn_id = "fixture_turn_command_0001";
        let path = sessions.join("task.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        for value in [
            json!({"type":"session_meta","payload":{"id":task_id,"source":"vscode"}}),
            json!({"type":"event_msg","payload":{"type":"task_started","turn_id":turn_id}}),
        ] {
            writeln!(file, "{value}").unwrap();
        }
        drop(file);
        let db = rusqlite::Connection::open(home.join("state_5.sqlite")).unwrap();
        db.execute_batch(
            "CREATE TABLE threads(id TEXT, title TEXT, cwd TEXT, rollout_path TEXT,
             archived INTEGER, updated_at INTEGER);",
        )
        .unwrap();
        db.execute(
            "INSERT INTO threads VALUES (?1, '任务', ?2, ?3, 0, 1)",
            rusqlite::params![task_id, workspace.to_string_lossy(), path.to_string_lossy()],
        )
        .unwrap();
        let mut worker = Worker {
            store: Store::open(&home.join("mobile.sqlite")).unwrap(),
            config: service_config().unwrap(),
            home: home.to_path_buf(),
            status: Arc::new(Mutex::new(MobileStatus::default())),
            pairing_id: None,
            confirmation: None,
            confirmation_id: None,
        };
        worker.store.state.selected.insert(task_id.into());
        let task = official_tasks::find_tasks(home, &BTreeSet::from([task_id.to_owned()]))
            .unwrap()
            .remove(0);
        let snapshot = TaskReader::default().read(home, &task).unwrap();
        let sent = HashMap::from([(task_id.to_owned(), snapshot)]);
        let configs = vec![ModelConfigSummary {
            id: "a".repeat(64),
            model: "gpt-test".into(),
            provider: "test-provider".into(),
        }];
        let envelope = |command_type: &str, payload: Value| {
            json!({
                "stateVersion": 7,
                "command": {
                    "commandType": command_type,
                    "remoteTaskId": task_id,
                    "clientRequestId": "fixture_request_command_0001",
                    "expectedStateVersion": 7,
                    "expiresAt": (Utc::now() + chrono::Duration::minutes(1)).to_rfc3339(),
                },
                "payload": payload,
            })
        };

        let send = worker
            .command_request_with_configs(
                &envelope("send_input", json!({"text":"继续"})),
                &sent,
                &mut HashMap::new(),
                &configs,
            )
            .unwrap();
        assert_eq!(send["commandType"], "send_input");
        assert_eq!(send["threadId"], task_id);
        assert_eq!(send["text"], "继续");

        std::fs::write(&path, "not valid json\n").unwrap();
        let mut create_readers = HashMap::new();
        let create = worker
            .command_request_with_configs(
                &envelope(
                    "create_task",
                    json!({"text":"手机新任务","initialText":"检查项目","modelConfigId":"a".repeat(64)}),
                ),
                &sent,
                &mut create_readers,
                &configs,
            )
            .unwrap();
        assert_eq!(create["commandType"], "create_task");
        assert_eq!(create["model"], "gpt-test");
        assert_eq!(create["name"], "手机新任务");
        assert_eq!(create["cwd"], workspace.to_string_lossy().as_ref());
        assert!(create_readers.is_empty());

        let mut file = std::fs::File::create(&path).unwrap();
        for value in [
            json!({"type":"session_meta","payload":{"id":task_id,"source":"vscode"}}),
            json!({"type":"event_msg","payload":{"type":"task_started","turn_id":turn_id}}),
        ] {
            writeln!(file, "{value}").unwrap();
        }
        drop(file);

        let stop = worker
            .command_request_with_configs(
                &envelope("stop_task", json!({})),
                &sent,
                &mut HashMap::new(),
                &configs,
            )
            .unwrap();
        assert_eq!(stop["commandType"], "stop_task");
        assert_eq!(stop["turnId"], turn_id);

        assert_eq!(
            worker.command_request_with_configs(
                &envelope("resume_task", json!({})),
                &sent,
                &mut HashMap::new(),
                &configs,
            ),
            Err("unsupported_operation")
        );
    }

    #[test]
    fn reply_stream_appends_history_and_ends_once() {
        let mut state = ReplyStreamState::default();
        let first = state.updates("第一轮", "none", false);
        assert_eq!(first[0]["messageType"], "reply-stream/reset");
        assert_eq!(first[0]["streamSeq"], 1);
        let next = state.updates("第一轮\n\n---\n\n第二轮", "none", false);
        assert_eq!(next.len(), 1);
        assert_eq!(next[0]["messageType"], "reply-stream/append");
        assert_eq!(next[0]["text"], "\n\n---\n\n第二轮");
        assert_eq!(next[0]["streamSeq"], 2);
        assert_eq!(next[0]["streamId"], first[0]["streamId"]);
        let end = state.updates("第一轮\n\n---\n\n第二轮", "completed", false);
        assert_eq!(end[0]["messageType"], "reply-stream/end");
        assert_eq!(end[0]["streamSeq"], 3);
        assert!(
            state
                .updates("第一轮\n\n---\n\n第二轮", "completed", false)
                .is_empty()
        );
    }

    #[test]
    fn reply_stream_reconnect_and_corrections_reset_complete_history() {
        let text = "历史回复\n\n---\n\n".repeat(100_000);
        let mut state = ReplyStreamState::default();
        let initial = state.updates(&text, "none", false);
        let reconnect = state.updates(&text, "none", true);
        assert_eq!(reconnect[0]["text"], text);
        assert_ne!(reconnect[0]["streamId"], initial[0]["streamId"]);
        let corrected = state.updates("更正回复", "completed", false);
        assert_eq!(corrected.len(), 2);
        assert_eq!(corrected[0]["messageType"], "reply-stream/reset");
        assert_eq!(corrected[0]["text"], "更正回复");
        let next = state.updates("更正回复\n\n---\n\n新回合", "none", false);
        assert_eq!(next[0]["messageType"], "reply-stream/reset");
        assert_ne!(next[0]["streamId"], corrected[0]["streamId"]);
    }

    #[cfg(windows)]
    #[tokio::test(flavor = "current_thread")]
    async fn locked_mobile_store_does_not_block_the_calling_runtime() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mobile.sqlite");
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE fixture(id INTEGER); BEGIN EXCLUSIVE;")
            .unwrap();
        let remote = MobileRemote::new(path, dir.path().to_path_buf());
        let startup = remote.ensure_started();
        tokio::pin!(startup);
        let started = Instant::now();
        tokio::select! {
            biased;
            _ = &mut startup => panic!("锁定的身份库不应立即完成初始化"),
            _ = tokio::time::sleep(Duration::from_millis(30)) => {
                assert!(started.elapsed() < Duration::from_millis(500));
                assert!(!remote.status().enabled);
            }
        }
        assert!(startup.await.is_err());
        db.execute_batch("ROLLBACK").unwrap();
    }

    #[test]
    fn existing_service_and_qr_contract_are_preserved() {
        let config = service_config().unwrap();
        assert!(config.wss_url.ends_with("/v1/gateway/connect"));
        let qr = json!({"pairingQrVersion":"2","environment":"dev","pairingHandle":id()});
        assert_eq!(qr.as_object().unwrap().len(), 3);
        assert!(opaque_id(qr["pairingHandle"].as_str().unwrap()));
        assert!(!opaque_id("../other"));
        assert!(!future(&json!("2020-01-01T00:00:00Z")));
    }

    #[cfg(windows)]
    #[test]
    fn pending_binding_rejects_stale_pairing_and_tampered_summary() {
        let dir = tempfile::tempdir().unwrap();
        let mut worker = Worker {
            store: Store::open(&dir.path().join("desktop.sqlite")).unwrap(),
            config: service_config().unwrap(),
            home: dir.path().to_path_buf(),
            status: Arc::new(Mutex::new(MobileStatus::default())),
            pairing_id: Some("current_pairing_0001".into()),
            confirmation: None,
            confirmation_id: None,
        };
        let mut pending = json!({
            "messageId":id(),"pcPairingMessageId":"old_pairing_0001",
            "confirmationExpiresAt":(Utc::now()+chrono::Duration::minutes(1)).to_rfc3339(),
            "bindingSummary":{"environment":"dev","pcDisplayName":"电脑","appDisplayName":"手机",
                "safetyPhrase":"青山-流水","summaryDigest":"invalid"}
        });
        worker.pending_confirmation(pending.clone()).unwrap();
        assert!(worker.confirmation.is_none());
        pending["pcPairingMessageId"] = json!("current_pairing_0001");
        assert!(worker.pending_confirmation(pending.clone()).is_err());
        assert!(worker.confirmation.is_none());
        pending["bindingSummary"]["summaryDigest"] = json!(format!(
            "{:x}",
            Sha256::digest("workagents-binding-summary-v1\ndev\n电脑\n手机\n青山-流水".as_bytes(),)
        ));
        worker.pending_confirmation(pending.clone()).unwrap();
        assert!(worker.confirmation.is_some());
        assert!(!worker.status.lock().unwrap().bound);
        worker.confirmation.as_mut().unwrap()["confirmationExpiresAt"] =
            json!("2020-01-01T00:00:00Z");
        worker.expire_pairing();
        assert!(worker.status.lock().unwrap().pending.is_none());
    }
}
