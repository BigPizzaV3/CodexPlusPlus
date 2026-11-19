//! 微信专注模式（#1809）：本人在微信里钉住一个 Codex 会话，消息原样发给它，它的回复原样转回微信，不经过加加。
//!
//! 会话就是桌面那套会话：列、读、发、新开都走远程控制的 [`RemoteService`]（会话在桌面开着就跟随桌面，
//! 没开就侧车后台执行），所以手机上专注过的会话回到电脑侧栏里能接着用。对远程控制的调用隔着 [`SessionPort`]，测试里换成假的。
//!
//! - `/会话` 列最近的会话（带编号，按联系人记住 30 分钟）；`/进入 N`、`/切换 N` 钉住；`/新会话 [项目] 任务` 新开并钉住；`/退出` 回到原来的模式。
//! - 专注中本人的文字（和附件）直接发给会话：正在跑就是插话，和远程控制一致。
//! - 后台转发任务定时看一眼钉住的会话：每一轮结束把最后一条 Codex 回复（附一行执行摘要）发回微信；
//!   只转进入专注之后结束的轮次（按轮次 ID 记着，重启也不重发）。审批请求以文字卡片发出，不带编号，回个数字就能答。
//! - 专注状态按联系人存在 `<账号>-jiajia.json`；会话没了就自动退出并说一次。
//! - 只有扫码绑定的本人能用；远程控制没开时这些命令只回一句提示。

use std::path::Path;

use super::*;
use crate::connect::inbox::Inbox;
use crate::connect::media;
use crate::remote_control::jiajia::events::{Phase, clip, project_name};
use crate::remote_control::jiajia::tools::relative_time;
use crate::remote_control::uploads::{Attachment, AttachmentKind};

/// `/会话` 最多列几条。
pub const LIST_LIMIT: usize = 10;
/// `/会话` 列出的编号多久内有效。
pub const LIST_TTL_MS: i64 = 30 * 60 * 1000;
/// 钉住的会话在忙（或刚发过消息）时多久看一次；空闲时多久看一次。
const POLL_BUSY_MS: i64 = 1500;
const POLL_IDLE_MS: i64 = 6000;
/// 本人发消息后这段时间里按忙的节奏看（会话可能还没开始跑）。
const BUSY_AFTER_SEND_MS: i64 = 3 * 60 * 1000;
/// 记多少个已经处理过的轮次 ID。
const SEEN_LIMIT: usize = 64;
/// 进入时附上的上一条回复最多多少字。
const ENTRY_REPLY_LIMIT: usize = 300;
const REMOTE_OFF: &str = "请先在 Codex++ 打开远程控制（专注模式用的是远程控制里的会话）。";

// ---------- 数据 ----------

/// `/会话` 里的一行（也是 [`SessionPort::list_threads`] 的结果）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSummary {
    pub id: String,
    pub title: String,
    pub cwd: String,
    /// 给人看的状态：运行中 / 等你处理 / 上一轮出错 / 空闲。
    pub state: String,
    /// 最近更新（秒）。
    pub updated_at: i64,
}

/// 联系人钉住的会话。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FocusState {
    pub thread_id: String,
    pub title: String,
    pub cwd: String,
    /// 进入的时间。
    pub since: i64,
    /// 已经结束、不用再转的轮次（进入时已有的 + 转过的）。
    pub seen_turns: Vec<String>,
    /// 已经发过卡片 / 提醒的请求（请求 id 的 JSON 文本），请求处理掉就移走。
    pub seen_requests: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListedThread {
    pub id: String,
    pub title: String,
}

/// 联系人最近一次 `/会话` 列出的编号。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SessionList {
    pub at: i64,
    pub threads: Vec<ListedThread>,
}

// ---------- 会话操作的接口 ----------

/// 专注模式要用到的会话操作。真实实现是 [`LiveSessions`]（远程控制的 [`RemoteService`]），测试里换成假的。
#[async_trait]
pub trait SessionPort: Send + Sync {
    /// 远程控制开着没有。
    fn available(&self) -> bool;
    /// 最近的会话（最近更新的在前）。
    async fn list_threads(&self) -> anyhow::Result<Vec<ThreadSummary>>;
    /// 会话视图（结构同远程控制网页用的那份）；会话已经不存在时 `Ok(None)`。
    async fn thread_view(&self, thread_id: &str) -> anyhow::Result<Option<Value>>;
    /// 以本人身份发消息（可带附件）：正在跑就插话。返回 true 表示是插话。
    async fn send(&self, thread_id: &str, text: &str, attachments: &[Attachment]) -> anyhow::Result<bool>;
    /// 在目录里新建会话，返回会话 ID。
    async fn create_thread(&self, cwd: &str, title: &str) -> anyhow::Result<String>;
}

/// 同进程里正在跑的远程控制服务。
pub struct LiveSessions;

/// 远程控制报「会话不存在」的几种说法（侧车 `thread/read` 读不到 rollout 等）。
fn is_missing_thread(error: &str) -> bool {
    let error = error.to_lowercase();
    ["not found", "no rollout", "does not exist", "unknown thread", "找不到这个会话"].iter().any(|needle| error.contains(needle))
}

#[async_trait]
impl SessionPort for LiveSessions {
    fn available(&self) -> bool {
        crate::remote_control::hosted_service().is_some()
    }

    async fn list_threads(&self) -> anyhow::Result<Vec<ThreadSummary>> {
        LiveJiajia::on_service(|service| async move {
            let list = service.list_threads(None).await?;
            let jiajia = service.jiajia();
            let mut threads = Vec::new();
            for thread in list.get("threads").and_then(Value::as_array).into_iter().flatten() {
                let Some(id) = thread.get("id").and_then(Value::as_str) else { continue };
                // 状态和加加的 list_threads 一致：加加看过的用它记的阶段，否则看列表里的运行状态。
                let listed = thread.get("status").and_then(Value::as_str).unwrap_or_default();
                let state = match jiajia.known_phase(id).await {
                    Some((Phase::Waiting, _)) => "等你处理",
                    Some((Phase::Running, _)) => "运行中",
                    Some((Phase::Idle, Some(end))) if end == "failed" => "上一轮出错",
                    _ if listed == "active" => "运行中",
                    _ => "空闲",
                };
                threads.push(ThreadSummary {
                    id: id.to_string(),
                    title: thread.get("title").and_then(Value::as_str).unwrap_or_default().to_string(),
                    cwd: thread.get("cwd").and_then(Value::as_str).unwrap_or_default().to_string(),
                    state: state.to_string(),
                    updated_at: thread.get("updatedAt").and_then(Value::as_i64).unwrap_or(0),
                });
            }
            Ok(threads)
        })
        .await
    }

    async fn thread_view(&self, thread_id: &str) -> anyhow::Result<Option<Value>> {
        let thread_id = thread_id.to_string();
        LiveJiajia::on_service(move |service| async move {
            match service.thread_view(&thread_id).await {
                // 侧车读到空会话时视图里没有 ID。
                Ok(view) if view.get("threadId").is_none_or(Value::is_null) => Ok(None),
                Ok(view) => Ok(Some(view)),
                Err(error) if is_missing_thread(&format!("{error:#}")) => Ok(None),
                Err(error) => Err(error),
            }
        })
        .await
    }

    async fn send(&self, thread_id: &str, text: &str, attachments: &[Attachment]) -> anyhow::Result<bool> {
        let (thread_id, text, attachments) = (thread_id.to_string(), text.to_string(), attachments.to_vec());
        LiveJiajia::on_service(move |service| async move {
            let result = service.send_with(&thread_id, &text, &attachments).await?;
            Ok(result.get("kind").and_then(Value::as_str) == Some("steer"))
        })
        .await
    }

    async fn create_thread(&self, cwd: &str, title: &str) -> anyhow::Result<String> {
        let (cwd, title) = (cwd.to_string(), title.to_string());
        LiveJiajia::on_service(move |service| async move { service.create_thread(&cwd, &title).await }).await
    }
}

// ---------- 纯函数 ----------

fn phase_label(phase: Phase) -> &'static str {
    match phase {
        Phase::Waiting => "等你处理",
        Phase::Running => "运行中",
        Phase::Idle => "空闲",
    }
}

fn display_title(title: &str) -> &str {
    if title.trim().is_empty() { "未命名会话" } else { title }
}

/// `/会话` 的一行：「1. 重构登录 · CodexPlusPlus · 运行中 · 3 分钟前」。
pub fn session_line(no: usize, thread: &ThreadSummary, now_secs: i64, current: bool) -> String {
    let mut parts = vec![clip(display_title(&thread.title), 30)];
    let project = project_name(&thread.cwd);
    if !project.is_empty() {
        parts.push(project);
    }
    if !thread.state.is_empty() {
        parts.push(thread.state.clone());
    }
    if thread.updated_at > 0 {
        parts.push(relative_time(thread.updated_at, now_secs));
    }
    format!("{no}. {}{}", parts.join(" · "), if current { "（当前专注）" } else { "" })
}

fn is_turn_end(status: &str) -> bool {
    matches!(status, "completed" | "failed" | "interrupted")
}

fn turn_id(turn: &Value) -> Option<String> {
    turn.get("turnId").or_else(|| turn.get("id")).and_then(Value::as_str).map(str::to_string)
}

/// 视图里已经结束的轮次 ID。
fn ended_turn_ids(view: &Value) -> Vec<String> {
    view.get("turns")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|turn| turn.get("status").and_then(Value::as_str).is_some_and(is_turn_end))
        .filter_map(turn_id)
        .collect()
}

/// 一轮里最后一条 Codex 回复（过程旁白 commentary 不算，除非只有旁白）。
fn final_reply(turn: &Value) -> Option<String> {
    let items = turn.get("items").and_then(Value::as_array)?;
    let replies: Vec<&Value> = items.iter().filter(|item| item.get("type").and_then(Value::as_str) == Some("agentMessage")).collect();
    let text = |item: &&Value| item.get("text").and_then(Value::as_str).map(str::trim).filter(|text| !text.is_empty()).map(str::to_string);
    replies
        .iter()
        .rev()
        .filter(|item| item.get("phase").and_then(Value::as_str) != Some("commentary"))
        .find_map(text)
        .or_else(|| replies.iter().rev().find_map(text))
}

/// 一轮结束后转回微信的文字：最后一条回复 + 一行执行摘要；出错 / 被停止时说一句。什么都没有时 None。
pub fn turn_report(turn: &Value) -> Option<String> {
    let items = turn.get("items").and_then(Value::as_array).cloned().unwrap_or_default();
    let kind = |item: &Value| item.get("type").and_then(Value::as_str).unwrap_or_default().to_string();
    let commands = items.iter().filter(|item| kind(item) == "commandExecution").count();
    let mut files: Vec<String> = Vec::new();
    for item in items.iter().filter(|item| kind(item) == "fileChange") {
        if matches!(item.get("status").and_then(Value::as_str), Some("declined" | "failed")) {
            continue;
        }
        for path in item.get("changes").and_then(Value::as_array).into_iter().flatten().filter_map(|change| change.get("path").and_then(Value::as_str)) {
            if !files.iter().any(|known| known == path) {
                files.push(path.to_string());
            }
        }
    }
    let mut parts: Vec<String> = Vec::new();
    if let Some(reply) = final_reply(turn) {
        parts.push(reply);
    }
    let mut did = Vec::new();
    if commands > 0 {
        did.push(format!("执行了 {commands} 条命令"));
    }
    if !files.is_empty() {
        did.push(format!("改了 {} 个文件", files.len()));
    }
    if !did.is_empty() {
        parts.push(format!("（本轮{}）", did.join("，")));
    }
    let error = turn.pointer("/error/message").and_then(Value::as_str);
    match (turn.get("status").and_then(Value::as_str).unwrap_or_default(), error) {
        (_, Some(error)) => parts.push(format!("（这一轮出错了：{}）", clip(error, 200))),
        ("failed", None) => parts.push("（这一轮出错了）".into()),
        ("interrupted", None) => parts.push("（这一轮被停止了）".into()),
        _ => {}
    }
    (!parts.is_empty()).then(|| parts.join("\n\n"))
}

/// 会话最近一条 Codex 回复（进入时给个上下文）。
fn last_reply(view: &Value) -> Option<String> {
    view.get("turns").and_then(Value::as_array)?.iter().rev().find_map(final_reply)
}

/// `/新会话` 的第一个词是不是项目：是绝对路径目录，或和最近会话里某个项目目录名一样（不分大小写）。
/// 返回 (项目目录, 剩下的任务)；任务可能为空（只写了项目）。
pub fn pick_project(threads: &[ThreadSummary], text: &str) -> Option<(String, String)> {
    let text = text.trim();
    let (first, task) = text.split_once(char::is_whitespace).unwrap_or((text, ""));
    let task = task.trim().to_string();
    let path = Path::new(first);
    if path.is_absolute() && path.is_dir() {
        return Some((first.to_string(), task));
    }
    let wanted = first.to_lowercase();
    threads.iter().map(|thread| &thread.cwd).find(|cwd| !cwd.is_empty() && project_name(cwd).to_lowercase() == wanted).map(|cwd| (cwd.clone(), task))
}

fn remember(seen: &mut Vec<String>, id: String) {
    if !seen.contains(&id) {
        seen.push(id);
    }
    let extra = seen.len().saturating_sub(SEEN_LIMIT);
    seen.drain(..extra);
}

/// 专注会话的审批登记成卡片，返回要发的文字。微信里答不了的只发提醒、不登记。
fn register_focus_card(state: &mut BridgeState, thread_id: &str, title: &str, request: &Value) -> String {
    let mut card = CardRef {
        no: next_card_no(&state.cards),
        message_id: 0,
        thread_id: thread_id.to_string(),
        title: title.to_string(),
        request_id: request.get("id").cloned().unwrap_or(Value::Null),
        options: card_options(request),
        focus: true,
    };
    if card.options.is_empty() || card.request_id.is_null() {
        return card_text(&card, request, 0);
    }
    // 同一个会话还有一张没答的（少见）：这张带上编号，免得分不清。
    if state.cards.iter().any(|existing| existing.thread_id == thread_id) {
        card.focus = false;
    }
    let text = card_text(&card, request, if card.focus { 0 } else { state.cards.len() });
    state.cards.push(card);
    text
}

// ---------- 桥上的专注模式 ----------

impl JiajiaBridge {
    pub(super) fn focus_of(&self, peer: &str) -> Option<FocusState> {
        self.inner().state.focus.get(peer).cloned()
    }

    /// 本人现在钉住的会话。
    pub(super) fn focused_thread(&self, inner: &Inner) -> Option<String> {
        let owner = self.owner.as_deref()?;
        inner.state.focus.get(owner).map(|focus| focus.thread_id.clone())
    }

    /// 退出专注后回到哪儿（加加 / 直连）。
    fn back_label(&self, peer: &str) -> &'static str {
        let settings = self.settings();
        let inner = self.inner();
        self.mode_of(&inner, peer, &settings).label()
    }

    /// 只在钉住的还是这个会话时退出（期间可能已经切走了）。
    fn drop_focus(&self, peer: &str, thread_id: &str) -> bool {
        let mut inner = self.inner();
        if inner.state.focus.get(peer).is_none_or(|focus| focus.thread_id != thread_id) {
            return false;
        }
        inner.state.focus.remove(peer);
        self.save(&inner.state);
        true
    }

    /// /会话 /进入 /切换 /新会话 /退出。
    pub(super) async fn run_focus_command(&self, peer: &str, command: Command) -> String {
        if !self.is_owner(peer) {
            return if self.owner.is_none() {
                "还没认出扫码绑定的本人，暂时不能用专注模式：请在 Codex++ 管理器里重新扫码登录微信后再试。".into()
            } else {
                "专注模式只对扫码绑定的本人开放；你的消息会继续直接交给 Codex。".into()
            };
        }
        if command == Command::Exit {
            return self.exit_focus(peer);
        }
        if !self.sessions.available() {
            return REMOTE_OFF.into();
        }
        match command {
            Command::Sessions => self.list_sessions(peer).await,
            Command::Focus(target) => self.focus_target(peer, &target, false).await,
            Command::Switch(target) => self.focus_target(peer, &target, true).await,
            Command::NewSession(text) => self.new_session(peer, &text).await,
            _ => String::new(),
        }
    }

    async fn list_sessions(&self, peer: &str) -> String {
        let threads = match self.sessions.list_threads().await {
            Ok(threads) => threads,
            Err(error) => return format!("没读到会话列表：{}", clip(&format!("{error:#}"), 160)),
        };
        let threads: Vec<ThreadSummary> = threads.into_iter().take(LIST_LIMIT).collect();
        if threads.is_empty() {
            return "还没有会话。发「/新会话 项目 任务」新开一个。".into();
        }
        let now = now_ms();
        let current = self.focus_of(peer).map(|focus| focus.thread_id);
        let lines: Vec<String> =
            threads.iter().enumerate().map(|(index, thread)| session_line(index + 1, thread, now / 1000, current.as_deref() == Some(thread.id.as_str()))).collect();
        {
            let mut inner = self.inner();
            let listed = threads.iter().map(|thread| ListedThread { id: thread.id.clone(), title: thread.title.clone() }).collect();
            inner.state.session_lists.insert(peer.to_string(), SessionList { at: now, threads: listed });
            self.save(&inner.state);
        }
        format!("{}\n\n回复 /进入 编号", lines.join("\n"))
    }

    async fn focus_target(&self, peer: &str, target: &str, switching: bool) -> String {
        let target = target.trim().trim_start_matches(['#', '＃']);
        let command = if switching { "/切换" } else { "/进入" };
        if target.is_empty() {
            return format!("回复「{command} 编号」（先发 /会话 看编号），也可以给会话 ID 前 8 位。");
        }
        let (thread_id, title) = if target.len() <= 3 && target.chars().all(|c| c.is_ascii_digit()) {
            let list = self.inner().state.session_lists.get(peer).cloned();
            let Some(list) = list else { return format!("先发 /会话 看看有哪些会话，再回复「{command} 编号」。") };
            if now_ms() - list.at > LIST_TTL_MS {
                return "会话列表已经过期了，重新发 /会话 再选。".into();
            }
            let no: usize = target.parse().unwrap_or(0);
            match no.checked_sub(1).and_then(|index| list.threads.get(index)) {
                Some(thread) => (thread.id.clone(), thread.title.clone()),
                None => return format!("列表里没有第 {no} 个（共 {} 个），重新发 /会话 看看。", list.threads.len()),
            }
        } else {
            match self.resolve_target(target).await {
                Ok(found) => found,
                Err(message) => return message,
            }
        };
        self.enter_focus(peer, &thread_id, &title, switching).await
    }

    /// 会话 ID（完整或前几位）→ (完整 ID, 标题)。
    async fn resolve_target(&self, raw: &str) -> Result<(String, String), String> {
        let threads = self.sessions.list_threads().await;
        if let Ok(uuid) = uuid::Uuid::parse_str(raw) {
            let id = uuid.to_string();
            let title = threads.ok().and_then(|threads| threads.into_iter().find(|thread| thread.id == id)).map(|thread| thread.title).unwrap_or_default();
            return Ok((id, title));
        }
        let prefix = raw.to_lowercase();
        if prefix.chars().count() < 4 || !prefix.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
            return Err(format!("「{}」不是编号也不是会话 ID：先发 /会话，再回复「/进入 编号」。", clip(raw, 40)));
        }
        let threads = threads.map_err(|error| format!("没读到会话列表：{}", clip(&format!("{error:#}"), 160)))?;
        let mut matches: Vec<ThreadSummary> = threads.into_iter().filter(|thread| thread.id.starts_with(&prefix)).collect();
        match matches.len() {
            1 => {
                let thread = matches.remove(0);
                Ok((thread.id, thread.title))
            }
            0 => Err(format!("找不到会话 {raw}，发 /会话 看看。")),
            _ => Err(format!("{raw} 对上了好几个会话，多给几位。")),
        }
    }

    /// 钉住一个会话：记下进入时已经结束的轮次（之后只转新的），回一句并附上它最近一条回复。
    pub(super) async fn enter_focus(&self, peer: &str, thread_id: &str, title_hint: &str, switching: bool) -> String {
        let view = match self.sessions.thread_view(thread_id).await {
            Ok(Some(view)) => view,
            Ok(None) => return "这个会话已经不在了，重新发 /会话 看看。".into(),
            Err(error) => return format!("没打开这个会话：{}", clip(&format!("{error:#}"), 160)),
        };
        let snapshot = events::snapshot(&view);
        let title = [snapshot.title.as_str(), title_hint].into_iter().find(|title| !title.trim().is_empty()).unwrap_or("未命名会话").to_string();
        let focus = FocusState {
            thread_id: thread_id.to_string(),
            title: title.clone(),
            cwd: snapshot.cwd.clone(),
            since: now_ms(),
            seen_turns: ended_turn_ids(&view),
            seen_requests: Vec::new(),
        };
        let previous = {
            let mut inner = self.inner();
            let previous = inner.state.focus.insert(peer.to_string(), focus);
            if !snapshot.cwd.is_empty() {
                inner.state.last_projects.insert(peer.to_string(), snapshot.cwd.clone());
            }
            inner.focus_polled_at = 0;
            inner.focus_busy = snapshot.phase != Phase::Idle;
            self.save(&inner.state);
            previous
        };
        let back = self.back_label(peer);
        let verb = if switching && previous.is_some_and(|previous| previous.thread_id != thread_id) { "已切换到" } else { "已进入" };
        let mut text = format!("{verb}「{title}」，之后的消息直接发给它；/退出 回到{back}");
        match snapshot.phase {
            Phase::Running => text.push_str("\n它正在跑，这一轮结束会把回复转过来。"),
            Phase::Waiting => text.push_str("\n它在等你处理，卡片马上发过来。"),
            Phase::Idle => {}
        }
        if let Some(reply) = last_reply(&view) {
            text.push_str(&format!("\n\n上次 Codex 说：{}", clip(&reply, ENTRY_REPLY_LIMIT)));
        }
        text
    }

    fn exit_focus(&self, peer: &str) -> String {
        let removed = {
            let mut inner = self.inner();
            let removed = inner.state.focus.remove(peer);
            if removed.is_some() {
                self.save(&inner.state);
            }
            removed
        };
        let back = self.back_label(peer);
        match removed {
            Some(focus) => format!("已退出「{}」，回到{back}。", display_title(&focus.title)),
            None => format!("现在没有专注的会话，消息交给{back}。"),
        }
    }

    /// `/新会话 [项目] 任务`：第一个词能对上项目就当项目，否则整句都是任务、项目用上次的（再退回微信连接的工作目录）。
    async fn new_session(&self, peer: &str, text: &str) -> String {
        const USAGE: &str = "用法：/新会话 [项目] 任务，例如「/新会话 CodexPlusPlus 修一下登录页」。项目可以省略（用上次的）。";
        let text = text.trim();
        if text.is_empty() {
            return USAGE.into();
        }
        let threads = self.sessions.list_threads().await.unwrap_or_default();
        let (cwd, task) = match pick_project(&threads, text) {
            Some((_, task)) if task.is_empty() => return USAGE.into(),
            Some((cwd, task)) => (Some(cwd), task),
            None => (self.default_project(peer), text.to_string()),
        };
        let Some(cwd) = cwd else {
            return "不知道在哪个项目里开：在任务前写上项目名，如「/新会话 项目名 任务」（/会话 里能看到项目名）。".into();
        };
        let title = clip(&task, 20);
        let thread_id = match self.sessions.create_thread(&cwd, &title).await {
            Ok(thread_id) => thread_id,
            Err(error) => return format!("没建成会话：{}", clip(&format!("{error:#}"), 160)),
        };
        {
            let mut inner = self.inner();
            inner.state.focus.insert(
                peer.to_string(),
                FocusState { thread_id: thread_id.clone(), title: title.clone(), cwd: cwd.clone(), since: now_ms(), ..Default::default() },
            );
            inner.state.last_projects.insert(peer.to_string(), cwd.clone());
            inner.focus_polled_at = 0;
            inner.focus_busy = true;
            inner.focus_sent_at = now_ms();
            self.save(&inner.state);
        }
        let back = self.back_label(peer);
        let project = project_name(&cwd);
        match self.sessions.send(&thread_id, &task, &[]).await {
            Ok(_) => format!("已在 {project} 新开「{title}」并发出任务，之后的消息直接发给它；/退出 回到{back}"),
            Err(error) => format!(
                "已在 {project} 新开「{title}」并进入，但任务没发出去：{}。直接再发一次就行；/退出 回到{back}",
                clip(&format!("{error:#}"), 160)
            ),
        }
    }

    /// 上次专注 / 新开时用的项目，再退回微信连接配置的工作目录（都得是本机存在的目录）。
    fn default_project(&self, peer: &str) -> Option<String> {
        let last = self.inner().state.last_projects.get(peer).cloned();
        let configured = self.settings.work_dir().filter(|dir| !dir.is_empty()).unwrap_or_else(|| self.work_dir.clone());
        [last, Some(configured)].into_iter().flatten().find(|dir| !dir.is_empty() && Path::new(dir).is_dir())
    }

    /// `/状态` 里的一行。
    pub(super) async fn focus_status_line(&self, peer: &str) -> String {
        let Some(focus) = self.focus_of(peer) else { return "未专注".into() };
        let state = if !self.sessions.available() {
            "远程控制没开"
        } else {
            match self.sessions.thread_view(&focus.thread_id).await {
                Ok(Some(view)) => phase_label(events::snapshot(&view).phase),
                Ok(None) => "会话已经不在了",
                Err(_) => "读不到状态",
            }
        };
        format!("当前专注：{}（{state}）", display_title(&focus.title))
    }

    // ---------- 专注中收到的消息 ----------

    /// 本人的消息（和附件）直接发给钉住的会话。
    pub(super) async fn send_to_focus(&self, peer: &str, token: &str, message: &WeixinMessage, text: &str, focus: &FocusState) -> Inbound {
        let title = display_title(&focus.title).to_string();
        if !self.sessions.available() {
            let back = self.back_label(peer);
            self.reply(peer, token, &format!("这条没发给「{title}」：{REMOTE_OFF}\n发 /退出 回到{back}。")).await;
            return Inbound::Handled;
        }
        let (attachments, warnings, _inbox) = if message.has_attachments() { self.receive_attachments(message, &focus.cwd).await } else { (Vec::new(), Vec::new(), None) };
        let body = text.trim();
        if body.is_empty() && attachments.is_empty() {
            if message.has_attachments() {
                let reason = warnings.first().map(String::as_str).unwrap_or("没有可用附件");
                self.reply(peer, token, &format!("附件没收到，没发给「{title}」：{}", clip(reason, 160))).await;
            }
            return Inbound::Handled;
        }
        let body = if body.is_empty() { "请看附件。" } else { body };
        match self.sessions.send(&focus.thread_id, body, &attachments).await {
            Ok(_) => {
                {
                    let mut inner = self.inner();
                    inner.focus_sent_at = now_ms();
                    inner.focus_busy = true;
                    inner.focus_polled_at = 0;
                }
                if !warnings.is_empty() {
                    self.reply(peer, token, &format!("有 {} 个附件没收到：{}", warnings.len(), clip(&warnings[0], 160))).await;
                }
                self.start_typing(peer, token).await;
            }
            Err(error) => {
                if matches!(self.sessions.thread_view(&focus.thread_id).await, Ok(None)) && self.drop_focus(peer, &focus.thread_id) {
                    let back = self.back_label(peer);
                    self.reply(peer, token, &format!("「{title}」已经不在了，已退出专注，回到{back}。这条没发出去，再发一次就交给{back}。")).await;
                } else {
                    self.reply(peer, token, &format!("没发给「{title}」：{}", clip(&format!("{error:#}"), 160))).await;
                }
            }
        }
        Inbound::Handled
    }

    /// 微信附件下载到会话目录的 `.codex-plus/weixin/inbox/` 下（和直连一样），转成远程控制的附件。
    /// 收件目录的锁要拿到发送完，所以一起返回。
    async fn receive_attachments(&self, message: &WeixinMessage, cwd: &str) -> (Vec<Attachment>, Vec<String>, Option<Inbox>) {
        let failed = || (Vec::new(), vec!["无法准备微信附件收件目录".to_string()], None);
        let dir = if !cwd.is_empty() && Path::new(cwd).is_dir() {
            PathBuf::from(cwd)
        } else {
            let fallback = crate::paths::default_app_state_dir().join("weixin-text-workspace");
            if std::fs::create_dir_all(&fallback).is_err() {
                return failed();
            }
            fallback
        };
        let Ok(inbox) = Inbox::create(&dir) else { return failed() };
        let downloaded = self.client.download_attachments(message, &inbox).await;
        let attachments = downloaded
            .attachments
            .iter()
            .map(|attachment| Attachment {
                kind: match attachment.kind {
                    media::AttachmentKind::Image => AttachmentKind::Image,
                    media::AttachmentKind::File => AttachmentKind::File,
                },
                name: attachment.path.file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_default(),
                path: attachment.path.clone(),
            })
            .collect();
        (attachments, downloaded.warnings, Some(inbox))
    }

    // ---------- 后台：转发钉住会话的回复和审批 ----------

    /// 加加（`focus_thread`）要微信进入专注：切过去并告诉本人。
    pub async fn take_focus_request(&self) {
        let Some(request) = self.port.take_focus_request() else { return };
        let Some(owner) = self.owner.clone() else { return };
        if !self.sessions.available() {
            return;
        }
        let text = self.enter_focus(&owner, &request.thread_id, &request.title, true).await;
        self.deliver(Kind::Notice, text, None).await;
    }

    /// 看一眼本人钉住的会话：新结束的轮次把回复发回微信，新的审批发卡片，会话没了就退出。
    /// `force` 为 false 时按节奏来（忙的时候 1.5 秒一次，空闲 6 秒一次）。
    pub async fn poll_focus(&self, force: bool) {
        let Some(owner) = self.owner.clone() else { return };
        let focus = {
            let mut inner = self.inner();
            let Some(focus) = inner.state.focus.get(&owner).cloned() else { return };
            let now = now_ms();
            let busy = inner.focus_busy || now - inner.focus_sent_at < BUSY_AFTER_SEND_MS;
            if !force && now - inner.focus_polled_at < if busy { POLL_BUSY_MS } else { POLL_IDLE_MS } {
                return;
            }
            inner.focus_polled_at = now;
            focus
        };
        if !self.sessions.available() {
            return;
        }
        let view = match self.sessions.thread_view(&focus.thread_id).await {
            Ok(Some(view)) => view,
            Ok(None) => {
                if self.drop_focus(&owner, &focus.thread_id) {
                    let back = self.back_label(&owner);
                    let text = format!("专注的会话「{}」已经不在了，已退出专注，回到{back}。", display_title(&focus.title));
                    self.deliver(Kind::Notice, text, None).await;
                }
                return;
            }
            Err(_) => return,
        };
        let snapshot = events::snapshot(&view);
        let mut outgoing: Vec<(Kind, String)> = Vec::new();
        let mut settled: Vec<u64> = Vec::new();
        {
            let mut guard = self.inner();
            let inner = &mut *guard;
            // 看的这会儿本人退出或切走了：这次的结果不要了。
            let Some(current) = inner.state.focus.get_mut(&owner).filter(|current| current.thread_id == focus.thread_id) else { return };
            let mut dirty = false;
            if !snapshot.title.is_empty() && current.title != snapshot.title {
                current.title = snapshot.title.clone();
                dirty = true;
            }
            for turn in view.get("turns").and_then(Value::as_array).into_iter().flatten() {
                if !turn.get("status").and_then(Value::as_str).is_some_and(is_turn_end) {
                    continue;
                }
                let Some(id) = turn_id(turn) else { continue };
                if current.seen_turns.contains(&id) {
                    continue;
                }
                remember(&mut current.seen_turns, id);
                dirty = true;
                if let Some(report) = turn_report(turn) {
                    outgoing.push((Kind::Reply, report));
                }
            }
            let live: Vec<String> = snapshot.requests.iter().map(|(key, _)| key.clone()).collect();
            let before = current.seen_requests.len();
            current.seen_requests.retain(|key| live.contains(key));
            dirty |= current.seen_requests.len() != before;
            let fresh: Vec<(String, Value)> = snapshot.requests.iter().filter(|(key, _)| !current.seen_requests.contains(key)).cloned().collect();
            current.seen_requests.extend(fresh.iter().map(|(key, _)| key.clone()));
            dirty |= !fresh.is_empty();
            let title = current.title.clone();
            // 在别处（桌面、网页）处理掉的审批：微信里的卡片也拿掉。
            inner.state.cards.retain(|card| {
                let keep = card.thread_id != focus.thread_id || live.contains(&card.request_id.to_string());
                if !keep {
                    settled.push(card.message_id);
                }
                keep
            });
            dirty |= !settled.is_empty();
            for (key, request) in fresh {
                // 进入专注前加加那边已经贴过的卡片不再发。
                if inner.state.cards.iter().any(|card| card.thread_id == focus.thread_id && card.request_id.to_string() == key) {
                    continue;
                }
                outgoing.push((Kind::Card, register_focus_card(&mut inner.state, &focus.thread_id, &title, &request)));
            }
            inner.focus_busy = snapshot.phase != Phase::Idle;
            if dirty {
                self.save(&inner.state);
            }
        }
        for message_id in settled.into_iter().filter(|id| *id != 0) {
            self.port.resolve_card(message_id);
        }
        for (kind, text) in outgoing {
            if kind == Kind::Reply {
                self.stop_typing().await;
            }
            self.deliver(kind, text, None).await;
        }
    }
}
