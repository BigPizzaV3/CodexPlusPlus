use super::archive::{ArchiveKey, Recovered};
use super::{Coverage, Store, decode_path, encode_path};
use crate::usage_history::structured::token_checkpoint::{
    Checkpoint, Counter, EventClassification, Observation,
    historical::{EvidenceOverlay, Historical},
};
use crate::usage_history::structured::{
    AttributionScope, ClassificationEvidence, DELAYED_REQUEST_MATCH_WINDOW_MS, FileState, Index,
    MAX_ACTIVITY, MAX_FILES, Query, RequestSessionAliases, Snapshot, UsageEvent, native_endpoints,
    ordered_scope_ids, segments, terminal_receipts, unknown_model, unknown_provider,
    usage_provider,
};
use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Local, NaiveDate, NaiveTime, TimeZone};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};

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
    linear: Option<NativeTail>,
    superseded: Vec<SupersededReceipt>,
    receipt_origins: HashMap<[u8; 32], Counter>,
    events: Vec<UsageEvent>,
}

enum NativeTail {
    Keep(i64),
    Advance,
}

#[derive(Clone, Copy, Default, Serialize, Deserialize)]
pub(in super::super) struct DayTotals {
    pub(in super::super) counts: Counter,
    pub(in super::super) records: u64,
    pub(in super::super) unresolved: u64,
    // 完整分组直接使用 counts，混合分组只保存可计价的子集。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(in super::super) priceable: Option<[u64; 3]>,
    #[serde(default, skip_serializing_if = "classification_unproven")]
    evidence: ClassificationEvidence,
}

fn classification_unproven(evidence: &ClassificationEvidence) -> bool {
    *evidence == ClassificationEvidence::None
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
                evidence: ClassificationEvidence::of(event),
                ..Self::default()
            }
        } else {
            Self::default()
        }
    }

    pub(in super::super) fn merge(&mut self, other: Self) {
        if other.records != 0 {
            self.evidence = if self.records == 0 {
                other.evidence
            } else {
                self.evidence.intersect(other.evidence)
            };
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

type GroupKey = (String, String, String, u8);
type Groups = BTreeMap<GroupKey, DayTotals>;

fn committed_day_groups(db: &Connection, session: &str, day: &str) -> Result<Groups> {
    let mut read = db.prepare_cached(
        "SELECT event_id, payload FROM token_resolved
         WHERE session_id = (SELECT id FROM token_sessions WHERE session = ?1) AND day = ?2",
    )?;
    let mut rows = read.query(params![session, day])?;
    let mut groups = Groups::new();
    while let Some(row) = rows.next()? {
        let stored: Observation = serde_json::from_slice(row.get_ref(1)?.as_blob()?)?;
        let event =
            stored.into_event(row.get_ref(0)?.as_blob()?.try_into()?, session.to_owned())?;
        groups
            .entry(group_key(&event))
            .or_default()
            .merge(DayTotals::from_event(&event));
    }
    Ok(groups)
}

#[derive(Default)]
struct DayFingerprints {
    summary_only: bool,
    accounting: Sha256,
    membership: BTreeMap<GroupKey, Sha256>,
}

pub(in crate::usage_history::structured) type ArchiveAnswer =
    (ArchiveKey, Result<Recovered, String>);

fn encode_groups(groups: Groups) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(&day_groups(groups))?)
}

fn day_groups(groups: Groups) -> Vec<DayGroup> {
    groups
        .into_iter()
        .map(|((model, provider, role, conflicts), totals)| DayGroup {
            model,
            provider,
            role,
            conflicts,
            totals,
        })
        .collect()
}

pub(in super::super) struct Selection {
    pub(in super::super) revision: i64,
    pub(in super::super) coverage: Option<Coverage>,
    pub(in super::super) days: Vec<Value>,
    pub(in super::super) unresolved: u64,
    unclassified: u64,
    pub(in super::super) archived: bool,
    pub(in super::super) needs_source_recovery: bool,
    pub(in super::super) session_cursors: Option<Vec<Value>>,
    trend: Option<Value>,
    precise_from: String,
    recovered_archive: bool,
    archive_classification_gap: bool,
}

pub(in super::super) struct ScopedSelection {
    pub(in super::super) session_id: String,
    pub(in super::super) session_ids: Vec<String>,
    pub(in super::super) usage: Selection,
}

impl Selection {
    fn range_coverage(
        &self,
        query: &Query,
        bounds: Option<(i64, i64)>,
        complete: bool,
    ) -> Result<Value> {
        let mut days = BTreeMap::new();
        if complete && let Some((start, end)) = bounds {
            let mut day = local_time(start)?
                .date_naive()
                .max(query.from.unwrap_or(NaiveDate::MIN));
            if !self.recovered_archive && !self.precise_from.is_empty() {
                day = day.max(self.precise_from.parse()?);
            }
            let last = query
                .to
                .unwrap_or(NaiveDate::MAX)
                .min(local_time(end - 1)?.date_naive())
                .min(Local::now().date_naive());
            while day <= last {
                days.insert(day.to_string(), "complete");
                day = day.succ_opt().context("invalid Token coverage day")?;
            }
        }
        Ok(json!({
            "days":days,
            "precision":if self.needs_source_recovery {"day"} else {"exact"},
            "reason":if self.needs_source_recovery {"retention-day-only"} else if complete {"indexed"} else {"discovery-unconfirmed"},
        }))
    }
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

fn local_time(ms: i64) -> Result<DateTime<Local>> {
    DateTime::from_timestamp_millis(ms)
        .map(|time| time.with_timezone(&Local))
        .context("Token range follows an unsupported timestamp")
}

fn local_midnight(time: DateTime<Local>) -> bool {
    time.time() == NaiveTime::MIN
        && Local.from_local_datetime(&time.naive_local()).single() == Some(time)
}

fn retract_append_proofs(
    db: &Connection,
    first: &UsageEvent,
    correction: bool,
) -> Result<BTreeSet<NaiveDate>> {
    let at = DateTime::parse_from_rfc3339(&first.timestamp)?.timestamp_millis();
    let lower = at.saturating_sub(2 * DELAYED_REQUEST_MATCH_WINDOW_MS);
    let upper = if correction {
        at.saturating_add(2 * DELAYED_REQUEST_MATCH_WINDOW_MS)
            .saturating_add(1)
    } else {
        at
    };
    let mut days = BTreeSet::new();
    let mut retracted = Vec::new();
    {
        let mut select = db.prepare_cached(
            "SELECT event_id, payload FROM token_resolved INDEXED BY token_classified_time
             WHERE session_id = (SELECT id FROM token_sessions WHERE session = ?1)
             AND timestamp_ms >= ?2 AND timestamp_ms < ?3
             AND day >= (SELECT precise_from FROM token_revision)
             AND json_type(CAST(payload AS TEXT), '$.classification_origin') = 'object'",
        )?;
        let mut rows = select.query(params![first.session, lower, upper])?;
        while let Some(row) = rows.next()? {
            let id: [u8; 32] = row.get_ref(0)?.as_blob()?.try_into()?;
            let mut stored: Observation = serde_json::from_slice(row.get_ref(1)?.as_blob()?)?;
            let origin = stored
                .classification_origin
                .take()
                .context("missing classification origin")?;
            let receipt_origin = stored.receipt_count_origin;
            let mut event = stored.into_event(id, first.session.clone())?;
            origin.apply(&mut event);
            days.insert(event.date);
            let mut restored = Observation::capture(&event);
            restored.receipt_count_origin = receipt_origin;
            retracted.push((id, serde_json::to_vec(&restored)?));
        }
    }
    if retracted.is_empty() {
        return Ok(days);
    }
    let mut update =
        db.prepare_cached("UPDATE token_resolved SET payload = ?2 WHERE event_id = ?1")?;
    for (id, data) in retracted {
        update.execute(params![id.as_slice(), data])?;
    }
    Ok(days)
}

pub(super) fn linear_source(state: &FileState) -> bool {
    !state.session.is_empty()
        && !state.continuation
        && state.history_base.is_none()
        && state.inherited_contexts.is_empty()
        && !state.identity_conflict
        && !state.invalid_counts
        && !state.rollout_out_of_order
        && state.issues == 0
}

pub(super) fn linear_observation(
    session: &str,
    event: &UsageEvent,
    previous: Option<&UsageEvent>,
) -> bool {
    event.session == session
        && !event.inherited
        && event.legacy_identity.is_none()
        && !event.provider.trim().eq_ignore_ascii_case("openai")
        && !event.context_conflicts.any()
        && !event.count_uncertain
        && !event.count_incomplete
        && !event.count_pending
        && previous.is_none_or(|before| {
            // 同一时间戳可能是副本；相同累计输入输出可能是历史缓存修正。
            DateTime::parse_from_rfc3339(&event.timestamp).unwrap()
                > DateTime::parse_from_rfc3339(&before.timestamp).unwrap()
                && event.total.is_monotonic_after(before.total)
                && (event.total.input_tokens, event.total.output_tokens)
                    != (before.total.input_tokens, before.total.output_tokens)
        })
}

fn linear_batch<'a>(
    state: &'a FileState,
    cutoff: NaiveDate,
    mut previous: Option<&'a UsageEvent>,
) -> bool {
    if !linear_source(state) {
        return false;
    }
    for event in state
        .events
        .iter()
        .filter(|event| event.terminal_response.is_none())
    {
        if event.date < cutoff || !linear_observation(&state.session, event, previous) {
            return false;
        }
        previous = Some(event);
    }
    previous.is_some()
}

type SupersededReceipt = ([u8; 32], NaiveDate);

fn correct_sealed_counters(
    read: &Connection,
    tail: &UsageEvent,
    cutoff: NaiveDate,
    incoming: &[&UsageEvent],
    mut budget: usize,
) -> Result<Option<Vec<(UsageEvent, Option<Counter>)>>> {
    let mut targets = BTreeMap::<_, (Observation, Vec<&UsageEvent>)>::new();
    let cutoff = cutoff.to_string();
    for &receipt in incoming {
        let last = receipt.last.expect("terminal receipt has last usage");
        let same_tail = (tail.total.input_tokens, tail.total.output_tokens)
            == (receipt.total.input_tokens, receipt.total.output_tokens);
        let payload = if same_tail {
            read.query_row(
                "SELECT event_id, payload FROM token_resolved WHERE event_id = ?1 AND day >= ?2",
                params![tail.id.as_slice(), cutoff],
                |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?)),
            )
            .optional()?
        } else {
            // A certified stream has at most one counter with these cumulative totals.
            let at = DateTime::parse_from_rfc3339(&receipt.timestamp)?.timestamp_millis();
            read.query_row(
                "SELECT event_id, payload FROM token_resolved INDEXED BY token_resolved_time
                 WHERE timestamp_ms BETWEEN ?1 AND ?2 AND day >= ?3
                   AND session_id = (SELECT id FROM token_sessions WHERE session = ?4)
                   AND json_type(CAST(payload AS TEXT), '$.terminal_response') IS NULL
                   AND json_extract(CAST(payload AS TEXT), '$.turn') = ?5
                   AND CAST(payload AS TEXT) -> '$.total.i' = ?6
                   AND CAST(payload AS TEXT) -> '$.total.o' = ?7
                   AND CAST(payload AS TEXT) -> '$.last.i' = ?8
                   AND CAST(payload AS TEXT) -> '$.last.o' = ?9",
                params![
                    at.saturating_sub(DELAYED_REQUEST_MATCH_WINDOW_MS),
                    at.saturating_add(DELAYED_REQUEST_MATCH_WINDOW_MS),
                    cutoff,
                    tail.session,
                    receipt.turn,
                    receipt.total.input_tokens.to_string(),
                    receipt.total.output_tokens.to_string(),
                    last.input_tokens.to_string(),
                    last.output_tokens.to_string(),
                ],
                |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?)),
            )
            .optional()?
        };
        let Some((id, payload)) = payload else {
            return Ok(None);
        };
        budget = budget
            .checked_sub(payload.len())
            .context("Token counter corrections exceed reconciliation input budget")?;
        let id: [u8; 32] = id.as_slice().try_into()?;
        let target = serde_json::from_slice::<Observation>(&payload)?;
        if target.turn != receipt.turn
            || !target.last.is_some_and(|counter| {
                (counter.0.input_tokens, counter.0.output_tokens)
                    == (last.input_tokens, last.output_tokens)
            })
        {
            return Ok(None);
        }
        let (_, receipts) = targets.entry(id).or_insert_with(|| (target, Vec::new()));
        receipts.push(receipt);
    }
    let mut corrected = Vec::new();
    let mut claims = read.prepare_cached(
        "SELECT 1 FROM token_observations INDEXED BY token_terminal_claim
         WHERE session_id = (SELECT id FROM token_sessions WHERE session = ?1)
           AND json_type(CAST(payload AS TEXT), '$.terminal_response') = 'text'
           AND json_extract(CAST(payload AS TEXT), '$.turn') = ?2
           AND CAST(payload AS TEXT) -> '$.total.i' = ?3
           AND CAST(payload AS TEXT) -> '$.total.o' = ?4
           AND CAST(payload AS TEXT) -> '$.last.i' = ?5
           AND CAST(payload AS TEXT) -> '$.last.o' = ?6
           AND json_extract(CAST(payload AS TEXT), '$.date') >= ?7 LIMIT 1",
    )?;
    for (id, (mut stored, receipts)) in targets {
        let receipt = receipts[0];
        let last = receipt.last.expect("terminal receipt has last usage");
        let had_claim = claims.exists(params![
            tail.session,
            receipt.turn,
            receipt.total.input_tokens.to_string(),
            receipt.total.output_tokens.to_string(),
            last.input_tokens.to_string(),
            last.output_tokens.to_string(),
            cutoff,
        ])?;
        let before = stored.delta.0;
        let old_origin = stored.receipt_count_origin;
        let native = match old_origin {
            Some(base) => base.0,
            None if had_claim => {
                // An index from before origin persistence may already contain an overlay.
                if id != tail.id
                    || !terminal_receipts::wholly_accounted_in_batch(
                        std::slice::from_ref(receipt),
                        std::slice::from_ref(tail),
                    )
                {
                    return Ok(None);
                }
                continue;
            }
            None => before,
        };
        let classification = stored.classification_origin.take();
        let mut event = stored.into_event(id, tail.session.clone())?;
        if let Some(origin) = classification {
            origin.apply(&mut event);
        }
        event.delta = native;
        let added = !had_claim
            && receipts.len() == 1
            && terminal_receipts::apply_receipt_counts(&mut event, receipt);
        if event.delta != before {
            corrected.push((
                event,
                old_origin.or_else(|| added.then_some(Counter(native))),
            ));
        }
    }
    Ok(Some(corrected))
}

fn reconcile_append_receipts(
    read: &Connection,
    tail: &UsageEvent,
    cutoff: NaiveDate,
    counters: &mut Vec<UsageEvent>,
    incoming: &[&UsageEvent],
    provider_names: &BTreeMap<String, String>,
    origins: &mut HashMap<[u8; 32], Counter>,
    mut remaining: usize,
) -> Result<Option<Vec<SupersededReceipt>>> {
    if counters.is_empty() && incoming.is_empty() {
        return Ok(Some(Vec::new()));
    }
    let mut open = Vec::new();
    let mut select = read.prepare_cached(
        "SELECT event_id, payload FROM token_resolved INDEXED BY token_resolved_open_receipt
         WHERE session_id = (SELECT id FROM token_sessions WHERE session = ?1)
         AND day >= ?2 AND json_type(CAST(payload AS TEXT), '$.terminal_response') = 'text'",
    )?;
    let mut rows = select.query(params![tail.session, cutoff.to_string()])?;
    while let Some(row) = rows.next()? {
        let payload = row.get_ref(1)?.as_blob()?;
        remaining = remaining
            .checked_sub(payload.len())
            .context("Token receipt suffix exceeds reconciliation input budget")?;
        let mut observation: Observation = serde_json::from_slice(payload)?;
        let origin = observation.classification_origin.take();
        let mut event =
            observation.into_event(row.get_ref(0)?.as_blob()?.try_into()?, tail.session.clone())?;
        if let Some(origin) = origin {
            origin.apply(&mut event);
        }
        if event.count_uncertain || event.count_incomplete {
            return Ok(None);
        }
        open.push(event);
    }
    let mut candidates = open.iter().collect::<Vec<_>>();
    let mut sealed = Vec::new();
    for &receipt in incoming {
        if receipt.session != tail.session || receipt.date < cutoff {
            return Ok(None);
        }
        if tail.total.is_monotonic_after(receipt.total) {
            sealed.push(receipt);
        } else if !receipt.total.is_monotonic_after(tail.total) {
            return Ok(None);
        } else {
            candidates.push(receipt);
        }
    }
    if !terminal_receipts::unique_claims(candidates.iter().copied())
        || incoming
            .iter()
            .map(|receipt| receipt.original_id)
            .collect::<HashSet<_>>()
            .len()
            != incoming.len()
    {
        return Ok(None);
    }
    let latest = counters.last().unwrap_or(tail).total;
    if !candidates.is_empty() {
        counters.insert(0, tail.clone());
        origins.extend(
            terminal_receipts::reconcile(candidates.iter().copied(), counters, provider_names)
                .into_iter()
                .map(|(id, native)| (id, Counter(native))),
        );
        counters.remove(0);
    }
    let mut emitted = HashSet::new();
    for event in counters
        .iter()
        .filter(|event| event.terminal_response.is_some())
    {
        if event.count_uncertain || event.count_incomplete {
            return Ok(None);
        }
        emitted.insert(event.id);
    }
    // A hidden receipt beyond the anchor could corroborate a future counter. Never
    // drop it from the open suffix merely because its time precedes a newer counter.
    if candidates
        .iter()
        .any(|receipt| !emitted.contains(&receipt.id) && !latest.is_monotonic_after(receipt.total))
    {
        return Ok(None);
    }
    let previous: HashSet<_> = open.iter().map(|event| event.id).collect();
    counters.retain(|event| !previous.contains(&event.id));
    let mut superseded: Vec<_> = open
        .into_iter()
        .filter(|event| !emitted.contains(&event.id))
        .map(|event| (event.id, event.date))
        .collect();
    if !sealed.is_empty() {
        let Some(corrected) = correct_sealed_counters(read, tail, cutoff, &sealed, remaining)?
        else {
            return Ok(None);
        };
        for (event, base) in corrected {
            superseded.push((event.id, event.date));
            if let Some(base) = base {
                origins.insert(event.id, base);
            }
            counters.push(event);
        }
    }
    Ok(Some(superseded))
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
                    "SELECT o.event_id, o.payload, owner.session, l.revision FROM token_sources s
                 JOIN token_linear_sources l ON s.id = l.source_id
                 JOIN token_observations o ON o.source_id = s.id AND o.revision = l.revision
                 JOIN token_sessions owner ON owner.id = o.session_id
                 WHERE s.source = ?1 AND s.revision = ?2 AND l.cutoff <= ?3
                 AND json_type(CAST(o.payload AS TEXT), '$.terminal_response') IS NULL
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
                            row.get::<_, i64>(3)?,
                        ))
                    },
                )
                .optional()?;
            if let Some((id, payload, owner, native_revision)) = tail {
                let tail = serde_json::from_slice::<Observation>(&payload)?.into_event(
                    id.try_into()
                        .map_err(|_| anyhow::anyhow!("invalid Token event identity"))?,
                    owner,
                )?;
                let mut eligible =
                    tail.session == state.session && linear_batch(state, cutoff, Some(&tail));
                if eligible {
                    let mut collision =
                        read.prepare_cached("SELECT 1 FROM token_collisions WHERE id = ?1")?;
                    let mut prior_receipt = read.prepare_cached(
                        "SELECT 1 FROM token_observations WHERE event_id = ?1
                         AND json_type(CAST(payload AS TEXT), '$.terminal_response') = 'text'
                         LIMIT 1",
                    )?;
                    for event in &state.events {
                        let id = [event.original_id.as_slice()];
                        if collision.exists(id)?
                            || (event.terminal_response.is_some() && prior_receipt.exists(id)?)
                        {
                            eligible = false;
                            break;
                        }
                    }
                }
                if eligible {
                    let (_, checkpoint) = Checkpoint::capture(state);
                    let mut bytes = serde_json::to_vec(&checkpoint)?.len() + payload.len();
                    let mut previous = tail.total;
                    let mut events = Vec::with_capacity(state.events.len().saturating_add(1));
                    let mut incoming = Vec::new();
                    for raw in &state.events {
                        bytes += serde_json::to_vec(&Observation::capture(raw))?.len();
                        ensure!(
                            bytes <= input_limit,
                            "Token batch exceeds reconciliation input budget"
                        );
                        if raw.terminal_response.is_some() {
                            incoming.push(raw);
                            continue;
                        }
                        let mut event = raw.clone();
                        event.delta = event.session_delta(Some(previous));
                        event.provider =
                            usage_provider(&event.model, &event.provider, &index.provider_names)
                                .to_owned();
                        previous = event.total;
                        events.push(event);
                    }
                    let linear = if events.is_empty() {
                        NativeTail::Keep(native_revision)
                    } else {
                        NativeTail::Advance
                    };
                    let mut receipt_origins = HashMap::new();
                    if state.previous == Some(previous)
                        && let Some(superseded) = reconcile_append_receipts(
                            &read,
                            &tail,
                            cutoff,
                            &mut events,
                            &incoming,
                            &index.provider_names,
                            &mut receipt_origins,
                            input_limit.saturating_sub(bytes),
                        )?
                    {
                        events.retain(|event| event.delta.total_tokens != 0);
                        read.commit()?;
                        return Ok(Prepared {
                            update,
                            base_revision,
                            cutoff,
                            sessions,
                            collisions: HashMap::new(),
                            append: true,
                            linear: Some(linear),
                            superseded,
                            receipt_origins,
                            events,
                        });
                    }
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
        let linear_candidate = files == 0
            && collisions.is_empty()
            && index.collision_families.is_empty()
            && linear_batch(state, cutoff, None);
        index.collision_families.extend(collisions.keys().copied());
        index.apply_collision_ids();
        let mut snapshot = Snapshot::default();
        index.select_observations(cutoff, &mut snapshot, &mut BTreeMap::new(), usize::MAX);
        index.resolve_counts(&mut snapshot);
        let linear = linear_candidate
            && terminal_receipts::wholly_accounted_in_batch(&state.events, &snapshot.events);
        let receipt_origins = terminal_receipts::project(&index, &mut snapshot)
            .into_iter()
            .map(|(id, native)| (id, Counter(native)))
            .collect();
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
            linear: linear.then_some(NativeTail::Advance),
            superseded: Vec::new(),
            receipt_origins,
            events: snapshot.events,
        })
    }

    pub(in super::super) fn publish(&mut self, prepared: Prepared<'_>) -> Result<i64> {
        self.publish_source(prepared, false)
    }

    pub(in super::super) fn retire_source(
        &mut self,
        source: &Path,
        previous_revision: i64,
        cutoff: NaiveDate,
        provider_names: BTreeMap<String, String>,
        input_limit: usize,
    ) -> Result<i64> {
        let empty = FileState::default();
        let prepared = self.prepare(
            SourceUpdate {
                source,
                state: &empty,
                previous_revision: Some(previous_revision),
                replace: true,
            },
            cutoff,
            provider_names,
            input_limit,
        )?;
        self.publish_source(prepared, true)
    }

    fn publish_source(&mut self, prepared: Prepared<'_>, retire: bool) -> Result<i64> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure!(
            revision(&tx)? == prepared.base_revision,
            "Token index changed during reconciliation"
        );
        let session = prepared.update.state.session.as_str();
        let mut rebuilt = BTreeSet::new();
        if !prepared.superseded.is_empty() {
            let mut remove = tx.prepare_cached("DELETE FROM token_resolved WHERE event_id = ?1")?;
            for (id, day) in &prepared.superseded {
                remove.execute([id.as_slice()])?;
                rebuilt.insert(*day);
            }
        }
        if prepared.append {
            let previous: HashSet<_> = prepared.superseded.iter().map(|(id, _)| *id).collect();
            let mut first_new: Option<&UsageEvent> = None;
            for event in &prepared.events {
                if previous.contains(&event.id) {
                    rebuilt.extend(retract_append_proofs(&tx, event, true)?);
                } else if first_new.is_none_or(|first| event.timestamp < first.timestamp) {
                    first_new = Some(event);
                }
            }
            // A newly committed competitor can make a nearby request non-unique.
            if let Some(first) = first_new {
                rebuilt.extend(retract_append_proofs(&tx, first, false)?);
            }
        }
        let mut days = BTreeMap::new();
        for day in rebuilt {
            days.insert(
                (session, day),
                committed_day_groups(&tx, session, &day.to_string())?,
            );
        }
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
                let bytes = (!groups.is_empty())
                    .then(|| encode_groups(groups))
                    .transpose()?;
                Ok((session, day.to_string(), bytes))
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
        if retire {
            let path = encode_path(update.source);
            tx.execute("DELETE FROM token_linear_sources WHERE source_id = (SELECT id FROM token_sources WHERE source = ?1)", [&path])?;
            tx.execute("DELETE FROM token_source_links WHERE source_id = (SELECT id FROM token_sources WHERE source = ?1)", [&path])?;
            tx.execute("DELETE FROM token_sources WHERE source = ?1", [&path])?;
        }
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
            if !retire && let Some(linear) = prepared.linear {
                let anchor = match linear {
                    NativeTail::Advance => next,
                    NativeTail::Keep(revision) => revision,
                };
                tx.execute(
                    "INSERT OR REPLACE INTO token_linear_sources
                     SELECT id, ?2, ?3 FROM token_sources WHERE source = ?1",
                    params![encode_path(update.source), anchor, cutoff],
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
                let mut stored = Observation::capture(event);
                stored.receipt_count_origin = prepared.receipt_origins.get(&event.id).copied();
                insert.execute(params![
                    event.id.as_slice(),
                    event.session,
                    DateTime::parse_from_rfc3339(&event.timestamp)?.timestamp_millis(),
                    event.date.to_string(),
                    serde_json::to_vec(&stored)?
                ])?;
            }
            let mut insert_day = tx.prepare(
                "INSERT INTO token_days VALUES ((SELECT id FROM token_sessions WHERE session = ?1), ?2, ?3)
                 ON CONFLICT(session_id, day) DO UPDATE SET groups = excluded.groups",
            )?;
            let mut remove_day = tx.prepare_cached(
                "DELETE FROM token_days WHERE day = ?2
                 AND session_id = (SELECT id FROM token_sessions WHERE session = ?1)",
            )?;
            for (session, day, groups) in days {
                if let Some(groups) = groups {
                    insert_day.execute(params![session, day, groups])?;
                } else {
                    remove_day.execute(params![session, day])?;
                }
            }
        }
        tx.commit()?;
        Ok(next)
    }

    pub(in super::super) fn recent_sessions(&self, since: i64) -> Result<Vec<(String, i64)>> {
        let mut statement = self.db.prepare_cached(
            "SELECT s.session, max(r.timestamp_ms) AS latest
             FROM token_resolved r JOIN token_sessions s ON s.id = r.session_id
             WHERE r.timestamp_ms >= ?1 AND r.day >= (SELECT precise_from FROM token_revision)
             GROUP BY r.session_id ORDER BY latest DESC, s.session",
        )?;
        let sessions = statement.query_map([since], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(sessions.collect::<rusqlite::Result<_>>()?)
    }

    pub(in super::super) fn classification_before(
        &self,
        session: &str,
        before: i64,
    ) -> Result<Option<i64>> {
        Ok(self
            .db
            .prepare_cached(
                "SELECT timestamp_ms FROM token_resolved
             WHERE session_id = (SELECT id FROM token_sessions WHERE session = ?1)
             AND timestamp_ms < ?2 AND day >= (SELECT precise_from FROM token_revision)
             ORDER BY timestamp_ms DESC LIMIT 1",
            )?
            .query_row(params![session, before], |row| row.get(0))
            .optional()?)
    }

    /// Re-evaluate a canonical session from its committed native identities, never from
    /// the supplier identities written in the last classification pass.
    #[cfg(test)]
    pub(in super::super) fn reclassify_session(
        &mut self,
        session: &str,
        input_limit: usize,
        scoped_count_gaps: &BTreeSet<PathBuf>,
        classify: impl FnOnce(&mut [UsageEvent], &BTreeMap<String, String>) -> Result<()>,
    ) -> Result<Option<i64>> {
        self.reclassify_window(
            session,
            None,
            input_limit,
            scoped_count_gaps,
            |events, aliases, _| classify(events, aliases),
        )
    }

    pub(in super::super) fn reclassify_window(
        &mut self,
        session: &str,
        window: Option<Range<i64>>,
        input_limit: usize,
        scoped_count_gaps: &BTreeSet<PathBuf>,
        classify: impl FnOnce(
            &mut [UsageEvent],
            &BTreeMap<String, String>,
            &AttributionScope,
        ) -> Result<()>,
    ) -> Result<Option<i64>> {
        let read = self.db.transaction()?;
        let base = revision(&read)?;
        let eligible = read.query_row(
            "SELECT EXISTS(SELECT 1 FROM token_coverage WHERE revision = ?1
                           AND pending_sources = 0 AND issues = ?2 AND NOT maintenance_blocked)",
            params![base, scoped_count_gaps.len()],
            |row| row.get::<_, bool>(0),
        )?;
        if !eligible {
            return Ok(None);
        }
        if !scoped_count_gaps.is_empty() {
            let mut related = read.prepare_cached(
                "WITH RECURSIVE component(id, link) AS (
                    SELECT NULL, ?1
                    UNION SELECT NULL, own.link FROM component
                        JOIN token_source_links own ON own.source_id = component.id
                    UNION SELECT neighbor.source_id, NULL FROM component
                        JOIN token_source_links neighbor ON neighbor.link = component.link
                 )
                 SELECT source FROM component JOIN token_sources s ON s.id = component.id",
            )?;
            let mut rows = related.query([format!("s:{session}")])?;
            while let Some(row) = rows.next()? {
                if scoped_count_gaps.contains(&decode_path(row.get(0)?)?) {
                    return Ok(None);
                }
            }
        }
        let scope = Self::scope(&read, &BTreeMap::new())?;
        let canonical = RequestSessionAliases::new(&scope.aliases);
        if !scope.sessions.contains(session)
            || canonical.resolve(session).as_deref() != Some(session)
        {
            return Ok(None);
        }
        let (targets, bounds, gateway_blocked, native_blocked) = if let Some(window) = window {
            let cutoff: String =
                read.query_row("SELECT precise_from FROM token_revision", [], |row| {
                    row.get(0)
                })?;
            let first = cutoff
                .parse::<NaiveDate>()?
                .and_time(NaiveTime::MIN)
                .and_local_timezone(Local)
                .earliest()
                .context("invalid classification retention date")?
                .timestamp_millis();
            let left = native_endpoints::WINDOW_MS.max(2 * DELAYED_REQUEST_MATCH_WINDOW_MS);
            let right = 2 * DELAYED_REQUEST_MATCH_WINDOW_MS;
            // A clipped competition window cannot certify a supplier or withdraw one.
            let start = window.start.max(first.saturating_add(left));
            if start >= window.end {
                return Ok(None);
            }
            let mut gaps = read.prepare_cached(
                "SELECT 1 FROM token_resolved
                 WHERE session_id = (SELECT id FROM token_sessions WHERE session = ?1)
                 AND day >= ?2
                 AND (json_extract(CAST(payload AS TEXT), '$.count_incomplete') = 1
                      OR json_extract(CAST(payload AS TEXT), '$.delta.k') = 0)
                 AND (?3 = 0 OR json_extract(CAST(payload AS TEXT), '$.count_incomplete') = 1)
                 LIMIT 1",
            )?;
            let gateway_blocked = gaps.exists(params![session, cutoff, 1])?;
            let native_blocked = gateway_blocked || gaps.exists(params![session, cutoff, 0])?;
            (
                start..window.end,
                start.saturating_sub(left)..window.end.saturating_add(right),
                gateway_blocked,
                native_blocked,
            )
        } else {
            (i64::MIN..i64::MAX, i64::MIN..i64::MAX, false, false)
        };
        let mut previous = Vec::new();
        let mut candidates = Vec::new();
        let mut target_rows = 0..0;
        {
            let mut budget = input_limit;
            let mut loaded = read.prepare_cached(
                "SELECT event_id, payload, timestamp_ms FROM token_resolved
                 WHERE session_id = (SELECT id FROM token_sessions WHERE session = ?1)
                 AND day >= (SELECT precise_from FROM token_revision)
                 AND timestamp_ms >= ?2 AND timestamp_ms < ?3
                 ORDER BY timestamp_ms, event_id",
            )?;
            let mut rows = loaded.query(params![session, bounds.start, bounds.end])?;
            while let Some(row) = rows.next()? {
                let at = row.get::<_, i64>(2)?;
                target_rows.start += usize::from(at < targets.start);
                target_rows.end += usize::from(at < targets.end);
                let bytes = row.get_ref(1)?.as_blob()?;
                budget = budget
                    .checked_sub(bytes.len())
                    .context("Token classification window exceeds input budget")?;
                let stored: Observation = serde_json::from_slice(bytes)?;
                let origin = stored.classification_origin.clone();
                let receipt_origin = stored.receipt_count_origin;
                let event = stored
                    .into_event(row.get_ref(0)?.as_blob()?.try_into()?, session.to_owned())?;
                let mut candidate = event.clone();
                if let Some(origin) = &origin {
                    origin.apply(&mut candidate);
                }
                previous.push((event, origin, receipt_origin));
                candidates.push(candidate);
            }
        }
        read.commit()?;
        if target_rows.is_empty() {
            return Ok(None);
        }
        let attribution = AttributionScope {
            targets: target_rows.clone(),
            gateway_blocked,
            native_blocked,
        };
        classify(&mut candidates, &scope.aliases, &attribution)?;
        let mut changed = Vec::new();
        let mut changed_days = BTreeSet::new();
        for ((event, origin, receipt_origin), candidate) in previous[target_rows.clone()]
            .iter_mut()
            .zip(&candidates[target_rows])
        {
            let prior = EventClassification::of(event);
            let next = EventClassification::of(candidate);
            if prior != next {
                let native = origin.take().unwrap_or(prior);
                next.apply(event);
                let mut stored = Observation::capture(event);
                stored.classification_origin = (native != next).then_some(native);
                stored.receipt_count_origin = *receipt_origin;
                changed.push((event.id, serde_json::to_vec(&stored)?));
                changed_days.insert(event.date);
            }
        }
        if changed.is_empty() {
            return Ok(None);
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure!(
            revision(&tx)? == base,
            "Token index changed during classification"
        );
        {
            let mut update =
                tx.prepare("UPDATE token_resolved SET payload = ?2 WHERE event_id = ?1")?;
            for (id, bytes) in changed {
                update.execute(params![id.as_slice(), bytes])?;
            }
            let mut update_days = tx.prepare(
                "UPDATE token_days SET groups = ?3 WHERE day = ?2
                 AND session_id = (SELECT id FROM token_sessions WHERE session = ?1)",
            )?;
            for date in changed_days {
                let date = date.to_string();
                let groups = committed_day_groups(&tx, session, &date)?;
                update_days.execute(params![session, date, encode_groups(groups)?])?;
            }
        }
        let next = tx.query_row(
            "UPDATE token_revision SET revision = revision + 1,
                updated_at = max(updated_at + 1, ?1) RETURNING revision",
            [Local::now().timestamp_millis()],
            |row| row.get::<_, i64>(0),
        )?;
        tx.execute(
            "UPDATE token_coverage SET revision = ?1 WHERE revision = ?2",
            params![next, base],
        )?;
        tx.commit()?;
        Ok(Some(next))
    }

    pub(in super::super) fn advance_retention(&mut self, cutoff: NaiveDate) -> Result<Option<i64>> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let next = tx
            .query_row(
                "UPDATE token_revision SET precise_from = ?1, revision = revision + 1,
                    updated_at = max(updated_at + 1, ?2)
                 WHERE precise_from < ?1 RETURNING revision",
                params![cutoff.to_string(), Local::now().timestamp_millis()],
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

    pub(in super::super) fn reclaim_expired_activity(
        &self,
        cutoff: NaiveDate,
        batch: u32,
    ) -> Result<usize> {
        // 与 Token 明细共用按本地日淘汰的边界；时间索引限定候选。
        let latest = cutoff.and_time(NaiveTime::MIN).and_utc().timestamp_millis() + 86_400_000;
        Ok(self.db.execute(
            "DELETE FROM token_activity WHERE (source_id, session_id, call_id) IN
             (SELECT a.source_id, a.session_id, a.call_id FROM token_activity a
              WHERE a.timestamp_ms < ?1 AND a.day < ?2 AND NOT EXISTS (
                  SELECT 1 FROM execution_observations e
                  WHERE e.session_id = a.session_id AND e.call_id = a.call_id AND e.is_output = 0
              ) ORDER BY a.timestamp_ms LIMIT ?3)",
            params![latest, cutoff.to_string(), batch],
        )?)
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
        let id = self
            .db
            .query_row(
                "SELECT id FROM token_sources WHERE id > ?1 OR (id = ?1 AND ?2)
                 ORDER BY id LIMIT 1",
                params![after_id, after_row != (0, 0)],
                |row| row.get(0),
            )
            .optional()?;
        let Some(id) = id else {
            return Ok(false);
        };
        let from = if id == after_id { after_row } else { (0, 0) };
        let (next, _) = self.reclaim_observations(id, from, batch)?;
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
            // 有序来源保护最新提交中的原生累计基线，其他来源仍保护尾记录。
            let mut query = tx.prepare(
                "SELECT revision, position, payload FROM token_observations
                 WHERE source_id = ?1 AND (revision, position) > (?2, ?3)
                   AND (revision, position) IS NOT (
                       SELECT tail.revision, tail.position FROM (
                           SELECT o.revision, o.position FROM token_linear_sources l
                           JOIN token_observations o ON o.source_id = l.source_id AND o.revision = l.revision
                           WHERE l.source_id = ?1
                           AND json_type(CAST(o.payload AS TEXT), '$.terminal_response') IS NULL
                           ORDER BY o.position DESC LIMIT 1
                       ) tail
                       UNION ALL
                       SELECT tail.revision, tail.position FROM (
                           SELECT revision, position FROM token_observations
                           WHERE source_id = ?1 ORDER BY revision DESC, position DESC LIMIT 1
                       ) tail
                       WHERE NOT EXISTS (SELECT 1 FROM token_linear_sources WHERE source_id = ?1)
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
                    if observation.terminal_response.is_none()
                        && let Some(position) = observation.rollout_position
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
        Self::visit_committed(&read, start_ms, end_ms, None, false, visit)?;
        read.commit()?;
        Ok(current)
    }

    fn visit_committed(
        read: &Connection,
        start_ms: i64,
        end_ms: i64,
        query: Option<&Query>,
        event_id_order: bool,
        mut visit: impl FnMut(UsageEvent) -> Result<()>,
    ) -> Result<()> {
        let order = if event_id_order {
            "r.event_id"
        } else {
            "r.timestamp_ms, r.event_id"
        };
        let mut statement = read.prepare_cached(&format!(
            "SELECT r.event_id, s.session, r.payload FROM token_resolved r
                 JOIN token_sessions s ON s.id = r.session_id
                 WHERE r.timestamp_ms >= ?1 AND r.timestamp_ms < ?2
                 AND r.day >= (SELECT precise_from FROM token_revision)
                 AND (?3 IS NULL OR r.day >= ?3)
                 AND (?4 IS NULL OR r.day <= ?4)
                 AND (?5 IS NULL OR s.session = ?5)
                 AND (?6 IS NULL OR json_extract(CAST(r.payload AS TEXT), '$.turn') = ?6)
                 ORDER BY {order}",
        ))?;
        let from = query.and_then(|q| q.from).map(|day| day.to_string());
        let to = query.and_then(|q| q.to).map(|day| day.to_string());
        let session = query.and_then(|q| q.session_id.as_deref());
        let turn = query.and_then(|q| q.turn_id.as_deref());
        let mut rows = statement.query(params![start_ms, end_ms, from, to, session, turn])?;
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

    pub(in super::super) fn historical_candidates(
        read: &Connection,
        timestamps: &BTreeSet<i64>,
    ) -> Result<Vec<UsageEvent>> {
        let mut events = Vec::new();
        let mut remaining: usize = 32 * 1024 * 1024;
        let mut statement = read.prepare_cached(
            "SELECT r.event_id, s.session, r.payload FROM token_resolved r
             JOIN token_sessions s ON s.id = r.session_id
             WHERE r.timestamp_ms = ?1 AND r.day >= (SELECT precise_from FROM token_revision)",
        )?;
        for timestamp in timestamps {
            let mut rows = statement.query([timestamp])?;
            while let Some(row) = rows.next()? {
                let raw = row.get_ref(2)?.as_blob()?;
                remaining = remaining
                    .checked_sub(raw.len())
                    .context("historical classification exceeds candidate budget")?;
                let event: Observation = serde_json::from_slice(raw)?;
                events.push(event.into_event(row.get_ref(0)?.as_blob()?.try_into()?, row.get(1)?)?);
            }
        }
        Ok(events)
    }

    pub(in super::super) fn historical_days(
        read: &Connection,
        affected: &BTreeSet<(NaiveDate, String)>,
        changes: &HashMap<[u8; 32], EventClassification>,
    ) -> Result<BTreeMap<NaiveDate, HashMap<String, Vec<DayGroup>>>> {
        let mut days = BTreeMap::<_, HashMap<_, _>>::new();
        let mut statement = read.prepare_cached(
            "SELECT r.event_id, r.payload FROM token_resolved r
             WHERE r.session_id = (SELECT id FROM token_sessions WHERE session = ?1)
             AND r.timestamp_ms >= ?2 AND r.timestamp_ms < ?3 AND r.day = ?4
             ORDER BY r.timestamp_ms, r.event_id",
        )?;
        for (day, session) in affected {
            let midnight = |day: NaiveDate| {
                Local
                    .from_local_datetime(&day.and_time(NaiveTime::MIN))
                    .earliest()
                    .map(|at| at.timestamp_millis())
                    .context("historical date has no local midnight")
            };
            let end = day.succ_opt().context("historical date overflow")?;
            let mut rows = statement.query(params![
                session,
                midnight(*day)?,
                midnight(end)?,
                day.to_string()
            ])?;
            let mut groups = Groups::new();
            while let Some(row) = rows.next()? {
                let raw: Observation = serde_json::from_slice(row.get_ref(1)?.as_blob()?)?;
                let mut event =
                    raw.into_event(row.get_ref(0)?.as_blob()?.try_into()?, session.clone())?;
                if let Some(classification) = changes.get(&event.id) {
                    classification.apply(&mut event);
                }
                groups
                    .entry(group_key(&event))
                    .or_default()
                    .merge(DayTotals::from_event(&event));
            }
            days.entry(*day)
                .or_default()
                .insert(session.clone(), day_groups(groups));
        }
        Ok(days)
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

    // 旧的完整本地日与保留期内的明细不重叠；摘要不能证明旧范围完整。
    pub(in super::super) fn read_range(
        db: &mut Connection,
        start_ms: i64,
        end_ms: i64,
        visit_detail: impl FnMut(UsageEvent) -> Result<()>,
        visit_archived_day: impl FnMut(String, NaiveDate, Vec<DayGroup>) -> Result<()>,
    ) -> Result<(i64, bool)> {
        let read = db.transaction()?;
        let current = revision(&read)?;
        let needs_source_recovery = Self::visit_range(
            &read,
            (start_ms, end_ms),
            None,
            false,
            visit_detail,
            Some(visit_archived_day),
        )?;
        read.commit()?;
        Ok((current, needs_source_recovery))
    }

    fn visit_range(
        read: &Connection,
        (start_ms, end_ms): (i64, i64),
        query: Option<&Query>,
        event_id_order: bool,
        visit_detail: impl FnMut(UsageEvent) -> Result<()>,
        visit_archived_day: Option<impl FnMut(String, NaiveDate, Vec<DayGroup>) -> Result<()>>,
    ) -> Result<bool> {
        ensure!(start_ms < end_ms, "invalid Token range");
        let start = local_time(start_ms)?;
        let end = local_time(end_ms)?;
        let start_day = start.date_naive();
        let end_day = end.date_naive();
        let cutoff: String =
            read.query_row("SELECT precise_from FROM token_revision", [], |row| {
                row.get(0)
            })?;
        let mut needs_source_recovery = false;
        if !cutoff.is_empty() {
            let cutoff = NaiveDate::parse_from_str(&cutoff, "%Y-%m-%d")?;
            let whole_start = if local_midnight(start) {
                start_day
            } else {
                start_day.succ_opt().context("Token start date overflow")?
            };
            let whole_end = end_day.min(cutoff);
            if whole_start < whole_end
                && let Some(visit_archived_day) = visit_archived_day
            {
                Self::visit_days(read, whole_start, whole_end, visit_archived_day)?;
            }
            needs_source_recovery = start_day < cutoff;
        }
        Self::visit_committed(read, start_ms, end_ms, query, event_id_order, visit_detail)?;
        Ok(needs_source_recovery)
    }

    pub(in super::super) fn read_selection(
        db: &mut Connection,
        query: &Query,
        bounds: Option<(i64, i64)>,
        sessions: Option<&HashSet<String>>,
    ) -> Result<Selection> {
        let read = db.transaction()?;
        let [selection] =
            Self::select(&read, query, bounds, [sessions], (false, false), None, None)?;
        read.commit()?;
        Ok(selection)
    }

    pub(in super::super) fn read_scoped_selection(
        db: &mut Connection,
        query: &Query,
        bounds: Option<(i64, i64)>,
        requested: &str,
    ) -> Result<(Selection, ScopedSelection)> {
        let read = db.transaction()?;
        let scope = Self::scope(&read, &BTreeMap::new())?;
        let (session, sessions) = scope.scope_members(requested);
        let [all, usage] = Self::select(
            &read,
            query,
            bounds,
            [None, Some(&sessions)],
            (false, false),
            None,
            None,
        )?;
        read.commit()?;
        Ok((
            all,
            ScopedSelection {
                session_id: session.to_owned(),
                session_ids: ordered_scope_ids(session, &sessions)
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                usage,
            },
        ))
    }

    #[cfg(test)]
    pub(in super::super) fn read_response(
        db: &mut Connection,
        query: &Query,
        request: &Value,
        requested: Option<&str>,
        provider_names: &BTreeMap<String, String>,
    ) -> Result<Value> {
        Self::read_response_with_archive(db, query, request, requested, provider_names, None, None)
            .map(|(response, _)| response)
    }

    pub(in super::super) fn read_response_with_archive(
        db: &mut Connection,
        query: &Query,
        request: &Value,
        requested: Option<&str>,
        provider_names: &BTreeMap<String, String>,
        archived: Option<&ArchiveAnswer>,
        historical: Option<&Historical>,
    ) -> Result<(Value, Option<ArchiveKey>)> {
        let read = db.transaction()?;
        let bounds = request["startAt"].as_i64().zip(request["endAt"].as_i64());
        let activity_page = request["activityPage"].as_u64().unwrap_or(1).clamp(1, 24) as usize;
        let include_trend = request["includeTrend"] == true;
        let include_fingerprints = request["includeFingerprints"] != false;
        let (current_revision, cutoff): (i64, String) = read.query_row(
            "SELECT revision, precise_from FROM token_revision",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let mut historical_error = None;
        let historical =
            historical.and_then(|evidence| match evidence.is_current(current_revision) {
                Ok(true) => Some(evidence),
                Ok(false) => None,
                Err(error) => {
                    historical_error = Some(format!("{error:#}"));
                    None
                }
            });
        let archive_key = if let Some((start, end)) = bounds
            && !cutoff.is_empty()
        {
            ensure!(start < end, "invalid Token archive range");
            let first = local_time(start)?
                .date_naive()
                .max(query.from.unwrap_or(NaiveDate::MIN));
            let last = local_time(end - 1)?
                .date_naive()
                .min(query.to.unwrap_or(NaiveDate::MAX));
            let end = last
                .succ_opt()
                .context("archive range end overflow")?
                .min(cutoff.parse()?);
            (first < end).then(|| ArchiveKey {
                first,
                end,
                session: query.session_id.clone(),
            })
        } else {
            None
        };
        let mut archive_status = None;
        let mut recovered = None;
        let mut refresh_archive = false;
        if let (Some(key), Some((cached, answer))) = (&archive_key, archived)
            && cached == key
        {
            match answer {
                Ok(answer) => match answer.is_current(&read, current_revision, key) {
                    Ok(true) => {
                        let evidence = match answer.evidence.as_ref() {
                            Some(Ok(evidence)) => match evidence.is_current() {
                                Ok(true) => Some(evidence),
                                Ok(false) => {
                                    refresh_archive = true;
                                    None
                                }
                                Err(error) => {
                                    historical_error = Some(format!("{error:#}"));
                                    None
                                }
                            },
                            Some(Err(error)) => {
                                historical_error = Some(error.clone());
                                None
                            }
                            None => None,
                        };
                        recovered = Some((key, answer, evidence));
                    }
                    Ok(false) => {}
                    Err(error) => {
                        archive_status =
                            Some(json!({"status":"partial","message":error.to_string()}))
                    }
                },
                Err(message) => {
                    archive_status = Some(json!({"status":"partial","message":message}))
                }
            }
        }
        let scope = Self::scope(&read, provider_names)?;
        let scoped = requested.map(|session| scope.scope_members(session));
        let (mut all, mut scoped_usage) = if let Some((_, sessions)) = &scoped {
            let [all, usage] = Self::select(
                &read,
                query,
                bounds,
                [None, Some(sessions)],
                (include_trend, include_fingerprints),
                recovered,
                historical,
            )?;
            (all, Some(usage))
        } else {
            let [all] = Self::select(
                &read,
                query,
                bounds,
                [None],
                (include_trend, include_fingerprints),
                recovered,
                historical,
            )?;
            (all, None)
        };
        let scan_complete = all.coverage.is_some_and(|coverage| {
            coverage.pending_sources == 0 && coverage.issues == 0 && !coverage.maintenance_blocked
        });
        let complete = scan_complete && all.unresolved == 0 && !all.needs_source_recovery;
        let updated_at: i64 =
            read.query_row("SELECT updated_at FROM token_revision", [], |row| {
                row.get(0)
            })?;
        let coverage = all.range_coverage(query, bounds, complete)?;
        let archive_evidence = recovered.and_then(|(_, _, evidence)| evidence);
        let retained_evidence = historical.map(|evidence| &evidence.overlay);
        let mut names = provider_names.clone();
        for (id, label) in [retained_evidence, archive_evidence]
            .into_iter()
            .flatten()
            .flat_map(|evidence| &evidence.provider_names)
        {
            let name = names.entry(id.clone()).or_default();
            if name.trim().is_empty() {
                name.clone_from(label);
            }
        }
        for selection in std::iter::once(&mut all).chain(scoped_usage.iter_mut()) {
            for day in &mut selection.days {
                for row in day["breakdowns"].as_array_mut().unwrap() {
                    let name = names.get(row["provider"].as_str().unwrap());
                    row["providerName"] = json!(name);
                }
            }
        }
        let mut result = json!({
            "status": if complete {"ok"} else {"partial"},
            "complete":complete,
            "source":"persistent-local-rollout",
            "accountingVersion":2,
            "revision":all.revision,
            "updatedAt":updated_at,
            "rangeApplied":bounds.is_some(),
            "coverage":coverage,
            "countStatus":{"complete":complete,"uncertainEvents":all.unresolved},
            "classificationStatus":{"complete":complete && all.unclassified == 0 && !all.archive_classification_gap,"uncertainEvents":all.unclassified},
            "days":all.days,
            "sessionIds":scope.sessions,
            "sessionAliases":scope.aliases,
            "sessionInfo":scope.session_info,
            "providerNames":names,
            "scan":{
                "ready":all.coverage.is_some(),
                "pendingFiles":all.coverage.map(|c| c.pending_sources),
                "sourceIssues":all.coverage.map(|c| c.issues),
                "storageBlocked":all.coverage.is_some_and(|c| c.maintenance_blocked),
                "uncertainCountEvents":all.unresolved,
                "retentionDays":codex_plus_core::settings::default_usage_history_precise_days(),
                "preciseFrom":(!cutoff.is_empty()).then_some(&cutoff),
            }
        });
        if include_trend {
            result["trend"] = json!(all.trend);
        }
        if retained_evidence.is_some() || archive_evidence.is_some() {
            result["classificationRevision"] = json!([
                retained_evidence.map(|evidence| evidence.updated_at),
                archive_evidence.map(|evidence| evidence.updated_at)
            ]);
        }
        if let Some(evidence) = retained_evidence {
            result["providerEvidence"] = evidence.legacy_status.clone();
            result["supplierEvidence"] = evidence.supplier_status.clone();
        }
        if let Some(error) = historical_error {
            result["historicalClassificationError"] = json!(error);
        }
        let has_recovery = recovered.is_some();
        let old_activity = archive_key.is_some();
        let schedule = if let Some(key) = archive_key {
            if has_recovery && !refresh_archive {
                result["archiveRecovery"] =
                    json!({"status":if complete {"complete"} else {"partial"}});
                None
            } else if let Some(status) = archive_status {
                result["archiveRecovery"] = status;
                None
            } else {
                result["archiveRecovery"] = json!({"status":"pending"});
                all.coverage
                    .is_some_and(|coverage| coverage.pending_sources == 0)
                    .then_some(key)
            }
        } else {
            if all.needs_source_recovery && include_trend {
                result["archiveRecovery"] = json!({"status":"unavailable"});
            }
            None
        };
        if let Some(cursors) = all.session_cursors {
            result["sessionCursors"] = json!(cursors);
        }
        if let (Some((session, sessions)), Some(usage)) = (scoped, scoped_usage) {
            let count_complete =
                scan_complete && usage.unresolved == 0 && !usage.needs_source_recovery;
            let coverage = usage.range_coverage(query, bounds, count_complete)?;
            result["scopedUsage"] = json!({
                "sessionId":session,
                "sessionIds":ordered_scope_ids(session, &sessions),
                "days":usage.days,
                "coverage":coverage,
                "complete":count_complete,
                "countStatus":{"complete":count_complete,"uncertainEvents":usage.unresolved},
                "classificationStatus":{"complete":count_complete && usage.unclassified == 0 && !usage.archive_classification_gap,"uncertainEvents":usage.unclassified},
            });
            if include_trend {
                result["scopedUsage"]["trend"] = json!(usage.trend);
            }
        }
        if query.include_activity {
            Self::add_activity(
                &read,
                query,
                bounds,
                activity_page,
                request["activityRevision"].as_i64() == Some(updated_at),
                &mut result,
            )?;
            if old_activity || all.needs_source_recovery {
                result["activityComplete"] = json!(false);
            }
        }
        read.commit()?;
        Ok((result, schedule))
    }

    fn add_activity(
        read: &Connection,
        query: &Query,
        bounds: Option<(i64, i64)>,
        page: usize,
        unchanged: bool,
        result: &mut Value,
    ) -> Result<()> {
        const PAGE_SIZE: usize = 512;
        let (start, end) = bounds.unwrap_or((i64::MIN, i64::MAX));
        let from = query.from.map(|day| day.to_string());
        let to = query.to.map(|day| day.to_string());
        let filter = "FROM token_activity a JOIN token_sources source ON source.id = a.source_id
            WHERE a.timestamp_ms >= ?1 AND a.timestamp_ms < ?2 AND a.day >= (SELECT precise_from FROM token_revision)
            AND (?3 IS NULL OR a.day >= ?3) AND (?4 IS NULL OR a.day <= ?4)
            AND (?5 IS NULL OR a.session_id = (SELECT id FROM token_sessions WHERE session = ?5))
            AND NOT EXISTS(
                SELECT 1 FROM token_activity copy JOIN token_sources other ON other.id = copy.source_id
                WHERE copy.session_id = a.session_id AND copy.call_id = a.call_id AND other.source > source.source
                AND copy.day >= (SELECT precise_from FROM token_revision)
            )";
        let args = params![start, end, from, to, query.session_id];
        let total: usize =
            read.query_row(&format!("SELECT count(*) {filter}"), args, |row| row.get(0))?;
        let visible = total.min(MAX_ACTIVITY);
        let offset = (page - 1) * PAGE_SIZE;
        let mut records = Vec::<Value>::new();
        if !unchanged && offset < visible {
            let mut statement = read.prepare_cached(&format!(
                "SELECT a.payload {filter} ORDER BY a.timestamp_ms, a.call_id, a.session_id LIMIT ?6 OFFSET ?7"
            ))?;
            let mut rows = statement.query(params![
                start,
                end,
                from,
                to,
                query.session_id,
                PAGE_SIZE,
                total - visible + offset
            ])?;
            while let Some(row) = rows.next()? {
                records.push(serde_json::from_slice(row.get_ref(0)?.as_blob()?)?);
            }
        }
        let truncated = total > MAX_ACTIVITY
            || read.query_row(
                "SELECT EXISTS (SELECT 1 FROM token_sources WHERE activity_truncated)",
                [],
                |row| row.get::<_, bool>(0),
            )?;
        result["activityRecords"] = json!(records);
        result["activityPagination"] = json!({
            "page":page,"pages":visible.div_ceil(PAGE_SIZE).max(1),"total":visible,"unchanged":unchanged,
        });
        result["activityTruncated"] = json!(truncated);
        result["activityComplete"] = json!(result["complete"] == true && !truncated);
        Ok(())
    }

    pub(super) fn scope(
        read: &Connection,
        provider_names: &BTreeMap<String, String>,
    ) -> Result<Snapshot> {
        #[derive(Deserialize)]
        struct ScopeCheckpoint {
            session: String,
            identity_conflict: bool,
            scope_parent_candidates: BTreeMap<String, HashSet<Option<String>>>,
            model: String,
            provider: String,
            role: String,
        }

        let mut scope = Snapshot::default();
        let mut sources =
            read.prepare("SELECT source,checkpoint FROM token_sources ORDER BY source")?;
        let mut rows = sources.query([])?;
        let mut candidates = Vec::new();
        while let Some(row) = rows.next()? {
            let mut saved: ScopeCheckpoint = serde_json::from_slice(row.get_ref(1)?.as_blob()?)?;
            if saved.identity_conflict || saved.session.is_empty() {
                continue;
            }
            if let Some(rollout) = segments::rollout_id(&decode_path(row.get(0)?)?) {
                let alias = rollout.to_string();
                scope
                    .aliases
                    .entry(alias.clone())
                    .and_modify(|session| {
                        if *session != saved.session {
                            session.clone_from(&alias);
                        }
                    })
                    .or_insert_with(|| saved.session.clone());
            }
            if let Some(parents) = saved.scope_parent_candidates.remove(&saved.session) {
                candidates.push((saved.session.clone(), parents));
            }
            let info = scope
                .session_info
                .entry(saved.session.clone())
                .or_insert(Value::Null);
            if !saved.model.is_empty() || info["model"].as_str().is_none_or(str::is_empty) {
                *info = json!({
                    "model":saved.model,
                    "provider":usage_provider(&saved.model, &saved.provider, provider_names),
                    "role":saved.role,
                });
            }
            scope.sessions.insert(saved.session);
        }
        for session in &scope.sessions {
            scope.aliases.insert(session.clone(), session.clone());
        }
        scope.collect_scope_candidates(
            candidates
                .iter()
                .map(|(session, parents)| (session.as_str(), parents)),
        );
        Ok(scope)
    }

    fn select<const N: usize>(
        read: &Connection,
        query: &Query,
        bounds: Option<(i64, i64)>,
        scopes: [Option<&HashSet<String>>; N],
        (include_trend, include_fingerprints): (bool, bool),
        recovered: Option<(&ArchiveKey, &Recovered, Option<&EvidenceOverlay>)>,
        historical: Option<&Historical>,
    ) -> Result<[Selection; N]> {
        let (revision, precise_from): (i64, String) = read.query_row(
            "SELECT revision, precise_from FROM token_revision",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        // 发布一批后若扫描进程中断，不能沿用上一版本的完整标记。
        let coverage = read
            .query_row(
                "SELECT pending_sources, issues, maintenance_blocked FROM token_coverage WHERE revision = ?1",
                [revision],
                |row| {
                    Ok(Coverage {
                        pending_sources: row.get(0)?,
                        issues: row.get(1)?,
                        maintenance_blocked: row.get(2)?,
                    })
                },
            )
            .optional()?;
        let mut cursor_totals = BTreeMap::<String, (String, Counter)>::new();
        if query.include_session_cursors {
            let mut statement = read.prepare_cached(
                "SELECT c.key, c.value FROM token_sources s,
                 json_each(CAST(s.checkpoint AS TEXT), '$.committed_cursors') c
                 WHERE json_extract(CAST(s.checkpoint AS TEXT), '$.identity_conflict') = 0
                 AND (?1 IS NULL OR c.key = ?1) ORDER BY s.source",
            )?;
            let mut rows = statement.query([query.session_id.as_deref()])?;
            while let Some(row) = rows.next()? {
                let (at, total): (String, Counter) =
                    serde_json::from_str(row.get_ref(1)?.as_str()?)?;
                let date = DateTime::parse_from_rfc3339(&at)?
                    .with_timezone(&Local)
                    .date_naive();
                if date.to_string().as_str() < precise_from.as_str() {
                    continue;
                }
                let cursor = cursor_totals
                    .entry(row.get(0)?)
                    .or_insert_with(|| (at.clone(), total));
                if at >= cursor.0 {
                    *cursor = (at, total);
                }
            }
        }
        let usage: RefCell<[_; N]> = RefCell::new(std::array::from_fn(|_| {
            BTreeMap::<NaiveDate, Groups>::new()
        }));
        let fingerprints: RefCell<[_; N]> = RefCell::new(std::array::from_fn(|_| {
            BTreeMap::<NaiveDate, DayFingerprints>::new()
        }));
        let trend_bounds = bounds
            .filter(|_| include_trend)
            .map(|(start, end)| {
                let span = end
                    .checked_sub(start)
                    .filter(|span| *span > 0)
                    .context("invalid Token trend range")?;
                let minimum = (span - 1) / 120 + 1;
                let interval = [30, 60, 180, 360, 720, 1440, 2880, 10080, 43200]
                    .map(|minutes| minutes * 60_000)
                    .into_iter()
                    .find(|interval| *interval >= minimum)
                    .unwrap_or(minimum);
                Ok::<_, anyhow::Error>((start, end, interval, ((span - 1) / interval + 1) as usize))
            })
            .transpose()?;
        let trend: RefCell<[_; N]> = RefCell::new(std::array::from_fn(|_| {
            trend_bounds.map_or_else(Vec::new, |(_, _, _, count)| vec![Groups::new(); count])
        }));
        let accumulate = |session: &str, day: NaiveDate, key: &GroupKey, totals| {
            for (groups, scope) in usage.borrow_mut().iter_mut().zip(scopes) {
                if scope.is_none_or(|scope| scope.contains(session)) {
                    groups
                        .entry(day)
                        .or_default()
                        .entry(key.clone())
                        .or_default()
                        .merge(totals);
                }
            }
        };
        let accumulate_summary = |session: &str, day: NaiveDate, groups: &[DayGroup]| {
            let mut contributed = false;
            for group in groups {
                let model = Some(group.model.as_str()).filter(|model| !model.is_empty());
                if query.accepts_fields(session, "", model, Some(day)) {
                    accumulate(
                        session,
                        day,
                        &(
                            group.model.clone(),
                            group.provider.clone(),
                            group.role.clone(),
                            group.conflicts,
                        ),
                        group.totals,
                    );
                    contributed |= group.totals.records != 0;
                }
            }
            contributed
        };
        let mark_summary = |session: &str, day: NaiveDate| {
            if include_fingerprints {
                for (days, scope) in fingerprints.borrow_mut().iter_mut().zip(scopes) {
                    if scope.is_none_or(|scope| scope.contains(session)) {
                        days.entry(day).or_default().summary_only = true;
                    }
                }
            }
        };
        let read_summary = |session: String, day: NaiveDate, groups: Vec<DayGroup>| {
            if recovered.is_some_and(|(key, _, _)| {
                day >= key.first
                    && day < key.end
                    && key.session.as_ref().is_none_or(|id| id == &session)
            }) {
                // 先比较回读与原汇总，再决定是否需要重新分组。
                return Ok(());
            }
            let groups = historical
                .and_then(|evidence| evidence.days.get(&day)?.get(&session))
                .unwrap_or(&groups);
            if accumulate_summary(&session, day, groups) {
                mark_summary(&session, day);
            }
            Ok(())
        };
        let read_event = |event: &UsageEvent, include_totals: bool, classification_agrees: bool| {
            let model = Some(event.model.as_str()).filter(|model| !model.is_empty());
            if query.accepts_fields(&event.session, &event.turn, model, Some(event.date)) {
                let mut key = group_key(event);
                let mut totals = DayTotals::from_event(event);
                if !classification_agrees {
                    key.0 = "unknown".into();
                    key.1 = "unknown".into();
                    key.3 |= 3;
                    totals.evidence = ClassificationEvidence::None;
                }
                if let Some((start, _, interval, _)) = trend_bounds {
                    let at = DateTime::parse_from_rfc3339(&event.timestamp)?.timestamp_millis();
                    let bucket = ((at - start) / interval) as usize;
                    for (buckets, scope) in trend.borrow_mut().iter_mut().zip(scopes) {
                        if scope.is_none_or(|scope| scope.contains(&event.session)) {
                            buckets[bucket]
                                .entry(key.clone())
                                .or_default()
                                .merge(totals);
                        }
                    }
                }
                if include_totals {
                    accumulate(&event.session, event.date, &key, totals);
                }
                if include_fingerprints && classification_agrees && totals.records != 0 {
                    for (days, scope) in fingerprints.borrow_mut().iter_mut().zip(scopes) {
                        if scope.is_none_or(|scope| scope.contains(&event.session)) {
                            let day = days.entry(event.date).or_default();
                            if !day.summary_only {
                                event.update_membership_digest(&mut day.accounting);
                                event.update_membership_digest(
                                    day.membership.entry(key.clone()).or_default(),
                                );
                            }
                        }
                    }
                }
            }
            Ok(())
        };
        let read_retained = |mut event: UsageEvent| {
            if let Some(evidence) = historical {
                evidence.overlay.apply(&mut event);
            }
            read_event(&event, true, true)
        };
        let whole_days = if !include_trend
            && !include_fingerprints
            && query.turn_id.is_none()
            && let Some((start, end)) = bounds
        {
            ensure!(start < end, "invalid Token range");
            let start = local_time(start)?;
            let end = local_time(end)?;
            (local_midnight(start) && local_midnight(end)).then_some((
                start.date_naive().max(query.from.unwrap_or(NaiveDate::MIN)),
                end.date_naive().min(
                    query
                        .to
                        .and_then(|day| day.succ_opt())
                        .unwrap_or(NaiveDate::MAX),
                ),
            ))
        } else {
            None
        };
        let needs_source_recovery = if let Some((start, end)) = whole_days {
            // 周/月视图按完整本地日取汇总，无需反序列化这些日期的逐笔明细。
            Self::visit_days(read, start, end, read_summary)?;
            start < end && start.to_string().as_str() < precise_from.as_str()
        } else if let Some((start, end)) = bounds {
            Self::visit_range(
                read,
                (start, end),
                Some(query),
                include_fingerprints,
                read_retained,
                // 按日摘要没有回合信息，不能拿它回答某一回合的用量。
                query.turn_id.is_none().then_some(read_summary),
            )?
        } else if query.turn_id.is_some() || include_fingerprints {
            Self::visit_committed(
                read,
                i64::MIN,
                i64::MAX,
                Some(query),
                include_fingerprints,
                read_retained,
            )?;
            if precise_from.is_empty() {
                false
            } else {
                let cutoff: NaiveDate = precise_from.parse()?;
                if query.turn_id.is_none() {
                    let end = query
                        .to
                        .and_then(|day| day.succ_opt())
                        .map_or(cutoff, |end| end.min(cutoff));
                    Self::visit_summary(read, query.from, Some(end), read_summary)?;
                    query
                        .from
                        .zip(query.to)
                        .is_some_and(|(from, _)| from < cutoff)
                } else {
                    query.from.is_none_or(|from| from < cutoff)
                }
            }
        } else {
            Self::visit_summary(
                read,
                query.from,
                query.to.and_then(|day| day.succ_opt()),
                read_summary,
            )?;
            query
                .from
                .zip(query.to)
                .is_some_and(|(from, _)| from.to_string().as_str() < precise_from.as_str())
        };
        let mut blocked_sessions = HashSet::new();
        let mut classification_gaps = HashSet::new();
        if let Some((key, archive, evidence)) = recovered
            && let Some((start, end)) = bounds
        {
            let mut observed = BTreeMap::<_, Groups>::new();
            let mut affected = HashSet::new();
            for event in &archive.events {
                observed
                    .entry((event.session.as_str(), event.date))
                    .or_default()
                    .entry(group_key(event))
                    .or_default()
                    .merge(DayTotals::from_event(event));
                if evidence.is_some_and(|evidence| evidence.contains(event)) {
                    affected.insert((event.session.as_str(), event.date));
                }
            }
            let start_local = local_time(start)?;
            let whole_start = start_local.date_naive()
                + chrono::Days::new(u64::from(!local_midnight(start_local)));
            let whole_end = local_time(end)?.date_naive();
            let mut preserved = HashSet::new();
            let mut blocked = HashSet::new();
            let mut disagrees = HashSet::new();
            Self::visit_days(read, key.first, key.end, |session, date, summary| {
                if key.session.as_ref().is_some_and(|id| id != &session) {
                    return Ok(());
                }
                let mut expected = DayTotals::default();
                let mut actual = DayTotals::default();
                let groups = observed.get(&(session.as_str(), date));
                for group in &summary {
                    expected.merge(group.totals);
                }
                for totals in groups.into_iter().flat_map(BTreeMap::values) {
                    actual.merge(*totals);
                }
                let counts_agree = expected.records == actual.records
                    && expected.counts.0 == actual.counts.0
                    && expected.unresolved == actual.unresolved
                    && expected.priceable == actual.priceable;
                let groups_agree = groups.is_none_or(|groups| {
                    summary.len() == groups.len()
                        && summary.iter().all(|group| {
                            groups
                                .get(&(
                                    group.model.clone(),
                                    group.provider.clone(),
                                    group.role.clone(),
                                    group.conflicts,
                                ))
                                .is_some_and(|totals| {
                                    totals.records == group.totals.records
                                        && totals.counts.0 == group.totals.counts.0
                                        && totals.priceable == group.totals.priceable
                                        && totals.evidence == group.totals.evidence
                                })
                        })
                });
                if date >= whole_start
                    && date < whole_end
                    && query.turn_id.is_none()
                    && (!counts_agree
                        || !groups_agree
                        || !affected.contains(&(session.as_str(), date)))
                {
                    accumulate_summary(&session, date, &summary);
                    preserved.insert((session.clone(), date));
                }
                if !counts_agree {
                    mark_summary(&session, date);
                    blocked.insert((session, date));
                } else if !groups_agree {
                    mark_summary(&session, date);
                    if query.model.is_some() || !query.search.is_empty() {
                        blocked.insert((session, date));
                    } else {
                        disagrees.insert((session, date));
                    }
                }
                Ok(())
            })?;
            // Archive replay returns original observations, not hashes of day totals.
            let mut archive_events: Vec<_> = archive.events.iter().collect();
            if include_fingerprints {
                archive_events.sort_unstable_by_key(|event| event.id);
            }
            for event in archive_events {
                let at = DateTime::parse_from_rfc3339(&event.timestamp)?.timestamp_millis();
                let session_day = (event.session.clone(), event.date);
                if at >= start && at < end && !blocked.contains(&session_day) {
                    // 已提交的日归类不证明每个分时桶的归类；只接受可验证的计数。
                    let agrees = !disagrees.contains(&session_day);
                    if agrees
                        && let Some(evidence) = evidence.filter(|evidence| evidence.contains(event))
                    {
                        let mut classified = event.clone();
                        evidence.apply(&mut classified);
                        read_event(&classified, !preserved.contains(&session_day), true)?;
                        continue;
                    }
                    read_event(event, !preserved.contains(&session_day), agrees)?;
                }
            }
            blocked_sessions.extend(blocked.into_iter().map(|(session, _)| session));
            classification_gaps.extend(disagrees.into_iter().map(|(session, _)| session));
        }
        let mut scopes = scopes.into_iter();
        let mut trends = trend.into_inner().into_iter();
        let mut proofs = fingerprints.into_inner().into_iter();
        Ok(usage.into_inner().map(|groups| {
            let scope = scopes.next().unwrap();
            let buckets = trends.next().unwrap();
            let affects = |sessions: &HashSet<String>| !sessions.is_empty()
                && scope.is_none_or(|scope| !scope.is_disjoint(sessions));
            let needs_source_recovery = if recovered.is_some() {
                affects(&blocked_sessions)
            } else {
                needs_source_recovery
            };
            let mut selection = Self::format_selection(
                revision,
                coverage,
                &precise_from,
                needs_source_recovery,
                groups,
                include_fingerprints.then(|| proofs.next().unwrap()),
            );
            selection.recovered_archive = recovered.is_some();
            selection.archive_classification_gap = affects(&classification_gaps);
            if let Some((start, end, interval, _)) = trend_bounds.filter(|_| !needs_source_recovery) {
                let buckets: Vec<_> = buckets.into_iter().enumerate().map(|(i, groups)| {
                    let at = start + i as i64 * interval;
                    let (mut row, _) = Self::format_day(groups, None);
                    row["startAt"] = json!(at);
                    row["endAt"] = json!(at.saturating_add(interval).min(end));
                    row
                }).collect();
                selection.trend = Some(json!({
                    "startAt":start,"endAt":end,"intervalMs":interval,
                    "precision":if selection.unresolved > 0 || coverage.is_none_or(|c| c.issues > 0 || c.pending_sources > 0 || c.maintenance_blocked) {"partial"} else {"exact"},
                    "buckets":buckets,
                }));
            }
            if query.include_session_cursors {
                selection.session_cursors = Some(
                    cursor_totals
                        .iter()
                        .filter(|(session, _)| scope.is_none_or(|ids| ids.contains(*session)))
                        .map(|(session, (at, total))| {
                            json!({
                                "sessionId":session, "timestamp":at, "total":total.0.value()
                            })
                        })
                        .collect(),
                );
            }
            selection
        }))
    }

    fn format_selection(
        revision: i64,
        coverage: Option<Coverage>,
        precise_from: &str,
        needs_source_recovery: bool,
        usage: BTreeMap<NaiveDate, Groups>,
        mut fingerprints: Option<BTreeMap<NaiveDate, DayFingerprints>>,
    ) -> Selection {
        let mut unresolved = 0;
        let mut unclassified = 0;
        let mut days = Vec::new();
        let archived = usage
            .keys()
            .next()
            .is_some_and(|date| date.to_string().as_str() < precise_from);
        for (date, groups) in usage {
            unclassified += groups
                .iter()
                .filter(|((model, provider, _, flags), _)| {
                    *flags & 3 != 0
                        || unknown_model(model)
                        || unknown_provider(provider)
                        || codex_plus_core::aggregate_catalog::is_aggregate_model(model)
                })
                .map(|(_, group)| group.records)
                .sum::<u64>();
            let proof = fingerprints
                .as_mut()
                .and_then(|days| days.remove(&date))
                .filter(|proof| !proof.summary_only && !proof.membership.is_empty());
            let (mut day, uncertain) = Self::format_day(groups, proof);
            if fingerprints.is_some() {
                day["fingerprintCoverage"] = json!(if day.get("accountingFingerprint").is_some() {
                    "events"
                } else {
                    "unavailable"
                });
            }
            unresolved += uncertain;
            day["date"] = json!(date.to_string());
            days.push(day);
        }
        Selection {
            revision,
            coverage,
            days,
            unresolved,
            unclassified,
            archived,
            needs_source_recovery,
            session_cursors: None,
            trend: None,
            precise_from: precise_from.to_owned(),
            recovered_archive: false,
            archive_classification_gap: false,
        }
    }

    fn format_day(groups: Groups, mut fingerprints: Option<DayFingerprints>) -> (Value, u64) {
        let mut unresolved = 0;
        let mut totals = DayTotals::default();
        let mut models = BTreeMap::<String, DayTotals>::new();
        let mut breakdowns = Vec::new();
        for (key, group) in groups {
            let (model, provider, role, conflicts) = &key;
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
            if let Some(digest) = fingerprints
                .as_mut()
                .and_then(|proof| proof.membership.remove(&key))
            {
                row["membershipFingerprint"] = json!(format!("{:x}", digest.finalize()));
            }
            group.evidence.write_to(&mut row);
            breakdowns.push(row);
            models.entry(key.0).or_default().merge(group);
            totals.merge(group);
        }
        let mut day = totals.value();
        if let Some(proof) = fingerprints {
            day["accountingFingerprint"] = json!(format!("{:x}", proof.accounting.finalize()));
        }
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
        (day, unresolved)
    }
}
