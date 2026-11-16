//! 任务面板：记下要做什么，以及每件事走到了哪一步。
//!
//! 数据是纯文件，一个任务一个 Markdown 文件，任何编辑器、Agent 都能直接读写：
//!
//! ```text
//! <状态目录>/tasks/
//!   .tasks.json   # {"next": 13}，下一个编号
//!   12.md         # frontmatter（每行 `键: JSON 值`）+ 正文
//! ```
//!
//! 编号对外写作 `T-12`。frontmatter 里不认识的键原样保留，以后加字段不会被旧版本抹掉。
//! 手改文件写坏了也不报错：状态不认识时归到「收集」，没有 frontmatter 时第一行当标题。
//!
//! 任务可以交给 Codex 去做（见「交给 Codex」一节）：对话按首条消息里的 `[T-12]` 认回任务，
//! 对话开始跑时任务进「进行中」，跑完进「待验收」。

use anyhow::{anyhow, bail, Context};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const TASKS_DIR: &str = "tasks";
const INDEX_FILE: &str = ".tasks.json";
/// 单个任务文件的上限，超过的跳过（防止误把大文件放进目录拖慢列表）。
const MAX_TASK_FILE_BYTES: u64 = 1024 * 1024;
const MAX_TITLE_CHARS: usize = 200;

/// 任务状态，顺序即看板列的顺序。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// 收集：刚记下，还没想清楚要不要做。
    Backlog,
    /// 待办：要做，还没开始。
    Todo,
    /// 进行中。
    InProgress,
    /// 待验收：做完了，等人确认。
    InReview,
    /// 卡住：暂时没法继续。
    Blocked,
    /// 完成。
    Done,
    /// 取消：不做了，记录保留。
    Cancelled,
}

impl TaskStatus {
    pub const ALL: [TaskStatus; 7] = [
        TaskStatus::Backlog,
        TaskStatus::Todo,
        TaskStatus::InProgress,
        TaskStatus::InReview,
        TaskStatus::Blocked,
        TaskStatus::Done,
        TaskStatus::Cancelled,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            TaskStatus::Backlog => "backlog",
            TaskStatus::Todo => "todo",
            TaskStatus::InProgress => "in_progress",
            TaskStatus::InReview => "in_review",
            TaskStatus::Blocked => "blocked",
            TaskStatus::Done => "done",
            TaskStatus::Cancelled => "cancelled",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|status| status.as_str() == value)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: u64,
    pub title: String,
    pub status: TaskStatus,
    /// 关联的项目目录，可空。
    pub project: Option<String>,
    /// 正文（Markdown）：要做什么、背景、验收条件。
    pub body: String,
    /// 毫秒时间戳。
    pub created_at: i64,
    pub updated_at: i64,
    /// 最近一次改状态的时间，看板上显示「在这一列待了多久」。
    pub status_at: i64,
    /// 不认识的 frontmatter 键，写回时原样保留。
    #[serde(skip)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewTask {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub status: Option<TaskStatus>,
    #[serde(default)]
    pub project: Option<String>,
}

/// 局部修改：只改给出的字段。`project` 传空字符串表示清除。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskPatch {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub status: Option<TaskStatus>,
    #[serde(default)]
    pub project: Option<String>,
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn clean_title(title: &str) -> String {
    let line = title.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or("");
    line.chars().take(MAX_TITLE_CHARS).collect()
}

fn clean_project(project: Option<&str>) -> Option<String> {
    project.map(str::trim).filter(|value| !value.is_empty()).map(str::to_string)
}

fn task_path(dir: &Path, id: u64) -> PathBuf {
    dir.join(format!("{id}.md"))
}

/// 先写临时文件再改名，写到一半断电也不会留下半个文件。
fn write_atomic(path: &Path, content: &str) -> anyhow::Result<()> {
    let tmp = path.with_extension("md.tmp");
    std::fs::write(&tmp, content).with_context(|| format!("写入 {} 失败", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("保存 {} 失败", path.display()))
}

// ---------- 文件格式 ----------

pub fn render_task(task: &Task) -> String {
    let mut lines = vec!["---".to_string()];
    let mut push = |key: &str, value: Value| lines.push(format!("{key}: {value}"));
    push("title", json!(task.title));
    push("status", json!(task.status.as_str()));
    if let Some(project) = &task.project {
        push("project", json!(project));
    }
    push("createdAt", json!(task.created_at));
    push("updatedAt", json!(task.updated_at));
    push("statusAt", json!(task.status_at));
    for (key, value) in &task.extra {
        push(key, value.clone());
    }
    lines.push("---".to_string());
    let mut out = lines.join("\n");
    out.push('\n');
    if !task.body.is_empty() {
        out.push('\n');
        out.push_str(task.body.trim_end());
        out.push('\n');
    }
    out
}

/// 解析任务文件。`fallback_time` 用于缺时间字段的手写文件（取文件修改时间）。
pub fn parse_task(id: u64, text: &str, fallback_time: i64) -> Task {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut fields: BTreeMap<String, Value> = BTreeMap::new();
    let mut body = text;
    let normalized_start = text.strip_prefix("---\n").or_else(|| text.strip_prefix("---\r\n"));
    if let Some(rest) = normalized_start {
        let mut offset = 0;
        let mut closed = None;
        for line in rest.split_inclusive('\n') {
            let trimmed = line.trim_end_matches(['\r', '\n']);
            if trimmed == "---" {
                closed = Some(offset + line.len());
                break;
            }
            if let Some((key, value)) = trimmed.split_once(':') {
                let key = key.trim();
                let raw = value.trim();
                if !key.is_empty() {
                    // 值按 JSON 解析；手写的裸字符串（title: 修登录）也认。
                    let parsed = serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_string()));
                    fields.insert(key.to_string(), parsed);
                }
            }
            offset += line.len();
        }
        match closed {
            Some(end) => body = &rest[end..],
            None => fields.clear(),
        }
    }
    let body = body.trim_start_matches(['\r', '\n']).trim_end().to_string();

    let take_str = |fields: &mut BTreeMap<String, Value>, key: &str| match fields.remove(key) {
        Some(Value::String(value)) => Some(value),
        Some(Value::Null) | None => None,
        Some(other) => Some(other.to_string()),
    };
    let take_time = |fields: &mut BTreeMap<String, Value>, key: &str| fields.remove(key).and_then(|value| value.as_i64());

    let mut title = take_str(&mut fields, "title").map(|value| clean_title(&value)).unwrap_or_default();
    if title.is_empty() {
        title = clean_title(body.trim_start_matches('#'));
    }
    let status = take_str(&mut fields, "status").and_then(|value| TaskStatus::parse(value.trim())).unwrap_or(TaskStatus::Backlog);
    let project = clean_project(take_str(&mut fields, "project").as_deref());
    let created_at = take_time(&mut fields, "createdAt").unwrap_or(fallback_time);
    let updated_at = take_time(&mut fields, "updatedAt").unwrap_or(created_at.max(fallback_time));
    let status_at = take_time(&mut fields, "statusAt").unwrap_or(created_at);
    Task { id, title, status, project, body, created_at, updated_at, status_at, extra: fields }
}

// ---------- 目录操作 ----------

fn read_next(dir: &Path) -> u64 {
    std::fs::read(dir.join(INDEX_FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|value| value.get("next").and_then(Value::as_u64))
        .unwrap_or(1)
        .max(1)
}

fn task_id_from_file(name: &str) -> Option<u64> {
    let stem = name.strip_suffix(".md")?;
    if stem.is_empty() || stem.starts_with('0') || !stem.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    stem.parse().ok()
}

fn modified_ms(meta: &std::fs::Metadata) -> i64 {
    meta.modified().ok().and_then(|time| time.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// 列出全部任务，编号大的（新的）在前。目录不存在时为空。
pub fn list_tasks(dir: &Path) -> Vec<Task> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut tasks: Vec<Task> = entries
        .flatten()
        .filter_map(|entry| {
            let id = task_id_from_file(&entry.file_name().to_string_lossy())?;
            let meta = entry.metadata().ok().filter(|meta| meta.is_file() && meta.len() <= MAX_TASK_FILE_BYTES)?;
            let text = std::fs::read_to_string(entry.path()).ok()?;
            Some(parse_task(id, &text, modified_ms(&meta)))
        })
        .collect();
    tasks.sort_by_key(|task| std::cmp::Reverse(task.id));
    tasks
}

pub fn read_task(dir: &Path, id: u64) -> anyhow::Result<Task> {
    let path = task_path(dir, id);
    let meta = std::fs::metadata(&path).map_err(|_| anyhow!("任务 T-{id} 不存在"))?;
    let text = std::fs::read_to_string(&path).with_context(|| format!("读取 T-{id} 失败"))?;
    Ok(parse_task(id, &text, modified_ms(&meta)))
}

/// 新建任务。编号取 `.tasks.json` 的 `next`，并跳过已被占用的编号（手动放进来的文件）。
pub fn create_task(dir: &Path, input: NewTask) -> anyhow::Result<Task> {
    let title = clean_title(&input.title);
    let body = input.body.trim_end().to_string();
    let title = if title.is_empty() { clean_title(body.trim_start_matches('#')) } else { title };
    if title.is_empty() {
        bail!("任务标题不能为空");
    }
    std::fs::create_dir_all(dir).with_context(|| format!("创建任务目录 {} 失败", dir.display()))?;
    let mut id = read_next(dir);
    while task_path(dir, id).exists() {
        id += 1;
    }
    let now = now_ms();
    let task = Task {
        id,
        title,
        status: input.status.unwrap_or(TaskStatus::Backlog),
        project: clean_project(input.project.as_deref()),
        body,
        created_at: now,
        updated_at: now,
        status_at: now,
        extra: BTreeMap::new(),
    };
    // create_new：并发新建时撞号直接失败，不会互相覆盖。
    {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(task_path(dir, id))
            .with_context(|| format!("新建 T-{id} 失败"))?;
        file.write_all(render_task(&task).as_bytes())?;
    }
    std::fs::write(dir.join(INDEX_FILE), json!({ "next": id + 1 }).to_string()).context("更新任务编号失败")?;
    Ok(task)
}

pub fn update_task(dir: &Path, id: u64, patch: TaskPatch) -> anyhow::Result<Task> {
    let mut task = read_task(dir, id)?;
    let before = task.clone();
    if let Some(title) = patch.title {
        let title = clean_title(&title);
        if title.is_empty() {
            bail!("任务标题不能为空");
        }
        task.title = title;
    }
    if let Some(body) = patch.body {
        task.body = body.trim_end().to_string();
    }
    if let Some(project) = patch.project {
        task.project = clean_project(Some(&project));
    }
    let now = now_ms();
    if let Some(status) = patch.status.filter(|status| *status != task.status) {
        task.status = status;
        task.status_at = now;
    }
    if task == before {
        return Ok(task);
    }
    task.updated_at = now;
    write_atomic(&task_path(dir, id), &render_task(&task))?;
    Ok(task)
}

/// 删除一个任务文件。编号不回收，免得以后的 `T-12` 指到另一件事。
pub fn delete_task(dir: &Path, id: u64) -> anyhow::Result<()> {
    let path = task_path(dir, id);
    if !path.is_file() {
        bail!("任务 T-{id} 不存在");
    }
    std::fs::remove_file(&path).with_context(|| format!("删除 T-{id} 失败"))
}

// ---------- 交给 Codex ----------
//
// 「在 Codex 中开始」打开 `codex://threads/new?prompt=…&path=…`：Codex 选中（没有就新建）该目录的项目，
// 新对话的输入框里预填好任务内容，用户确认后自己发送。提示词以 `[T-12]` 开头，Codex 把首条用户消息
// 存进线程表的 `first_user_message`，靠它把对话认回任务；手动新开的对话里写上 `[T-12]` 也算。
// 对话的运行状态读 rollout 末尾的 `task_started` / `task_complete` / `turn_aborted` 事件。

/// rollout 只读末尾这么多字节找最近一次回合事件。
const ROLLOUT_TAIL_BYTES: u64 = 256 * 1024;
/// 首条消息里只在开头这么多字符内找 `[T-N]`（前面可能有 Codex 拼进去的附件说明）。
const MARKER_SCAN_CHARS: usize = 4000;
/// 最多认这么多条带编号的对话。
const MAX_LINKED_THREADS: usize = 500;
const LAST_MESSAGE_CHARS: usize = 240;

/// 交给 Codex 的提示词：`[T-12] 标题`，有正文时空一行接正文。
pub fn task_prompt(task: &Task) -> String {
    let mut prompt = format!("[T-{}] {}", task.id, task.title);
    let body = task.body.trim();
    if !body.is_empty() {
        prompt.push_str("\n\n");
        prompt.push_str(body);
    }
    prompt
}

fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// 新对话链接。项目目录存在时带上 `path`，Codex 会切到（或新建）这个项目；不存在就用当前项目。
pub fn new_thread_link(task: &Task) -> String {
    let mut link = format!("codex://threads/new?prompt={}", percent_encode(&task_prompt(task)));
    if let Some(project) = task.project.as_deref().filter(|path| Path::new(path).is_absolute() && Path::new(path).is_dir()) {
        link.push_str("&path=");
        link.push_str(&percent_encode(project));
    }
    link
}

/// 消息里第一个 `[T-数字]` 的编号。
pub fn task_marker_id(text: &str) -> Option<u64> {
    let head: String = text.chars().take(MARKER_SCAN_CHARS).collect();
    let mut rest = head.as_str();
    while let Some(start) = rest.find("[T-") {
        let after = &rest[start + 3..];
        let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
        if !digits.is_empty()
            && !digits.starts_with('0')
            && after[digits.len()..].starts_with(']')
            && let Ok(id) = digits.parse()
        {
            return Some(id);
        }
        rest = after;
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    /// 回合进行中。
    Running,
    /// 最近一个回合正常结束。
    Finished,
    /// 最近一个回合被中断。
    Aborted,
    /// 还没有回合事件（刚建好、或 rollout 读不到）。
    Idle,
}

/// 一条认回任务的 Codex 对话。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRun {
    pub task_id: u64,
    pub thread_id: String,
    pub title: String,
    pub cwd: String,
    pub state: RunState,
    /// 毫秒：线程表的更新时间。
    pub updated_at: i64,
    /// 毫秒：最近一个回合开始 / 结束的时间。
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    /// 回合结束时 Codex 最后说的话（截断）。
    pub last_message: Option<String>,
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// rollout 每行的 `timestamp`（`2026-11-02T10:08:57.450Z`，UTC）转毫秒。
pub fn parse_rollout_timestamp(text: &str) -> Option<i64> {
    let text = text.strip_suffix('Z')?;
    let (date, time) = text.split_once('T')?;
    let mut date_parts = date.split('-').map(|part| part.parse::<i64>().ok());
    let (year, month, day) = (date_parts.next()??, date_parts.next()??, date_parts.next()??);
    let (clock, fraction) = time.split_once('.').unwrap_or((time, ""));
    let mut clock_parts = clock.split(':').map(|part| part.parse::<i64>().ok());
    let (hour, minute, second) = (clock_parts.next()??, clock_parts.next()??, clock_parts.next()??);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let millis = format!("{fraction:0<3}").get(..3)?.parse::<i64>().ok()?;
    Some(((days_from_civil(year, month, day) * 24 + hour) * 60 + minute) * 60_000 + second * 1000 + millis)
}

/// 从 rollout 末尾倒着找最近的回合事件：运行状态、开始 / 结束时间、最后一句话。
pub fn rollout_run_state(path: &Path) -> (RunState, Option<i64>, Option<i64>, Option<String>) {
    use std::io::{Read, Seek, SeekFrom};
    let idle = (RunState::Idle, None, None, None);
    let Ok(mut file) = std::fs::File::open(path) else { return idle };
    let len = file.metadata().map(|meta| meta.len()).unwrap_or(0);
    let start = len.saturating_sub(ROLLOUT_TAIL_BYTES);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return idle;
    }
    let mut bytes = Vec::new();
    if file.read_to_end(&mut bytes).is_err() {
        return idle;
    }
    let text = String::from_utf8_lossy(&bytes);
    let mut lines: Vec<&str> = text.lines().collect();
    if start > 0 && !lines.is_empty() {
        lines.remove(0); // 从中间开始读，第一行不完整。
    }
    let (mut state, mut finished_at, mut last_message) = (None, None, None);
    for line in lines.iter().rev() {
        if !line.contains("\"event_msg\"") || !(line.contains("task_started") || line.contains("task_complete") || line.contains("turn_aborted")) {
            continue;
        }
        let Ok(event) = serde_json::from_str::<Value>(line) else { continue };
        let at = event.get("timestamp").and_then(Value::as_str).and_then(parse_rollout_timestamp);
        let payload = event.get("payload").unwrap_or(&Value::Null);
        match payload.get("type").and_then(Value::as_str) {
            Some("task_started") => return (state.unwrap_or(RunState::Running), at, finished_at, last_message),
            Some("task_complete") if state.is_none() => {
                state = Some(RunState::Finished);
                finished_at = at;
                last_message = payload
                    .get("last_agent_message")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .map(|text| text.chars().take(LAST_MESSAGE_CHARS).collect());
            }
            Some("turn_aborted") if state.is_none() => {
                state = Some(RunState::Aborted);
                finished_at = at;
            }
            _ => {}
        }
    }
    match state {
        Some(state) => (state, None, finished_at, last_message),
        None => idle,
    }
}

/// 秒或毫秒都转成毫秒（线程表的时间列在不同版本里单位不一样）。
fn to_ms(value: i64) -> i64 {
    if value.abs() < 100_000_000_000 { value * 1000 } else { value }
}

/// 从 Codex 线程表找首条消息带 `[T-N]` 的对话（不含已归档），新的在前。读不到数据库时为空。
pub fn linked_runs(home: &Path) -> Vec<TaskRun> {
    let Ok((connection, columns)) = crate::extension_history::thread_db(home) else { return Vec::new() };
    let has = |column: &str| columns.iter().any(|existing| existing == column);
    if !has("first_user_message") {
        return Vec::new();
    }
    let column = |name: &str| if has(name) { name.to_string() } else { format!("NULL AS {name}") };
    let archived = if has("archived") { " AND (archived = 0 OR archived IS NULL)" } else { "" };
    let order = if has("updated_at") { " ORDER BY updated_at DESC" } else { "" };
    let sql = format!(
        "SELECT id, {}, {}, {}, {}, first_user_message FROM threads WHERE instr(first_user_message, '[T-') > 0{archived}{order} LIMIT {MAX_LINKED_THREADS}",
        column("title"),
        column("cwd"),
        column("updated_at"),
        column("rollout_path"),
    );
    let Ok(mut statement) = connection.prepare(&sql) else { return Vec::new() };
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?.unwrap_or_default(),
            row.get::<_, Option<String>>(2)?.unwrap_or_default(),
            row.get::<_, Option<i64>>(3)?.unwrap_or(0),
            row.get::<_, Option<String>>(4)?,
            row.get::<_, Option<String>>(5)?.unwrap_or_default(),
        ))
    });
    let Ok(rows) = rows else { return Vec::new() };
    rows.filter_map(Result::ok)
        .filter_map(|(thread_id, title, cwd, updated_at, rollout, message)| {
            let task_id = task_marker_id(&message)?;
            let (state, started_at, finished_at, last_message) =
                rollout.as_deref().filter(|path| !path.is_empty()).map(|path| rollout_run_state(Path::new(path))).unwrap_or((RunState::Idle, None, None, None));
            let title = if title.trim().is_empty() { message.lines().next().unwrap_or("").chars().take(MAX_TITLE_CHARS).collect() } else { title };
            Some(TaskRun { task_id, thread_id, title, cwd, state, updated_at: to_ms(updated_at), started_at, finished_at, last_message })
        })
        .collect()
}

/// 按对话的进展推任务状态。只看「进入当前状态之后」发生的事，手动挪过的状态不会被旧事件拽回去：
/// - 有回合在跑，且开始于进入当前状态之后：收集 / 待办 / 待验收 → 进行中。
/// - 没有回合在跑，最近一次正常结束在进入当前状态之后：收集 / 待办 / 进行中 → 待验收。
///
/// 卡住、完成、已取消不自动动。
pub fn auto_status(task: &Task, runs: &[TaskRun]) -> Option<TaskStatus> {
    let runs: Vec<&TaskRun> = runs.iter().filter(|run| run.task_id == task.id).collect();
    let running_since = runs.iter().filter(|run| run.state == RunState::Running).filter_map(|run| run.started_at).max();
    if runs.iter().any(|run| run.state == RunState::Running) {
        let fresh = running_since.is_some_and(|at| at > task.status_at);
        return (fresh && matches!(task.status, TaskStatus::Backlog | TaskStatus::Todo | TaskStatus::InReview)).then_some(TaskStatus::InProgress);
    }
    let finished = runs.iter().filter(|run| run.state == RunState::Finished).filter_map(|run| run.finished_at).max()?;
    (finished > task.status_at && matches!(task.status, TaskStatus::Backlog | TaskStatus::Todo | TaskStatus::InProgress)).then_some(TaskStatus::InReview)
}

/// 把对话进展写回任务文件，返回改了几个。
pub fn apply_auto_status(dir: &Path, tasks: &[Task], runs: &[TaskRun]) -> usize {
    tasks
        .iter()
        .filter_map(|task| auto_status(task, runs).map(|status| (task.id, status)))
        .filter(|(id, status)| update_task(dir, *id, TaskPatch { status: Some(*status), ..Default::default() }).is_ok())
        .count()
}

// ---------- 项目 ----------
//
// 详情里的「项目」从列表里选，不手打路径。新版 Codex 把项目存在 app-server 里，本地只有项目 ID，
// 所以候选取自线程表里最近对话的工作目录（`cwd`），去掉 Codex 自己生成的目录；列表外的用系统窗口选文件夹。

/// 最多列这么多个最近项目。
const MAX_RECENT_PROJECTS: usize = 20;
/// 往回扫这么多条对话找工作目录。
const RECENT_PROJECT_SCAN_THREADS: usize = 2000;

/// 不当作项目的工作目录：系统临时目录、Codex 给无项目对话建的 `Documents/Codex/<日期>/…`、Codex 的 worktree。
pub fn is_scratch_dir(path: &str) -> bool {
    let normalized = path.replace('\\', "/");
    let lower = normalized.to_lowercase();
    let temp = std::env::temp_dir().to_string_lossy().replace('\\', "/").to_lowercase();
    let in_dir = |root: &str| {
        let root = root.trim_end_matches('/');
        !root.is_empty() && (lower == root || lower.starts_with(&format!("{root}/")))
    };
    if in_dir("/tmp") || in_dir("/private/tmp") || in_dir("/var/folders") || in_dir("/private/var/folders") || in_dir(&temp) {
        return true;
    }
    if lower.contains("/.codex/worktrees/") {
        return true;
    }
    // `…/Codex/2026-11-16/new-chat`：Codex 给无项目对话自动建的目录。
    let parts: Vec<&str> = normalized.split('/').filter(|part| !part.is_empty()).collect();
    parts.windows(2).any(|pair| {
        pair[0].eq_ignore_ascii_case("codex")
            && pair[1].len() == 10
            && pair[1].bytes().enumerate().all(|(index, byte)| if index == 4 || index == 7 { byte == b'-' } else { byte.is_ascii_digit() })
    })
}

/// 最近在 Codex 里用过的项目目录，最近的在前；目录已经不存在的、临时目录不算。读不到数据库时为空。
pub fn recent_projects(home: &Path) -> Vec<String> {
    let Ok((connection, columns)) = crate::extension_history::thread_db(home) else { return Vec::new() };
    let has = |column: &str| columns.iter().any(|existing| existing == column);
    if !has("cwd") {
        return Vec::new();
    }
    let archived = if has("archived") { " AND (archived = 0 OR archived IS NULL)" } else { "" };
    let order = if has("updated_at") { " ORDER BY updated_at DESC" } else { "" };
    let sql = format!("SELECT cwd FROM threads WHERE cwd IS NOT NULL AND cwd != ''{archived}{order} LIMIT {RECENT_PROJECT_SCAN_THREADS}");
    let Ok(mut statement) = connection.prepare(&sql) else { return Vec::new() };
    let Ok(rows) = statement.query_map([], |row| row.get::<_, String>(0)) else { return Vec::new() };
    let mut seen = std::collections::HashSet::new();
    let mut projects = Vec::new();
    for cwd in rows.filter_map(Result::ok) {
        let cwd = cwd.trim().trim_end_matches(['/', '\\']).to_string();
        if cwd.is_empty() || !seen.insert(cwd.clone()) || is_scratch_dir(&cwd) || !Path::new(&cwd).is_dir() {
            continue;
        }
        projects.push(cwd);
        if projects.len() >= MAX_RECENT_PROJECTS {
            break;
        }
    }
    projects
}

/// 用系统自己的窗口选一个文件夹；用户取消时为 None。
fn pick_folder(initial: Option<&str>) -> anyhow::Result<Option<String>> {
    let initial = initial.filter(|path| Path::new(path).is_dir());
    #[cfg(target_os = "macos")]
    {
        // 先 activate：osascript 自己到前台，窗口才不会被 Codex 挡在后面。
        let mut script = String::from("activate\nset chosen to choose folder with prompt \"选择任务的项目目录\"");
        if let Some(path) = initial {
            script.push_str(&format!(" default location (POSIX file {})", serde_json::to_string(path)?));
        }
        script.push_str("\nPOSIX path of chosen");
        let output = std::process::Command::new("osascript").arg("-e").arg(&script).output().context("打开选择文件夹窗口失败")?;
        if !output.status.success() {
            let message = String::from_utf8_lossy(&output.stderr);
            // -128：用户点了取消。
            if message.contains("-128") {
                return Ok(None);
            }
            bail!("选择文件夹失败：{}", message.trim());
        }
        let path = String::from_utf8_lossy(&output.stdout).trim().trim_end_matches('/').to_string();
        Ok((!path.is_empty()).then_some(path))
    }
    #[cfg(windows)]
    {
        let mut script = String::from(
            "Add-Type -AssemblyName System.Windows.Forms; $d = New-Object System.Windows.Forms.FolderBrowserDialog; $d.Description = '选择任务的项目目录'; $d.UseDescriptionForTitle = $true;",
        );
        if let Some(path) = initial {
            script.push_str(&format!(" $d.SelectedPath = '{}';", path.replace('\'', "''")));
        }
        script.push_str(" $owner = New-Object System.Windows.Forms.Form -Property @{TopMost=$true}; if ($d.ShowDialog($owner) -eq 'OK') { [Console]::OutputEncoding = [Text.Encoding]::UTF8; Write-Output $d.SelectedPath }");
        let mut command = std::process::Command::new("powershell.exe");
        command.args(["-NoProfile", "-STA", "-Command", &script]);
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let output = command.output().context("打开选择文件夹窗口失败")?;
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok((!path.is_empty()).then_some(path))
    }
    #[cfg(target_os = "linux")]
    {
        let start = initial.map(|path| format!("{path}/")).unwrap_or_default();
        let attempts: [(&str, Vec<String>); 2] = [
            ("zenity", vec!["--file-selection".into(), "--directory".into(), "--title=选择任务的项目目录".into(), format!("--filename={start}")]),
            ("kdialog", vec!["--getexistingdirectory".into(), if start.is_empty() { ".".into() } else { start.clone() }]),
        ];
        for (program, args) in attempts {
            let Ok(output) = std::process::Command::new(program).args(&args).output() else { continue };
            let path = String::from_utf8_lossy(&output.stdout).trim().trim_end_matches('/').to_string();
            return Ok((output.status.success() && !path.is_empty()).then_some(path));
        }
        bail!("没有找到 zenity 或 kdialog，没法弹出选择文件夹窗口")
    }
    #[cfg(all(not(windows), not(target_os = "macos"), not(target_os = "linux")))]
    {
        let _ = initial;
        bail!("当前平台不支持选择文件夹")
    }
}

fn open_link(url: &str) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        crate::windows_open_url(url)
    }
    #[cfg(target_os = "macos")]
    {
        let status = std::process::Command::new("open").arg(url).status().context("打开 Codex 链接失败")?;
        if !status.success() {
            bail!("打开 Codex 链接失败（open 退出码 {status}）");
        }
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open").arg(url).spawn().map(|_| ()).context("打开 Codex 链接失败")
    }
    #[cfg(all(not(windows), not(target_os = "macos"), not(target_os = "linux")))]
    {
        let _ = url;
        bail!("当前平台不支持打开 Codex 链接")
    }
}

// ---------- 前端分发 ----------

#[derive(Default)]
struct Overrides {
    tasks_dir: Option<PathBuf>,
    codex_home: Option<PathBuf>,
    /// 测试里把要打开的链接记下来，不真的打开。
    opened: Option<Vec<String>>,
    /// 测试里「选择文件夹」直接返回这个，不弹窗（外层 None 表示没设置）。
    picked: Option<Option<String>>,
}

fn overrides() -> std::sync::MutexGuard<'static, Overrides> {
    static OVERRIDES: Mutex<Overrides> = Mutex::new(Overrides { tasks_dir: None, codex_home: None, opened: None, picked: None });
    OVERRIDES.lock().unwrap_or_else(|e| e.into_inner())
}

/// 测试用：把任务目录挪到临时目录。
pub fn set_tasks_dir_for_tests(dir: Option<PathBuf>) {
    overrides().tasks_dir = dir;
}

/// 测试用：Codex 主目录挪到临时目录，并且「在 Codex 中开始」只记下链接不真的打开（传 None 恢复）。
pub fn set_codex_home_for_tests(home: Option<PathBuf>) {
    let mut overrides = overrides();
    overrides.opened = home.as_ref().map(|_| Vec::new());
    overrides.codex_home = home;
}

/// 测试用：「选择文件夹」不弹窗，直接返回给定结果（`Some(None)` 表示用户取消，`None` 恢复真实窗口）。
pub fn set_picked_folder_for_tests(picked: Option<Option<String>>) {
    overrides().picked = picked;
}

/// 测试用：取走记下的链接。
pub fn take_opened_links_for_tests() -> Vec<String> {
    overrides().opened.as_mut().map(std::mem::take).unwrap_or_default()
}

pub fn tasks_dir() -> PathBuf {
    if let Some(dir) = overrides().tasks_dir.clone() {
        return dir;
    }
    crate::paths::default_app_state_dir().join(TASKS_DIR)
}

fn codex_home() -> PathBuf {
    overrides().codex_home.clone().unwrap_or_else(crate::codex_sqlite::default_codex_home_dir)
}

fn board_payload(dir: &Path) -> Value {
    let runs = linked_runs(&codex_home());
    let mut tasks = list_tasks(dir);
    if apply_auto_status(dir, &tasks, &runs) > 0 {
        tasks = list_tasks(dir);
    }
    // 只回传认得出任务的对话。
    let runs: Vec<&TaskRun> = runs.iter().filter(|run| tasks.iter().any(|task| task.id == run.task_id)).collect();
    json!({ "dir": dir.display().to_string(), "tasks": tasks, "runs": runs, "projects": recent_projects(&codex_home()) })
}

/// Codex 页面（`47-task-board-page.js`）经 bridge 路由 `/task-board` 调用：`{ action, ...args }`。
/// 只读写本地任务文件，不依赖管理器是否在跑。
pub fn bridge_route(payload: &Value) -> anyhow::Result<Value> {
    let action = payload.get("action").and_then(Value::as_str).unwrap_or("list");
    let mut value = dispatch(action, payload)?;
    if let Some(object) = value.as_object_mut() {
        object.insert("status".into(), json!("ok"));
    }
    Ok(value)
}

/// 动作分发：每个动作都返回整个面板（任务不多，整表刷新最简单，也不会和手改文件对不上）。
pub fn dispatch(action: &str, args: &Value) -> anyhow::Result<Value> {
    let dir = tasks_dir();
    let id = || -> anyhow::Result<u64> { args.get("id").and_then(Value::as_u64).ok_or_else(|| anyhow!("缺少任务编号")) };
    let decode = |key: &str| -> anyhow::Result<Value> { Ok(args.get(key).cloned().unwrap_or_else(|| Value::Object(Map::new()))) };
    match action {
        "list" => {}
        "create" => {
            let input: NewTask = serde_json::from_value(decode("task")?).context("任务内容格式不对")?;
            let task = create_task(&dir, input)?;
            let mut payload = board_payload(&dir);
            payload["created"] = json!(task.id);
            return Ok(payload);
        }
        "update" => {
            let patch: TaskPatch = serde_json::from_value(decode("patch")?).context("修改内容格式不对")?;
            update_task(&dir, id()?, patch)?;
        }
        "delete" => delete_task(&dir, id()?)?,
        "start" => {
            let task = read_task(&dir, id()?)?;
            let link = new_thread_link(&task);
            let recorded = overrides().opened.as_mut().map(|opened| opened.push(link.clone())).is_some();
            if !recorded {
                open_link(&link)?;
            }
        }
        // 用系统窗口给任务选项目目录，选好直接写进任务；取消时什么都不改。
        "pickProject" => {
            let id = id()?;
            let task = read_task(&dir, id)?;
            let fake = overrides().picked.clone();
            let picked = match fake {
                Some(picked) => picked,
                None => pick_folder(task.project.as_deref())?,
            };
            if let Some(path) = &picked {
                update_task(&dir, id, TaskPatch { project: Some(path.clone()), ..Default::default() })?;
            }
            let mut payload = board_payload(&dir);
            payload["picked"] = json!(picked);
            return Ok(payload);
        }
        other => bail!("未知的任务面板动作：{other}"),
    }
    Ok(board_payload(&dir))
}
