use super::{Store, decode_path, encode_path};
use crate::usage_history::structured::token_checkpoint::{Checkpoint, Counter, Observation};
use crate::usage_history::structured::{
    FileState, Index, MAX_FILES, Snapshot, UsageEvent, segments, usage_provider,
};
use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Local, NaiveDate, NaiveTime, TimeZone};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
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
    cutoff: NaiveDate,
    sessions: BTreeSet<String>,
    collisions: HashMap<[u8; 32], i64>,
    append: bool,
    linear: bool,
    events: Vec<UsageEvent>,
}

#[derive(Default, Serialize, Deserialize)]
pub(in super::super) struct DayTotals {
    pub(in super::super) counts: Counter,
    pub(in super::super) records: u64,
    pub(in super::super) unresolved: u64,
    // 完整分组直接使用 counts，混合分组只保存可计价的子集。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(in super::super) priceable: Option<[u64; 3]>,
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
            let mut rows = affected.query(params![path_key, seed_json, MAX_FILES + 1])?;
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
            let conflict = event.context_conflicts;
            let flags = u8::from(conflict.model)
                | u8::from(conflict.provider) << 1
                | u8::from(conflict.role) << 2
                | u8::from(conflict.turn) << 3;
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
            let totals = groups
                .entry((
                    event.model.clone(),
                    event.provider.clone(),
                    event.role.clone(),
                    flags,
                ))
                .or_default();
            if event.count_uncertain || event.count_incomplete {
                totals.unresolved += 1;
            } else if event.delta.total_tokens > 0 {
                let previous = totals.counts.0;
                if previous.cached_known && !event.delta.cached_known && totals.records > 0 {
                    totals.priceable = Some([
                        previous.input_tokens,
                        previous.cached_input_tokens,
                        previous.output_tokens,
                    ]);
                } else if !previous.cached_known && event.delta.cached_known {
                    let [input, cached, output] = totals.priceable.get_or_insert([0; 3]);
                    *input = input.saturating_add(event.delta.input_tokens);
                    *cached = cached.saturating_add(event.delta.cached_input_tokens);
                    *output = output.saturating_add(event.delta.output_tokens);
                }
                totals.counts.0.add(event.delta);
                totals.records += 1;
            }
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
                "DELETE FROM token_resolved WHERE session_id = (SELECT id FROM token_sessions WHERE session = ?1)",
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
                    remove.execute([&session])?;
                    // 保留已过明细截止点的日汇总。
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
        mut visit: impl FnMut(String, NaiveDate, Vec<DayGroup>) -> Result<()>,
    ) -> Result<()> {
        let mut query = read.prepare(
            "SELECT s.session, d.day, d.groups FROM token_days d
                 JOIN token_sessions s ON s.id = d.session_id
                 WHERE d.day >= ?1 AND d.day < ?2 ORDER BY d.day, s.session",
        )?;
        let mut rows = query.query(params![start.to_string(), end.to_string()])?;
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
        let read = db.transaction()?;
        let (current, cutoff): (i64, String) = read.query_row(
            "SELECT revision, precise_from FROM token_revision",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
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
                Self::visit_days(&read, whole_start, whole_end, visit_archived_day)?;
            }
            missing_exact =
                !start_aligned && start_day < cutoff || !end_aligned && end_day < cutoff;
        }
        Self::visit_committed(&read, start_ms, end_ms, visit_detail)?;
        read.commit()?;
        Ok((current, missing_exact))
    }
}
