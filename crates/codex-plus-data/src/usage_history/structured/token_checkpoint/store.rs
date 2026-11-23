use super::{Checkpoint, FileState, Observation, UsageEvent};
use anyhow::{Result, ensure};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

pub(super) struct Store {
    pub(super) db: Connection,
}

impl Store {
    pub(super) fn open(db: Connection) -> Result<Self> {
        db.pragma_update(None, "foreign_keys", true)?;
        db.pragma_update(None, "journal_mode", "WAL")?;
        db.execute_batch(
            "BEGIN IMMEDIATE;
             CREATE TABLE IF NOT EXISTS token_revision (
                singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                revision INTEGER NOT NULL
             ) STRICT;
             INSERT OR IGNORE INTO token_revision VALUES (1, 0);
             CREATE TABLE IF NOT EXISTS token_sources (
                id INTEGER PRIMARY KEY,
                source BLOB NOT NULL UNIQUE,
                revision INTEGER NOT NULL,
                offset INTEGER NOT NULL,
                checkpoint BLOB NOT NULL
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
             COMMIT;",
        )?;
        Ok(Self { db })
    }

    pub(super) fn save(
        &mut self,
        source: &[u8],
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
        source: &[u8],
        expected_revision: Option<i64>,
        state: &FileState,
        replace: bool,
    ) -> Result<i64> {
        ensure!(!state.skipping, "cannot checkpoint within a skipped line");
        let (offset, checkpoint) = Checkpoint::capture(state);
        let previous: Option<(i64, i64, u64)> = tx
            .query_row(
                "SELECT id, revision, offset FROM token_sources WHERE source = ?1",
                [source],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        ensure!(
            previous.map(|row| row.1) == expected_revision,
            "Token source changed before commit"
        );
        if !replace && let Some((_, _, old_offset)) = previous {
            ensure!(
                offset > old_offset || offset == old_offset && state.events.is_empty(),
                "Token source offset did not advance"
            );
        }
        let revision = tx.query_row(
            "UPDATE token_revision SET revision = revision + 1 RETURNING revision",
            [],
            |row| row.get::<_, i64>(0),
        )?;
        let source_id = tx.query_row(
            "INSERT INTO token_sources (source, revision, offset, checkpoint)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (source) DO UPDATE SET revision = excluded.revision,
                offset = excluded.offset, checkpoint = excluded.checkpoint
             RETURNING id",
            params![source, revision, offset, serde_json::to_vec(&checkpoint)?],
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
        Ok(revision)
    }

    pub(super) fn checkpoint(&self, source: &[u8]) -> Result<Option<(i64, FileState)>> {
        let row: Option<(i64, u64, Vec<u8>)> = self
            .db
            .query_row(
                "SELECT revision, offset, checkpoint FROM token_sources WHERE source = ?1",
                [source],
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

    pub(super) fn observations(&self, source: &[u8]) -> Result<Vec<UsageEvent>> {
        let mut stmt = self.db.prepare(
            "SELECT o.event_id, s.session, o.payload FROM token_observations o
             JOIN token_sources t ON t.id = o.source_id
             JOIN token_sessions s ON s.id = o.session_id
             WHERE t.source = ?1 ORDER BY o.revision, o.position",
        )?;
        let mut rows = stmt.query([source])?;
        let mut observations = Vec::new();
        while let Some(row) = rows.next()? {
            let id = row.get_ref(0)?.as_blob()?.try_into()?;
            let observation: Observation = serde_json::from_slice(row.get_ref(2)?.as_blob()?)?;
            observations.push(observation.into_event(id, row.get(1)?)?);
        }
        Ok(observations)
    }
}
