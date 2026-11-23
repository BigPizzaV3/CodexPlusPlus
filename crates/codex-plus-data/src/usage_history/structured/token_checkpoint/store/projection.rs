use super::{Store, decode_path, encode_path};
use crate::usage_history::structured::token_checkpoint::{Checkpoint, Counter, Observation};
use crate::usage_history::structured::{
    FileState, Index, MAX_FILES, Query, Snapshot, UsageEvent, segments, usage_provider,
};
use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Local, NaiveDate, NaiveTime, TimeZone};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
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
    cutoff: NaiveDate,
    sessions: BTreeSet<String>,
    collisions: HashMap<[u8; 32], i64>,
    append: bool,
    linear: bool,
    events: Vec<UsageEvent>,
}

#[derive(Clone, Copy, Default, Serialize, Deserialize)]
pub(in super::super) struct DayTotals {
    pub(in super::super) counts: Counter,
    pub(in super::super) records: u64,
    pub(in super::super) unresolved: u64,
    // 完整分组直接使用 counts，混合分组只保存可计价的子集。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(in super::super) priceable: Option<[u64; 3]>,
}

impl DayTotals {
    fn from_event(event: &UsageEvent) -> Self {
        if event.count_uncertain || event.count_incomplete {
            Self {
                unresolved: 1,
                ..Self::default()
            }
        } else if event.delta.total_tokens != 0 {
            Self {
                counts: Counter(event.delta),
                records: 1,
                ..Self::default()
            }
        } else {
            Self::default()
        }
    }

    pub(in super::super) fn merge(&mut self, other: Self) {
        if other.records != 0 {
            let incoming = other.counts.0;
            if self.counts.0.cached_known && !incoming.cached_known && self.records != 0 {
                let prior = self.counts.0;
                self.priceable = Some([
                    prior.input_tokens,
                    prior.cached_input_tokens,
                    prior.output_tokens,
                ]);
            }
            if !self.counts.0.cached_known || !incoming.cached_known {
                let known = if incoming.cached_known {
                    Some([
                        incoming.input_tokens,
                        incoming.cached_input_tokens,
                        incoming.output_tokens,
                    ])
                } else {
                    other.priceable
                };
                if let Some(known) = known {
                    let amount = self.priceable.get_or_insert([0; 3]);
                    for (sum, part) in amount.iter_mut().zip(known) {
                        *sum = sum.saturating_add(part);
                    }
                }
            }
            self.counts.0.add(incoming);
            self.records += other.records;
        }
        self.unresolved += other.unresolved;
    }

    fn value(&self) -> Value {
        let mut row = json!({"usage":self.counts.0.value(),"recordCount":self.records});
        if let Some([input, cached, output]) = self.priceable {
            row["priceableUsage"] = super::super::super::Tokens::new(input, cached, output).value();
        }
        row
    }
}

#[derive(Serialize, Deserialize)]
pub(in super::super) struct DayGroup {
    pub(in super::super) model: String,
    pub(in super::super) provider: String,
    pub(in super::super) role: String,
    pub(in super::super) conflicts: u8,
    pub(in super::super) totals: DayTotals,
}

type Groups = BTreeMap<(String, String, String, u8), DayTotals>;

pub(in super::super) struct Selection {
    pub(in super::super) revision: i64,
    pub(in super::super) days: Vec<Value>,
    pub(in super::super) unresolved: u64,
    pub(in super::super) archived: bool,
    pub(in super::super) needs_source_recovery: bool,
}

fn group_key(event: &UsageEvent) -> (String, String, String, u8) {
    let conflict = event.context_conflicts;
    let flags = u8::from(conflict.model)
        | u8::from(conflict.provider) << 1
        | u8::from(conflict.role) << 2
        | u8::from(conflict.turn) << 3;
    (
        event.model.clone(),
        event.provider.clone(),
        event.role.clone(),
        flags,
    )
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

fn linear_batch<'a>(
    state: &'a FileState,
    cutoff: NaiveDate,
    mut previous: Option<&'a UsageEvent>,
) -> bool {
    if state.session.is_empty()
        || state.continuation
        || state.history_base.is_some()
        || !state.inherited_contexts.is_empty()
        || state.identity_conflict
        || state.invalid_counts
        || state.rollout_out_of_order
        || state.issues != 0
        || state.events.is_empty()
    {
        return false;
    }
    for event in &state.events {
        if event.session != state.session
            || event.date < cutoff
            || event.inherited
            || event.legacy_identity.is_some()
            || event.provider.trim().eq_ignore_ascii_case("openai")
            || event.context_conflicts.any()
            || event.count_uncertain
            || event.count_incomplete
            || event.count_pending
            || previous.is_some_and(|before| {
                // 同一时间戳可能是副本；相同累计输入输出可能是历史缓存修正。
                DateTime::parse_from_rfc3339(&event.timestamp).unwrap()
                    <= DateTime::parse_from_rfc3339(&before.timestamp).unwrap()
                    || !event.total.is_monotonic_after(before.total)
                    || (event.total.input_tokens, event.total.output_tokens)
                        == (before.total.input_tokens, before.total.output_tokens)
            })
        {
            return false;
        }
        previous = Some(event);
    }
    true
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
        let cutoff_key = cutoff.to_string();
        let precise_from: String =
            read.query_row("SELECT precise_from FROM token_revision", [], |row| {
                row.get(0)
            })?;
        ensure!(
            cutoff_key >= precise_from,
            "Token retention boundary cannot move backward"
        );
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
        let path_key = encode_path(source);
        let seed_json = serde_json::to_string(&seeds)?;
        if !replace && let Some(previous_revision) = previous_revision {
            let tail = read
                .query_row(
                    "SELECT o.event_id, o.payload, owner.session FROM token_sources s
                 JOIN token_linear_sources l ON s.id = l.source_id
                 JOIN token_observations o ON o.source_id = s.id
                 JOIN token_sessions owner ON owner.id = o.session_id
                 WHERE s.source = ?1 AND s.revision = ?2 AND l.revision = ?2 AND l.cutoff <= ?3
                 AND NOT EXISTS (
                    SELECT 1 FROM token_source_links
                    WHERE source_id != s.id AND link IN (SELECT value FROM json_each(?4))
                 )
                 ORDER BY o.revision DESC, o.position DESC LIMIT 1",
                    params![path_key, previous_revision, cutoff_key, seed_json],
                    |row| {
                        Ok((
                            row.get::<_, Vec<u8>>(0)?,
                            row.get::<_, Vec<u8>>(1)?,
                            row.get::<_, String>(2)?,
                        ))
                    },
                )
                .optional()?;
            if let Some((id, payload, owner)) = tail {
                let tail = serde_json::from_slice::<Observation>(&payload)?.into_event(
                    id.try_into()
                        .map_err(|_| anyhow::anyhow!("invalid Token event identity"))?,
                    owner,
                )?;
                let mut collision =
                    read.prepare_cached("SELECT 1 FROM token_collisions WHERE id = ?1")?;
                let mut eligible =
                    tail.session == state.session && linear_batch(state, cutoff, Some(&tail));
                if eligible {
                    for event in &state.events {
                        if collision.exists([event.original_id.as_slice()])? {
                            eligible = false;
                            break;
                        }
                    }
                }
                drop(collision);
                if eligible {
                    let (_, checkpoint) = Checkpoint::capture(state);
                    let mut bytes = serde_json::to_vec(&checkpoint)?.len() + payload.len();
                    let mut previous = tail.total;
                    let mut events = Vec::with_capacity(state.events.len());
                    for raw in &state.events {
                        bytes += serde_json::to_vec(&Observation::capture(raw))?.len();
                        ensure!(
                            bytes <= input_limit,
                            "Token batch exceeds reconciliation input budget"
                        );
                        let mut event = raw.clone();
                        event.delta = event.session_delta(Some(previous));
                        event.provider =
                            usage_provider(&event.model, &event.provider, &index.provider_names)
                                .to_owned();
                        previous = event.total;
                        if event.delta.total_tokens != 0 {
                            events.push(event);
                        }
                    }
                    read.commit()?;
                    return Ok(Prepared {
                        update,
                        base_revision,
                        cutoff,
                        sessions,
                        collisions: HashMap::new(),
                        append: true,
                        linear: true,
                        events,
                    });
                }
            }
        }
        let mut files = 0;
        {
            // 来源和关联分别去重，避免同一会话的每份副本都重新展开整组来源。
            let mut affected = read.prepare(
                "WITH RECURSIVE
                     component(id, link) AS (
                        SELECT id, NULL FROM token_sources WHERE source = ?1
                        UNION SELECT NULL, value FROM json_each(?2)
                        UNION SELECT NULL, own.link FROM component
                           JOIN token_source_links own ON own.source_id = component.id
                        UNION SELECT neighbor.source_id, NULL FROM component
                           JOIN token_source_links neighbor ON neighbor.link = component.link
                     )
                 SELECT component.link, s.id, s.source, s.revision, s.offset, s.checkpoint
                 FROM component LEFT JOIN token_sources s ON s.id = component.id",
            )?;
            let mut observation = read.prepare(
                "SELECT o.event_id, s.session, o.payload FROM token_observations o
                 JOIN token_sessions s ON s.id = o.session_id
                 WHERE o.source_id = ?1 ORDER BY o.revision, o.position",
            )?;
            let mut stored_collision =
                read.prepare("SELECT 1 FROM token_collisions WHERE id = ?1")?;
            let mut rows = affected.query(params![path_key, seed_json])?;
            let mut remaining = input_limit;
            let mut charge = |bytes: usize| -> Result<()> {
                remaining = remaining.checked_sub(bytes).ok_or_else(|| {
                    anyhow::anyhow!("Token component exceeds reconciliation input budget")
                })?;
                Ok(())
            };
            while let Some(row) = rows.next()? {
                if let Some(link) = row.get::<_, Option<String>>(0)? {
                    charge(link.len())?;
                    if let Some(session) = link.strip_prefix("s:") {
                        sessions.insert(session.to_owned());
                    }
                    continue;
                }
                files += 1;
                ensure!(files <= MAX_FILES, "Token component exceeds source budget");
                let id: i64 = row.get(1)?;
                let path = decode_path(row.get(2)?)?;
                if path == source {
                    source_revision = Some(row.get(3)?);
                    ensure!(
                        source_revision == previous_revision,
                        "Token source changed before reconciliation"
                    );
                    if replace {
                        continue;
                    }
                }
                let record = row.get_ref(5)?.as_blob()?;
                charge(record.len())?;
                let mut old = serde_json::from_slice::<Checkpoint>(record)?.restore(row.get(4)?);
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
            if let Some(old) = index.files.remove(source) {
                if let Some((position, total)) = old.expired_count
                    && incoming
                        .expired_count
                        .is_none_or(|(current, _)| position > current)
                {
                    incoming.expired_count = Some((position, total));
                }
                incoming.events = old.events;
            }
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
        let linear = files == 0
            && collisions.is_empty()
            && index.collision_families.is_empty()
            && linear_batch(state, cutoff, None);
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
            cutoff,
            sessions,
            collisions,
            append: false,
            linear,
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
        let mut days: BTreeMap<(&str, NaiveDate), Groups> = BTreeMap::new();
        let mut prior_day = tx.prepare_cached(
            "SELECT d.groups FROM token_days d JOIN token_sessions s ON s.id = d.session_id
             WHERE s.session = ?1 AND d.day = ?2",
        )?;
        for event in &prepared.events {
            let groups = match days.entry((&event.session, event.date)) {
                std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::btree_map::Entry::Vacant(entry) => {
                    let mut groups = Groups::new();
                    if prepared.append
                        && let Some(payload) = prior_day
                            .query_row(params![event.session, event.date.to_string()], |row| {
                                row.get::<_, Vec<u8>>(0)
                            })
                            .optional()?
                    {
                        for group in serde_json::from_slice::<Vec<DayGroup>>(&payload)? {
                            groups.insert(
                                (group.model, group.provider, group.role, group.conflicts),
                                group.totals,
                            );
                        }
                    }
                    entry.insert(groups)
                }
            };
            groups
                .entry(group_key(event))
                .or_default()
                .merge(DayTotals::from_event(event));
        }
        drop(prior_day);
        let days = days
            .into_iter()
            .map(|((session, day), groups)| {
                let groups: Vec<_> = groups
                    .into_iter()
                    .map(|((model, provider, role, conflicts), totals)| DayGroup {
                        model,
                        provider,
                        role,
                        conflicts,
                        totals,
                    })
                    .collect();
                Ok((session, day.to_string(), serde_json::to_vec(&groups)?))
            })
            .collect::<Result<Vec<_>>>()?;
        let update = prepared.update;
        let next = Self::stage(
            &tx,
            update.source,
            update.previous_revision,
            update.state,
            update.replace,
        )?;
        let cutoff = prepared.cutoff.to_string();
        tx.execute("UPDATE token_revision SET precise_from = ?1", [&cutoff])?;
        {
            let mut remove = tx.prepare(
                "DELETE FROM token_resolved WHERE session_id = (SELECT id FROM token_sessions WHERE session = ?1)
                 AND day >= ?2",
            )?;
            let mut remove_days = tx.prepare(
                "DELETE FROM token_days WHERE session_id = (SELECT id FROM token_sessions WHERE session = ?1)
                 AND day >= ?2",
            )?;
            if !prepared.append {
                let mut invalidate = tx.prepare(
                    "DELETE FROM token_linear_sources WHERE source_id IN
                     (SELECT source_id FROM token_source_links WHERE link = ?1)",
                )?;
                for session in prepared.sessions {
                    // 归档后的明细由分批回收处理，日汇总继续保留。
                    remove.execute(params![session, cutoff])?;
                    remove_days.execute(params![session, cutoff])?;
                    invalidate.execute([format!("s:{session}")])?;
                }
            }
            if prepared.linear {
                tx.execute(
                    "INSERT OR REPLACE INTO token_linear_sources
                     SELECT id, ?2, ?3 FROM token_sources WHERE source = ?1",
                    params![encode_path(update.source), next, cutoff],
                )?;
            }
            let mut collide = tx.prepare("INSERT OR IGNORE INTO token_collisions VALUES (?1)")?;
            for id in prepared.collisions.keys() {
                collide.execute([id.as_slice()])?;
            }
            let mut insert = tx.prepare(
                "INSERT INTO token_resolved VALUES (?1, (SELECT id FROM token_sessions WHERE session = ?2), ?3, ?4, ?5)",
            )?;
            for event in &prepared.events {
                insert.execute(params![
                    event.id.as_slice(),
                    event.session,
                    DateTime::parse_from_rfc3339(&event.timestamp)?.timestamp_millis(),
                    event.date.to_string(),
                    serde_json::to_vec(&Observation::capture(event))?
                ])?;
            }
            let mut insert_day = tx.prepare(
                "INSERT INTO token_days VALUES ((SELECT id FROM token_sessions WHERE session = ?1), ?2, ?3)
                 ON CONFLICT(session_id, day) DO UPDATE SET groups = excluded.groups",
            )?;
            for (session, day, groups) in days {
                insert_day.execute(params![session, day, groups])?;
            }
        }
        tx.commit()?;
        Ok(next)
    }

    pub(in super::super) fn advance_retention(&mut self, cutoff: NaiveDate) -> Result<Option<i64>> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let next = tx
            .query_row(
                "UPDATE token_revision SET precise_from = ?1, revision = revision + 1
                 WHERE precise_from < ?1 RETURNING revision",
                [cutoff.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        tx.commit()?;
        Ok(next)
    }

    pub(in super::super) fn reclaim_expired_details(&mut self, batch: u32) -> Result<usize> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let cutoff: String =
            tx.query_row("SELECT precise_from FROM token_revision", [], |row| {
                row.get(0)
            })?;
        if cutoff.is_empty() {
            return Ok(0);
        }
        let day = NaiveDate::parse_from_str(&cutoff, "%Y-%m-%d")?;
        // 用现有时间索引限制读取；日期才是删除依据，UTC 偏移小于一天。
        let latest = day
            .and_time(chrono::NaiveTime::MIN)
            .and_utc()
            .timestamp_millis()
            + 86_400_000;
        let removed = tx.execute(
            "DELETE FROM token_resolved WHERE event_id IN (
                SELECT event_id FROM token_resolved
                WHERE timestamp_ms < ?1 AND day < ?2
                ORDER BY timestamp_ms LIMIT ?3
             )",
            params![latest, cutoff, batch],
        )?;
        tx.commit()?;
        Ok(removed)
    }

    #[cfg(test)]
    pub(in super::super) fn reclaim_source_observations(
        &mut self,
        source: &Path,
        after: (i64, i64),
        batch: u32,
    ) -> Result<(Option<(i64, i64)>, usize)> {
        let source_id = self.db.query_row(
            "SELECT id FROM token_sources WHERE source = ?1",
            [encode_path(source)],
            |row| row.get(0),
        )?;
        self.reclaim_observations(source_id, after, batch)
    }

    pub(in super::super) fn reclaim_next_source(
        &mut self,
        cursor: &mut (i64, (i64, i64)),
        batch: u32,
    ) -> Result<bool> {
        let (after_id, after_row) = *cursor;
        let id = if after_row == (0, 0) {
            self.db
                .query_row(
                    "SELECT id FROM token_sources WHERE id > ?1 ORDER BY id LIMIT 1",
                    [after_id],
                    |row| row.get(0),
                )
                .optional()?
        } else {
            Some(after_id)
        };
        let Some(id) = id else {
            return Ok(false);
        };
        let (next, _) = self.reclaim_observations(id, after_row, batch)?;
        *cursor = (id, next.unwrap_or((0, 0)));
        Ok(true)
    }

    fn reclaim_observations(
        &mut self,
        source_id: i64,
        after: (i64, i64),
        batch: u32,
    ) -> Result<(Option<(i64, i64)>, usize)> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let cutoff: String =
            tx.query_row("SELECT precise_from FROM token_revision", [], |row| {
                row.get(0)
            })?;
        let payload: Vec<u8> = tx.query_row(
            "SELECT checkpoint FROM token_sources WHERE id = ?1",
            [source_id],
            |row| row.get(0),
        )?;
        let mut checkpoint: Checkpoint = serde_json::from_slice(&payload)?;
        let mut next = None;
        let mut expired = Vec::new();
        {
            // 复用来源主键，保留最后一条原始观察供有序追加取基线。
            let mut query = tx.prepare(
                "SELECT revision, position, payload FROM token_observations
                 WHERE source_id = ?1 AND (revision, position) > (?2, ?3)
                   AND (revision, position) < (
                       SELECT revision, position FROM token_observations WHERE source_id = ?1
                       ORDER BY revision DESC, position DESC LIMIT 1
                   )
                 ORDER BY revision, position LIMIT ?4",
            )?;
            let mut rows = query.query(params![source_id, after.0, after.1, batch])?;
            while let Some(row) = rows.next()? {
                let key = (row.get(0)?, row.get(1)?);
                next = Some(key);
                let observation: Observation = serde_json::from_slice(row.get_ref(2)?.as_blob()?)?;
                if observation.date < cutoff {
                    expired.push(key);
                    if let Some(position) = observation.rollout_position
                        && checkpoint
                            .expired_count
                            .as_ref()
                            .is_none_or(|(current, _)| position > *current)
                    {
                        checkpoint.expired_count = Some((position, observation.total));
                    }
                }
            }
        }
        if !expired.is_empty() {
            let mut remove = tx.prepare(
                "DELETE FROM token_observations WHERE source_id = ?1 AND revision = ?2 AND position = ?3",
            )?;
            for (revision, position) in &expired {
                remove.execute(params![source_id, revision, position])?;
            }
            tx.execute(
                "UPDATE token_sources SET checkpoint = ?2 WHERE id = ?1",
                params![source_id, serde_json::to_vec(&checkpoint)?],
            )?;
        }
        tx.commit()?;
        Ok((next, expired.len()))
    }

    pub(in super::super) fn read_committed(
        db: &mut Connection,
        start_ms: i64,
        end_ms: i64,
        visit: impl FnMut(UsageEvent) -> Result<()>,
    ) -> Result<i64> {
        let read = db.transaction()?;
        let current = revision(&read)?;
        Self::visit_committed(&read, start_ms, end_ms, visit)?;
        read.commit()?;
        Ok(current)
    }

    fn visit_committed(
        read: &Connection,
        start_ms: i64,
        end_ms: i64,
        mut visit: impl FnMut(UsageEvent) -> Result<()>,
    ) -> Result<()> {
        let mut query = read.prepare(
            "SELECT r.event_id, s.session, r.payload FROM token_resolved r
                 JOIN token_sessions s ON s.id = r.session_id
                 WHERE r.timestamp_ms >= ?1 AND r.timestamp_ms < ?2
                 AND r.day >= (SELECT precise_from FROM token_revision)
                 ORDER BY r.timestamp_ms, r.event_id",
        )?;
        let mut rows = query.query([start_ms, end_ms])?;
        while let Some(row) = rows.next()? {
            let id = row.get_ref(0)?.as_blob()?.try_into()?;
            let event: Observation = serde_json::from_slice(row.get_ref(2)?.as_blob()?)?;
            visit(event.into_event(id, row.get(1)?)?)?;
        }
        Ok(())
    }

    pub(in super::super) fn read_days(
        db: &mut Connection,
        start: NaiveDate,
        end: NaiveDate,
        visit: impl FnMut(String, NaiveDate, Vec<DayGroup>) -> Result<()>,
    ) -> Result<i64> {
        let read = db.transaction()?;
        let current = revision(&read)?;
        Self::visit_days(&read, start, end, visit)?;
        read.commit()?;
        Ok(current)
    }

    fn visit_days(
        read: &Connection,
        start: NaiveDate,
        end: NaiveDate,
        visit: impl FnMut(String, NaiveDate, Vec<DayGroup>) -> Result<()>,
    ) -> Result<()> {
        Self::visit_summary(read, Some(start), Some(end), visit)
    }

    fn visit_summary(
        read: &Connection,
        start: Option<NaiveDate>,
        end: Option<NaiveDate>,
        mut visit: impl FnMut(String, NaiveDate, Vec<DayGroup>) -> Result<()>,
    ) -> Result<()> {
        let select = "SELECT s.session, d.day, d.groups FROM token_days d
                      JOIN token_sessions s ON s.id = d.session_id WHERE d.day >= ?1";
        let (sql, values) = match end {
            Some(end) => (
                format!("{select} AND d.day < ?2 ORDER BY d.day, s.session"),
                vec![
                    start.map(|day| day.to_string()).unwrap_or_default(),
                    end.to_string(),
                ],
            ),
            None => (
                format!("{select} ORDER BY d.day, s.session"),
                vec![start.map(|day| day.to_string()).unwrap_or_default()],
            ),
        };
        let mut query = read.prepare_cached(&sql)?;
        let mut rows = query.query(rusqlite::params_from_iter(values))?;
        while let Some(row) = rows.next()? {
            visit(
                row.get(0)?,
                NaiveDate::parse_from_str(row.get_ref(1)?.as_str()?, "%Y-%m-%d")?,
                serde_json::from_slice(row.get_ref(2)?.as_blob()?)?,
            )?;
        }
        Ok(())
    }

    // 旧的完整本地日与保留期内的明细不重叠；残日必须由原会话回读补齐。
    pub(in super::super) fn read_range(
        db: &mut Connection,
        start_ms: i64,
        end_ms: i64,
        visit_detail: impl FnMut(UsageEvent) -> Result<()>,
        visit_archived_day: impl FnMut(String, NaiveDate, Vec<DayGroup>) -> Result<()>,
    ) -> Result<(i64, bool)> {
        let read = db.transaction()?;
        let current = revision(&read)?;
        let missing_exact =
            Self::visit_range(&read, start_ms, end_ms, visit_detail, visit_archived_day)?;
        read.commit()?;
        Ok((current, missing_exact))
    }

    fn visit_range(
        read: &Connection,
        start_ms: i64,
        end_ms: i64,
        visit_detail: impl FnMut(UsageEvent) -> Result<()>,
        visit_archived_day: impl FnMut(String, NaiveDate, Vec<DayGroup>) -> Result<()>,
    ) -> Result<bool> {
        ensure!(start_ms < end_ms, "invalid Token range");
        let local = |ms| {
            DateTime::from_timestamp_millis(ms)
                .map(|time| time.with_timezone(&Local))
                .context("Token range follows an unsupported timestamp")
        };
        let start = local(start_ms)?;
        let end = local(end_ms)?;
        let start_day = start.date_naive();
        let end_day = end.date_naive();
        let is_midnight = |time: DateTime<Local>| {
            time.time() == NaiveTime::MIN
                && Local.from_local_datetime(&time.naive_local()).single() == Some(time)
        };
        let start_aligned = is_midnight(start);
        let end_aligned = is_midnight(end);
        let cutoff: String =
            read.query_row("SELECT precise_from FROM token_revision", [], |row| {
                row.get(0)
            })?;
        let mut missing_exact = false;
        if !cutoff.is_empty() {
            let cutoff = NaiveDate::parse_from_str(&cutoff, "%Y-%m-%d")?;
            let whole_start = if start_aligned {
                start_day
            } else {
                start_day.succ_opt().context("Token start date overflow")?
            };
            let whole_end = end_day.min(cutoff);
            if whole_start < whole_end {
                Self::visit_days(read, whole_start, whole_end, visit_archived_day)?;
            }
            missing_exact =
                !start_aligned && start_day < cutoff || !end_aligned && end_day < cutoff;
        }
        Self::visit_committed(read, start_ms, end_ms, visit_detail)?;
        Ok(missing_exact)
    }

    pub(in super::super) fn read_selection(
        db: &mut Connection,
        query: &Query,
        bounds: Option<(i64, i64)>,
        sessions: Option<&HashSet<String>>,
    ) -> Result<Selection> {
        ensure!(
            query.turn_id.is_none() && !query.include_activity && !query.include_session_cursors,
            "Token snapshot selection requires the existing compatibility reader for this query"
        );
        let read = db.transaction()?;
        let revision = revision(&read)?;
        let session_in_scope =
            |session: &str| sessions.is_none_or(|sessions| sessions.contains(session));
        let mut summary = BTreeMap::<NaiveDate, Groups>::new();
        let mut detail = BTreeMap::<NaiveDate, Groups>::new();
        let read_summary = |session: String, day: NaiveDate, groups: Vec<DayGroup>| {
            if session_in_scope(&session) {
                for group in groups {
                    let model = Some(group.model.as_str()).filter(|model| !model.is_empty());
                    if query.accepts_fields(&session, "", model, Some(day)) {
                        summary
                            .entry(day)
                            .or_default()
                            .entry((group.model, group.provider, group.role, group.conflicts))
                            .or_default()
                            .merge(group.totals);
                    }
                }
            }
            Ok(())
        };
        let (archived, needs_source_recovery) = if let Some((start, end)) = bounds {
            let needs_source_recovery = Self::visit_range(
                &read,
                start,
                end,
                |event| {
                    let model = Some(event.model.as_str()).filter(|model| !model.is_empty());
                    if session_in_scope(&event.session)
                        && query.accepts_fields(
                            &event.session,
                            &event.turn,
                            model,
                            Some(event.date),
                        )
                    {
                        detail
                            .entry(event.date)
                            .or_default()
                            .entry(group_key(&event))
                            .or_default()
                            .merge(DayTotals::from_event(&event));
                    }
                    Ok(())
                },
                read_summary,
            )?;
            (!summary.is_empty(), needs_source_recovery)
        } else {
            Self::visit_summary(
                &read,
                query.from,
                query.to.and_then(|day| day.succ_opt()),
                read_summary,
            )?;
            let precise_from: String =
                read.query_row("SELECT precise_from FROM token_revision", [], |row| {
                    row.get(0)
                })?;
            let archived = summary
                .keys()
                .next()
                .is_some_and(|date| date.to_string() < precise_from);
            (archived, false)
        };
        read.commit()?;
        let mut unresolved = 0;
        let mut days = Vec::new();
        for (date, groups) in summary.into_iter().chain(detail) {
            let mut totals = DayTotals::default();
            let mut models = BTreeMap::<String, DayTotals>::new();
            let mut breakdowns = Vec::new();
            for ((model, provider, role, conflicts), group) in groups {
                unresolved += group.unresolved;
                if group.records == 0 {
                    continue;
                }
                let mut row = group.value();
                row["model"] = json!(model);
                row["provider"] = json!(provider);
                row["role"] = json!(if conflicts & 4 != 0 { "unknown" } else { &role });
                if conflicts & 2 != 0 {
                    row["providerConflict"] = json!(true);
                }
                breakdowns.push(row);
                models.entry(model).or_default().merge(group);
                totals.merge(group);
            }
            let mut day = totals.value();
            day["date"] = json!(date.to_string());
            day["models"] = json!(
                models
                    .into_iter()
                    .map(|(model, counts)| {
                        let mut row = counts.value();
                        row["model"] = json!(model);
                        row
                    })
                    .collect::<Vec<_>>()
            );
            day["breakdowns"] = json!(breakdowns);
            days.push(day);
        }
        Ok(Selection {
            revision,
            days,
            unresolved,
            archived,
            needs_source_recovery,
        })
    }
}
