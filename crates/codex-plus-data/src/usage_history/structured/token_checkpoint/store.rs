use super::{Checkpoint, Counter, FileState, Observation, UsageEvent};
use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Local};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::ffi::OsString;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

pub(super) mod archive;
pub(super) mod executions;
pub(super) mod observation_encoding;
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
             CREATE TABLE IF NOT EXISTS token_attribution_coverage (
                singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                revision INTEGER NOT NULL,
                blocked_sessions TEXT NOT NULL
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
             CREATE TABLE IF NOT EXISTS token_observation_encoding (
                source_id INTEGER PRIMARY KEY REFERENCES token_sources(id) ON DELETE CASCADE,
                revision INTEGER NOT NULL, position INTEGER NOT NULL
             ) STRICT;
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

    pub(super) fn record_coverage(
        &mut self,
        coverage: Coverage,
        unknown_count_scope: bool,
    ) -> Result<()> {
        let tx = self.db.transaction()?;
        if unknown_count_scope {
            projection::retract_attribution(&tx, None)?;
        }
        tx.execute("DELETE FROM token_attribution_coverage", [])?;
        Self::write_coverage(&tx, coverage)?;
        Ok(tx.commit()?)
    }

    pub(super) fn record_scoped_coverage(
        &mut self,
        coverage: Coverage,
        invalidated_sources: &BTreeSet<PathBuf>,
        rechecking_sources: &BTreeSet<PathBuf>,
    ) -> Result<()> {
        let tx = self.db.transaction()?;
        let affected_sessions = |sources: &BTreeSet<PathBuf>| -> Result<String> {
            if sources.is_empty() {
                return Ok("[]".to_owned());
            }
            let mut lookup = tx.prepare_cached("SELECT id FROM token_sources WHERE source=?1")?;
            let sources = sources
                .iter()
                .map(|path| lookup.query_row([encode_path(path)], |row| row.get::<_, i64>(0)))
                .collect::<rusqlite::Result<Vec<_>>>()?;
            // Both physical rollout aliases and canonical sessions participate in the closure.
            Ok(tx.query_row(
                "WITH RECURSIVE component(id,link) AS (
                    SELECT value,NULL FROM json_each(?1)
                    UNION SELECT NULL,l.link FROM component c JOIN token_source_links l ON l.source_id=c.id
                    UNION SELECT l.source_id,NULL FROM component c JOIN token_source_links l ON l.link=c.link
                 )
                 SELECT json_group_array(s.id ORDER BY s.id) FROM component c
                 JOIN token_sessions s ON s.session=substr(c.link,3) WHERE c.link GLOB 's:*'",
                [serde_json::to_string(&sources)?],
                |row| row.get::<_, String>(0),
            )?)
        };
        if !invalidated_sources.is_empty() {
            projection::retract_attribution(&tx, Some(&affected_sessions(invalidated_sources)?))?;
        }
        // 对未变更的旧前缀暂停新归属；只有回读实际改变计数或竞争关系时才撤回旧证明。
        let blocked = invalidated_sources.union(rechecking_sources).cloned().collect();
        tx.execute(
            "INSERT INTO token_attribution_coverage
             SELECT 1,revision,?1 FROM token_revision WHERE singleton=1
             ON CONFLICT(singleton) DO UPDATE SET revision=excluded.revision,
                blocked_sessions=excluded.blocked_sessions
             WHERE (revision,blocked_sessions) != (excluded.revision,excluded.blocked_sessions)",
            [affected_sessions(&blocked)?],
        )?;
        Self::write_coverage(&tx, coverage)?;
        Ok(tx.commit()?)
    }

    fn write_coverage(tx: &Connection, coverage: Coverage) -> Result<()> {
        tx.execute(
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
            for link in projection::links(source, state, true) {
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
        }
        Self::write_activity(tx, source_id, state, false)?;
        Ok(revision)
    }

    fn write_activity(tx: &Connection, source_id: i64, state: &FileState, repair: bool) -> Result<()> {
        if state.identity_conflict {
            return Ok(());
        }
        let mut add_session =
            tx.prepare("INSERT OR IGNORE INTO token_sessions (session) VALUES (?1)")?;
        let mut activity = tx.prepare(
            "INSERT INTO token_activity
             SELECT ?1, id, ?3, ?4, ?5, ?6 FROM token_sessions WHERE session = ?2
             ON CONFLICT(source_id, session_id, call_id) DO UPDATE SET
                timestamp_ms = excluded.timestamp_ms, day = excluded.day, payload = excluded.payload",
        )?;
        for tool in &state.tools {
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
        if let Some(batch) = &state.executions && !batch.rows.is_empty() {
            add_session.execute([&state.session])?;
            let conflict = if repair { " ON CONFLICT(source_id,position) DO NOTHING" } else { "" };
            let mut insert = tx.prepare(&format!(
                "INSERT INTO execution_observations
                 SELECT ?1,id,?3,?4,?5,?6,?7,?8,?9 FROM token_sessions WHERE session=?2{conflict}",
            ))?;
            let mut process = tx.prepare(&format!(
                "INSERT INTO execution_process VALUES(?1,?2,?3){conflict}"
            ))?;
            for row in &batch.rows {
                insert.execute(params![
                    source_id, state.session, row.position, row.timestamp_ms, row.call_id,
                    row.end, row.is_output, row.name, row.namespace,
                ])?;
                if let Some(key) = &row.process {
                    process.execute(params![source_id, row.position, key])?;
                }
            }
        }
        Ok(())
    }

    pub(super) fn repair_tool_batch(
        &mut self, source: &Path, revision: i64, replay: &FileState, complete: bool,
    ) -> Result<()> {
        let tx = self.db.transaction()?;
        let id: i64 = tx.query_row(
            "SELECT id FROM token_sources WHERE source=? AND revision=?",
            params![encode_path(source), revision], |row| row.get(0),
        ).context("source changed during tool repair")?;
        Self::write_activity(&tx, id, replay, true)?;
        if complete {
            tx.execute(
                "UPDATE token_sources SET activity_truncated=?2, checkpoint=CAST(json_set(CAST(checkpoint AS TEXT),
                 '$.large_tools_checked',json('true'),'$.activity_truncated',json(?3),
                 '$.calls',json(?4),'$.exec_skills',json(?5),'$.exec_polls',json(?6)) AS BLOB) WHERE id=?1",
                params![id, replay.activity_truncated, serde_json::to_string(&replay.activity_truncated)?,
                    serde_json::to_string(&replay.calls)?, serde_json::to_string(&replay.exec_skills)?,
                    serde_json::to_string(&replay.exec_polls)?],
            )?;
        }
        tx.execute(
            "UPDATE token_revision SET revision=revision+1,updated_at=max(updated_at+1,?1)",
            [Local::now().timestamp_millis()],
        )?;
        tx.commit()?;
        Ok(())
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

    pub(super) fn prioritize_growing(
        &self,
        growing: &BTreeMap<PathBuf, u64>,
        queue: &mut VecDeque<PathBuf>,
    ) -> Result<()> {
        let mut source_id = self.db.prepare_cached("SELECT id FROM token_sources WHERE source=?")?;
        let seeds: Vec<i64> = growing.keys().map(|path| {
            source_id.query_row([encode_path(path)], |r| r.get(0))
        }).collect::<rusqlite::Result<_>>()?;
        let mut statement = self.db.prepare_cached(
            "WITH RECURSIVE component(id,link) AS (
                 SELECT value,NULL FROM json_each(?1)
                 UNION SELECT NULL,l.link FROM component c JOIN token_source_links l ON l.source_id=c.id
                 UNION SELECT l.source_id,NULL FROM component c JOIN token_source_links l ON l.link=c.link
             )
             SELECT s.source FROM component c JOIN token_sources s ON s.id=c.id
             WHERE COALESCE(json_extract(CAST(s.checkpoint AS TEXT),'$.legacy_identity_prefix'),s.offset)>0",
        )?;
        let mut rows = statement.query([serde_json::to_string(&seeds)?])?;
        let mut linked = HashSet::new();
        while let Some(row) = rows.next()? {
            linked.insert(decode_path(row.get(0)?)?);
        }
        let (new_tails, history): (VecDeque<_>, VecDeque<_>) =
            std::mem::take(queue).into_iter().partition(|path| growing.contains_key(path));
        let (mut linked, mut other): (VecDeque<_>, VecDeque<_>) =
            history.into_iter().partition(|path| linked.contains(path));
        *queue = new_tails;
        while !linked.is_empty() && !other.is_empty() {
            queue.extend(linked.pop_front());
            queue.extend(other.pop_front());
        }
        queue.extend(linked);
        queue.extend(other);
        Ok(())
    }

    pub(super) fn finish_identity_probe(&self, source: &Path, revision: i64) -> Result<()> {
        let changed = self.db.execute(
            "UPDATE token_sources SET checkpoint = CAST(json_set(CAST(checkpoint AS TEXT),
               '$.legacy_identity_prefix', 0, '$.large_tools_checked', json('true')) AS BLOB) WHERE source = ?1 AND revision = ?2",
            params![encode_path(source), revision],
        )?;
        anyhow::ensure!(changed == 1, "source changed during identity check");
        Ok(())
    }

    pub(super) fn sealed_queued_sources(
        &self,
        discovered: &[PathBuf],
    ) -> Result<(
        BTreeMap<PathBuf, super::SourceStamp>,
        BTreeSet<PathBuf>,
        BTreeMap<PathBuf, u64>,
        BTreeMap<PathBuf, super::SourceStamp>,
        BTreeMap<PathBuf, (super::SourceStamp, bool)>,
    )> {
        #[derive(serde::Deserialize)]
        struct CountState {
            stamp: Option<super::SourceStamp>,
            resume_pending: bool,
            skipping: bool,
            session: String,
            issues: usize,
            #[serde(default)]
            malformed_token_counts: usize,
            invalid_counts: bool,
            identity_conflict: bool,
            #[serde(default)]
            legacy_identity_prefix: Option<u64>,
            #[serde(default)]
            activity_truncated: bool,
            #[serde(default)]
            large_tools_checked: bool,
        }
        let present: HashSet<_> = discovered.iter().map(PathBuf::as_path).collect();
        let current = |source: &Path| -> Result<Option<super::SourceStamp>> {
            match FileState::source_stamp(source) {
                Ok(actual) => Ok(Some(actual)),
                Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
                Err(error) => Err(error.into()),
            }
        };
        let mut sealed = BTreeMap::new();
        let mut gaps = BTreeSet::new();
        let mut growing = BTreeMap::new();
        let mut unchecked = BTreeMap::new();
        let mut scoped_prefixes = BTreeMap::new();
        let mut owners = HashSet::new();
        let mut sources = self
            .db
            .prepare_cached("SELECT id, source, offset, checkpoint FROM token_sources")?;
        let mut rows = sources.query([])?;
        while let Some(row) = rows.next()? {
            let path = decode_path(row.get(1)?)?;
            if !present.contains(path.as_path()) {
                continue;
            }
            let saved: CountState = serde_json::from_slice(row.get_ref(3)?.as_blob()?)?;
            let Some(stamp @ (size, Some(_), Some(_))) = saved.stamp else {
                continue;
            };
            let scoped_gap = !saved.session.is_empty()
                && saved.issues != 0
                && saved.issues == saved.malformed_token_counts;
            let offset = row.get::<_, u64>(2)?;
            if super::recheck_prefix(
                offset,
                saved.legacy_identity_prefix,
                saved.large_tools_checked,
                saved.activity_truncated,
            ) != 0
            {
                let actual = current(&path)?;
                if actual == Some(stamp)
                    && !saved.session.is_empty()
                    && !saved.resume_pending
                    && !saved.skipping
                    && !saved.identity_conflict
                    && saved.issues == saved.malformed_token_counts
                    && (!saved.invalid_counts || scoped_gap)
                    && (size == offset || path.extension().is_some_and(|ext| ext == "zst"))
                {
                    scoped_prefixes
                        .insert(path.clone(), (stamp, saved.legacy_identity_prefix != Some(0)));
                    if scoped_gap {
                        gaps.insert(path.clone());
                    }
                }
                if path.extension().is_some_and(|ext| ext == "jsonl") {
                    if let Some((length, _, identity)) = actual
                        && length > size && identity == stamp.2
                    {
                        growing.insert(path.clone(), length - size);
                    }
                    unchecked.insert(path, stamp);
                }
                continue;
            }
            if saved.resume_pending
                || saved.skipping
                || saved.identity_conflict
                || (saved.issues != 0 || saved.invalid_counts) && !scoped_gap
                || !path.extension().is_some_and(|ext| ext == "zst")
                    && size != row.get::<_, u64>(2)?
            {
                continue;
            }
            let actual = current(&path)?;
            if actual != Some(stamp) {
                if !saved.invalid_counts
                    && path.extension().is_some_and(|ext| ext == "jsonl")
                    && stamp.2.is_some()
                    && let Some((length, _, id)) = actual
                    && length > size
                    && id == stamp.2
                {
                    growing.insert(path, length - size);
                }
                continue;
            }
            if scoped_gap {
                gaps.insert(path.clone());
            } else {
                owners.insert(row.get::<_, i64>(0)?);
            }
            sealed.insert(path, stamp);
        }
        // A verified replica only covers the bytes certified for its unchanged owner.
        let mut copies = self
            .db
            .prepare_cached("SELECT source, owner_id, stamp FROM token_source_copies")?;
        let mut rows = copies.query([])?;
        while let Some(row) = rows.next()? {
            if owners.contains(&row.get::<_, i64>(1)?) {
                let path = decode_path(row.get(0)?)?;
                let stamp = serde_json::from_slice(row.get_ref(2)?.as_blob()?)?;
                if present.contains(path.as_path()) && current(&path)? == Some(stamp) {
                    sealed.insert(path, stamp);
                }
            }
        }
        Ok((sealed, gaps, growing, unchecked, scoped_prefixes))
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
