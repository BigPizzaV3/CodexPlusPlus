//! 仅修复可从 rollout 交叉验证的原生用户消息投影，不发送消息或修改原始记录。
use crate::session_index_scan::{Candidate, scan};
use anyhow::{Context, bail};
use fs2::FileExt;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant, UNIX_EPOCH},
};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionIndexRepairReport {
    pub scanned_files: usize,
    pub cached_files: usize,
    pub repaired_items: usize,
    pub already_present: usize,
    pub skipped_items: usize,
    #[serde(default)]
    pub deferred_items: usize,
    #[serde(default)]
    pub checked_at_ms: i64,
    #[serde(default)]
    pub pending_details: Vec<PendingDetail>,
    pub issues: Vec<String>,
    pub backup_path: Option<PathBuf>,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingDetail {
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
    pub reason: String,
    pub state: String,
    pub first_seen_at_ms: i64,
    pub last_checked_at_ms: i64,
    pub checks: i64,
}

// 不改写原生轮次状态；连续等待 30 分钟后明确告知需核查，后续仍重新核验。
const WAIT_LIMIT_MS: i64 = 30 * 60 * 1000;
const OLD_SOURCE_MS: i64 = 24 * 60 * 60 * 1000;

fn pending_issue(
    report: &mut SessionIndexRepairReport,
    cache: &Connection,
    c: Option<&Candidate>,
    reason: &str,
    source: &str,
    modified_ms: i64,
) -> anyhow::Result<()> {
    let now = report.checked_at_ms;
    let key = match c {
        Some(c) => format!(
            "{}:{}:{}:{:x}:{reason}",
            c.thread,
            c.turn,
            c.ordinal,
            Sha256::digest(c.text.as_bytes())
        ),
        None => format!("file:{source}:{reason}"),
    };
    cache.execute("INSERT INTO pending(key,first_seen,last_seen,checks) VALUES(?1,?2,?2,1) ON CONFLICT(key) DO UPDATE SET last_seen=?2,checks=checks+1", params![key,now])?;
    let (first, checks): (i64, i64) = cache.query_row(
        "SELECT first_seen,checks FROM pending WHERE key=?1",
        [&key],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let old_source = c.is_some_and(|c| now.saturating_sub(c.created) >= OLD_SOURCE_MS)
        && modified_ms > 0
        && now.saturating_sub(modified_ms) >= OLD_SOURCE_MS;
    let blocked = old_source || now.saturating_sub(first) >= WAIT_LIMIT_MS;
    let explanation = if blocked {
        format!(
            "{reason}；{}，仍会自动核验，不会猜测补建轮次或修改执行状态",
            if old_source {
                "原文及文件已超过 24 小时未更新"
            } else {
                "已连续等待至少 30 分钟"
            }
        )
    } else {
        reason.to_owned()
    };
    if blocked {
        issue(
            report,
            match c {
                Some(c) => format!("{} / {}：{explanation}", c.thread, c.turn),
                None => format!("{source}：{explanation}"),
            },
        );
    } else {
        report.deferred_items += 1;
    }
    report.pending_details.push(PendingDetail {
        thread_id: c.map(|c| c.thread.clone()),
        turn_id: c.map(|c| c.turn.clone()),
        reason: explanation,
        state: if blocked { "blocked" } else { "waiting" }.into(),
        first_seen_at_ms: first,
        last_checked_at_ms: now,
        checks,
    });
    Ok(())
}

fn home(path: Option<&Path>) -> PathBuf {
    path.map(Path::to_path_buf)
        .unwrap_or_else(codex_plus_core::relay_config::default_codex_home_dir)
}

pub fn load_session_index_repair_report(
    path: Option<&Path>,
) -> anyhow::Result<Option<SessionIndexRepairReport>> {
    let path = home(path).join("session-index-repair/report.json");
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(serde_json::from_slice(&fs::read(path)?)?))
}

fn collect_files(dir: &Path, files: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            collect_files(&entry.path(), files)?;
        } else if kind.is_file() && entry.path().extension().is_some_and(|s| s == "jsonl") {
            files.push(entry.path());
        }
    }
    Ok(())
}

// 原生分片可能保留旧 session_meta.id；只信任目录数据库中的确切路径映射。
fn rollout_owners(home: &Path) -> anyhow::Result<std::collections::HashMap<PathBuf, String>> {
    let mut owners = std::collections::HashMap::new();
    let path = home.join("state_5.sqlite");
    if !path.is_file() {
        return Ok(owners);
    }
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut stmt =
        db.prepare("SELECT id,rollout_path FROM threads WHERE rollout_path IS NOT NULL")?;
    let mut ambiguous = std::collections::HashSet::new();
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (id, raw) = row?;
        let path = PathBuf::from(raw);
        let path = if path.is_absolute() {
            path
        } else {
            home.join(path)
        };
        if let Ok(path) = fs::canonicalize(path) {
            if owners.get(&path).is_some_and(|old| old != &id) {
                ambiguous.insert(path.clone());
            }
            owners.insert(path, id);
        }
    }
    for path in ambiguous {
        owners.remove(&path);
    }
    Ok(owners)
}

fn item_text(raw: &str) -> Option<String> {
    let item: Value = serde_json::from_str(raw).ok()?;
    let parts = item["content"].as_array()?;
    let mut text = String::new();
    for part in parts {
        if !matches!(part["type"].as_str(), Some("text" | "Text" | "input_text")) {
            return None;
        }
        text.push_str(part["text"].as_str()?);
    }
    Some(text)
}

// 续写分片可能已不在目录表中。文件名只作约束，必须有原生完成事件的
// turn + item ID + ordinal + 完整正文证据，并排除其他归属，才整份映射。
fn anchored_split_owner(
    db: &Connection,
    path: &Path,
    items: &[Candidate],
    turns: &HashMap<String, HashSet<String>>,
) -> anyhow::Result<Option<String>> {
    let Some((_, suffix)) = path
        .file_stem()
        .and_then(|v| v.to_str())
        .and_then(|v| v.rsplit_once('_'))
    else {
        return Ok(None);
    };
    if uuid::Uuid::parse_str(suffix).is_err() {
        return Ok(None);
    }
    let mut anchored = false;
    for c in items {
        if let Some(owners) = turns.get(&c.turn) {
            if owners.len() != 1 || !owners.contains(suffix) {
                return Ok(None);
            }
            if let Some((id, ordinal)) = &c.projection {
                let existing: Option<(String,i64)> = db.query_row("SELECT item_json,rollout_ordinal FROM thread_items WHERE thread_id=?1 AND turn_id=?2 AND item_id=?3 AND item_type='userMessage'",params![suffix,c.turn,id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
                if let Some((raw, n)) = existing {
                    if n != *ordinal || item_text(&raw).as_deref() != Some(c.text.as_str()) {
                        return Ok(None);
                    }
                    anchored = true;
                }
            }
        }
    }
    Ok(anchored.then(|| suffix.to_owned()))
}

fn resolve_owner(
    db: &Connection,
    path: &Path,
    items: &[Candidate],
    directory_owner: Option<String>,
    turns: &HashMap<String, HashSet<String>>,
) -> anyhow::Result<Option<String>> {
    let owner = anchored_split_owner(db, path, items, turns)?.or(directory_owner);
    if let Some(owner) = &owner {
        if items
            .iter()
            .any(|c| turns.get(&c.turn).is_some_and(|ids| !ids.contains(owner)))
        {
            bail!("目录归属与原生轮次身份冲突，未写入");
        }
    }
    Ok(owner)
}

fn issue(report: &mut SessionIndexRepairReport, message: String) {
    report.skipped_items += 1;
    // 报告不保存消息正文，避免 UI 加载大报告或泄露原文。
    if report.issues.len() < 200 {
        report.issues.push(message);
    }
}

enum Action {
    Present,
    Insert(String),
    Link(String),
    Prepend(String, String),
    RestoreTurn,
    Wait(&'static str),
    Skip(&'static str),
}

fn recovery_id(c: &Candidate) -> String {
    if c.goal {
        if let Some(key) = &c.goal_key {
            return format!(
                "recovered-goal-{:x}",
                Sha256::digest(
                    serde_json::to_vec(&(&c.thread, key, &c.text))
                        .expect("serializable goal identity")
                )
            );
        }
    }
    format!("recovered-user-{}-{}", c.turn, c.ordinal)
}

fn is_original_first(c: &Candidate) -> bool {
    if !c.first_user_evidence_valid {
        return false;
    }
    if c.goal {
        c.first_user_response_ordinal.is_none_or(|n| c.ordinal < n)
            && c.first_user_projection
                .as_ref()
                .is_none_or(|(_, n)| c.ordinal < *n)
    } else {
        c.first_user_response_ordinal == Some(c.ordinal)
            && c.first_user_projection
                .as_ref()
                .is_none_or(|p| c.projection.as_ref() == Some(p))
    }
}

fn deduplicate_goals(
    db: &Connection,
    mut candidates: Vec<Candidate>,
) -> anyhow::Result<Vec<Candidate>> {
    candidates.sort_by_key(|c| (c.created, c.ordinal));
    let mut goals = HashMap::new();
    let mut unique: Vec<Candidate> = Vec::new();
    for c in candidates {
        if let Some(key) = c.goal_key.as_ref().filter(|_| c.goal) {
            let identity = (c.thread.clone(), key.clone(), c.text.clone());
            if let Some(&index) = goals.get(&identity) {
                // 旧版恢复ID不含goal身份。先用每个分片自己的轮次/位置核验，
                // 保留已经恢复的那个分片，不能因更早分片后来出现而再次插入。
                if matches!(inspect(db, &c)?, Action::Present | Action::Link(_))
                    && !matches!(
                        inspect(db, &unique[index])?,
                        Action::Present | Action::Link(_)
                    )
                {
                    unique[index] = c;
                }
                continue;
            }
            goals.insert(identity, unique.len());
        }
        unique.push(c);
    }
    Ok(unique)
}

fn inspect(db: &Connection, c: &Candidate) -> anyhow::Result<Action> {
    let position = c
        .projection
        .as_ref()
        .map_or(c.ordinal, |(_, ordinal)| *ordinal);
    if c.goal && c.goal_key.is_some() {
        let exists: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM thread_items i JOIN thread_turns t ON t.thread_id=i.thread_id AND t.turn_id=i.turn_id AND t.first_user_item_id=i.item_id WHERE i.thread_id=?1 AND i.item_id=?2 AND i.item_type='userMessage' AND json_valid(i.item_json) AND json_extract(i.item_json,'$.content[0].text')=?3)",
            params![c.thread,recovery_id(c),c.text], |r| r.get(0))?;
        if exists {
            return Ok(Action::Present);
        }
    }
    let turn: Option<(Option<String>, String)> = db
        .query_row(
            "SELECT first_user_item_id,status FROM thread_turns WHERE thread_id=?1 AND turn_id=?2",
            params![c.thread, c.turn],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((first, status)) = turn else {
        return Ok(Action::Wait("原生数据库中尚无对应轮次"));
    };
    let mut finished = matches!(status.as_str(), "completed" | "interrupted" | "failed");
    if !finished && status == "inProgress" {
        if let Some((next, ordinal)) = &c.following_turn {
            // 旧投影状态可能没结束。只有原始日志后继边界与原生后继轮次一致时
            // 才允许补用户消息；不改写原生 status，也不把时间长当作结束证据。
            if next != &c.turn && *ordinal > position {
                finished = db.query_row("SELECT EXISTS(SELECT 1 FROM thread_turns WHERE thread_id=?1 AND turn_id=?2 AND rollout_ordinal=?3) AND EXISTS(SELECT 1 FROM thread_turns WHERE thread_id=?1 AND turn_id=?4 AND rollout_ordinal=?5)",params![c.thread,next,ordinal,c.turn,c.start_ordinal],|r|r.get(0))?;
            }
        }
    }
    let mut stmt = db.prepare_cached("SELECT item_id,item_json,rollout_ordinal FROM thread_items WHERE thread_id=?1 AND turn_id=?2 AND item_type='userMessage'")?;
    let rows = stmt
        .query_map(params![c.thread, c.turn], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let valid_first = rows.iter().find(|(id, _, _)| first.as_ref() == Some(id));
    for (id, raw, ordinal) in &rows {
        let text = item_text(raw);
        let same_source = c
            .projection
            .as_ref()
            .map_or(*ordinal == c.ordinal, |(source, n)| {
                source == id && *n == *ordinal
            })
            || (id.starts_with("recovered-") && *ordinal == c.ordinal);
        if same_source && text.as_deref() == Some(c.text.as_str()) {
            let conflict: bool = db.query_row(
                "SELECT EXISTS(SELECT 1 FROM thread_items WHERE thread_id=?1 AND rollout_ordinal=?3 AND NOT(turn_id=?2 AND item_id=?4))",
                params![c.thread, c.turn, position, id], |r| r.get(0))?;
            if conflict {
                return Ok(Action::Skip("相同记录位置已有不同内容"));
            }
            return Ok(if first.as_ref() == Some(id) {
                Action::Present
            } else if first.is_none() && rows.len() == 1 && is_original_first(c) {
                if finished {
                    Action::Link(id.clone())
                } else {
                    Action::Wait("原生轮次仍在执行或状态未知，暂不修改首条消息指针")
                }
            } else if !c.goal && valid_first.is_some_and(|(_, _, n)| *n < *ordinal) {
                Action::Present
            } else {
                Action::Skip("已有用户消息顺序存在冲突")
            });
        }
    }
    // 已存在消息只读核验不受轮次状态影响；只有真正需要写入时才等待结束。
    if !finished {
        return Ok(Action::Wait("原生轮次仍在执行或状态未知，暂不补入消息"));
    }
    let occupied: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM thread_items WHERE thread_id=?1 AND (rollout_ordinal=?3 OR (turn_id=?2 AND item_id=?4)))",
        params![c.thread, c.turn, position, c.projection.as_ref().map(|(id, _)| id)], |r| r.get(0))?;
    if occupied {
        return Ok(Action::Skip("相同记录位置已有不同内容"));
    }
    if !rows.is_empty() {
        // 目标有目标事件与内部 objective 双重证据，位置又早于全部用户消息。
        // 补回目标并调整首条指针，完整保留其后的选项回复和 steering。
        if c.goal && is_original_first(c) && rows.iter().all(|(_, _, n)| *n > position) {
            if let Some((first_id, _, first_ordinal)) = valid_first {
                if rows.iter().all(|(_, _, n)| n >= first_ordinal) {
                    return Ok(Action::Prepend(recovery_id(c), first_id.clone()));
                }
            }
        }
        // steering 是同轮次的后续用户消息；只在首条指针有效且顺序明确时补入。
        if c.goal || c.projection.is_none() || !valid_first.is_some_and(|(_, _, n)| *n < position) {
            return Ok(Action::Skip("该轮次用户消息顺序无法可靠确认"));
        }
        return Ok(Action::Insert(c.projection.as_ref().unwrap().0.clone()));
    }
    if !is_original_first(c) {
        // 同一批次可能先扫描到后续 steering，再扫描到首条消息；暂缓到写入阶段
        // 重新核验，避免把后续内容提升为首条，同时保留首条出现后可恢复的机会。
        return Ok(Action::Wait("等待同轮次已确认的首条用户消息"));
    }
    if let Some(id) = first {
        let exists: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM thread_items WHERE thread_id=?1 AND turn_id=?2 AND item_id=?3)",params![c.thread,c.turn,id],|r|r.get(0))?;
        return Ok(if exists {
            Action::Skip("首条消息指针指向不同类型的记录")
        } else if c
            .projection
            .as_ref()
            .is_some_and(|(native, _)| native == &id)
        {
            Action::Insert(id)
        } else {
            Action::Skip("悬空首条指针与候选消息身份不一致")
        });
    }
    Ok(Action::Insert(
        c.projection
            .as_ref()
            .map(|(id, _)| id.clone())
            .unwrap_or_else(|| recovery_id(c)),
    ))
}

fn apply(db: &Connection, c: &Candidate, action: Action) -> anyhow::Result<bool> {
    if matches!(action, Action::RestoreTurn) {
        let boundary = c.turn_boundary.as_ref().context("缺少完整轮次边界")?;
        db.execute("INSERT INTO thread_turns(thread_id,turn_id,rollout_ordinal,status,error_json,started_at,completed_at,duration_ms,rollout_end_ordinal) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![c.thread,c.turn,boundary.start_ordinal,boundary.status,boundary.error_json,boundary.started_at,boundary.completed_at,boundary.duration_ms,boundary.end_ordinal])?;
        return apply(db, c, inspect(db, c)?);
    }
    let previous_first = match &action {
        Action::Prepend(_, old) => Some(old.clone()),
        _ => None,
    };
    let id = match action {
        Action::Insert(id) | Action::Prepend(id, _) => {
            let ordinal = c
                .projection
                .as_ref()
                .map_or(c.ordinal, |(_, ordinal)| *ordinal);
            let item = json!({"type":"userMessage","id":id,"content":[{"type":"text","text":c.text,"text_elements":[]}]}).to_string();
            db.execute("INSERT INTO thread_items(thread_id,turn_id,item_id,rollout_ordinal,created_at_ms,item_json,item_type,updated_at_ordinal) VALUES(?1,?2,?3,?4,?5,?6,'userMessage',?4)",params![c.thread,c.turn,id,ordinal,c.created,item])?;
            id
        }
        Action::Link(id) => id,
        _ => return Ok(false),
    };
    if let Some(previous_first) = previous_first {
        let changed = db.execute("UPDATE thread_turns SET first_user_item_id=?3 WHERE thread_id=?1 AND turn_id=?2 AND first_user_item_id=?4", params![c.thread,c.turn,id,previous_first])?;
        if changed != 1 {
            bail!("首条消息指针发生变化，取消本次事务");
        }
        return Ok(true);
    }
    db.execute("UPDATE thread_turns SET first_user_item_id=?3 WHERE thread_id=?1 AND turn_id=?2 AND first_user_item_id IS NULL",params![c.thread,c.turn,id])?;
    Ok(true)
}

fn inspect_with_source(
    db: &Connection,
    c: &Candidate,
    verified_source: bool,
) -> anyhow::Result<Action> {
    let action = inspect(db, c)?;
    if matches!(action, Action::Wait("原生数据库中尚无对应轮次")) && verified_source {
        if let Some(b) = &c.turn_boundary {
            let position = c.projection.as_ref().map_or(c.ordinal, |(_, n)| *n);
            let original_first = if c.goal {
                b.first_user_response_ordinal.is_none_or(|n| c.ordinal < n)
                    && b.first_user_projection
                        .as_ref()
                        .is_none_or(|(_, n)| position < *n)
            } else {
                c.projection.is_some()
                    && c.projection == b.first_user_projection
                    && b.first_user_response_ordinal == Some(c.ordinal)
            };
            if b.start_ordinal < c.ordinal
                && c.ordinal <= position
                && position < b.end_ordinal
                && original_first
                && b.started_at <= b.completed_at
                && matches!(b.status.as_str(), "completed" | "failed" | "interrupted")
            {
                let conflicting: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM thread_turns WHERE turn_id=?2 OR (thread_id=?1 AND rollout_ordinal=?3)) OR EXISTS(SELECT 1 FROM thread_items WHERE thread_id=?1 AND (turn_id=?2 OR rollout_ordinal=?4 OR item_id=?5))",params![c.thread,c.turn,b.start_ordinal,position,c.projection.as_ref().map(|(id,_)|id)],|r|r.get(0))?;
                if !conflicting {
                    return Ok(Action::RestoreTurn);
                }
            }
        }
    }
    Ok(action)
}

pub fn repair_session_index(path: Option<&Path>) -> anyhow::Result<SessionIndexRepairReport> {
    let started = Instant::now();
    let home = home(path);
    let work = home.join("session-index-repair");
    fs::create_dir_all(&work)?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(work.join("repair.lock"))?;
    lock.try_lock_exclusive()
        .context("会话索引修复正在运行，请稍后查看报告")?;
    let _lifecycle = crate::try_acquire_provider_sync_lifecycle_guard(Some(&home))?;
    let mut report = SessionIndexRepairReport {
        checked_at_ms: chrono::Utc::now().timestamp_millis(),
        ..Default::default()
    };
    let db_path = home.join("thread_history_1.sqlite");
    if !db_path.is_file() {
        bail!("未找到原生会话历史数据库，尚不能修复消息索引");
    }
    let mut db = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    db.busy_timeout(Duration::from_secs(5))?;
    // 未知数据库版本直接失败，不创建或改造原生 schema。
    db.prepare("SELECT thread_id,turn_id,first_user_item_id,status FROM thread_turns LIMIT 0")?;
    db.prepare("SELECT thread_id,turn_id,item_id,rollout_ordinal,created_at_ms,item_json,item_type,updated_at_ordinal FROM thread_items LIMIT 0")?;
    let mut cache = Connection::open(work.join("scan-cache.sqlite"))?;
    cache.execute_batch("CREATE TABLE IF NOT EXISTS files(path TEXT PRIMARY KEY,stamp TEXT NOT NULL,candidates TEXT NOT NULL);")?;
    cache.execute_batch("CREATE TABLE IF NOT EXISTS pending(key TEXT PRIMARY KEY,first_seen INTEGER NOT NULL,last_seen INTEGER NOT NULL,checks INTEGER NOT NULL);")?;
    let mut files = Vec::new();
    for dir in ["sessions", "archived_sessions"] {
        collect_files(&home.join(dir), &mut files)?;
    }
    files.sort();
    let mut candidates = Vec::new();
    let owners = rollout_owners(&home)?;
    let mut turn_owners: HashMap<String, HashSet<String>> = HashMap::new();
    {
        let mut stmt = db.prepare("SELECT thread_id,turn_id FROM thread_turns")?;
        for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
            let (thread, turn) = row?;
            turn_owners.entry(turn).or_default().insert(thread);
        }
    }
    let mut source_times: HashMap<(String, String, i64), i64> = HashMap::new();
    let mut verified_sources = HashSet::new();
    let cache_tx = cache.transaction()?;
    for path in files {
        report.scanned_files += 1;
        let result: anyhow::Result<Vec<Candidate>> = (|| {
            let meta = fs::metadata(&path)?;
            let stamp = format!(
                "v7:{}:{}",
                meta.len(),
                meta.modified()?.duration_since(UNIX_EPOCH)?.as_nanos()
            );
            let key = path.to_string_lossy();
            let cached: Option<String> = cache_tx
                .query_row(
                    "SELECT candidates FROM files WHERE path=?1 AND stamp=?2",
                    params![key, stamp],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(raw) = cached
                && let Ok(items) = serde_json::from_str(&raw)
            {
                report.cached_files += 1;
                return Ok(items);
            }
            let items = scan(&path)?;
            let after = fs::metadata(&path)?;
            if after.len() == meta.len() && after.modified()? == meta.modified()? {
                cache_tx.execute(
                    "INSERT OR REPLACE INTO files VALUES(?1,?2,?3)",
                    params![key, stamp, serde_json::to_string(&items)?],
                )?;
            } else {
                bail!("文件扫描期间仍在写入，将在下次检查");
            }
            Ok(items)
        })();
        match result {
            Ok(mut items) => {
                let owner = fs::canonicalize(&path)
                    .ok()
                    .and_then(|p| owners.get(&p).cloned());
                // 目录表可能仅指向当前分片而保留父任务 ID，原生完成事件证据优先。
                // 如证据仍指向别的 owner，整文件保留待核查，禁止重复补到目录 owner。
                let owner = match resolve_owner(&db, &path, &items, owner, &turn_owners) {
                    Ok(owner) => owner,
                    Err(error) => {
                        issue(&mut report, format!("{}：{error}", path.display()));
                        continue;
                    }
                };
                let modified = fs::metadata(&path)?
                    .modified()?
                    .duration_since(UNIX_EPOCH)?
                    .as_millis() as i64;
                for item in &mut items {
                    if let Some(owner) = &owner {
                        item.thread.clone_from(owner);
                        verified_sources.insert((
                            item.thread.clone(),
                            item.turn.clone(),
                            item.ordinal,
                        ));
                    }
                    source_times
                        .entry((item.thread.clone(), item.turn.clone(), item.ordinal))
                        .and_modify(|t| *t = (*t).max(modified))
                        .or_insert(modified);
                }
                candidates.extend(items);
            }
            Err(error)
                if error.to_string().contains("文件扫描期间仍在写入")
                    || error.to_string().contains("会话记录尾行不完整") =>
            {
                let modified = fs::metadata(&path)
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_millis() as i64);
                let reason = if error.to_string().contains("会话记录尾行不完整") {
                    "会话记录尾行不完整"
                } else {
                    "文件扫描期间仍在写入"
                };
                if reason == "会话记录尾行不完整"
                    && modified > 0
                    && report.checked_at_ms.saturating_sub(modified) >= OLD_SOURCE_MS
                {
                    issue(
                        &mut report,
                        format!(
                            "{}：尾行截断且超过24小时未更新，需核查原始日志",
                            path.display()
                        ),
                    );
                    continue;
                }
                pending_issue(
                    &mut report,
                    &cache_tx,
                    None,
                    reason,
                    &path.to_string_lossy(),
                    modified,
                )?
            }
            Err(error) => issue(&mut report, format!("{}：{}", path.display(), error)),
        }
    }
    cache_tx.commit()?;
    // 同目标优先保留已有原生投影；尚未恢复时才选择最早分片。
    candidates = deduplicate_goals(&db, candidates)?;
    candidates
        .sort_by(|a, b| (&a.thread, &a.turn, a.ordinal).cmp(&(&b.thread, &b.turn, b.ordinal)));
    candidates.dedup_by(|a, b| {
        a.thread == b.thread && a.turn == b.turn && a.ordinal == b.ordinal && a.text == b.text
    });
    let mut conflicts = std::collections::HashSet::new();
    for pair in candidates.windows(2) {
        if pair[0].thread == pair[1].thread
            && pair[0].turn == pair[1].turn
            && pair[0].ordinal == pair[1].ordinal
            && pair[0].text != pair[1].text
        {
            conflicts.insert((
                pair[0].thread.clone(),
                pair[0].turn.clone(),
                pair[0].ordinal,
            ));
        }
    }
    let mut pending = Vec::new();
    let mut waiting = Vec::new();
    for c in candidates {
        if conflicts.contains(&(c.thread.clone(), c.turn.clone(), c.ordinal)) {
            issue(
                &mut report,
                format!(
                    "{} / {}：相同记录位置有多个候选原文，需人工核对",
                    c.thread, c.turn
                ),
            );
            continue;
        }
        match inspect_with_source(
            &db,
            &c,
            verified_sources.contains(&(c.thread.clone(), c.turn.clone(), c.ordinal)),
        )? {
            Action::Present => report.already_present += 1,
            Action::Skip(reason) => {
                issue(&mut report, format!("{} / {}：{reason}", c.thread, c.turn))
            }
            Action::Wait(_) => waiting.push(c),
            _ => pending.push(c),
        }
    }
    if !pending.is_empty() {
        pending.append(&mut waiting);
        pending
            .sort_by(|a, b| (&a.thread, &a.turn, a.ordinal).cmp(&(&b.thread, &b.turn, b.ordinal)));
        let backup = work.join(format!("before-repair-{}.sqlite", uuid::Uuid::new_v4()));
        db.execute("VACUUM INTO ?1", [backup.to_string_lossy().as_ref()])?;
        report.backup_path = Some(backup);
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        for c in pending {
            // 获取写锁后重新核验，避免扫描与写入之间原生进程已补齐消息。
            let action = inspect_with_source(
                &tx,
                &c,
                verified_sources.contains(&(c.thread.clone(), c.turn.clone(), c.ordinal)),
            )?;
            match action {
                Action::Present => report.already_present += 1,
                Action::Skip(reason) => {
                    issue(&mut report, format!("{} / {}：{reason}", c.thread, c.turn))
                }
                Action::Wait(reason) => pending_issue(
                    &mut report,
                    &cache,
                    Some(&c),
                    reason,
                    "",
                    *source_times
                        .get(&(c.thread.clone(), c.turn.clone(), c.ordinal))
                        .unwrap_or(&0),
                )?,
                action => {
                    if apply(&tx, &c, action)? {
                        report.repaired_items += 1;
                    }
                }
            }
        }
        tx.commit()?;
    }
    for c in waiting {
        match inspect_with_source(
            &db,
            &c,
            verified_sources.contains(&(c.thread.clone(), c.turn.clone(), c.ordinal)),
        )? {
            Action::Present => report.already_present += 1,
            Action::Skip(reason) => {
                issue(&mut report, format!("{} / {}：{reason}", c.thread, c.turn))
            }
            action => {
                let reason = if let Action::Wait(reason) = action {
                    reason
                } else {
                    "原生状态刚发生变化，将在下次检查重新核验"
                };
                pending_issue(
                    &mut report,
                    &cache,
                    Some(&c),
                    reason,
                    "",
                    *source_times
                        .get(&(c.thread.clone(), c.turn.clone(), c.ordinal))
                        .unwrap_or(&0),
                )?;
            }
        }
    }
    report.elapsed_ms = started.elapsed().as_millis() as u64;
    // 不再待处理的候选删除观察状态；以后真正再次缺失时重新计时。
    cache.execute(
        "DELETE FROM pending WHERE last_seen<>?1",
        [report.checked_at_ms],
    )?;
    let report_temp = work.join("report.tmp");
    fs::write(&report_temp, serde_json::to_vec_pretty(&report)?)?;
    fs::rename(report_temp, work.join("report.json"))?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_index_uses_catalog_path_for_split_thread_identity() {
        let home = home_with_rollout();
        let native = Connection::open(home.path().join("thread_history_1.sqlite")).unwrap();
        native
            .execute("UPDATE thread_turns SET thread_id='split'", [])
            .unwrap();
        let state = Connection::open(home.path().join("state_5.sqlite")).unwrap();
        state
            .execute_batch("CREATE TABLE threads(id TEXT,rollout_path TEXT)")
            .unwrap();
        state
            .execute(
                "INSERT INTO threads VALUES('split',?1)",
                [home
                    .path()
                    .join("sessions/rollout.jsonl")
                    .to_string_lossy()
                    .as_ref()],
            )
            .unwrap();
        let report = repair_session_index(Some(home.path())).unwrap();
        assert_eq!(report.repaired_items, 1);
        let owner: String = native
            .query_row("SELECT thread_id FROM thread_items", [], |r| r.get(0))
            .unwrap();
        assert_eq!(owner, "split");
        assert_eq!(
            repair_session_index(Some(home.path()))
                .unwrap()
                .already_present,
            1
        );
    }

    #[test]
    fn session_index_waits_are_tracked_and_eventually_actionable() {
        let cache = Connection::open_in_memory().unwrap();
        cache.execute_batch("CREATE TABLE pending(key TEXT PRIMARY KEY,first_seen INTEGER,last_seen INTEGER,checks INTEGER)").unwrap();
        let c = candidate();
        let mut first = SessionIndexRepairReport {
            checked_at_ms: 200_000,
            ..Default::default()
        };
        pending_issue(&mut first, &cache, Some(&c), "没有原生轮次", "", 200_000).unwrap();
        assert_eq!((first.deferred_items, first.skipped_items), (1, 0));
        let mut later = SessionIndexRepairReport {
            checked_at_ms: 200_000 + WAIT_LIMIT_MS,
            ..Default::default()
        };
        let later_time = later.checked_at_ms;
        pending_issue(&mut later, &cache, Some(&c), "没有原生轮次", "", later_time).unwrap();
        assert_eq!((later.deferred_items, later.skipped_items), (0, 1));
        assert_eq!(later.pending_details[0].checks, 2);
        assert_eq!(later.pending_details[0].first_seen_at_ms, 200_000);
        let mut old = SessionIndexRepairReport {
            checked_at_ms: 200_000 + OLD_SOURCE_MS,
            ..Default::default()
        };
        pending_issue(&mut old, &cache, Some(&c), "原生轮次仍在执行", "", 100_000).unwrap();
        assert_eq!((old.deferred_items, old.skipped_items), (0, 1));
    }

    #[test]
    fn session_index_goal_dedup_preserves_old_repair_but_relinks_orphans() {
        let db = Connection::open_in_memory().unwrap();
        schema(&db);
        let mut c = candidate();
        c.goal = true;
        c.goal_key = Some("goal-identity".into());
        c.first_user_response_ordinal = None;
        apply(&db, &c, inspect(&db, &c).unwrap()).unwrap();
        db.execute("UPDATE thread_turns SET first_user_item_id=NULL", [])
            .unwrap();
        assert!(matches!(inspect(&db, &c).unwrap(), Action::Link(_)));
        apply(&db, &c, inspect(&db, &c).unwrap()).unwrap();
        c.turn = "continuation".into();
        db.execute(
            "INSERT INTO thread_turns VALUES('thread','continuation','completed',NULL)",
            [],
        )
        .unwrap();
        assert!(matches!(inspect(&db, &c).unwrap(), Action::Present));
        assert_eq!(counts(&db).0, 1);
    }

    #[test]
    fn session_index_existing_active_messages_are_not_deferred() {
        let db = Connection::open_in_memory().unwrap();
        schema(&db);
        let first = candidate();
        apply(&db, &first, inspect(&db, &first).unwrap()).unwrap();
        let mut later = first.clone();
        later.ordinal = 8;
        later.projection = Some(("steering".into(), 9));
        later.text = "补充".into();
        apply(&db, &later, inspect(&db, &later).unwrap()).unwrap();
        db.execute("UPDATE thread_turns SET status='inProgress'", [])
            .unwrap();
        for c in [&first, &later] {
            assert!(matches!(inspect(&db, c).unwrap(), Action::Present));
        }
        later.text = "缺失".into();
        assert!(matches!(inspect(&db, &later).unwrap(), Action::Wait(_)));
    }

    #[test]
    fn session_index_goal_prepend_preserves_question_reply_and_steering() {
        let db = Connection::open_in_memory().unwrap();
        schema(&db);
        let mut reply = candidate();
        reply.ordinal = 10;
        reply.first_user_response_ordinal = Some(10);
        reply.text="<send_user_message_question_reply>model question + user answer</send_user_message_question_reply>".into();
        apply(&db, &reply, inspect(&db, &reply).unwrap()).unwrap();
        let before: String = db
            .query_row("SELECT item_json FROM thread_items", [], |r| r.get(0))
            .unwrap();
        let mut goal = candidate();
        goal.goal = true;
        goal.first_user_response_ordinal = None;
        goal.ordinal = 3;
        assert!(matches!(
            inspect(&db, &goal).unwrap(),
            Action::Prepend(_, _)
        ));
        apply(&db, &goal, inspect(&db, &goal).unwrap()).unwrap();
        let after: String = db
            .query_row(
                "SELECT item_json FROM thread_items WHERE rollout_ordinal=10",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(before, after);
        assert_eq!(counts(&db), (2, Some("recovered-user-turn-3".into())));
        assert!(matches!(inspect(&db, &goal).unwrap(), Action::Present));
        assert!(matches!(inspect(&db, &reply).unwrap(), Action::Present));
    }

    #[test]
    fn session_index_split_mapping_requires_strict_anchor_and_no_opposing_owner() {
        let db = Connection::open_in_memory().unwrap();
        schema(&db);
        let owner = "11111111-1111-4111-8111-111111111111";
        let mut anchor = candidate();
        anchor.thread = owner.into();
        anchor.projection = Some(("native-id".into(), 4));
        db.execute("UPDATE thread_turns SET thread_id=?1", [owner])
            .unwrap();
        apply(&db, &anchor, inspect(&db, &anchor).unwrap()).unwrap();
        anchor.thread = "legacy".into();
        let path = PathBuf::from(format!("rollout-legacy_{owner}.jsonl"));
        let mut owners = HashMap::from([("turn".into(), HashSet::from([owner.into()]))]);
        assert_eq!(
            anchored_split_owner(&db, &path, &[anchor.clone()], &owners)
                .unwrap()
                .as_deref(),
            Some(owner)
        );
        assert_eq!(
            resolve_owner(
                &db,
                &path,
                &[anchor.clone()],
                Some("legacy".into()),
                &owners
            )
            .unwrap()
            .as_deref(),
            Some(owner)
        );
        anchor.text = "相同ID但不同原文".into();
        assert!(
            anchored_split_owner(&db, &path, &[anchor.clone()], &owners)
                .unwrap()
                .is_none()
        );
        anchor.text = candidate().text;
        owners.get_mut("turn").unwrap().insert("other".into());
        assert!(
            anchored_split_owner(&db, &path, &[anchor], &owners)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn session_index_old_active_goal_needs_native_following_boundary() {
        let db = Connection::open_in_memory().unwrap();
        schema(&db);
        db.execute_batch("ALTER TABLE thread_turns ADD COLUMN rollout_ordinal INTEGER; UPDATE thread_turns SET status='inProgress'; INSERT INTO thread_turns(thread_id,turn_id,status,rollout_ordinal) VALUES('thread','next','completed',20)").unwrap();
        db.execute(
            "UPDATE thread_turns SET rollout_ordinal=1 WHERE turn_id='turn'",
            [],
        )
        .unwrap();
        let mut c = candidate();
        c.goal = true;
        c.following_turn = Some(("next".into(), 20));
        c.first_user_response_ordinal = None;
        c.start_ordinal = Some(1);
        assert!(matches!(inspect(&db, &c).unwrap(), Action::Insert(_)));
        apply(&db, &c, inspect(&db, &c).unwrap()).unwrap();
        let status: String = db
            .query_row(
                "SELECT status FROM thread_turns WHERE turn_id='turn'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "inProgress");
        c.turn = "absent".into();
        c.text = "另一个目标".into();
        assert!(matches!(inspect(&db, &c).unwrap(), Action::Wait(_)));
    }

    #[test]
    fn session_index_rebuilds_only_verified_missing_turn_with_complete_evidence() {
        let db = Connection::open_in_memory().unwrap();
        schema(&db);
        db.execute_batch("ALTER TABLE thread_turns ADD COLUMN rollout_ordinal INTEGER; ALTER TABLE thread_turns ADD COLUMN error_json TEXT; ALTER TABLE thread_turns ADD COLUMN started_at INTEGER; ALTER TABLE thread_turns ADD COLUMN completed_at INTEGER; ALTER TABLE thread_turns ADD COLUMN duration_ms INTEGER; ALTER TABLE thread_turns ADD COLUMN rollout_end_ordinal INTEGER;").unwrap();
        let mut c = candidate();
        c.turn = "missing".into();
        c.projection = Some(("native-id".into(), 4));
        c.turn_boundary = Some(crate::session_index_scan::TurnBoundary {
            start_ordinal: 1,
            started_at: 100,
            end_ordinal: 10,
            completed_at: 105,
            status: "failed".into(),
            error_json: Some("{\"message\":\"original error\"}".into()),
            duration_ms: Some(5123),
            first_user_projection: Some(("native-id".into(), 4)),
            first_user_response_ordinal: Some(3),
        });
        assert!(matches!(
            inspect_with_source(&db, &c, false).unwrap(),
            Action::Wait(_)
        ));
        c.turn_boundary
            .as_mut()
            .unwrap()
            .first_user_response_ordinal = Some(2);
        assert!(matches!(
            inspect_with_source(&db, &c, true).unwrap(),
            Action::Wait(_)
        ));
        c.turn_boundary
            .as_mut()
            .unwrap()
            .first_user_response_ordinal = Some(3);
        assert!(matches!(
            inspect_with_source(&db, &c, true).unwrap(),
            Action::RestoreTurn
        ));
        let tx = db.unchecked_transaction().unwrap();
        apply(&tx, &c, inspect_with_source(&tx, &c, true).unwrap()).unwrap();
        tx.commit().unwrap();
        let actual:(String,i64,i64,String)=db.query_row("SELECT status,started_at,completed_at,first_user_item_id FROM thread_turns WHERE turn_id='missing'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        assert_eq!(actual, ("failed".into(), 100, 105, "native-id".into()));
        assert!(matches!(
            inspect_with_source(&db, &c, true).unwrap(),
            Action::Present
        ));
        let mut wrong_owner = c.clone();
        wrong_owner.thread = "other".into();
        assert!(matches!(
            inspect_with_source(&db, &wrong_owner, true).unwrap(),
            Action::Wait(_)
        ));
        c.turn = "another".into();
        assert!(matches!(
            inspect_with_source(&db, &c, true).unwrap(),
            Action::Wait(_)
        ));
    }

    #[test]
    fn session_index_scans_archives_and_deduplicates_goals_across_files() {
        let home = home_with_rollout();
        fs::create_dir(home.path().join("archived_sessions")).unwrap();
        let db = Connection::open(home.path().join("thread_history_1.sqlite")).unwrap();
        for (turn, created) in [("goal-a", 100), ("goal-b", 100)] {
            db.execute(
                "INSERT INTO thread_turns VALUES('thread',?1,'completed',NULL)",
                [turn],
            )
            .unwrap();
            let rows = [
                json!({"type":"session_meta","payload":{"id":"thread"}}),
                json!({"type":"event_msg","payload":{"type":"thread_goal_updated","threadId":"thread","goal":{"objective":"目标全文","createdAt":created}}}),
                json!({"type":"event_msg","ordinal":1,"payload":{"type":"task_started","turn_id":turn}}),
                json!({"type":"response_item","ordinal":4,"payload":{"role":"user","content":[{"type":"input_text","text":"<codex_internal_context source=\"goal\"><objective>\n目标全文\n</objective></codex_internal_context>"}],"internal_chat_message_metadata_passthrough":{"turn_id":turn}}}),
            ];
            fs::write(
                home.path().join(format!("archived_sessions/{turn}.jsonl")),
                rows.iter().map(|r| format!("{r}\n")).collect::<String>(),
            )
            .unwrap();
        }
        let report = repair_session_index(Some(home.path())).unwrap();
        assert_eq!((report.scanned_files, report.repaired_items), (3, 2));
        let repaired_turn: String = db
            .query_row(
                "SELECT turn_id FROM thread_items WHERE turn_id LIKE 'goal-%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(repaired_turn, "goal-a");
    }

    #[test]
    fn session_index_restores_missing_item_behind_existing_pointer() {
        let db = Connection::open_in_memory().unwrap();
        schema(&db);
        db.execute(
            "UPDATE thread_turns SET first_user_item_id='original-id'",
            [],
        )
        .unwrap();
        let mut c = candidate();
        c.projection = Some(("original-id".into(), 3));
        c.first_user_projection = c.projection.clone();
        assert!(apply(&db, &c, inspect(&db, &c).unwrap()).unwrap());
        assert_eq!(counts(&db), (1, Some("original-id".into())));
        assert!(matches!(inspect(&db, &c).unwrap(), Action::Present));
    }

    #[test]
    fn legacy_goal_in_later_fragment_prevents_duplicate_repair() {
        let db = Connection::open_in_memory().unwrap();
        schema(&db);
        db.execute_batch("CREATE UNIQUE INDEX idx_thread_items_page ON thread_items(thread_id,rollout_ordinal); INSERT INTO thread_turns VALUES('thread','earlier','completed',NULL)").unwrap();
        let mut legacy = candidate();
        legacy.goal = true;
        legacy.ordinal = 40;
        legacy.first_user_response_ordinal = None;
        // 使用旧版没有goal_key时的恢复ID，正文与位置保持原样。
        apply(&db, &legacy, inspect(&db, &legacy).unwrap()).unwrap();
        legacy.goal_key = Some("created:100".into());
        let mut earlier = legacy.clone();
        earlier.turn = "earlier".into();
        earlier.ordinal = 4;
        let selected = deduplicate_goals(&db, vec![earlier.clone(), legacy.clone()]).unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].turn, legacy.turn);
        assert!(matches!(
            inspect(&db, &selected[0]).unwrap(),
            Action::Present
        ));
        // 同文但不同身份的新目标仍然独立保留。
        earlier.goal_key = Some("created:200".into());
        assert_eq!(
            deduplicate_goals(&db, vec![earlier, legacy]).unwrap().len(),
            2
        );
        assert_eq!(counts(&db).0, 1);
    }

    fn schema(db: &Connection) {
        db.execute_batch("CREATE TABLE thread_turns(thread_id TEXT,turn_id TEXT,status TEXT,first_user_item_id TEXT,PRIMARY KEY(thread_id,turn_id));
            CREATE TABLE thread_items(thread_id TEXT,turn_id TEXT,item_id TEXT,rollout_ordinal INTEGER,created_at_ms INTEGER,item_json TEXT,item_type TEXT,updated_at_ordinal INTEGER,PRIMARY KEY(thread_id,turn_id,item_id));
            INSERT INTO thread_turns VALUES('thread','turn','completed',NULL);").unwrap();
    }

    #[test]
    fn existing_empty_turn_does_not_promote_steering_or_borrow_dangling_id() {
        let db = Connection::open_in_memory().unwrap();
        schema(&db);
        let mut c = candidate();
        c.ordinal = 7;
        c.projection = Some(("steering-native".into(), 8));
        c.first_user_response_ordinal = Some(2);
        c.first_user_projection = Some(("image-native".into(), 4));
        for first in [None, Some("image-native")] {
            db.execute("UPDATE thread_turns SET first_user_item_id=?1", [first])
                .unwrap();
            assert!(matches!(inspect(&db, &c).unwrap(), Action::Wait(_)));
            assert_eq!(counts(&db).0, 0);
        }
        c.ordinal = 2;
        c.projection = Some(("different-native".into(), 4));
        c.first_user_projection = c.projection.clone();
        assert!(matches!(inspect(&db, &c).unwrap(), Action::Skip(_)));
        db.execute(
            "UPDATE thread_turns SET first_user_item_id='different-native'",
            [],
        )
        .unwrap();
        assert!(apply(&db, &c, inspect(&db, &c).unwrap()).unwrap());
        assert!(matches!(inspect(&db, &c).unwrap(), Action::Present));
    }

    #[test]
    fn thread_wide_ordinal_conflict_does_not_abort_other_repairs() {
        let home = home_with_rollout();
        let db = Connection::open(home.path().join("thread_history_1.sqlite")).unwrap();
        db.execute_batch("CREATE UNIQUE INDEX idx_thread_items_page ON thread_items(thread_id,rollout_ordinal); INSERT INTO thread_turns VALUES('thread','other','completed',NULL);").unwrap();
        db.execute("INSERT INTO thread_items VALUES('thread','other','occupied',3,1,'{}','agentMessage',3)",[]).unwrap();
        let original = fs::read_to_string(home.path().join("sessions/rollout.jsonl")).unwrap();
        let other = original
            .replace("\"turn\"", "\"good\"")
            .replace("\"ordinal\":3", "\"ordinal\":9")
            .replace("完整原文", "独立可修消息");
        db.execute(
            "INSERT INTO thread_turns VALUES('thread','good','completed',NULL)",
            [],
        )
        .unwrap();
        fs::write(home.path().join("sessions/good.jsonl"), other).unwrap();
        let report = repair_session_index(Some(home.path())).unwrap();
        assert_eq!((report.repaired_items, report.skipped_items), (1, 1));
        assert!(
            load_session_index_repair_report(Some(home.path()))
                .unwrap()
                .is_some()
        );
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM thread_items WHERE item_type='userMessage'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
    }

    #[test]
    fn goals_with_same_text_but_different_identity_both_survive() {
        let db = Connection::open_in_memory().unwrap();
        schema(&db);
        db.execute_batch("CREATE UNIQUE INDEX idx_thread_items_page ON thread_items(thread_id,rollout_ordinal); INSERT INTO thread_turns VALUES('thread','second','completed',NULL);").unwrap();
        let mut a = candidate();
        a.goal = true;
        a.goal_key = Some("goal-one:100".into());
        a.first_user_response_ordinal = None;
        apply(&db, &a, inspect(&db, &a).unwrap()).unwrap();
        let mut b = a.clone();
        b.turn = "second".into();
        b.ordinal = 20;
        b.goal_key = Some("goal-two:200".into());
        assert!(apply(&db, &b, inspect(&db, &b).unwrap()).unwrap());
        assert_ne!(recovery_id(&a), recovery_id(&b));
        for c in [&a, &b] {
            assert!(matches!(inspect(&db, c).unwrap(), Action::Present));
        }
        b.turn = "continuation".into();
        assert!(matches!(inspect(&db, &b).unwrap(), Action::Present));
        assert_eq!(counts(&db).0, 2);
    }

    #[test]
    fn incomplete_tail_is_never_success_cached_and_rechecks_when_completed() {
        let home = home_with_rollout();
        let path = home.path().join("sessions/rollout.jsonl");
        let original = fs::read_to_string(&path).unwrap();
        fs::write(&path, format!("{original}{{\"type\":")).unwrap();
        for _ in 0..2 {
            let r = repair_session_index(Some(home.path())).unwrap();
            assert_eq!(
                (r.cached_files, r.repaired_items, r.deferred_items),
                (0, 0, 1)
            );
            assert!(r.pending_details[0].reason.contains("尾行不完整"));
        }
        let cache =
            Connection::open(home.path().join("session-index-repair/scan-cache.sqlite")).unwrap();
        cache
            .execute("UPDATE pending SET first_seen=0", [])
            .unwrap();
        let r = repair_session_index(Some(home.path())).unwrap();
        assert_eq!((r.deferred_items, r.skipped_items), (0, 1));
        fs::write(&path,format!("{original}{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"task_complete\",\"turn_id\":\"turn\"}}}}\n")).unwrap();
        let r = repair_session_index(Some(home.path())).unwrap();
        assert_eq!(
            (r.repaired_items, r.skipped_items, r.deferred_items),
            (1, 0, 0)
        );
    }

    #[test]
    fn single_completion_event_cannot_insert_two_repeated_messages() {
        let home = home_with_rollout();
        let rows = [
            json!({"type":"session_meta","ordinal":0,"payload":{"id":"thread"}}),
            json!({"type":"event_msg","ordinal":1,"payload":{"type":"task_started","turn_id":"turn"}}),
            json!({"type":"response_item","ordinal":2,"timestamp":"2026-09-20T01:00:00Z","payload":{"role":"user","content":[{"type":"input_text","text":"重复文本"}],"internal_chat_message_metadata_passthrough":{"turn_id":"turn"}}}),
            json!({"type":"response_item","ordinal":5,"timestamp":"2026-09-20T01:00:00Z","payload":{"role":"user","content":[{"type":"input_text","text":"重复文本"}],"internal_chat_message_metadata_passthrough":{"turn_id":"turn"}}}),
            json!({"type":"event_msg","ordinal":6,"payload":{"type":"item_completed","thread_id":"thread","turn_id":"turn","item":{"type":"UserMessage","id":"native","content":[{"type":"text","text":"重复文本"}]}}}),
        ];
        fs::write(
            home.path().join("sessions/rollout.jsonl"),
            rows.iter().map(|r| format!("{r}\n")).collect::<String>(),
        )
        .unwrap();
        let report = repair_session_index(Some(home.path())).unwrap();
        assert_eq!((report.repaired_items, report.skipped_items), (0, 0));
        let db = Connection::open(home.path().join("thread_history_1.sqlite")).unwrap();
        assert_eq!(counts(&db), (0, None));
    }

    fn candidate() -> Candidate {
        Candidate {
            thread: "thread".into(),
            turn: "turn".into(),
            text: "完整原文".into(),
            ordinal: 3,
            created: 100_000,
            goal: false,
            projection: None,
            following_turn: None,
            turn_boundary: None,
            start_ordinal: None,
            goal_key: None,
            first_user_projection: None,
            first_user_response_ordinal: Some(3),
            first_user_evidence_valid: true,
        }
    }

    #[test]
    fn session_index_recognizes_multiple_messages_and_restores_missing_steering() {
        let db = Connection::open_in_memory().unwrap();
        schema(&db);
        let first = candidate();
        apply(&db, &first, inspect(&db, &first).unwrap()).unwrap();
        let mut last = first.clone();
        last.ordinal = 9;
        last.projection = Some(("last".into(), 10));
        last.text = "后续补充".into();
        assert!(apply(&db, &last, inspect(&db, &last).unwrap()).unwrap());
        let mut middle = first.clone();
        middle.ordinal = 6;
        middle.projection = Some(("middle".into(), 7));
        middle.text = "中途调整要求".into();
        assert!(apply(&db, &middle, inspect(&db, &middle).unwrap()).unwrap());
        for c in [&first, &middle, &last] {
            assert!(matches!(inspect(&db, c).unwrap(), Action::Present));
        }
        assert_eq!(counts(&db), (3, Some("recovered-user-turn-3".into())));
        middle.text = "相同位置的冲突原文".into();
        assert!(matches!(inspect(&db, &middle).unwrap(), Action::Skip(_)));
        middle.ordinal = 2;
        middle.projection = Some(("earlier".into(), 2));
        assert!(matches!(inspect(&db, &middle).unwrap(), Action::Skip(_)));
        assert_eq!(counts(&db).0, 3);
    }

    #[test]
    fn session_index_uses_native_completion_identity_and_preserves_repeated_text() {
        let db = Connection::open_in_memory().unwrap();
        schema(&db);
        let mut first = candidate();
        first.projection = Some(("native-first".into(), 4));
        apply(&db, &first, inspect(&db, &first).unwrap()).unwrap();
        assert!(matches!(inspect(&db, &first).unwrap(), Action::Present));
        let mut repeated = first.clone();
        repeated.ordinal = 8;
        repeated.projection = Some(("native-second".into(), 9));
        assert!(apply(&db, &repeated, inspect(&db, &repeated).unwrap()).unwrap());
        assert_eq!(counts(&db), (2, Some("native-first".into())));
        let ordinal: i64 = db
            .query_row(
                "SELECT rollout_ordinal FROM thread_items WHERE item_id='native-second'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(ordinal, 9);
        first.projection = None;
        assert!(matches!(inspect(&db, &first).unwrap(), Action::Skip(_)));
        repeated.projection = None;
        repeated.text = "没有完成事件的待补消息".into();
        assert!(matches!(inspect(&db, &repeated).unwrap(), Action::Skip(_)));
    }

    #[test]
    fn session_index_scans_multiple_messages_without_turn_wide_conflict() {
        let home = home_with_rollout();
        let path = home.path().join("sessions/rollout.jsonl");
        let mut raw = fs::read_to_string(&path).unwrap();
        let first_completed = json!({"type":"event_msg","ordinal":4,"payload":{"type":"item_completed","thread_id":"thread","turn_id":"turn","item":{"id":"first","type":"UserMessage","content":[{"type":"Text","text":"完整原文"}]}}});
        raw.push_str(&format!("{first_completed}\n"));
        for (ordinal, text) in [(5, "补充要求"), (7, "再次补充")] {
            for row in [
                json!({"type":"event_msg","payload":{"type":"user_message","message":text}}),
                json!({"type":"response_item","ordinal":ordinal,"timestamp":"2026-09-14T12:25:30Z","payload":{"role":"user","content":[{"type":"input_text","text":text}],"internal_chat_message_metadata_passthrough":{"turn_id":"turn"}}}),
                json!({"type":"event_msg","ordinal":ordinal+1,"payload":{"type":"item_completed","thread_id":"thread","turn_id":"turn","item":{"id":format!("item-{ordinal}"),"type":"UserMessage","content":[{"type":"Text","text":text}]}}}),
            ] {
                raw.push_str(&format!("{row}\n"));
            }
        }
        fs::write(&path, raw).unwrap();
        let report = repair_session_index(Some(home.path())).unwrap();
        assert_eq!((report.repaired_items, report.skipped_items), (3, 0));
        let again = repair_session_index(Some(home.path())).unwrap();
        assert_eq!(
            (
                again.repaired_items,
                again.already_present,
                again.skipped_items
            ),
            (0, 3, 0)
        );
    }

    #[test]
    fn session_index_rejects_conflicting_candidates_at_same_ordinal() {
        let home = home_with_rollout();
        let path = home.path().join("sessions/conflict.jsonl");
        let raw = fs::read_to_string(home.path().join("sessions/rollout.jsonl")).unwrap();
        fs::write(path, raw.replace("完整原文", "冲突原文")).unwrap();
        let report = repair_session_index(Some(home.path())).unwrap();
        assert_eq!((report.repaired_items, report.skipped_items), (0, 2));
        let db = Connection::open(home.path().join("thread_history_1.sqlite")).unwrap();
        assert_eq!(counts(&db), (0, None));
    }

    fn counts(db: &Connection) -> (i64, Option<String>) {
        (
            db.query_row("SELECT count(*) FROM thread_items", [], |r| r.get(0))
                .unwrap(),
            db.query_row(
                "SELECT first_user_item_id FROM thread_turns WHERE turn_id='turn'",
                [],
                |r| r.get(0),
            )
            .unwrap(),
        )
    }

    #[test]
    fn session_index_insert_is_atomic_and_idempotent() {
        let mut db = Connection::open_in_memory().unwrap();
        schema(&db);
        let c = candidate();
        {
            let tx = db.transaction().unwrap();
            assert!(apply(&tx, &c, inspect(&tx, &c).unwrap()).unwrap());
            assert_eq!(counts(&tx).0, 1);
            // 未提交事务必须同时回滚消息和 first_user_item_id。
        }
        assert_eq!(counts(&db), (0, None));
        let tx = db.transaction().unwrap();
        assert!(apply(&tx, &c, inspect(&tx, &c).unwrap()).unwrap());
        tx.commit().unwrap();
        assert!(matches!(inspect(&db, &c).unwrap(), Action::Present));
        assert!(!apply(&db, &c, inspect(&db, &c).unwrap()).unwrap());
        assert_eq!(counts(&db), (1, Some("recovered-user-turn-3".into())));
        let raw: String = db
            .query_row("SELECT item_json FROM thread_items", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&raw).unwrap()["content"][0]["text"],
            c.text
        );
    }

    #[test]
    fn session_index_preserves_conflicting_messages_and_skips_unfinished_turns() {
        let db = Connection::open_in_memory().unwrap();
        schema(&db);
        let c = candidate();
        for status in ["inProgress", "running", "unknown"] {
            db.execute("UPDATE thread_turns SET status=?1", [status])
                .unwrap();
            assert!(matches!(inspect(&db, &c).unwrap(), Action::Wait(_)));
            assert_eq!(counts(&db), (0, None));
        }
        db.execute("UPDATE thread_turns SET status='completed'", [])
            .unwrap();
        let mut other = candidate();
        other.text = "原有的不同内容".into();
        apply(&db, &other, inspect(&db, &other).unwrap()).unwrap();
        assert!(matches!(inspect(&db, &c).unwrap(), Action::Skip(_)));
        db.execute("UPDATE thread_turns SET first_user_item_id=NULL", [])
            .unwrap();
        assert!(matches!(inspect(&db, &c).unwrap(), Action::Skip(_)));
        assert_eq!(counts(&db), (1, None));
        let mut missing = candidate();
        missing.turn = "不存在的轮次".into();
        assert!(matches!(inspect(&db, &missing).unwrap(), Action::Wait(_)));
    }

    #[test]
    fn session_index_relinks_only_unambiguous_orphan_user_message() {
        let db = Connection::open_in_memory().unwrap();
        schema(&db);
        let c = candidate();
        apply(&db, &c, inspect(&db, &c).unwrap()).unwrap();
        db.execute("UPDATE thread_turns SET first_user_item_id=NULL", [])
            .unwrap();
        assert!(matches!(inspect(&db, &c).unwrap(), Action::Link(_)));
        assert!(apply(&db, &c, inspect(&db, &c).unwrap()).unwrap());
        assert_eq!(counts(&db).0, 1);
        db.execute(
            "UPDATE thread_turns SET first_user_item_id='different-existing-item'",
            [],
        )
        .unwrap();
        assert!(matches!(inspect(&db, &c).unwrap(), Action::Skip(_)));
        assert_eq!(counts(&db).1.as_deref(), Some("different-existing-item"));
    }

    fn home_with_rollout() -> tempfile::TempDir {
        let home = tempfile::tempdir().unwrap();
        let db = Connection::open(home.path().join("thread_history_1.sqlite")).unwrap();
        schema(&db);
        fs::create_dir(home.path().join("sessions")).unwrap();
        let rows = [
            json!({"type":"session_meta","payload":{"id":"thread"}}),
            json!({"type":"event_msg","ordinal":1,"payload":{"type":"task_started","turn_id":"turn"}}),
            json!({"type":"event_msg","payload":{"type":"user_message","message":"完整原文"}}),
            json!({"type":"response_item","ordinal":3,"timestamp":"2026-09-14T12:24:30Z","payload":{"role":"user","content":[{"type":"input_text","text":"完整原文"}],"internal_chat_message_metadata_passthrough":{"turn_id":"turn"}}}),
        ];
        fs::write(
            home.path().join("sessions/rollout.jsonl"),
            rows.iter().map(|r| format!("{r}\n")).collect::<String>(),
        )
        .unwrap();
        home
    }

    #[test]
    fn session_index_cached_scan_rechecks_rebuilt_native_index_and_saves_report() {
        let home = home_with_rollout();
        let report = repair_session_index(Some(home.path())).unwrap();
        assert_eq!(
            (
                report.scanned_files,
                report.cached_files,
                report.repaired_items
            ),
            (1, 0, 1)
        );
        let backup = Connection::open(report.backup_path.unwrap()).unwrap();
        assert_eq!(counts(&backup), (0, None));
        let again = repair_session_index(Some(home.path())).unwrap();
        assert_eq!(
            (
                again.cached_files,
                again.repaired_items,
                again.already_present
            ),
            (1, 0, 1)
        );
        assert!(again.backup_path.is_none());
        let db = Connection::open(home.path().join("thread_history_1.sqlite")).unwrap();
        db.execute_batch(
            "DELETE FROM thread_items; UPDATE thread_turns SET first_user_item_id=NULL;",
        )
        .unwrap();
        let rebuilt = repair_session_index(Some(home.path())).unwrap();
        assert_eq!((rebuilt.cached_files, rebuilt.repaired_items), (1, 1));
        assert_eq!(counts(&db).0, 1);
        let saved = load_session_index_repair_report(Some(home.path()))
            .unwrap()
            .unwrap();
        assert_eq!((saved.cached_files, saved.repaired_items), (1, 1));
        assert!(!home.path().join("session-index-repair/report.tmp").exists());
    }

    #[test]
    fn session_index_waiting_clears_after_native_turn_finishes_without_backups_while_waiting() {
        let home = home_with_rollout();
        let db = Connection::open(home.path().join("thread_history_1.sqlite")).unwrap();
        db.execute("UPDATE thread_turns SET status='inProgress'", [])
            .unwrap();
        let waiting = repair_session_index(Some(home.path())).unwrap();
        assert_eq!(waiting.deferred_items, 1);
        assert!(waiting.backup_path.is_none());
        assert_eq!(waiting.pending_details[0].checks, 1);
        let cache =
            Connection::open(home.path().join("session-index-repair/scan-cache.sqlite")).unwrap();
        cache
            .execute("UPDATE pending SET first_seen=0", [])
            .unwrap();
        let blocked = repair_session_index(Some(home.path())).unwrap();
        assert_eq!((blocked.deferred_items, blocked.skipped_items), (0, 1));
        assert!(blocked.backup_path.is_none());
        db.execute("UPDATE thread_turns SET status='completed'", [])
            .unwrap();
        let fixed = repair_session_index(Some(home.path())).unwrap();
        assert_eq!(
            (
                fixed.repaired_items,
                fixed.deferred_items,
                fixed.skipped_items
            ),
            (1, 0, 0)
        );
        assert!(fixed.pending_details.is_empty());
        let remaining: i64 = cache
            .query_row("SELECT count(*) FROM pending", [], |r| r.get(0))
            .unwrap();
        assert_eq!(remaining, 0);
        let repeat = repair_session_index(Some(home.path())).unwrap();
        assert_eq!(
            (
                repeat.repaired_items,
                repeat.already_present,
                repeat.deferred_items
            ),
            (0, 1, 0)
        );
        assert!(repeat.backup_path.is_none());
    }
}
