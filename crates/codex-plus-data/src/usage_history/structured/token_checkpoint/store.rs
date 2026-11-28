use super::{Checkpoint, FileState, Observation, UsageEvent};
use anyhow::{Result, ensure};
use rusqlite::{
    Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior, params,
};
use std::path::Path;
use std::time::Duration;

const BUSY_TIMEOUT: Duration = Duration::from_secs(2);

fn checked_version(db: &Connection) -> Result<i64> {
    let version: i64 = db.pragma_query_value(None, "user_version", |row| row.get(0))?;
    ensure!(
        matches!(version, 0 | 1),
        "unsupported Token snapshot version {version}"
    );
    if version == 0 {
        let nonempty: bool =
            db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master)", [], |row| {
                row.get(0)
            })?;
        ensure!(!nonempty, "unversioned Token snapshot is not empty");
    }
    Ok(version)
}

pub(super) struct Store {
    pub(super) db: Connection,
}

impl Store {
    pub(super) fn open(path: &Path) -> Result<Self> {
        if path.try_exists()? {
            // 读写连接关闭时可能回写 WAL，即使没有执行任何 SQL 写入。
            let probe = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
            probe.busy_timeout(BUSY_TIMEOUT)?;
            checked_version(&probe)?;
        }
        let mut db = Connection::open(path)?;
        db.busy_timeout(BUSY_TIMEOUT)?;
        db.pragma_update(None, "foreign_keys", true)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if checked_version(&tx)? == 0 {
            tx.execute_batch(
                "CREATE TABLE token_revision (
                    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                    revision INTEGER NOT NULL
                 ) STRICT;
                 INSERT INTO token_revision VALUES (1, 0);
                 CREATE TABLE token_sources (
                    id INTEGER PRIMARY KEY,
                    source BLOB NOT NULL UNIQUE,
                    revision INTEGER NOT NULL,
                    offset INTEGER NOT NULL,
                    generation INTEGER NOT NULL,
                    checkpoint BLOB NOT NULL
                 ) STRICT;
                 CREATE TABLE token_sessions (
                    id INTEGER PRIMARY KEY,
                    session TEXT NOT NULL UNIQUE
                 ) STRICT;
                 CREATE TABLE token_observations (
                    source_id INTEGER NOT NULL REFERENCES token_sources(id) ON DELETE CASCADE,
                    revision INTEGER NOT NULL,
                    position INTEGER NOT NULL,
                    session_id INTEGER NOT NULL REFERENCES token_sessions(id),
                    event_id BLOB NOT NULL,
                    original_id BLOB NOT NULL,
                    payload BLOB NOT NULL,
                    PRIMARY KEY (source_id, position)
                 ) WITHOUT ROWID, STRICT;
                 CREATE INDEX token_observation_sessions ON token_observations (session_id);
                 CREATE TRIGGER token_remove_orphan_session AFTER DELETE ON token_observations
                 BEGIN
                    DELETE FROM token_sessions WHERE id = OLD.session_id
                        AND NOT EXISTS (
                            SELECT 1 FROM token_observations WHERE session_id = OLD.session_id
                        );
                 END;
                 PRAGMA user_version = 1;",
            )?;
        }
        tx.commit()?;
        db.pragma_update(None, "journal_mode", "WAL")?;
        Ok(Self { db })
    }

    pub(super) fn save(
        &mut self,
        source: &[u8],
        expected_revision: Option<i64>,
        state: &FileState,
    ) -> Result<i64> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let revision = Self::stage(&tx, source, expected_revision, state)?;
        tx.commit()?;
        Ok(revision)
    }

    pub(super) fn stage(
        tx: &Transaction<'_>,
        source: &[u8],
        expected_revision: Option<i64>,
        state: &FileState,
    ) -> Result<i64> {
        ensure!(!state.skipping, "cannot checkpoint within a skipped line");
        let (offset, checkpoint) = Checkpoint::capture(state);
        let previous: Option<(i64, u64, u64)> = tx
            .query_row(
                "SELECT revision, offset, generation FROM token_sources WHERE source = ?1",
                [source],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        ensure!(
            previous.map(|row| row.0) == expected_revision,
            "Token source changed before commit"
        );
        let replace = previous.is_some_and(|row| row.2 != state.source_generation);
        let committed_offset = previous.filter(|_| !replace).map_or(0, |row| row.1);
        ensure!(
            offset >= committed_offset,
            "Token source offset moved backwards"
        );
        let revision = tx.query_row(
            "UPDATE token_revision SET revision = revision + 1 RETURNING revision",
            [],
            |row| row.get::<_, i64>(0),
        )?;
        let source_id = tx.query_row(
            "INSERT INTO token_sources (source, revision, offset, generation, checkpoint)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (source) DO UPDATE SET revision = excluded.revision,
                offset = excluded.offset, generation = excluded.generation,
                checkpoint = excluded.checkpoint
             RETURNING id",
            params![
                source,
                revision,
                offset,
                state.source_generation,
                serde_json::to_vec(&checkpoint)?
            ],
            |row| row.get::<_, i64>(0),
        )?;
        if replace {
            tx.execute(
                "DELETE FROM token_observations WHERE source_id = ?1",
                [source_id],
            )?;
        }
        {
            let mut add_session =
                tx.prepare("INSERT OR IGNORE INTO token_sessions (session) VALUES (?1)")?;
            let mut insert = tx.prepare(
                "INSERT INTO token_observations
                 SELECT ?1, ?2, ?3, id, ?5, ?6, ?7 FROM token_sessions WHERE session = ?4",
            )?;
            // 观察按原文件位置追加，旧日志即使没有 ordinal 也能区分两条同 ID 的记录。
            let first = state
                .events
                .partition_point(|event| event.source_end <= committed_offset);
            for event in &state.events[first..] {
                add_session.execute([&event.session])?;
                insert.execute(params![
                    source_id,
                    revision,
                    event.source_end,
                    event.session,
                    event.id.as_slice(),
                    event.original_id.as_slice(),
                    serde_json::to_vec(&Observation::capture(event))?,
                ])?;
            }
        }
        Ok(revision)
    }

    pub(super) fn checkpoint(&self, source: &[u8]) -> Result<Option<(i64, FileState)>> {
        let row: Option<(i64, u64, u64, Vec<u8>)> = self
            .db
            .query_row(
                "SELECT revision, offset, generation, checkpoint FROM token_sources WHERE source = ?1",
                [source],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        row.map(|(revision, offset, generation, payload)| {
            Ok((
                revision,
                serde_json::from_slice::<Checkpoint>(&payload)?.restore(offset, generation),
            ))
        })
        .transpose()
    }

    pub(super) fn observations(&self, source: &[u8]) -> Result<Vec<UsageEvent>> {
        let mut stmt = self.db.prepare(
            "SELECT o.event_id, s.session, o.payload, o.original_id, o.position FROM token_observations o
             JOIN token_sources t ON t.id = o.source_id
             JOIN token_sessions s ON s.id = o.session_id
             WHERE t.source = ?1 ORDER BY o.position",
        )?;
        let mut rows = stmt.query([source])?;
        let mut observations = Vec::new();
        while let Some(row) = rows.next()? {
            let id = row.get_ref(0)?.as_blob()?.try_into()?;
            let observation: Observation = serde_json::from_slice(row.get_ref(2)?.as_blob()?)?;
            let original_id = row.get_ref(3)?.as_blob()?.try_into()?;
            observations.push(observation.into_event(id, original_id, row.get(1)?, row.get(4)?)?);
        }
        Ok(observations)
    }
}
