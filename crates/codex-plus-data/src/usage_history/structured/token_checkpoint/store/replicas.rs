use super::{Checkpoint, FileState, Observation, Store, decode_path, encode_path, projection};
use crate::usage_history::structured::{
    UsageEvent, index_budget::Budget, segments::rollout_id, token_checkpoint::SourceStamp,
};
use anyhow::{Result, ensure};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use std::collections::HashSet;
use std::fs::File;
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};

// Give other sessions half of the existing round budget while verification is pending.
const COPY_ROUND_BYTES: u64 = 2 * 1024 * 1024;

struct Owner {
    id: i64,
    revision: i64,
    path: PathBuf,
    offset: u64,
    checkpoint: Checkpoint,
}

impl Owner {
    fn read(row: &rusqlite::Row<'_>) -> Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            path: decode_path(row.get(1)?)?,
            revision: row.get(2)?,
            offset: row.get(3)?,
            checkpoint: serde_json::from_slice(row.get_ref(4)?.as_blob()?)?,
        })
    }

    fn stable_prefix(&self) -> Option<SourceStamp> {
        let stamp @ (size, Some(_), Some(_)) = self.checkpoint.stamp? else {
            return None;
        };
        (size == self.offset
            && size != 0
            && !self.checkpoint.resume_pending
            && !self.checkpoint.skipping
            && self.checkpoint.issues == 0
            && !self.checkpoint.invalid_counts
            && !self.checkpoint.identity_conflict)
            .then_some(stamp)
    }
}

fn is_same_growth(expected: SourceStamp, actual: SourceStamp) -> bool {
    actual.2 == expected.2 && (actual.0 > expected.0 || actual == expected)
}

pub(in crate::usage_history::structured::token_checkpoint) enum Probe {
    ReadSource,
    WaitForOwner(PathBuf),
    Covered { moved_from: Option<PathBuf> },
    Compare(Box<Comparison>),
}

pub(in crate::usage_history::structured::token_checkpoint) struct Comparison {
    owner: Owner,
    materialized: Option<Owner>,
    source: PathBuf,
    original_stamp: SourceStamp,
    copy_stamp: SourceStamp,
    original: File,
    copy: File,
    remaining: u64,
    last: Option<u8>,
}

impl Comparison {
    pub(in crate::usage_history::structured::token_checkpoint) fn path(&self) -> &Path {
        &self.source
    }

    pub(in crate::usage_history::structured::token_checkpoint) fn holds(
        &self,
        path: &Path,
    ) -> bool {
        self.materialized.is_some() && self.owner.path == path
    }

    pub(in crate::usage_history::structured::token_checkpoint) fn step(
        mut self: Box<Self>,
        store: &mut Store,
        budget: &mut Budget,
    ) -> Result<Probe> {
        let mut left = [0u8; 32 * 1024];
        let mut right = [0u8; 32 * 1024];
        let mut slice = COPY_ROUND_BYTES;
        while self.remaining > 0 {
            let count = self
                .remaining
                .min(left.len() as u64)
                .min(budget.source_left.min(budget.read_left).min(slice) / 2)
                as usize;
            if count == 0 {
                return Ok(Probe::Compare(self));
            }
            self.original.read_exact(&mut left[..count])?;
            self.copy.read_exact(&mut right[..count])?;
            budget.charge_read(count * 2);
            budget.source_left -= 2 * count as u64;
            slice -= 2 * count as u64;
            if left[..count] != right[..count] {
                return Ok(Probe::ReadSource);
            }
            self.remaining -= count as u64;
            self.last = Some(left[count - 1]);
        }
        if self.last != Some(b'\n')
            || FileState::source_stamp(&self.owner.path)? != self.original_stamp
            || FileState::source_stamp(&self.source)? != self.copy_stamp
            || std::fs::canonicalize(&self.owner.path)? != self.owner.path
            || std::fs::canonicalize(&self.source)? != self.source
        {
            return Ok(Probe::ReadSource);
        }
        if let Some(copy) = self.materialized {
            return store.collapse_materialized_copy(
                self.owner,
                copy,
                self.original_stamp,
                self.copy_stamp,
            );
        }
        let move_owner = self.copy_stamp.0 > self.owner.offset;
        store.commit_copy(
            self.owner,
            &self.source,
            self.copy_stamp,
            move_owner,
            Some(self.original_stamp),
        )
    }
}

impl Store {
    pub(super) fn remove_missing_copies(&self, present: &HashSet<&Path>) -> Result<()> {
        let mut list = self
            .db
            .prepare_cached("SELECT source FROM token_source_copies")?;
        let mut missing = Vec::new();
        let mut rows = list.query([])?;
        while let Some(row) = rows.next()? {
            let key: Vec<u8> = row.get(0)?;
            if !present.contains(decode_path(key.clone())?.as_path()) {
                missing.push(key);
            }
        }
        drop(rows);
        let mut remove = self
            .db
            .prepare_cached("DELETE FROM token_source_copies WHERE source = ?1")?;
        for key in missing {
            remove.execute([key])?;
        }
        Ok(())
    }

    pub(in crate::usage_history::structured::token_checkpoint) fn probe_copy(
        &mut self,
        source: &Path,
    ) -> Result<Probe> {
        let Some(rollout) =
            rollout_id(source).filter(|_| source.extension().is_some_and(|e| e == "jsonl"))
        else {
            return Ok(Probe::ReadSource);
        };
        let path = encode_path(source);
        let known: Option<(i64, Vec<u8>)> = self
            .db
            .query_row(
                "SELECT owner_id, stamp FROM token_source_copies WHERE source = ?1",
                [&path],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let (owner, materialized) = {
            let mut select = self.db.prepare_cached(
                "SELECT s.id, s.source, s.revision, s.offset, s.checkpoint
                 FROM token_source_links l JOIN token_sources s ON s.id = l.source_id
                 WHERE l.link = ?1 AND EXISTS (
                    SELECT 1 FROM token_source_links candidate
                    JOIN token_sources peer ON peer.id = candidate.source_id
                    WHERE candidate.link = l.link AND peer.source != ?2
                 )",
            )?;
            let mut matches = select.query(params![format!("r:{rollout}"), path])?;
            let mut owner = None;
            let mut materialized = None;
            while let Some(row) = matches.next()? {
                let candidate = decode_path(row.get(1)?)?;
                if candidate.file_name() != source.file_name() {
                    continue;
                }
                if candidate == source {
                    materialized = Some(Owner::read(row)?);
                    continue;
                }
                if owner.is_some() {
                    return Ok(Probe::ReadSource);
                }
                owner = Some(Owner::read(row)?);
            }
            let Some(owner) = owner else {
                return Ok(Probe::ReadSource);
            };
            (owner, materialized)
        };
        let Some(expected @ (size, Some(_), Some(_))) = owner.checkpoint.stamp else {
            return Ok(Probe::ReadSource);
        };
        let copy_stamp = FileState::source_stamp(source)?;
        let recorded = known
            .map(|(id, bytes)| -> Result<_> {
                Ok(id == owner.id && serde_json::from_slice::<SourceStamp>(&bytes)? == copy_stamp)
            })
            .transpose()?
            .unwrap_or(false);
        let original = match FileState::source_stamp(&owner.path) {
            Ok(stamp) => Some(stamp),
            Err(err) if err.kind() == ErrorKind::NotFound => None,
            Err(err) => return Err(err.into()),
        };
        let replaced = original.is_none_or(|stamp| !is_same_growth(expected, stamp));
        if materialized.is_none() && recorded && replaced && copy_stamp.0 == owner.offset {
            return self.commit_copy(owner, source, copy_stamp, true, None);
        }
        if materialized.is_none() && recorded && !replaced {
            return Ok(Probe::Covered { moved_from: None });
        }
        if materialized.is_none() && !replaced && original != Some(expected) {
            return Ok(Probe::WaitForOwner(owner.path));
        }
        let Some(original_stamp) = original else {
            return Ok(Probe::ReadSource);
        };
        if let Some(copy) = &materialized {
            // The second, unchanged file certifies the indexed prefix when its peer has grown.
            if original_stamp == expected
                && let Some(previous) = copy.stable_prefix()
                && copy.offset == owner.offset
                && copy_stamp.0 > previous.0
                && is_same_growth(previous, copy_stamp)
            {
                return Ok(Probe::WaitForOwner(owner.path));
            }
            if replaced
                || owner.stable_prefix().is_none()
                || copy.stable_prefix() != Some(copy_stamp)
                || copy.offset != owner.offset
                || (original_stamp == expected && owner.id > copy.id)
            {
                return Ok(Probe::ReadSource);
            }
        } else if owner.stable_prefix().is_none()
            || original_stamp != expected
            || copy_stamp.1.is_none()
            || copy_stamp.2.is_none()
            || copy_stamp.0 == 0
        {
            return Ok(Probe::ReadSource);
        }
        let original = File::open(&owner.path)?;
        let copy = File::open(source)?;
        let identity = codex_plus_core::codex_sqlite::opened_file_identity;
        if Some(identity(&original)?) != expected.2 || Some(identity(&copy)?) != copy_stamp.2 {
            return Ok(Probe::ReadSource);
        }
        Ok(Probe::Compare(Box::new(Comparison {
            remaining: size.min(copy_stamp.0),
            last: None,
            owner,
            materialized,
            source: source.to_owned(),
            original_stamp,
            copy_stamp,
            original,
            copy,
        })))
    }

    fn collapse_materialized_copy(
        &mut self,
        owner: Owner,
        copy: Owner,
        owner_stamp: SourceStamp,
        stamp: SourceStamp,
    ) -> Result<Probe> {
        let owner_previous = owner.checkpoint.previous.map(|count| count.0);
        let copy_previous = copy.checkpoint.previous.map(|count| count.0);
        if owner_previous.is_none()
            || owner_previous != copy_previous
            || owner.checkpoint.session != copy.checkpoint.session
        {
            return Ok(Probe::ReadSource);
        }
        let owner_id = owner.id;
        let copy_id = copy.id;
        let state = owner.checkpoint.restore(owner.offset);
        if !projection::linear_source(&state)
            || !projection::linear_source(&copy.checkpoint.restore(copy.offset))
        {
            return Ok(Probe::ReadSource);
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let linked = tx
            .prepare_cached(
                "SELECT 1
             FROM token_source_links own JOIN token_source_links other ON other.link = own.link
             WHERE own.source_id IN (?1, ?2) AND other.source_id NOT IN (?1, ?2) LIMIT 1",
            )?
            .exists(params![owner_id, copy_id])?;
        if linked {
            return Ok(Probe::ReadSource);
        }
        let anchor = {
            let sql = "SELECT o.revision, o.event_id, o.payload, s.session
                       FROM token_observations o JOIN token_sessions s ON s.id = o.session_id
                       WHERE o.source_id = ?1 ORDER BY o.revision, o.position";
            let mut original = tx.prepare(sql)?;
            let mut duplicate = tx.prepare(sql)?;
            let mut original = original.query([owner_id])?;
            let mut duplicate = duplicate.query([copy_id])?;
            let mut previous: Option<(i64, UsageEvent)> = None;
            let mut probe_claim = tx.prepare_cached(
                "SELECT 1 FROM token_observations
                 WHERE event_id = ?1 AND source_id = ?2
                 AND json_type(CAST(payload AS TEXT), '$.terminal_response') = 'text'
                 LIMIT 1 OFFSET 1",
            )?;
            loop {
                match (original.next()?, duplicate.next()?) {
                    (None, None) => break,
                    (Some(left), Some(right)) => {
                        let id = left.get_ref(1)?.as_blob()?;
                        let payload = left.get_ref(2)?.as_blob()?;
                        let session = left.get_ref(3)?.as_str()?;
                        if right.get_ref(1)?.as_blob()? != id
                            || right.get_ref(2)?.as_blob()? != payload
                            || right.get_ref(3)?.as_str()? != session
                        {
                            return Ok(Probe::ReadSource);
                        }
                        let raw: Observation = serde_json::from_slice(payload)?;
                        if raw.terminal_response.is_some() {
                            if session != state.session
                                || probe_claim.exists(params![id, owner_id])?
                            {
                                return Ok(Probe::ReadSource);
                            }
                            continue;
                        }
                        let event = raw.into_event(id.try_into()?, session.to_owned())?;
                        if event.id != event.original_id
                            || !projection::linear_observation(
                                &state.session,
                                &event,
                                previous.as_ref().map(|(_, event)| event),
                            )
                        {
                            return Ok(Probe::ReadSource);
                        }
                        previous = Some((left.get(0)?, event));
                    }
                    _ => return Ok(Probe::ReadSource),
                }
            }
            let Some((revision, last)) = previous else {
                return Ok(Probe::ReadSource);
            };
            if Some(last.total) != owner_previous {
                return Ok(Probe::ReadSource);
            }
            revision
        };
        // The projection and revision are left unchanged except for its source membership.
        // If a call exists only on the duplicate, keep both sources rather than dropping it.
        let unique_activity = tx.prepare_cached(
            "SELECT 1 WHERE EXISTS (
                SELECT 1 FROM token_activity copy
                WHERE copy.source_id = ?2 AND NOT EXISTS (
                    SELECT 1 FROM token_activity own
                    WHERE own.source_id = ?1 AND own.session_id = copy.session_id AND own.call_id = copy.call_id
                      AND (own.timestamp_ms, own.day, own.payload) = (copy.timestamp_ms, copy.day, copy.payload)
                )
             ) OR EXISTS (
                SELECT 1 FROM execution_observations copy
                WHERE copy.source_id = ?2 AND NOT EXISTS (
                    SELECT 1 FROM execution_observations own WHERE own.source_id = ?1
                      AND own.position = copy.position AND own.session_id = copy.session_id
                      AND (own.timestamp_ms, own.call_id, own.end, own.is_output)
                          IS NOT DISTINCT FROM
                          (copy.timestamp_ms, copy.call_id, copy.end, copy.is_output)
                      AND ((own.name, own.namespace) IS NOT DISTINCT FROM (copy.name, copy.namespace)
                           OR (own.is_output = 0 AND own.timestamp_ms <= ?3
                               AND (own.name IS NULL OR copy.name IS NULL)))
                )
             ) OR EXISTS (
                SELECT 1 FROM execution_process copy
                WHERE copy.source_id = ?2 AND NOT EXISTS (
                    SELECT 1 FROM execution_process own WHERE own.source_id = ?1
                      AND own.position = copy.position AND own.process = copy.process
                )
             )",
        )?.exists(params![
            owner_id,
            copy_id,
            chrono::Local::now().timestamp_millis()
                - crate::usage_history::structured::execution_observations::LIST_MS
        ])?;
        if unique_activity
            || FileState::source_stamp(&owner.path)? != owner_stamp
            || FileState::source_stamp(&copy.path)? != stamp
        {
            return Ok(Probe::ReadSource);
        }
        let current = tx
            .prepare_cached(
                "SELECT count(*) FROM token_sources
             WHERE (id, revision, source) IN ((?1, ?2, ?3), (?4, ?5, ?6))",
            )?
            .query_row(
                params![
                    owner_id,
                    owner.revision,
                    encode_path(&owner.path),
                    copy_id,
                    copy.revision,
                    encode_path(&copy.path)
                ],
                |row| row.get::<_, u32>(0),
            )?;
        ensure!(
            current == 2,
            "Token materialized copies changed before commit"
        );
        let cutoff: String =
            tx.query_row("SELECT precise_from FROM token_revision", [], |row| {
                row.get(0)
            })?;
        ensure!(
            !cutoff.is_empty(),
            "Token copy reconciliation has no retention boundary"
        );
        tx.execute(
            "UPDATE token_source_copies SET owner_id = ?1 WHERE owner_id = ?2",
            params![owner_id, copy_id],
        )?;
        tx.execute(
            "DELETE FROM token_linear_sources WHERE source_id = ?1",
            [copy_id],
        )?;
        tx.execute(
            "DELETE FROM token_source_links WHERE source_id = ?1",
            [copy_id],
        )?;
        tx.execute(
            "DELETE FROM token_observations WHERE source_id = ?1",
            [copy_id],
        )?;
        tx.execute("DELETE FROM token_sources WHERE id = ?1", [copy_id])?;
        tx.execute(
            "INSERT OR REPLACE INTO token_linear_sources VALUES (?1, ?2, ?3)",
            params![owner_id, anchor, cutoff],
        )?;
        tx.execute(
            "INSERT INTO token_source_copies (source, owner_id, stamp) VALUES (?1, ?2, ?3)",
            params![
                encode_path(&copy.path),
                owner_id,
                serde_json::to_vec(&stamp)?
            ],
        )?;
        tx.execute(
            "UPDATE token_revision SET revision = revision + 1, updated_at = max(updated_at + 1, ?1)",
            [chrono::Local::now().timestamp_millis()],
        )?;
        tx.commit()?;
        Ok(Probe::Covered { moved_from: None })
    }

    fn commit_copy(
        &mut self,
        mut owner: Owner,
        path: &Path,
        stamp: SourceStamp,
        move_owner: bool,
        old_copy: Option<SourceStamp>,
    ) -> Result<Probe> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let source = encode_path(path);
        if FileState::source_stamp(path)? != stamp {
            return Ok(Probe::ReadSource);
        }
        if let Some(original) = old_copy {
            if FileState::source_stamp(&owner.path)? != original {
                return Ok(Probe::ReadSource);
            }
        } else if move_owner {
            let prior = owner.checkpoint.stamp.expect("verified owner stamp");
            match FileState::source_stamp(&owner.path) {
                Ok(current) if is_same_growth(prior, current) => {
                    return Ok(Probe::ReadSource);
                }
                Ok(_) => {}
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        ensure!(
            tx.prepare_cached(
                "SELECT 1 FROM token_sources WHERE id = ?1 AND revision = ?2 AND source = ?3"
            )?
            .exists(params![owner.id, owner.revision, encode_path(&owner.path)])?,
            "Token copy owner changed before commit"
        );
        let remember = |tx: &Transaction, path, stamp| -> Result<()> {
            tx.execute(
                "INSERT INTO token_source_copies (source, owner_id, stamp) VALUES (?1, ?2, ?3)
                 ON CONFLICT(source) DO UPDATE SET owner_id = excluded.owner_id, stamp = excluded.stamp",
                params![path, owner.id, serde_json::to_vec(&stamp)?],
            )?;
            Ok(())
        };
        if move_owner {
            owner.checkpoint.stamp = Some(stamp);
            owner.checkpoint.resume_pending = stamp.0 > owner.offset;
            tx.execute(
                "UPDATE token_sources SET source = ?1, checkpoint = ?2 WHERE id = ?3",
                params![source, serde_json::to_vec(&owner.checkpoint)?, owner.id],
            )?;
            tx.execute(
                "DELETE FROM token_source_copies WHERE source = ?1",
                [&source],
            )?;
            if let Some(old) = old_copy {
                remember(&tx, encode_path(&owner.path), old)?;
            }
            tx.execute(
                "UPDATE token_revision SET revision = revision + 1, updated_at = max(updated_at + 1, ?1)",
                [chrono::Local::now().timestamp_millis()],
            )?;
        } else {
            remember(&tx, source, stamp)?;
        }
        tx.commit()?;
        Ok(if move_owner && stamp.0 > owner.offset {
            Probe::ReadSource
        } else {
            Probe::Covered {
                moved_from: move_owner.then_some(owner.path),
            }
        })
    }
}
