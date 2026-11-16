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

// ---------- 前端分发 ----------

fn dir_override() -> &'static Mutex<Option<PathBuf>> {
    static OVERRIDE: Mutex<Option<PathBuf>> = Mutex::new(None);
    &OVERRIDE
}

/// 测试用：把任务目录挪到临时目录。
pub fn set_tasks_dir_for_tests(dir: Option<PathBuf>) {
    *dir_override().lock().unwrap_or_else(|e| e.into_inner()) = dir;
}

pub fn tasks_dir() -> PathBuf {
    if let Some(dir) = dir_override().lock().unwrap_or_else(|e| e.into_inner()).clone() {
        return dir;
    }
    crate::paths::default_app_state_dir().join(TASKS_DIR)
}

fn board_payload(dir: &Path) -> Value {
    json!({ "dir": dir.display().to_string(), "tasks": list_tasks(dir) })
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
        other => bail!("未知的任务面板动作：{other}"),
    }
    Ok(board_payload(&dir))
}
