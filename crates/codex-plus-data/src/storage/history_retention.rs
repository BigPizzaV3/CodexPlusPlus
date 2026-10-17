//! 删除标记与历史文件分开管理：只有不可从保留会话到达的已删除历史才能回收。
use super::history_graph::HistoryGraph;
use super::*;
use anyhow::{Context, bail};
use fs2::FileExt;
use std::collections::{BTreeMap, BTreeSet};

const TABLE: &str = "codex_plus_deleted_threads";
const RESTORED_TABLE: &str = "codex_plus_restored_threads";

pub(super) struct HistoryRetention {
    _lock: File,
    home: PathBuf,
    databases: Vec<PathBuf>,
}

impl HistoryRetention {
    pub fn contains_history(&self, id: &str) -> anyhow::Result<bool> {
        Ok(self.graph()?.files.contains_key(id))
    }

    pub fn acquire(home: &Path, paths: &[PathBuf]) -> anyhow::Result<Self> {
        fs::create_dir_all(home.join("tmp"))?;
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(home.join("tmp/history-retention.lock"))?;
        lock.try_lock_exclusive()
            .context("会话删除或恢复正在进行，请稍后重试")?;
        let mut databases = paths.to_vec();
        for path in codex_plus_core::codex_sqlite::codex_session_db_paths_from_home(home) {
            if !databases.contains(&path) {
                databases.push(path);
            }
        }
        Ok(Self {
            _lock: lock,
            home: home.to_path_buf(),
            databases,
        })
    }

    fn deleted_ids(&self) -> anyhow::Result<BTreeSet<String>> {
        let mut ids = BTreeSet::new();
        for path in &self.databases {
            if !path.is_file() {
                continue;
            }
            let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
            if has_table(&db, TABLE)? {
                let mut stmt = db.prepare(&format!("SELECT thread_id FROM {TABLE}"))?;
                for id in stmt.query_map([], |row| row.get::<_, String>(0))? {
                    ids.insert(id?);
                }
            }
        }
        Ok(ids)
    }

    fn graph(&self) -> anyhow::Result<HistoryGraph> {
        let mut paths = Vec::new();
        for path in &self.databases {
            if !path.is_file() {
                continue;
            }
            let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
            if has_columns(&db, "threads", &["id", "rollout_path"])? {
                let mut stmt = db.prepare("SELECT id, rollout_path FROM threads WHERE rollout_path IS NOT NULL AND rollout_path != ''")?;
                for row in stmt.query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })? {
                    let (id, path) = row?;
                    let original = PathBuf::from(&path);
                    if original.is_file() {
                        paths.push((id, original));
                    } else if let Some(alt) = wsl_path_alternative(&path).map(PathBuf::from) {
                        if alt.is_file() {
                            paths.push((id, alt));
                        }
                    }
                }
            }
        }
        HistoryGraph::scan(&self.home, &paths)
    }

    /// 先备份依赖闭包再提交删除意图，备份完成前不改变任何用户数据。
    pub fn prepare_delete(&self, id: &str, tables: &mut Map<String, Value>) -> anyhow::Result<()> {
        let graph = self.graph()?;
        let mut files = Vec::new();
        if graph.files.contains_key(id) {
            for path in graph.closure_files(id)? {
                if !graph.files[id].contains(&path) {
                    // 先确认祖先位于可恢复的原生目录，不能产生无法兑现的撤销备份。
                    self.validate_history_path(&path)?;
                }
                let bytes = fs::read(&path)?;
                files.push(rollout_file_backup_entry(
                    &path.to_string_lossy(),
                    &path.to_string_lossy(),
                    bytes,
                ));
            }
        }
        // 即使没有祖先也写此字段，区分支持依赖恢复的新备份与旧撤销契约。
        tables.insert("__history_files".into(), Value::Array(files));
        Ok(())
    }

    pub fn mark_deleted(db: &Connection, id: &str, token: &str) -> anyhow::Result<()> {
        if has_table(db, RESTORED_TABLE)? {
            db.execute(
                &format!("DELETE FROM {RESTORED_TABLE} WHERE thread_id=?1"),
                [id],
            )?;
        }
        db.execute_batch(&format!("CREATE TABLE IF NOT EXISTS {TABLE} (thread_id TEXT PRIMARY KEY, backup_token TEXT NOT NULL)"))?;
        db.execute(&format!("INSERT INTO {TABLE} (thread_id, backup_token) VALUES (?1,?2) ON CONFLICT(thread_id) DO UPDATE SET backup_token=excluded.backup_token"), [id, token])?;
        Ok(())
    }

    pub fn is_deleted(db: &Connection, id: &str) -> anyhow::Result<bool> {
        if !has_table(db, TABLE)? {
            return Ok(false);
        }
        Ok(db
            .query_row(
                &format!("SELECT 1 FROM {TABLE} WHERE thread_id=?1"),
                [id],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    pub fn collect(&self) -> anyhow::Result<usize> {
        let deleted = self.deleted_ids()?;
        if deleted.is_empty() {
            return Ok(0);
        }
        if deleted.iter().any(|id| uuid::Uuid::parse_str(id).is_ok()) && native_writers_running()? {
            return Ok(0);
        }
        let graph = self.graph()?;
        // 原生 fork 不遵守本模块的锁。在线只软删除，避免检查后新建 fork 的竞态。
        if has_native_history(&graph) && native_writers_running()? {
            return Ok(0);
        }
        self.collect_quiescent(graph, &deleted)
    }

    fn collect_quiescent(
        &self,
        graph: HistoryGraph,
        deleted: &BTreeSet<String>,
    ) -> anyhow::Result<usize> {
        let mut roots: BTreeSet<String> = graph
            .files
            .keys()
            .filter(|id| !deleted.contains(*id))
            .cloned()
            .collect();
        // 部分数据库删除失败时，另一库的未删除行仍然拥有历史。
        for path in &self.databases {
            if !path.is_file() {
                continue;
            }
            let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
            if !has_columns(&db, "threads", &["id", "rollout_path"])? {
                continue;
            }
            let mut stmt = db.prepare("SELECT id, rollout_path FROM threads")?;
            for row in stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })? {
                let (id, rollout) = row?;
                if Self::is_deleted(&db, &id)? {
                    continue;
                }
                if graph.files.contains_key(&id) {
                    roots.insert(id);
                }
                if let Some(rollout) = rollout {
                    let resolved = fs::canonicalize(&rollout).or_else(|_| {
                        wsl_path_alternative(&rollout)
                            .map(fs::canonicalize)
                            .unwrap_or_else(|| fs::canonicalize(&rollout))
                    });
                    if let Ok(resolved) = resolved {
                        for (physical_id, files) in &graph.files {
                            if files.contains(&resolved) {
                                roots.insert(physical_id.clone());
                            }
                        }
                    }
                }
            }
        }
        let reachable = graph.reachable_from(roots.iter().cloned());
        // 任一保留会话的闭包不完整时，先停止回收，避免把损坏的索引当作无引用。
        for id in &roots {
            graph.closure_files(id)?;
        }
        let mut removed = 0;
        for id in deleted.iter().filter(|id| !reachable.contains(*id)) {
            for path in graph.files.get(id).into_iter().flatten() {
                // 每个文件删除前再确认原生写入方未启动。
                if has_native_history(&graph) && native_writers_running()? {
                    return Ok(removed);
                }
                match fs::remove_file(path) {
                    Ok(()) => removed += 1,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => {
                        return Err(error).context("回收已删除的历史文件失败，稍后自动重试");
                    }
                }
            }
            for path in &self.databases {
                if !path.is_file() {
                    continue;
                }
                let mut db = Connection::open(path)?;
                if !has_table(&db, "threads")? || !Self::is_deleted(&db, id)? {
                    continue;
                }
                let tx = db.transaction()?;
                for table in ["thread_dynamic_tools", "thread_goals", "stage1_outputs"] {
                    delete_related_rows(&tx, table, "thread_id = ?1", &[id])?;
                }
                delete_related_rows(
                    &tx,
                    "thread_spawn_edges",
                    "parent_thread_id = ?1 OR child_thread_id = ?1",
                    &[id],
                )?;
                if has_columns(&tx, "agent_job_items", &["assigned_thread_id"])? {
                    tx.execute("UPDATE agent_job_items SET assigned_thread_id=NULL WHERE assigned_thread_id=?1", [id])?;
                }
                tx.execute("DELETE FROM threads WHERE id=?1", [id])?;
                tx.commit()?;
                // 标记继续保留：撤销叶子时祖先文件恢复后仍是软删除，不能变成列表中的活会话。
            }
        }
        Ok(removed)
    }

    /// 新备份允许保留中的同一文件存在，但绝不覆盖追加后的历史。
    pub fn prepare_undo(&self, backups: &mut [Value]) -> anyhow::Result<()> {
        let mut all_files = BTreeMap::<PathBuf, (Vec<u8>, Value)>::new();
        for backup in backups.iter_mut() {
            let db_path = backup_source_db(backup, &self.databases[0], &self.databases)?;
            let db = Connection::open(&db_path)?;
            let id = backup["session_id"].as_str().unwrap_or("").to_string();
            let token = backup["token"].as_str().unwrap_or("").to_string();
            let Some(tables) = backup["tables"].as_object_mut() else {
                continue;
            };
            validate_restore_tables(tables)?;
            if !tables.contains_key("__history_files") {
                continue;
            }
            let target_paths = tables
                .get("threads")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|row| row["rollout_path"].as_str())
                .map(|path| resolved_history_path(Path::new(path)))
                .collect::<anyhow::Result<BTreeSet<_>>>()?;
            let current_token = if has_table(&db, TABLE)? {
                db.query_row(
                    &format!("SELECT backup_token FROM {TABLE} WHERE thread_id=?1"),
                    [&id],
                    |row| row.get::<_, String>(0),
                )
                .optional()?
            } else {
                None
            };
            let completed_token = if current_token.is_none() && has_table(&db, RESTORED_TABLE)? {
                db.query_row(
                    &format!("SELECT backup_token FROM {RESTORED_TABLE} WHERE thread_id=?1"),
                    [&id],
                    |row| row.get::<_, String>(0),
                )
                .optional()?
            } else {
                None
            };
            if current_token.as_deref() != Some(&token)
                && completed_token.as_deref() != Some(&token)
            {
                bail!("restore conflict: deletion state changed for {id}");
            }
            let allowed_paths = allowed_backup_file_paths(tables);
            let mut archived = Vec::new();
            for (table, rows) in tables
                .iter_mut()
                .filter(|(name, _)| !name.starts_with("__"))
            {
                let Some(rows) = rows.as_array_mut() else {
                    continue;
                };
                let mut retained = Vec::new();
                for row in std::mem::take(rows) {
                    let Some(object) = row.as_object() else {
                        continue;
                    };
                    // 软删除保留任务关联；只跳过按主键核对后完全未变化的行。
                    // 已清空的关联仍交给原恢复流程，已改派或修改的行不能覆盖。
                    if table == "agent_job_items"
                        && let Some(job_id) = object.get("id")
                        && has_table(&db, table)?
                    {
                        let job_id = OwnedSqlValue(json_to_sql_value(job_id));
                        let current = select_dicts(
                            &db,
                            "SELECT * FROM agent_job_items WHERE id=?1",
                            &[&job_id as &dyn ToSql],
                        )?;
                        if let Some(current) = current.first()
                            && current
                                .get("assigned_thread_id")
                                .is_some_and(|value| !value.is_null())
                        {
                            if current != &row {
                                bail!("restore conflict: agent_job_items row changed");
                            }
                            continue;
                        }
                    }
                    if restore_row_conflicts(&db, table, object)? {
                        if table == "threads" && row["id"].as_str() == Some(&id) {
                            let current =
                                select_dicts(&db, "SELECT * FROM threads WHERE id=?1", &[&id])?;
                            let mut original = row.clone();
                            original["archived"] = current[0]["archived"].clone();
                            if object.contains_key("archived_at") {
                                original["archived_at"] = current[0]["archived_at"].clone();
                            }
                            if original != current[0] {
                                bail!("restore conflict: thread changed since deletion");
                            }
                            archived.push(row.clone());
                        } else {
                            // 软删除不修改关联行；相同的行无需重复插入。
                            let cols = restore_conflict_key_columns(table, object);
                            let clauses = cols
                                .iter()
                                .enumerate()
                                .map(|(i, k)| format!("\"{k}\"=?{}", i + 1))
                                .collect::<Vec<_>>()
                                .join(" AND ");
                            let vals = cols
                                .iter()
                                .map(|k| OwnedSqlValue(json_to_sql_value(&object[*k])))
                                .collect::<Vec<_>>();
                            let refs = vals.iter().map(|v| v as &dyn ToSql).collect::<Vec<_>>();
                            let current = select_dicts(
                                &db,
                                &format!("SELECT * FROM \"{table}\" WHERE {clauses}"),
                                &refs,
                            )?;
                            if !current.contains(&row) {
                                bail!("restore conflict: {table} row changed");
                            }
                        }
                    } else {
                        retained.push(row);
                    }
                }
                *rows = retained;
            }
            tables.insert("__restore_archived".into(), Value::Array(archived));
            for key in ["__files", "__history_files"] {
                for file in tables
                    .get(key)
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let path = file["path"]
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("invalid history backup path"))?;
                    if key == "__files" && !allowed_paths.contains(path) {
                        bail!("unexpected backup file path: {path}");
                    }
                    if key == "__history_files"
                        && !target_paths.contains(&comparable_history_path(Path::new(path)))
                    {
                        self.validate_history_path(Path::new(path))?;
                    }
                    let normalized = comparable_history_path(Path::new(path));
                    let bytes = base64::Engine::decode(
                        &base64::engine::general_purpose::STANDARD,
                        file["content_b64"]
                            .as_str()
                            .ok_or_else(|| anyhow::anyhow!("invalid history backup content"))?,
                    )?;
                    if let Some((previous, _)) = all_files.get(&normalized) {
                        if !previous.starts_with(&bytes) && !bytes.starts_with(previous) {
                            bail!("restore conflict: incompatible history snapshots: {path}");
                        }
                        if previous.len() >= bytes.len() {
                            continue;
                        }
                    }
                    all_files.insert(normalized, (bytes, file.clone()));
                }
            }
            tables.insert("__files".into(), Value::Array(Vec::new()));
        }
        let shared_manifest = all_files
            .values()
            .map(|(_, file)| file.clone())
            .collect::<Vec<_>>();
        let mut missing = Vec::new();
        for (path, (bytes, file)) in all_files {
            match fs::read(&path) {
                Ok(current) if current.starts_with(&bytes) => {}
                Ok(_) => bail!("restore conflict: history file changed: {}", path.display()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => missing.push(file),
                Err(error) => return Err(error.into()),
            }
        }
        if let Some(backup) = backups
            .iter_mut()
            .find(|b| b["tables"].get("__history_files").is_some())
        {
            backup["tables"]["__files"] = Value::Array(missing);
            backup["tables"]["__history_files"] = Value::Array(shared_manifest);
        }
        Ok(())
    }

    fn validate_history_path(&self, path: &Path) -> anyhow::Result<()> {
        let home = fs::canonicalize(&self.home)?;
        // canonicalize 在 Windows 上增加合法的扩展路径前缀。
        let normalized = comparable_history_path(path);
        let path = normalized.as_path();
        // 路径必须来自本 home 的两个原生历史目录；拒绝 .. 与逃逸链接。
        for root in ["sessions", "archived_sessions"] {
            // 扫描返回规范路径；原始 home 可能含 Windows 短文件名、不同大小写或链接。
            let original_root = comparable_history_path(&self.home).join(root);
            let canonical_root = comparable_history_path(&home).join(root);
            if (path.starts_with(&original_root) || path.starts_with(&canonical_root))
                && !path
                    .components()
                    .any(|part| matches!(part, std::path::Component::ParentDir))
            {
                let mut parent = path.parent();
                while let Some(candidate) = parent {
                    if candidate.exists() {
                        if fs::canonicalize(candidate)?.starts_with(&home) {
                            return Ok(());
                        }
                        break;
                    }
                    parent = candidate.parent();
                }
            }
        }
        bail!("unexpected history dependency path: {}", path.display())
    }

    pub fn finish_undo(&self, backups: &[Value]) -> anyhow::Result<()> {
        for backup in backups {
            if backup["tables"].get("__history_files").is_none() {
                continue;
            }
            let db_path = backup_source_db(backup, &self.databases[0], &self.databases)?;
            let mut db = Connection::open(db_path)?;
            let tx = db.transaction()?;
            tx.execute_batch(&format!("CREATE TABLE IF NOT EXISTS {RESTORED_TABLE} (thread_id TEXT PRIMARY KEY, backup_token TEXT NOT NULL)"))?;
            tx.execute(&format!("INSERT INTO {RESTORED_TABLE}(thread_id, backup_token) VALUES (?1,?2) ON CONFLICT(thread_id) DO UPDATE SET backup_token=excluded.backup_token"), [backup["session_id"].as_str().unwrap_or(""), backup["token"].as_str().unwrap_or("")])?;
            tx.execute(
                &format!("DELETE FROM {TABLE} WHERE thread_id=?1 AND backup_token=?2"),
                [
                    backup["session_id"].as_str().unwrap_or(""),
                    backup["token"].as_str().unwrap_or(""),
                ],
            )?;
            tx.commit()?;
            crate::provider_sync::clear_thread_tombstones(
                &self.home,
                &HashSet::from([backup["session_id"].as_str().unwrap_or("").to_string()]),
            );
        }
        Ok(())
    }
}

/// Windows 的扩展路径与普通路径必须在同一命名空间内比较；UNC 仍保留服务器前缀。
fn comparable_history_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let text = path.to_string_lossy();
        if let Some(rest) = text.strip_prefix("\\\\?\\UNC\\") {
            return PathBuf::from(format!("\\\\{rest}"));
        }
        if let Some(rest) = text.strip_prefix("\\\\?\\") {
            return PathBuf::from(rest);
        }
    }
    path.to_path_buf()
}

// 回收后的文件可能已不存在；从最近的现存祖先解析短路径与目录大小写。
fn resolved_history_path(path: &Path) -> anyhow::Result<PathBuf> {
    let mut ancestor = path;
    let mut missing = Vec::new();
    loop {
        match fs::canonicalize(ancestor) {
            Ok(mut resolved) => {
                for name in missing.into_iter().rev() {
                    resolved.push(name);
                }
                return Ok(comparable_history_path(&resolved));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let name = ancestor.file_name().ok_or_else(|| {
                    anyhow::anyhow!("cannot resolve history path: {}", path.display())
                })?;
                missing.push(name.to_os_string());
                ancestor = ancestor.parent().ok_or(error)?;
            }
            Err(error) => return Err(error.into()),
        }
    }
}

#[cfg(all(test, windows))]
mod windows_path_tests {
    use super::*;

    #[test]
    fn extended_paths_compare_with_regular_paths_without_losing_unc_server() {
        for (extended, regular) in [
            (
                r"\\?\C:\codex\sessions\history.jsonl",
                r"C:\codex\sessions\history.jsonl",
            ),
            (
                r"\\?\UNC\server\share\codex\sessions",
                r"\\server\share\codex\sessions",
            ),
        ] {
            assert_eq!(
                comparable_history_path(Path::new(extended)),
                PathBuf::from(regular)
            );
            assert_eq!(
                comparable_history_path(Path::new(regular)),
                PathBuf::from(regular)
            );
        }
        assert!(
            !comparable_history_path(Path::new(r"\\?\UNC\other\share\sessions"))
                .starts_with(comparable_history_path(Path::new(r"\\?\UNC\server\share")))
        );
    }
}

fn has_native_history(graph: &HistoryGraph) -> bool {
    graph
        .files
        .keys()
        .any(|id| uuid::Uuid::parse_str(id).is_ok())
}

fn native_writers_running() -> anyhow::Result<bool> {
    #[cfg(windows)]
    {
        let processes = codex_plus_core::windows_enumerate_processes();
        if processes.is_empty() {
            bail!("无法读取进程列表，暂缓历史回收");
        }
        Ok(processes.iter().any(|process| {
            process.exe_file.eq_ignore_ascii_case("codex.exe")
                || process.exe_file.eq_ignore_ascii_case("chatgpt.exe")
        }))
    }
    #[cfg(not(windows))]
    {
        let output = std::process::Command::new("ps")
            .args(["-axo", "comm="])
            .output()?;
        if !output.status.success() {
            bail!("无法确认 Codex 已停止，暂缓历史回收");
        }
        Ok(String::from_utf8_lossy(&output.stdout).lines().any(|line| {
            let name = Path::new(line.trim())
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            ["codex", "chatgpt", "codex dev"]
                .iter()
                .any(|wanted| name.eq_ignore_ascii_case(wanted))
        }))
    }
}

pub(crate) fn collect_deleted_history(home: &Path) -> anyhow::Result<usize> {
    let paths = codex_plus_core::codex_sqlite::codex_session_db_paths_from_home(home);
    HistoryRetention::acquire(home, &paths)?.collect()
}
