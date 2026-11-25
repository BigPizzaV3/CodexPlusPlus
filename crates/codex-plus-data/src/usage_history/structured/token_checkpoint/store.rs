use super::{Checkpoint, Counter, FileState, Observation, UsageEvent};
use anyhow::{Result, ensure};
use chrono::{DateTime, Local};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::ffi::OsString;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

pub(super) mod archive;
pub(super) mod executions;
pub(super) mod projection;
pub(super) mod replicas;

#[cfg(unix)]
fn encode_path(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().to_vec()
}

#[cfg(unix)]
fn decode_path(bytes: Vec<u8>) -> Result<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    Ok(OsString::from_vec(bytes).into())
}

#[cfg(windows)]
fn encode_path(path: &Path) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str()
        .encode_wide()
        .flat_map(u16::to_le_bytes)
        .collect()
}

#[cfg(windows)]
fn decode_path(bytes: Vec<u8>) -> Result<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    let chunks = bytes.chunks_exact(2);
    ensure!(
        chunks.remainder().is_empty(),
        "invalid encoded Token source path"
    );
    let wide: Vec<_> = chunks
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    Ok(OsString::from_wide(&wide).into())
}

pub(super) struct Store {
    pub(super) db: Connection,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in super::super) struct Coverage {
    pub(in super::super) pending_sources: usize,
    pub(in super::super) issues: usize,
    pub(in super::super) maintenance_blocked: bool,
}

impl Store {
    pub(super) fn open(db: Connection) -> Result<Self> {
        db.pragma_update(None, "foreign_keys", true)?;
        db.pragma_update(None, "journal_mode", "WAL")?;
        let compactable_metadata = executions::COMPACTABLE_METADATA;
        db.execute_batch(&format!(
            "BEGIN IMMEDIATE;
             CREATE TABLE IF NOT EXISTS token_revision (
                singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                revision INTEGER NOT NULL,
                precise_from TEXT NOT NULL,
                updated_at INTEGER NOT NULL DEFAULT 0
             ) STRICT;
             INSERT OR IGNORE INTO token_revision (singleton, revision, precise_from) VALUES (1, 0, '');
             CREATE TABLE IF NOT EXISTS token_coverage (
                singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                revision INTEGER NOT NULL,
                pending_sources INTEGER NOT NULL,
                issues INTEGER NOT NULL,
                maintenance_blocked INTEGER NOT NULL
             ) STRICT;
             CREATE TABLE IF NOT EXISTS token_sources (
                id INTEGER PRIMARY KEY,
                source BLOB NOT NULL UNIQUE,
                revision INTEGER NOT NULL,
                offset INTEGER NOT NULL,
                checkpoint BLOB NOT NULL,
                activity_truncated INTEGER NOT NULL
             ) STRICT;
             CREATE TABLE IF NOT EXISTS token_sessions (
                id INTEGER PRIMARY KEY,
                session TEXT NOT NULL UNIQUE
             ) STRICT;
             CREATE TABLE IF NOT EXISTS token_observations (
                source_id INTEGER NOT NULL REFERENCES token_sources(id),
                revision INTEGER NOT NULL,
                position INTEGER NOT NULL,
                session_id INTEGER NOT NULL REFERENCES token_sessions(id),
                event_id BLOB NOT NULL,
                payload BLOB NOT NULL,
                PRIMARY KEY (source_id, revision, position)
             ) WITHOUT ROWID, STRICT;
             CREATE INDEX IF NOT EXISTS token_terminal_response_identity
                ON token_observations(event_id)
                WHERE json_type(CAST(payload AS TEXT), '$.terminal_response') = 'text';
             CREATE INDEX IF NOT EXISTS token_terminal_claim
                ON token_observations(session_id,
                   json_extract(CAST(payload AS TEXT), '$.turn'),
                   CAST(payload AS TEXT) -> '$.total.i', CAST(payload AS TEXT) -> '$.total.o')
                WHERE json_type(CAST(payload AS TEXT), '$.terminal_response') = 'text';
             CREATE TABLE IF NOT EXISTS token_source_links (
                source_id INTEGER NOT NULL REFERENCES token_sources(id),
                link TEXT NOT NULL,
                PRIMARY KEY (source_id, link)
             ) WITHOUT ROWID, STRICT;
             CREATE INDEX IF NOT EXISTS token_link_sources ON token_source_links(link);
             CREATE TABLE IF NOT EXISTS token_linear_sources (
                source_id INTEGER PRIMARY KEY REFERENCES token_sources(id),
                revision INTEGER NOT NULL,
                cutoff TEXT NOT NULL
             ) STRICT;
             CREATE TABLE IF NOT EXISTS token_source_copies (
                source BLOB PRIMARY KEY,
                owner_id INTEGER NOT NULL REFERENCES token_sources(id) ON DELETE CASCADE,
                stamp BLOB NOT NULL
             ) WITHOUT ROWID, STRICT;
             CREATE INDEX IF NOT EXISTS token_copy_owner ON token_source_copies(owner_id);
             CREATE TABLE IF NOT EXISTS token_collisions (
                id BLOB PRIMARY KEY
             ) WITHOUT ROWID, STRICT;
             CREATE TABLE IF NOT EXISTS token_resolved (
                event_id BLOB PRIMARY KEY,
                session_id INTEGER NOT NULL REFERENCES token_sessions(id),
                timestamp_ms INTEGER NOT NULL,
                day TEXT NOT NULL,
                payload BLOB NOT NULL
             ) WITHOUT ROWID, STRICT;
             CREATE INDEX IF NOT EXISTS token_resolved_time ON token_resolved(timestamp_ms);
             CREATE INDEX IF NOT EXISTS token_resolved_session_time ON token_resolved(session_id, timestamp_ms);
             CREATE INDEX IF NOT EXISTS token_resolved_session_day ON token_resolved(session_id, day);
             CREATE INDEX IF NOT EXISTS token_resolved_count_gaps ON token_resolved(session_id, day)
                WHERE json_extract(CAST(payload AS TEXT), '$.count_incomplete') = 1
                   OR json_extract(CAST(payload AS TEXT), '$.delta.k') = 0;
             DROP INDEX IF EXISTS token_resolved_session;
             CREATE INDEX IF NOT EXISTS token_resolved_open_receipt
                ON token_resolved(session_id, day)
                WHERE json_type(CAST(payload AS TEXT), '$.terminal_response') = 'text';
             CREATE INDEX IF NOT EXISTS token_classified_time
                ON token_resolved(session_id, timestamp_ms)
                WHERE json_type(CAST(payload AS TEXT), '$.classification_origin') = 'object';
             CREATE TABLE IF NOT EXISTS token_days (
                session_id INTEGER NOT NULL REFERENCES token_sessions(id),
                day TEXT NOT NULL,
                groups BLOB NOT NULL,
                PRIMARY KEY (session_id, day)
             ) WITHOUT ROWID, STRICT;
             CREATE INDEX IF NOT EXISTS token_days_date ON token_days(day);
             CREATE TABLE IF NOT EXISTS token_activity (
                source_id INTEGER NOT NULL REFERENCES token_sources(id) ON DELETE CASCADE,
                session_id INTEGER NOT NULL REFERENCES token_sessions(id),
                call_id TEXT NOT NULL,
                timestamp_ms INTEGER NOT NULL,
                day TEXT NOT NULL,
                payload BLOB NOT NULL,
                PRIMARY KEY (source_id, session_id, call_id)
             ) WITHOUT ROWID, STRICT;
             CREATE INDEX IF NOT EXISTS token_activity_time ON token_activity(timestamp_ms);
             CREATE INDEX IF NOT EXISTS token_activity_call ON token_activity(session_id, call_id);
             CREATE TABLE IF NOT EXISTS execution_observations (
                source_id INTEGER NOT NULL REFERENCES token_sources(id) ON DELETE CASCADE,
                session_id INTEGER NOT NULL REFERENCES token_sessions(id),
                position INTEGER NOT NULL,
                timestamp_ms INTEGER NOT NULL,
                call_id TEXT NOT NULL,
                end INTEGER,
                is_output INTEGER NOT NULL,
                name TEXT,
                namespace TEXT,
                PRIMARY KEY (source_id, position)
             ) WITHOUT ROWID, STRICT;
             CREATE INDEX IF NOT EXISTS execution_by_time
                ON execution_observations (timestamp_ms, session_id, call_id) WHERE is_output = 0;
             CREATE INDEX IF NOT EXISTS execution_by_call
                ON execution_observations (session_id, call_id);
             CREATE INDEX IF NOT EXISTS execution_locator_expiry
                ON execution_observations (is_output, timestamp_ms) WHERE end IS NOT NULL;
             CREATE INDEX IF NOT EXISTS execution_metadata_expiry
                ON execution_observations (timestamp_ms)
                WHERE is_output = 0 AND {compactable_metadata};
             CREATE TABLE IF NOT EXISTS execution_process (
                source_id INTEGER NOT NULL,
                position INTEGER NOT NULL,
                process TEXT NOT NULL,
                PRIMARY KEY(source_id, position),
                FOREIGN KEY(source_id, position)
                    REFERENCES execution_observations(source_id, position) ON DELETE CASCADE
             ) WITHOUT ROWID, STRICT;
             CREATE INDEX IF NOT EXISTS execution_process_by_id ON execution_process(process);
             CREATE TABLE IF NOT EXISTS execution_daily (
                day TEXT NOT NULL,
                session_id INTEGER NOT NULL REFERENCES token_sessions(id),
                count INTEGER NOT NULL,
                skill INTEGER NOT NULL,
                mcp INTEGER NOT NULL,
                tool INTEGER NOT NULL,
                partial INTEGER NOT NULL,
                PRIMARY KEY(day, session_id)
             ) WITHOUT ROWID, STRICT;
             CREATE TABLE IF NOT EXISTS execution_coverage_start (
                singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                started_at INTEGER
             ) STRICT;
             CREATE TABLE IF NOT EXISTS execution_evicted (
                singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                through_day TEXT NOT NULL
             ) STRICT;
             INSERT OR IGNORE INTO execution_coverage_start
                SELECT 1, CASE WHEN EXISTS (SELECT 1 FROM token_sources)
                    THEN CAST(unixepoch('subsec') * 1000 AS INTEGER) END;
             COMMIT;",
        ))?;
        Ok(Self { db })
    }

    pub(super) fn record_coverage(&self, coverage: Coverage) -> Result<()> {
        self.db.execute(
            "INSERT INTO token_coverage
             SELECT 1, revision, ?1, ?2, ?3 FROM token_revision WHERE singleton = 1
             ON CONFLICT(singleton) DO UPDATE SET revision = excluded.revision,
                 pending_sources = excluded.pending_sources, issues = excluded.issues,
                 maintenance_blocked = excluded.maintenance_blocked
             WHERE (token_coverage.revision, token_coverage.pending_sources, token_coverage.issues,
                    token_coverage.maintenance_blocked) IS DISTINCT FROM
                   (excluded.revision, excluded.pending_sources, excluded.issues,
                    excluded.maintenance_blocked)",
            params![
                coverage.pending_sources,
                coverage.issues,
                coverage.maintenance_blocked
            ],
        )?;
        Ok(())
    }

    pub(super) fn expire_execution_locators(&mut self, now_ms: i64, batch: u32) -> Result<()> {
        let tx = self.db.transaction()?;
        let cutoff = now_ms - super::super::execution_observations::DETAIL_MS;
        tx.execute(
            "DELETE FROM execution_observations WHERE (source_id, position) IN (
                 SELECT source_id, position FROM execution_observations
                 WHERE end IS NOT NULL AND timestamp_ms <= ?1 AND is_output = 1
                 ORDER BY timestamp_ms LIMIT ?2
             )",
            params![cutoff, batch],
        )?;
        tx.execute(
            "DELETE FROM execution_process WHERE (source_id, position) IN (
                SELECT source_id, position FROM execution_observations
                WHERE end IS NOT NULL AND timestamp_ms <= ?1 AND is_output = 0
                ORDER BY timestamp_ms LIMIT ?2
             )",
            params![cutoff, batch],
        )?;
        tx.execute(
            "UPDATE execution_observations SET end = NULL WHERE (source_id, position) IN (
                 SELECT source_id, position FROM execution_observations
                 WHERE end IS NOT NULL AND timestamp_ms <= ?1 AND is_output = 0
                 ORDER BY timestamp_ms LIMIT ?2
             )",
            params![cutoff, batch],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub(super) fn save(
        &mut self,
        source: &Path,
        expected_revision: Option<i64>,
        state: &FileState,
        replace: bool,
    ) -> Result<i64> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let revision = Self::stage(&tx, source, expected_revision, state, replace)?;
        tx.commit()?;
        Ok(revision)
    }

    pub(super) fn stage(
        tx: &Transaction<'_>,
        source: &Path,
        expected_revision: Option<i64>,
        state: &FileState,
        replace: bool,
    ) -> Result<i64> {
        ensure!(!state.skipping, "cannot checkpoint within a skipped line");
        let (offset, mut checkpoint) = Checkpoint::capture(state);
        let path = encode_path(source);
        let previous: Option<(i64, i64, u64)> = tx
            .query_row(
                "SELECT id, revision, offset FROM token_sources WHERE source = ?1",
                [&path],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        ensure!(
            previous.map(|row| row.1) == expected_revision,
            "Token source changed before commit"
        );
        if replace && let Some((id, _, _)) = previous {
            ensure!(
                !tx.prepare_cached(
                    "SELECT 1 FROM token_source_copies WHERE owner_id = ?1 LIMIT 1"
                )?
                .exists([id])?,
                "Token source was replaced while a verified copy still needs reconciliation"
            );
        }
        if !replace && let Some((_, _, old_offset)) = previous {
            ensure!(
                offset > old_offset || offset == old_offset && state.events.is_empty(),
                "Token source offset did not advance"
            );
            let (prior, cursors): (Option<String>, Option<String>) = tx.query_row(
                "SELECT json_extract(CAST(checkpoint AS TEXT), '$.expired_count'),
                        json_extract(CAST(checkpoint AS TEXT), '$.committed_cursors')
                 FROM token_sources WHERE source = ?1",
                [&path],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            if let Some(prior) = prior {
                let prior: ((u64, u64, u64), super::Counter) = serde_json::from_str(&prior)?;
                if checkpoint
                    .expired_count
                    .as_ref()
                    .is_none_or(|(current, _)| prior.0 > *current)
                {
                    checkpoint.expired_count = Some(prior);
                }
            }
            if let Some(cursors) = cursors {
                for (session, older) in
                    serde_json::from_str::<BTreeMap<String, (String, Counter)>>(&cursors)?
                {
                    let current = checkpoint
                        .committed_cursors
                        .entry(session)
                        .or_insert_with(|| older.clone());
                    if older.0 > current.0 {
                        *current = older;
                    }
                }
            }
        }
        let revision = tx.query_row(
            "UPDATE token_revision SET revision = revision + 1,
                updated_at = max(updated_at + 1, ?1) RETURNING revision",
            [Local::now().timestamp_millis()],
            |row| row.get::<_, i64>(0),
        )?;
        let source_id = tx.query_row(
            "INSERT INTO token_sources (source, revision, offset, checkpoint, activity_truncated)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (source) DO UPDATE SET revision = excluded.revision, offset = excluded.offset,
                checkpoint = excluded.checkpoint, activity_truncated = excluded.activity_truncated
             RETURNING id",
            params![path, revision, offset, serde_json::to_vec(&checkpoint)?, state.activity_truncated],
            |row| row.get::<_, i64>(0),
        )?;
        if previous.is_none() {
            tx.execute("DELETE FROM token_source_copies WHERE source = ?1", [&path])?;
        }
        if replace {
            tx.execute(
                "DELETE FROM token_observations WHERE source_id = ?1",
                [source_id],
            )?;
            tx.execute(
                "DELETE FROM token_source_links WHERE source_id = ?1",
                [source_id],
            )?;
            tx.execute(
                "DELETE FROM token_activity WHERE source_id = ?1",
                [source_id],
            )?;
            tx.execute(
                "DELETE FROM execution_observations WHERE source_id = ?1",
                [source_id],
            )?;
        }
        {
            let mut insert = tx.prepare(
                "INSERT OR IGNORE INTO token_source_links (source_id, link) VALUES (?1, ?2)",
            )?;
            for link in projection::links(source, state) {
                insert.execute(params![source_id, link])?;
            }
        }
        {
            let mut add_session =
                tx.prepare("INSERT OR IGNORE INTO token_sessions (session) VALUES (?1)")?;
            let mut insert = tx.prepare(
                "INSERT INTO token_observations
                 SELECT ?1, ?2, ?3, id, ?5, ?6 FROM token_sessions WHERE session = ?4",
            )?;
            for (index, event) in state.events.iter().enumerate() {
                add_session.execute([&event.session])?;
                insert.execute(params![
                    source_id,
                    revision,
                    index,
                    event.session,
                    event.id.as_slice(),
                    serde_json::to_vec(&Observation::capture(event))?,
                ])?;
            }
            let mut activity = tx.prepare(
                "INSERT INTO token_activity
                 SELECT ?1, id, ?3, ?4, ?5, ?6 FROM token_sessions WHERE session = ?2
                 ON CONFLICT(source_id, session_id, call_id) DO UPDATE SET
                    timestamp_ms = excluded.timestamp_ms, day = excluded.day, payload = excluded.payload",
            )?;
            for tool in state.tools.iter().filter(|_| !state.identity_conflict) {
                add_session.execute([&tool.call.session])?;
                activity.execute(params![
                    source_id,
                    tool.call.session,
                    tool.id,
                    DateTime::parse_from_rfc3339(&tool.call.timestamp)?.timestamp_millis(),
                    tool.date.to_string(),
                    serde_json::to_vec(&serde_json::json!({
                        "conversationKey":tool.call.session,
                        "timestamp":tool.call.timestamp,
                        "call":tool.call.payload,
                        "output":tool.output,
                    }))?,
                ])?;
            }
            if let Some(batch) = &state.executions
                && !batch.rows.is_empty()
                && !state.identity_conflict
            {
                add_session.execute([&state.session])?;
                let mut insert = tx.prepare(
                    "INSERT INTO execution_observations
                     SELECT ?1, id, ?3, ?4, ?5, ?6, ?7, ?8, ?9 FROM token_sessions WHERE session = ?2",
                )?;
                let mut process = tx.prepare("INSERT INTO execution_process VALUES(?1,?2,?3)")?;
                for row in &batch.rows {
                    insert.execute(params![
                        source_id,
                        state.session,
                        row.position,
                        row.timestamp_ms,
                        row.call_id,
                        row.end,
                        row.is_output,
                        row.name,
                        row.namespace,
                    ])?;
                    if let Some(key) = &row.process {
                        process.execute(params![source_id, row.position, key])?;
                    }
                }
            }
        }
        Ok(revision)
    }

    pub(super) fn checkpoint(&self, source: &Path) -> Result<Option<(i64, FileState)>> {
        let row: Option<(i64, u64, Vec<u8>)> = self
            .db
            .query_row(
                "SELECT revision, offset, checkpoint FROM token_sources WHERE source = ?1",
                [encode_path(source)],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        row.map(|(revision, offset, payload)| {
            Ok((
                revision,
                serde_json::from_slice::<Checkpoint>(&payload)?.restore(offset),
            ))
        })
        .transpose()
    }

    pub(super) fn missing_sources(
        &self,
        discovered: &[PathBuf],
        limit: usize,
    ) -> Result<(VecDeque<(PathBuf, i64)>, bool)> {
        let present: HashSet<_> = discovered.iter().map(PathBuf::as_path).collect();
        self.remove_missing_copies(&present)?;
        let mut statement = self
            .db
            .prepare("SELECT source, revision FROM token_sources ORDER BY id")?;
        let mut rows = statement.query([])?;
        let mut missing = VecDeque::new();
        while let Some(row) = rows.next()? {
            let source = decode_path(row.get(0)?)?;
            if !present.contains(source.as_path()) {
                if missing.len() == limit {
                    return Ok((missing, true));
                }
                missing.push_back((source, row.get(1)?));
            }
        }
        Ok((missing, false))
    }

    pub(super) fn relocate_source(
        &mut self,
        old: &Path,
        expected_revision: i64,
        new: &Path,
    ) -> Result<bool> {
        if old.file_name() != new.file_name() || self.checkpoint(new)?.is_some() {
            return Ok(false);
        }
        let Some((revision, state)) = self.checkpoint(old)? else {
            return Ok(false);
        };
        if revision != expected_revision {
            return Ok(false);
        }
        let Some(stamp @ (_, Some(_), Some(_))) = state.stamp else {
            return Ok(false);
        };
        match std::fs::symlink_metadata(old) {
            Ok(_) => return Ok(false),
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        // 文件名保留物理 rollout 标识；句柄身份、长度和写入时间均须匹配。
        if FileState::source_stamp(new)? != stamp {
            return Ok(false);
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = tx.execute(
            "UPDATE token_sources SET source = ?1 WHERE source = ?2 AND revision = ?3
             AND NOT EXISTS (SELECT 1 FROM token_sources WHERE source = ?1)",
            params![encode_path(new), encode_path(old), revision],
        )?;
        if changed == 1 {
            tx.execute(
                "DELETE FROM token_source_copies WHERE source = ?1",
                [encode_path(new)],
            )?;
            // 内容与计数未变，但必须让旧路径上的异步回读判定过期。
            tx.execute(
                "UPDATE token_revision SET revision = revision + 1,
                    updated_at = max(updated_at + 1, ?1)",
                [Local::now().timestamp_millis()],
            )?;
        }
        tx.commit()?;
        Ok(changed == 1)
    }

    pub(super) fn observations(&self, source: &Path) -> Result<Vec<UsageEvent>> {
        let mut stmt = self.db.prepare(
            "SELECT o.event_id, s.session, o.payload FROM token_observations o
             JOIN token_sources t ON t.id = o.source_id
             JOIN token_sessions s ON s.id = o.session_id
             WHERE t.source = ?1 ORDER BY o.revision, o.position",
        )?;
        let mut rows = stmt.query([encode_path(source)])?;
        let mut observations = Vec::new();
        while let Some(row) = rows.next()? {
            let id = row.get_ref(0)?.as_blob()?.try_into()?;
            let observation: Observation = serde_json::from_slice(row.get_ref(2)?.as_blob()?)?;
            observations.push(observation.into_event(id, row.get(1)?)?);
        }
        Ok(observations)
    }
}
