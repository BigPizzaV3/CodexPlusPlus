//! 持久游标所需的解析上下文；已提交事件和工具记录不保留在这里。
use super::super::{
    ContextConflicts, FileState, InheritedUsageContext, LegacyIdentity, SessionModelObservation,
    Tokens, UsageEvent, index_budget,
};
use anyhow::{Context, ensure};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::SystemTime;

#[derive(Serialize, Deserialize)]
struct SavedTokens {
    #[serde(rename = "i")]
    input: u64,
    #[serde(rename = "c")]
    cached: u64,
    #[serde(rename = "o")]
    output: u64,
    #[serde(rename = "t")]
    total: u64,
    #[serde(rename = "r")]
    reasoning: Option<u64>,
    #[serde(rename = "k")]
    cached_known: bool,
}

impl From<Tokens> for SavedTokens {
    fn from(tokens: Tokens) -> Self {
        Self {
            input: tokens.input_tokens,
            cached: tokens.cached_input_tokens,
            output: tokens.output_tokens,
            total: tokens.total_tokens,
            reasoning: tokens.reasoning_output_tokens,
            cached_known: tokens.cached_known,
        }
    }
}

impl From<SavedTokens> for Tokens {
    fn from(tokens: SavedTokens) -> Self {
        Self {
            input_tokens: tokens.input,
            cached_input_tokens: tokens.cached,
            output_tokens: tokens.output,
            total_tokens: tokens.total,
            reasoning_output_tokens: tokens.reasoning,
            cached_known: tokens.cached_known,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct SavedEvent {
    #[serde(rename = "t", alias = "turn")]
    turn: String,
    #[serde(rename = "m", alias = "model")]
    model: String,
    #[serde(rename = "a", alias = "timestamp")]
    timestamp: String,
    #[serde(rename = "d", alias = "day")]
    day: String,
    #[serde(rename = "u", alias = "total")]
    total: SavedTokens,
    #[serde(rename = "v", alias = "delta")]
    delta: SavedTokens,
    #[serde(rename = "b", alias = "has_file_baseline")]
    has_file_baseline: bool,
    #[serde(rename = "l", alias = "last")]
    last: Option<SavedTokens>,
    #[serde(rename = "q", alias = "count_uncertain")]
    count_uncertain: bool,
    #[serde(rename = "c", alias = "count_incomplete")]
    count_incomplete: bool,
    #[serde(rename = "p", alias = "provider")]
    provider: String,
    #[serde(rename = "s", alias = "provider_source")]
    provider_source: Option<String>,
    #[serde(rename = "r", alias = "role")]
    role: String,
    #[serde(rename = "h", alias = "inherited")]
    inherited: bool,
    #[serde(rename = "i", alias = "legacy_identity")]
    legacy_identity: Option<([u8; 32], String, String)>,
    #[serde(rename = "x", alias = "context_conflicts")]
    context_conflicts: (bool, bool, bool, bool),
}

pub(super) type PayloadCursor = (Vec<u8>, i64);

const COMPRESSED_OBSERVATION: u8 = 0;
const OBSERVATION_HEADER_BYTES: usize = 5;
const COMPACTION_BATCH_BYTES: usize = 1024 * 1024;

pub(super) fn observation_size(bytes: &[u8]) -> anyhow::Result<usize> {
    if bytes.first() != Some(&COMPRESSED_OBSERVATION) {
        return Ok(bytes.len());
    }
    let length = u32::from_le_bytes(
        bytes
            .get(1..OBSERVATION_HEADER_BYTES)
            .context("invalid compressed usage observation header")?
            .try_into()
            .expect("observation length has four bytes"),
    ) as usize;
    ensure!(
        length <= index_budget::MAX_RETAINED_BYTES,
        "compressed usage observation exceeds memory budget"
    );
    Ok(length)
}

fn compress_observation(bytes: Vec<u8>) -> anyhow::Result<Vec<u8>> {
    let length = u32::try_from(bytes.len()).context("usage observation is too large")?;
    let compressed = zstd::bulk::compress(&bytes, 1)?;
    if compressed.len() + OBSERVATION_HEADER_BYTES >= bytes.len() {
        return Ok(bytes);
    }
    let mut encoded = Vec::with_capacity(compressed.len() + OBSERVATION_HEADER_BYTES);
    encoded.push(COMPRESSED_OBSERVATION);
    encoded.extend_from_slice(&length.to_le_bytes());
    encoded.extend_from_slice(&compressed);
    Ok(encoded)
}

/// 每批只改编码，不改事件身份、计数或来源游标；旧键和新键在迁移期间可同时读取。
pub(super) fn compact_payloads(
    tx: &rusqlite::Transaction<'_>,
    cursor: Option<&PayloadCursor>,
) -> anyhow::Result<(Option<PayloadCursor>, usize)> {
    let started = std::time::Instant::now();
    // 超大旧记录保留原样，避免迁移时把整条记录复制到内存。
    let mut statement = tx.prepare_cached(
        "SELECT source, position,
                CASE WHEN length(payload) <= ?3 THEN payload END
         FROM usage_observations
         WHERE (source, position) > (?1, ?2) ORDER BY source, position LIMIT 1024",
    )?;
    let after_source = cursor.map_or(&[][..], |cursor| cursor.0.as_slice());
    let after_position = cursor.map_or(-1, |cursor| cursor.1);
    let mut rows = statement.query(rusqlite::params![
        after_source, after_position, COMPACTION_BATCH_BYTES as i64,
    ])?;
    let mut batch = Vec::new();
    let mut batch_bytes = 0;
    while let Some(row) = rows.next()? {
        let bytes = match row.get_ref(2)? {
            rusqlite::types::ValueRef::Null => None,
            value => Some(value.as_blob()?),
        };
        let length = bytes.map_or(0, <[u8]>::len);
        if !batch.is_empty()
            && (batch_bytes + length > COMPACTION_BATCH_BYTES
                || started.elapsed() >= std::time::Duration::from_millis(100))
        {
            break;
        }
        batch_bytes += length;
        batch.push((
            row.get::<_, Vec<u8>>(0)?,
            row.get::<_, i64>(1)?,
            bytes.map(<[u8]>::to_vec),
        ));
    }
    drop(rows);
    drop(statement);
    let mut next = None;
    let mut rewritten = 0;
    for (source, position, bytes) in batch {
        if let Some(bytes) = bytes {
            if bytes.first() != Some(&COMPRESSED_OBSERVATION) {
                let event: SavedEvent = serde_json::from_slice(&bytes).context("invalid usage observation")?;
                let compact = compress_observation(serde_json::to_vec(&event)?)?;
                if compact != bytes {
                    rewritten += tx.execute(
                        "UPDATE usage_observations SET payload = ?3 WHERE source = ?1 AND position = ?2",
                        rusqlite::params![source, position, compact],
                    )?;
                }
            }
        }
        next = Some((source, position));
        if started.elapsed() >= std::time::Duration::from_millis(100) {
            break;
        }
    }
    Ok((next, rewritten))
}

pub(super) fn save_event(event: &UsageEvent) -> anyhow::Result<Vec<u8>> {
    compress_observation(serde_json::to_vec(&SavedEvent {
        turn: event.turn.clone(),
        model: event.model.clone(),
        timestamp: event.timestamp.clone(),
        day: event.date.to_string(),
        total: event.total.into(),
        delta: event.delta.into(),
        has_file_baseline: event.has_file_baseline,
        last: event.last.map(Into::into),
        count_uncertain: event.count_uncertain,
        count_incomplete: event.count_incomplete,
        provider: event.provider.clone(),
        provider_source: event.provider_source.clone(),
        role: event.role.clone(),
        inherited: event.inherited,
        legacy_identity: event
            .legacy_identity
            .as_ref()
            .map(|identity| (identity.id, identity.session.clone(), identity.role.clone())),
        context_conflicts: (
            event.context_conflicts.model,
            event.context_conflicts.provider,
            event.context_conflicts.role,
            event.context_conflicts.turn,
        ),
    })?)
}

pub(super) fn restore_event(
    id: [u8; 32],
    session: String,
    bytes: &[u8],
) -> anyhow::Result<UsageEvent> {
    let decompressed;
    let bytes = if bytes.first() == Some(&COMPRESSED_OBSERVATION) {
        let length = observation_size(bytes)?;
        decompressed = zstd::bulk::decompress(&bytes[OBSERVATION_HEADER_BYTES..], length)
            .context("invalid compressed usage observation")?;
        ensure!(decompressed.len() == length, "usage observation length mismatch");
        &decompressed
    } else {
        bytes
    };
    let saved: SavedEvent = serde_json::from_slice(bytes).context("invalid usage observation")?;
    Ok(UsageEvent {
        id,
        session,
        turn: saved.turn,
        model: saved.model,
        timestamp: saved.timestamp,
        date: chrono::NaiveDate::parse_from_str(&saved.day, "%Y-%m-%d")?,
        total: saved.total.into(),
        delta: saved.delta.into(),
        has_file_baseline: saved.has_file_baseline,
        last: saved.last.map(Into::into),
        count_uncertain: saved.count_uncertain,
        count_incomplete: saved.count_incomplete,
        provider: saved.provider,
        provider_source: saved.provider_source,
        role: saved.role,
        inherited: saved.inherited,
        legacy_identity: saved
            .legacy_identity
            .map(|(id, session, role)| Box::new(LegacyIdentity { id, session, role })),
        context_conflicts: ContextConflicts {
            model: saved.context_conflicts.0,
            provider: saved.context_conflicts.1,
            role: saved.context_conflicts.2,
            turn: saved.context_conflicts.3,
        },
    })
}

#[derive(Serialize, Deserialize)]
struct Checkpoint {
    stamp: Option<(u64, Option<SystemTime>, Option<(u64, u64)>)>,
    offset: u64,
    needs_resume: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pending: Vec<u8>,
    skipping: bool,
    activity_truncated: bool,
    session: String,
    model: String,
    #[serde(default = "super::super::unverified_model_context")]
    model_unavailable: bool,
    turn: String,
    previous: Option<SavedTokens>,
    invalid_counts: bool,
    unlocalized_invalid_counts: bool,
    pending_count_gap: bool,
    identity_conflict: bool,
    continuation: bool,
    issues: usize,
    provider: String,
    role: String,
    history_parent: Option<String>,
    history_start: Option<u64>,
    inherited_contexts: Vec<InheritedUsageContext>,
    #[serde(default)]
    model_observations: HashMap<String, SessionModelObservation>,
    legacy_header: Option<(String, String)>,
    scope_parent_candidates: BTreeMap<String, HashSet<Option<String>>>,
    #[serde(default)]
    execution_incomplete: bool,
}

pub(super) fn committed_offset(state: &FileState) -> u64 {
    state.offset - state.pending.len() as u64
}

pub(super) fn save(state: &FileState) -> anyhow::Result<Vec<u8>> {
    Ok(serde_json::to_vec(&Checkpoint {
        stamp: state.stamp,
        offset: committed_offset(state),
        needs_resume: state.reader.is_some() || state.needs_resume || !state.pending.is_empty(),
        pending: Vec::new(),
        skipping: state.skipping,
        activity_truncated: state.activity_truncated,
        session: state.session.clone(),
        model: state.model.clone(),
        model_unavailable: state.model_unavailable,
        turn: state.turn.clone(),
        previous: state.previous.map(Into::into),
        invalid_counts: state.invalid_counts,
        unlocalized_invalid_counts: state.unlocalized_invalid_counts,
        pending_count_gap: state.pending_count_gap,
        identity_conflict: state.identity_conflict,
        continuation: state.continuation,
        issues: state.issues,
        provider: state.provider.clone(),
        role: state.role.clone(),
        history_parent: state.history_boundary.parent.clone(),
        history_start: state.history_boundary.start,
        inherited_contexts: state.inherited_contexts.clone(),
        model_observations: state.model_observations.clone(),
        legacy_header: state.legacy_header.clone(),
        scope_parent_candidates: state.scope_parent_candidates.clone(),
        execution_incomplete: state.execution_incomplete,
    })?)
}

pub(super) fn restore(bytes: &[u8]) -> anyhow::Result<FileState> {
    let saved: Checkpoint = serde_json::from_slice(bytes).context("invalid usage parser state")?;
    ensure!(
        saved.pending.len() <= super::super::MAX_LINE
            && saved.pending.len() as u64 <= saved.offset
            && (!saved.skipping || saved.pending.is_empty()),
        "invalid pending usage input"
    );
    let mut pending = index_budget::PendingLine::default();
    ensure!(
        pending.try_extend(&saved.pending),
        "usage parser pending-input budget exhausted"
    );
    let mut state = FileState {
        stamp: saved.stamp,
        offset: saved.offset,
        needs_resume: saved.needs_resume,
        pending,
        skipping: saved.skipping,
        activity_truncated: saved.activity_truncated,
        session: saved.session,
        model: saved.model,
        model_unavailable: saved.model_unavailable,
        turn: saved.turn,
        previous: saved.previous.map(Into::into),
        invalid_counts: saved.invalid_counts,
        unlocalized_invalid_counts: saved.unlocalized_invalid_counts,
        pending_count_gap: saved.pending_count_gap,
        identity_conflict: saved.identity_conflict,
        continuation: saved.continuation,
        issues: saved.issues,
        provider: saved.provider,
        role: saved.role,
        inherited_contexts: saved.inherited_contexts,
        model_observations: saved.model_observations,
        legacy_header: saved.legacy_header,
        scope_parent_candidates: saved.scope_parent_candidates,
        execution_incomplete: saved.execution_incomplete,
        ..FileState::default()
    };
    state.history_boundary.parent = saved.history_parent;
    state.history_boundary.start = saved.history_start;
    index_budget::Meter::new(&state).refresh(&mut state, 0);
    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use serde_json::json;

    fn usage(input: u64, output: u64, cached: Option<u64>, last: serde_json::Value) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "type": "event_msg",
            "timestamp": "2026-11-10T00:00:00Z",
            "payload": {
                "type": "token_count",
                "info": {
                    "total_token_usage": {
                        "input_tokens": input,
                        "output_tokens": output,
                        "cached_input_tokens": cached
                    },
                    "last_token_usage": last
                }
            }
        }))
        .unwrap()
    }

    #[test]
    fn restores_inherited_context_and_rewinds_pending_input() {
        let mut state = FileState {
            offset: 100,
            session: "session-a".into(),
            model: "model-a".into(),
            provider: "provider-a".into(),
            turn: "turn-a".into(),
            pending_count_gap: true,
            continuation: true,
            ..FileState::default()
        };
        state.history_boundary.parent = Some("parent".into());
        state.history_boundary.start = Some(8);
        state.inherited_contexts.push(InheritedUsageContext {
            session: "parent".into(),
            model: "model-parent".into(),
            end: 8,
            ..InheritedUsageContext::default()
        });
        state.previous = Some(Tokens {
            input_tokens: 100,
            cached_input_tokens: 0,
            output_tokens: 10,
            total_tokens: 110,
            reasoning_output_tokens: Some(2),
            cached_known: false,
        });
        assert!(state.pending.try_extend(b"{\"type\":"));

        let bytes = save(&state).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("\\\"type\\\""));
        let restored = restore(&bytes).unwrap();
        assert_eq!(restored.offset, 92);
        assert!(restored.pending.is_empty());
        assert!(restored.needs_resume);
        assert!(restored.pending_count_gap);
        assert!(restored.continuation);
        assert_eq!(restored.history_boundary.parent.as_deref(), Some("parent"));
        assert_eq!(restored.history_boundary.start, Some(8));
        assert_eq!(restored.inherited_contexts[0].model, "model-parent");
        let previous = restored.previous.unwrap();
        assert!(!previous.cached_known);
        assert_eq!(previous.reasoning_output_tokens, Some(2));
        assert!(restored.retained_charge >= restored.pending.capacity());
    }

    #[test]
    fn legacy_checkpoint_cannot_reuse_unverified_model_context() {
        let mut state = FileState {
            session: "session-a".into(),
            model: "model-a".into(),
            ..FileState::default()
        };
        state.inherited_contexts.push(InheritedUsageContext {
            session: "parent".into(),
            model: "parent-model".into(),
            ..InheritedUsageContext::default()
        });
        let mut saved: serde_json::Value = serde_json::from_slice(&save(&state).unwrap()).unwrap();
        saved.as_object_mut().unwrap().remove("model_unavailable");
        saved.as_object_mut().unwrap().remove("model_observations");
        saved["inherited_contexts"][0]
            .as_object_mut()
            .unwrap()
            .remove("model_unavailable");

        let restored = restore(&serde_json::to_vec(&saved).unwrap()).unwrap();
        assert!(restored.model_unavailable);
        assert!(restored.inherited_contexts[0].model_unavailable);
    }

    #[test]
    fn restored_cumulative_state_matches_uninterrupted_parsing() {
        let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let mut original = FileState {
            session: "session-a".into(),
            model: "model-a".into(),
            role: "main".into(),
            ..FileState::default()
        };
        original.accept(&usage(100, 10, None, serde_json::Value::Null), cutoff);
        original.pending_count_gap = true;
        let mut restored = restore(&save(&original).unwrap()).unwrap();
        assert!(restored.events.is_empty());

        let next = usage(
            150,
            20,
            Some(40),
            json!({"input_tokens":50,"output_tokens":10,"cached_input_tokens":10}),
        );
        original.accept(&next, cutoff);
        restored.accept(&next, cutoff);
        let expected = original.events.last().unwrap();
        let actual = restored.events.last().unwrap();
        assert!(actual == expected);
        assert!(actual.count_incomplete);
        assert!(actual.delta.cached_known);
        assert_eq!(actual.delta.cached_input_tokens, 10);
        assert!(!restored.pending_count_gap);
    }

    #[test]
    fn checkpoint_does_not_persist_tool_contents() {
        let mut state = FileState::default();
        state.calls.insert(
            "call".into(),
            super::super::super::Call {
                timestamp: "2026-11-10T00:00:00Z".into(),
                session: "session-a".into(),
                payload: json!({"arguments":"private-command-content"}),
            },
        );
        let bytes = save(&state).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("private-command-content"));
        assert!(restore(&bytes).unwrap().calls.is_empty());
    }

    #[test]
    fn observation_round_trip_retains_unknown_components_and_context() {
        let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let mut state = FileState {
            session: "session-a".into(),
            model: "model-a".into(),
            role: "main".into(),
            ..FileState::default()
        };
        state.accept(&usage(100, 10, None, serde_json::Value::Null), cutoff);
        let event = state.events.first_mut().unwrap();
        event.context_conflicts.provider = true;
        event.count_incomplete = true;
        event.legacy_identity = Some(Box::new(LegacyIdentity {
            id: [4; 32],
            session: "parent".into(),
            role: "main".into(),
        }));
        assert!(!event.total.cached_known);
        let restored =
            restore_event(event.id, event.session.clone(), &save_event(event).unwrap()).unwrap();
        assert!(restored == *event);
    }

    fn legacy_event(bytes: &[u8]) -> Vec<u8> {
        let decompressed;
        let bytes = if bytes.first() == Some(&COMPRESSED_OBSERVATION) {
            decompressed = zstd::bulk::decompress(
                &bytes[OBSERVATION_HEADER_BYTES..],
                observation_size(bytes).unwrap(),
            ).unwrap();
            &decompressed
        } else {
            bytes
        };
        let mut value: serde_json::Value = serde_json::from_slice(bytes).unwrap();
        let fields = value.as_object_mut().unwrap();
        for (short, long) in [
            ("t", "turn"), ("m", "model"), ("a", "timestamp"), ("d", "day"),
            ("u", "total"), ("v", "delta"), ("b", "has_file_baseline"), ("l", "last"),
            ("q", "count_uncertain"), ("c", "count_incomplete"), ("p", "provider"),
            ("s", "provider_source"), ("r", "role"), ("h", "inherited"),
            ("i", "legacy_identity"), ("x", "context_conflicts"),
        ] {
            let value = fields.remove(short).unwrap();
            fields.insert(long.into(), value);
        }
        // 与旧 SavedEvent 的固定首字段一致。
        let turn = fields.remove("turn").unwrap();
        let tail = serde_json::to_string(&value).unwrap();
        format!("{{\"turn\":{turn},{}", &tail[1..]).into_bytes()
    }

    #[test]
    fn compact_observations_read_legacy_encoding_without_losing_any_fields() {
        let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let mut state = FileState {
            session: "session-a".into(), model: "model-a".into(),
            provider: "provider-a".into(), role: "main".into(), ..FileState::default()
        };
        state.accept(&usage(100, 10, Some(20), serde_json::Value::Null), cutoff);
        let event = &state.events[0];
        let compact = save_event(event).unwrap();
        assert_eq!(compact.first(), Some(&COMPRESSED_OBSERVATION));
        let previous = zstd::bulk::decompress(
            &compact[OBSERVATION_HEADER_BYTES..],
            observation_size(&compact).unwrap(),
        ).unwrap();
        let legacy = legacy_event(&compact);
        assert!(legacy.len() > compact.len() + 100);
        assert!(restore_event(event.id, event.session.clone(), &legacy).unwrap() == *event);
        assert!(restore_event(event.id, event.session.clone(), &previous).unwrap() == *event);
        assert!(restore_event(event.id, event.session.clone(), &compact).unwrap() == *event);
    }

    #[test]
    fn compressed_observation_rejects_corruption_and_oversized_payloads() {
        assert!(observation_size(&[COMPRESSED_OBSERVATION]).is_err());
        let mut oversized = vec![COMPRESSED_OBSERVATION];
        oversized.extend_from_slice(&((index_budget::MAX_RETAINED_BYTES + 1) as u32).to_le_bytes());
        assert!(observation_size(&oversized).is_err());
        let mut invalid = vec![COMPRESSED_OBSERVATION];
        invalid.extend_from_slice(&10u32.to_le_bytes());
        invalid.extend_from_slice(b"invalid");
        assert!(restore_event([0; 32], "session".into(), &invalid).is_err());
    }

    #[test]
    fn payload_compaction_limits_batch_bytes_and_resumes() {
        let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let mut state = FileState {
            session: "session-a".into(), model: "model-a".into(),
            role: "main".into(), ..FileState::default()
        };
        state.accept(&usage(100, 10, Some(20), serde_json::Value::Null), cutoff);
        let encoded = save_event(&state.events[0]).unwrap();
        let plain = zstd::bulk::decompress(
            &encoded[OBSERVATION_HEADER_BYTES..],
            observation_size(&encoded).unwrap(),
        ).unwrap();
        let mut event: serde_json::Value = serde_json::from_slice(&plain).unwrap();
        event["m"] = "x".repeat(256 * 1024).into();
        let plain = serde_json::to_vec(&event).unwrap();
        let mut db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE usage_observations(source BLOB, position INTEGER, payload BLOB,
             PRIMARY KEY(source, position)) WITHOUT ROWID",
        ).unwrap();
        let tx = db.transaction().unwrap();
        for position in 0..16 {
            tx.execute(
                "INSERT INTO usage_observations VALUES (X'01', ?1, ?2)",
                rusqlite::params![position, plain],
            ).unwrap();
        }
        tx.commit().unwrap();

        let tx = db.transaction().unwrap();
        let (mut cursor, changed) = compact_payloads(&tx, None).unwrap();
        tx.commit().unwrap();
        assert!(changed > 0);
        assert!(changed * plain.len() <= COMPACTION_BATCH_BYTES);
        assert_eq!(cursor.as_ref().unwrap().1, changed as i64 - 1);
        let mut total = changed;
        while cursor.is_some() {
            let tx = db.transaction().unwrap();
            let (next, changed) = compact_payloads(&tx, cursor.as_ref()).unwrap();
            tx.commit().unwrap();
            total += changed;
            cursor = next;
        }
        assert_eq!(total, 16);

        event["m"] = "x".repeat(COMPACTION_BATCH_BYTES).into();
        let oversized = serde_json::to_vec(&event).unwrap();
        assert!(oversized.len() > COMPACTION_BATCH_BYTES);
        db.execute(
            "INSERT INTO usage_observations VALUES (X'01', 16, ?1), (X'01', 17, ?2)",
            rusqlite::params![oversized, plain],
        ).unwrap();
        let mut cursor = Some((vec![1], 15));
        let mut changed = 0;
        while cursor.is_some() {
            let tx = db.transaction().unwrap();
            let (next, count) = compact_payloads(&tx, cursor.as_ref()).unwrap();
            tx.commit().unwrap();
            changed += count;
            cursor = next;
        }
        assert_eq!(changed, 1);
        assert_eq!(db.query_row(
            "SELECT payload FROM usage_observations WHERE position = 16",
            [],
            |row| row.get::<_, Vec<u8>>(0),
        ).unwrap(), oversized);
        assert_eq!(db.query_row(
            "SELECT substr(payload, 1, 1) FROM usage_observations WHERE position = 17",
            [],
            |row| row.get::<_, Vec<u8>>(0),
        ).unwrap(), vec![COMPRESSED_OBSERVATION]);
    }

    #[test]
    fn full_index_payload_rewrite_rolls_back_then_resumes_without_changing_counts_or_cursor() {
        use super::super::{PersistentIndex, resolve, scan};
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("rollout.jsonl");
        let database = root.path().join("usage.sqlite");
        let session = "00000000-0000-4000-8000-000000000010";
        let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let mut contents = format!("{}\n", json!({"type":"session_meta","payload":{"id":session}}));
        for count in 1..=1100 {
            contents.push_str(&String::from_utf8(usage(
                count * 100, count * 10, Some(count * 20),
                json!({"input_tokens":100,"cached_input_tokens":20,"output_tokens":10}),
            )).unwrap());
            contents.push('\n');
        }
        std::fs::write(&path, contents).unwrap();
        let path = std::fs::canonicalize(path).unwrap();
        let mut index = PersistentIndex::open(&database).unwrap();
        let mut state = FileState::default();
        loop {
            if scan::scan_source(
                &mut index, &mut state, &path, cutoff,
                &mut index_budget::Budget::new(std::iter::empty()),
                |tx, sessions| resolve::resolve(tx, sessions, &BTreeMap::new()),
            ).unwrap() { break; }
        }
        let originals = index.db.prepare(
            "SELECT source, position, payload FROM usage_observations ORDER BY source, position",
        ).unwrap().query_map([], |row| Ok((
            row.get::<_, Vec<u8>>(0)?, row.get::<_, i64>(1)?, row.get::<_, Vec<u8>>(2)?,
        ))).unwrap().collect::<Result<Vec<_>, _>>().unwrap();
        for (ordinal, (source, position, payload)) in originals.iter().enumerate() {
            let previous = if ordinal % 2 == 0 {
                legacy_event(payload)
            } else {
                zstd::bulk::decompress(
                    &payload[OBSERVATION_HEADER_BYTES..],
                    observation_size(payload).unwrap(),
                ).unwrap()
            };
            index.db.execute(
                "UPDATE usage_observations SET payload = ?3 WHERE source = ?1 AND position = ?2",
                rusqlite::params![source, position, previous],
            ).unwrap();
        }
        let source_key = scan::encode_source(&path);
        let saved_source = index.source_state(&source_key).unwrap().unwrap();
        let revision = index.revision().unwrap();
        let counts = index.db.query_row(
            "SELECT SUM(input), SUM(output), SUM(records) FROM usage_daily", [],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?)),
        ).unwrap();
        let bytes_before: i64 = index.db.query_row(
            "SELECT SUM(length(payload)) FROM usage_observations", [], |row| row.get(0),
        ).unwrap();
        let pages: i64 = index.db.pragma_query_value(None, "page_count", |row| row.get(0)).unwrap();
        index.db.pragma_update(None, "user_version", 13).unwrap();
        drop(index);
        let mut index = PersistentIndex::open_with_budget(&database, pages as u64 * 4096 * 3).unwrap();
        assert_eq!(index.db.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0)).unwrap(), 14);
        assert_eq!(index.db.pragma_query_value(None, "max_page_count", |row| row.get::<_, i64>(0)).unwrap(), pages);
        index.db.execute_batch(
            "CREATE TEMP TRIGGER reject_shrink BEFORE UPDATE OF payload ON usage_observations
             BEGIN SELECT RAISE(ABORT, 'synthetic rewrite failure'); END;",
        ).unwrap();
        assert!(index.compact_usage_payloads(None).unwrap_err().to_string().contains("synthetic rewrite failure"));
        assert_eq!(index.source_state(&source_key).unwrap().unwrap(), saved_source);
        assert_eq!(index.db.query_row(
            "SELECT SUM(length(payload)) FROM usage_observations", [], |row| row.get::<_, i64>(0),
        ).unwrap(), bytes_before);
        index.db.execute_batch("DROP TRIGGER reject_shrink;").unwrap();
        let (cursor, changed) = index.compact_usage_payloads(None).unwrap();
        assert!(cursor.is_some());
        assert!(changed > 0 && changed <= 1024);
        assert_eq!(index.source_state(&source_key).unwrap().unwrap(), saved_source);
        assert_eq!(index.revision().unwrap(), revision);
        drop(index);

        let mut reopened = PersistentIndex::open(&database).unwrap();
        reopened.db.pragma_update(None, "max_page_count", pages).unwrap();
        let mut cursor = None;
        let mut remaining = 0;
        loop {
            let (next, changed) = reopened.compact_usage_payloads(cursor.as_ref()).unwrap();
            remaining += changed;
            cursor = next;
            if cursor.is_none() { break; }
        }
        assert_eq!(changed + remaining, originals.len());
        assert_eq!(reopened.source_state(&source_key).unwrap().unwrap(), saved_source);
        assert_eq!(reopened.revision().unwrap(), revision);
        assert!(reopened.db.query_row(
            "SELECT SUM(length(payload)) FROM usage_observations", [], |row| row.get::<_, i64>(0),
        ).unwrap() < bytes_before);
        assert_eq!(reopened.db.query_row(
            "SELECT SUM(input), SUM(output), SUM(records) FROM usage_daily", [],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?)),
        ).unwrap(), counts);
        reopened.repack_usage_payloads().unwrap();
        let packed_pages: i64 = reopened.db.pragma_query_value(None, "page_count", |row| row.get(0)).unwrap();
        assert!(packed_pages < pages, "{packed_pages} >= {pages}");
        assert_eq!(reopened.db.pragma_query_value(None, "max_page_count", |row| row.get::<_, i64>(0)).unwrap(), pages);
        let tx = reopened.db.transaction().unwrap();
        let restored = resolve::resolve(&tx, &std::collections::BTreeSet::from([session.into()]), &BTreeMap::new()).unwrap();
        assert_eq!(restored.len(), originals.len());
        assert_eq!(restored.iter().map(|event| event.input).sum::<u64>(), counts.0 as u64);
        tx.commit().unwrap();
        let mut source = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        use std::io::Write;
        for count in 1101..=1200 {
            writeln!(source, "{}", String::from_utf8(usage(
                count * 100, count * 10, Some(count * 20),
                json!({"input_tokens":100,"cached_input_tokens":20,"output_tokens":10}),
            )).unwrap()).unwrap();
        }
        drop(source);
        let mut state = scan::restore_source(&reopened, &path).unwrap();
        assert!(scan::scan_source(
            &mut reopened, &mut state, &path, cutoff,
            &mut index_budget::Budget::new(std::iter::empty()),
            |tx, sessions| resolve::resolve(tx, sessions, &BTreeMap::new()),
        ).unwrap());
        assert_eq!(reopened.db.query_row(
            "SELECT SUM(input) FROM usage_daily", [], |row| row.get::<_, i64>(0),
        ).unwrap(), counts.0 + 10_000);
        assert!(reopened.db.pragma_query_value(None, "page_count", |row| row.get::<_, i64>(0)).unwrap() <= pages);
    }

    #[test]
    fn compressed_reader_replays_only_committed_prefix_after_restart() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("rollout-checkpoint.jsonl.zst");
        let session = "00000000-0000-4000-8000-000000000010";
        let head = json!({"type":"session_meta","payload":{"id":session}});
        let first = String::from_utf8(usage(100, 10, Some(20), serde_json::Value::Null)).unwrap();
        let second = String::from_utf8(usage(
            150,
            20,
            Some(30),
            json!({"input_tokens":50,"output_tokens":10,"cached_input_tokens":10}),
        ))
        .unwrap();
        let filler = format!(
            "{}\n",
            json!({"type":"response_item","payload":{"type":"message","content":"x".repeat(1024)}})
        );
        let contents = format!("{head}\n{first}\n{}{second}\n", filler.repeat(4300));
        std::fs::write(
            &path,
            zstd::stream::encode_all(contents.as_bytes(), 1).unwrap(),
        )
        .unwrap();
        let path = std::fs::canonicalize(path).unwrap();
        let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();

        let mut original = FileState::default();
        let mut budget = index_budget::Budget::new(std::iter::empty());
        assert!(
            !original
                .read_with_budget(&path, cutoff, &mut budget)
                .unwrap()
        );
        assert!(original.reader.is_some());
        assert_eq!(original.events.len(), 1);
        let mut resumed = restore(&save(&original).unwrap()).unwrap();
        assert!(resumed.needs_resume);

        let mut completed = false;
        for _ in 0..8 {
            let mut budget = index_budget::Budget::new(std::iter::empty());
            if resumed
                .read_with_budget(&path, cutoff, &mut budget)
                .unwrap()
            {
                completed = true;
                break;
            }
        }
        assert!(completed);
        assert_eq!(resumed.events.len(), 1);
        let mut budget = index_budget::Budget::new(std::iter::empty());
        assert!(
            original
                .read_with_budget(&path, cutoff, &mut budget)
                .unwrap()
        );
        assert!(resumed.events[0] == original.events[1]);
        assert_eq!(resumed.events[0].delta.input_tokens, 50);
    }
}
