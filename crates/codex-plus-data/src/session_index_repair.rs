//! 仅修复可从 rollout 交叉验证的原生用户消息投影，不发送消息或修改原始记录。
use crate::session_index_scan::{Candidate, scan};
use anyhow::{Context, bail};
use fs2::FileExt;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
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
    pub issues: Vec<String>,
    pub backup_path: Option<PathBuf>,
    pub elapsed_ms: u64,
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
    if !path.is_file() { return Ok(owners); }
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut stmt = db.prepare("SELECT id,rollout_path FROM threads WHERE rollout_path IS NOT NULL")?;
    let mut ambiguous = std::collections::HashSet::new();
    for row in stmt.query_map([], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))? {
        let (id, raw) = row?;
        let path = PathBuf::from(raw);
        let path = if path.is_absolute() { path } else { home.join(path) };
        if let Ok(path) = fs::canonicalize(path) {
            if owners.get(&path).is_some_and(|old| old != &id) { ambiguous.insert(path.clone()); }
            owners.insert(path,id);
        }
    }
    for path in ambiguous { owners.remove(&path); }
    Ok(owners)
}

fn issue(report: &mut SessionIndexRepairReport, message: String) {
    if message.ends_with("轮次仍在执行或状态未知") || message.ends_with("原生轮次尚未建立") || message.contains("文件扫描期间仍在写入") {
        report.deferred_items += 1;
        return;
    }
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
    Skip(&'static str),
}

fn inspect(db: &Connection, c: &Candidate) -> anyhow::Result<Action> {
    let position = c.projection.as_ref().map_or(c.ordinal, |(_, ordinal)| *ordinal);
    if c.goal {
        let exists: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM thread_items i JOIN thread_turns t ON t.thread_id=i.thread_id AND t.turn_id=i.turn_id AND t.first_user_item_id=i.item_id WHERE i.thread_id=?1 AND i.item_type='userMessage' AND json_valid(i.item_json) AND json_extract(i.item_json,'$.content[0].text')=?2)",
            params![c.thread,c.text], |r| r.get(0))?;
        if exists { return Ok(Action::Present); }
    }
    let turn: Option<(Option<String>, String)> = db
        .query_row(
            "SELECT first_user_item_id,status FROM thread_turns WHERE thread_id=?1 AND turn_id=?2",
            params![c.thread, c.turn],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((first, status)) = turn else {
        return Ok(Action::Skip("原生轮次尚未建立"));
    };
    if !matches!(status.as_str(), "completed" | "interrupted" | "failed") {
        return Ok(Action::Skip("轮次仍在执行或状态未知"));
    }
    let mut stmt = db.prepare_cached("SELECT item_id,item_json,rollout_ordinal FROM thread_items WHERE thread_id=?1 AND turn_id=?2 AND item_type='userMessage'")?;
    let rows = stmt
        .query_map(params![c.thread, c.turn], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let valid_first = rows.iter().find(|(id, _, _)| first.as_ref() == Some(id));
    for (id, raw, ordinal) in &rows {
        let item: Value = serde_json::from_str(raw)?;
        let text = item["content"].as_array().map(|a| {
            a.iter()
                .filter_map(|v| v["text"].as_str())
                .collect::<String>()
        });
        let same_source = c.projection.as_ref().is_none_or(|(source, _)| source == id)
            || (id.starts_with("recovered-") && *ordinal == c.ordinal);
        if same_source && text.as_deref() == Some(c.text.as_str()) {
            let conflict: bool = db.query_row(
                "SELECT EXISTS(SELECT 1 FROM thread_items WHERE thread_id=?1 AND turn_id=?2 AND rollout_ordinal=?3 AND item_id<>?4)",
                params![c.thread, c.turn, position, id], |r| r.get(0))?;
            if conflict {
                return Ok(Action::Skip("相同记录位置已有不同内容"));
            }
            return Ok(if first.as_ref() == Some(id) {
                Action::Present
            } else if first.is_none() && rows.len() == 1 {
                Action::Link(id.clone())
            } else if !c.goal && valid_first.is_some_and(|(_, _, n)| *n < *ordinal) {
                Action::Present
            } else {
                Action::Skip("已有用户消息顺序存在冲突")
            });
        }
    }
    let occupied: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM thread_items WHERE thread_id=?1 AND turn_id=?2 AND (rollout_ordinal=?3 OR item_id=?4))",
        params![c.thread, c.turn, position, c.projection.as_ref().map(|(id, _)| id)], |r| r.get(0))?;
    if occupied {
        return Ok(Action::Skip("相同记录位置已有不同内容"));
    }
    if !rows.is_empty() {
        // steering 是同轮次的后续用户消息；只在首条指针有效且顺序明确时补入。
        if c.goal || c.projection.is_none() || !valid_first.is_some_and(|(_, _, n)| *n < position) {
            return Ok(Action::Skip("该轮次用户消息顺序无法可靠确认"));
        }
        return Ok(Action::Insert(c.projection.as_ref().unwrap().0.clone()));
    }
    if let Some(id) = first {
        let exists: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM thread_items WHERE thread_id=?1 AND turn_id=?2 AND item_id=?3)",params![c.thread,c.turn,id],|r|r.get(0))?;
        return Ok(if exists {
            Action::Skip("首条消息指针指向不同类型的记录")
        } else {
            Action::Insert(id)
        });
    }
    Ok(Action::Insert(c.projection.as_ref().map(|(id, _)| id.clone()).unwrap_or_else(|| format!(
        "recovered-user-{}-{}", c.turn, c.ordinal
    ))))
}

fn apply(db: &Connection, c: &Candidate, action: Action) -> anyhow::Result<bool> {
    let id = match action {
        Action::Insert(id) => {
            let ordinal = c.projection.as_ref().map_or(c.ordinal, |(_, ordinal)| *ordinal);
            let item = json!({"type":"userMessage","id":id,"content":[{"type":"text","text":c.text,"text_elements":[]}]}).to_string();
            db.execute("INSERT INTO thread_items(thread_id,turn_id,item_id,rollout_ordinal,created_at_ms,item_json,item_type,updated_at_ordinal) VALUES(?1,?2,?3,?4,?5,?6,'userMessage',?4)",params![c.thread,c.turn,id,ordinal,c.created,item])?;
            id
        }
        Action::Link(id) => id,
        _ => return Ok(false),
    };
    db.execute("UPDATE thread_turns SET first_user_item_id=?3 WHERE thread_id=?1 AND turn_id=?2 AND first_user_item_id IS NULL",params![c.thread,c.turn,id])?;
    Ok(true)
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
    let mut report = SessionIndexRepairReport::default();
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
    let mut files = Vec::new();
    for dir in ["sessions", "archived_sessions"] {
        collect_files(&home.join(dir), &mut files)?;
    }
    files.sort();
    let mut candidates = Vec::new();
    let owners = rollout_owners(&home)?;
    let cache_tx = cache.transaction()?;
    for path in files {
        report.scanned_files += 1;
        let result: anyhow::Result<Vec<Candidate>> = (|| {
            let meta = fs::metadata(&path)?;
            let stamp = format!(
                "v4:{}:{}",
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
                if let Ok(canonical) = fs::canonicalize(&path) && let Some(owner) = owners.get(&canonical) {
                    for item in &mut items { item.thread.clone_from(owner); }
                }
                candidates.extend(items);
            },
            Err(error) => issue(
                &mut report,
                format!("{}：{}", path.display(), error),
            ),
        }
    }
    cache_tx.commit()?;
    // 分片可能重复保存目标事件，只保留该目标最早的可信轮次。
    candidates.sort_by_key(|c| (c.created, c.ordinal));
    let mut goals = std::collections::HashSet::new();
    candidates.retain(|c| !c.goal || goals.insert((c.thread.clone(), c.text.clone())));
    candidates
        .sort_by(|a, b| (&a.thread, &a.turn, a.ordinal).cmp(&(&b.thread, &b.turn, b.ordinal)));
    candidates.dedup_by(|a, b| a.thread == b.thread && a.turn == b.turn && a.ordinal == b.ordinal && a.text == b.text);
    let mut conflicts = std::collections::HashSet::new();
    for pair in candidates.windows(2) {
        if pair[0].thread == pair[1].thread
            && pair[0].turn == pair[1].turn
            && pair[0].ordinal == pair[1].ordinal
            && pair[0].text != pair[1].text
        {
            conflicts.insert((pair[0].thread.clone(), pair[0].turn.clone(), pair[0].ordinal));
        }
    }
    let mut pending = Vec::new();
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
        match inspect(&db, &c)? {
            Action::Present => report.already_present += 1,
            Action::Skip(reason) => {
                issue(&mut report, format!("{} / {}：{reason}", c.thread, c.turn))
            }
            _ => pending.push(c),
        }
    }
    if !pending.is_empty() {
        let backup = work.join(format!("before-repair-{}.sqlite", uuid::Uuid::new_v4()));
        db.execute("VACUUM INTO ?1", [backup.to_string_lossy().as_ref()])?;
        report.backup_path = Some(backup);
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        for c in pending {
            // 获取写锁后重新核验，避免扫描与写入之间原生进程已补齐消息。
            let action = inspect(&tx, &c)?;
            match action {
                Action::Present => report.already_present += 1,
                Action::Skip(reason) => {
                    issue(&mut report, format!("{} / {}：{reason}", c.thread, c.turn))
                }
                action => {
                    if apply(&tx, &c, action)? {
                        report.repaired_items += 1;
                    }
                }
            }
        }
        tx.commit()?;
    }
    report.elapsed_ms = started.elapsed().as_millis() as u64;
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
        native.execute("UPDATE thread_turns SET thread_id='split'", []).unwrap();
        let state = Connection::open(home.path().join("state_5.sqlite")).unwrap();
        state.execute_batch("CREATE TABLE threads(id TEXT,rollout_path TEXT)").unwrap();
        state.execute("INSERT INTO threads VALUES('split',?1)",[home.path().join("sessions/rollout.jsonl").to_string_lossy().as_ref()]).unwrap();
        let report = repair_session_index(Some(home.path())).unwrap();
        assert_eq!(report.repaired_items,1);
        let owner:String=native.query_row("SELECT thread_id FROM thread_items",[],|r|r.get(0)).unwrap();
        assert_eq!(owner,"split");
        assert_eq!(repair_session_index(Some(home.path())).unwrap().already_present,1);
    }

    #[test]
    fn session_index_defers_transient_states_without_manual_review_noise() {
        let mut report = SessionIndexRepairReport::default();
        issue(&mut report,"t：轮次仍在执行或状态未知".into());
        issue(&mut report,"t：原生轮次尚未建立".into());
        issue(&mut report,"p：文件扫描期间仍在写入，将在下次检查".into());
        issue(&mut report,"p：会话记录包含无效 JSON 行".into());
        assert_eq!((report.deferred_items,report.skipped_items,report.issues.len()),(3,1,1));
    }

    #[test]
    fn session_index_goal_dedup_preserves_old_repair_but_relinks_orphans() {
        let db = Connection::open_in_memory().unwrap();
        schema(&db);
        let mut c = candidate();
        c.goal = true;
        apply(&db, &c, inspect(&db, &c).unwrap()).unwrap();
        db.execute("UPDATE thread_turns SET first_user_item_id=NULL", []).unwrap();
        assert!(matches!(inspect(&db, &c).unwrap(), Action::Link(_)));
        apply(&db, &c, inspect(&db, &c).unwrap()).unwrap();
        c.turn = "continuation".into();
        db.execute("INSERT INTO thread_turns VALUES('thread','continuation','completed',NULL)", []).unwrap();
        assert!(matches!(inspect(&db, &c).unwrap(), Action::Present));
        assert_eq!(counts(&db).0, 1);
    }

    #[test]
    fn session_index_scans_archives_and_deduplicates_goals_across_files() {
        let home = home_with_rollout();
        fs::create_dir(home.path().join("archived_sessions")).unwrap();
        let db = Connection::open(home.path().join("thread_history_1.sqlite")).unwrap();
        for (turn, created) in [("goal-a", 100), ("goal-b", 200)] {
            db.execute(
                "INSERT INTO thread_turns VALUES('thread',?1,'completed',NULL)",
                [turn],
            )
            .unwrap();
            let rows = [
                json!({"type":"session_meta","payload":{"id":"thread"}}),
                json!({"type":"event_msg","payload":{"type":"thread_goal_updated","threadId":"thread","goal":{"objective":"目标全文","createdAt":created}}}),
                json!({"type":"event_msg","payload":{"type":"task_started","turn_id":turn}}),
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
        let c = candidate();
        assert!(apply(&db, &c, inspect(&db, &c).unwrap()).unwrap());
        assert_eq!(counts(&db), (1, Some("original-id".into())));
        assert!(matches!(inspect(&db, &c).unwrap(), Action::Present));
    }

    fn schema(db: &Connection) {
        db.execute_batch("CREATE TABLE thread_turns(thread_id TEXT,turn_id TEXT,status TEXT,first_user_item_id TEXT,PRIMARY KEY(thread_id,turn_id));
            CREATE TABLE thread_items(thread_id TEXT,turn_id TEXT,item_id TEXT,rollout_ordinal INTEGER,created_at_ms INTEGER,item_json TEXT,item_type TEXT,updated_at_ordinal INTEGER,PRIMARY KEY(thread_id,turn_id,item_id));
            INSERT INTO thread_turns VALUES('thread','turn','completed',NULL);").unwrap();
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
        let ordinal: i64 = db.query_row("SELECT rollout_ordinal FROM thread_items WHERE item_id='native-second'", [], |r| r.get(0)).unwrap();
        assert_eq!(ordinal, 9);
        first.projection = None;
        assert!(matches!(inspect(&db, &first).unwrap(), Action::Present));
        repeated.projection = None;
        repeated.text = "没有完成事件的待补消息".into();
        assert!(matches!(inspect(&db, &repeated).unwrap(), Action::Skip(_)));
    }

    #[test]
    fn session_index_scans_multiple_messages_without_turn_wide_conflict() {
        let home = home_with_rollout();
        let path = home.path().join("sessions/rollout.jsonl");
        let mut raw = fs::read_to_string(&path).unwrap();
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
        assert_eq!((again.repaired_items, again.already_present, again.skipped_items), (0, 3, 0));
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
            assert!(matches!(inspect(&db, &c).unwrap(), Action::Skip(_)));
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
        assert!(matches!(inspect(&db, &missing).unwrap(), Action::Skip(_)));
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
            json!({"type":"event_msg","payload":{"type":"task_started","turn_id":"turn"}}),
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
}
