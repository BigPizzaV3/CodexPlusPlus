//! 跟随一个会话：维护桌面推送的会话状态副本，并代理 follower 请求。
//!
//! owner（打开着这个会话的桌面窗口）广播 `thread-stream-state-changed`：
//!   `{ type: "snapshot", revision, conversationState }`
//!   `{ type: "patches", baseRevision, revision, patches }`（immer 补丁，见 [`super::patch`]）
//! baseRevision 对不上时向 owner 要一次完整快照（`thread-follower-load-complete-history`）。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, bail};
use serde_json::{Value, json};
use tokio::sync::{broadcast, watch};

use super::ipc::IpcConnection;
use super::patch::apply_patches;

const OWNER_TIMEOUT: Duration = Duration::from_secs(3);
const FOLLOWER_TIMEOUT: Duration = Duration::from_secs(20);
const SNAPSHOT_WAIT: Duration = Duration::from_secs(8);
/// 视图里最多带多少轮，避免长会话每次推送都很大。
const VIEW_TURN_LIMIT: usize = 40;

#[derive(Default)]
struct FollowState {
    state: Option<Value>,
    revision: Option<u64>,
    owner: Option<String>,
}

pub struct Follower {
    pub thread_id: String,
    conn: Arc<IpcConnection>,
    inner: std::sync::Mutex<FollowState>,
    changed: watch::Sender<u64>,
    alive: AtomicBool,
}

/// 查谁持有这个会话；没有 owner 时返回 None。
pub async fn discover_owner(conn: &IpcConnection, thread_id: &str) -> anyhow::Result<Option<String>> {
    let response = conn
        .request("thread-owner-discovery", json!({ "hostId": "local", "conversationId": thread_id }), None, OWNER_TIMEOUT)
        .await?;
    Ok(response.ok().then(|| response.handled_by().map(str::to_string)).flatten())
}

impl Follower {
    /// 找 owner、声明跟随、等第一份快照。没有 owner 时返回 `Ok(None)`。
    pub async fn start(conn: Arc<IpcConnection>, thread_id: &str) -> anyhow::Result<Option<Arc<Self>>> {
        let Some(owner) = discover_owner(&conn, thread_id).await? else {
            return Ok(None);
        };
        let (changed, _) = watch::channel(0);
        let follower = Arc::new(Self {
            thread_id: thread_id.to_string(),
            conn: conn.clone(),
            inner: std::sync::Mutex::new(FollowState { owner: Some(owner), ..Default::default() }),
            changed,
            alive: AtomicBool::new(true),
        });
        tokio::spawn(event_loop(Arc::downgrade(&follower), conn.subscribe()));
        conn.broadcast(
            "thread-stream-following-changed",
            json!({ "conversationId": thread_id, "hostId": "local", "following": true }),
        )
        .await?;
        let mut updates = follower.subscribe();
        follower.resync().await?;
        let deadline = tokio::time::Instant::now() + SNAPSHOT_WAIT;
        while follower.lock().state.is_none() {
            if tokio::time::timeout_at(deadline, updates.changed()).await.is_err() {
                bail!("8 秒内没有收到会话快照");
            }
        }
        Ok(Some(follower))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, FollowState> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst) && !self.conn.is_closed()
    }

    /// 状态每变一次，版本号加一。
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    pub fn owner(&self) -> Option<String> {
        self.lock().owner.clone()
    }

    pub fn state(&self) -> Option<Value> {
        self.lock().state.clone()
    }

    async fn resync(&self) -> anyhow::Result<()> {
        let owner = self.owner();
        self.conn
            .request(
                "thread-follower-load-complete-history",
                json!({ "conversationId": self.thread_id }),
                owner.as_deref(),
                Duration::from_secs(8),
            )
            .await?
            .into_result()
            .map(|_| ())
    }

    /// 处理一条广播；返回是否需要重新要快照。
    fn handle_change(&self, params: &Value, source: Option<&str>) -> bool {
        let Some(change) = params.get("change") else { return false };
        let mut inner = self.lock();
        match change.get("type").and_then(Value::as_str) {
            Some("snapshot") => {
                inner.state = change.get("conversationState").cloned();
                inner.revision = change.get("revision").and_then(Value::as_u64);
                if let Some(source) = source {
                    inner.owner = Some(source.to_string());
                }
            }
            Some("patches") => {
                let base = change.get("baseRevision").and_then(Value::as_u64);
                if inner.state.is_none() || base != inner.revision {
                    return true;
                }
                let patches = change.get("patches").and_then(Value::as_array).cloned().unwrap_or_default();
                let applied = inner.state.as_mut().map(|state| apply_patches(state, &patches));
                if !matches!(applied, Some(Ok(()))) {
                    inner.state = None;
                    inner.revision = None;
                    return true;
                }
                inner.revision = change.get("revision").and_then(Value::as_u64);
            }
            _ => return false,
        }
        drop(inner);
        self.changed.send_modify(|version| *version += 1);
        false
    }

    async fn call(&self, method: &str, mut params: Value) -> anyhow::Result<Value> {
        params["conversationId"] = json!(self.thread_id);
        let owner = self.owner().context("会话没有 owner")?;
        self.conn.request(method, params, Some(&owner), FOLLOWER_TIMEOUT).await?.into_result()
    }

    pub async fn start_turn(&self, text: &str) -> anyhow::Result<Value> {
        let client_user_message_id = uuid::Uuid::new_v4().to_string();
        self.call(
            "thread-follower-start-turn",
            json!({
                "turnStart": {
                    "request": {
                        "threadId": self.thread_id,
                        "input": [{ "type": "text", "text": text, "text_elements": [] }],
                        "clientUserMessageId": client_user_message_id,
                    },
                    "context": {},
                }
            }),
        )
        .await
    }

    /// 运行中插话。`restoreMessage` 不能为 null：owner 会读它的 cwd 与 context.commentAttachments。
    pub async fn steer(&self, text: &str) -> anyhow::Result<Value> {
        let cwd = self
            .state()
            .and_then(|state| state.get("cwd").and_then(Value::as_str).map(str::to_string))
            .unwrap_or_default();
        let id = uuid::Uuid::new_v4().to_string();
        self.call(
            "thread-follower-steer-turn",
            json!({
                "input": [{ "type": "text", "text": text, "text_elements": [] }],
                "clientUserMessageId": id,
                "restoreMessage": {
                    "id": id,
                    "text": text,
                    "cwd": cwd,
                    "context": {
                        "workspaceRoots": [cwd],
                        "commentAttachments": [],
                        "fileAttachments": [],
                        "collaborationMode": null,
                    },
                },
                "attachments": [],
                "serviceTier": null,
            }),
        )
        .await
    }

    pub async fn interrupt(&self) -> anyhow::Result<Value> {
        let turn_id = self.state().as_ref().and_then(active_turn_id);
        self.call("thread-follower-interrupt-turn", json!({ "mode": "user-stop", "expectedTurnId": turn_id }))
            .await
    }

    pub fn is_running(&self) -> bool {
        self.state()
            .as_ref()
            .and_then(|state| state.pointer("/threadRuntimeStatus/type").and_then(Value::as_str).map(str::to_string))
            .is_some_and(|status| status == "active")
    }

    /// 回复一条待处理请求（审批 / 提问 / 表单）。按请求的 method 选 follower 方法。
    pub async fn respond(&self, request_id: &Value, body: &Value) -> anyhow::Result<Value> {
        let state = self.state().context("还没有会话状态")?;
        let request = state
            .get("requests")
            .and_then(Value::as_array)
            .and_then(|requests| requests.iter().find(|request| request.get("id") == Some(request_id)))
            .cloned()
            .context("这条请求已处理或不存在")?;
        let method = request.get("method").and_then(Value::as_str).unwrap_or_default();
        let (follower_method, params) = response_for(method, request_id, body)?;
        self.call(follower_method, params).await
    }

    pub async fn stop(&self) {
        self.alive.store(false, Ordering::SeqCst);
        let _ = self
            .conn
            .broadcast(
                "thread-stream-following-changed",
                json!({ "conversationId": self.thread_id, "hostId": "local", "following": false }),
            )
            .await;
    }

    /// 给网页用的精简视图。
    pub fn view(&self) -> Value {
        let inner = self.lock();
        let Some(state) = inner.state.as_ref() else {
            return json!({ "threadId": self.thread_id, "live": true, "turns": [], "requests": [] });
        };
        build_view(&self.thread_id, state, inner.revision)
    }
}

/// 把请求的 method 映射到 follower 方法与参数。
pub fn response_for(method: &str, request_id: &Value, body: &Value) -> anyhow::Result<(&'static str, Value)> {
    let decision = || body.get("decision").cloned().context("缺少 decision");
    let response = || body.get("response").cloned().context("缺少 response");
    Ok(match method {
        "item/commandExecution/requestApproval" => (
            "thread-follower-command-approval-decision",
            json!({ "requestId": request_id, "decision": decision()? }),
        ),
        "item/fileChange/requestApproval" => (
            "thread-follower-file-approval-decision",
            json!({ "requestId": request_id, "decision": decision()? }),
        ),
        "item/tool/requestUserInput" => {
            let answers = body.get("answers").cloned().context("缺少 answers")?;
            ("thread-follower-submit-user-input", json!({ "requestId": request_id, "response": { "answers": answers } }))
        }
        "item/permissions/requestApproval" => (
            "thread-follower-permissions-request-approval-response",
            json!({ "requestId": request_id, "response": response()? }),
        ),
        method if method.starts_with("mcpServer/elicitation") => (
            "thread-follower-submit-mcp-server-elicitation-response",
            json!({ "requestId": request_id, "response": response()? }),
        ),
        other => bail!("暂不支持远程处理这类请求：{other}"),
    })
}

fn active_turn_id(state: &Value) -> Option<String> {
    turns_of(state)
        .into_iter()
        .rev()
        .find(|turn| turn.get("status").and_then(Value::as_str) == Some("inProgress"))
        .and_then(|turn| turn.get("turnId").and_then(Value::as_str).map(str::to_string))
}

/// 会话状态里按显示顺序排的轮次：历史在 `turnHistory.history`，`turns` 通常为空。
pub fn turns_of(state: &Value) -> Vec<&Value> {
    if let Some(history) = state.pointer("/turnHistory/history") {
        let entities = history.get("entitiesByKey");
        let islands = history.get("islands").and_then(Value::as_array);
        if let (Some(entities), Some(islands)) = (entities, islands) {
            return islands
                .iter()
                .filter_map(|island| island.get("entries").and_then(Value::as_array))
                .flatten()
                .filter_map(|entry| entry.get("value").and_then(Value::as_str))
                .filter_map(|key| entities.get(key))
                .collect();
        }
    }
    state.get("turns").and_then(Value::as_array).map(|turns| turns.iter().collect()).unwrap_or_default()
}

pub fn build_view(thread_id: &str, state: &Value, revision: Option<u64>) -> Value {
    let turns = turns_of(state);
    let skip = turns.len().saturating_sub(VIEW_TURN_LIMIT);
    let turns: Vec<Value> = turns
        .into_iter()
        .skip(skip)
        .map(|turn| {
            json!({
                "turnId": turn.get("turnId"),
                "status": turn.get("status"),
                "items": turn.get("items").and_then(Value::as_array).map(|items| items.iter().map(view_item).collect::<Vec<_>>()).unwrap_or_default(),
            })
        })
        .collect();
    json!({
        "threadId": thread_id,
        "live": true,
        "revision": revision,
        "title": state.get("title"),
        "cwd": state.get("cwd"),
        "model": state.get("latestModel"),
        "runtime": state.pointer("/threadRuntimeStatus/type"),
        "truncatedTurns": skip,
        "turns": turns,
        "requests": state.get("requests").cloned().unwrap_or_else(|| json!([])),
    })
}

/// 侧车 `thread/read` 的结果转成同样的视图（会话没在桌面打开时用）。
pub fn build_offline_view(thread: &Value) -> Value {
    let turns = thread.get("turns").and_then(Value::as_array).cloned().unwrap_or_default();
    let skip = turns.len().saturating_sub(VIEW_TURN_LIMIT);
    json!({
        "threadId": thread.get("id"),
        "live": false,
        "title": thread.get("name").filter(|name| !name.is_null()).or_else(|| thread.get("preview")),
        "cwd": thread.get("cwd"),
        "model": thread.get("model"),
        "runtime": "notLoaded",
        "truncatedTurns": skip,
        "turns": turns.iter().skip(skip).map(|turn| json!({
            "turnId": turn.get("id"),
            "status": turn.get("status"),
            "items": turn.get("items").and_then(Value::as_array).map(|items| items.iter().map(view_item).collect::<Vec<_>>()).unwrap_or_default(),
        })).collect::<Vec<_>>(),
        "requests": [],
    })
}

/// 条目原样透传，只给用户消息补一个 `text`，并截断过长的命令输出。
fn view_item(item: &Value) -> Value {
    let mut item = item.clone();
    let kind = item.get("type").and_then(Value::as_str).unwrap_or_default().to_string();
    if matches!(kind.as_str(), "userMessage" | "steeringUserMessage") && item.get("text").is_none() {
        let source = item.get("content").or_else(|| item.get("input"));
        let text = source
            .and_then(Value::as_array)
            .map(|parts| parts.iter().filter_map(|part| part.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join("\n"))
            .unwrap_or_default();
        item["text"] = json!(text);
    }
    if let Some(output) = item.get("aggregatedOutput").and_then(Value::as_str) {
        const LIMIT: usize = 8 * 1024;
        if output.len() > LIMIT {
            let mut cut = output.len() - LIMIT;
            while !output.is_char_boundary(cut) {
                cut += 1;
            }
            item["aggregatedOutput"] = json!(format!("…{}", &output[cut..]));
        }
    }
    item
}

async fn event_loop(follower: std::sync::Weak<Follower>, mut events: broadcast::Receiver<Value>) {
    loop {
        let message = match events.recv().await {
            Ok(message) => message,
            Err(broadcast::error::RecvError::Lagged(_)) => {
                let Some(follower) = follower.upgrade() else { return };
                let _ = follower.resync().await;
                continue;
            }
            Err(broadcast::error::RecvError::Closed) => break,
        };
        let Some(follower) = follower.upgrade() else { return };
        if !follower.alive.load(Ordering::SeqCst) {
            return;
        }
        if message.get("method").and_then(Value::as_str) != Some("thread-stream-state-changed") {
            continue;
        }
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        if params.get("conversationId").and_then(Value::as_str) != Some(follower.thread_id.as_str()) {
            continue;
        }
        let source = message.get("sourceClientId").and_then(Value::as_str);
        if follower.handle_change(&params, source) {
            let _ = follower.resync().await;
        }
    }
    if let Some(follower) = follower.upgrade() {
        follower.alive.store(false, Ordering::SeqCst);
        follower.changed.send_modify(|version| *version += 1);
    }
}
