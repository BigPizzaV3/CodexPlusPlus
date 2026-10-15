use super::{
    LocalSession, SchemaKind, codex_thread_filter, optional_column_expression, schema_kind,
    table_columns,
};
use rusqlite::{Connection, OpenFlags, Row, params_from_iter};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

#[cfg(test)]
mod tests;

// 文件还原可能保留时间戳及 SQLite 计数器，任何指纹命中最多复用 30 秒。
const INDEX_MAX_AGE: Duration = Duration::from_secs(30);

#[derive(Default)]
pub struct LocalSessionPage {
    pub sessions: Vec<LocalSession>,
    pub total_count: usize,
    pub has_more: bool,
    pub errors: Vec<String>,
}

#[derive(Default)]
pub struct LocalSessionPager {
    databases: Vec<(PathBuf, CachedDatabase)>,
    // 只缓存 ID、排序键和来源，不缓存可能很长的标题或路径，也不常驻数据库连接。
    order: Vec<(usize, usize)>,
    #[cfg(test)]
    after_snapshot: Option<Box<dyn FnOnce() + Send>>,
}

#[derive(Default)]
struct CachedDatabase {
    refreshed_at: Option<Instant>,
    stamp: Option<DatabaseStamp>,
    query: String,
    index: Vec<IndexEntry>,
}

struct IndexEntry {
    id: String,
    updated_at_ms: Option<i64>,
}

#[derive(PartialEq, Eq)]
struct FileStamp {
    len: u64,
    modified: Option<SystemTime>,
    created: Option<SystemTime>,
    header: Vec<u8>,
}

impl FileStamp {
    fn read(path: &Path, header_len: u64) -> std::io::Result<Option<Self>> {
        use std::io::Read;
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let metadata = file.metadata()?;
        let mut header = Vec::new();
        file.take(header_len).read_to_end(&mut header)?;
        Ok(Some(Self {
            len: metadata.len(),
            modified: metadata.modified().ok(),
            created: metadata.created().ok(),
            header,
        }))
    }
}

#[derive(PartialEq, Eq)]
struct DatabaseStamp {
    main: FileStamp,
    wal: Option<FileStamp>,
    shm: Option<FileStamp>,
}

impl DatabaseStamp {
    fn read(path: &Path) -> std::io::Result<Option<Self>> {
        let Some(main) = FileStamp::read(path, 100)? else {
            return Ok(None);
        };
        let sidecar = |suffix| {
            let mut name = path.as_os_str().to_os_string();
            name.push(suffix);
            PathBuf::from(name)
        };
        Ok(Some(Self {
            main,
            // WAL 头包含重用盐值；SHM 的两份 48 字节索引头包含提交帧和变更计数。
            // 只比较原始字节，不解析平台字节序；同时保留 30 秒期限作为兜底。
            wal: FileStamp::read(&sidecar("-wal"), 32)?,
            shm: FileStamp::read(&sidecar("-shm"), 96)?,
        }))
    }
}

impl LocalSessionPager {
    /// 每个库在短读事务内校验索引并读取详情，确保同一库的总数和页面一致。
    /// 独立数据库之间没有全局事务；同一 ID 按最新时间取值，时间相同时保留输入路径顺序。
    pub fn list_page(
        &mut self,
        paths: &[PathBuf],
        offset: usize,
        limit: usize,
    ) -> LocalSessionPage {
        let mut page = LocalSessionPage::default();
        let mut changed = false;
        if !self.databases.iter().map(|(path, _)| path).eq(paths) {
            self.databases = paths
                .iter()
                .cloned()
                .map(|path| (path, CachedDatabase::default()))
                .collect();
            changed = true;
        }

        // 每次请求只短暂打开连接。Windows 上即使只读连接也会阻止数据库重命名或删除。
        let mut connections = Vec::with_capacity(paths.len());
        for (path, cached) in &mut self.databases {
            let result = (|| -> anyhow::Result<_> {
                if !path.try_exists()? {
                    return Ok(None);
                }
                let connection =
                    Connection::open_with_flags(path.as_path(), OpenFlags::SQLITE_OPEN_READ_ONLY)?;
                connection.busy_timeout(Duration::from_millis(250))?;
                let stamp = DatabaseStamp::read(path)?;
                Ok(Some((connection, stamp)))
            })();
            match result {
                Ok(Some(connection)) => connections.push(Some(connection)),
                result => {
                    changed |= cached.stamp.take().is_some() || !cached.index.is_empty();
                    cached.index.clear();
                    if let Err(error) = result {
                        page.errors.push(format!("{}: {error}", path.display()));
                    }
                    connections.push(None);
                }
            }
        }

        let mut snapshots = Vec::with_capacity(self.databases.len());
        let mut versions = Vec::with_capacity(self.databases.len());
        for ((path, cached), connection) in self.databases.iter_mut().zip(&connections) {
            let Some((connection, before)) = connection else {
                snapshots.push(None);
                versions.push(None);
                continue;
            };
            let result = (|| -> anyhow::Result<_> {
                let transaction = connection.unchecked_transaction()?;
                // 先固定读快照；事务后的同连接 data_version 用来防止缓存绑定到较新的文件指纹。
                transaction.query_row("SELECT COUNT(*) FROM sqlite_schema", [], |_| Ok(()))?;
                let version: i64 =
                    transaction.query_row("PRAGMA data_version", [], |row| row.get(0))?;
                #[cfg(test)]
                if let Some(after_snapshot) = self.after_snapshot.take() {
                    after_snapshot();
                }
                let stamp = DatabaseStamp::read(path)?;
                let reusable = stamp.is_some()
                    && &stamp == before
                    && stamp == cached.stamp
                    && cached
                        .refreshed_at
                        .is_some_and(|time| time.elapsed() < INDEX_MAX_AGE);
                if !reusable {
                    changed = true;
                    let query = session_query(&transaction)?;
                    let mut statement =
                        transaction.prepare(&format!("SELECT id, updated_at_ms FROM ({query})"))?;
                    let index = statement
                        .query_map([], |row| {
                            Ok(IndexEntry {
                                id: row.get(0)?,
                                updated_at_ms: row.get(1)?,
                            })
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    cached.index = index;
                    cached.query = query;
                    cached.refreshed_at = Some(Instant::now());
                    cached.stamp = stamp;
                }
                Ok((transaction, version))
            })();
            match result {
                Ok((transaction, version)) => {
                    snapshots.push(Some((path, transaction, &cached.index, &cached.query)));
                    versions.push(Some(version));
                }
                Err(error) => {
                    changed = true;
                    cached.stamp = None;
                    cached.index.clear();
                    page.errors.push(format!("{}: {error}", path.display()));
                    snapshots.push(None);
                    versions.push(None);
                }
            }
        }

        if changed {
            self.order.clear();
            for (source, snapshot) in snapshots.iter().enumerate() {
                if let Some((_, _, index, _)) = snapshot {
                    self.order
                        .extend((0..index.len()).map(|entry| (source, entry)));
                }
            }
            let entry = |&(source, index): &(usize, usize)| {
                &snapshots[source]
                    .as_ref()
                    .expect("索引仅来自成功的读快照")
                    .2[index]
            };
            self.order.sort_by(|left, right| {
                let (left, right) = (entry(left), entry(right));
                right
                    .updated_at_ms
                    .cmp(&left.updated_at_ms)
                    .then_with(|| right.id.cmp(&left.id))
            });
            let mut seen = HashSet::new();
            self.order
                .retain(|item| seen.insert(entry(item).id.as_str()));
        }

        page.total_count = self.order.len();
        page.has_more = offset.saturating_add(limit) < page.total_count;
        let selected = self
            .order
            .get(offset..offset.saturating_add(limit).min(self.order.len()))
            .unwrap_or_default();
        for (source, snapshot) in snapshots.iter().enumerate() {
            let Some((path, transaction, index, query)) = snapshot else {
                continue;
            };
            let ids: Vec<_> = selected
                .iter()
                .filter(|(db, _)| *db == source)
                .map(|(_, entry)| &index[*entry].id)
                .collect();
            // 限制参数数量；这个数据层入口也可能被页大小更大的调用者使用。
            for chunk in ids.chunks(100) {
                let result = (|| -> anyhow::Result<Vec<LocalSession>> {
                    let placeholders = vec!["?"; chunk.len()].join(",");
                    // automation_runs 可能为同一会话保存多次运行，只解码最新的一条。
                    let sql = format!(
                        "SELECT * FROM (
                            SELECT *, ROW_NUMBER() OVER (PARTITION BY id ORDER BY updated_at_ms DESC) AS session_rank
                            FROM ({query}) WHERE id IN ({placeholders})
                         ) WHERE session_rank = 1 ORDER BY updated_at_ms DESC, id DESC"
                    );
                    let mut statement = transaction.prepare(&sql)?;
                    let rows = statement
                        .query_map(params_from_iter(chunk), |row| read_session(row, path))?;
                    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
                })();
                match result {
                    Ok(rows) => page.sessions.extend(rows),
                    Err(error) => page.errors.push(format!("{}: {error}", path.display())),
                }
            }
        }
        page.sessions.sort_by(|left, right| {
            right
                .updated_at_ms
                .cmp(&left.updated_at_ms)
                .then_with(|| right.id.cmp(&left.id))
        });
        drop(snapshots);
        for (((path, cached), connection), version) in
            self.databases.iter_mut().zip(&connections).zip(versions)
        {
            if let Some((connection, _)) = connection {
                let after = connection
                    .query_row("PRAGMA data_version", [], |row| row.get::<_, i64>(0))
                    .ok();
                if after != version || DatabaseStamp::read(path).ok().flatten() != cached.stamp {
                    // 页面仍属于刚才的读快照，但下次请求必须重新建索引。
                    cached.stamp = None;
                }
            }
        }
        // 读事务和文件句柄都在返回前释放，不需要后台轮询清理闲置连接。
        drop(connections);
        page
    }
}

pub(super) fn read_session(row: &Row<'_>, path: &Path) -> rusqlite::Result<LocalSession> {
    Ok(LocalSession {
        id: row.get(0)?,
        title: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
        cwd: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
        model_provider: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
        archived: row.get::<_, Option<i64>>(4)?.unwrap_or_default() != 0,
        updated_at_ms: row.get(5)?,
        rollout_path: row.get::<_, Option<String>>(6)?.unwrap_or_default(),
        db_path: path.to_string_lossy().into_owned(),
    })
}

// 列表、轻量索引和当前页详情共用字段映射及子会话过滤，避免三个入口逐渐偏离。
pub(super) fn session_query(db: &Connection) -> anyhow::Result<String> {
    match schema_kind(db)? {
        Some(SchemaKind::CodexThreads) => {
            let columns = table_columns(db, "threads")?
                .into_iter()
                .collect::<HashSet<_>>();
            let column = |name, fallback| optional_column_expression(&columns, name, fallback);
            let title = column("title", "''");
            let cwd = column("cwd", "''");
            let provider = column("model_provider", "''");
            let archived = column("archived", "0");
            let rollout = column("rollout_path", "''");
            let updated = if columns.contains("updated_at_ms") {
                "updated_at_ms"
            } else if columns.contains("updated_at") {
                "updated_at * 1000"
            } else if columns.contains("created_at_ms") {
                "created_at_ms"
            } else {
                "NULL"
            };
            let filter = codex_thread_filter(db)?;
            Ok(format!(
                "SELECT id, {title} AS title, {cwd} AS cwd, {provider} AS model_provider, {archived} AS archived, {updated} AS updated_at_ms, {rollout} AS rollout_path FROM threads {filter}"
            ))
        }
        Some(SchemaKind::CodexAutomationRuns) => {
            let columns = table_columns(db, "automation_runs")?
                .into_iter()
                .collect::<HashSet<_>>();
            let column = |name, fallback| optional_column_expression(&columns, name, fallback);
            let title = column("thread_title", "''");
            let cwd = column("source_cwd", "''");
            let status = column("status", "''");
            let updated = column("updated_at", "NULL");
            let created = column("created_at", "NULL");
            Ok(format!(
                "SELECT thread_id AS id, {title} AS title, {cwd} AS cwd, '' AS model_provider, LOWER({status}) = 'archived' AS archived, COALESCE({updated}, {created}) AS updated_at_ms, '' AS rollout_path FROM automation_runs WHERE COALESCE(thread_id, '') <> ''"
            ))
        }
        _ => anyhow::bail!("Unsupported local storage schema"),
    }
}
