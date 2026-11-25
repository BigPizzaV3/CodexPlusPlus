use super::{Checkpoint, FileState, Store, decode_path, encode_path};
use crate::usage_history::structured::{
    index_budget::Budget, segments::rollout_id, token_checkpoint::SourceStamp,
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
        if self
            .db
            .prepare_cached("SELECT 1 FROM token_sources WHERE source = ?1")?
            .exists([&path])?
        {
            return Ok(Probe::ReadSource);
        }
        let known: Option<(i64, Vec<u8>)> = self
            .db
            .query_row(
                "SELECT owner_id, stamp FROM token_source_copies WHERE source = ?1",
                [&path],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let owner = {
            let mut select = self.db.prepare_cached(
                "SELECT s.id, s.source, s.revision, s.offset, s.checkpoint
                 FROM token_source_links l JOIN token_sources s ON s.id = l.source_id
                 WHERE l.link = ?1",
            )?;
            let mut matches = select.query([format!("r:{rollout}")])?;
            let mut owner = None;
            while let Some(row) = matches.next()? {
                let candidate = decode_path(row.get(1)?)?;
                if candidate.file_name() != source.file_name() {
                    continue;
                }
                if owner.is_some() {
                    return Ok(Probe::ReadSource);
                }
                owner = Some(Owner {
                    id: row.get(0)?,
                    path: candidate,
                    revision: row.get(2)?,
                    offset: row.get(3)?,
                    checkpoint: serde_json::from_slice(row.get_ref(4)?.as_blob()?)?,
                });
            }
            let Some(owner) = owner else {
                return Ok(Probe::ReadSource);
            };
            owner
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
        if recorded && replaced && copy_stamp.0 == owner.offset {
            return self.commit_copy(owner, source, copy_stamp, true, None);
        }
        if recorded && !replaced {
            return Ok(Probe::Covered { moved_from: None });
        }
        if !replaced && original != Some(expected) {
            return Ok(Probe::WaitForOwner(owner.path));
        }
        if original != Some(expected)
            || size != owner.offset
            || owner.checkpoint.resume_pending
            || owner.checkpoint.issues != 0
            || owner.checkpoint.invalid_counts
            || owner.checkpoint.identity_conflict
            || copy_stamp.1.is_none()
            || copy_stamp.2.is_none()
            || size.min(copy_stamp.0) == 0
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
            source: source.to_owned(),
            original_stamp: expected,
            copy_stamp,
            original,
            copy,
        })))
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
