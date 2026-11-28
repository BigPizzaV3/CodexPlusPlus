use super::{
    CacheAccounting, ContextConflicts, FileState, InheritedUsageContext, LegacyIdentity,
    SessionModelObservation, Tokens, UsageEvent, segments::HistoryBase,
};
use crate::usage_history::UsageHistoryBoundary;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::SystemTime;

// Tokens 的公开序列化不包含 cached_known，不能作为持久格式使用。
#[derive(Serialize, Deserialize)]
#[serde(remote = "Tokens")]
struct StoredTokens {
    #[serde(rename = "i")]
    input_tokens: u64,
    #[serde(rename = "c")]
    cached_input_tokens: u64,
    #[serde(rename = "o")]
    output_tokens: u64,
    #[serde(rename = "t")]
    total_tokens: u64,
    #[serde(rename = "r")]
    reasoning_output_tokens: Option<u64>,
    #[serde(rename = "k")]
    cached_known: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(transparent)]
struct Counter(#[serde(with = "StoredTokens")] Tokens);

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Observation {
    original_id: [u8; 32],
    turn: String,
    model: String,
    timestamp: String,
    date: String,
    total: Counter,
    delta: Counter,
    last: Option<Counter>,
    has_file_baseline: bool,
    count_uncertain: bool,
    count_incomplete: bool,
    count_pending: bool,
    rollout_position: Option<(u64, u64, u64)>,
    provider: String,
    provider_source: Option<String>,
    role: String,
    inherited: bool,
    legacy_identity: Option<Box<LegacyIdentity>>,
    context_conflicts: ContextConflicts,
}

impl Observation {
    fn capture(event: &UsageEvent) -> Self {
        Self {
            original_id: event.original_id,
            turn: event.turn.clone(),
            model: event.model.clone(),
            timestamp: event.timestamp.clone(),
            date: event.date.to_string(),
            total: Counter(event.total),
            delta: Counter(event.delta),
            last: event.last.map(Counter),
            has_file_baseline: event.has_file_baseline,
            count_uncertain: event.count_uncertain,
            count_incomplete: event.count_incomplete,
            count_pending: event.count_pending,
            rollout_position: event.rollout_position,
            provider: event.provider.clone(),
            provider_source: event.provider_source.clone(),
            role: event.role.clone(),
            inherited: event.inherited,
            legacy_identity: event.legacy_identity.clone(),
            context_conflicts: event.context_conflicts,
        }
    }

    fn into_event(self, id: [u8; 32], session: String) -> anyhow::Result<UsageEvent> {
        Ok(UsageEvent {
            id,
            original_id: self.original_id,
            session,
            turn: self.turn,
            model: self.model,
            timestamp: self.timestamp,
            date: NaiveDate::parse_from_str(&self.date, "%Y-%m-%d")?,
            total: self.total.0,
            delta: self.delta.0,
            last: self.last.map(|value| value.0),
            has_file_baseline: self.has_file_baseline,
            count_uncertain: self.count_uncertain,
            count_incomplete: self.count_incomplete,
            count_pending: self.count_pending,
            rollout_position: self.rollout_position,
            provider: self.provider,
            provider_source: self.provider_source,
            role: self.role,
            inherited: self.inherited,
            legacy_identity: self.legacy_identity,
            context_conflicts: self.context_conflicts,
        })
    }
}

type SourceStamp = (u64, Option<SystemTime>, Option<(u64, u64)>);

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Checkpoint {
    stamp: Option<SourceStamp>,
    resume_pending: bool,
    session: String,
    model: String,
    model_unavailable: bool,
    turn: String,
    previous: Option<Counter>,
    previous_cache_accounting: Option<CacheAccounting>,
    invalid_counts: bool,
    identity_conflict: bool,
    continuation: bool,
    history_base: Option<([u8; 16], u64, u64)>,
    rollout_extent: Option<(u64, u64, u64)>,
    rollout_out_of_order: bool,
    expired_count: Option<((u64, u64, u64), Counter)>,
    provider: String,
    role: String,
    history_boundary: UsageHistoryBoundary,
    inherited_contexts: Vec<InheritedUsageContext>,
    model_observations: HashMap<String, SessionModelObservation>,
    legacy_header: Option<(String, String)>,
    scope_parent_candidates: BTreeMap<String, HashSet<Option<String>>>,
    issues: usize,
    skipping: bool,
}

impl Checkpoint {
    fn capture(state: &FileState) -> (u64, Self) {
        // 未处理的尾行留在原会话文件，重开后从该行起点重读。
        let offset = state.offset - state.pending.len() as u64;
        (
            offset,
            Self {
                stamp: state.stamp,
                resume_pending: state.resume_pending
                    || state.reader.is_some()
                    || !state.pending.is_empty(),
                session: state.session.clone(),
                model: state.model.clone(),
                model_unavailable: state.model_unavailable,
                turn: state.turn.clone(),
                previous: state.previous.map(Counter),
                previous_cache_accounting: state.previous_cache_accounting,
                invalid_counts: state.invalid_counts,
                identity_conflict: state.identity_conflict,
                continuation: state.continuation,
                history_base: state
                    .history_base
                    .as_ref()
                    .map(|base| (*base.rollout.as_bytes(), base.end_ordinal, base.end_offset)),
                rollout_extent: state.rollout_extent,
                rollout_out_of_order: state.rollout_out_of_order,
                expired_count: state
                    .expired_count
                    .map(|(position, count)| (position, Counter(count))),
                provider: state.provider.clone(),
                role: state.role.clone(),
                history_boundary: state.history_boundary.clone(),
                inherited_contexts: state.inherited_contexts.clone(),
                model_observations: state.model_observations.clone(),
                legacy_header: state.legacy_header.clone(),
                scope_parent_candidates: state.scope_parent_candidates.clone(),
                issues: state.issues,
                skipping: state.skipping,
            },
        )
    }

    fn restore(self, offset: u64) -> FileState {
        FileState {
            stamp: self.stamp,
            offset,
            resume_pending: self.resume_pending,
            session: self.session,
            model: self.model,
            model_unavailable: self.model_unavailable,
            turn: self.turn,
            previous: self.previous.map(|value| value.0),
            previous_cache_accounting: self.previous_cache_accounting,
            invalid_counts: self.invalid_counts,
            identity_conflict: self.identity_conflict,
            continuation: self.continuation,
            history_base: self
                .history_base
                .map(|(rollout, end_ordinal, end_offset)| HistoryBase {
                    rollout: uuid::Uuid::from_bytes(rollout),
                    end_ordinal,
                    end_offset,
                }),
            rollout_extent: self.rollout_extent,
            rollout_out_of_order: self.rollout_out_of_order,
            expired_count: self
                .expired_count
                .map(|(position, count)| (position, count.0)),
            provider: self.provider,
            role: self.role,
            history_boundary: self.history_boundary,
            inherited_contexts: self.inherited_contexts,
            model_observations: self.model_observations,
            legacy_header: self.legacy_header,
            scope_parent_candidates: self.scope_parent_candidates,
            issues: self.issues,
            skipping: self.skipping,
            ..FileState::default()
        }
    }
}

mod store;
mod tests;
