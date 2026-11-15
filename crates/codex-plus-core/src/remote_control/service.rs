//! 远程控制的业务层：管理 IPC 连接、侧车、会话跟随，给 HTTP 层提供操作。
//!
//! 原则：执行只发生在桌面窗口持有的会话里。侧车只读和建会话（建完放锁）；
//! 没在桌面打开的会话，先经 CDP 向页面发 `navigate-to-route` 让桌面打开，桌面成为 owner 后再跟随。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, bail};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use super::follower::{Follower, PERMISSION_PRESETS, SettingsChange, build_offline_view, current_controls, discover_owner};
use super::ipc::{IpcConnection, IpcEndpoint, resolve_endpoint};
use super::sidecar::{Sidecar, resolve_codex_cli};
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

pub struct RemoteService {
    config: ServiceConfig,
    ipc: Mutex<Option<Arc<IpcConnection>>>,
    sidecar: Mutex<Option<Arc<Sidecar>>>,
    followers: Mutex<HashMap<String, Arc<Follower>>>,
}

impl RemoteService {
    pub fn new(config: ServiceConfig) -> Arc<Self> {
        Arc::new(Self {
            config,
            ipc: Mutex::new(None),
            sidecar: Mutex::new(None),
            followers: Mutex::new(HashMap::new()),
        })
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

    async fn sidecar(&self) -> anyhow::Result<Arc<Sidecar>> {
        let mut slot = self.sidecar.lock().await;
        if let Some(sidecar) = slot.as_ref().filter(|sidecar| sidecar.is_alive()) {
            return Ok(sidecar.clone());
        }
        let cli = resolve_codex_cli(self.config.app_dir.as_deref())
            .context("没找到 Codex 自带的 CLI（需要它来读取会话列表）")?;
        let sidecar = Sidecar::start(&cli, &self.config.codex_home).await?;
        *slot = Some(sidecar.clone());
        Ok(sidecar)
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
        let conn = self.ipc().await?;
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
        let thread = self.sidecar().await?.read_thread(thread_id).await?;
        Ok(build_offline_view(&thread))
    }

    /// 让桌面打开会话并跟随。
    pub async fn open_thread(&self, thread_id: &str) -> anyhow::Result<Arc<Follower>> {
        if let Some(follower) = self.follower(thread_id).await? {
            return Ok(follower);
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
        let thread_id = sidecar.create_thread(cwd, name).await?;
        self.open_thread(&thread_id).await?;
        Ok(thread_id)
    }

    /// 发消息：会话正在跑就插话，否则开新一轮。
    pub async fn send(&self, thread_id: &str, text: &str) -> anyhow::Result<Value> {
        let follower = self.open_thread(thread_id).await?;
        if follower.is_running() {
            let result = follower.steer(text).await?;
            Ok(json!({ "kind": "steer", "result": result }))
        } else {
            let result = follower.start_turn(text).await?;
            Ok(json!({ "kind": "start", "result": result }))
        }
    }

    /// 会话可选的模型、推理强度、权限模式、协作模式，以及当前值。
    pub async fn options(&self, thread_id: &str) -> anyhow::Result<Value> {
        let sidecar = self.sidecar().await?;
        let models = sidecar.call("model/list", json!({ "cursor": null, "includeHidden": false, "limit": 100 })).await?;
        let modes = sidecar.call("collaborationMode/list", json!({})).await.unwrap_or(Value::Null);
        let current = match self.follower(thread_id).await? {
            Some(follower) => follower.state().as_ref().map(current_controls).unwrap_or(Value::Null),
            None => Value::Null,
        };
        Ok(json!({
            "models": models.get("data").and_then(Value::as_array).map(|models| models.iter().map(model_option).collect::<Vec<_>>()).unwrap_or_default(),
            "permissionModes": PERMISSION_PRESETS.iter().map(|preset| json!({ "id": preset.id, "label": preset.label, "detail": preset.detail })).collect::<Vec<_>>(),
            "collaborationModes": collaboration_modes(&modes),
            "current": current,
        }))
    }

    /// 改会话设置（下一轮起生效）。会话没在桌面打开时先打开。
    pub async fn update_settings(&self, thread_id: &str, body: &Value) -> anyhow::Result<Value> {
        let change = SettingsChange::from_json(body)?;
        let follower = self.open_thread(thread_id).await?;
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
        let follower = self.follower(thread_id).await?.context("会话没有在桌面打开")?;
        follower.interrupt().await
    }

    pub async fn respond(&self, thread_id: &str, request_id: &Value, body: &Value) -> anyhow::Result<Value> {
        let follower = self.follower(thread_id).await?.context("会话没有在桌面打开")?;
        follower.respond(request_id, body).await
    }

    pub async fn shutdown(&self) {
        for (_, follower) in self.followers.lock().await.drain() {
            follower.stop().await;
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

