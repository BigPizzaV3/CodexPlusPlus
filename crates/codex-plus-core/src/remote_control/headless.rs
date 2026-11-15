//! 后台执行：让侧车 app-server 自己跑一轮，桌面窗口不用切过去。
//!
//! 为什么需要：「跟随桌面」这条路必须由桌面窗口持有会话，网页发消息就得让桌面打开它，
//! 用户正在桌面看别的会话时会被打断。这里用侧车 app-server（`thread/resume` + `turn/start`）在后台跑，
//! 桌面保持原样；审批、提问挂在视图的 `requests` 上，网页回答后经 `respond_server_request` 交回。
//!
//! 写锁只在跑的时候持有：开一轮前 `thread/resume` 拿锁，这一轮结束（且没有待处理的请求）后放锁，
//! 所以跑完之后桌面随时能打开这个会话、看到完整历史。
//!
//! 限制：`turn/start` 没有协作模式字段，后台执行不支持计划模式（网页提示先在桌面打开）。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use anyhow::{Context, bail};
use serde_json::{Value, json};
use tokio::sync::{Mutex, watch};

use super::follower::{SettingsChange, permission_mode_of, permission_preset, turn_error, view_item};
use super::sidecar::Sidecar;

/// 一轮跑完后等多久放锁：给网页留出读完最后几条推送的时间，也避免连发两条时反复拿锁放锁。
const RELEASE_AFTER_TURN: Duration = Duration::from_millis(800);
const TURN_START_TIMEOUT: Duration = Duration::from_secs(60);
/// 视图里最多带多少轮（取最近的）。
const VIEW_TURN_LIMIT: usize = 40;

#[derive(Default)]
struct SessionState {
    title: Option<String>,
    cwd: String,
    model: Option<String>,
    effort: Option<String>,
    /// None：还没拿过锁，不知道会话当前的权限（网页显示「沿用会话设置」）。
    permission_mode: Option<String>,
    runtime: &'static str,
    revision: u64,
    turns: Vec<Value>,
    requests: Vec<Value>,
    turn_id: Option<String>,
    token_usage: Value,
    /// 网页改过、还没随 `turn/start` 发出去的设置。
    pending: SettingsChange,
}

/// 一个会话的后台执行状态。
pub struct HeadlessSession {
    pub thread_id: String,
    sidecar: Weak<Sidecar>,
    state: Mutex<SessionState>,
    /// 串行化「拿锁 + 开一轮」与「放锁」，避免放锁正好撞上刚开的一轮。
    lifecycle: Mutex<()>,
    changed: watch::Sender<u64>,
    alive: AtomicBool,
    /// 侧车是否持有这个会话的写锁。
    holding: AtomicBool,
}

impl HeadlessSession {
    /// 由侧车读到的会话（`thread/read`）建一个后台会话。这一步不拿锁。
    pub fn new(sidecar: &Arc<Sidecar>, thread: &Value) -> anyhow::Result<Arc<Self>> {
        let thread_id = thread.get("id").and_then(Value::as_str).context("会话没有 ID")?.to_string();
        let (changed, _) = watch::channel(0);
        let mut state = SessionState { runtime: "idle", ..Default::default() };
        refresh_from_thread(&mut state, thread);
        Ok(Arc::new(Self {
            thread_id,
            sidecar: Arc::downgrade(sidecar),
            state: Mutex::new(state),
            lifecycle: Mutex::new(()),
            changed,
            alive: AtomicBool::new(true),
            holding: AtomicBool::new(false),
        }))
    }

    fn sidecar(&self) -> anyhow::Result<Arc<Sidecar>> {
        self.sidecar.upgrade().filter(|sidecar| sidecar.is_alive()).context("后台执行进程已退出")
    }

    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst) && self.sidecar.upgrade().is_some_and(|sidecar| sidecar.is_alive())
    }

    pub fn is_holding(&self) -> bool {
        self.holding.load(Ordering::SeqCst)
    }

    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    pub async fn is_running(&self) -> bool {
        self.state.lock().await.runtime == "active"
    }

    pub async fn view(&self) -> Value {
        let state = self.state.lock().await;
        let skip = state.turns.len().saturating_sub(VIEW_TURN_LIMIT);
        json!({
            "threadId": self.thread_id,
            "live": true,
            "headless": true,
            "revision": state.revision,
            "title": state.title,
            "cwd": state.cwd,
            "model": state.model,
            "runtime": state.runtime,
            "controls": {
                "model": state.pending.model.clone().or_else(|| state.model.clone()),
                "effort": match &state.pending.effort { Some(effort) => effort.clone(), None => state.effort.clone() },
                "permissionMode": state.pending.permission_mode.clone().or_else(|| state.permission_mode.clone()),
                "collaborationMode": "default",
            },
            "truncatedTurns": skip,
            "tokenUsage": state.token_usage,
            "turns": state.turns.iter().skip(skip).map(|turn| json!({
                "turnId": turn.get("id").or_else(|| turn.get("turnId")),
                "status": turn.get("status"),
                "error": turn_error(turn.get("error")),
                "startedAtMs": turn.get("startedAt").and_then(Value::as_i64).map(|seconds| seconds * 1000),
                "items": turn.get("items").and_then(Value::as_array).map(|items| items.iter().map(view_item).collect::<Vec<_>>()).unwrap_or_default(),
            })).collect::<Vec<_>>(),
            "requests": state.requests,
        })
    }

    /// 改设置：记下来，随下一次 `turn/start` 发出（app-server 会把它们沿用到后续轮次）。
    pub async fn apply_settings(&self, change: &SettingsChange) -> anyhow::Result<()> {
        if change.collaboration_mode.as_deref().is_some_and(|mode| !headless_supports(mode)) {
            bail!("后台运行不支持计划模式：先点「在桌面打开」，再在桌面里切到计划模式");
        }
        if let Some(mode) = &change.permission_mode {
            permission_preset(mode).context("未知的权限模式")?;
        }
        let mut state = self.state.lock().await;
        if let Some(model) = &change.model {
            state.pending.model = Some(model.clone());
        }
        if let Some(effort) = &change.effort {
            state.pending.effort = Some(effort.clone());
        }
        if let Some(mode) = &change.permission_mode {
            state.pending.permission_mode = Some(mode.clone());
        }
        self.touch(&mut state);
        Ok(())
    }

    /// 拿写锁：`thread/resume`，顺带读回会话当前的模型 / 推理强度 / 权限和最新历史。
    async fn ensure_loaded(&self, sidecar: &Sidecar) -> anyhow::Result<()> {
        if self.is_holding() {
            return Ok(());
        }
        let cwd = self.state.lock().await.cwd.clone();
        let mut params = json!({ "threadId": self.thread_id });
        if !cwd.is_empty() {
            params["cwd"] = json!(cwd);
        }
        let resumed = sidecar.call("thread/resume", params).await.map_err(|error| {
            if error.to_string().contains("active writer") {
                anyhow::anyhow!("桌面正占用这个会话，暂时不能在后台运行；稍后再试，或点「在桌面打开」")
            } else {
                error
            }
        })?;
        self.holding.store(true, Ordering::SeqCst);
        let history = sidecar.read_thread(&self.thread_id).await.unwrap_or(Value::Null);
        let mut state = self.state.lock().await;
        if !history.is_null() {
            refresh_from_thread(&mut state, &history);
        }
        if let Some(model) = resumed.get("model").and_then(Value::as_str) {
            state.model = Some(model.to_string());
        }
        state.effort = resumed.get("reasoningEffort").and_then(Value::as_str).map(str::to_string).or(state.effort.take());
        let settings = json!({
            "sandboxPolicy": resumed.get("sandbox"),
            "approvalPolicy": resumed.get("approvalPolicy"),
            "approvalsReviewer": resumed.get("approvalsReviewer"),
        });
        state.permission_mode = Some(permission_mode_of(&settings).to_string());
        self.touch(&mut state);
        Ok(())
    }

    /// 开一轮。网页改过的设置随这一轮发出。
    pub async fn start_turn(&self, input: Vec<Value>) -> anyhow::Result<Value> {
        let sidecar = self.sidecar()?;
        let _lifecycle = self.lifecycle.lock().await;
        self.ensure_loaded(&sidecar).await?;
        let mut params = json!({ "threadId": self.thread_id, "input": input });
        let pending = std::mem::take(&mut self.state.lock().await.pending);
        if let Some(model) = &pending.model {
            params["model"] = json!(model);
        }
        if let Some(Some(effort)) = &pending.effort {
            params["effort"] = json!(effort);
        }
        if let Some(preset) = pending.permission_mode.as_deref().and_then(permission_preset) {
            params["approvalPolicy"] = json!(preset.approval_policy);
            params["approvalsReviewer"] = json!(preset.reviewer);
            params["sandboxPolicy"] = sandbox_policy_for(preset.profile);
        }
        let result = match tokio::time::timeout(TURN_START_TIMEOUT, sidecar.call("turn/start", params)).await {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => {
                self.state.lock().await.pending = pending;
                return Err(error);
            }
            Err(_) => {
                self.state.lock().await.pending = pending;
                bail!("后台开这一轮超时");
            }
        };
        let mut state = self.state.lock().await;
        // 先记下这一轮，免得 turn/started 还没到时插话、中断找不到轮次，或者放锁抢在前面。
        state.runtime = "active";
        state.turn_id = result.pointer("/turn/id").and_then(Value::as_str).map(str::to_string);
        if let Some(model) = pending.model {
            state.model = Some(model);
        }
        if let Some(effort) = pending.effort {
            state.effort = effort;
        }
        if let Some(mode) = pending.permission_mode {
            state.permission_mode = Some(mode);
        }
        self.touch(&mut state);
        Ok(json!({ "turn": result.get("turn").cloned().unwrap_or(Value::Null) }))
    }

    pub async fn steer(&self, input: Vec<Value>) -> anyhow::Result<Value> {
        let turn_id = self.state.lock().await.turn_id.clone().context("这一轮已经结束")?;
        self.sidecar()?
            .call("turn/steer", json!({ "threadId": self.thread_id, "expectedTurnId": turn_id, "input": input }))
            .await
    }

    pub async fn interrupt(&self) -> anyhow::Result<Value> {
        let turn_id = self.state.lock().await.turn_id.clone().context("这一轮已经结束")?;
        self.sidecar()?.call("turn/interrupt", json!({ "threadId": self.thread_id, "turnId": turn_id })).await
    }

    /// 回答审批 / 提问：`body` 与桌面那条路一致（decision / answers / response）。
    pub async fn respond(&self, request_id: &Value, body: &Value) -> anyhow::Result<Value> {
        let sidecar = self.sidecar()?;
        let mut state = self.state.lock().await;
        let index = state
            .requests
            .iter()
            .position(|request| request.get("id") == Some(request_id))
            .context("这个请求已经处理过了")?;
        let method = state.requests[index].get("method").and_then(Value::as_str).unwrap_or_default().to_string();
        let result = app_server_result(&method, body)?;
        sidecar.respond_server_request(request_id, result).await?;
        state.requests.remove(index);
        self.touch(&mut state);
        Ok(json!({ "ok": true }))
    }

    /// 不再使用：放锁并标记失效。
    pub async fn stop(&self) {
        self.alive.store(false, Ordering::SeqCst);
        let _lifecycle = self.lifecycle.lock().await;
        let _ = self.release().await;
    }

    /// 放掉写锁，桌面才能打开这个会话。
    async fn release(&self) -> anyhow::Result<()> {
        if !self.holding.swap(false, Ordering::SeqCst) {
            return Ok(());
        }
        self.sidecar()?.release(&self.thread_id).await
    }

    /// 这一轮结束后放锁：等一小会儿，期间又开了新的一轮或还有没回答的请求就不放。
    fn schedule_release(self: &Arc<Self>) {
        let session = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(RELEASE_AFTER_TURN).await;
            let _lifecycle = session.lifecycle.lock().await;
            let idle = {
                let state = session.state.lock().await;
                state.runtime != "active" && state.requests.is_empty()
            };
            if idle {
                let _ = session.release().await;
            }
        });
    }

    fn touch(&self, state: &mut SessionState) {
        state.revision += 1;
        let _ = self.changed.send(state.revision);
    }

    /// 处理一条侧车消息（通知或请求）。`Hub` 已按 `threadId` 分好。
    async fn apply(self: &Arc<Self>, message: &Value) {
        let Some(method) = message.get("method").and_then(Value::as_str) else { return };
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        let mut state = self.state.lock().await;
        // 服务端请求：挂到 requests 上等网页回答。
        if let Some(id) = message.get("id") {
            if !state.requests.iter().any(|request| request.get("id") == Some(id)) {
                state.requests.push(json!({ "method": method, "id": id, "params": params }));
                self.touch(&mut state);
            }
            return;
        }
        let mut turn_finished = false;
        match method {
            "thread/status/changed" => {
                let kind = params.pointer("/status/type").and_then(Value::as_str).unwrap_or("idle");
                if kind == "active" {
                    state.runtime = "active";
                }
            }
            "turn/started" => {
                let turn_id = params.pointer("/turn/id").and_then(Value::as_str).map(str::to_string);
                state.turn_id = turn_id.clone();
                state.runtime = "active";
                if !state.turns.iter().any(|turn| turn_id_of(turn) == turn_id.as_deref()) {
                    state.turns.push(json!({ "id": turn_id, "status": "inProgress", "items": [] }));
                }
            }
            "item/started" | "item/completed" => {
                let (Some(item), Some(turn_id)) = (params.get("item"), params.get("turnId").and_then(Value::as_str)) else { return };
                let Some(turn) = turn_mut(&mut state.turns, turn_id) else { return };
                upsert_item(turn, item.clone());
            }
            "item/agentMessage/delta" => {
                let (Some(item_id), Some(turn_id)) =
                    (params.get("itemId").and_then(Value::as_str), params.get("turnId").and_then(Value::as_str))
                else {
                    return;
                };
                let delta = params.get("delta").and_then(Value::as_str).unwrap_or_default();
                let Some(turn) = turn_mut(&mut state.turns, turn_id) else { return };
                let Some(items) = turn.get_mut("items").and_then(Value::as_array_mut) else { return };
                match items.iter_mut().find(|item| item.get("id").and_then(Value::as_str) == Some(item_id)) {
                    Some(item) => {
                        let text = format!("{}{delta}", item.get("text").and_then(Value::as_str).unwrap_or_default());
                        item["text"] = json!(text);
                    }
                    None => items.push(json!({ "type": "agentMessage", "id": item_id, "text": delta })),
                }
            }
            "turn/completed" => {
                if let Some(turn) = params.get("turn") {
                    let id = turn.get("id").and_then(Value::as_str);
                    match state.turns.iter_mut().find(|existing| turn_id_of(existing) == id) {
                        Some(existing) => {
                            // 完成通知里的 items 可能只有最后几条：保留流式期间收集的完整列表。
                            let collected = existing.get("items").cloned();
                            *existing = turn.clone();
                            let final_items = turn.get("items").and_then(Value::as_array).map(Vec::len).unwrap_or(0);
                            let collected_len = collected.as_ref().and_then(Value::as_array).map(Vec::len).unwrap_or(0);
                            if collected_len > final_items {
                                existing["items"] = collected.unwrap_or_default();
                            }
                        }
                        None => state.turns.push(turn.clone()),
                    }
                }
                state.runtime = "idle";
                state.turn_id = None;
                turn_finished = true;
            }
            "serverRequest/resolved" => {
                let id = params.get("requestId");
                state.requests.retain(|request| request.get("id") != id);
            }
            "thread/tokenUsage/updated" => {
                state.token_usage = super::follower::token_usage(params.get("tokenUsage"));
            }
            "thread/name/updated" => {
                if let Some(name) = params.get("name").and_then(Value::as_str) {
                    state.title = Some(name.to_string());
                }
            }
            _ => return,
        }
        self.touch(&mut state);
        drop(state);
        if turn_finished {
            self.schedule_release();
        }
    }
}

fn turn_id_of(turn: &Value) -> Option<&str> {
    turn.get("id").or_else(|| turn.get("turnId")).and_then(Value::as_str)
}

fn turn_mut<'a>(turns: &'a mut [Value], turn_id: &str) -> Option<&'a mut Value> {
    turns.iter_mut().rev().find(|turn| turn_id_of(turn) == Some(turn_id))
}

fn upsert_item(turn: &mut Value, item: Value) {
    if !turn.get("items").is_some_and(Value::is_array) {
        turn["items"] = json!([]);
    }
    let Some(items) = turn.get_mut("items").and_then(Value::as_array_mut) else { return };
    let id = item.get("id").and_then(Value::as_str).map(str::to_string);
    match items.iter_mut().find(|existing| id.is_some() && existing.get("id").and_then(Value::as_str) == id.as_deref()) {
        Some(existing) => *existing = item,
        None => items.push(item),
    }
}

/// 用 `thread/read` 的结果刷新标题、目录、模型和历史。
fn refresh_from_thread(state: &mut SessionState, thread: &Value) {
    state.title = thread
        .get("name")
        .filter(|name| !name.is_null())
        .or_else(|| thread.get("preview"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or(state.title.take());
    if let Some(cwd) = thread.get("cwd").and_then(Value::as_str) {
        state.cwd = cwd.to_string();
    }
    if let Some(model) = thread.get("model").and_then(Value::as_str) {
        state.model = Some(model.to_string());
    }
    if let Some(effort) = thread.get("reasoningEffort").and_then(Value::as_str) {
        state.effort = Some(effort.to_string());
    }
    if let Some(turns) = thread.get("turns").and_then(Value::as_array) {
        state.turns = turns.clone();
    }
}

/// 网页的回答（与桌面那条路同一套字段）转成 app-server 的回包。
pub fn app_server_result(method: &str, body: &Value) -> anyhow::Result<Value> {
    Ok(match method {
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
            json!({ "decision": body.get("decision").cloned().context("缺少 decision")? })
        }
        "item/tool/requestUserInput" => json!({ "answers": body.get("answers").cloned().context("缺少 answers")? }),
        "item/permissions/requestApproval" => body.get("response").cloned().context("缺少 response")?,
        method if method.starts_with("mcpServer/elicitation") => body.get("response").cloned().context("缺少 response")?,
        other => bail!("暂不支持远程处理这类请求：{other}"),
    })
}

/// 侧车只有一个事件流：按 `params.threadId` 分给对应的后台会话。没人认领的请求回绝，免得那一轮挂住。
pub struct Hub {
    sessions: Mutex<HashMap<String, Arc<HeadlessSession>>>,
}

impl Hub {
    pub fn new(sidecar: &Arc<Sidecar>) -> Arc<Self> {
        let hub = Arc::new(Self { sessions: Mutex::new(HashMap::new()) });
        let mut events = sidecar.events();
        let weak_hub = Arc::downgrade(&hub);
        let weak_sidecar = Arc::downgrade(sidecar);
        tokio::spawn(async move {
            loop {
                let message = match events.recv().await {
                    Ok(message) => message,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                };
                let (Some(hub), Some(sidecar)) = (weak_hub.upgrade(), weak_sidecar.upgrade()) else { return };
                hub.dispatch(&sidecar, message).await;
            }
        });
        hub
    }

    pub async fn insert(&self, session: Arc<HeadlessSession>) {
        self.sessions.lock().await.insert(session.thread_id.clone(), session);
    }

    pub async fn get(&self, thread_id: &str) -> Option<Arc<HeadlessSession>> {
        self.sessions.lock().await.get(thread_id).cloned()
    }

    pub async fn remove(&self, thread_id: &str) -> Option<Arc<HeadlessSession>> {
        self.sessions.lock().await.remove(thread_id)
    }

    pub async fn drain(&self) -> Vec<Arc<HeadlessSession>> {
        self.sessions.lock().await.drain().map(|(_, session)| session).collect()
    }

    async fn dispatch(&self, sidecar: &Arc<Sidecar>, message: Value) {
        let session = match message.pointer("/params/threadId").and_then(Value::as_str) {
            Some(thread_id) => self.get(thread_id).await,
            None => None,
        };
        match session {
            Some(session) => session.apply(&message).await,
            None => {
                if let Some(id) = message.get("id").filter(|_| message.get("method").is_some()) {
                    let _ = sidecar.reject_server_request(id, "no remote client is handling this thread").await;
                }
            }
        }
    }
}

/// 权限预设的 profile 名映射到 app-server 的 `sandboxPolicy`（与桌面预设一一对应）。
pub fn sandbox_policy_for(profile: &str) -> Value {
    match profile {
        ":read-only" => json!({ "type": "readOnly" }),
        ":danger-full-access" => json!({ "type": "dangerFullAccess" }),
        _ => json!({ "type": "workspaceWrite" }),
    }
}

/// 后台执行是否支持这种协作模式（`turn/start` 没有协作模式字段，不支持计划模式）。
pub fn headless_supports(collaboration_mode: &str) -> bool {
    collaboration_mode != "plan"
}
