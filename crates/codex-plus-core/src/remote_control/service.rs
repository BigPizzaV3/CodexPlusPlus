//! 远程控制的业务层：管理 IPC 连接、侧车、会话跟随，给 HTTP 层提供操作。
//!
//! 两条执行路径：
//! - 会话已在桌面打开、桌面窗口持有它：跟随桌面（`Follower`），操作经 IPC follower 协议下发；
//! - 没在桌面打开：**后台执行**（`headless::HeadlessSession`，侧车 `thread/resume` + `turn/start`），
//!   桌面窗口不切换，用户可以在桌面里继续干别的。只有用户主动点「在桌面打开」才经 CDP 让桌面打开它。
//!
//! 后台执行跑的时候侧车持有该会话的写锁，此时「在桌面打开」会先放锁再打开。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, bail};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use super::follower::{Follower, PERMISSION_PRESETS, SettingsChange, build_offline_view, current_controls, discover_owner};
use super::headless::{HeadlessSession, Hub, headless_supports};
use super::jiajia::Jiajia;
use super::ipc::{IpcConnection, IpcEndpoint, resolve_endpoint};
use super::sidecar::{Sidecar, resolve_codex_cli};
use super::uploads::{Attachment, user_inputs};
use super::versions::{IpcVersions, app_asar_path, default_versions, read_versions_from_asar};

const OPEN_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_DEBUG_PORT: u16 = 9229;

#[derive(Clone, Debug)]
pub struct ServiceConfig {
    pub codex_home: PathBuf,
    pub app_dir: Option<PathBuf>,
    /// Codex 桌面的调试端口；None 时每次从 latest-status.json 读（Codex++ 每次启动 Codex 都会写）。
    pub debug_port: Option<u16>,
}

impl ServiceConfig {
    pub fn detect() -> Self {
        Self {
            codex_home: crate::codex_home::default_codex_home_dir(),
            app_dir: crate::app_paths::resolve_codex_app_dir(None),
            debug_port: None,
        }
    }

    fn debug_port(&self) -> u16 {
        self.debug_port.unwrap_or_else(debug_port)
    }
}

/// 实时推送的来源。
pub enum StreamSource {
    Desktop(Arc<Follower>),
    Background(Arc<HeadlessSession>),
}

pub struct RemoteService {
    config: ServiceConfig,
    ipc: Mutex<Option<Arc<IpcConnection>>>,
    sidecar: Mutex<Option<Arc<Sidecar>>>,
    followers: Mutex<HashMap<String, Arc<Follower>>>,
    headless: Mutex<Option<Arc<Hub>>>,
    /// 加加（聊天机器人），第一次用到时建。
    jiajia: std::sync::OnceLock<Arc<Jiajia>>,
}

impl RemoteService {
    pub fn new(config: ServiceConfig) -> Arc<Self> {
        Arc::new(Self {
            config,
            ipc: Mutex::new(None),
            sidecar: Mutex::new(None),
            followers: Mutex::new(HashMap::new()),
            headless: Mutex::new(None),
            jiajia: std::sync::OnceLock::new(),
        })
    }

    /// 加加。状态放在 Codex++ 状态目录的 `jiajia/` 下。
    pub fn jiajia(self: &Arc<Self>) -> Arc<Jiajia> {
        self.jiajia.get_or_init(|| Jiajia::new(Jiajia::default_dir(), Arc::downgrade(self))).clone()
    }

    pub fn codex_home(&self) -> &Path {
        &self.config.codex_home
    }

    fn endpoint(&self) -> IpcEndpoint {
        resolve_endpoint(&self.config.codex_home)
    }

    fn versions(&self) -> (IpcVersions, &'static str) {
        let Some(app_dir) = &self.config.app_dir else { return (default_versions(), "builtin") };
        match read_versions_from_asar(&app_asar_path(app_dir)) {
            Ok(versions) => (versions, "app.asar"),
            Err(_) => (default_versions(), "builtin"),
        }
    }

    async fn ipc(&self) -> anyhow::Result<Arc<IpcConnection>> {
        let mut slot = self.ipc.lock().await;
        if let Some(conn) = slot.as_ref().filter(|conn| !conn.is_closed()) {
            return Ok(conn.clone());
        }
        let (versions, _) = self.versions();
        let conn = IpcConnection::connect(&self.endpoint(), "codex-plus-remote", versions).await?;
        *slot = Some(conn.clone());
        // 新连接上之前的跟随全部失效。
        self.followers.lock().await.clear();
        Ok(conn)
    }

    pub(crate) async fn sidecar(&self) -> anyhow::Result<Arc<Sidecar>> {
        let mut slot = self.sidecar.lock().await;
        if let Some(sidecar) = slot.as_ref().filter(|sidecar| sidecar.is_alive()) {
            return Ok(sidecar.clone());
        }
        let cli = resolve_codex_cli(self.config.app_dir.as_deref())
            .context("没找到 Codex 自带的 CLI（需要它来读取会话列表）")?;
        let sidecar = Sidecar::start(&cli, &self.config.codex_home).await?;
        *slot = Some(sidecar.clone());
        // 换了进程：之前的后台会话都不在它上面了。
        self.headless.lock().await.take();
        Ok(sidecar)
    }

    /// 后台执行的分发器（和只读侧车用同一个 app-server 进程）。
    async fn headless(&self) -> anyhow::Result<Arc<Hub>> {
        let sidecar = self.sidecar().await?;
        let mut slot = self.headless.lock().await;
        if let Some(hub) = slot.as_ref() {
            return Ok(hub.clone());
        }
        let hub = Hub::new(&sidecar);
        *slot = Some(hub.clone());
        Ok(hub)
    }

    /// 已有的后台会话（不新建）。
    async fn headless_session(&self, thread_id: &str) -> Option<Arc<HeadlessSession>> {
        let hub = self.headless.lock().await.clone()?;
        hub.get(thread_id).await.filter(|session| session.is_alive())
    }

    /// 取（或建）一个后台会话。新建只读历史、不拿锁；开一轮时才拿。
    pub async fn ensure_headless(&self, thread_id: &str) -> anyhow::Result<Arc<HeadlessSession>> {
        let hub = self.headless().await?;
        if let Some(session) = hub.get(thread_id).await.filter(|session| session.is_alive()) {
            return Ok(session);
        }
        let sidecar = self.sidecar().await?;
        let thread = sidecar.read_thread(thread_id).await?;
        if thread.is_null() {
            bail!("找不到这个会话");
        }
        let session = HeadlessSession::new(&sidecar, &thread)?;
        hub.insert(session.clone()).await;
        Ok(session)
    }

    /// 推送来源：桌面持有就跟随桌面，否则用后台会话（没有就建一个，不拿锁）。
    pub async fn stream_source(&self, thread_id: &str) -> anyhow::Result<StreamSource> {
        if let Some(follower) = self.follower(thread_id).await? {
            return Ok(StreamSource::Desktop(follower));
        }
        Ok(StreamSource::Background(self.ensure_headless(thread_id).await?))
    }

    /// Codex 桌面的调试端口（桌面镜像用）。
    pub fn debug_port(&self) -> u16 {
        self.config.debug_port()
    }

    pub async fn status(&self) -> Value {
        let (versions, version_source) = self.versions();
        let ipc = match self.ipc().await {
            Ok(conn) => json!({ "connected": true, "clientId": conn.client_id() }),
            Err(error) => json!({ "connected": false, "error": error.to_string() }),
        };
        json!({
            "codexHome": self.config.codex_home,
            "appDir": self.config.app_dir,
            "endpoint": self.endpoint().to_string(),
            "ipc": ipc,
            "ipcVersionSource": version_source,
            "ipcStreamVersion": versions.get("thread-stream-state-changed"),
            "cli": resolve_codex_cli(self.config.app_dir.as_deref()),
            "debugPort": self.config.debug_port(),
        })
    }

    pub async fn list_threads(&self, cursor: Option<&str>) -> anyhow::Result<Value> {
        let sidecar = self.sidecar().await?;
        let result = sidecar.list_threads(cursor, 50).await?;
        let threads = result
            .get("data")
            .and_then(Value::as_array)
            .map(|threads| {
                threads
                    .iter()
                    .filter(|thread| thread.get("ephemeral").and_then(Value::as_bool) != Some(true))
                    .map(|thread| {
                        json!({
                            "id": thread.get("id"),
                            "title": thread.get("name").filter(|name| !name.is_null()).or_else(|| thread.get("preview")),
                            "preview": thread.get("preview"),
                            "cwd": thread.get("cwd"),
                            "model": thread.get("model"),
                            "updatedAt": thread.get("updatedAt"),
                            "status": thread.pointer("/status/type"),
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Ok(json!({ "threads": threads, "nextCursor": result.get("nextCursor") }))
    }

    /// 已在跟随且仍有效的会话；否则尝试跟随（不会让桌面打开它）。
    pub async fn follower(&self, thread_id: &str) -> anyhow::Result<Option<Arc<Follower>>> {
        if let Some(follower) = self.followers.lock().await.get(thread_id).filter(|f| f.is_alive()) {
            return Ok(Some(follower.clone()));
        }
        // 桌面 Codex 没开（连不上 IPC）就当没有桌面持有：查看读历史、发消息走后台执行。
        let Ok(conn) = self.ipc().await else {
            return Ok(None);
        };
        let Some(follower) = Follower::start(conn, thread_id).await? else {
            return Ok(None);
        };
        self.followers.lock().await.insert(thread_id.to_string(), follower.clone());
        Ok(Some(follower))
    }

    pub async fn thread_view(&self, thread_id: &str) -> anyhow::Result<Value> {
        if let Some(follower) = self.follower(thread_id).await? {
            return Ok(follower.view());
        }
        // 后台正在跑（或刚跑完还没放锁）时用它的实时视图；否则读磁盘上的历史。
        if let Some(session) = self.headless_session(thread_id).await {
            if session.is_holding() || session.is_running().await {
                return Ok(session.view().await);
            }
        }
        let thread = self.sidecar().await?.read_thread(thread_id).await?;
        Ok(build_offline_view(&thread))
    }

    /// 让桌面打开会话并跟随。后台会话先放锁，桌面才拿得到写入权。
    pub async fn open_thread(&self, thread_id: &str) -> anyhow::Result<Arc<Follower>> {
        if let Some(follower) = self.follower(thread_id).await? {
            return Ok(follower);
        }
        if let Some(session) = self.headless_session(thread_id).await {
            if session.is_running().await {
                bail!("这个会话正在后台运行，等它跑完再在桌面打开");
            }
            session.stop().await;
            if let Some(hub) = self.headless.lock().await.clone() {
                hub.remove(thread_id).await;
            }
        }
        open_in_desktop(self.config.debug_port(), thread_id).await?;
        let conn = self.ipc().await?;
        let deadline = tokio::time::Instant::now() + OPEN_TIMEOUT;
        loop {
            if discover_owner(&conn, thread_id).await?.is_some() {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                bail!("桌面 10 秒内没有打开这个会话");
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        self.follower(thread_id).await?.context("桌面打开了会话，但跟随失败")
    }

    pub async fn create_thread(&self, cwd: &str, name: &str) -> anyhow::Result<String> {
        let cwd_path = Path::new(cwd);
        if !cwd_path.is_absolute() || !cwd_path.is_dir() {
            bail!("工作目录必须是本机已存在的绝对路径");
        }
        let sidecar = self.sidecar().await?;
        // 建完放锁就返回：第一条消息走后台执行，桌面不用切过去。
        sidecar.create_thread(cwd, name).await
    }

    /// 发消息：会话正在跑就插话，否则开新一轮。
    ///
    /// 已由桌面持有的会话跟随桌面；否则在后台执行（桌面窗口不切换）。
    pub async fn send(&self, thread_id: &str, text: &str) -> anyhow::Result<Value> {
        self.send_with(thread_id, text, &[]).await
    }

    /// 发消息（可带图片 / 文件附件）。
    pub async fn send_with(&self, thread_id: &str, text: &str, attachments: &[Attachment]) -> anyhow::Result<Value> {
        let input = user_inputs(text, attachments);
        if let Some(follower) = self.follower(thread_id).await? {
            let value = if follower.is_running() {
                json!({ "kind": "steer", "result": follower.steer(text, input, super::uploads::desktop_attachments(attachments)).await? })
            } else {
                json!({ "kind": "start", "result": follower.start_turn(input, super::uploads::desktop_attachments(attachments)).await? })
            };
            return Ok(value);
        }
        let session = self.ensure_headless(thread_id).await?;
        let value = if session.is_running().await {
            json!({ "kind": "steer", "result": session.steer(input).await? })
        } else {
            json!({ "kind": "start", "background": true, "result": session.start_turn(input).await? })
        };
        Ok(value)
    }

    /// 会话可选的模型、推理强度、权限模式、协作模式，以及当前值。
    pub async fn options(&self, thread_id: &str) -> anyhow::Result<Value> {
        let models = self.models().await?;
        let modes = self.sidecar().await?.call("collaborationMode/list", json!({})).await.unwrap_or(Value::Null);
        let current = match self.follower(thread_id).await? {
            Some(follower) => follower.state().as_ref().map(current_controls).unwrap_or(Value::Null),
            None => match self.headless_session(thread_id).await {
                Some(session) => session.view().await.get("controls").cloned().unwrap_or(Value::Null),
                None => Value::Null,
            },
        };
        Ok(json!({
            "models": models,
            "permissionModes": PERMISSION_PRESETS.iter().map(|preset| json!({ "id": preset.id, "label": preset.label, "detail": preset.detail })).collect::<Vec<_>>(),
            "collaborationModes": collaboration_modes(&modes),
            "current": current,
        }))
    }

    /// 可选的模型（含推理强度档位）。
    pub async fn models(&self) -> anyhow::Result<Vec<Value>> {
        let models = self.sidecar().await?.call("model/list", json!({ "cursor": null, "includeHidden": false, "limit": 100 })).await?;
        Ok(models.get("data").and_then(Value::as_array).map(|models| models.iter().map(model_option).collect()).unwrap_or_default())
    }

    /// 改会话设置（下一轮起生效）。
    ///
    /// 桌面持有的会话经桌面改；否则记在后台会话上，随下一轮发出（不打开桌面）。
    /// 计划模式后台不支持：只有这时才需要先在桌面打开。
    pub async fn update_settings(&self, thread_id: &str, body: &Value) -> anyhow::Result<Value> {
        let change = SettingsChange::from_json(body)?;
        let follower = match self.follower(thread_id).await? {
            Some(follower) => follower,
            None if change.collaboration_mode.as_deref().is_none_or(headless_supports) => {
                let session = self.ensure_headless(thread_id).await?;
                session.apply_settings(&change).await?;
                return Ok(session.view().await);
            }
            None => self.open_thread(thread_id).await?,
        };
        let mut updates = follower.subscribe();
        follower.update_settings(&change).await?;
        // 等 owner 把改后的值推回来再返回：设置分几次补丁到达，只等第一次推送的话，
        // 紧接着发的消息可能还按旧的协作模式发出（实测会丢计划模式）。
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while !follower.state().as_ref().map(current_controls).is_some_and(|controls| change.is_reflected_in(&controls)) {
            if tokio::time::timeout_at(deadline, updates.changed()).await.is_err() {
                bail!("桌面 5 秒内没有确认新设置");
            }
        }
        Ok(follower.view())
    }

    pub async fn interrupt(&self, thread_id: &str) -> anyhow::Result<Value> {
        if let Some(follower) = self.follower(thread_id).await? {
            return follower.interrupt().await;
        }
        self.headless_session(thread_id).await.context("会话没有在运行")?.interrupt().await
    }

    pub async fn respond(&self, thread_id: &str, request_id: &Value, body: &Value) -> anyhow::Result<Value> {
        if let Some(follower) = self.follower(thread_id).await? {
            return follower.respond(request_id, body).await;
        }
        self.headless_session(thread_id).await.context("这个请求已经处理过了")?.respond(request_id, body).await
    }

    pub async fn shutdown(&self) {
        for (_, follower) in self.followers.lock().await.drain() {
            follower.stop().await;
        }
        if let Some(hub) = self.headless.lock().await.take() {
            for session in hub.drain().await {
                session.stop().await;
            }
        }
        if let Some(sidecar) = self.sidecar.lock().await.take() {
            sidecar.shutdown();
        }
        self.ipc.lock().await.take();
    }
}

/// Codex++ 启动 Codex 时开的调试端口（写在 latest-status.json 里）。
pub fn debug_port() -> u16 {
    crate::status::StatusStore::default()
        .load_latest()
        .ok()
        .flatten()
        .and_then(|status| status.debug_port)
        .unwrap_or(DEFAULT_DEBUG_PORT)
}

/// 经 CDP 让桌面窗口打开会话：页面在 `window` 上收 `{ type: "navigate-to-route", path }`，
/// 主进程处理 `codex://threads/<id>` 时发的也是这条消息。
pub async fn open_in_desktop(port: u16, thread_id: &str) -> anyhow::Result<()> {
    let thread_id = uuid::Uuid::parse_str(thread_id).context("会话 ID 无效")?.to_string();
    let targets = crate::cdp::list_targets(port)
        .await
        .with_context(|| format!("连不上 Codex 的调试端口 {port}：只有从 Codex++ 启动的 Codex 才能远程打开会话"))?;
    let target = crate::cdp::pick_injectable_codex_page_target(&targets)?;
    let websocket = target.web_socket_debugger_url.context("Codex 页面没有调试地址")?;
    let script = format!(
        "window.dispatchEvent(new MessageEvent('message', {{ data: {{ type: 'navigate-to-route', path: '/local/{thread_id}' }} }})); true"
    );
    crate::bridge::evaluate_script(&websocket, &script).await?;
    Ok(())
}

fn model_option(model: &Value) -> Value {
    let efforts: Vec<Value> = model
        .get("supportedReasoningEfforts")
        .and_then(Value::as_array)
        .map(|efforts| {
            efforts
                .iter()
                .filter_map(|effort| effort.get("reasoningEffort").or(Some(effort)).and_then(Value::as_str).map(|effort| json!(effort)))
                .collect()
        })
        .unwrap_or_default();
    json!({
        "id": model.get("model").or_else(|| model.get("id")),
        "label": model.get("displayName").or_else(|| model.get("model")),
        "description": model.get("description"),
        "efforts": efforts,
        "defaultEffort": model.get("defaultReasoningEffort"),
        "isDefault": model.get("isDefault"),
    })
}

/// 协作模式：服务端给的列表里挑计划和默认两种；拿不到时给内置的两种。
fn collaboration_modes(list: &Value) -> Vec<Value> {
    let label = |mode: &str| if mode == "plan" { "计划" } else { "默认" };
    let mut modes: Vec<Value> = list
        .get("data")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("mode").and_then(Value::as_str))
                .filter(|mode| *mode == "plan" || *mode == "default")
                .map(|mode| json!({ "id": mode, "label": label(mode) }))
                .collect()
        })
        .unwrap_or_default();
    if modes.is_empty() {
        modes = vec![json!({ "id": "default", "label": "默认" }), json!({ "id": "plan", "label": "计划" })];
    }
    modes.sort_by_key(|mode| if mode["id"] == "default" { 0 } else { 1 });
    modes
}

