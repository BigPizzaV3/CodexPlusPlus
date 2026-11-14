use super::super::super::{FileState, Tokens};
use super::super::{
    Observation, PersistentIndex, ResolvedEvent, append::Appender, encode_source, resolve,
};
use super::*;
use chrono::{NaiveDate, TimeZone};
use serde_json::json;
use std::path::Path;

const SESSION: &str = "00000000-0000-4000-8000-000000000010";

fn at(day: &str) -> i64 {
    Local
        .from_local_datetime(
            &NaiveDate::parse_from_str(day, "%Y-%m-%d")
                .unwrap()
                .and_hms_opt(12, 0, 0)
                .unwrap(),
        )
        .single()
        .unwrap()
        .timestamp_millis()
}

fn now() -> i64 {
    at("2026-11-10")
}

fn accept(state: &mut FileState, timestamp: i64, input: u64) {
    state.accept(
        &serde_json::to_vec(&json!({
            "type":"event_msg",
            "timestamp":DateTime::from_timestamp_millis(timestamp).unwrap().to_rfc3339(),
            "payload":{
                "type":"token_count",
                "info":{
                    "total_token_usage":{
                        "input_tokens":input,
                        "cached_input_tokens":input / 5,
                        "output_tokens":input / 10
                    },
                    "last_token_usage":{
                        "input_tokens":10,
                        "cached_input_tokens":2,
                        "output_tokens":1
                    }
                }
            }
        }))
        .unwrap(),
        NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
    );
}

fn source_batch(path: &Path, count: usize, day: &str) -> SourceBatch {
    let mut state = FileState {
        session: SESSION.into(),
        model: "model-a".into(),
        provider: "supplier-a".into(),
        role: "main".into(),
        ..FileState::default()
    };
    for index in 0..count {
        accept(&mut state, at(day) + index as i64, (index as u64 + 1) * 10);
    }
    state.offset = (count as u64 + 1) * 100;
    let observations = state
        .events
        .iter()
        .enumerate()
        .map(|(index, event)| Observation {
            position: (index as u64 + 1) * 100,
            event_id: event.id,
            session: event.session.clone(),
            payload: checkpoint::save_event(event).unwrap(),
        })
        .collect();
    SourceBatch {
        source: encode_source(path),
        identity: b"source-a".to_vec(),
        expected_offset: 0,
        next_offset: state.offset,
        parser_state: checkpoint::save(&state).unwrap(),
        complete: true,
        usage_cutoff: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        replace: false,
        observations,
        execution_observations: Vec::new(),
        scope_session: SESSION.into(),
        scope_valid: true,
        scope_parents: vec![(SESSION.into(), None)],
        scope_aliases: vec![(SESSION.into(), SESSION.into())],
        execution_valid: true,
        count_gap_sessions: None,
        now_ms: now(),
    }
}

fn open(path: &Path) -> PersistentIndex {
    let mut index = PersistentIndex::open(path).unwrap();
    let tx = index.db.transaction().unwrap();
    initialize(&tx).unwrap();
    tx.commit().unwrap();
    index
}

fn commit(index: &mut PersistentIndex, batch: &mut SourceBatch) {
    index
        .commit_batch(batch, |tx, sessions| {
            resolve::resolve(tx, sessions, &BTreeMap::new())
        })
        .unwrap();
}

fn maintain(index: &mut PersistentIndex, cursor: Option<&Cursor>) -> Batch {
    let tx = index.db.transaction().unwrap();
    let batch = compact(&tx, now(), cursor).unwrap();
    assert!(batch.scanned <= BATCH_ROWS);
    assert!(batch.deleted <= BATCH_ROWS);
    if batch.changed {
        tx.execute("UPDATE usage_meta SET revision = revision + 1", [])
            .unwrap();
    }
    tx.commit().unwrap();
    batch
}

fn finish(index: &mut PersistentIndex) {
    let mut cursor = None;
    for _ in 0..40 {
        cursor = maintain(index, cursor.as_ref()).next;
        if cursor.is_none() {
            return;
        }
    }
    panic!("sealed maintenance did not finish");
}

fn daily(index: &PersistentIndex) -> Vec<(String, i64, Option<i64>, i64, i64)> {
    index
        .db
        .prepare("SELECT day, input, cached, output, records FROM usage_daily ORDER BY day")
        .unwrap()
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn count(index: &PersistentIndex, table: &str) -> i64 {
    index
        .db
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}

fn appended(index: &PersistentIndex, source: &[u8], day: &str, input: u64) -> SourceBatch {
    let saved = index.source_state(source).unwrap().unwrap();
    let mut state = checkpoint::restore(&saved.parser_state).unwrap();
    accept(&mut state, at(day) + input as i64, input);
    state.offset = saved.offset + 100;
    let observations = state
        .events
        .iter()
        .map(|event| Observation {
            position: saved.offset,
            event_id: event.id,
            session: event.session.clone(),
            payload: checkpoint::save_event(event).unwrap(),
        })
        .collect();
    SourceBatch {
        source: source.to_vec(),
        identity: saved.identity,
        expected_offset: saved.offset,
        next_offset: state.offset,
        parser_state: checkpoint::save(&state).unwrap(),
        complete: true,
        usage_cutoff: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        replace: false,
        observations,
        execution_observations: Vec::new(),
        scope_session: SESSION.into(),
        scope_valid: true,
        scope_parents: vec![(SESSION.into(), None)],
        scope_aliases: vec![(SESSION.into(), SESSION.into())],
        execution_valid: true,
        count_gap_sessions: None,
        now_ms: now(),
    }
}

#[test]
fn old_completed_source_is_sealed_in_bounded_steps_and_resumes_cleanup_after_restart() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("usage.sqlite");
    let mut index = open(&path);
    let mut input = source_batch(&directory.path().join("rollout-a.jsonl"), 600, "2026-08-01");
    commit(&mut index, &mut input);
    let before = daily(&index);
    let source_before = index.source_state(&input.source).unwrap().unwrap();
    let first = maintain(&mut index, None);
    assert_eq!(first.scanned, 256);
    assert_eq!(first.sealed, 0);
    assert_eq!(count(&index, "usage_observations"), 600);
    let second = maintain(&mut index, first.next.as_ref());
    assert_eq!(second.scanned, 256);
    let third = maintain(&mut index, second.next.as_ref());
    assert_eq!(third.scanned, 88);
    assert_eq!(third.sealed, 1);
    assert_eq!(count(&index, "usage_observations"), 600);
    let fourth = maintain(&mut index, third.next.as_ref());
    assert_eq!(fourth.deleted, 256);
    drop(index);

    let mut index = open(&path);
    finish(&mut index);
    assert_eq!(daily(&index), before);
    assert_eq!(count(&index, "usage_observations"), 1);
    assert_eq!(count(&index, "usage_events"), 0);
    assert_eq!(count(&index, "usage_day_only"), 1);
    assert_eq!(
        index.source_state(&input.source).unwrap().unwrap(),
        source_before
    );
    let retained: (i64, Vec<u8>) = index
        .db
        .query_row(
            "SELECT position, payload FROM usage_observations",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(retained.0, 60_000);
    let sealed: (String, i64, Option<String>) = index
        .db
        .query_row(
            "SELECT cutoff, anchor_position, review_reason FROM usage_sealed_sessions",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(sealed, ("2026-09-09".into(), 60_000, None));
}

#[test]
fn recent_append_uses_retained_baseline_without_replaying_frozen_totals() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("usage.sqlite");
    let mut index = open(&path);
    let mut old = source_batch(&directory.path().join("rollout-a.jsonl"), 20, "2026-08-01");
    commit(&mut index, &mut old);
    let frozen = daily(&index);
    finish(&mut index);
    drop(index);
    let mut index = open(&path);
    let mut appender = Appender::default();
    for input in [210, 220] {
        let mut next = appended(&index, &old.source, "2026-09-09", input);
        let tx = index.db.transaction().unwrap();
        assert!(guard_batch(&tx, &next).unwrap().is_empty());
        tx.rollback().unwrap();
        index
            .commit_batch_resolved(&mut next, |tx, sessions, batch| {
                let mut result = appender.resolve(tx, sessions, batch, &BTreeMap::new())?;
                let seals = ranges(tx, sessions)?;
                assert!(matches!(result, Resolution::Append(_)));
                assert!(filter_resolution(&seals, batch, &mut result).is_empty());
                Ok(result)
            })
            .unwrap();
    }
    let values = daily(&index);
    assert_eq!(values[0], frozen[0]);
    assert_eq!(values[1], ("2026-09-09".into(), 20, Some(4), 2, 2));
    assert_eq!(count(&index, "usage_events"), 2);
    assert_eq!(count(&index, "usage_observations"), 3);
}

#[test]
fn unknown_classification_does_not_prevent_reclaiming_correct_old_counts() {
    let directory = tempfile::tempdir().unwrap();
    let mut known = open(&directory.path().join("known.sqlite"));
    let mut unknown = open(&directory.path().join("unknown.sqlite"));
    let path = directory.path().join("rollout-a.jsonl");
    let mut control = source_batch(&path, 10, "2026-08-01");
    commit(&mut known, &mut control);
    let mut candidate = source_batch(&path, 10, "2026-08-01");
    let mut state = checkpoint::restore(&candidate.parser_state).unwrap();
    state.model = "unknown".into();
    state.provider = "openai".into();
    candidate.parser_state = checkpoint::save(&state).unwrap();
    for observation in &mut candidate.observations {
        let mut event = checkpoint::restore_event(
            observation.event_id,
            observation.session.clone(),
            &observation.payload,
        )
        .unwrap();
        event.model = "unknown".into();
        event.provider = "openai".into();
        observation.payload = checkpoint::save_event(&event).unwrap();
    }
    commit(&mut unknown, &mut candidate);
    assert_eq!(daily(&unknown), daily(&known));
    finish(&mut unknown);
    assert_eq!(count(&unknown, "usage_sealed_sessions"), 1);
    assert_eq!(count(&unknown, "usage_observations"), 1);
    assert_eq!(daily(&unknown), daily(&known));
    let model: String = unknown
        .db
        .query_row("SELECT model FROM usage_daily", [], |row| row.get(0))
        .unwrap();
    assert_eq!(model, "unknown");
}

#[test]
fn mixed_recent_incomplete_inherited_and_multiple_sources_are_not_sealed() {
    for scenario in [
        "recent",
        "mixed",
        "incomplete",
        "inherited",
        "multiple",
        "unordered",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let mut index = open(&directory.path().join("usage.sqlite"));
        let day = if scenario == "recent" {
            "2026-09-09"
        } else {
            "2026-08-01"
        };
        let mut input = source_batch(&directory.path().join("rollout-a.jsonl"), 3, day);
        if scenario == "incomplete" {
            input.complete = false;
        }
        if scenario == "inherited" {
            let mut state = checkpoint::restore(&input.parser_state).unwrap();
            state.history_boundary.parent = Some("parent".into());
            input.parser_state = checkpoint::save(&state).unwrap();
            input.scope_parents = vec![(SESSION.into(), Some("parent".into()))];
        }
        if scenario == "unordered" {
            let row = &mut input.observations[1];
            let mut event =
                checkpoint::restore_event(row.event_id, row.session.clone(), &row.payload).unwrap();
            event.timestamp = DateTime::from_timestamp_millis(at(day) - 1)
                .unwrap()
                .to_rfc3339();
            event.id = event.original_id();
            row.event_id = event.id;
            row.payload = checkpoint::save_event(&event).unwrap();
        }
        commit(&mut index, &mut input);
        if scenario == "mixed" {
            let mut next = appended(&index, &input.source, "2026-09-09", 40);
            commit(&mut index, &mut next);
        }
        if scenario == "multiple" {
            index
                .db
                .execute(
                    "INSERT INTO usage_sources(source, identity, offset, parser_state, complete,
                    scope_session, scope_indexed, scope_valid)
                 SELECT X'02', identity, offset, parser_state, complete, scope_session, 1, 1
                 FROM usage_sources WHERE source = ?1",
                    [&input.source],
                )
                .unwrap();
        }
        let before = daily(&index);
        let observations = count(&index, "usage_observations");
        finish(&mut index);
        assert_eq!(count(&index, "usage_sealed_sessions"), 0, "{scenario}");
        assert_eq!(
            count(&index, "usage_observations"),
            observations,
            "{scenario}"
        );
        assert_eq!(daily(&index), before, "{scenario}");
    }
}

#[test]
fn source_changes_during_validation_restart_without_pruning() {
    let directory = tempfile::tempdir().unwrap();
    let mut index = open(&directory.path().join("usage.sqlite"));
    let mut input = source_batch(&directory.path().join("rollout-a.jsonl"), 300, "2026-08-01");
    commit(&mut index, &mut input);
    let first = maintain(&mut index, None);
    assert_eq!(first.scanned, 256);
    let mut recent = appended(&index, &input.source, "2026-09-09", 3010);
    commit(&mut index, &mut recent);
    let before = daily(&index);
    let second = maintain(&mut index, first.next.as_ref());
    assert_eq!(second.scanned, 0);
    finish(&mut index);
    assert_eq!(count(&index, "usage_sealed_sessions"), 0);
    assert_eq!(count(&index, "usage_observations"), 301);
    assert_eq!(daily(&index), before);
}

#[test]
fn late_replacement_and_second_source_require_review_without_advancing_or_erasing() {
    for scenario in ["late", "replacement", "second-source", "ambiguous"] {
        let directory = tempfile::tempdir().unwrap();
        let mut index = open(&directory.path().join("usage.sqlite"));
        let mut input = source_batch(&directory.path().join("rollout-a.jsonl"), 5, "2026-08-01");
        commit(&mut index, &mut input);
        finish(&mut index);
        let before = daily(&index);
        let source = index.source_state(&input.source).unwrap().unwrap();
        let mut next = appended(&index, &input.source, "2026-09-09", 60);
        match scenario {
            "late" => next = appended(&index, &input.source, "2026-08-02", 60),
            "replacement" => {
                next.replace = true;
                next.identity = b"replacement".to_vec();
                next.observations.clear();
                next.next_offset = 0;
            }
            "second-source" => {
                next.source = encode_source(&directory.path().join("rollout-b.jsonl"));
                next.expected_offset = 0;
            }
            "ambiguous" => next.count_gap_sessions = Some(BTreeSet::from([SESSION.into()])),
            _ => unreachable!(),
        }
        let tx = index.db.transaction().unwrap();
        let reviews = guard_batch(&tx, &next).unwrap();
        assert_eq!(reviews.len(), 1, "{scenario}");
        assert_eq!(reviews[0].session, SESSION);
        tx.rollback().unwrap();
        let observations = next.observations.len();
        let error = index
            .commit_batch(&mut next, |_, _| {
                panic!("sealed source must be rejected before full resolution")
            })
            .unwrap_err();
        assert!(
            error
                .downcast_ref::<super::super::SealedReviewRequired>()
                .is_some()
        );
        assert_eq!(next.observations.len(), observations);
        let tx = index.db.transaction().unwrap();
        assert!(!mark_review(&tx, &reviews).unwrap());
        tx.commit().unwrap();
        assert_eq!(daily(&index), before, "{scenario}");
        assert_eq!(
            index.source_state(&input.source).unwrap().unwrap(),
            source,
            "{scenario}"
        );
        assert_eq!(count(&index, "usage_observations"), 1, "{scenario}");
        let status: Option<String> = index
            .db
            .query_row(
                "SELECT review_reason FROM usage_sealed_sessions WHERE session = ?1",
                [SESSION],
                |row| row.get(0),
            )
            .unwrap();
        assert!(status.is_some());
    }
}

#[test]
fn full_resolution_rolls_back_new_usage_and_persists_review_separately() {
    let directory = tempfile::tempdir().unwrap();
    let mut index = open(&directory.path().join("usage.sqlite"));
    let mut input = source_batch(&directory.path().join("rollout-a.jsonl"), 5, "2026-08-01");
    commit(&mut index, &mut input);
    finish(&mut index);
    let before = daily(&index);
    let source = index.source_state(&input.source).unwrap().unwrap();
    let mut next = appended(&index, &input.source, "2026-09-09", 60);
    let error = index
        .commit_batch(&mut next, |tx, sessions| {
            resolve::resolve(tx, sessions, &BTreeMap::new())
        })
        .unwrap_err();
    assert!(
        error
            .downcast_ref::<super::super::SealedReviewRequired>()
            .is_some()
    );
    assert_eq!(daily(&index), before);
    assert_eq!(index.source_state(&input.source).unwrap().unwrap(), source);
    assert_eq!(count(&index, "usage_observations"), 1);
    let reason: String = index
        .db
        .query_row(
            "SELECT review_reason FROM usage_sealed_sessions WHERE session = ?1",
            [SESSION],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(reason, "sealed usage requires full resolution");
}

#[test]
fn resolution_filter_keeps_recent_rows_and_rejects_recomputing_new_usage() {
    let directory = tempfile::tempdir().unwrap();
    let batch = source_batch(&directory.path().join("rollout-a.jsonl"), 1, "2026-08-01");
    let seals = vec![SealedRange {
        session: SESSION.into(),
        cutoff: "2026-09-09".into(),
    }];
    let event = |day: &str| ResolvedEvent {
        id: [1; 32],
        session: SESSION.into(),
        day: day.into(),
        timestamp_ms: at(day),
        model: "model-a".into(),
        provider: "supplier-a".into(),
        provider_source: None,
        role: "main".into(),
        context_conflicts: 0,
        count_uncertain: false,
        count_incomplete: false,
        input: 10,
        cached: Some(2),
        output: 1,
        reasoning: None,
    };
    let mut full = Resolution::Full(vec![event("2026-08-01"), event("2026-09-09")]);
    assert_eq!(filter_resolution(&seals, &batch, &mut full).len(), 1);
    let Resolution::Full(events) = full else {
        unreachable!()
    };
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].day, "2026-09-09");
    let mut tool_only = batch;
    tool_only.observations.clear();
    let mut full = Resolution::Full(vec![event("2026-08-01"), event("2026-09-09")]);
    assert!(filter_resolution(&seals, &tool_only, &mut full).is_empty());
    let mut append = Resolution::Append(vec![event("2026-08-01")]);
    assert_eq!(filter_resolution(&seals, &tool_only, &mut append).len(), 1);
}

#[test]
fn failed_seal_or_prune_rolls_back_and_keeps_daily_and_cumulative_baseline() {
    let directory = tempfile::tempdir().unwrap();
    let mut index = open(&directory.path().join("usage.sqlite"));
    let mut input = source_batch(&directory.path().join("rollout-a.jsonl"), 5, "2026-08-01");
    commit(&mut index, &mut input);
    let before = daily(&index);
    index
        .db
        .execute_batch(
            "CREATE TRIGGER reject_seal BEFORE INSERT ON usage_sealed_sessions
         BEGIN SELECT RAISE(ABORT, 'synthetic seal failure'); END;",
        )
        .unwrap();
    let tx = index.db.transaction().unwrap();
    assert!(compact(&tx, now(), None).is_err());
    tx.rollback().unwrap();
    assert_eq!(count(&index, "usage_sealed_sessions"), 0);
    assert_eq!(count(&index, "usage_observations"), 5);
    index.db.execute_batch("DROP TRIGGER reject_seal;").unwrap();
    let sealed = maintain(&mut index, None);
    assert_eq!(sealed.sealed, 1);
    index
        .db
        .execute_batch(
            "CREATE TRIGGER reject_prune BEFORE DELETE ON usage_observations
         BEGIN SELECT RAISE(ABORT, 'synthetic prune failure'); END;",
        )
        .unwrap();
    let tx = index.db.transaction().unwrap();
    assert!(compact(&tx, now(), sealed.next.as_ref()).is_err());
    tx.rollback().unwrap();
    assert_eq!(count(&index, "usage_observations"), 5);
    assert_eq!(daily(&index), before);
    index
        .db
        .execute_batch("DROP TRIGGER reject_prune;")
        .unwrap();
    finish(&mut index);
    assert_eq!(count(&index, "usage_observations"), 1);
    let saved = index.source_state(&input.source).unwrap().unwrap();
    assert_eq!(
        checkpoint::restore(&saved.parser_state).unwrap().previous,
        Some(Tokens::new(50, 10, 5))
    );
    assert_eq!(daily(&index), before);
}

#[test]
fn pruning_releases_observation_pages_with_no_budget_increase() {
    let directory = tempfile::tempdir().unwrap();
    let mut index = open(&directory.path().join("usage.sqlite"));
    let mut input = source_batch(&directory.path().join("rollout-a.jsonl"), 600, "2026-08-01");
    commit(&mut index, &mut input);
    let before = daily(&index);
    let pages: u64 = index
        .db
        .pragma_query_value(None, "page_count", |row| row.get(0))
        .unwrap();
    index
        .db
        .pragma_update(None, "max_page_count", pages)
        .unwrap();
    finish(&mut index);
    let limit: u64 = index
        .db
        .pragma_query_value(None, "max_page_count", |row| row.get(0))
        .unwrap();
    let free: u64 = index
        .db
        .pragma_query_value(None, "freelist_count", |row| row.get(0))
        .unwrap();
    assert_eq!(limit, pages);
    assert!(free > 0);
    assert_eq!(daily(&index), before);
}
