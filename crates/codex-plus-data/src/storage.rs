use crate::BackupStore;
use crate::backup::BackupFileSource;
use codex_plus_core::models::{DeleteResult, DeleteStatus, SessionRef};
use rusqlite::types::{ToSqlOutput, Value as SqlValue, ValueRef};
use rusqlite::{Connection, OpenFlags, OptionalExtension, ToSql};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::HashSet;
use std::fs;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

mod session_paging;
pub use session_paging::{LocalSessionPage, LocalSessionPager};

/// 按线程/会话 id 批量查询标题（只读、best-effort，供请求日志等展示用）。
///
/// 只用于展示：库打不开、schema 不支持或字段缺失都跳过，返回已查到的部分。
pub fn local_session_titles(
    db_paths: &[PathBuf],
    ids: &[String],
) -> std::collections::HashMap<String, String> {
    let wanted: Vec<&str> = ids
        .iter()
        .map(|id| id.trim())
        .filter(|id| !id.is_empty())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let mut titles = std::collections::HashMap::new();
    if wanted.is_empty() {
        return titles;
    }
    for db_path in db_paths {
        if !db_path.exists() {
            continue;
        }
        let Ok(db) = Connection::open(db_path) else {
            continue;
        };
        let Ok(query) = session_paging::session_query(&db) else {
            continue;
        };
        for chunk in wanted.chunks(200) {
            let placeholders = vec!["?"; chunk.len()].join(",");
            let sql = format!("SELECT id, title FROM ({query}) WHERE id IN ({placeholders})");
            let Ok(mut statement) = db.prepare(&sql) else {
                continue;
            };
            let Ok(rows) = statement.query_map(
                rusqlite::params_from_iter(chunk.iter().copied()),
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            ) else {
                continue;
            };
            for row in rows.flatten() {
                let (id, title) = row;
                let title = title.trim();
                if !title.is_empty() {
                    titles.entry(id).or_insert_with(|| title.to_string());
                }
            }
        }
    }
    titles
}

pub fn delete_local_from_paths(
    db_paths: impl IntoIterator<Item = PathBuf>,
    backup_store: BackupStore,
    session: &SessionRef,
    codex_home: Option<&Path>,
) -> DeleteResult {
    let mut result = failed(
        &session.session_id,
        "Thread not found in local storage".to_string(),
    );
    let mut deleted_count = 0usize;
    let mut backup_tokens = Vec::new();
    let mut backup_paths = Vec::new();
    let mut failures = Vec::new();
    let mut only_not_found = true;
    for db_path in db_paths {
        let adapter = match codex_home {
            Some(home) => {
                SQLiteStorageAdapter::new(db_path, backup_store.clone()).with_codex_home(home)
            }
            None => SQLiteStorageAdapter::new(db_path, backup_store.clone()),
        };
        let (candidate_result, disposition) = adapter.delete_local_classified(session);
        only_not_found &= disposition == DeleteDisposition::NotFound;
        if let Some(token) = candidate_result.undo_token.as_ref() {
            let tokens =
                serde_json::from_str::<Vec<String>>(token).unwrap_or_else(|_| vec![token.clone()]);
            backup_tokens.extend(tokens);
        }
        if let Some(path) = candidate_result.backup_path.as_ref() {
            backup_paths.push(path.clone());
        }
        if matches!(candidate_result.status, DeleteStatus::LocalDeleted) {
            deleted_count += 1;
            result = candidate_result;
        } else {
            if disposition != DeleteDisposition::NotFound {
                failures.push(candidate_result.message.clone());
            }
            if deleted_count == 0 {
                result = candidate_result;
            }
        }
    }
    if !failures.is_empty() {
        result.status = DeleteStatus::Failed;
        result.message = format!(
            "已从 {deleted_count} 个本地存储删除；删除未完成：{}",
            failures.join("；")
        );
    } else if deleted_count > 1 {
        result.message = format!("已从 {deleted_count} 个本地存储删除");
    }
    result.undo_token = match backup_tokens.len() {
        0 => None,
        1 => backup_tokens.pop(),
        _ => Some(json!(backup_tokens).to_string()),
    };
    result.backup_path = if backup_paths.len() == 1 {
        backup_paths.first().cloned()
    } else {
        None
    };
    if !backup_paths.is_empty() && (!failures.is_empty() || backup_paths.len() > 1) {
        result.message = format!("{}；备份：{}", result.message, backup_paths.join("、"));
    }
    // 纯 API 模式（model_provider = "custom"）下 threads 表是空的，上面每个库都查不到
    // 记录，于是直接返回「Thread not found in local storage」而会话行仍留在列表里
    // ——因为 UI 读的是 session_index.jsonl，那条记录没人清（#1998）。
    //
    // 数据库里没有不代表索引里没有，这里退一步清索引：真清掉了就算删除成功，
    // 索引里也没有才是真的找不到。
    if only_not_found && let Some(home) = codex_home {
        let thread_id = normalize_codex_thread_id(&session.session_id);
        let mut cleanup_failed = false;
        match crate::provider_sync::remove_session_index_entry(home, &thread_id) {
            Ok(removed) if removed > 0 => {
                result.status = DeleteStatus::LocalDeleted;
                result.message = format!("已从 session_index.jsonl 清理 {removed} 条记录");
            }
            Ok(_) => {}
            Err(error) => {
                cleanup_failed = true;
                result.message =
                    format!("{}；session_index.jsonl 清理失败：{error}", result.message);
            }
        }
        match crate::provider_sync::remove_thread_sidebar_references(home, &thread_id) {
            Ok(cleanup)
                if cleanup.global_state_entries_removed > 0 || cleanup.catalog_rows_removed > 0 =>
            {
                if matches!(result.status, DeleteStatus::Failed) {
                    result.status = DeleteStatus::LocalDeleted;
                    result.message = "已清理侧边栏索引".to_string();
                }
            }
            Ok(_) => {}
            Err(error) => {
                cleanup_failed = true;
                result.message = format!("{}；侧边栏索引清理失败：{error}", result.message);
            }
        }
        if cleanup_failed {
            result.status = DeleteStatus::Failed;
        }
    }
    result
}

#[derive(Debug, Clone)]
pub struct SQLiteStorageAdapter {
    db_path: PathBuf,
    backup_store: BackupStore,
    allowed_db_paths: Vec<PathBuf>,
    codex_home: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SchemaKind {
    GenericSessions,
    CodexThreads,
    CodexAutomationRuns,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeleteDisposition {
    NotFound,
    Attempted,
}

fn codex_thread_filter(db: &Connection) -> anyhow::Result<String> {
    let mut subagent_filters = Vec::new();
    if has_table(db, "thread_spawn_edges")?
        && table_columns(db, "thread_spawn_edges")?
            .iter()
            .any(|column| column == "child_thread_id")
    {
        subagent_filters.push(
            "NOT EXISTS (SELECT 1 FROM thread_spawn_edges e WHERE e.child_thread_id = threads.id)",
        );
    }
    if has_table(db, "agent_job_items")?
        && table_columns(db, "agent_job_items")?
            .iter()
            .any(|column| column == "assigned_thread_id")
    {
        subagent_filters.push(
            "NOT EXISTS (SELECT 1 FROM agent_job_items j WHERE j.assigned_thread_id = threads.id)",
        );
    }
    Ok(if subagent_filters.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", subagent_filters.join(" AND "))
    })
}

fn sqlite_limit(limit: usize) -> i64 {
    i64::try_from(limit).unwrap_or(i64::MAX)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalSession {
    pub id: String,
    pub title: String,
    pub cwd: String,
    pub model_provider: String,
    pub archived: bool,
    pub updated_at_ms: Option<i64>,
    pub rollout_path: String,
    pub db_path: String,
}

#[derive(Debug, Clone)]
struct OwnedSqlValue(SqlValue);

impl ToSql for OwnedSqlValue {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::Owned(self.0.clone()))
    }
}

impl SQLiteStorageAdapter {
    pub fn new(db_path: impl Into<PathBuf>, backup_store: BackupStore) -> Self {
        let db_path = db_path.into();
        Self {
            allowed_db_paths: vec![db_path.clone()],
            db_path,
            backup_store,
            codex_home: None,
        }
    }

    pub fn with_allowed_db_paths(mut self, db_paths: impl IntoIterator<Item = PathBuf>) -> Self {
        for db_path in db_paths {
            if !self.allowed_db_paths.contains(&db_path) {
                self.allowed_db_paths.push(db_path);
            }
        }
        self
    }

    pub fn with_codex_home(mut self, codex_home: impl Into<PathBuf>) -> Self {
        self.codex_home = Some(codex_home.into());
        self
    }

    /// issue #875：原报错只写「Database not found: <一条路径>」，用户拿不到任何线索。
    ///
    /// 现在把「用哪条路径、该路径在不在、还试过哪些候选路径」一并列出来，
    /// 让用户/我们能立刻分辨是路径解析错了还是库确实不在。
    fn database_not_found_message(&self) -> String {
        let mut lines = vec![format!(
            "Database not found: {} (exists: {})",
            self.db_path.to_string_lossy(),
            self.db_path.is_file()
        )];
        let others = self
            .allowed_db_paths
            .iter()
            .filter(|path| *path != &self.db_path);
        let others = others.collect::<Vec<_>>();
        if !others.is_empty() {
            lines.push(format!(
                "候选路径：{}",
                others
                    .iter()
                    .map(|path| format!("{} (exists: {})", path.to_string_lossy(), path.is_file()))
                    .collect::<Vec<_>>()
                    .join("、")
            ));
        }
        if let Some(home) = self.codex_home.as_deref() {
            lines.push(format!("Codex home：{}", home.to_string_lossy()));
        }
        lines.join("；")
    }

    pub fn delete_local(&self, session: &SessionRef) -> DeleteResult {
        self.delete_local_classified(session).0
    }

    fn delete_local_classified(&self, session: &SessionRef) -> (DeleteResult, DeleteDisposition) {
        match fs::metadata(&self.db_path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return (
                    failed(&session.session_id, self.database_not_found_message()),
                    DeleteDisposition::NotFound,
                );
            }
            Err(error) => {
                return (
                    failed(&session.session_id, error.to_string()),
                    DeleteDisposition::Attempted,
                );
            }
            Ok(_) => {}
        }
        let mut disposition = DeleteDisposition::Attempted;
        let result = (|| -> anyhow::Result<DeleteResult> {
            let mut db =
                Connection::open_with_flags(&self.db_path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
            let kind = schema_kind(&db)?
                .ok_or_else(|| anyhow::anyhow!("Unsupported local storage schema"))?;
            let thread_id = normalize_codex_thread_id(&session.session_id);
            let (table, column, id) = match kind {
                SchemaKind::GenericSessions => ("sessions", "id", session.session_id.as_str()),
                SchemaKind::CodexThreads => ("threads", "id", thread_id.as_str()),
                SchemaKind::CodexAutomationRuns => {
                    ("automation_runs", "thread_id", thread_id.as_str())
                }
            };
            let mut found = db
                .query_row(
                    &format!("SELECT 1 FROM {table} WHERE {column} = ?1 LIMIT 1"),
                    [id],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if !found && kind == SchemaKind::CodexAutomationRuns && has_table(&db, "inbox_items")? {
                found = db
                    .query_row(
                        "SELECT 1 FROM inbox_items WHERE thread_id = ?1 LIMIT 1",
                        [id],
                        |_| Ok(()),
                    )
                    .optional()?
                    .is_some();
            }
            if !found {
                disposition = DeleteDisposition::NotFound;
                return Ok(failed(
                    &session.session_id,
                    "Thread not found in local storage".to_string(),
                ));
            }
            match kind {
                SchemaKind::GenericSessions => self.delete_generic_session(&mut db, session),
                SchemaKind::CodexThreads => self.delete_codex_thread(&mut db, session),
                SchemaKind::CodexAutomationRuns => {
                    self.delete_codex_automation_run(&mut db, session)
                }
            }
        })();
        let mut result = result.unwrap_or_else(|err| failed(&session.session_id, err.to_string()));
        // 删成功就一并清 session_index.jsonl。
        //
        // 放在这个统一出口而不是各个 delete_* 里：三种 schema 里原先只有
        // delete_codex_thread 清了索引，另外两种删掉数据库行却把索引条目留着，
        // 于是重启后 UI 从索引读，会话又冒出来，再删再冒（#1979）。放在出口
        // 处理，将来加新 schema 也不会漏。
        //
        // delete_codex_thread 里那次调用保留：它需要把清理失败并进自己那条
        // 「数据库已删但文件删除失败」的消息里；这里对已清理过的再调一次是幂等的
        // （条目已不在，返回 0）。
        if matches!(result.status, DeleteStatus::LocalDeleted)
            && let Some(home) = self.codex_home.as_deref()
        {
            let thread_id = normalize_codex_thread_id(&session.session_id);
            if let Err(error) = crate::provider_sync::remove_session_index_entry(home, &thread_id) {
                result.status = DeleteStatus::Failed;
                result.message =
                    format!("{}；session_index.jsonl 清理失败：{error}", result.message);
            }
            if let Err(error) =
                crate::provider_sync::remove_thread_sidebar_references(home, &thread_id)
            {
                result.status = DeleteStatus::Failed;
                result.message = format!("{}；侧边栏索引清理失败：{error}", result.message);
            }
        }
        (result, disposition)
    }

    pub fn list_local_sessions(&self) -> anyhow::Result<Vec<LocalSession>> {
        self.list_local_sessions_limited(usize::MAX)
    }

    pub fn list_local_sessions_limited(&self, limit: usize) -> anyhow::Result<Vec<LocalSession>> {
        if !self.db_path.exists() {
            return Ok(Vec::new());
        }
        let db = Connection::open(&self.db_path)?;
        let query = session_paging::session_query(&db)?;
        let mut stmt = db.prepare(&format!(
            "{query} ORDER BY updated_at_ms DESC, id DESC LIMIT ?1"
        ))?;
        let rows = stmt.query_map([sqlite_limit(limit)], |row| {
            session_paging::read_session(row, &self.db_path)
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn list_local_session_ids(&self) -> anyhow::Result<Vec<String>> {
        if !self.db_path.exists() {
            return Ok(Vec::new());
        }
        let db = Connection::open(&self.db_path)?;
        let (table, id_column, filter) = match schema_kind(&db)? {
            Some(SchemaKind::CodexThreads) => ("threads", "id", codex_thread_filter(&db)?),
            Some(SchemaKind::CodexAutomationRuns) => (
                "automation_runs",
                "thread_id",
                "WHERE COALESCE(thread_id, '') <> ''".to_string(),
            ),
            _ => anyhow::bail!("Unsupported local storage schema"),
        };
        let sql = format!("SELECT {id_column} FROM {table} {filter} ORDER BY {id_column}");
        let mut stmt = db.prepare(&sql)?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn undo(&self, token: &str) -> DeleteResult {
        let result = (|| -> anyhow::Result<DeleteResult> {
            let backups = undo_backups(&self.backup_store, token)?;
            let session_id = backups[0]["session_id"].as_str().unwrap_or("").to_string();
            restore_backups(
                &backups,
                &self.db_path,
                &self.allowed_db_paths,
                self.codex_home.as_deref(),
                &self.backup_store,
            )?;
            Ok(DeleteResult {
                status: DeleteStatus::Undone,
                session_id,
                message: "Local session restored from backup".to_string(),
                undo_token: Some(token.to_string()),
                backup_path: None,
            })
        })();
        result.unwrap_or_else(|err| failed_with_undo("", err.to_string(), token, None))
    }

    pub fn find_archived_thread_by_title(&self, title: &str) -> Option<SessionRef> {
        let db = Connection::open(&self.db_path).ok()?;
        if schema_kind(&db).ok().flatten() != Some(SchemaKind::CodexThreads)
            || !has_columns(&db, "threads", &["archived"]).ok()?
        {
            return None;
        }
        let mut stmt = db
            .prepare(
                "SELECT id, title FROM threads
                 WHERE archived = 1 AND (title = ?1 OR title LIKE ?2 OR ?1 LIKE '%' || title || '%')
                 ORDER BY archived_at DESC LIMIT 1",
            )
            .ok()?;
        let mut rows = stmt.query((title, format!("%{title}%"))).ok()?;
        let row = rows.next().ok().flatten()?;
        let id: String = row.get(0).ok()?;
        let row_title: Option<String> = row.get(1).ok()?;
        SessionRef::new(id, row_title.unwrap_or_else(|| title.to_string())).ok()
    }

    /// 解析会话对应的 rollout 文件。
    ///
    /// `Unavailable` 里已经是可直接返回前端的失败对象：语义是「本地没有这个会话的
    /// 历史」，不是后端故障，所以不走 anyhow；真正的 IO / SQLite 故障走 Err。
    fn codex_thread_rollout(&self, session: &SessionRef) -> anyhow::Result<CodexThreadRollout> {
        let thread_id = normalize_codex_thread_id(&session.session_id);
        let mut errors = Vec::new();
        let mut missing = Vec::new();
        for candidate in &self.allowed_db_paths {
            let lookup = (|| -> anyhow::Result<Option<String>> {
                match fs::metadata(candidate) {
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        missing.push(format!("{}: database missing", candidate.display()));
                        return Ok(None);
                    }
                    Err(error) => return Err(error.into()),
                }
                let db = Connection::open_with_flags(candidate, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
                db.busy_timeout(std::time::Duration::from_millis(50))?;
                // has_table 的兼容探测会把错误转为 false；先显式识别坏库/IO，避免误报无目标。
                db.query_row("PRAGMA schema_version", [], |row| row.get::<_, i64>(0))?;
                if schema_kind(&db)? != Some(SchemaKind::CodexThreads)
                    || !has_columns(&db, "threads", &["rollout_path"])?
                {
                    missing.push(format!(
                        "{}: unsupported local storage schema",
                        candidate.display()
                    ));
                    return Ok(None);
                }
                let path = db
                    .query_row(
                        "SELECT rollout_path FROM threads WHERE id = ?1",
                        [&thread_id],
                        |row| row.get::<_, Option<String>>(0),
                    )
                    .optional()?
                    .flatten();
                if path.as_ref().is_none_or(|path| path.trim().is_empty()) {
                    missing.push(format!(
                        "{}: target thread or rollout path missing",
                        candidate.display()
                    ));
                    return Ok(None);
                }
                Ok(path)
            })();
            let path = match lookup {
                Ok(Some(path)) => path,
                Ok(None) => continue,
                Err(error) => {
                    errors.push(format!("{}: {error:#}", candidate.display()));
                    continue;
                }
            };
            for view in std::iter::once(path.clone()).chain(wsl_path_alternative(&path)) {
                let rollout = PathBuf::from(&view);
                match fs::metadata(&rollout) {
                    Ok(metadata) if metadata.is_file() => match File::open(&rollout) {
                        Ok(_) => {
                            return Ok(CodexThreadRollout::Ready {
                                thread_id,
                                rollout_path: view,
                                rollout,
                            });
                        }
                        Err(error) => errors.push(format!("{}: {error}", rollout.display())),
                    },
                    Ok(_) => {
                        missing.push(format!("{}: rollout is not a file", candidate.display()))
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        missing.push(format!("rollout file not found: {view}"))
                    }
                    Err(error) => errors.push(format!("{}: {error}", rollout.display())),
                }
            }
        }
        if !errors.is_empty() {
            anyhow::bail!(
                "No readable rollout found for thread {thread_id}; candidate errors: {}",
                errors.join("; ")
            );
        }
        Ok(CodexThreadRollout::Unavailable(json!({
            "status": "failed", "session_id": thread_id,
            "message": format!("Thread rollout unavailable: {}; {}", missing.join("; "), self.database_not_found_message()),
        })))
    }

    pub fn codex_thread_usage_history(&self, session: &SessionRef) -> serde_json::Value {
        let with_history = |mut value: Value| {
            if let Some(object) = value.as_object_mut() {
                object.insert("history".to_string(), json!([]));
            }
            value
        };
        match self.codex_thread_rollout(session) {
            Err(err) => with_history(json!({
                "status": "failed",
                "session_id": session.session_id,
                "message": err.to_string(),
            })),
            Ok(CodexThreadRollout::Unavailable(value)) => with_history(value),
            Ok(CodexThreadRollout::Ready {
                thread_id,
                rollout_path,
                rollout,
            }) => match read_rollout_usage_history(&rollout, &thread_id) {
                Ok(history) => json!({
                    "status": "ok",
                    "session_id": thread_id,
                    "rollout_path": rollout_path,
                    "history": history,
                }),
                Err(err) => with_history(json!({
                    "status": "failed",
                    "session_id": session.session_id,
                    "message": err.to_string(),
                })),
            },
        }
    }

    /// Codex 输入框下方状态栏用的轻量用量快照。
    ///
    /// 与 `codex_thread_usage_history` 的关键差别是**只读 rollout 尾部**：状态栏按
    /// 秒级轮询，而真实会话的 rollout 可以到几百 MB，整读整解析会把 spawn_blocking
    /// 线程拖住（同一个 adapter 还要服务删除、导出）。尾部窗口拿不到足够事件时按倍数
    /// 放大，封顶后直接返回已有事件，不做全量兜底。
    pub fn codex_thread_usage_live(&self, session: &SessionRef) -> serde_json::Value {
        let with_events = |mut value: Value| {
            if let Some(object) = value.as_object_mut() {
                object.insert("events".to_string(), json!([]));
            }
            value
        };
        match self.codex_thread_rollout(session) {
            Err(err) => with_events(json!({
                "status": "failed",
                "session_id": session.session_id,
                "message": err.to_string(),
            })),
            Ok(CodexThreadRollout::Unavailable(value)) => with_events(value),
            Ok(CodexThreadRollout::Ready {
                thread_id,
                rollout_path,
                rollout,
            }) => match read_rollout_usage_tail(&rollout, USAGE_TAIL_EVENT_LIMIT) {
                Ok(events) => json!({
                    "status": "ok",
                    "session_id": thread_id,
                    "rollout_path": rollout_path,
                    "events": events,
                }),
                Err(err) => with_events(json!({
                    "status": "failed",
                    "session_id": session.session_id,
                    "message": err.to_string(),
                })),
            },
        }
    }

    fn delete_generic_session(
        &self,
        db: &mut Connection,
        session: &SessionRef,
    ) -> anyhow::Result<DeleteResult> {
        let sessions = select_dicts(
            db,
            "SELECT * FROM sessions WHERE id = ?1",
            &[&session.session_id],
        )?;
        if sessions.is_empty() {
            return Ok(failed(
                &session.session_id,
                "Session not found in local storage".to_string(),
            ));
        }
        let messages = if has_table(db, "messages")? {
            select_dicts(
                db,
                "SELECT * FROM messages WHERE session_id = ?1",
                &[&session.session_id],
            )?
        } else {
            Vec::new()
        };
        let mut tables = Map::new();
        tables.insert("sessions".to_string(), Value::Array(sessions));
        tables.insert("messages".to_string(), Value::Array(messages));
        self.add_thread_sidebar_backups(&mut tables, &session.session_id)?;
        let token = self.backup_store.write_backup(
            &session.session_id,
            &self.db_path,
            Value::Object(tables),
        )?;
        let backup_path = self.backup_store.path_for(&token);
        let delete_result = (|| -> anyhow::Result<()> {
            let tx = db.transaction()?;
            if has_table(&tx, "messages")? {
                tx.execute(
                    "DELETE FROM messages WHERE session_id = ?1",
                    [&session.session_id],
                )?;
            }
            tx.execute("DELETE FROM sessions WHERE id = ?1", [&session.session_id])?;
            tx.commit()?;
            Ok(())
        })();
        if let Err(err) = delete_result {
            return Ok(failed_with_undo(
                &session.session_id,
                err.to_string(),
                &token,
                Some(&backup_path),
            ));
        }
        Ok(local_deleted(&session.session_id, &token, &backup_path))
    }

    fn delete_codex_thread(
        &self,
        db: &mut Connection,
        session: &SessionRef,
    ) -> anyhow::Result<DeleteResult> {
        let thread_id = normalize_codex_thread_id(&session.session_id);
        let thread_rows = select_dicts(db, "SELECT * FROM threads WHERE id = ?1", &[&thread_id])?;
        if thread_rows.is_empty() {
            return Ok(failed(
                &session.session_id,
                "Thread not found in local storage".to_string(),
            ));
        }
        let mut tables = Map::new();
        tables.insert("threads".to_string(), Value::Array(thread_rows));
        backup_related_rows(
            db,
            &mut tables,
            "thread_dynamic_tools",
            "thread_id = ?1",
            &[&thread_id],
        )?;
        backup_related_rows(
            db,
            &mut tables,
            "thread_goals",
            "thread_id = ?1",
            &[&thread_id],
        )?;
        backup_related_rows(
            db,
            &mut tables,
            "thread_spawn_edges",
            "parent_thread_id = ?1 OR child_thread_id = ?1",
            &[&thread_id],
        )?;
        backup_related_rows(
            db,
            &mut tables,
            "stage1_outputs",
            "thread_id = ?1",
            &[&thread_id],
        )?;
        backup_related_rows(
            db,
            &mut tables,
            "agent_job_items",
            "assigned_thread_id = ?1",
            &[&thread_id],
        )?;
        let (file_sources, unreadable_rollouts) =
            rollout_file_sources(tables.get("threads").and_then(Value::as_array));
        if !unreadable_rollouts.is_empty() {
            anyhow::bail!(
                "rollout backup unreadable; database and original files were not deleted: {}",
                unreadable_rollouts.join("; ")
            );
        }
        self.add_thread_sidebar_backups(&mut tables, &thread_id)?;
        let (token, file_backups) = self.backup_store.write_backup_with_files(
            &thread_id,
            &self.db_path,
            Value::Object(tables),
            &file_sources,
        )?;
        let backup_path = self.backup_store.path_for(&token);
        let delete_result = (|| -> anyhow::Result<()> {
            for file in &file_backups {
                self.backup_store.source_file_unchanged(file)?;
            }
            let tx = db.transaction()?;
            delete_related_rows(&tx, "thread_dynamic_tools", "thread_id = ?1", &[&thread_id])?;
            delete_related_rows(&tx, "thread_goals", "thread_id = ?1", &[&thread_id])?;
            delete_related_rows(
                &tx,
                "thread_spawn_edges",
                "parent_thread_id = ?1 OR child_thread_id = ?1",
                &[&thread_id],
            )?;
            delete_related_rows(&tx, "stage1_outputs", "thread_id = ?1", &[&thread_id])?;
            if has_table(&tx, "agent_job_items")?
                && has_columns(&tx, "agent_job_items", &["assigned_thread_id"])?
            {
                tx.execute(
                    "UPDATE agent_job_items SET assigned_thread_id = NULL WHERE assigned_thread_id = ?1",
                    [&thread_id],
                )?;
            }
            tx.execute("DELETE FROM threads WHERE id = ?1", [&thread_id])?;
            tx.commit()?;
            Ok(())
        })();
        if let Err(err) = delete_result {
            return Ok(failed_with_undo(
                &thread_id,
                err.to_string(),
                &token,
                Some(&backup_path),
            ));
        }
        // issue #162：rollout 文件存在但两种视角都读取失败时，不允许静默报删除成功
        let mut file_errors = unreadable_rollouts
            .into_iter()
            .map(|path| format!("{path}: 读取失败，未删除"))
            .collect::<Vec<_>>();
        for file in file_backups {
            if let Some(path) = file.get("path").and_then(Value::as_str) {
                if let Err(error) = self.backup_store.source_file_unchanged(&file) {
                    file_errors.push(error.to_string());
                    continue;
                }
                if let Err(err) = fs::remove_file(path) {
                    if err.kind() != std::io::ErrorKind::NotFound {
                        file_errors.push(format!("{path}: {err}"));
                    }
                }
            }
        }
        let session_index_note = self.codex_home.as_deref().and_then(|home| {
            crate::provider_sync::remove_session_index_entry(home, &thread_id)
                .err()
                .map(|error| format!("session_index.jsonl 清理失败：{error}"))
        });
        if !file_errors.is_empty() {
            let mut message = format!(
                "本地数据库已删除，但文件删除失败：{}",
                file_errors.join("; ")
            );
            if let Some(note) = session_index_note.as_deref() {
                message = format!("{message}；{note}");
            }
            return Ok(DeleteResult {
                status: DeleteStatus::Failed,
                session_id: thread_id,
                message,
                undo_token: Some(token.clone()),
                backup_path: Some(backup_path.to_string_lossy().to_string()),
            });
        }
        let mut result = local_deleted(&thread_id, &token, &backup_path);
        if let Some(note) = session_index_note.as_deref() {
            result.message = format!("{}；{}", result.message, note);
        }
        Ok(result)
    }

    fn add_thread_sidebar_backups(
        &self,
        tables: &mut Map<String, Value>,
        thread_id: &str,
    ) -> anyhow::Result<()> {
        let Some(home) = self.codex_home.as_deref() else {
            return Ok(());
        };
        let session_index_lines =
            crate::provider_sync::session_index_lines_for_thread(home, thread_id)?;
        if !session_index_lines.is_empty() {
            tables.insert(
                "__session_index".to_string(),
                Value::Array(session_index_lines.into_iter().map(Value::String).collect()),
            );
        }
        tables.insert(
            "__sidebar".to_string(),
            crate::provider_sync::snapshot_thread_sidebar_references(home, thread_id)?,
        );
        Ok(())
    }

    fn delete_codex_automation_run(
        &self,
        db: &mut Connection,
        session: &SessionRef,
    ) -> anyhow::Result<DeleteResult> {
        let thread_id = normalize_codex_thread_id(&session.session_id);
        let mut tables = Map::new();
        backup_related_rows(
            db,
            &mut tables,
            "automation_runs",
            "thread_id = ?1",
            &[&thread_id],
        )?;
        backup_related_rows(
            db,
            &mut tables,
            "inbox_items",
            "thread_id = ?1",
            &[&thread_id],
        )?;
        self.add_thread_sidebar_backups(&mut tables, &thread_id)?;
        if tables.values().all(|rows| {
            rows.as_array()
                .map(|items| items.is_empty())
                .unwrap_or(true)
        }) {
            return Ok(failed(
                &session.session_id,
                "Thread not found in local storage".to_string(),
            ));
        }
        let token =
            self.backup_store
                .write_backup(&thread_id, &self.db_path, Value::Object(tables))?;
        let backup_path = self.backup_store.path_for(&token);
        let delete_result = (|| -> anyhow::Result<()> {
            let tx = db.transaction()?;
            delete_related_rows(&tx, "automation_runs", "thread_id = ?1", &[&thread_id])?;
            delete_related_rows(&tx, "inbox_items", "thread_id = ?1", &[&thread_id])?;
            tx.commit()?;
            Ok(())
        })();
        if let Err(err) = delete_result {
            return Ok(failed_with_undo(
                &thread_id,
                err.to_string(),
                &token,
                Some(&backup_path),
            ));
        }
        Ok(local_deleted(&thread_id, &token, &backup_path))
    }
}

fn optional_column_expression<'a>(
    columns: &HashSet<String>,
    column: &'a str,
    fallback: &'a str,
) -> &'a str {
    if columns.contains(column) {
        column
    } else {
        fallback
    }
}

fn failed(session_id: &str, message: String) -> DeleteResult {
    DeleteResult {
        status: DeleteStatus::Failed,
        session_id: session_id.to_string(),
        message,
        undo_token: None,
        backup_path: None,
    }
}

fn local_deleted(session_id: &str, token: &str, backup_path: &Path) -> DeleteResult {
    DeleteResult {
        status: DeleteStatus::LocalDeleted,
        session_id: session_id.to_string(),
        message: "已从本地存储删除".to_string(),
        undo_token: Some(token.to_string()),
        backup_path: Some(backup_path.to_string_lossy().to_string()),
    }
}

fn read_rollout_usage_history(rollout_path: &Path, thread_id: &str) -> anyhow::Result<Vec<Value>> {
    let file = File::open(rollout_path)?;
    let reader = BufReader::new(file);
    let mut current_turn_id = String::new();
    let mut history = Vec::new();

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => continue,
        };
        match value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
        {
            "turn_context" => {
                current_turn_id = value
                    .get("payload")
                    .and_then(|payload| payload.get("turn_id"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
            }
            "event_msg" => {
                let payload = match value.get("payload") {
                    Some(payload)
                        if payload.get("type").and_then(Value::as_str) == Some("token_count") =>
                    {
                        payload
                    }
                    _ => continue,
                };
                let info = match payload.get("info") {
                    Some(info) => info,
                    None => continue,
                };
                let last = info.get("last_token_usage");
                let total = info.get("total_token_usage");
                let model_context_window = info
                    .get("model_context_window")
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                let input_tokens = last
                    .and_then(|usage| usage.get("input_tokens"))
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                let output_tokens = last
                    .and_then(|usage| usage.get("output_tokens"))
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                let total_tokens = last
                    .and_then(|usage| usage.get("total_tokens"))
                    .and_then(Value::as_i64)
                    .unwrap_or_else(|| {
                        total
                            .and_then(|usage| usage.get("total_tokens"))
                            .and_then(Value::as_i64)
                            .unwrap_or(0)
                    });
                let cached_tokens = last
                    .and_then(|usage| usage.get("cached_input_tokens"))
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                let context_used = total
                    .and_then(|usage| usage.get("total_tokens"))
                    .and_then(Value::as_i64)
                    .unwrap_or(total_tokens);
                if input_tokens <= 0 && output_tokens <= 0 && total_tokens <= 0 && context_used <= 0
                {
                    continue;
                }
                history.push(json!({
                    "source": "rollout-history",
                    "conversation_id": format!("local:{thread_id}"),
                    "turn_id": current_turn_id,
                    "observed_at": value.get("timestamp").and_then(Value::as_str).unwrap_or_default(),
                    "usage": {
                        "inputTokens": input_tokens,
                        "outputTokens": output_tokens,
                        "totalTokens": total_tokens,
                        "cachedTokens": cached_tokens,
                        "cacheReadTokens": 0,
                        "cacheCreationTokens": 0,
                        "contextUsed": context_used,
                        "contextLimit": model_context_window,
                        "hasBreakdown": input_tokens > 0 || output_tokens > 0 || cached_tokens > 0,
                    }
                }));
            }
            _ => {}
        }
    }

    Ok(history)
}

/// `SQLiteStorageAdapter::codex_thread_rollout` 的结果。
enum CodexThreadRollout {
    Ready {
        thread_id: String,
        rollout_path: String,
        rollout: PathBuf,
    },
    /// 已经可以当作路由返回值的失败对象（本地没有这个会话的历史）。
    Unavailable(Value),
}

/// 状态栏尾部读取的初始窗口。
const USAGE_TAIL_INITIAL_BYTES: u64 = 512 * 1024;
/// 窗口放大上限：单条工具输出动辄几百 KB，一条就能把整个窗口占满。
const USAGE_TAIL_MAX_BYTES: u64 = 8 * 1024 * 1024;
/// 返回给渲染侧的 token_count 事件条数上限，够算速度与最近一轮用量即可。
const USAGE_TAIL_EVENT_LIMIT: usize = 24;

/// 只读 rollout 尾部，取出最近的 `token_count` 事件。
fn read_rollout_usage_tail(rollout: &Path, limit: usize) -> anyhow::Result<Vec<Value>> {
    let file = File::open(rollout)?;
    let len = file.metadata()?.len();
    let mut window = USAGE_TAIL_INITIAL_BYTES;
    loop {
        let start = len.saturating_sub(window);
        let mut events = Vec::new();
        read_rollout_usage_window(&file, start, limit, &mut events)?;
        // 至少两条事件渲染侧才算得出生成速度。还要找到真实轮边界：
        // total_token_usage 是会话累计，不能靠输出计数回落判断换轮。
        let latest_turn_known = events
            .last()
            .and_then(|event| event.get("turn_id"))
            .and_then(Value::as_str)
            .is_some();
        // 到顶或已经从头开始读时不再尝试，避免退化成整文件解析。
        if events.len() >= 2 && latest_turn_known || start == 0 || window >= USAGE_TAIL_MAX_BYTES {
            return Ok(events);
        }
        window = (window * 4).min(USAGE_TAIL_MAX_BYTES);
    }
}

fn read_rollout_usage_window(
    file: &File,
    start: u64,
    limit: usize,
    out: &mut Vec<Value>,
) -> anyhow::Result<()> {
    let mut handle = file.try_clone()?;
    handle.seek(SeekFrom::Start(start))?;
    let mut buffer = Vec::new();
    handle.read_to_end(&mut buffer)?;
    // 起始处多半落在行中间；UTF-8 的续字节不会等于 b'\n'，按字节切行不会切坏字符。
    let text = String::from_utf8_lossy(&buffer);
    let mut lines = text.split('\n');
    if start > 0 {
        lines.next();
    }
    let mut current_turn_id: Option<String> = None;
    let mut segment_start = 0;
    for line in lines {
        let line = line.trim();
        // 先做子串预筛，避免为每条 response_item 付 serde 解析成本。
        if line.is_empty()
            || !["\"event_msg\"", "\"turn_context\""]
                .iter()
                .any(|kind| line.contains(kind))
        {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let kind = value.get("type").and_then(Value::as_str);
        let payload = &value["payload"];
        let event_type = payload.get("type").and_then(Value::as_str);
        let turn_id = payload
            .get("turn_id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty());
        if kind == Some("turn_context") {
            // 老版本上下文没有 id 时，保留本轮 task_started 已经确认的归属。
            if let Some(id) = turn_id {
                if current_turn_id.as_deref() != Some(id) {
                    segment_start = out.len();
                    current_turn_id = Some(id.to_owned());
                }
            }
            continue;
        }
        if kind != Some("event_msg") {
            continue;
        }
        match event_type {
            // Codex v1 持久化名称为 task_*，turn_* 是新版协议的别名。
            Some("task_started" | "turn_started") => {
                segment_start = out.len();
                current_turn_id = turn_id.map(str::to_owned);
                continue;
            }
            Some(
                "task_complete" | "turn_complete" | "turn_completed" | "task_failed"
                | "turn_aborted" | "task_aborted",
            ) => {
                // 起始标记可能在尾部窗口之外，结束标记仍可确认它前面的用量归属。
                if let Some(id) = turn_id {
                    for event in &mut out[segment_start..] {
                        if event.get("turn_id").is_none() {
                            event["turn_id"] = json!(id);
                        }
                    }
                }
                segment_start = out.len();
                current_turn_id = None;
                continue;
            }
            _ => {}
        }
        if let Some(event) = rollout_usage_event(&value, current_turn_id.as_deref()) {
            out.push(event);
        }
    }
    if out.len() > limit {
        out.drain(..out.len() - limit);
    }
    Ok(())
}

fn rollout_usage_event(value: &Value, turn_id: Option<&str>) -> Option<Value> {
    if value.get("type").and_then(Value::as_str) != Some("event_msg") {
        return None;
    }
    let payload = value.get("payload")?;
    if payload.get("type").and_then(Value::as_str) != Some("token_count") {
        return None;
    }
    let info = payload.get("info")?;
    let last = info.get("last_token_usage")?;
    let total = info.get("total_token_usage");
    let field = |source: Option<&Value>, key: &str| {
        source
            .and_then(|usage| usage.get(key))
            .and_then(Value::as_i64)
            .unwrap_or(0)
    };
    let input_tokens = field(Some(last), "input_tokens");
    let output_tokens = field(Some(last), "output_tokens");
    let total_tokens = field(Some(last), "total_tokens");
    if input_tokens <= 0 && output_tokens <= 0 && total_tokens <= 0 {
        return None;
    }
    let mut event = json!({
        "t": value.get("timestamp").and_then(Value::as_str).unwrap_or_default(),
        // 本次模型调用的用量：上下文占用取 total_tokens / context_window。
        "input_tokens": input_tokens,
        "cached_input_tokens": field(Some(last), "cached_input_tokens"),
        "output_tokens": output_tokens,
        "reasoning_output_tokens": field(Some(last), "reasoning_output_tokens"),
        "total_tokens": total_tokens,
        "context_window": field(Some(info), "model_context_window"),
        // 历史字段名，实际是会话累计输出；必须结合 turn_id 才能计算同轮速度。
        "turn_output_tokens": field(total, "output_tokens"),
    });
    if let Some(id) = turn_id {
        event["turn_id"] = json!(id);
    }
    Some(event)
}

fn failed_with_undo(
    session_id: &str,
    message: String,
    token: &str,
    backup_path: Option<&Path>,
) -> DeleteResult {
    DeleteResult {
        status: DeleteStatus::Failed,
        session_id: session_id.to_string(),
        message,
        undo_token: Some(token.to_string()),
        backup_path: backup_path.map(|path| path.to_string_lossy().to_string()),
    }
}

fn normalize_codex_thread_id(session_id: &str) -> String {
    session_id
        .strip_prefix("local:")
        .unwrap_or(session_id)
        .to_string()
}

fn undo_backups(backup_store: &BackupStore, token: &str) -> anyhow::Result<Vec<Value>> {
    let tokens =
        serde_json::from_str::<Vec<String>>(token).unwrap_or_else(|_| vec![token.to_string()]);
    if tokens.is_empty() {
        anyhow::bail!("empty undo token");
    }
    tokens
        .into_iter()
        .map(|token| backup_store.read_backup(&token))
        .collect()
}

fn restore_backups(
    backups: &[Value],
    fallback_db_path: &Path,
    allowed_db_paths: &[PathBuf],
    codex_home: Option<&Path>,
    backup_store: &BackupStore,
) -> anyhow::Result<()> {
    // 所有清单先完整校验；损坏的后续备份不能在前一份已经恢复后才被发现。
    for backup in backups {
        validate_restore_backup(backup)?;
    }
    for backup in backups {
        let tables = backup["tables"].as_object().unwrap();
        let source_db = backup_source_db(backup, fallback_db_path, allowed_db_paths)?;
        let db = Connection::open_with_flags(&source_db, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        detect_restore_conflicts(&db, tables)?;
        detect_file_restore_conflicts(
            tables,
            backup_store,
            backup["token"].as_str().unwrap_or(""),
        )?;
        preflight_restore_rows(&db, tables)?;
        if let Some(sidebar) = tables.get("__sidebar") {
            let home = codex_home
                .ok_or_else(|| anyhow::anyhow!("sidebar restore requires a Codex home"))?;
            crate::provider_sync::validate_thread_sidebar_snapshot(home, sidebar)?;
        }
        if let Some(entries) = tables.get("__session_index") {
            let home = codex_home
                .ok_or_else(|| anyhow::anyhow!("session index restore requires a Codex home"))?;
            let lines = entries
                .as_array()
                .unwrap()
                .iter()
                .map(|line| line.as_str().unwrap().to_string())
                .collect::<Vec<_>>();
            crate::provider_sync::preflight_restore_session_index_entries(home, &lines)?;
        }
    }

    for backup in backups {
        let tables = backup["tables"].as_object().unwrap();
        let source_db = backup_source_db(backup, fallback_db_path, allowed_db_paths)?;
        let mut db = Connection::open_with_flags(&source_db, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        let tx = db.transaction()?;
        restore_rows(&tx, tables)?;
        tx.commit()?;
        if let Some(files) = tables.get("__files").and_then(Value::as_array) {
            for file in files {
                backup_store.restore_file(backup["token"].as_str().unwrap_or(""), file)?;
            }
        }
        if let Some(entries) = tables.get("__session_index").and_then(Value::as_array) {
            let lines = entries
                .iter()
                .map(|line| line.as_str().unwrap().to_string())
                .collect::<Vec<_>>();
            if !lines.is_empty() {
                if let Some(home) = codex_home {
                    crate::provider_sync::restore_session_index_entries(home, &lines)?;
                }
            }
        }
        if let Some(sidebar) = tables.get("__sidebar") {
            if let Some(home) = codex_home {
                let _ = crate::provider_sync::restore_thread_sidebar_references(home, sidebar)?;
            }
        }
    }
    Ok(())
}

fn preflight_restore_rows(db: &Connection, tables: &Map<String, Value>) -> anyhow::Result<()> {
    db.execute_batch("SAVEPOINT codex_plus_restore_preflight")?;
    let restore_result = restore_rows(db, tables);
    let rollback_result = db.execute_batch(
        "ROLLBACK TO codex_plus_restore_preflight; RELEASE codex_plus_restore_preflight",
    );
    restore_result?;
    rollback_result?;
    Ok(())
}

fn restore_rows(db: &Connection, tables: &Map<String, Value>) -> anyhow::Result<()> {
    for (table, rows) in tables {
        if table.starts_with("__") {
            continue;
        }
        let rows = rows
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("restore table {table} must be an array"))?;
        for row in rows {
            let row = row
                .as_object()
                .ok_or_else(|| anyhow::anyhow!("restore table {table} row must be an object"))?;
            if table == "agent_job_items" && update_existing_agent_job_item(db, row)? {
                continue;
            }
            insert_row(db, table, row)?;
        }
    }
    Ok(())
}

fn backup_source_db(
    backup: &Value,
    fallback_db_path: &Path,
    allowed_db_paths: &[PathBuf],
) -> anyhow::Result<PathBuf> {
    let source_db = backup["source_db"]
        .as_str()
        .filter(|path| !path.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| fallback_db_path.to_path_buf());
    if !source_db.is_file() {
        anyhow::bail!(
            "Backup source database not found: {}",
            source_db.to_string_lossy()
        );
    }
    let source_db = fs::canonicalize(source_db)?;
    let allowed = allowed_db_paths
        .iter()
        .filter_map(|path| fs::canonicalize(path).ok())
        .any(|path| path == source_db);
    if !allowed {
        anyhow::bail!("Backup source database is not an allowed local storage path");
    }
    Ok(source_db)
}

fn schema_kind(db: &Connection) -> anyhow::Result<Option<SchemaKind>> {
    if has_table(db, "sessions")? && has_columns(db, "sessions", &["id", "title"])? {
        if has_table(db, "messages")? && !has_columns(db, "messages", &["session_id"])? {
            return Ok(None);
        }
        return Ok(Some(SchemaKind::GenericSessions));
    }
    if has_table(db, "threads")? && has_columns(db, "threads", &["id", "title", "rollout_path"])? {
        return Ok(Some(SchemaKind::CodexThreads));
    }
    if has_table(db, "automation_runs")? && has_columns(db, "automation_runs", &["thread_id"])? {
        return Ok(Some(SchemaKind::CodexAutomationRuns));
    }
    Ok(None)
}

pub(crate) fn has_table(db: &Connection, table: &str) -> anyhow::Result<bool> {
    Ok(db
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [table],
            |_| Ok(()),
        )
        .is_ok())
}

fn has_columns(db: &Connection, table: &str, columns: &[&str]) -> anyhow::Result<bool> {
    let existing: HashSet<String> = table_columns(db, table)?.into_iter().collect();
    Ok(columns.iter().all(|column| existing.contains(*column)))
}

fn table_columns(db: &Connection, table: &str) -> anyhow::Result<Vec<String>> {
    let mut stmt = db.prepare(&format!(
        "PRAGMA table_info(\"{}\")",
        table.replace('"', "\"\"")
    ))?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub(crate) fn select_dicts(
    db: &Connection,
    sql: &str,
    params: &[&dyn ToSql],
) -> anyhow::Result<Vec<Value>> {
    let mut stmt = db.prepare(sql)?;
    let columns: Vec<String> = stmt
        .column_names()
        .iter()
        .map(|name| name.to_string())
        .collect();
    let rows = stmt.query_map(params, |row| {
        let mut data = Map::new();
        for (index, column) in columns.iter().enumerate() {
            data.insert(column.clone(), sql_value_to_json(row.get_ref(index)?));
        }
        Ok(Value::Object(data))
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn validate_restore_tables(tables: &Map<String, Value>) -> anyhow::Result<()> {
    let allowed = [
        "sessions",
        "messages",
        "threads",
        "thread_dynamic_tools",
        "thread_goals",
        "thread_spawn_edges",
        "stage1_outputs",
        "agent_job_items",
        "automation_runs",
        "inbox_items",
        "__files",
        "__session_index",
        "__sidebar",
    ];
    let mut has_content = false;
    for (table, value) in tables {
        if !allowed.contains(&table.as_str()) {
            anyhow::bail!("unknown restore table: {table}");
        }
        match table.as_str() {
            "__files" => {
                let files = value
                    .as_array()
                    .ok_or_else(|| anyhow::anyhow!("backup __files must be an array"))?;
                for file in files {
                    if file
                        .get("path")
                        .and_then(Value::as_str)
                        .is_none_or(|path| path.trim().is_empty())
                    {
                        anyhow::bail!("backup file is missing path");
                    }
                    if let Some(content) = file.get("content_b64") {
                        if !content.is_string() {
                            anyhow::bail!("legacy backup file content_b64 must be a string");
                        }
                    } else if file
                        .get("blob_ref")
                        .and_then(Value::as_str)
                        .is_none_or(|path| path.is_empty())
                        || file.get("size_bytes").and_then(Value::as_u64).is_none()
                        || file
                            .get("sha256")
                            .and_then(Value::as_str)
                            .is_none_or(|hash| {
                                hash.len() != 64
                                    || !hash.bytes().all(|byte| byte.is_ascii_hexdigit())
                            })
                    {
                        anyhow::bail!("backup file is missing valid blob metadata");
                    }
                    if file
                        .get("unix_mode")
                        .is_some_and(|mode| mode.as_u64().is_none())
                    {
                        anyhow::bail!("backup file unix_mode must be an unsigned integer");
                    }
                }
                has_content |= !files.is_empty();
            }
            "__session_index" => {
                let entries = value
                    .as_array()
                    .ok_or_else(|| anyhow::anyhow!("backup __session_index must be an array"))?;
                if entries
                    .iter()
                    .any(|line| line.as_str().is_none_or(|line| line.trim().is_empty()))
                {
                    anyhow::bail!("backup session index entries must be nonempty strings");
                }
                has_content |= !entries.is_empty();
            }
            "__sidebar" => has_content |= validate_sidebar_restore_shape(value)?,
            _ => {
                let rows = value
                    .as_array()
                    .ok_or_else(|| anyhow::anyhow!("backup table {table} must be an array"))?;
                for row in rows {
                    let row = row.as_object().ok_or_else(|| {
                        anyhow::anyhow!("backup table {table} row must be an object")
                    })?;
                    if row.is_empty()
                        || row
                            .values()
                            .any(|value| value.is_array() || value.is_object())
                    {
                        anyhow::bail!("backup table {table} row must contain scalar SQL values");
                    }
                }
                has_content |= !rows.is_empty();
            }
        }
    }
    if !has_content {
        anyhow::bail!("backup has no recoverable content");
    }
    Ok(())
}

fn validate_restore_backup(backup: &Value) -> anyhow::Result<()> {
    if backup
        .get("session_id")
        .and_then(Value::as_str)
        .is_none_or(|id| id.trim().is_empty())
    {
        anyhow::bail!("backup is missing session_id");
    }
    if backup
        .get("version")
        .is_some_and(|version| !matches!(version.as_u64(), Some(1 | 2)))
    {
        anyhow::bail!("unsupported deletion backup version");
    }
    let tables = backup
        .get("tables")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow::anyhow!("backup tables must be an object"))?;
    validate_restore_tables(tables)
}

fn validate_sidebar_restore_shape(value: &Value) -> anyhow::Result<bool> {
    let object = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("backup __sidebar must be an object"))?;
    if object
        .get("thread_id")
        .and_then(Value::as_str)
        .is_none_or(|id| id.trim().is_empty())
    {
        anyhow::bail!("sidebar snapshot is missing thread_id");
    }
    let mut has_content = false;
    if let Some(global) = object.get("global_state") {
        let global = global
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("sidebar global_state must be an object"))?;
        for (key, entry) in global {
            match key.as_str() {
                "projectless-thread-ids" => {
                    let ids = entry.as_array().ok_or_else(|| {
                        anyhow::anyhow!("sidebar projectless-thread-ids must be an array")
                    })?;
                    if ids.iter().any(|id| !id.is_string()) {
                        anyhow::bail!("sidebar thread ids must be strings");
                    }
                    has_content |= !ids.is_empty();
                }
                "thread_maps" => {
                    let maps = entry
                        .as_object()
                        .ok_or_else(|| anyhow::anyhow!("sidebar thread_maps must be an object"))?;
                    for map in maps.values() {
                        let map = map.as_object().ok_or_else(|| {
                            anyhow::anyhow!("sidebar thread map entries must be objects")
                        })?;
                        has_content |= !map.is_empty();
                    }
                }
                "atom_entries" => {
                    let entries = entry
                        .as_object()
                        .ok_or_else(|| anyhow::anyhow!("sidebar atom_entries must be an object"))?;
                    has_content |= !entries.is_empty();
                }
                _ => anyhow::bail!("unknown sidebar snapshot field: {key}"),
            }
        }
    }
    if let Some(catalog) = object.get("catalog") {
        let entries = catalog
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("sidebar catalog must be an array"))?;
        for entry in entries {
            let rows = entry
                .get("rows")
                .and_then(Value::as_array)
                .ok_or_else(|| anyhow::anyhow!("sidebar catalog rows must be an array"))?;
            if rows.iter().any(|row| !row.is_object()) {
                anyhow::bail!("sidebar catalog rows must be objects");
            }
            has_content |= !rows.is_empty();
        }
    }
    Ok(has_content)
}

fn detect_restore_conflicts(db: &Connection, tables: &Map<String, Value>) -> anyhow::Result<()> {
    for (table, rows) in tables {
        if table.starts_with("__") {
            continue;
        }
        let Some(rows) = rows.as_array() else {
            continue;
        };
        for row in rows {
            let Some(row) = row.as_object() else {
                continue;
            };
            if restore_row_conflicts(db, table, row)? {
                anyhow::bail!("restore conflict: {table} row already exists");
            }
        }
    }
    Ok(())
}

fn restore_row_conflicts(
    db: &Connection,
    table: &str,
    row: &Map<String, Value>,
) -> anyhow::Result<bool> {
    let key_columns = restore_conflict_key_columns(table, row);
    if key_columns.is_empty() || !has_table(db, table)? {
        return Ok(false);
    }
    let where_clause = key_columns
        .iter()
        .enumerate()
        .map(|(index, column)| format!("\"{}\" = ?{}", column.replace('"', "\"\""), index + 1))
        .collect::<Vec<_>>()
        .join(" AND ");
    let values = key_columns
        .iter()
        .map(|column| OwnedSqlValue(json_to_sql_value(&row[*column])))
        .collect::<Vec<_>>();
    let refs = values
        .iter()
        .map(|value| value as &dyn ToSql)
        .collect::<Vec<_>>();
    Ok(db
        .query_row(
            &format!("SELECT 1 FROM \"{table}\" WHERE {where_clause} LIMIT 1"),
            refs.as_slice(),
            |_| Ok(()),
        )
        .is_ok())
}

fn restore_conflict_key_columns<'a>(table: &str, row: &'a Map<String, Value>) -> Vec<&'a String> {
    let wanted: &[&str] = match table {
        "sessions" | "threads" => &["id"],
        "messages" => &["id"],
        "automation_runs" | "inbox_items" => &["thread_id"],
        "thread_dynamic_tools" => &["thread_id", "tool_name"],
        "thread_goals" => &["thread_id", "goal"],
        "thread_spawn_edges" => &["parent_thread_id", "child_thread_id"],
        "stage1_outputs" => &["thread_id"],
        _ => &[],
    };
    let keys = wanted
        .iter()
        .filter_map(|column| row.get_key_value(*column).map(|(key, _)| key))
        .collect::<Vec<_>>();
    if table == "messages" && keys.is_empty() {
        row.get_key_value("session_id")
            .map(|(key, _)| vec![key])
            .unwrap_or_default()
    } else {
        keys
    }
}

fn detect_file_restore_conflicts(
    tables: &Map<String, Value>,
    backup_store: &BackupStore,
    token: &str,
) -> anyhow::Result<()> {
    let Some(files) = tables.get("__files").and_then(Value::as_array) else {
        return Ok(());
    };
    let allowed_paths = allowed_backup_file_paths(tables);
    for file in files {
        if let Some(path) = file.get("path").and_then(Value::as_str) {
            if !allowed_paths.contains(path) {
                anyhow::bail!("unexpected backup file path: {path}");
            }
            if Path::new(path).exists() {
                anyhow::bail!("restore conflict: file already exists: {path}");
            }
            backup_store.validate_file_backup(token, file)?;
        }
    }
    Ok(())
}

fn allowed_backup_file_paths(tables: &Map<String, Value>) -> HashSet<String> {
    tables
        .get("threads")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|row| row.get("rollout_path").and_then(Value::as_str))
        .filter(|path| !path.trim().is_empty())
        .flat_map(|path| {
            // issue #162：__files 条目的 path 可能是互转后的可读写视角，
            // 撤销校验需要同时放行原始写法与互转写法
            let mut views = vec![path.to_string()];
            if let Some(alternative) = wsl_path_alternative(path) {
                views.push(alternative);
            }
            views
        })
        .collect()
}

fn insert_row(db: &Connection, table: &str, row: &Map<String, Value>) -> anyhow::Result<()> {
    let columns: Vec<&String> = row.keys().collect();
    if columns.is_empty() {
        return Ok(());
    }
    let quoted = columns
        .iter()
        .map(|column| format!("\"{}\"", column.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(", ");
    let marks = (0..columns.len())
        .map(|index| format!("?{}", index + 1))
        .collect::<Vec<_>>()
        .join(", ");
    let values = columns
        .iter()
        .map(|column| OwnedSqlValue(json_to_sql_value(&row[*column])))
        .collect::<Vec<_>>();
    let refs = values
        .iter()
        .map(|value| value as &dyn ToSql)
        .collect::<Vec<_>>();
    db.execute(
        &format!("INSERT INTO \"{table}\" ({quoted}) VALUES ({marks})"),
        refs.as_slice(),
    )?;
    Ok(())
}

fn update_existing_agent_job_item(
    db: &Connection,
    row: &Map<String, Value>,
) -> anyhow::Result<bool> {
    let Some(id) = row.get("id") else {
        return Ok(false);
    };
    if !row.contains_key("assigned_thread_id") || !has_table(db, "agent_job_items")? {
        return Ok(false);
    }
    let id_value = OwnedSqlValue(json_to_sql_value(id));
    let current_assignment = db.query_row(
        "SELECT assigned_thread_id FROM agent_job_items WHERE id = ?1 LIMIT 1",
        [&id_value as &dyn ToSql],
        |row| row.get::<_, Option<String>>(0),
    );
    let current_assignment = match current_assignment {
        Ok(value) => value,
        Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(false),
        Err(err) => return Err(err.into()),
    };
    if current_assignment.is_some() {
        anyhow::bail!("restore conflict: agent_job_items row already assigned");
    }
    let assigned = OwnedSqlValue(json_to_sql_value(&row["assigned_thread_id"]));
    db.execute(
        "UPDATE agent_job_items SET assigned_thread_id = ?1 WHERE id = ?2 AND assigned_thread_id IS NULL",
        [&assigned as &dyn ToSql, &id_value as &dyn ToSql],
    )?;
    Ok(true)
}

fn backup_related_rows(
    db: &Connection,
    tables: &mut Map<String, Value>,
    table: &str,
    where_clause: &str,
    params: &[&dyn ToSql],
) -> anyhow::Result<()> {
    if has_table(db, table)? {
        let rows = select_dicts(
            db,
            &format!("SELECT * FROM \"{table}\" WHERE {where_clause}"),
            params,
        )?;
        tables.insert(table.to_string(), Value::Array(rows));
    }
    Ok(())
}

fn delete_related_rows(
    db: &Connection,
    table: &str,
    where_clause: &str,
    params: &[&dyn ToSql],
) -> anyhow::Result<()> {
    if has_table(db, table)? {
        db.execute(
            &format!("DELETE FROM \"{table}\" WHERE {where_clause}"),
            params,
        )?;
    }
    Ok(())
}

/// 为 threads 行里的每个 rollout 文件生成 `__files` 备份条目。
///
/// issue #162：WSL 模式下 codex 写入 `threads.rollout_path` 的是 WSL 视角路径
/// （如 `/mnt/c/Users/.../rollout-xxx.jsonl`），Windows 侧进程按原路径读不到文件。
/// 这里先按原路径读取，失败再尝试互转后的另一种视角（见 [`wsl_path_alternative`]）；
/// 两种视角下文件都确实不存在时才跳过（与旧行为一致，视为文件已缺失）。
/// 文件存在但读取失败时记入 unreadable，交由调用方拒绝静默成功。
fn rollout_file_sources(thread_rows: Option<&Vec<Value>>) -> (Vec<BackupFileSource>, Vec<String>) {
    let mut sources = Vec::new();
    let mut unreadable = Vec::new();
    for row in thread_rows.into_iter().flatten() {
        let Some(path) = row
            .get("rollout_path")
            .and_then(Value::as_str)
            .filter(|path| !path.trim().is_empty())
        else {
            continue;
        };
        for view in std::iter::once(path.to_string()).chain(wsl_path_alternative(path)) {
            match File::open(&view) {
                Ok(file) => match file.metadata() {
                    Ok(metadata) if metadata.is_file() => {
                        sources.push(BackupFileSource {
                            path: PathBuf::from(view),
                            original_path: path.to_string(),
                        });
                        break;
                    }
                    Ok(_) => {}
                    Err(_) => {
                        unreadable.push(path.to_string());
                        break;
                    }
                },
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => {
                    unreadable.push(path.to_string());
                    break;
                }
            }
        }
        // 与旧行为一致：两个视角均不存在可跳过；已存在但不可读必须中止删除。
    }
    (sources, unreadable)
}

/// `/mnt/<盘符>/...` 与 `<盘符>:/...` 两种路径视角互转（issue #162）。
///
/// WSL 把 Windows 盘符挂载在 `/mnt/<盘符>` 下：同一文件在 WSL 侧写作
/// `/mnt/c/Users/a.jsonl`，在 Windows 侧写作 `C:/Users/a.jsonl`（或反斜杠）。
/// 只做字符串形式转换，不保证目标存在；路径不属于这两种形式时返回 None。
///
/// `pub(crate)`：会话修复（provider_sync）也要按同一套视角回退，见 issue #1424。
pub(crate) fn wsl_path_alternative(path: &str) -> Option<String> {
    let normalized = path.replace('\\', "/");
    if let Some(rest) = normalized.strip_prefix("/mnt/") {
        let rest_bytes = rest.as_bytes();
        if rest_bytes.len() > 1
            && rest_bytes[0].is_ascii_alphabetic()
            && rest_bytes[1] == b'/'
            && rest.len() > 2
        {
            // 前两个字节是 ASCII，切片安全
            let tail = &rest[2..];
            let drive = rest_bytes[0].to_ascii_uppercase() as char;
            return Some(format!("{drive}:/{tail}"));
        }
        return None;
    }
    let bytes = normalized.as_bytes();
    if bytes.len() > 3 && bytes[1] == b':' && bytes[2] == b'/' && bytes[0].is_ascii_alphabetic() {
        let drive = bytes[0].to_ascii_lowercase() as char;
        let tail = &normalized[3..];
        return Some(format!("/mnt/{drive}/{tail}"));
    }
    None
}

fn sql_value_to_json(value: ValueRef<'_>) -> Value {
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(value) => json!(value),
        ValueRef::Real(value) => json!(value),
        ValueRef::Text(value) => json!(String::from_utf8_lossy(value).to_string()),
        ValueRef::Blob(value) => json!(base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            value
        )),
    }
}

pub(crate) fn json_to_sql_value(value: &Value) -> SqlValue {
    match value {
        Value::Null => SqlValue::Null,
        Value::Bool(value) => SqlValue::Integer(i64::from(*value)),
        Value::Number(number) => {
            if let Some(value) = number.as_i64() {
                SqlValue::Integer(value)
            } else if let Some(value) = number.as_f64() {
                SqlValue::Real(value)
            } else {
                SqlValue::Text(number.to_string())
            }
        }
        Value::String(value) => SqlValue::Text(value.clone()),
        other => SqlValue::Text(other.to_string()),
    }
}

#[cfg(test)]
mod local_session_title_tests {
    use super::*;

    #[test]
    fn looks_up_titles_by_thread_id_across_databases() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("state_5.sqlite");
        let second = temp.path().join("other.sqlite");
        {
            let db = Connection::open(&first).unwrap();
            db.execute_batch(
                "CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    title TEXT,
                    cwd TEXT,
                    model_provider TEXT,
                    archived INTEGER DEFAULT 0,
                    updated_at_ms INTEGER,
                    rollout_path TEXT
                 );
                 INSERT INTO threads (id, title, updated_at_ms) VALUES
                    ('t-1', '会话标题一', 1),
                    ('t-2', '', 2);",
            )
            .unwrap();
        }
        {
            let db = Connection::open(&second).unwrap();
            db.execute_batch(
                "CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    title TEXT,
                    updated_at_ms INTEGER,
                    rollout_path TEXT
                 );
                 INSERT INTO threads (id, title, updated_at_ms) VALUES ('t-3', '会话标题三', 3);",
            )
            .unwrap();
        }

        let titles = local_session_titles(
            &[first, second],
            &[
                "t-1".to_string(),
                "t-2".to_string(),
                "t-3".to_string(),
                "missing".to_string(),
            ],
        );
        assert_eq!(titles.get("t-1").map(String::as_str), Some("会话标题一"));
        // 空标题视为未知，让前端继续显示短 id。
        assert!(!titles.contains_key("t-2"));
        assert_eq!(titles.get("t-3").map(String::as_str), Some("会话标题三"));
        assert!(!titles.contains_key("missing"));
        assert!(local_session_titles(&[], &[]).is_empty());
    }
}

#[cfg(test)]
mod database_not_found_tests {
    use super::*;

    fn adapter(
        db_path: PathBuf,
        allowed: Vec<PathBuf>,
        home: Option<&Path>,
    ) -> SQLiteStorageAdapter {
        let mut adapter = SQLiteStorageAdapter::new(db_path, BackupStore::new("/tmp/backups"))
            .with_allowed_db_paths(allowed);
        if let Some(home) = home {
            adapter = adapter.with_codex_home(home);
        }
        adapter
    }

    /// issue #875：报错必须带上候选路径与 Codex home，否则用户无从判断是解析错还是库不在。
    #[test]
    fn message_lists_missing_path_and_candidate_paths() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join(".codex");
        let primary = home.join("state_5.sqlite");
        let candidate = home.join("sqlite").join("state_5.sqlite");

        let adapter = adapter(
            primary.clone(),
            vec![primary.clone(), candidate.clone()],
            Some(&home),
        );
        let message = adapter.database_not_found_message();

        assert!(message.contains(&primary.to_string_lossy().to_string()));
        assert!(message.contains("exists: false"));
        assert!(
            message.contains(&candidate.to_string_lossy().to_string()),
            "候选路径应出现在报错里：{message}"
        );
        assert!(
            message.contains(&home.to_string_lossy().to_string()),
            "Codex home 应出现在报错里：{message}"
        );
    }

    /// 候选路径里已经存在的那个要标 exists: true，用户才能看出真正该用哪条。
    #[test]
    fn message_marks_existing_candidate_path() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join(".codex");
        fs::create_dir_all(&home).unwrap();
        let primary = home.join("state_5.sqlite");
        let existing = home.join("sqlite").join("state_5.sqlite");
        fs::create_dir_all(existing.parent().unwrap()).unwrap();
        fs::write(&existing, b"placeholder").unwrap();

        let adapter = adapter(primary, vec![existing.clone()], None);
        let message = adapter.database_not_found_message();

        assert!(
            message.contains(&format!("{} (exists: true)", existing.to_string_lossy())),
            "已存在的候选路径应标为 true：{message}"
        );
    }

    /// 没有 codex_home 时不该硬塞一行空的 home。
    #[test]
    fn message_omits_codex_home_when_unknown() {
        let temp = tempfile::tempdir().unwrap();
        let primary = temp.path().join("state_5.sqlite");

        let adapter = adapter(primary, vec![], None);
        let message = adapter.database_not_found_message();

        assert!(
            !message.contains("Codex home："),
            "不该出现空 home：{message}"
        );
    }
}

#[cfg(test)]
mod wsl_path_tests {
    use super::wsl_path_alternative;

    #[test]
    fn converts_wsl_mount_view_to_windows_drive_view() {
        assert_eq!(
            wsl_path_alternative("/mnt/c/Users/tom/.codex/sessions/a.jsonl").as_deref(),
            Some("C:/Users/tom/.codex/sessions/a.jsonl")
        );
        assert_eq!(
            wsl_path_alternative("/mnt/f/TEMP/rollout.jsonl").as_deref(),
            Some("F:/TEMP/rollout.jsonl")
        );
    }

    #[test]
    fn converts_windows_drive_view_to_wsl_mount_view() {
        assert_eq!(
            wsl_path_alternative("C:\\Users\\tom\\.codex\\sessions\\a.jsonl").as_deref(),
            Some("/mnt/c/Users/tom/.codex/sessions/a.jsonl")
        );
        assert_eq!(
            wsl_path_alternative("C:/Users/tom/a.jsonl").as_deref(),
            Some("/mnt/c/Users/tom/a.jsonl")
        );
    }

    #[test]
    fn roundtrip_between_views_is_stable() {
        let windows = "D:/data/rollout-1.jsonl";
        let wsl = wsl_path_alternative(windows).unwrap();
        assert_eq!(wsl, "/mnt/d/data/rollout-1.jsonl");
        assert_eq!(wsl_path_alternative(&wsl).as_deref(), Some(windows));
    }

    #[test]
    fn rejects_paths_outside_both_views() {
        // 非 /mnt/<盘符> 形式的 POSIX 路径不可互转
        assert_eq!(wsl_path_alternative("/mnt/external/catalog.json"), None);
        assert_eq!(wsl_path_alternative("/home/tom/a.jsonl"), None);
        assert_eq!(wsl_path_alternative("mnt/c/a.jsonl"), None);
        // 盘符后不是路径分隔符
        assert_eq!(wsl_path_alternative("C:temp.jsonl"), None);
        // 只有前缀没有剩余部分
        assert_eq!(wsl_path_alternative("/mnt/c/"), None);
        assert_eq!(wsl_path_alternative("C:/"), None);
    }
}
