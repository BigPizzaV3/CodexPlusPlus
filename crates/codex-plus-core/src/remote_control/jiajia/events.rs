//! 会话动静：从会话视图推出「运行中 / 等审批 / 空闲」和最近一轮的结果，对比前后两次得出事件；
//! 事件攒一会儿、按紧急程度排好，再一次性交给加加（每唤醒一次就是一轮模型调用）。
//!
//! 视图是远程控制已有的那份（桌面跟随 / 后台执行 / 磁盘历史三种来源同一结构，见 `follower::build_view`）。

use serde_json::Value;

/// 等审批的事件攒多久再投递（要人点，紧一些）。
pub const GATHER_WAITING_MS: i64 = 15_000;
pub const GATHER_FAILED_MS: i64 = 30_000;
pub const GATHER_DONE_MS: i64 = 60_000;
/// 太久以前跑完的不再报。
pub const STALE_DONE_MS: i64 = 6 * 3600 * 1000;
/// 一次最多列几条。
pub const BATCH_LIMIT: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Running,
    Waiting,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TurnEnd {
    pub turn_id: String,
    /// completed / failed / interrupted
    pub status: String,
    pub error: Option<String>,
    /// 这一轮最后一条回复（截断）。
    pub last_reply: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub phase: Phase,
    /// 待处理请求：(请求 id 的 JSON 文本, 请求本身)。
    pub requests: Vec<(String, Value)>,
    pub last_end: Option<TurnEnd>,
    pub title: String,
    pub cwd: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    Waiting,
    Failed,
    Done,
}

impl EventKind {
    pub fn id(self) -> &'static str {
        match self {
            EventKind::Waiting => "waiting",
            EventKind::Failed => "failed",
            EventKind::Done => "done",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            EventKind::Waiting => "等你处理",
            EventKind::Failed => "出错了",
            EventKind::Done => "跑完了",
        }
    }

    fn rank(self) -> u8 {
        match self {
            EventKind::Waiting => 0,
            EventKind::Failed => 1,
            EventKind::Done => 2,
        }
    }

    fn gather_ms(self) -> i64 {
        match self {
            EventKind::Waiting => GATHER_WAITING_MS,
            EventKind::Failed => GATHER_FAILED_MS,
            EventKind::Done => GATHER_DONE_MS,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub kind: EventKind,
    pub thread_id: String,
    pub title: String,
    pub cwd: String,
    pub detail: String,
    /// 等审批时带上请求（发审批卡片用）。
    pub request: Option<Value>,
    /// 跑完 / 出错的是哪一轮（加加已经读过这一轮就不再交给它）。
    pub turn_id: Option<String>,
}

/// 从视图推出当前状态。
pub fn snapshot(view: &Value) -> Snapshot {
    let requests: Vec<(String, Value)> = view
        .get("requests")
        .and_then(Value::as_array)
        .map(|requests| requests.iter().filter_map(|request| Some((request.get("id")?.to_string(), request.clone()))).collect())
        .unwrap_or_default();
    let turns = view.get("turns").and_then(Value::as_array).cloned().unwrap_or_default();
    let last = turns.last();
    let last_status = last.and_then(|turn| turn.get("status")).and_then(Value::as_str).unwrap_or_default();
    let runtime = view.get("runtime").and_then(Value::as_str).unwrap_or_default();
    let phase = if !requests.is_empty() {
        Phase::Waiting
    } else if runtime == "active" || last_status == "inProgress" {
        Phase::Running
    } else {
        Phase::Idle
    };
    let last_end = turns.iter().rev().find_map(|turn| {
        let status = turn.get("status").and_then(Value::as_str)?;
        if !matches!(status, "completed" | "failed" | "interrupted") {
            return None;
        }
        let turn_id = turn.get("turnId").or_else(|| turn.get("id")).and_then(Value::as_str)?.to_string();
        let error = turn.pointer("/error/message").and_then(Value::as_str).map(|text| clip(text, 200));
        let last_reply = turn
            .get("items")
            .and_then(Value::as_array)
            .and_then(|items| items.iter().rev().find(|item| item.get("type").and_then(Value::as_str) == Some("agentMessage")))
            .and_then(|item| item.get("text").and_then(Value::as_str))
            .map(|text| clip(text, 240));
        let status = if error.is_some() && status == "completed" { "failed" } else { status };
        Some(TurnEnd { turn_id, status: status.to_string(), error, last_reply })
    });
    Snapshot {
        phase,
        requests,
        last_end,
        title: view.get("title").and_then(Value::as_str).unwrap_or_default().to_string(),
        cwd: view.get("cwd").and_then(Value::as_str).unwrap_or_default().to_string(),
    }
}

/// 前后两次状态之间发生的事件。
///
/// `prev` 为空表示第一次看到这个会话：只报等审批；跑完 / 出错只在 `report_new_end` 时报
/// （加加盯着的会话，比如刚派出去、两次查看之间就跑完了的活）。
pub fn transitions(thread_id: &str, prev: Option<&Snapshot>, next: &Snapshot, report_new_end: bool) -> Vec<Event> {
    let mut events = Vec::new();
    let event = |kind: EventKind, detail: String, request: Option<Value>| Event {
        kind,
        thread_id: thread_id.to_string(),
        title: next.title.clone(),
        cwd: next.cwd.clone(),
        detail,
        request,
        turn_id: if kind == EventKind::Waiting { None } else { next.last_end.as_ref().map(|end| end.turn_id.clone()) },
    };
    for (key, request) in &next.requests {
        if prev.is_some_and(|prev| prev.requests.iter().any(|(old, _)| old == key)) {
            continue;
        }
        events.push(event(EventKind::Waiting, request_summary(request), Some(request.clone())));
    }
    if let Some(end) = &next.last_end {
        let is_new = match prev {
            Some(prev) => prev.last_end.as_ref().is_none_or(|old| old.turn_id != end.turn_id),
            None => report_new_end,
        };
        if is_new {
            match end.status.as_str() {
                "failed" => events.push(event(EventKind::Failed, end.error.clone().unwrap_or_else(|| "没有拿到失败原因".into()), None)),
                "completed" => events.push(event(
                    EventKind::Done,
                    end.last_reply.as_deref().map(|reply| format!("最后回复：{reply}")).unwrap_or_default(),
                    None,
                )),
                _ => {}
            }
        }
    }
    events
}

/// 一句话说清请求要什么。
pub fn request_summary(request: &Value) -> String {
    let method = request.get("method").and_then(Value::as_str).unwrap_or_default();
    let params = request.get("params").cloned().unwrap_or(Value::Null);
    match method {
        "item/commandExecution/requestApproval" => format!("要运行命令 `{}`", clip(&command_text(params.get("command")), 160)),
        "item/fileChange/requestApproval" => {
            let reason = params.get("reason").and_then(Value::as_str).map(|reason| format!("：{}", clip(reason, 120))).unwrap_or_default();
            format!("要改文件{reason}")
        }
        "item/tool/requestUserInput" => {
            let question = params
                .get("questions")
                .and_then(Value::as_array)
                .and_then(|questions| questions.first())
                .and_then(|question| question.get("question").or_else(|| question.get("header")))
                .and_then(Value::as_str)
                .unwrap_or("有问题要问你");
            format!("在提问：{}", clip(question, 160))
        }
        "item/permissions/requestApproval" => "要申请更多权限".into(),
        other if other.starts_with("mcpServer/elicitation") => "工具要你填写信息".into(),
        other => format!("要你处理：{other}"),
    }
}

fn command_text(command: Option<&Value>) -> String {
    match command {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" "),
        _ => String::new(),
    }
}

/// 截到 `limit` 个字符，多的用省略号。换行压成空格。
pub fn clip(text: &str, limit: usize) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= limit {
        return flat;
    }
    let mut cut: String = flat.chars().take(limit).collect();
    cut.push('…');
    cut
}

/// 待投递的事件：每个会话只留最新一条，到点一起交给加加。
#[derive(Debug, Default)]
pub struct Pacer {
    pending: Vec<(i64, Event)>,
}

impl Pacer {
    pub fn add(&mut self, event: Event, now: i64) {
        // 同一会话只留最新的；但等审批不被「跑完」顶掉（还要人点）。
        if let Some(index) = self.pending.iter().position(|(_, old)| old.thread_id == event.thread_id) {
            let old = &self.pending[index].1;
            if old.kind == EventKind::Waiting && event.kind != EventKind::Waiting {
                self.pending.push((now, event));
                return;
            }
            self.pending.remove(index);
        }
        self.pending.push((now, event));
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// 最早该投递的时刻。
    pub fn due_at(&self) -> Option<i64> {
        self.pending.iter().map(|(at, event)| at + event.kind.gather_ms()).min()
    }

    pub fn is_due(&self, now: i64) -> bool {
        self.due_at().is_some_and(|due| due <= now)
    }

    /// 取出全部待投递事件：丢掉过期的「跑完」，按 等审批 → 出错 → 跑完 排，最多 [`BATCH_LIMIT`] 条。
    pub fn take(&mut self, now: i64) -> Vec<Event> {
        let mut events: Vec<(i64, Event)> = std::mem::take(&mut self.pending)
            .into_iter()
            .filter(|(at, event)| !(event.kind == EventKind::Done && now - at > STALE_DONE_MS))
            .collect();
        events.sort_by_key(|(at, event)| (event.kind.rank(), *at));
        events.truncate(BATCH_LIMIT);
        events.into_iter().map(|(_, event)| event).collect()
    }

    /// 某个会话的请求被处理了：撤掉还没投递的等审批事件。
    pub fn drop_waiting(&mut self, thread_id: &str) {
        self.pending.retain(|(_, event)| !(event.thread_id == thread_id && event.kind == EventKind::Waiting));
    }
}

/// 一批事件写成交给加加的一段话。
pub fn format_batch(events: &[Event]) -> String {
    let mut text = String::from("<事件 说明=\"Codex++ 自动发来的会话动静，不是用户说的话\">\n");
    for event in events {
        let project = project_name(&event.cwd);
        let place = if project.is_empty() { String::new() } else { format!("，项目 {project}") };
        let detail = if event.detail.is_empty() { String::new() } else { format!(" — {}", event.detail) };
        text.push_str(&format!(
            "- {}：「{}」（会话 {}{place}）{detail}\n",
            event.kind.label(),
            if event.title.is_empty() { "未命名" } else { &event.title },
            short_id(&event.thread_id),
        ));
    }
    text.push_str("</事件>");
    text
}

/// 不唤醒加加时，程序以加加名义发的固定格式提醒。
pub fn notice_text(event: &Event) -> String {
    let title = if event.title.is_empty() { "未命名会话" } else { &event.title };
    match event.kind {
        EventKind::Waiting => format!("「{title}」{}", event.detail),
        EventKind::Failed => format!("「{title}」出错了：{}", event.detail),
        EventKind::Done => format!("「{title}」跑完了"),
    }
}

pub fn short_id(thread_id: &str) -> &str {
    thread_id.get(..8).unwrap_or(thread_id)
}

pub fn project_name(cwd: &str) -> String {
    std::path::Path::new(cwd).file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_default()
}
