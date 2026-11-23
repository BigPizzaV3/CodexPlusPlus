use super::{Store, decode_path, encode_path};
use crate::usage_history::structured::token_checkpoint::{Checkpoint, Observation};
use crate::usage_history::structured::{
    FileState, Index, MAX_FILES, Snapshot, UsageEvent, segments,
};
use anyhow::{Result, ensure};
use chrono::{DateTime, NaiveDate};
use rusqlite::{Connection, TransactionBehavior, params};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

pub(super) fn links(path: &Path, state: &FileState) -> BTreeSet<String> {
    let mut result = BTreeSet::new();
    for session in std::iter::once(&state.session)
        .chain(
            state
                .inherited_contexts
                .iter()
                .map(|context| &context.session),
        )
        .chain(state.events.iter().map(|event| &event.session))
        .filter(|session| !session.is_empty())
    {
        result.insert(format!("s:{session}"));
    }
    for rollout in segments::rollout_id(path)
        .into_iter()
        .chain(state.history_base.as_ref().map(|base| base.rollout))
    {
        result.insert(format!("r:{rollout}"));
    }
    result
}

pub(in super::super) struct Prepared<'a> {
    update: SourceUpdate<'a>,
    base_revision: i64,
    sessions: BTreeSet<String>,
    collisions: HashMap<[u8; 32], i64>,
    events: Vec<UsageEvent>,
}

#[derive(Clone, Copy)]
pub(in super::super) struct SourceUpdate<'a> {
    pub(in super::super) source: &'a Path,
    pub(in super::super) state: &'a FileState,
    pub(in super::super) previous_revision: Option<i64>,
    pub(in super::super) replace: bool,
}

fn revision(db: &Connection) -> Result<i64> {
    Ok(db.query_row("SELECT revision FROM token_revision", [], |row| row.get(0))?)
}

impl Store {
    pub(in super::super) fn prepare<'a>(
        &mut self,
        update: SourceUpdate<'a>,
        cutoff: NaiveDate,
        provider_names: BTreeMap<String, String>,
        input_limit: usize,
    ) -> Result<Prepared<'a>> {
        let SourceUpdate {
            source,
            state,
            previous_revision,
            replace,
        } = update;
        let read = self.db.transaction()?;
        let base_revision = revision(&read)?;
        let mut source_revision = None;
        let mut index = Index {
            provider_names,
            ..Index::default()
        };
        let seeds = links(source, state);
        let mut sessions: BTreeSet<_> = seeds
            .iter()
            .filter_map(|link| link.strip_prefix("s:").map(str::to_owned))
            .collect();
        {
            // 物理前缀和同会话副本都参与计数。同一来源的旧关联和新关联均为种子。
            let mut affected = read.prepare(
                "WITH RECURSIVE
                     seeds(id) AS (
                        SELECT id FROM token_sources WHERE source = ?1
                        UNION SELECT source_id FROM token_source_links
                              WHERE link IN (SELECT value FROM json_each(?2))
                     ),
                     component(id) AS (
                        SELECT id FROM seeds
                        UNION SELECT neighbor.source_id FROM component
                           JOIN token_source_links own ON own.source_id = component.id
                           JOIN token_source_links neighbor ON neighbor.link = own.link
                        LIMIT ?3
                     )
                 SELECT s.id, s.source, s.revision, s.offset, s.checkpoint
                 FROM component JOIN token_sources s ON s.id = component.id",
            )?;
            let mut observation = read.prepare(
                "SELECT o.event_id, s.session, o.payload FROM token_observations o
                 JOIN token_sessions s ON s.id = o.session_id
                 WHERE o.source_id = ?1 ORDER BY o.revision, o.position",
            )?;
            let mut membership = read.prepare(
                "SELECT link FROM token_source_links WHERE source_id = ?1 AND link LIKE 's:%'",
            )?;
            let mut stored_collision =
                read.prepare("SELECT 1 FROM token_collisions WHERE id = ?1")?;
            let path_key = encode_path(source);
            let mut rows = affected.query(params![
                path_key,
                serde_json::to_string(&seeds)?,
                MAX_FILES + 1
            ])?;
            let mut files = 0;
            let mut remaining = input_limit;
            let mut charge = |bytes: usize| -> Result<()> {
                remaining = remaining.checked_sub(bytes).ok_or_else(|| {
                    anyhow::anyhow!("Token component exceeds reconciliation input budget")
                })?;
                Ok(())
            };
            while let Some(row) = rows.next()? {
                files += 1;
                ensure!(files <= MAX_FILES, "Token component exceeds source budget");
                let id: i64 = row.get(0)?;
                let path = decode_path(row.get(1)?)?;
                for member in membership.query_map([id], |r| r.get::<_, String>(0))? {
                    sessions.insert(member?[2..].to_owned());
                }
                if path == source {
                    source_revision = Some(row.get(2)?);
                    ensure!(
                        source_revision == previous_revision,
                        "Token source changed before reconciliation"
                    );
                    if replace {
                        continue;
                    }
                }
                let record = row.get_ref(4)?.as_blob()?;
                charge(record.len())?;
                let mut old = serde_json::from_slice::<Checkpoint>(record)?.restore(row.get(3)?);
                let mut records = observation.query([id])?;
                while let Some(record) = records.next()? {
                    let bytes = record.get_ref(2)?.as_blob()?;
                    charge(bytes.len())?;
                    let event_id = record.get_ref(0)?.as_blob()?.try_into()?;
                    let event: Observation = serde_json::from_slice(bytes)?;
                    if stored_collision.exists([event.original_id.as_slice()])? {
                        index.collision_families.insert(event.original_id);
                    }
                    old.events.push(event.into_event(event_id, record.get(1)?)?);
                }
                index.files.insert(path, old);
            }
            ensure!(
                source_revision == previous_revision,
                "Token source changed before reconciliation"
            );
            let (offset, checkpoint) = Checkpoint::capture(state);
            charge(serde_json::to_vec(&checkpoint)?.len())?;
            let mut incoming = checkpoint.restore(offset);
            incoming.events = index
                .files
                .remove(source)
                .map_or_else(Vec::new, |old| old.events);
            for event in &state.events {
                charge(serde_json::to_vec(&Observation::capture(event))?.len())?;
                if stored_collision.exists([event.original_id.as_slice()])? {
                    index.collision_families.insert(event.original_id);
                }
                incoming.events.push(event.clone());
            }
            index.files.insert(source.to_path_buf(), incoming);
        }
        read.commit()?;

        for state in index.files.values_mut() {
            state.expire_observations(cutoff);
        }

        let collisions = index.observed_collisions();
        index.collision_families.extend(collisions.keys().copied());
        index.apply_collision_ids();
        let mut snapshot = Snapshot::default();
        index.select_observations(cutoff, &mut snapshot, &mut BTreeMap::new(), usize::MAX);
        index.resolve_counts(&mut snapshot);
        snapshot.events.retain(|event| {
            event.delta.total_tokens > 0
                || event.count_uncertain
                || event.count_incomplete
                || event.context_conflicts.any()
        });
        Ok(Prepared {
            update,
            base_revision,
            sessions,
            collisions,
            events: snapshot.events,
        })
    }

    pub(in super::super) fn publish(&mut self, prepared: Prepared<'_>) -> Result<i64> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure!(
            revision(&tx)? == prepared.base_revision,
            "Token index changed during reconciliation"
        );
        let update = prepared.update;
        let next = Self::stage(
            &tx,
            update.source,
            update.previous_revision,
            update.state,
            update.replace,
        )?;
        {
            let mut remove = tx.prepare(
                "DELETE FROM token_resolved WHERE session_id = (SELECT id FROM token_sessions WHERE session = ?1)",
            )?;
            for session in prepared.sessions {
                remove.execute([session])?;
            }
            let mut collide = tx.prepare("INSERT OR IGNORE INTO token_collisions VALUES (?1)")?;
            for id in prepared.collisions.keys() {
                collide.execute([id.as_slice()])?;
            }
            let mut insert = tx.prepare(
                "INSERT INTO token_resolved VALUES (?1, (SELECT id FROM token_sessions WHERE session = ?2), ?3, ?4)",
            )?;
            for event in &prepared.events {
                insert.execute(params![
                    event.id.as_slice(),
                    event.session,
                    DateTime::parse_from_rfc3339(&event.timestamp)?.timestamp_millis(),
                    serde_json::to_vec(&Observation::capture(event))?
                ])?;
            }
        }
        tx.commit()?;
        Ok(next)
    }

    pub(in super::super) fn read_committed(
        db: &mut Connection,
        start_ms: i64,
        end_ms: i64,
        mut visit: impl FnMut(UsageEvent) -> Result<()>,
    ) -> Result<i64> {
        let read = db.transaction()?;
        let current = revision(&read)?;
        {
            let mut query = read.prepare(
                "SELECT r.event_id, s.session, r.payload FROM token_resolved r
                 JOIN token_sessions s ON s.id = r.session_id
                 WHERE r.timestamp_ms >= ?1 AND r.timestamp_ms < ?2
                 ORDER BY r.timestamp_ms, r.event_id",
            )?;
            let mut rows = query.query([start_ms, end_ms])?;
            while let Some(row) = rows.next()? {
                let id = row.get_ref(0)?.as_blob()?.try_into()?;
                let event: Observation = serde_json::from_slice(row.get_ref(2)?.as_blob()?)?;
                visit(event.into_event(id, row.get(1)?)?)?;
            }
        }
        read.commit()?;
        Ok(current)
    }
}
