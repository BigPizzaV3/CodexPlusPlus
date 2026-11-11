//! 结构化用量的事务存储；来源解析和跨文件计数归并由调用方完成。
use anyhow::{Context, ensure};
use rusqlite::{Connection, OpenFlags, OptionalExtension, Transaction, params};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::execution_index::{self, ExecutionObservation};

mod append;
pub(super) mod attribution;
mod checkpoint;
#[allow(dead_code)]
pub(in crate::usage_history::structured) mod controller;
mod retention;
pub(in crate::usage_history::structured) mod reclassification;
mod resolve;
mod scan;
mod sealed_retention;
mod source_migration;
pub(super) use scan::{decode_source, encode_identity, encode_source};

// 14 允许压缩用量观察；旧写入器不能识别新的观察格式。
const SCHEMA_VERSION: i64 = 14;
const WORK_BUDGET_BYTES: u64 = 1024 * 1024 * 1024;
const BUDGET_BLOCKED: &str =
    "usage index storage is full or exceeds its database page budget; ingestion is paused";

#[derive(Debug)]
struct SealedReviewRequired;

impl std::fmt::Display for SealedReviewRequired {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("sealed usage history requires review; related source ingestion is paused")
    }
}

impl std::error::Error for SealedReviewRequired {}

pub(super) struct Observation {
    pub(super) position: u64,
    pub(super) event_id: [u8; 32],
    pub(super) session: String,
    pub(super) payload: Vec<u8>,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct ResolvedEvent {
    pub(super) id: [u8; 32],
    pub(super) session: String,
    pub(super) day: String,
    pub(super) timestamp_ms: i64,
    pub(super) model: String,
    pub(super) provider: String,
    pub(super) provider_source: Option<String>,
    pub(super) role: String,
    pub(super) context_conflicts: u8,
    pub(super) count_uncertain: bool,
    pub(super) count_incomplete: bool,
    pub(super) input: u64,
    pub(super) cached: Option<u64>,
    pub(super) output: u64,
    pub(super) reasoning: Option<u64>,
}

pub(super) enum Resolution {
    Full(Vec<ResolvedEvent>),
    Append(Vec<ResolvedEvent>),
}

pub(super) struct SourceBatch {
    pub(super) source: Vec<u8>,
    pub(super) identity: Vec<u8>,
    pub(super) expected_offset: u64,
    pub(super) next_offset: u64,
    pub(super) parser_state: Vec<u8>,
    pub(super) complete: bool,
    pub(super) usage_cutoff: chrono::NaiveDate,
    pub(super) replace: bool,
    pub(super) observations: Vec<Observation>,
    pub(super) execution_observations: Vec<ExecutionObservation>,
    pub(super) scope_session: String,
    pub(super) scope_valid: bool,
    pub(super) scope_parents: Vec<(String, Option<String>)>,
    pub(super) scope_aliases: Vec<(String, String)>,
    pub(super) execution_valid: bool,
    pub(super) count_gap_sessions: Option<BTreeSet<String>>,
    pub(super) now_ms: i64,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct SourceState {
    pub(super) identity: Vec<u8>,
    pub(super) offset: u64,
    pub(super) parser_state: Vec<u8>,
    pub(super) complete: bool,
    pub(super) usage_cutoff: Option<String>,
    pub(super) scope_indexed: bool,
    pub(super) execution_indexed: bool,
}

pub(super) struct PersistentIndex {
    db: Connection,
    budget_blocked: bool,
    path: PathBuf,
    budget_bytes: u64,
    source_migration_complete: bool,
}

impl PersistentIndex {
    pub(super) fn open(path: &Path) -> anyhow::Result<Self> {
        Self::open_with_budget(path, WORK_BUDGET_BYTES)
    }

    fn open_with_budget(path: &Path, budget_bytes: u64) -> anyhow::Result<Self> {
        Self::open_config(path, budget_bytes, true)
    }

    fn open_config(path: &Path, budget_bytes: u64, create: bool) -> anyhow::Result<Self> {
        let mut flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        if create {
            flags |= OpenFlags::SQLITE_OPEN_CREATE;
        }
        let mut db = Connection::open_with_flags(path, flags)?;
        db.busy_timeout(std::time::Duration::from_millis(100))?;
        db.pragma_update(None, "foreign_keys", true)?;
        let page_size: u64 = db.pragma_query_value(None, "page_size", |row| row.get(0))?;
        // 回滚日志可能包含数据库所有原始页及日志头。留出两份数据库的空间，
        // 使事务在 SQLite 页数上限处失败并回滚，而不是先提交游标再检查容量。
        let max_pages = budget_bytes / page_size / 3;
        ensure!(max_pages > 0, "usage index budget is too small");
        let mut current_pages: u64 = db.pragma_query_value(None, "page_count", |row| row.get(0))?;
        let version: i64 = db.pragma_query_value(None, "user_version", |row| row.get(0))?;
        ensure!(
            (0..=SCHEMA_VERSION).contains(&version),
            "unsupported usage index schema version {version}"
        );
        let auto_vacuum: i64 = db.pragma_query_value(None, "auto_vacuum", |row| row.get(0))?;
        if current_pages > max_pages && auto_vacuum == 2
            && current_pages.saturating_mul(page_size).saturating_mul(2)
                .saturating_add(1024 * 1024) <= budget_bytes
        {
            // 只回收空闲页；不增加页数，保留一份完整回滚日志及日志头空间。
            let mode: String = db.pragma_query_value(None, "journal_mode", |row| row.get(0))?;
            if mode.eq_ignore_ascii_case("delete") {
                reclaim_free_pages(&db)?;
                current_pages = db.pragma_query_value(None, "page_count", |row| row.get(0))?;
            }
        }
        if current_pages > max_pages {
            ensure!(
                version >= 5,
                "usage index schema migration is paused by its database page budget"
            );
            drop(db);
            let db = Connection::open_with_flags(
                path,
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )?;
            db.busy_timeout(std::time::Duration::from_millis(100))?;
            return Ok(Self {
                db,
                budget_blocked: true,
                path: path.to_path_buf(),
                budget_bytes,
                source_migration_complete: false,
            });
        }
        // 使用回滚日志并为其预留空间；单靠限制数据库页数不能限制 WAL。
        let journal_mode: String = db.pragma_query_value(None, "journal_mode", |row| row.get(0))?;
        if !journal_mode.eq_ignore_ascii_case("delete") {
            let selected: String =
                db.pragma_update_and_check(None, "journal_mode", "DELETE", |row| row.get(0))?;
            ensure!(
                selected.eq_ignore_ascii_case("delete"),
                "usage index cannot switch to rollback journal"
            );
        }
        let selected: u64 = db.pragma_update_and_check(
            None,
            "max_page_count",
            i64::try_from(max_pages).context("usage index page budget is too large")?,
            |row| row.get(0),
        )?;
        ensure!(
            selected <= max_pages,
            "usage index page budget was rejected"
        );
        if version < SCHEMA_VERSION {
            let migration = (|| -> anyhow::Result<()> {
                if auto_vacuum != 2 {
                    db.pragma_update(None, "auto_vacuum", "INCREMENTAL")?;
                    if version != 0 {
                        // 旧候选仅在三份数据库均装得下既有工作预算时转换一次。
                        db.execute_batch("VACUUM;")?;
                    }
                }
                let tx = db.transaction()?;
                if version == 0 {
                    tx.execute_batch(
                        "CREATE TABLE usage_meta (
                    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
                    revision INTEGER NOT NULL
                );
                INSERT INTO usage_meta VALUES (1, 0);
                CREATE TABLE usage_sources (
                    source BLOB PRIMARY KEY,
                    identity BLOB NOT NULL,
                    offset INTEGER NOT NULL,
                    parser_state BLOB NOT NULL,
                    complete INTEGER NOT NULL
                );
                CREATE TABLE usage_observations (
                    source BLOB NOT NULL REFERENCES usage_sources(source),
                    position INTEGER NOT NULL,
                    event_id BLOB NOT NULL,
                    session TEXT NOT NULL,
                    payload BLOB NOT NULL,
                    PRIMARY KEY(source, position)
                ) WITHOUT ROWID;
                CREATE INDEX usage_observations_session ON usage_observations(session);
                CREATE TABLE usage_events (
                    id BLOB PRIMARY KEY,
                    session TEXT NOT NULL,
                    day TEXT NOT NULL,
                    timestamp_ms INTEGER NOT NULL,
                    model TEXT NOT NULL,
                    provider TEXT NOT NULL,
                    provider_source TEXT,
                    role TEXT NOT NULL,
                    context_conflicts INTEGER NOT NULL,
                    count_uncertain INTEGER NOT NULL,
                    count_incomplete INTEGER NOT NULL,
                    input INTEGER NOT NULL,
                    cached INTEGER,
                    output INTEGER NOT NULL,
                    reasoning INTEGER
                );
                CREATE INDEX usage_events_session ON usage_events(session, id);
                CREATE INDEX usage_events_session_day ON usage_events(session, day);
                CREATE TABLE usage_daily (
                    day TEXT NOT NULL,
                    session TEXT NOT NULL,
                    model TEXT NOT NULL,
                    provider TEXT NOT NULL,
                    provider_source TEXT NOT NULL,
                    role TEXT NOT NULL,
                    input INTEGER NOT NULL,
                    cached INTEGER,
                    output INTEGER NOT NULL,
                    reasoning INTEGER,
                    records INTEGER NOT NULL,
                    uncertain_events INTEGER NOT NULL,
                    PRIMARY KEY(day, session, model, provider, provider_source, role)
                ) WITHOUT ROWID;",
                    )?;
                }
                if version < 2 {
                    tx.execute_batch(
                    "ALTER TABLE usage_sources ADD COLUMN scope_session TEXT NOT NULL DEFAULT '';
                 ALTER TABLE usage_sources ADD COLUMN scope_indexed INTEGER NOT NULL DEFAULT 0;
                 ALTER TABLE usage_sources ADD COLUMN scope_valid INTEGER NOT NULL DEFAULT 0;
                 ALTER TABLE usage_sources ADD COLUMN execution_indexed INTEGER NOT NULL DEFAULT 0;
                 ALTER TABLE usage_sources ADD COLUMN execution_valid INTEGER NOT NULL DEFAULT 0;
                 CREATE TABLE usage_scope_parents (
                    source BLOB NOT NULL REFERENCES usage_sources(source),
                    session TEXT NOT NULL,
                    parent TEXT NOT NULL,
                    PRIMARY KEY(source, session, parent)
                 ) WITHOUT ROWID;
                 CREATE INDEX usage_scope_parents_session ON usage_scope_parents(session);
                 CREATE TABLE usage_scope_aliases (
                    source BLOB NOT NULL REFERENCES usage_sources(source),
                    alias TEXT NOT NULL,
                    session TEXT NOT NULL,
                    PRIMARY KEY(source, alias, session)
                 ) WITHOUT ROWID;
                 CREATE INDEX usage_scope_aliases_alias ON usage_scope_aliases(alias);",
                )?;
                    execution_index::initialize(&tx)?;
                }
                if version < 3 {
                    let has_process_id = {
                        let mut statement =
                            tx.prepare("PRAGMA table_info(execution_observations)")?;
                        let names = statement
                            .query_map([], |row| row.get::<_, String>(1))?
                            .collect::<Result<Vec<_>, _>>()?;
                        names.iter().any(|name| name == "process_id")
                    };
                    if !has_process_id {
                        tx.execute_batch(
                            "ALTER TABLE execution_observations ADD COLUMN process_id TEXT;",
                        )?;
                    }
                    execution_index::initialize(&tx)?;
                    tx.execute_batch(
                        "CREATE TABLE usage_collision_families (
                        id BLOB PRIMARY KEY
                    ) WITHOUT ROWID;",
                    )?;
                }
                if version < 4 {
                    tx.execute_batch(
                        "CREATE INDEX usage_events_day_timestamp_session
                         ON usage_events(day, timestamp_ms, session);
                     CREATE TABLE usage_count_gaps (
                         source BLOB NOT NULL REFERENCES usage_sources(source),
                         session TEXT NOT NULL,
                         PRIMARY KEY(source, session)
                     ) WITHOUT ROWID;
                     CREATE INDEX usage_count_gaps_session ON usage_count_gaps(session, source);
                     CREATE TABLE usage_unindexed_checkpoints (
                         source BLOB PRIMARY KEY REFERENCES usage_sources(source),
                         error TEXT NOT NULL
                     ) WITHOUT ROWID;",
                    )?;
                    if version > 0 {
                        let mut statement = tx.prepare(
                            "SELECT source, parser_state FROM usage_sources ORDER BY source",
                        )?;
                        let mut rows = statement.query([])?;
                        while let Some(row) = rows.next()? {
                            let source: Vec<u8> = row.get(0)?;
                            let parser_state: Vec<u8> = row.get(1)?;
                            match checkpoint::restore(&parser_state) {
                                Ok(state) => {
                                    if state.unlocalized_invalid_counts || state.pending_count_gap {
                                        let mut sessions = BTreeSet::new();
                                        if !state.session.is_empty() {
                                            sessions.insert(state.session);
                                        }
                                        sessions.extend(
                                            state.inherited_contexts.into_iter().filter_map(
                                                |context| {
                                                    (!context.session.is_empty())
                                                        .then_some(context.session)
                                                },
                                            ),
                                        );
                                        let mut observed = tx.prepare(
                                            "SELECT DISTINCT session FROM usage_observations
                                         WHERE source = ?1",
                                        )?;
                                        for session in observed
                                            .query_map([&source], |row| row.get::<_, String>(0))?
                                        {
                                            sessions.insert(session?);
                                        }
                                        for session in sessions {
                                            tx.execute(
                                                "INSERT INTO usage_count_gaps(source, session)
                                             VALUES (?1, ?2)",
                                                params![source, session],
                                            )?;
                                        }
                                    }
                                }
                                Err(error) => {
                                    tx.execute(
                                        "INSERT INTO usage_unindexed_checkpoints(source, error)
                                     VALUES (?1, ?2)",
                                        params![source, format!("{error:#}")],
                                    )?;
                                }
                            }
                        }
                    }
                }
                if version < 5 {
                    tx.execute_batch("ALTER TABLE usage_sources ADD COLUMN usage_cutoff TEXT;")?;
                }
                if version < 6 {
                    tx.execute_batch(
                    "DROP TABLE usage_daily;
                     CREATE TABLE usage_daily (
                        day TEXT NOT NULL,
                        session TEXT NOT NULL,
                        model TEXT NOT NULL,
                        provider TEXT NOT NULL,
                        provider_source TEXT NOT NULL,
                        role TEXT NOT NULL,
                        context_conflicts INTEGER NOT NULL,
                        input INTEGER NOT NULL,
                        cached INTEGER,
                        cached_known_sum INTEGER NOT NULL,
                        output INTEGER NOT NULL,
                        reasoning INTEGER,
                        records INTEGER NOT NULL,
                        uncertain_events INTEGER NOT NULL,
                        total_events INTEGER NOT NULL,
                        PRIMARY KEY(day, session, model, provider, provider_source, role, context_conflicts)
                     ) WITHOUT ROWID;",
                )?;
                    populate_daily(&tx, None)?;
                }
                if version < 7 {
                    tx.execute_batch(
                        "CREATE INDEX IF NOT EXISTS executions_detail_expiry
                         ON execution_observations(timestamp_ms, session, call_id)
                         WHERE is_output = 0 AND end IS NOT NULL;",
                    )?;
                }
                if version < 8 {
                    tx.execute_batch(
                        "CREATE INDEX IF NOT EXISTS execution_daily_session
                         ON execution_daily(session);",
                    )?;
                }
                if version < 9 {
                    tx.execute_batch(
                        "CREATE TABLE IF NOT EXISTS usage_day_only (
                            day TEXT NOT NULL,
                            session TEXT NOT NULL,
                            PRIMARY KEY(day, session)
                         ) WITHOUT ROWID;
                         CREATE INDEX IF NOT EXISTS usage_day_only_session
                         ON usage_day_only(session);",
                    )?;
                }
                if version < 10 {
                    tx.execute_batch(
                        "DROP INDEX usage_observations_session;
                         CREATE INDEX usage_observations_session
                         ON usage_observations(session, source, event_id);
                         CREATE INDEX IF NOT EXISTS usage_scope_aliases_session
                         ON usage_scope_aliases(session, alias);",
                    )?;
                    sealed_retention::initialize(&tx)?;
                }
                if version < 11 {
                    let has_counts = {
                        let mut columns = tx.prepare("PRAGMA table_info(execution_daily)")?;
                        columns.query_map([], |row| row.get::<_, String>(1))?
                            .collect::<Result<Vec<_>, _>>()?.iter().any(|name| name == "skill")
                    };
                    if !has_counts {
                        tx.execute_batch(
                            "ALTER TABLE execution_daily ADD COLUMN skill INTEGER NOT NULL DEFAULT 0;
                             ALTER TABLE execution_daily ADD COLUMN mcp INTEGER NOT NULL DEFAULT 0;
                             ALTER TABLE execution_daily ADD COLUMN tool INTEGER NOT NULL DEFAULT 0;",
                        )?;
                    }
                }
                if version < 13 {
                    tx.execute_batch(
                        "CREATE TABLE IF NOT EXISTS usage_source_migration (
                            singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
                            old_source BLOB NOT NULL,
                            new_source BLOB NOT NULL
                        );",
                    )?;
                    if version > 0 {
                        tx.execute(
                            "INSERT OR IGNORE INTO usage_source_migration
                             VALUES (1, X'', X'')",
                            [],
                        )?;
                    }
                }
                tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
                tx.commit()?;
                Ok(())
            })();
            if let Err(error) = migration {
                if version < 5
                    || !matches!(
                        error.downcast_ref::<rusqlite::Error>(),
                        Some(rusqlite::Error::SqliteFailure(error, _))
                            if error.code == rusqlite::ErrorCode::DiskFull
                    )
                {
                    return Err(error);
                }
                drop(db);
                let db = Connection::open_with_flags(
                    path,
                    OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
                )?;
                db.busy_timeout(std::time::Duration::from_millis(100))?;
                return Ok(Self {
                    db,
                    budget_blocked: true,
                    path: path.to_path_buf(),
                    budget_bytes,
                    source_migration_complete: false,
                });
            }
        }
        let source_migration_complete = if cfg!(windows) {
            db.query_row(
                "SELECT NOT EXISTS(SELECT 1 FROM usage_source_migration)",
                [],
                |row| row.get(0),
            )?
        } else {
            true
        };
        Ok(Self {
            db,
            budget_blocked: false,
            path: path.to_path_buf(),
            budget_bytes,
            source_migration_complete,
        })
    }

    pub(super) fn source_state(&self, source: &[u8]) -> anyhow::Result<Option<SourceState>> {
        self.db
            .query_row(
                "SELECT identity, offset, parser_state, complete, scope_indexed, execution_indexed,
                        usage_cutoff
                 FROM usage_sources WHERE source = ?1",
                [source],
                |row| {
                    let offset: i64 = row.get(1)?;
                    let offset = u64::try_from(offset).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            1,
                            rusqlite::types::Type::Integer,
                            Box::new(error),
                        )
                    })?;
                    Ok(SourceState {
                        identity: row.get(0)?,
                        offset,
                        parser_state: row.get(2)?,
                        complete: row.get(3)?,
                        scope_indexed: row.get(4)?,
                        execution_indexed: row.get(5)?,
                        usage_cutoff: row.get(6)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub(super) fn revision(&self) -> anyhow::Result<i64> {
        Ok(self.db.query_row(
            "SELECT revision FROM usage_meta WHERE singleton = 1",
            [],
            |row| row.get(0),
        )?)
    }

    fn compact_usage_payloads(
        &mut self,
        cursor: Option<&checkpoint::PayloadCursor>,
    ) -> anyhow::Result<(Option<checkpoint::PayloadCursor>, usize)> {
        ensure!(!self.budget_blocked, "{BUDGET_BLOCKED}");
        let tx = self.db.transaction()?;
        let result = checkpoint::compact_payloads(&tx, cursor)?;
        tx.commit()?;
        Ok(result)
    }

    fn repack_usage_payloads(&mut self) -> anyhow::Result<()> {
        ensure!(!self.budget_blocked, "{BUDGET_BLOCKED}");
        let pages: u64 = self.db.pragma_query_value(None, "page_count", |row| row.get(0))?;
        let page_size: u64 = self.db.pragma_query_value(None, "page_size", |row| row.get(0))?;
        ensure!(
            pages.saturating_mul(page_size).saturating_mul(3) <= self.budget_bytes,
            "usage index repack exceeds its working budget"
        );
        // 短编码先释放的是页内空间；复用一次 VACUUM 将它合并成真正可再利用的页预算。
        self.db.execute_batch("VACUUM")?;
        Ok(())
    }

    pub(super) fn purge_execution_details(&mut self, now_ms: i64) -> anyhow::Result<bool> {
        ensure!(!self.budget_blocked, "{BUDGET_BLOCKED}");
        let tx = self.db.transaction()?;
        let before = tx.total_changes();
        let calls_changed = execution_index::purge_details(&tx, now_ms)?;
        let changed = tx.total_changes() != before;
        if calls_changed {
            tx.execute(
                "UPDATE usage_meta SET revision = revision + 1 WHERE singleton = 1",
                [],
            )?;
        }
        tx.commit()?;
        Ok(changed)
    }

    fn reclaim_execution_details(
        &mut self,
        now_ms: i64,
        cursor: Option<&execution_index::DetailReclaimCursor>,
    ) -> anyhow::Result<execution_index::DetailReclaimBatch> {
        ensure!(!self.budget_blocked, "{BUDGET_BLOCKED}");
        let tx = self.db.transaction()?;
        let batch = execution_index::reclaim_oldest_details(&tx, now_ms, cursor)?;
        if batch.changed {
            tx.execute(
                "UPDATE usage_meta SET revision = revision + 1 WHERE singleton = 1",
                [],
            )?;
        }
        tx.commit()?;
        Ok(batch)
    }

    pub(super) fn repair_execution_metadata(
        &mut self,
        home: &Path,
        cursor: Option<&execution_index::MetadataCursor>,
    ) -> anyhow::Result<execution_index::MetadataRepair> {
        ensure!(!self.budget_blocked, "{BUDGET_BLOCKED}");
        execution_index::repair_metadata(&mut self.db, home, cursor)
    }

    pub(super) fn compact_execution_times(
        &mut self,
        now_ms: i64,
        cursor: Option<&execution_index::CompactionCursor>,
        allow_incomplete: bool,
    ) -> anyhow::Result<execution_index::CompactionBatch> {
        ensure!(!self.budget_blocked, "{BUDGET_BLOCKED}");
        let tx = self.db.transaction()?;
        let batch = execution_index::compact_batch(&tx, now_ms, cursor, allow_incomplete)?;
        if batch.summarized {
            tx.execute(
                "UPDATE usage_meta SET revision = revision + 1 WHERE singleton = 1",
                [],
            )?;
        }
        tx.commit()?;
        Ok(batch)
    }

    fn compact_usage_events(
        &mut self,
        now_ms: i64,
        cursor: Option<&retention::Cursor>,
    ) -> anyhow::Result<retention::Batch> {
        ensure!(!self.budget_blocked, "{BUDGET_BLOCKED}");
        let tx = self.db.transaction()?;
        let batch = retention::compact(&tx, now_ms, cursor)?;
        if batch.precision_changed {
            tx.execute(
                "UPDATE usage_meta SET revision = revision + 1 WHERE singleton = 1",
                [],
            )?;
        }
        tx.commit()?;
        reclaim_free_pages(&self.db)?;
        Ok(batch)
    }

    fn compact_usage_observations(
        &mut self,
        now_ms: i64,
        cursor: Option<&sealed_retention::Cursor>,
    ) -> anyhow::Result<sealed_retention::Batch> {
        ensure!(!self.budget_blocked, "{BUDGET_BLOCKED}");
        let tx = self.db.transaction()?;
        let batch = sealed_retention::compact(&tx, now_ms, cursor)?;
        if batch.changed {
            tx.execute(
                "UPDATE usage_meta SET revision = revision + 1 WHERE singleton = 1",
                [],
            )?;
        }
        tx.commit()?;
        if batch.deleted != 0 {
            reclaim_free_pages(&self.db)?;
        }
        Ok(batch)
    }

    /// 回调必须从本事务中的全部来源候选重算受影响会话。
    /// 失败时原游标、既有结果和调用方批次均保留，成功后释放批次中的候选。
    pub(super) fn commit_batch(
        &mut self,
        batch: &mut SourceBatch,
        resolve: impl FnOnce(&Transaction<'_>, &BTreeSet<String>) -> anyhow::Result<Vec<ResolvedEvent>>,
    ) -> anyhow::Result<i64> {
        self.commit_batch_resolved(batch, |tx, sessions, _| {
            resolve(tx, sessions).map(Resolution::Full)
        })
    }

    pub(super) fn commit_batch_resolved(
        &mut self,
        batch: &mut SourceBatch,
        resolve: impl FnOnce(
            &Transaction<'_>,
            &BTreeSet<String>,
            &SourceBatch,
        ) -> anyhow::Result<Resolution>,
    ) -> anyhow::Result<i64> {
        ensure!(!self.budget_blocked, "{BUDGET_BLOCKED}");
        ensure!(
            batch.replace || batch.next_offset >= batch.expected_offset,
            "source offset moved backwards"
        );
        let expected =
            i64::try_from(batch.expected_offset).context("source offset exceeds SQLite integer")?;
        let next =
            i64::try_from(batch.next_offset).context("source offset exceeds SQLite integer")?;
        let tx = self.db.transaction()?;
        let reviews = sealed_retention::guard_batch(&tx, batch)?;
        if !reviews.is_empty() {
            tx.rollback()?;
            let tx = self.db.transaction()?;
            if sealed_retention::mark_review(&tx, &reviews)? {
                tx.execute("UPDATE usage_meta SET revision = revision + 1 WHERE singleton = 1", [])?;
            }
            tx.commit()?;
            return Err(SealedReviewRequired.into());
        }
        let previous: Option<(Vec<u8>, i64, Option<String>)> = tx
            .query_row(
                "SELECT identity, offset, usage_cutoff FROM usage_sources WHERE source = ?1",
                [&batch.source],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let mut affected = BTreeSet::new();
        let cutoff = batch.usage_cutoff.to_string();
        let usage_cutoff = if let Some((identity, offset, previous_cutoff)) = previous {
            ensure!(offset == expected, "stale source offset");
            if batch.replace {
                let mut statement = tx
                    .prepare("SELECT DISTINCT session FROM usage_observations WHERE source = ?1")?;
                let sessions =
                    statement.query_map([&batch.source], |row| row.get::<_, String>(0))?;
                for session in sessions {
                    affected.insert(session?);
                }
                drop(statement);
                tx.execute(
                    "DELETE FROM usage_observations WHERE source = ?1",
                    [&batch.source],
                )?;
            } else {
                ensure!(identity == batch.identity, "source identity changed");
            }
            if batch.replace {
                Some(cutoff)
            } else if next > expected {
                previous_cutoff.map(|previous| previous.max(cutoff))
            } else {
                previous_cutoff
            }
        } else {
            ensure!(expected == 0, "new source must start at zero");
            tx.execute(
                "INSERT INTO usage_sources(source, identity, offset, parser_state, complete)
                 VALUES(?1, ?2, 0, X'', 0)",
                params![batch.source, batch.identity],
            )?;
            Some(cutoff)
        };
        let previous_gaps: BTreeSet<String> = {
            let mut statement =
                tx.prepare("SELECT session FROM usage_count_gaps WHERE source = ?1")?;
            statement
                .query_map([&batch.source], |row| row.get(0))?
                .collect::<Result<_, _>>()?
        };
        {
            let mut insert = tx.prepare_cached(
                "INSERT INTO usage_observations(source, position, event_id, session, payload)
                 VALUES(?1, ?2, ?3, ?4, ?5)",
            )?;
            for observation in &batch.observations {
                let position = i64::try_from(observation.position)
                    .context("event position exceeds SQLite integer")?;
                ensure!(
                    observation.position < batch.next_offset,
                    "event position is outside the source batch"
                );
                ensure!(!observation.session.is_empty(), "event session is empty");
                affected.insert(observation.session.clone());
                insert.execute(params![
                    batch.source,
                    position,
                    observation.event_id.as_slice(),
                    observation.session,
                    observation.payload,
                ])?;
            }
        }
        let mut current_gaps = batch.count_gap_sessions.clone().unwrap_or_default();
        if batch.count_gap_sessions.is_some() {
            let mut statement =
                tx.prepare("SELECT DISTINCT session FROM usage_observations WHERE source = ?1")?;
            for session in statement.query_map([&batch.source], |row| row.get::<_, String>(0))? {
                current_gaps.insert(session?);
            }
        }
        if previous_gaps != current_gaps {
            affected.extend(previous_gaps.symmetric_difference(&current_gaps).cloned());
            tx.execute(
                "DELETE FROM usage_count_gaps WHERE source = ?1",
                [&batch.source],
            )?;
            for session in &current_gaps {
                tx.execute(
                    "INSERT INTO usage_count_gaps(source, session) VALUES (?1, ?2)",
                    params![batch.source, session],
                )?;
            }
        }
        if batch.replace {
            tx.execute(
                "DELETE FROM usage_unindexed_checkpoints WHERE source = ?1",
                [&batch.source],
            )?;
        }
        execution_index::apply(
            &tx,
            &batch.source,
            batch.replace,
            &batch.execution_observations,
            batch.now_ms,
        )?;
        tx.execute(
            "DELETE FROM usage_scope_parents WHERE source = ?1",
            [&batch.source],
        )?;
        {
            let mut insert = tx.prepare_cached(
                "INSERT INTO usage_scope_parents(source, session, parent) VALUES (?1, ?2, ?3)",
            )?;
            for (session, parent) in &batch.scope_parents {
                insert.execute(params![
                    batch.source,
                    session,
                    parent.as_deref().unwrap_or("")
                ])?;
            }
        }
        tx.execute(
            "DELETE FROM usage_scope_aliases WHERE source = ?1",
            [&batch.source],
        )?;
        {
            let mut insert = tx.prepare_cached(
                "INSERT INTO usage_scope_aliases(source, alias, session) VALUES (?1, ?2, ?3)",
            )?;
            for (alias, session) in &batch.scope_aliases {
                insert.execute(params![batch.source, alias, session])?;
            }
        }
        tx.execute(
            "UPDATE usage_sources SET identity = ?2, offset = ?3, parser_state = ?4, complete = ?5,
                 scope_session = ?6, scope_indexed = 1, scope_valid = ?7,
                 execution_indexed = 1, execution_valid = ?8, usage_cutoff = ?9
             WHERE source = ?1",
            params![
                batch.source,
                batch.identity,
                next,
                batch.parser_state,
                batch.complete,
                batch.scope_session,
                batch.scope_valid,
                batch.execution_valid,
                usage_cutoff
            ],
        )?;
        let sealed = sealed_retention::ranges(&tx, &affected)?;
        let mut resolution = resolve(&tx, &affected, batch)?;
        let reviews = sealed_retention::filter_resolution(&sealed, batch, &mut resolution);
        if !reviews.is_empty() {
            tx.rollback()?;
            let tx = self.db.transaction()?;
            if sealed_retention::mark_review(&tx, &reviews)? {
                tx.execute("UPDATE usage_meta SET revision = revision + 1 WHERE singleton = 1", [])?;
            }
            tx.commit()?;
            return Err(SealedReviewRequired.into());
        }
        let (mut events, append) = match resolution {
            Resolution::Full(events) => (events, false),
            Resolution::Append(events) => {
                ensure!(
                    !batch.replace && affected.len() == 1,
                    "incremental resolution requires one appended session"
                );
                (events, true)
            }
        };
        events.sort_unstable_by(|a, b| a.session.cmp(&b.session).then(a.id.cmp(&b.id)));
        for (index, event) in events.iter().enumerate() {
            ensure!(
                affected.contains(&event.session),
                "resolved event outside affected sessions"
            );
            ensure!(
                index == 0
                    || events[index - 1].session != event.session
                    || events[index - 1].id != event.id,
                "duplicate resolved event"
            );
            ensure!(
                event.cached.is_none_or(|cached| cached <= event.input)
                    && event
                        .reasoning
                        .is_none_or(|reasoning| reasoning <= event.output),
                "resolved event has invalid token components"
            );
            chrono::NaiveDate::parse_from_str(&event.day, "%Y-%m-%d")
                .context("resolved event has invalid day")?;
            i64::try_from(event.input).context("input count exceeds SQLite integer")?;
            event
                .cached
                .map(i64::try_from)
                .transpose()
                .context("cached count exceeds SQLite integer")?;
            i64::try_from(event.output).context("output count exceeds SQLite integer")?;
            event
                .reasoning
                .map(i64::try_from)
                .transpose()
                .context("reasoning count exceeds SQLite integer")?;
        }

        let mut day_only = BTreeSet::new();
        {
            let mut statement =
                tx.prepare_cached("SELECT day FROM usage_day_only WHERE session = ?1")?;
            for session in &affected {
                for day in statement.query_map([session], |row| row.get::<_, String>(0))? {
                    day_only.insert((session.clone(), day?));
                }
            }
        }
        let (events, summarized): (Vec<_>, Vec<_>) = events
            .into_iter()
            .partition(|event| !day_only.contains(&(event.session.clone(), event.day.clone())));
        if !append {
            for (session, day) in &day_only {
                if sealed.iter().any(|range| range.contains(session, day)) {
                    continue;
                }
                tx.execute(
                    "DELETE FROM usage_daily WHERE session = ?1 AND day = ?2",
                    params![session, day],
                )?;
            }
        }
        let mut deletes = Vec::new();
        let mut writes = Vec::new();
        let mut days = BTreeSet::new();
        let mut event_index = 0;
        if append {
            writes.extend(0..events.len());
        } else {
            for session in &affected {
                let end = event_index
                    + events[event_index..].partition_point(|event| &event.session == session);
                let mut statement = tx.prepare_cached(
                    "SELECT id, session, day, timestamp_ms, model, provider, provider_source, role,
                        context_conflicts, count_uncertain, count_incomplete,
                        input, cached, output, reasoning
                 FROM usage_events WHERE session = ?1 ORDER BY id",
                )?;
                let mut rows = statement.query([session])?;
                while let Some(row) = rows.next()? {
                    let id: Vec<u8> = row.get(0)?;
                    let id: [u8; 32] = id
                        .try_into()
                        .map_err(|_| anyhow::anyhow!("invalid stored event id"))?;
                    let previous = ResolvedEvent {
                        id,
                        session: row.get(1)?,
                        day: row.get(2)?,
                        timestamp_ms: row.get(3)?,
                        model: row.get(4)?,
                        provider: row.get(5)?,
                        provider_source: row.get(6)?,
                        role: row.get(7)?,
                        context_conflicts: row.get(8)?,
                        count_uncertain: row.get(9)?,
                        count_incomplete: row.get(10)?,
                        input: row.get(11)?,
                        cached: row.get(12)?,
                        output: row.get(13)?,
                        reasoning: row.get(14)?,
                    };
                    while event_index < end && events[event_index].id < previous.id {
                        days.insert((session.clone(), events[event_index].day.clone()));
                        writes.push(event_index);
                        event_index += 1;
                    }
                    if event_index == end || events[event_index].id > previous.id {
                        days.insert((session.clone(), previous.day));
                        deletes.push(previous.id);
                    } else {
                        if events[event_index] != previous {
                            days.insert((session.clone(), previous.day));
                            days.insert((session.clone(), events[event_index].day.clone()));
                            writes.push(event_index);
                        }
                        event_index += 1;
                    }
                }
                while event_index < end {
                    days.insert((session.clone(), events[event_index].day.clone()));
                    writes.push(event_index);
                    event_index += 1;
                }
            }
        }
        {
            let mut delete = tx.prepare_cached("DELETE FROM usage_events WHERE id = ?1")?;
            for id in deletes {
                delete.execute([id.as_slice()])?;
            }
            let insert =
                "INSERT INTO usage_events(id, session, day, timestamp_ms, model, provider, role,
                    provider_source, context_conflicts, count_uncertain, count_incomplete,
                    input, cached, output, reasoning)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)";
            let upsert = format!(
                "{insert}
                 ON CONFLICT(id) DO UPDATE SET
                    day = excluded.day, timestamp_ms = excluded.timestamp_ms,
                    model = excluded.model, provider = excluded.provider, role = excluded.role,
                    provider_source = excluded.provider_source,
                    context_conflicts = excluded.context_conflicts,
                    count_uncertain = excluded.count_uncertain,
                    count_incomplete = excluded.count_incomplete,
                    input = excluded.input, cached = excluded.cached, output = excluded.output,
                    reasoning = excluded.reasoning
                 WHERE usage_events.session = excluded.session"
            );
            let mut write = tx.prepare_cached(if append { insert } else { &upsert })?;
            for index in writes {
                let event = &events[index];
                let changed = write.execute(params![
                    event.id.as_slice(),
                    event.session,
                    event.day,
                    event.timestamp_ms,
                    event.model,
                    event.provider,
                    event.role,
                    event.provider_source,
                    event.context_conflicts,
                    event.count_uncertain,
                    event.count_incomplete,
                    i64::try_from(event.input)?,
                    event.cached.map(i64::try_from).transpose()?,
                    i64::try_from(event.output)?,
                    event.reasoning.map(i64::try_from).transpose()?,
                ])?;
                ensure!(changed == 1, "event id belongs to another session");
            }
        }
        if append || !summarized.is_empty() {
            let mut merge = tx.prepare_cached(
                "INSERT INTO usage_daily(day, session, model, provider, provider_source, role,
                    context_conflicts, input, cached, cached_known_sum, output, reasoning, records,
                    uncertain_events, total_events)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, 1)
                 ON CONFLICT(day, session, model, provider, provider_source, role, context_conflicts)
                 DO UPDATE SET
                    input = usage_daily.input + excluded.input,
                    cached = CASE
                        WHEN excluded.records = 0 THEN usage_daily.cached
                        WHEN usage_daily.records = 0 THEN excluded.cached
                        WHEN usage_daily.cached IS NULL OR excluded.cached IS NULL THEN NULL
                        ELSE usage_daily.cached + excluded.cached END,
                    cached_known_sum = usage_daily.cached_known_sum + excluded.cached_known_sum,
                    output = usage_daily.output + excluded.output,
                    reasoning = CASE
                        WHEN excluded.records = 0 THEN usage_daily.reasoning
                        WHEN usage_daily.records = 0 THEN excluded.reasoning
                        WHEN usage_daily.reasoning IS NULL OR excluded.reasoning IS NULL THEN NULL
                        ELSE usage_daily.reasoning + excluded.reasoning END,
                    records = usage_daily.records + excluded.records,
                    uncertain_events = usage_daily.uncertain_events + excluded.uncertain_events,
                    total_events = usage_daily.total_events + 1
                 WHERE usage_daily.input <= 9223372036854775807 - excluded.input
                   AND usage_daily.cached_known_sum <= 9223372036854775807 - excluded.cached_known_sum
                   AND usage_daily.output <= 9223372036854775807 - excluded.output
                   AND COALESCE(usage_daily.cached <= 9223372036854775807 - excluded.cached, 1)
                   AND COALESCE(usage_daily.reasoning <= 9223372036854775807 - excluded.reasoning, 1)
                   AND usage_daily.records <= 9223372036854775807 - excluded.records
                   AND usage_daily.uncertain_events <= 9223372036854775807 - excluded.uncertain_events
                   AND usage_daily.total_events < 9223372036854775807",
            )?;
            for event in events.iter().filter(|_| append).chain(&summarized) {
                let valid = !event.count_uncertain
                    && !event.count_incomplete
                    && (event.input > 0 || event.output > 0);
                let cached = if valid {
                    event.cached.map(i64::try_from).transpose()?
                } else {
                    None
                };
                let reasoning = if valid {
                    event.reasoning.map(i64::try_from).transpose()?
                } else {
                    None
                };
                let changed = merge.execute(params![
                    event.day,
                    event.session,
                    event.model,
                    event.provider,
                    event.provider_source.as_deref().unwrap_or(""),
                    event.role,
                    event.context_conflicts,
                    if valid {
                        i64::try_from(event.input)?
                    } else {
                        0
                    },
                    cached,
                    cached.unwrap_or(0),
                    if valid {
                        i64::try_from(event.output)?
                    } else {
                        0
                    },
                    reasoning,
                    i64::from(valid),
                    i64::from(event.count_uncertain || event.count_incomplete),
                ])?;
                ensure!(changed == 1, "daily usage summary exceeds SQLite integer");
            }
        }
        if !append {
            for (session, day) in days {
                if day_only.contains(&(session.clone(), day.clone()))
                    || sealed.iter().any(|range| range.contains(&session, &day))
                {
                    continue;
                }
                tx.execute(
                    "DELETE FROM usage_daily WHERE session = ?1 AND day = ?2",
                    params![session, day],
                )?;
                populate_daily(&tx, Some((&session, &day)))?;
            }
        }
        let revision: i64 = tx.query_row(
            "UPDATE usage_meta SET revision = revision + 1 WHERE singleton = 1 RETURNING revision",
            [],
            |row| row.get(0),
        )?;
        tx.commit()?;
        batch.observations = Vec::new();
        batch.execution_observations = Vec::new();
        Ok(revision)
    }
}

fn reclaim_free_pages(db: &Connection) -> anyhow::Result<()> {
    // incremental_vacuum 每回收一页返回一行，必须消费到结束才能完成本批。
    let mut statement = db.prepare("PRAGMA incremental_vacuum(128)")?;
    let mut rows = statement.query([])?;
    while rows.next()?.is_some() {}
    Ok(())
}

fn populate_daily(tx: &Transaction<'_>, scope: Option<(&str, &str)>) -> anyhow::Result<()> {
    let valid = "count_uncertain = 0 AND count_incomplete = 0 AND (input > 0 OR output > 0)";
    let filter = if scope.is_some() {
        "WHERE session = ?1 AND day = ?2"
    } else {
        ""
    };
    let sql = format!(
        "INSERT INTO usage_daily(day, session, model, provider, provider_source, role,
            context_conflicts, input, cached, cached_known_sum, output, reasoning, records,
            uncertain_events, total_events)
         SELECT day, session, model, provider, COALESCE(provider_source, ''), role,
            context_conflicts,
            SUM(CASE WHEN {valid} THEN input ELSE 0 END),
            CASE WHEN SUM(CASE WHEN {valid} THEN 1 ELSE 0 END) > 0
                      AND SUM(CASE WHEN {valid} AND cached IS NULL THEN 1 ELSE 0 END) = 0
                 THEN SUM(CASE WHEN {valid} THEN COALESCE(cached, 0) ELSE 0 END)
                 ELSE NULL END,
            SUM(CASE WHEN {valid} THEN COALESCE(cached, 0) ELSE 0 END),
            SUM(CASE WHEN {valid} THEN output ELSE 0 END),
            CASE WHEN SUM(CASE WHEN {valid} THEN 1 ELSE 0 END) > 0
                      AND SUM(CASE WHEN {valid} AND reasoning IS NULL THEN 1 ELSE 0 END) = 0
                 THEN SUM(CASE WHEN {valid} THEN COALESCE(reasoning, 0) ELSE 0 END)
                 ELSE NULL END,
            SUM(CASE WHEN {valid} THEN 1 ELSE 0 END),
            SUM(CASE WHEN count_uncertain != 0 OR count_incomplete != 0 THEN 1 ELSE 0 END),
            COUNT(*)
         FROM usage_events {filter}
         GROUP BY day, session, model, provider, COALESCE(provider_source, ''), role,
            context_conflicts"
    );
    if let Some((session, day)) = scope {
        tx.execute(&sql, params![session, day])?;
    } else {
        tx.execute(&sql, [])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
