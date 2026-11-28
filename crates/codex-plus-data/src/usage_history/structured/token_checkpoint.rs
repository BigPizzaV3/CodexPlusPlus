use super::{
    CacheAccounting, Call, ContextConflicts, ExecSkill, FileState, InheritedUsageContext,
    LegacyIdentity, NativeResponseIdentity, SessionModelObservation, TokenDaySpan, Tokens,
    UsageEvent, segments::HistoryBase,
};
use crate::usage_history::UsageHistoryBoundary;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::SystemTime;

const RECENT_CLASSIFICATION_MS: i64 = 86_400_000;

fn disk_full(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<rusqlite::Error>()
        .is_some_and(|error| error.sqlite_error_code() == Some(rusqlite::ErrorCode::DiskFull))
}

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

#[derive(Clone, Copy, Default, Serialize, Deserialize)]
#[serde(transparent)]
struct Counter(#[serde(with = "StoredTokens")] Tokens);

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
struct EventClassification {
    model: String,
    provider: String,
    provider_source: Option<String>,
    provider_conflict: bool,
    #[serde(default)]
    count_uncertain: bool,
}

impl EventClassification {
    fn of(event: &UsageEvent) -> Self {
        Self {
            model: event.model.clone(),
            provider: event.provider.clone(),
            provider_source: event.provider_source.clone(),
            provider_conflict: event.context_conflicts.provider,
            count_uncertain: event.count_uncertain,
        }
    }

    fn apply(&self, event: &mut UsageEvent) {
        event.model.clone_from(&self.model);
        event.provider.clone_from(&self.provider);
        event.provider_source.clone_from(&self.provider_source);
        event.context_conflicts.provider = self.provider_conflict;
        event.count_uncertain = self.count_uncertain;
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Observation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    terminal_response: Option<Box<str>>,
    original_id: [u8; 32],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    native_response_id: Option<NativeResponseIdentity>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    provider_source: Option<String>,
    role: String,
    inherited: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    legacy_identity: Option<Box<LegacyIdentity>>,
    context_conflicts: ContextConflicts,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    classification_origin: Option<EventClassification>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    receipt_count_origin: Option<Counter>,
}

impl Observation {
    fn capture(event: &UsageEvent) -> Self {
        Self {
            terminal_response: event.terminal_response.clone(),
            original_id: event.original_id,
            native_response_id: event.native_response_id.clone(),
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
            classification_origin: None,
            receipt_count_origin: None,
        }
    }

    fn into_event(self, id: [u8; 32], session: String) -> anyhow::Result<UsageEvent> {
        Ok(UsageEvent {
            terminal_response: self.terminal_response,
            id,
            original_id: self.original_id,
            native_response_id: self.native_response_id,
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
    #[serde(default)]
    malformed_token_counts: usize,
    identity_conflict: bool,
    continuation: bool,
    history_base: Option<([u8; 16], u64, u64)>,
    rollout_extent: Option<(u64, u64, u64)>,
    rollout_out_of_order: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    nul_padding_end: Option<u64>,
    expired_count: Option<((u64, u64, u64), Counter)>,
    #[serde(default)]
    token_day_span: Option<TokenDaySpan>,
    provider: String,
    role: String,
    history_boundary: UsageHistoryBoundary,
    inherited_contexts: Vec<InheritedUsageContext>,
    model_observations: HashMap<String, SessionModelObservation>,
    legacy_header: Option<(String, String)>,
    scope_parent_candidates: BTreeMap<String, HashSet<Option<String>>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    committed_cursors: BTreeMap<String, (String, Counter)>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    calls: HashMap<String, Call>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    exec_skills: HashMap<String, ExecSkill>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    exec_polls: HashMap<String, String>,
    #[serde(default)]
    activity_truncated: bool,
    issues: usize,
    skipping: bool,
    #[serde(default)]
    large_meter_checked: bool,
    // 兼容曾按解析器版本触发全量回读的检查点；沿用当前按实际缺口回读的规则。
    #[serde(default, rename = "receipt_parser_version", skip_serializing)]
    _receipt_parser_version: Option<u32>,
    #[serde(default)]
    legacy_identity_prefix: Option<u64>,
    #[serde(default)]
    large_tools_checked: bool,
}

fn recheck_prefix(offset: u64, identity_prefix: Option<u64>, large_tools_checked: bool, activity_truncated: bool) -> u64 {
    if !large_tools_checked && activity_truncated && offset > super::MAX_LINE as u64 {
        offset
    } else {
        identity_prefix.unwrap_or(offset)
    }
}

impl Checkpoint {
    fn capture(state: &FileState) -> (u64, Self) {
        // 未处理的尾行留在原会话文件，重开后从该行起点重读。
        let offset = state.committed_offset();
        let mut committed_cursors = BTreeMap::new();
        for event in &state.events {
            let cursor = committed_cursors
                .entry(event.session.clone())
                .or_insert_with(|| (event.timestamp.clone(), Counter(event.total)));
            if event.timestamp >= cursor.0 {
                cursor.0.clone_from(&event.timestamp);
                cursor.1 = Counter(event.total);
            }
        }
        (
            offset,
            Self {
                stamp: state.stamp,
                resume_pending: state.resume_pending
                    || state.reader.is_some()
                    || offset < state.offset,
                session: state.session.clone(),
                model: state.model.clone(),
                model_unavailable: state.model_unavailable,
                turn: state.turn.clone(),
                previous: state.previous.map(Counter),
                previous_cache_accounting: state.previous_cache_accounting,
                invalid_counts: state.invalid_counts,
                malformed_token_counts: state.malformed_token_counts,
                identity_conflict: state.identity_conflict,
                continuation: state.continuation,
                history_base: state
                    .history_base
                    .as_ref()
                    .map(|base| (*base.rollout.as_bytes(), base.end_ordinal, base.end_offset)),
                rollout_extent: state.rollout_extent,
                rollout_out_of_order: state.rollout_out_of_order,
                nul_padding_end: state.nul_padding_end,
                expired_count: state
                    .expired_count
                    .map(|(position, count)| (position, Counter(count))),
                token_day_span: state.token_day_span,
                provider: state.provider.clone(),
                role: state.role.clone(),
                history_boundary: state.history_boundary.clone(),
                inherited_contexts: state.inherited_contexts.clone(),
                model_observations: state.model_observations.clone(),
                legacy_header: state.legacy_header.clone(),
                scope_parent_candidates: state.scope_parent_candidates.clone(),
                committed_cursors,
                calls: state.calls.clone(),
                exec_skills: state.exec_skills.clone(),
                exec_polls: state.exec_polls.clone(),
                activity_truncated: state.activity_truncated,
                issues: state.issues,
                skipping: state.skipping,
                large_meter_checked: !state.recheck_large_meter,
                _receipt_parser_version: None,
                legacy_identity_prefix: Some(if state.legacy_tools_only { 0 } else { state.legacy_identity_prefix }),
                large_tools_checked: !state.legacy_tools_only,
            },
        )
    }

    fn restore(self, offset: u64) -> FileState {
        let recheck_large_meter = !self.large_meter_checked
            && offset > super::MAX_LINE as u64
            && self.issues > self.malformed_token_counts;
        let prefix = recheck_prefix(offset, self.legacy_identity_prefix, self.large_tools_checked, self.activity_truncated);
        FileState {
            stamp: self.stamp,
            offset,
            resume_pending: self.resume_pending || recheck_large_meter,
            recheck_large_meter,
            legacy_identity_prefix: prefix,
            legacy_tools_only: prefix != 0 && self.legacy_identity_prefix == Some(0),
            session: self.session,
            model: self.model,
            model_unavailable: self.model_unavailable,
            turn: self.turn,
            previous: self.previous.map(|value| value.0),
            previous_cache_accounting: self.previous_cache_accounting,
            invalid_counts: self.invalid_counts,
            malformed_token_counts: self.malformed_token_counts,
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
            nul_padding_end: self.nul_padding_end,
            expired_count: self
                .expired_count
                .map(|(position, count)| (position, count.0)),
            token_day_span: self.token_day_span,
            provider: self.provider,
            role: self.role,
            history_boundary: self.history_boundary,
            inherited_contexts: self.inherited_contexts,
            model_observations: self.model_observations,
            legacy_header: self.legacy_header,
            scope_parent_candidates: self.scope_parent_candidates,
            calls: self.calls,
            exec_skills: self.exec_skills,
            exec_polls: self.exec_polls,
            activity_truncated: self.activity_truncated,
            issues: self.issues,
            skipping: self.skipping,
            ..FileState::default()
        }
    }
}

mod historical;
mod ingest;
#[cfg(test)]
mod projection_tests;
mod scan;
mod store;
#[cfg(test)]
mod tests;

mod runtime;
pub(super) use runtime::{ReadState, RefreshReaders, Runtime, read_execution_detail, recover_archive};
pub(super) use store::{
    archive::ArchiveKey,
    executions::{DetailQuery, ExecutionQuery},
    projection::ArchiveAnswer,
};
