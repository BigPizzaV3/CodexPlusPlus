//! 会话目录重定位（#1085）：项目文件夹挪了位置之后，把会话记录的工作目录改到新位置。
//!
//! 两种用法共用一套流程：
//! - 目录重映射：旧目录（含子目录）下的全部会话整体搬到新目录，Codex 里的项目根路径一起改；
//! - 批量移动：选中的会话改到指定目录，目标目录是某个 Codex 项目的根时归到这个项目下。
//!
//! 一个会话的工作目录记在三处，必须一起改，否则列表上看着对了、恢复时仍用旧目录：
//! 会话库 `threads.cwd`、侧边栏目录库 `local_thread_catalog.cwd`、
//! rollout 里 `session_meta` / `turn_context` 记录中的路径（工作目录、工作区根、沙箱可写目录）。
//!
//! 先用 [`plan_session_relocation`] 给界面预览，再用 [`apply_session_relocation`] 执行。
//! 执行时按同一个请求重新算一遍，不信任界面传回的列表；改动前的值写进备份目录。
//! 调用方要保证 Codex 桌面端已退出：它运行时会用内存里的状态覆盖全局状态文件，
//! 正在写的 rollout 也不能被替换。

use crate::provider_sync::global_state_snapshot;
use crate::storage::{has_table, wsl_path_alternative};
use anyhow::Context;
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

const GLOBAL_STATE_FILE: &str = ".codex-global-state.json";
const GLOBAL_STATE_BACKUP_FILE: &str = ".codex-global-state.json.bak";
/// 路径不会这么长；更长的字符串（提示词正文等）直接跳过，不做路径比较。
const MAX_PATH_TEXT_BYTES: usize = 4096;
/// 记着「会话 → 路径」的全局状态表；新版 Codex 的键名带账号后缀。
const THREAD_PATH_MAPS: [&str; 2] = [
    "thread-workspace-root-hints",
    "thread-projectless-output-directories",
];
const THREAD_ROOT_HINTS: &str = "thread-workspace-root-hints";
/// 旧版 Codex 直接存工作区根路径的几个列表。
const WORKSPACE_ROOT_LISTS: [&str; 3] = [
    "electron-saved-workspace-roots",
    "active-workspace-roots",
    "project-order",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SessionRelocationRequest {
    /// 旧目录及其子目录下的全部会话搬到新目录。
    #[serde(rename_all = "camelCase")]
    Remap { from: String, to: String },
    /// 选中的会话改到指定目录。
    #[serde(rename_all = "camelCase")]
    Move { thread_ids: Vec<String>, to: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRelocationItem {
    pub thread_id: String,
    pub title: String,
    pub old_cwd: String,
    pub new_cwd: String,
    pub archived: bool,
    #[serde(skip)]
    rollout_path: String,
    /// rollout 里的路径按这个前缀映射改写。
    #[serde(skip)]
    mapping: PathMapping,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRelocationRoot {
    pub old_path: String,
    pub new_path: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRelocationPlan {
    pub items: Vec<SessionRelocationItem>,
    /// 目录重映射时会跟着改的 Codex 项目根路径。
    pub project_roots: Vec<SessionRelocationRoot>,
    /// 批量移动时目标目录对应的 Codex 项目名：会话会归到它下面；没有对应项目时为空。
    pub target_project: Option<String>,
    /// 没有处理的会话及原因。
    pub skipped: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRelocationReport {
    /// 会话库里已改到新目录的会话数。
    pub moved: usize,
    pub rollouts_rewritten: usize,
    /// 找不到 rollout 文件、只改了索引的会话数。
    pub rollouts_missing: usize,
    pub project_roots_updated: usize,
    pub failures: Vec<String>,
    pub skipped: Vec<String>,
    pub backup_dir: Option<String>,
}

/// 「`from` 及其下的路径换到 `to` 下」。比较时统一分隔符、忽略末尾斜杠，
/// Windows 盘符和 UNC 路径不分大小写；换出来的路径沿用 `to` 的分隔符。
#[derive(Debug, Clone, PartialEq, Eq)]
struct PathMapping {
    from_key: String,
    to_root: String,
    backslash: bool,
}

/// 目标目录写进各处时的写法：去掉首尾空白和末尾的分隔符，其余原样。
fn directory_text(path: &str) -> &str {
    path.trim().trim_end_matches(['/', '\\'])
}

impl PathMapping {
    fn new(from: &str, to: &str) -> Self {
        Self {
            from_key: comparable_path(from),
            to_root: directory_text(to).to_string(),
            backslash: to.contains('\\') && !to.contains('/'),
        }
    }

    fn apply(&self, path: &str) -> Option<String> {
        if self.from_key.is_empty() || path.len() < self.from_key.len() || path.len() > MAX_PATH_TEXT_BYTES {
            return None;
        }
        let normalized = normalized_path(path);
        // 只做 ASCII 小写，字节长度不变，下面按字节位置切原串才安全。
        let key = comparable_key(&normalized);
        let suffix = if key == self.from_key {
            ""
        } else if key.starts_with(&self.from_key) && key.as_bytes()[self.from_key.len()] == b'/' {
            &normalized[self.from_key.len()..]
        } else {
            return None;
        };
        let suffix = if self.backslash { suffix.replace('/', "\\") } else { suffix.to_string() };
        Some(format!("{}{suffix}", self.to_root))
    }
}

/// 统一成 `/`、去掉末尾斜杠和 Windows 长路径前缀（`\\?\C:\` → `C:/`，`\\?\UNC\srv\share` → `//srv/share`）。
fn normalized_path(path: &str) -> String {
    let mut path = path.trim().replace('\\', "/");
    while path.len() > 1 && path.ends_with('/') {
        path.pop();
    }
    for prefix in ["//?/", "//./"] {
        let Some(rest) = path.strip_prefix(prefix) else {
            continue;
        };
        if rest.get(..4).is_some_and(|head| head.eq_ignore_ascii_case("UNC/")) {
            return format!("//{}", &rest[4..]);
        }
        let bytes = rest.as_bytes();
        if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
            return rest.to_string();
        }
    }
    path
}

fn comparable_key(normalized: &str) -> String {
    let bytes = normalized.as_bytes();
    let windows = normalized.starts_with("//")
        || (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':');
    if windows { normalized.to_ascii_lowercase() } else { normalized.to_string() }
}

fn comparable_path(path: &str) -> String {
    comparable_key(&normalized_path(path))
}

/// 目录参数要是一个具体的目录：不能为空，也不能是根目录。
fn validated_directory(label: &str, path: &str) -> anyhow::Result<String> {
    let key = comparable_path(path);
    anyhow::ensure!(!key.is_empty(), "{label}不能为空");
    let bytes = key.as_bytes();
    let drive_root = bytes.len() == 2 && bytes[1] == b':';
    anyhow::ensure!(!key.trim_matches('/').is_empty() && !drive_root, "{label}不能是根目录");
    Ok(key)
}

struct ThreadRow {
    id: String,
    cwd: String,
    title: String,
    rollout_path: String,
    archived: bool,
}

/// 预览只读打开，不在别人的库旁边留下自己的写痕迹；执行时才读写打开。
fn open_database(path: &Path, writable: bool) -> anyhow::Result<Connection> {
    let flags = if writable { OpenFlags::SQLITE_OPEN_READ_WRITE } else { OpenFlags::SQLITE_OPEN_READ_ONLY };
    let db = Connection::open_with_flags(path, flags)
        .with_context(|| format!("打不开数据库 {}", path.display()))?;
    db.busy_timeout(Duration::from_secs(2))?;
    Ok(db)
}

fn table_columns(db: &Connection, table: &str) -> anyhow::Result<HashSet<String>> {
    if !has_table(db, table)? {
        return Ok(HashSet::new());
    }
    let mut statement = db.prepare(&format!("PRAGMA table_info(\"{table}\")"))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<HashSet<_>>>()?;
    Ok(columns)
}

/// 一个会话库里的全部线程（含子会话和已归档的）；没有 `threads.cwd` 的库返回空。
fn read_threads(path: &Path) -> anyhow::Result<Vec<ThreadRow>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let db = open_database(path, false)?;
    let columns = table_columns(&db, "threads")?;
    if !columns.contains("id") || !columns.contains("cwd") {
        return Ok(Vec::new());
    }
    let optional = |name: &str, fallback: &str| {
        if columns.contains(name) { format!("COALESCE({name}, {fallback})") } else { fallback.to_string() }
    };
    let sql = format!(
        "SELECT id, COALESCE(cwd, ''), {}, {}, {} FROM threads",
        optional("title", "''"),
        optional("rollout_path", "''"),
        optional("archived", "0"),
    );
    let mut statement = db.prepare(&sql)?;
    let rows = statement
        .query_map([], |row| {
            Ok(ThreadRow {
                id: row.get(0)?,
                cwd: row.get(1)?,
                title: row.get(2)?,
                rollout_path: row.get(3)?,
                archived: row.get::<_, i64>(4)? != 0,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn session_db_paths(home: &Path) -> Vec<PathBuf> {
    codex_plus_core::codex_sqlite::codex_session_db_paths_from_home(home)
}

fn load_global_state(home: &Path) -> anyhow::Result<(Vec<u8>, Map<String, Value>)> {
    global_state_snapshot(&home.join(GLOBAL_STATE_FILE))
}

/// 预览：哪些会话会改、旧新目录各是什么，以及会跟着改的项目根路径。只读。
pub fn plan_session_relocation(
    home: &Path,
    request: &SessionRelocationRequest,
) -> anyhow::Result<SessionRelocationPlan> {
    let mut plan = SessionRelocationPlan::default();
    // 同一线程可能同时在新旧两个库里，预览里只列一次。
    let mut threads: Vec<ThreadRow> = Vec::new();
    let mut seen = HashSet::new();
    for path in session_db_paths(home) {
        for row in read_threads(&path)? {
            if seen.insert(row.id.clone()) {
                threads.push(row);
            }
        }
    }
    let (_, state) = load_global_state(home)?;
    match request {
        SessionRelocationRequest::Remap { from, to } => {
            let from_key = validated_directory("旧目录", from)?;
            let to_key = validated_directory("新目录", to)?;
            anyhow::ensure!(from_key != to_key, "新旧目录相同，不需要重新定位");
            anyhow::ensure!(
                !format!("{to_key}/").starts_with(&format!("{from_key}/"))
                    && !format!("{from_key}/").starts_with(&format!("{to_key}/")),
                "新旧目录不能互相包含"
            );
            let mapping = PathMapping::new(from, to);
            for row in threads {
                if let Some(new_cwd) = mapping.apply(&row.cwd) {
                    plan.items.push(SessionRelocationItem {
                        thread_id: row.id,
                        title: row.title,
                        old_cwd: row.cwd,
                        new_cwd,
                        archived: row.archived,
                        rollout_path: row.rollout_path,
                        mapping: mapping.clone(),
                    });
                }
            }
            let mut roots: Vec<String> = Vec::new();
            collect_project_roots(&state, &mut roots);
            for path in session_db_paths(home) {
                roots.extend(read_project_root_paths(&path)?);
            }
            let mut listed = HashSet::new();
            for old_path in roots {
                if let Some(new_path) = mapping.apply(&old_path)
                    && listed.insert(comparable_path(&old_path))
                {
                    plan.project_roots.push(SessionRelocationRoot { old_path, new_path });
                }
            }
        }
        SessionRelocationRequest::Move { thread_ids, to } => {
            let to_key = validated_directory("目标目录", to)?;
            let target = directory_text(to).to_string();
            let by_id: HashMap<&str, &ThreadRow> = threads.iter().map(|row| (row.id.as_str(), row)).collect();
            let mut requested = HashSet::new();
            for id in thread_ids {
                let id = id.trim();
                let id = id.strip_prefix("local:").unwrap_or(id);
                if id.is_empty() || !requested.insert(id) {
                    continue;
                }
                let Some(row) = by_id.get(id) else {
                    plan.skipped.push(format!("{id}：本地会话库里没有这个会话"));
                    continue;
                };
                let label = if row.title.trim().is_empty() { id } else { row.title.trim() };
                if row.cwd.trim().is_empty() {
                    plan.skipped.push(format!("{label}：没有记录工作目录"));
                    continue;
                }
                if comparable_path(&row.cwd) == to_key {
                    plan.skipped.push(format!("{label}：已经在目标目录"));
                    continue;
                }
                plan.items.push(SessionRelocationItem {
                    thread_id: row.id.clone(),
                    title: row.title.clone(),
                    old_cwd: row.cwd.clone(),
                    new_cwd: target.clone(),
                    archived: row.archived,
                    rollout_path: row.rollout_path.clone(),
                    mapping: PathMapping::new(&row.cwd, &target),
                });
            }
            plan.target_project = target_project(&state, &to_key).map(|(_, name)| name);
        }
    }
    Ok(plan)
}

/// 全局状态里登记的项目根路径：新版在 `local-projects.*.rootPaths`，旧版直接存路径列表。
fn collect_project_roots(state: &Map<String, Value>, roots: &mut Vec<String>) {
    if let Some(projects) = state.get("local-projects").and_then(Value::as_object) {
        for project in projects.values() {
            if let Some(paths) = project.get("rootPaths").and_then(Value::as_array) {
                roots.extend(paths.iter().filter_map(Value::as_str).map(str::to_string));
            }
        }
    }
    for key in WORKSPACE_ROOT_LISTS {
        match state.get(key) {
            Some(Value::Array(items)) => roots.extend(items.iter().filter_map(Value::as_str).map(str::to_string)),
            Some(Value::String(item)) => roots.push(item.clone()),
            _ => {}
        }
    }
}

fn read_project_root_paths(path: &Path) -> anyhow::Result<Vec<String>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let db = open_database(path, false)?;
    if !table_columns(&db, "project_roots")?.contains("path") {
        return Ok(Vec::new());
    }
    let mut statement = db.prepare("SELECT DISTINCT path FROM project_roots WHERE path IS NOT NULL")?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// 根路径正好是目标目录的本地项目；有且只有一个时返回 (id, 名称)。
fn target_project(state: &Map<String, Value>, to_key: &str) -> Option<(String, String)> {
    let projects = state.get("local-projects")?.as_object()?;
    let mut matches = projects.iter().filter(|(_, project)| {
        project
            .get("rootPaths")
            .and_then(Value::as_array)
            .is_some_and(|paths| paths.iter().filter_map(Value::as_str).any(|path| comparable_path(path) == to_key))
    });
    let (id, project) = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    let name = project.get("name").and_then(Value::as_str).unwrap_or(id).to_string();
    Some((id.clone(), name))
}

/// 执行重定位。单个会话失败不影响其余的，原因记在报告里；同一个请求可以重复执行，已改好的不会再动。
pub fn apply_session_relocation(
    home: &Path,
    request: &SessionRelocationRequest,
    backup_root: &Path,
) -> anyhow::Result<SessionRelocationReport> {
    // 与供应商同步、会话索引修复互斥：它们也会改写 rollout 和会话库。
    let _lifecycle = crate::try_acquire_provider_sync_lifecycle_guard(Some(home))
        .context("有其它会话维护任务正在运行，请稍后再试")?;
    let plan = plan_session_relocation(home, request)?;
    let mut report = SessionRelocationReport {
        skipped: plan.skipped.clone(),
        ..Default::default()
    };
    if plan.items.is_empty() && plan.project_roots.is_empty() {
        return Ok(report);
    }

    let (state_bytes, mut state) = load_global_state(home)?;
    let backup_dir = create_backup(backup_root, request, &plan, &state_bytes)?;
    report.backup_dir = Some(backup_dir.to_string_lossy().into_owned());

    // 先改 rollout：它是会话库的来源，改不动的会话索引也不动，保持两边一致。
    let mut ready: Vec<&SessionRelocationItem> = Vec::new();
    for item in &plan.items {
        match rewrite_item_rollout(home, item, &backup_dir) {
            Ok(Some(_)) => {
                report.rollouts_rewritten += 1;
                ready.push(item);
            }
            Ok(None) => {
                report.rollouts_missing += 1;
                ready.push(item);
            }
            Err(error) => report.failures.push(format!("{}：{error:#}", item_label(item))),
        }
    }

    let mut moved: HashSet<&str> = HashSet::new();
    for path in session_db_paths(home) {
        match update_thread_rows(&path, &ready) {
            Ok(ids) => moved.extend(ids),
            Err(error) => report.failures.push(format!("{}：{error:#}", path.display())),
        }
    }
    for item in &ready {
        if !moved.contains(item.thread_id.as_str()) {
            report.failures.push(format!("{}：会话库里的工作目录已变化，未更新", item_label(item)));
        }
    }
    report.moved = moved.len();
    let moved_items: Vec<&SessionRelocationItem> =
        ready.into_iter().filter(|item| moved.contains(item.thread_id.as_str())).collect();

    for path in codex_plus_core::codex_sqlite::codex_thread_reference_db_paths_from_home(home) {
        if let Err(error) = update_catalog_rows(&path, &moved_items) {
            report.failures.push(format!("{}：{error:#}", path.display()));
        }
    }

    let changed = match request {
        SessionRelocationRequest::Remap { from, to } => {
            let mapping = PathMapping::new(from, to);
            for path in session_db_paths(home) {
                match update_project_root_rows(&path, &mapping) {
                    Ok(count) => report.project_roots_updated += count,
                    Err(error) => report.failures.push(format!("{}：{error:#}", path.display())),
                }
            }
            let roots = remap_global_state(&mut state, &mapping);
            report.project_roots_updated += roots.project_roots;
            roots.total
        }
        SessionRelocationRequest::Move { to, .. } => move_threads_in_global_state(&mut state, &moved_items, directory_text(to)),
    };
    if changed > 0
        && let Err(error) = write_global_state(home, &state_bytes, &state)
    {
        report.failures.push(format!("全局状态文件：{error:#}"));
    }
    Ok(report)
}

fn item_label(item: &SessionRelocationItem) -> &str {
    let title = item.title.trim();
    if title.is_empty() { &item.thread_id } else { title }
}

/// 备份目录：请求和清单、全局状态原文；各 rollout 改动前的原行在改写时追加进来。
fn create_backup(
    backup_root: &Path,
    request: &SessionRelocationRequest,
    plan: &SessionRelocationPlan,
    state_bytes: &[u8],
) -> anyhow::Result<PathBuf> {
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let directory = backup_root.join(format!("session-relocate-{stamp}-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]));
    fs::create_dir_all(directory.join("rollouts"))
        .with_context(|| format!("创建备份目录失败：{}", directory.display()))?;
    let manifest = json!({
        "request": request,
        "items": plan.items,
        "projectRoots": plan.project_roots,
    });
    fs::write(directory.join("manifest.json"), serde_json::to_vec_pretty(&manifest)?)?;
    if !state_bytes.is_empty() {
        fs::write(directory.join("codex-global-state.json"), state_bytes)?;
    }
    Ok(directory)
}

/// 会话记录的 rollout 路径在本机的实际位置：原样、WSL 的另一种写法、相对 Codex 目录。
fn resolve_rollout(home: &Path, raw: &str) -> Option<PathBuf> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let mut candidates = vec![PathBuf::from(raw)];
    candidates.extend(wsl_path_alternative(raw).map(PathBuf::from));
    if Path::new(raw).is_relative() {
        candidates.push(home.join(raw));
    }
    candidates.into_iter().find(|path| path.is_file())
}

/// 改写一个会话的 rollout。`Ok(None)`：找不到文件；`Ok(Some(n))`：改了 n 行（可以是 0）。
fn rewrite_item_rollout(
    home: &Path,
    item: &SessionRelocationItem,
    backup_dir: &Path,
) -> anyhow::Result<Option<usize>> {
    let Some(path) = resolve_rollout(home, &item.rollout_path) else {
        return Ok(None);
    };
    let name: String = item
        .thread_id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    let backup = backup_dir.join("rollouts").join(format!("{name}.jsonl"));
    rewrite_rollout(&path, &item.mapping, &backup).map(Some)
}

fn split_line_ending(segment: &str) -> (&str, &str) {
    if let Some(line) = segment.strip_suffix("\r\n") {
        (line, "\r\n")
    } else if let Some(line) = segment.strip_suffix('\n') {
        (line, "\n")
    } else {
        (segment, "")
    }
}

/// `session_meta` / `turn_context` 记录里的旧路径换成新路径；别的记录（对话正文、工具输出）是历史，不动。
fn remap_rollout_record(record: &str, mapping: &PathMapping) -> Option<String> {
    if !record.contains("\"session_meta\"") && !record.contains("\"turn_context\"") {
        return None;
    }
    let mut value: Value = serde_json::from_str(record).ok()?;
    let kind = value.get("type")?.as_str()?;
    if kind != "session_meta" && kind != "turn_context" {
        return None;
    }
    if remap_value(value.get_mut("payload")?, mapping) == 0 {
        return None;
    }
    serde_json::to_string(&value).ok()
}

/// 把值里所有落在旧目录下的路径字符串换掉，返回换了几处。
fn remap_value(value: &mut Value, mapping: &PathMapping) -> usize {
    match value {
        Value::String(text) => match mapping.apply(text) {
            Some(next) if next != *text => {
                *text = next;
                1
            }
            _ => 0,
        },
        Value::Array(items) => items.iter_mut().map(|item| remap_value(item, mapping)).sum(),
        Value::Object(map) => map.values_mut().map(|item| remap_value(item, mapping)).sum(),
        _ => 0,
    }
}

/// 流式改写 rollout：先找出要改的行并把原行写进备份，再原子替换整个文件。
/// 期间文件被追加过（会话正在运行）就放弃，原文件不动。修改时间保持不变，不影响会话排序。
fn rewrite_rollout(path: &Path, mapping: &PathMapping, backup: &Path) -> anyhow::Result<usize> {
    let before = fs::metadata(path)?;
    let stamp = (before.len(), before.modified().ok());
    let mut replacements: HashMap<usize, String> = HashMap::new();
    {
        let mut reader = BufReader::new(File::open(path)?);
        let mut originals = Vec::new();
        let mut line = String::new();
        let mut number = 0usize;
        loop {
            line.clear();
            if reader.read_line(&mut line)? == 0 {
                break;
            }
            number += 1;
            let (record, _) = split_line_ending(&line);
            if let Some(next) = remap_rollout_record(record, mapping) {
                originals.push(json!({ "line": number, "text": record }));
                replacements.insert(number, next);
            }
        }
        if replacements.is_empty() {
            return Ok(0);
        }
        let mut writer = BufWriter::new(File::create(backup)?);
        for original in originals {
            serde_json::to_writer(&mut writer, &original)?;
            writer.write_all(b"\n")?;
        }
        writer.flush()?;
    }
    codex_plus_core::settings::atomic_write_with(path, |file| {
        let mut writer = BufWriter::new(file);
        let mut reader = BufReader::new(File::open(path)?);
        let mut line = String::new();
        let mut number = 0usize;
        loop {
            line.clear();
            if reader.read_line(&mut line)? == 0 {
                break;
            }
            number += 1;
            match replacements.get(&number) {
                Some(next) => {
                    writer.write_all(next.as_bytes())?;
                    writer.write_all(split_line_ending(&line).1.as_bytes())?;
                }
                None => writer.write_all(line.as_bytes())?,
            }
        }
        writer.flush()?;
        let after = fs::metadata(path)?;
        if (after.len(), after.modified().ok()) != stamp {
            return Err(std::io::Error::other("会话正在被写入，rollout 未改动"));
        }
        Ok(())
    })?;
    if let Some(modified) = stamp.1
        && let Ok(file) = File::options().write(true).open(path)
    {
        let _ = file.set_times(fs::FileTimes::new().set_modified(modified));
    }
    Ok(replacements.len())
}

/// 改一个会话库里的 `threads.cwd`，返回这个库里改到的线程 id。旧值对不上的不改。
fn update_thread_rows<'a>(
    path: &Path,
    items: &[&'a SessionRelocationItem],
) -> anyhow::Result<Vec<&'a str>> {
    if items.is_empty() || !path.exists() {
        return Ok(Vec::new());
    }
    let mut db = open_database(path, true)?;
    let columns = table_columns(&db, "threads")?;
    if !columns.contains("id") || !columns.contains("cwd") {
        return Ok(Vec::new());
    }
    let transaction = db.transaction()?;
    let mut updated = Vec::new();
    for item in items {
        let changed = transaction.execute(
            "UPDATE threads SET cwd = ?1 WHERE id = ?2 AND cwd = ?3",
            (&item.new_cwd, &item.thread_id, &item.old_cwd),
        )?;
        if changed > 0 {
            updated.push(item.thread_id.as_str());
        }
    }
    transaction.commit()?;
    Ok(updated)
}

/// 侧边栏目录库里同一批会话的 `cwd`；改了就推进目录版本号，让 Codex 重新读取。
fn update_catalog_rows(path: &Path, items: &[&SessionRelocationItem]) -> anyhow::Result<()> {
    if items.is_empty() || !path.exists() {
        return Ok(());
    }
    let mut db = open_database(path, true)?;
    let columns = table_columns(&db, "local_thread_catalog")?;
    if !columns.contains("thread_id") || !columns.contains("cwd") {
        return Ok(());
    }
    let transaction = db.transaction()?;
    let mut changed = 0usize;
    for item in items {
        changed += transaction.execute(
            "UPDATE local_thread_catalog SET cwd = ?1 WHERE thread_id = ?2 AND cwd = ?3",
            (&item.new_cwd, &item.thread_id, &item.old_cwd),
        )?;
    }
    if changed > 0 && table_columns(&transaction, "local_thread_catalog_metadata")?.contains("catalog_revision") {
        transaction.execute(
            "UPDATE local_thread_catalog_metadata SET catalog_revision = catalog_revision + ?1",
            [changed as i64],
        )?;
    }
    transaction.commit()?;
    Ok(())
}

/// 会话库里的项目根路径（`project_roots.path`）跟着目录重映射。
fn update_project_root_rows(path: &Path, mapping: &PathMapping) -> anyhow::Result<usize> {
    let roots = read_project_root_paths(path)?;
    let changes: Vec<(String, String)> = roots
        .into_iter()
        .filter_map(|old| mapping.apply(&old).filter(|new| *new != old).map(|new| (old, new)))
        .collect();
    if changes.is_empty() {
        return Ok(0);
    }
    let mut db = open_database(path, true)?;
    let transaction = db.transaction()?;
    let mut changed = 0usize;
    for (old, new) in &changes {
        changed += transaction.execute("UPDATE project_roots SET path = ?1 WHERE path = ?2", (new, old))?;
    }
    transaction.commit()?;
    Ok(changed)
}

struct GlobalStateRemap {
    total: usize,
    project_roots: usize,
}

fn is_thread_path_map(key: &str, names: &[&str]) -> bool {
    names.iter().any(|name| {
        key == *name
            || key
                .strip_prefix("electron-owner-value:")
                .and_then(|rest| rest.strip_prefix(name))
                .is_some_and(|rest| rest.starts_with(':'))
    })
}

/// 目录重映射时全局状态里要跟着改的路径：项目根、旧版的工作区根列表和标签、各会话记着的根目录。
fn remap_global_state(state: &mut Map<String, Value>, mapping: &PathMapping) -> GlobalStateRemap {
    let mut project_roots = 0usize;
    if let Some(projects) = state.get_mut("local-projects").and_then(Value::as_object_mut) {
        for project in projects.values_mut() {
            if let Some(paths) = project.get_mut("rootPaths") {
                project_roots += remap_value(paths, mapping);
            }
        }
    }
    for key in WORKSPACE_ROOT_LISTS {
        if let Some(value) = state.get_mut(key) {
            project_roots += remap_value(value, mapping);
        }
    }
    let mut total = project_roots;
    if let Some(Value::Object(labels)) = state.get_mut("electron-workspace-root-labels") {
        let renamed: Vec<(String, String)> = labels
            .keys()
            .filter_map(|key| mapping.apply(key).filter(|next| next != key).map(|next| (key.clone(), next)))
            .collect();
        for (old, new) in renamed {
            if let Some(label) = labels.remove(&old) {
                total += 1;
                labels.entry(new).or_insert(label);
            }
        }
    }
    for (key, value) in state.iter_mut() {
        if is_thread_path_map(key, &THREAD_PATH_MAPS) && value.is_object() {
            total += remap_value(value, mapping);
        }
    }
    GlobalStateRemap { total, project_roots }
}

/// 批量移动后全局状态里跟着改的两处：会话记着的工作区根目录改成目标目录；
/// 目标目录正好是某个本地项目的根时，把会话归到这个项目下。返回改了几处。
fn move_threads_in_global_state(
    state: &mut Map<String, Value>,
    items: &[&SessionRelocationItem],
    target: &str,
) -> usize {
    let mut changed = 0usize;
    for (key, value) in state.iter_mut() {
        if !is_thread_path_map(key, &[THREAD_ROOT_HINTS]) {
            continue;
        }
        let Some(hints) = value.as_object_mut() else {
            continue;
        };
        for item in items {
            for candidate in [item.thread_id.clone(), format!("local:{}", item.thread_id)] {
                if let Some(hint) = hints.get_mut(&candidate)
                    && hint.as_str() != Some(target)
                {
                    *hint = json!(target);
                    changed += 1;
                }
            }
        }
    }
    let Some((project_id, _)) = target_project(state, &comparable_path(target)) else {
        return changed;
    };
    // 只在这一版 Codex 用「会话 → 项目」表时才写，旧版按目录归组，不需要。
    let Some(assignments) = state.get_mut("thread-project-assignments").and_then(Value::as_object_mut) else {
        return changed;
    };
    let assignment = json!({ "projectKind": "local", "projectId": project_id });
    let mut assigned = Vec::new();
    for item in items {
        if assignments.get(&item.thread_id) != Some(&assignment) {
            assignments.insert(item.thread_id.clone(), assignment.clone());
            changed += 1;
        }
        assigned.push(item.thread_id.as_str());
    }
    if let Some(hosts) = state.get_mut("thread-project-membership-host-ids").and_then(Value::as_object_mut) {
        for id in &assigned {
            if !hosts.contains_key(*id) {
                hosts.insert((*id).to_string(), json!("local"));
                changed += 1;
            }
        }
    }
    if let Some(projectless) = state.get_mut("projectless-thread-ids").and_then(Value::as_array_mut) {
        let before = projectless.len();
        projectless.retain(|value| {
            !value.as_str().is_some_and(|id| assigned.contains(&id.strip_prefix("local:").unwrap_or(id)))
        });
        changed += before - projectless.len();
    }
    changed
}

/// 写回全局状态（连同 Codex 自己的 `.bak`）。读入之后文件被别人改过就不写，避免覆盖新内容。
fn write_global_state(home: &Path, original: &[u8], state: &Map<String, Value>) -> anyhow::Result<()> {
    let path = home.join(GLOBAL_STATE_FILE);
    let current = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error.into()),
    };
    anyhow::ensure!(current == original, "文件在处理期间被修改，未写入；请确认 Codex 已退出后重试");
    let text = serde_json::to_string_pretty(&Value::Object(state.clone()))?;
    codex_plus_core::settings::atomic_write(&path, text.as_bytes())?;
    codex_plus_core::settings::atomic_write(&home.join(GLOBAL_STATE_BACKUP_FILE), text.as_bytes())?;
    Ok(())
}
