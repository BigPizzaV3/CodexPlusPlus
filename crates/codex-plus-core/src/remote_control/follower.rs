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

    /// 开新一轮。总是带上会话当前的协作模式（计划 / 默认）：桌面自己发送时也是这样带的，
    /// 不带的话 owner 会按它界面上的模式发，远程切的计划模式就丢了。
    ///
    /// `attachments` 是桌面画文件卡片用的列表（`uploads::desktop_attachments`），桌面自己发消息时也放在
    /// `turnStart.context.attachments`；不带的话桌面上这条消息只显示原话，看不到附的文件。
    pub async fn start_turn(&self, input: Vec<Value>, attachments: Vec<Value>) -> anyhow::Result<Value> {
        let client_user_message_id = uuid::Uuid::new_v4().to_string();
        let mut request = json!({
            "threadId": self.thread_id,
            "input": input,
            "clientUserMessageId": client_user_message_id,
        });
        if let Some(mode) = self.state().as_ref().and_then(collaboration_mode_of) {
            request["collaborationMode"] = mode;
        }
        self.call("thread-follower-start-turn", json!({ "turnStart": { "request": request, "context": { "attachments": attachments } } }))
            .await
    }

    /// 改下一轮起生效的会话设置：模型、推理强度、权限模式、协作模式。
    /// 走 owner 的 `thread-follower-update-thread-settings`，桌面界面同步显示。
    pub async fn update_settings(&self, change: &SettingsChange) -> anyhow::Result<Value> {
        let state = self.state().context("还没有会话状态")?;
        let current = current_controls(&state);
        let model = change.model.clone().or_else(|| current.get("model").and_then(Value::as_str).map(str::to_string));
        let effort = match &change.effort {
            Some(effort) => effort.clone(),
            None => current.get("effort").and_then(Value::as_str).map(str::to_string),
        };
        let mut settings = serde_json::Map::new();
        if let Some(model) = &change.model {
            settings.insert("model".into(), json!(model));
        }
        if change.effort.is_some() {
            settings.insert("effort".into(), json!(effort));
        }
        if let Some(mode) = &change.permission_mode {
            let preset = permission_preset(mode).with_context(|| format!("未知的权限模式：{mode}"))?;
            settings.insert("permissions".into(), json!(preset.profile));
            settings.insert("approvalPolicy".into(), json!(preset.approval_policy));
            settings.insert("approvalsReviewer".into(), json!(preset.reviewer));
        }
        let collaboration = change
            .collaboration_mode
            .clone()
            .or_else(|| (change.model.is_some() || change.effort.is_some()).then(|| current.get("collaborationMode").and_then(Value::as_str).map(str::to_string)).flatten());
        if let Some(mode) = collaboration {
            if mode != "plan" && mode != "default" {
                bail!("未知的协作模式：{mode}");
            }
            settings.insert(
                "collaborationMode".into(),
                json!({ "mode": mode, "settings": { "model": model, "reasoning_effort": effort, "developer_instructions": null } }),
            );
        }
        if settings.is_empty() {
            bail!("没有要改的设置");
        }
        self.call("thread-follower-update-thread-settings", json!({ "threadSettings": Value::Object(settings) }))
            .await
    }

    /// 运行中插话。`restoreMessage` 不能为 null：owner 会读它的 cwd 与 context.commentAttachments。
    pub async fn steer(&self, text: &str, input: Vec<Value>, attachments: Vec<Value>) -> anyhow::Result<Value> {
        let cwd = self
            .state()
            .and_then(|state| state.get("cwd").and_then(Value::as_str).map(str::to_string))
            .unwrap_or_default();
        let id = uuid::Uuid::new_v4().to_string();
        self.call(
            "thread-follower-steer-turn",
            json!({
                "input": input,
                "clientUserMessageId": id,
                "restoreMessage": {
                    "id": id,
                    "text": text,
                    "cwd": cwd,
                    "context": {
                        "workspaceRoots": [cwd],
                        "commentAttachments": [],
                        "fileAttachments": attachments,
                        "collaborationMode": null,
                    },
                },
                "attachments": attachments,
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

/// 远程改会话设置的入参；没给的字段保持不变。`effort: Some(None)` 表示改回模型默认档位。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SettingsChange {
    pub model: Option<String>,
    pub effort: Option<Option<String>>,
    pub permission_mode: Option<String>,
    pub collaboration_mode: Option<String>,
}

impl SettingsChange {
    /// 当前设置里是否已经是改后的值。
    pub fn is_reflected_in(&self, controls: &Value) -> bool {
        let text = |key: &str| controls.get(key).and_then(Value::as_str);
        self.model.as_deref().is_none_or(|model| text("model") == Some(model))
            && self.effort.as_ref().is_none_or(|effort| text("effort") == effort.as_deref())
            && self.permission_mode.as_deref().is_none_or(|mode| text("permissionMode") == Some(mode))
            && self.collaboration_mode.as_deref().is_none_or(|mode| text("collaborationMode") == Some(mode))
    }

    pub fn from_json(body: &Value) -> anyhow::Result<Self> {
        let text = |key: &str| -> anyhow::Result<Option<String>> {
            match body.get(key) {
                None => Ok(None),
                Some(Value::String(value)) if !value.trim().is_empty() => Ok(Some(value.trim().to_string())),
                Some(other) => bail!("{key} 无效：{other}"),
            }
        };
        let effort = match body.get("effort") {
            None => None,
            Some(Value::Null) => Some(None),
            Some(Value::String(value)) if !value.trim().is_empty() => Some(Some(value.trim().to_string())),
            Some(other) => bail!("effort 无效：{other}"),
        };
        Ok(Self {
            model: text("model")?,
            effort,
            permission_mode: text("permissionMode")?,
            collaboration_mode: text("collaborationMode")?,
        })
    }
}

/// 桌面的四个权限预设（与 Codex 页面里的 agent-mode 表一致）。
pub struct PermissionPreset {
    pub id: &'static str,
    pub label: &'static str,
    pub detail: &'static str,
    pub profile: &'static str,
    pub approval_policy: &'static str,
    pub reviewer: &'static str,
}

pub const PERMISSION_PRESETS: &[PermissionPreset] = &[
    PermissionPreset { id: "read-only", label: "只读", detail: "只能读文件，改动和联网都要先问", profile: ":read-only", approval_policy: "on-request", reviewer: "user" },
    PermissionPreset { id: "auto", label: "默认", detail: "可以改工作区里的文件，越界操作先问你", profile: ":workspace", approval_policy: "on-request", reviewer: "user" },
    PermissionPreset { id: "guardian-approvals", label: "自动审批", detail: "越界操作交给审批助手判断，不再问你", profile: ":workspace", approval_policy: "on-request", reviewer: "guardian_subagent" },
    PermissionPreset { id: "full-access", label: "完全访问", detail: "不受沙箱限制、从不询问，命令直接执行", profile: ":danger-full-access", approval_policy: "never", reviewer: "user" },
];

pub fn permission_preset(id: &str) -> Option<&'static PermissionPreset> {
    PERMISSION_PRESETS.iter().find(|preset| preset.id == id)
}

/// 按会话设置反推当前是哪个权限预设；对不上任何预设时返回 `custom`。
pub fn permission_mode_of(settings: &Value) -> &'static str {
    let profile = settings
        .pointer("/activePermissionProfile/id")
        .or_else(|| settings.get("permissions"))
        .and_then(Value::as_str);
    let sandbox = settings.pointer("/sandboxPolicy/type").and_then(Value::as_str);
    let approval = settings.get("approvalPolicy").and_then(Value::as_str);
    let reviewer = settings.get("approvalsReviewer").and_then(Value::as_str).unwrap_or("user");
    let guardian = matches!(reviewer, "guardian_subagent" | "auto_review");
    match (profile, sandbox, approval) {
        (Some(":danger-full-access"), _, Some("never")) | (_, Some("dangerFullAccess"), Some("never")) => "full-access",
        (Some(":read-only"), _, Some("on-request")) | (None, Some("readOnly"), Some("on-request")) => "read-only",
        (Some(":workspace"), _, Some("on-request")) | (None, Some("workspaceWrite"), Some("on-request")) => {
            if guardian { "guardian-approvals" } else { "auto" }
        }
        _ => "custom",
    }
}

/// 会话当前的模型、推理强度、权限模式、协作模式。
pub fn current_controls(state: &Value) -> Value {
    let settings = state.get("latestThreadSettings").cloned().unwrap_or(Value::Null);
    let model = settings
        .get("model")
        .and_then(Value::as_str)
        .filter(|model| !model.is_empty())
        .or_else(|| state.get("latestModel").and_then(Value::as_str).filter(|model| !model.is_empty()));
    let effort = settings
        .get("effort")
        .and_then(Value::as_str)
        .or_else(|| state.get("latestReasoningEffort").and_then(Value::as_str));
    json!({
        "model": model,
        "effort": effort,
        "permissionMode": permission_mode_of(&settings),
        "collaborationMode": state.pointer("/latestCollaborationMode/mode").and_then(Value::as_str).unwrap_or("default"),
    })
}

/// 开新一轮时带上的协作模式：按会话当前模式、模型与档位拼，说明文本交给服务端按模型模板填。
fn collaboration_mode_of(state: &Value) -> Option<Value> {
    let controls = current_controls(state);
    let mode = controls.get("collaborationMode").and_then(Value::as_str)?;
    Some(json!({
        "mode": mode,
        "settings": { "model": controls.get("model"), "reasoning_effort": controls.get("effort"), "developer_instructions": null },
    }))
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
                "error": turn_error(turn.get("error")),
                "startedAtMs": turn.get("turnStartedAtMs"),
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
        "controls": current_controls(state),
        "truncatedTurns": skip,
        "turns": turns,
        "tokenUsage": token_usage(state.get("latestTokenUsageInfo")),
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
            "error": turn_error(turn.get("error")),
            "startedAtMs": turn.get("startedAt").and_then(Value::as_i64).map(|seconds| seconds * 1000),
            "items": turn.get("items").and_then(Value::as_array).map(|items| items.iter().map(view_item).collect::<Vec<_>>()).unwrap_or_default(),
        })).collect::<Vec<_>>(),
        "requests": [],
    })
}

/// 一轮失败的原因：桌面与 app-server 的 turn 都带 `error: { message, additionalDetails, codexErrorInfo }`，
/// 也兼容直接是字符串。网页只拿 `message` 和 `details` 两段文字。
pub fn turn_error(error: Option<&Value>) -> Value {
    let Some(error) = error.filter(|error| !error.is_null()) else { return Value::Null };
    if let Some(text) = error.as_str() {
        return json!({ "message": text });
    }
    let text = |key: &str| error.get(key).and_then(Value::as_str).map(str::trim).filter(|text| !text.is_empty());
    let message = text("message").or_else(|| text("reason"));
    let details = text("additionalDetails").filter(|details| Some(*details) != message);
    let kind = error.get("codexErrorInfo").and_then(|info| info.as_str().map(str::to_string).or_else(|| info.as_object()?.keys().next().cloned()));
    if message.is_none() && details.is_none() && kind.is_none() {
        return json!({ "message": error.to_string() });
    }
    json!({ "message": message, "details": details, "kind": kind })
}

/// 上下文用量：最近一次请求占了多少 token / 模型上下文窗口（桌面与 app-server 同一结构）。
pub fn token_usage(info: Option<&Value>) -> Value {
    let Some(info) = info else { return Value::Null };
    let used = info.pointer("/last/totalTokens").and_then(Value::as_u64);
    let window = info.get("modelContextWindow").and_then(Value::as_u64);
    match (used, window) {
        (Some(used), Some(window)) if window > 0 => json!({ "used": used, "window": window }),
        _ => Value::Null,
    }
}

/// 条目原样透传，只给用户消息补一个 `text`，并截断过长的命令输出。
fn file_name(path: Option<&Value>) -> Value {
    path.and_then(Value::as_str)
        .and_then(|path| std::path::Path::new(path).file_name())
        .map(|name| json!(name.to_string_lossy()))
        .unwrap_or(Value::Null)
}

pub(super) fn view_item(item: &Value) -> Value {
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
    if matches!(kind.as_str(), "userMessage" | "steeringUserMessage") && item.get("attachments").is_none() {
        // 图片 / 文件附件单独列出来，网页显示缩略图或文件卡片（`path` 在上传目录里时网页能取到原图）。
        let source = item.get("content").or_else(|| item.get("input"));
        let attachments: Vec<Value> = source
            .and_then(Value::as_array)
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(|part| match part.get("type").and_then(Value::as_str)? {
                        "localImage" => Some(json!({ "kind": "image", "path": part.get("path"), "name": file_name(part.get("path")) })),
                        "image" => Some(json!({ "kind": "image", "url": part.get("url").or_else(|| part.get("imageUrl")) })),
                        "mention" => Some(json!({ "kind": "file", "path": part.get("path"), "name": part.get("name") })),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut attachments = attachments;
        // 文件附件写在文字里（见 uploads::message_text）：拆出来，文字只留用户原话。
        if let Some((files, request)) = item.get("text").and_then(Value::as_str).and_then(super::uploads::split_message_text) {
            attachments.splice(0..0, files);
            item["text"] = json!(request);
        }
        if !attachments.is_empty() {
            item["attachments"] = json!(attachments);
        }
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
