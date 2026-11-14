use super::*;
use crate::usage_history::structured::FileState;
use serde_json::json;
use std::collections::BTreeMap;

fn observation(position: u64, id: u8, input: u64, cached: Option<u64>) -> Observation {
    Observation {
        position,
        event_id: [id; 32],
        session: "session-a".into(),
        payload: serde_json::to_vec(&json!({"input":input,"cached":cached})).unwrap(),
    }
}

fn batch(source: &str, expected: u64, next: u64, observations: Vec<Observation>) -> SourceBatch {
    SourceBatch {
        source: source.as_bytes().to_vec(),
        identity: format!("{source}-identity").into_bytes(),
        expected_offset: expected,
        next_offset: next,
        parser_state: format!("state-{next}").into_bytes(),
        complete: true,
        usage_cutoff: chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        replace: false,
        observations,
        execution_observations: Vec::new(),
        scope_session: "session-a".into(),
        scope_valid: true,
        scope_parents: vec![("session-a".into(), None)],
        scope_aliases: vec![],
        execution_valid: true,
        count_gap_sessions: None,
        now_ms: 1_000,
    }
}

fn resolve(
    tx: &Transaction<'_>,
    sessions: &BTreeSet<String>,
) -> anyhow::Result<Vec<ResolvedEvent>> {
    let mut unique = BTreeMap::<[u8; 32], (String, Vec<u8>)>::new();
    for session in sessions {
        let mut statement = tx.prepare(
            "SELECT event_id, session, payload FROM usage_observations
             WHERE session = ?1 ORDER BY source, position",
        )?;
        let rows = statement.query_map([session], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Vec<u8>>(2)?,
            ))
        })?;
        for row in rows {
            let (id, session, payload) = row?;
            let id: [u8; 32] = id.try_into().map_err(|_| anyhow::anyhow!("invalid id"))?;
            if let Some(previous) = unique.get(&id) {
                ensure!(*previous == (session, payload), "conflicting observations");
            } else {
                unique.insert(id, (session, payload));
            }
        }
    }
    unique
        .into_iter()
        .map(|(id, (session, payload))| {
            let payload: serde_json::Value = serde_json::from_slice(&payload)?;
            Ok(ResolvedEvent {
                id,
                session,
                day: "2026-11-10".into(),
                timestamp_ms: 1000,
                model: "model-a".into(),
                provider: "provider-a".into(),
                provider_source: None,
                role: "main".into(),
                context_conflicts: 0,
                count_uncertain: false,
                count_incomplete: false,
                input: payload["input"].as_u64().unwrap(),
                cached: payload["cached"].as_u64(),
                output: 1,
                reasoning: None,
            })
        })
        .collect()
}

fn source(index: &PersistentIndex, name: &str) -> (i64, Vec<u8>) {
    index
        .db
        .query_row(
            "SELECT offset, parser_state FROM usage_sources WHERE source = ?1",
            [name.as_bytes()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
}

fn daily(index: &PersistentIndex) -> (i64, Option<i64>, i64) {
    index
        .db
        .query_row(
            "SELECT input, cached, records FROM usage_daily WHERE session = 'session-a'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap()
}

#[test]
fn oversized_index_reclaims_free_pages_without_external_vacuum() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("usage.sqlite");
    let mut index = PersistentIndex::open(&path).unwrap();
    index
        .commit_batch(
            &mut batch("source-a", 0, 20, vec![observation(5, 1, 10, Some(2))]),
            resolve,
        )
        .unwrap();
    assert_eq!(
        index.db.pragma_query_value(None, "auto_vacuum", |row| row.get::<_, i64>(0)).unwrap(),
        2
    );
    index.db.execute_batch(
        "CREATE TABLE reclaim_fixture(value BLOB);
         INSERT INTO reclaim_fixture VALUES (zeroblob(4194304));
         DROP TABLE reclaim_fixture;",
    ).unwrap();
    let pages: u64 = index.db.pragma_query_value(None, "page_count", |row| row.get(0)).unwrap();
    let page_size: u64 = index.db.pragma_query_value(None, "page_size", |row| row.get(0)).unwrap();
    let budget = (pages - 256) * page_size * 3;
    let original_revision = index.revision().unwrap();
    drop(index);

    let first = PersistentIndex::open_with_budget(&path, budget).unwrap();
    assert!(first.budget_blocked, "reclamation is bounded per retry");
    assert_eq!(daily(&first), (10, Some(2), 1));
    assert_eq!(first.revision().unwrap(), original_revision);
    drop(first);
    let mut reopened = PersistentIndex::open_with_budget(&path, budget).unwrap();
    assert!(!reopened.budget_blocked);
    assert_eq!(source(&reopened, "source-a").0, 20);
    assert_eq!(daily(&reopened), (10, Some(2), 1));
    assert_eq!(reopened.budget_bytes, budget);
    reopened.commit_batch(
        &mut batch("source-a", 20, 30, vec![observation(25, 2, 20, Some(3))]),
        resolve,
    ).unwrap();
    assert_eq!(daily(&reopened), (30, Some(5), 2));
}

#[test]
fn duplicate_observations_commit_once_and_resume_after_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("usage.sqlite");
    let mut index = PersistentIndex::open(&path).unwrap();
    let mut first = batch("source-a", 0, 20, vec![observation(5, 1, 10, Some(2))]);
    assert_eq!(index.commit_batch(&mut first, resolve).unwrap(), 1);
    assert!(first.observations.is_empty());
    assert_eq!(first.observations.capacity(), 0);
    let mut copy = batch("source-b", 0, 20, vec![observation(7, 1, 10, Some(2))]);
    assert_eq!(index.commit_batch(&mut copy, resolve).unwrap(), 2);
    assert_eq!(daily(&index), (10, Some(2), 1));
    drop(index);

    let mut reopened = PersistentIndex::open(&path).unwrap();
    assert_eq!(source(&reopened, "source-a"), (20, b"state-20".to_vec()));
    assert_eq!(
        reopened.source_state(b"source-a").unwrap(),
        Some(SourceState {
            identity: b"source-a-identity".to_vec(),
            offset: 20,
            parser_state: b"state-20".to_vec(),
            complete: true,
            usage_cutoff: Some("2026-01-01".into()),
            scope_indexed: true,
            execution_indexed: true,
        })
    );
    assert_eq!(reopened.source_state(b"unknown").unwrap(), None);
    assert_eq!(reopened.revision().unwrap(), 2);
    assert_eq!(daily(&reopened), (10, Some(2), 1));
    let mut stale = batch("source-a", 0, 20, vec![observation(5, 1, 10, Some(2))]);
    assert!(reopened.commit_batch(&mut stale, resolve).is_err());
    assert_eq!(stale.observations.len(), 1);
    assert_eq!(source(&reopened, "source-a").0, 20);
    assert_eq!(daily(&reopened), (10, Some(2), 1));
}

#[test]
fn failed_resolution_rolls_back_events_daily_and_cursor_before_retry() {
    let directory = tempfile::tempdir().unwrap();
    let mut index = PersistentIndex::open(&directory.path().join("usage.sqlite")).unwrap();
    index
        .commit_batch(
            &mut batch("source-a", 0, 20, vec![observation(5, 1, 10, Some(2))]),
            resolve,
        )
        .unwrap();
    let mut pending = batch("source-a", 20, 30, vec![observation(25, 2, 20, None)]);
    assert!(
        index
            .commit_batch(&mut pending, |_, _| anyhow::bail!("synthetic failure"))
            .is_err()
    );
    assert_eq!(pending.observations.len(), 1);
    assert_eq!(source(&index, "source-a").0, 20);
    assert_eq!(daily(&index), (10, Some(2), 1));
    assert_eq!(index.revision().unwrap(), 1);
    let observations: usize = index
        .db
        .query_row("SELECT COUNT(*) FROM usage_observations", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(observations, 1);
    assert_eq!(index.commit_batch(&mut pending, resolve).unwrap(), 2);
    assert_eq!(source(&index, "source-a").0, 30);
    assert_eq!(daily(&index), (30, None, 2));
}

#[test]
fn source_cutoff_tracks_only_committed_consumption_and_replacement() {
    let directory = tempfile::tempdir().unwrap();
    let mut index = PersistentIndex::open(&directory.path().join("usage.sqlite")).unwrap();
    let mut first = batch("source-a", 0, 20, vec![observation(5, 1, 10, Some(2))]);
    first.usage_cutoff = chrono::NaiveDate::from_ymd_opt(2026, 1, 10).unwrap();
    index.commit_batch(&mut first, resolve).unwrap();

    let mut empty = batch("source-a", 20, 20, vec![]);
    empty.usage_cutoff = chrono::NaiveDate::from_ymd_opt(2026, 1, 20).unwrap();
    index.commit_batch(&mut empty, resolve).unwrap();
    assert_eq!(
        index
            .source_state(b"source-a")
            .unwrap()
            .unwrap()
            .usage_cutoff,
        Some("2026-01-10".into())
    );

    let mut older = batch("source-a", 20, 30, vec![observation(25, 2, 20, Some(3))]);
    older.usage_cutoff = chrono::NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
    index.commit_batch(&mut older, resolve).unwrap();
    assert_eq!(
        index
            .source_state(b"source-a")
            .unwrap()
            .unwrap()
            .usage_cutoff,
        Some("2026-01-10".into())
    );

    let mut later = batch("source-a", 30, 40, vec![observation(35, 3, 30, Some(4))]);
    later.usage_cutoff = chrono::NaiveDate::from_ymd_opt(2026, 1, 20).unwrap();
    assert!(
        index
            .commit_batch(&mut later, |_, _| anyhow::bail!("synthetic failure"))
            .is_err()
    );
    let state = index.source_state(b"source-a").unwrap().unwrap();
    assert_eq!(state.offset, 30);
    assert_eq!(state.usage_cutoff.as_deref(), Some("2026-01-10"));
    assert_eq!(daily(&index), (30, Some(5), 2));

    index.commit_batch(&mut later, resolve).unwrap();
    let state = index.source_state(b"source-a").unwrap().unwrap();
    assert_eq!(state.offset, 40);
    assert_eq!(state.usage_cutoff.as_deref(), Some("2026-01-20"));

    let mut replacement = batch("source-a", 40, 10, vec![observation(5, 4, 7, None)]);
    replacement.replace = true;
    replacement.identity = b"new-identity".to_vec();
    replacement.usage_cutoff = chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    index.commit_batch(&mut replacement, resolve).unwrap();
    let state = index.source_state(b"source-a").unwrap().unwrap();
    assert_eq!(state.offset, 10);
    assert_eq!(state.usage_cutoff.as_deref(), Some("2026-01-01"));
    assert_eq!(daily(&index), (7, None, 1));
}

#[test]
fn v4_cutoff_remains_unknown_until_source_is_replaced() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("usage.sqlite");
    let mut index = PersistentIndex::open(&path).unwrap();
    index
        .commit_batch(
            &mut batch("source-a", 0, 20, vec![observation(5, 1, 10, Some(2))]),
            resolve,
        )
        .unwrap();
    index
        .db
        .execute_batch(
            "ALTER TABLE usage_sources DROP COLUMN usage_cutoff;
             PRAGMA user_version = 4;",
        )
        .unwrap();
    drop(index);

    let mut index = PersistentIndex::open(&path).unwrap();
    assert_eq!(daily(&index), (10, Some(2), 1));
    assert_eq!(
        index
            .source_state(b"source-a")
            .unwrap()
            .unwrap()
            .usage_cutoff,
        None
    );
    index
        .commit_batch(
            &mut batch("source-a", 20, 30, vec![observation(25, 2, 20, Some(3))]),
            resolve,
        )
        .unwrap();
    assert_eq!(
        index
            .source_state(b"source-a")
            .unwrap()
            .unwrap()
            .usage_cutoff,
        None
    );

    let mut replacement = batch("source-a", 30, 10, vec![observation(5, 3, 7, None)]);
    replacement.replace = true;
    index.commit_batch(&mut replacement, resolve).unwrap();
    assert_eq!(
        index
            .source_state(b"source-a")
            .unwrap()
            .unwrap()
            .usage_cutoff,
        Some("2026-01-01".into())
    );
}

#[test]
fn full_index_preserves_the_committed_cursor_and_accepts_a_smaller_retry() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("usage.sqlite");
    let budget = 1024 * 1024;
    let mut index = PersistentIndex::open_with_budget(&path, budget).unwrap();
    index
        .commit_batch(
            &mut batch("source-a", 0, 20, vec![observation(5, 1, 10, Some(2))]),
            |tx, sessions| {
                let database = std::fs::metadata(&path)?.len();
                let journal = std::fs::metadata(path.with_file_name("usage.sqlite-journal"))
                    .map(|metadata| metadata.len())
                    .unwrap_or(0);
                ensure!(database + journal <= budget, "working budget exceeded");
                resolve(tx, sessions)
            },
        )
        .unwrap();

    let mut large = observation(25, 2, 20, None);
    large.payload = vec![b'x'; budget as usize];
    let mut pending = batch("source-a", 20, 30, vec![large]);
    assert!(index.commit_batch(&mut pending, resolve).is_err());
    assert_eq!(pending.observations.len(), 1);
    assert_eq!(source(&index, "source-a").0, 20);
    assert_eq!(daily(&index), (10, Some(2), 1));
    drop(index);

    let mut reopened = PersistentIndex::open_with_budget(&path, budget).unwrap();
    assert_eq!(source(&reopened, "source-a").0, 20);
    assert_eq!(daily(&reopened), (10, Some(2), 1));
    assert_eq!(reopened.revision().unwrap(), 1);
    pending.observations[0] = observation(25, 2, 20, None);
    reopened.commit_batch(&mut pending, resolve).unwrap();
    assert_eq!(source(&reopened, "source-a").0, 30);
    assert_eq!(daily(&reopened), (30, None, 2));
}

#[test]
fn oversized_existing_index_keeps_committed_usage_readable_and_pauses_writes() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("usage.sqlite");
    let budget = 1024 * 1024;
    let mut index = PersistentIndex::open_with_budget(&path, budget).unwrap();
    index
        .commit_batch(
            &mut batch("source-a", 0, 20, vec![observation(5, 1, 10, Some(2))]),
            resolve,
        )
        .unwrap();
    let pages: u64 = index
        .db
        .pragma_query_value(None, "page_count", |row| row.get(0))
        .unwrap();
    let page_size: u64 = index
        .db
        .pragma_query_value(None, "page_size", |row| row.get(0))
        .unwrap();
    assert!(pages > 1);
    drop(index);

    let mut read_only =
        PersistentIndex::open_with_budget(&path, (pages - 1) * page_size * 3).unwrap();
    assert!(read_only.budget_blocked);
    assert!(
        read_only
            .db
            .is_readonly(rusqlite::DatabaseName::Main)
            .unwrap()
    );
    assert_eq!(source(&read_only, "source-a").0, 20);
    assert_eq!(daily(&read_only), (10, Some(2), 1));
    let request = json!({});
    let options = super::super::Query::parse(
        &request,
        chrono::NaiveDate::from_ymd_opt(2026, 11, 10).unwrap(),
    )
    .unwrap();
    let result =
        super::super::history_api::query(&mut read_only.db, &options, &request, &BTreeMap::new())
            .unwrap();
    assert_eq!(result["days"][0]["usage"]["inputTokens"], 10);
    assert_eq!(result["revision"], 1);

    let mut pending = batch("source-a", 20, 30, vec![observation(25, 2, 20, None)]);
    let error = read_only.commit_batch(&mut pending, resolve).unwrap_err();
    assert!(format!("{error:#}").contains("ingestion is paused"));
    assert_eq!(pending.observations.len(), 1);
    assert_eq!(source(&read_only, "source-a").0, 20);
    assert_eq!(read_only.revision().unwrap(), 1);
    drop(read_only);

    let mut resumed = PersistentIndex::open_with_budget(&path, budget).unwrap();
    assert!(!resumed.budget_blocked);
    resumed.commit_batch(&mut pending, resolve).unwrap();
    assert_eq!(source(&resumed, "source-a").0, 30);
    assert_eq!(daily(&resumed), (30, None, 2));
}

#[test]
fn database_and_journal_remain_within_budget_during_a_spilled_transaction() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("usage.sqlite");
    let budget = 2 * 1024 * 1024;
    let mut index = PersistentIndex::open_with_budget(&path, budget).unwrap();
    index.db.pragma_update(None, "cache_size", 8).unwrap();
    let observations = (0..32)
        .map(|id| {
            let mut item = observation(u64::from(id) + 1, id, u64::from(id) + 1, None);
            item.payload = serde_json::to_vec(&json!({
                "input": u64::from(id) + 1,
                "cached": null,
                "padding": "x".repeat(8192)
            }))
            .unwrap();
            item
        })
        .collect();
    index
        .commit_batch(
            &mut batch("source-a", 0, 40, observations),
            |tx, sessions| {
                let mut bytes = 0;
                for path in [
                    path.clone(),
                    path.with_file_name("usage.sqlite-journal"),
                    path.with_file_name("usage.sqlite-wal"),
                ] {
                    match std::fs::metadata(path) {
                        Ok(metadata) => bytes += metadata.len(),
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) => return Err(error.into()),
                    }
                }
                ensure!(
                    bytes <= budget,
                    "working files used {bytes} of {budget} bytes"
                );
                resolve(tx, sessions)
            },
        )
        .unwrap();
    assert_eq!(source(&index, "source-a").0, 40);
    assert_eq!(daily(&index), (528, None, 32));
}

#[test]
fn stores_and_resumes_more_than_the_former_event_limit() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("usage.sqlite");
    let mut index = PersistentIndex::open(&path).unwrap();
    let count = 100_001u64;
    let observations = (0..count)
        .map(|position| {
            let mut id = [0; 32];
            id[..8].copy_from_slice(&position.to_be_bytes());
            Observation {
                position,
                event_id: id,
                session: "session-a".into(),
                payload: b"{\"input\":1,\"cached\":0}".to_vec(),
            }
        })
        .collect();
    let mut first = batch("source-a", 0, count, observations);
    index.commit_batch(&mut first, resolve).unwrap();
    assert_eq!(daily(&index), (count as i64, Some(0), count as i64));
    assert!(first.observations.is_empty());
    drop(index);

    let mut reopened = PersistentIndex::open(&path).unwrap();
    assert_eq!(source(&reopened, "source-a").0, count as i64);
    let mut extra_id = [0; 32];
    extra_id[..8].copy_from_slice(&count.to_be_bytes());
    let extra = Observation {
        position: count,
        event_id: extra_id,
        session: "session-a".into(),
        payload: b"{\"input\":2,\"cached\":1}".to_vec(),
    };
    let mut next = batch("source-a", count, count + 1, vec![extra]);
    reopened.commit_batch(&mut next, resolve).unwrap();
    assert_eq!(
        daily(&reopened),
        (count as i64 + 2, Some(1), count as i64 + 1)
    );
}

#[test]
fn replacing_a_source_recomputes_the_whole_affected_session() {
    let directory = tempfile::tempdir().unwrap();
    let mut index = PersistentIndex::open(&directory.path().join("usage.sqlite")).unwrap();
    for source in ["source-a", "source-b"] {
        index
            .commit_batch(
                &mut batch(source, 0, 20, vec![observation(5, 1, 10, Some(2))]),
                resolve,
            )
            .unwrap();
    }
    let mut replaced = batch("source-a", 20, 12, vec![observation(4, 2, 20, Some(3))]);
    replaced.replace = true;
    replaced.identity = b"replacement".to_vec();
    index.commit_batch(&mut replaced, resolve).unwrap();
    assert_eq!(daily(&index), (30, Some(5), 2));
    let mut emptied = batch("source-b", 20, 0, vec![]);
    emptied.replace = true;
    emptied.identity = b"empty-replacement".to_vec();
    index.commit_batch(&mut emptied, resolve).unwrap();
    assert_eq!(daily(&index), (20, Some(3), 1));
}

#[test]
fn duplicate_identity_conflict_and_invalid_components_do_not_advance_cursor() {
    let directory = tempfile::tempdir().unwrap();
    let mut index = PersistentIndex::open(&directory.path().join("usage.sqlite")).unwrap();
    index
        .commit_batch(
            &mut batch("source-a", 0, 20, vec![observation(5, 1, 10, Some(2))]),
            resolve,
        )
        .unwrap();
    let mut conflict = batch("source-b", 0, 20, vec![observation(5, 1, 30, Some(2))]);
    assert!(index.commit_batch(&mut conflict, resolve).is_err());
    assert_eq!(conflict.observations.len(), 1);
    let source_count: usize = index
        .db
        .query_row("SELECT COUNT(*) FROM usage_sources", [], |row| row.get(0))
        .unwrap();
    assert_eq!(source_count, 1);
    let mut invalid = batch("source-a", 20, 30, vec![observation(25, 2, 5, Some(6))]);
    assert!(index.commit_batch(&mut invalid, resolve).is_err());
    assert_eq!(source(&index, "source-a").0, 20);
    assert_eq!(daily(&index), (10, Some(2), 1));
}

#[test]
fn unknown_provider_and_count_conflicts_preserve_confirmed_totals_and_partial_status() {
    let directory = tempfile::tempdir().unwrap();
    let mut index = PersistentIndex::open(&directory.path().join("usage.sqlite")).unwrap();
    let mut pending = batch(
        "source-a",
        0,
        20,
        vec![
            observation(5, 1, 10, Some(2)),
            observation(10, 2, 20, Some(3)),
            observation(15, 3, 30, Some(4)),
        ],
    );
    index
        .commit_batch(&mut pending, |tx, sessions| {
            let mut events = resolve(tx, sessions)?;
            for event in &mut events {
                event.provider.clear();
                event.provider_source = None;
                event.count_uncertain = event.id[0] == 2;
                event.count_incomplete = event.id[0] == 3;
            }
            Ok(events)
        })
        .unwrap();
    let row: (String, i64, Option<i64>, i64, i64) = index
        .db
        .query_row(
            "SELECT provider, input, cached, records, uncertain_events FROM usage_daily",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(row, (String::new(), 10, Some(2), 1, 2));
}

#[test]
fn daily_summary_matches_event_query_for_partial_components_and_conflicts() {
    let directory = tempfile::tempdir().unwrap();
    let mut index = PersistentIndex::open(&directory.path().join("usage.sqlite")).unwrap();
    let observations = (1..=5)
        .map(|id| observation(u64::from(id), id, 10, Some(2)))
        .collect();
    index
        .commit_batch(
            &mut batch("source-a", 0, 20, observations),
            |tx, sessions| {
                let mut events = resolve(tx, sessions)?;
                for event in &mut events {
                    match event.id[0] {
                        1 => event.reasoning = Some(1),
                        2 => {
                            event.cached = None;
                            event.reasoning = None;
                        }
                        3 => event.context_conflicts = 1,
                        4 => event.count_uncertain = true,
                        5 => {
                            event.input = 0;
                            event.cached = Some(0);
                            event.output = 0;
                            event.context_conflicts = 2;
                        }
                        _ => unreachable!(),
                    }
                }
                Ok(events)
            },
        )
        .unwrap();
    let today = chrono::NaiveDate::from_ymd_opt(2026, 11, 10).unwrap();
    let exact_request = json!({"startAt":0,"endAt":2000});
    let exact_options = super::super::Query::parse(&exact_request, today).unwrap();
    let exact = super::super::history_api::query(
        &mut index.db,
        &exact_options,
        &exact_request,
        &BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(exact["days"][0]["usage"]["inputTokens"], 30);
    assert_eq!(exact["days"][0]["usage"]["cachedInputTokens"], 4);
    assert_eq!(exact["days"][0]["usage"]["cachedKnown"], false);
    assert_eq!(exact["days"][0]["recordCount"], 3);
    assert_eq!(exact["countStatus"]["uncertainEvents"], 1);
    assert_eq!(exact["classificationStatus"]["uncertainEvents"], 2);

    // 全日查询应只依赖已提交日汇总，不能重新扫描逐次事件。
    index
        .db
        .execute_batch("ALTER TABLE usage_events RENAME TO usage_events_unavailable;")
        .unwrap();
    let request = json!({});
    let options = super::super::Query::parse(&request, today).unwrap();
    let summary =
        super::super::history_api::query(&mut index.db, &options, &request, &BTreeMap::new())
            .unwrap();
    assert_eq!(summary["days"], exact["days"]);
    assert_eq!(summary["countStatus"], exact["countStatus"]);
    assert_eq!(
        summary["classificationStatus"],
        exact["classificationStatus"]
    );
}

#[test]
fn daily_summary_preserves_scope_filters_and_provider_groups() {
    const PARENT: &str = "00000000-0000-4000-8000-000000000001";
    const CHILD: &str = "00000000-0000-4000-8000-000000000002";
    const OTHER: &str = "00000000-0000-4000-8000-000000000003";
    let directory = tempfile::tempdir().unwrap();
    let mut index = PersistentIndex::open(&directory.path().join("usage.sqlite")).unwrap();
    for (source, session, parent, first) in [
        ("source-a", PARENT, None, 1),
        ("source-b", CHILD, Some(PARENT), 4),
        ("source-c", OTHER, None, 7),
    ] {
        let observations = (first..first + 3)
            .map(|id| {
                let mut event = observation(u64::from(id - first + 1), id, 10, Some(2));
                event.session = session.into();
                event
            })
            .collect();
        let mut pending = batch(source, 0, 20, observations);
        pending.scope_session = session.into();
        pending.scope_parents = vec![(session.into(), parent.map(str::to_owned))];
        index
            .commit_batch(&mut pending, |tx, sessions| {
                let mut events = resolve(tx, sessions)?;
                for event in &mut events {
                    let id = event.id[0];
                    if id % 2 == 0 {
                        event.day = "2026-11-09".into();
                        event.model = "model-b".into();
                        event.provider = "provider-b".into();
                    }
                    if id % 3 == 0 {
                        event.cached = None;
                        event.reasoning = None;
                        event.context_conflicts = 2;
                    } else {
                        event.reasoning = Some(1);
                    }
                    if id % 3 == 1 {
                        event.provider_source = Some("request".into());
                    }
                    if id == 8 {
                        event.count_incomplete = true;
                    }
                }
                Ok(events)
            })
            .unwrap();
    }
    let today = chrono::NaiveDate::from_ymd_opt(2026, 11, 10).unwrap();
    for request in [
        json!({"scopeSessionId":PARENT}),
        json!({"period":"today","scopeSessionId":PARENT}),
        json!({"from":"2026-11-09","to":"2026-11-09"}),
        json!({"model":"model-b","scopeSessionId":PARENT}),
        json!({"search":"model-a"}),
        json!({"search":"2026-11-09"}),
        json!({"sessionId":CHILD}),
    ] {
        let mut exact_request = request.clone();
        exact_request["startAt"] = json!(0);
        exact_request["endAt"] = json!(2000);
        let exact_options = super::super::Query::parse(&exact_request, today).unwrap();
        let summary_options = super::super::Query::parse(&request, today).unwrap();
        let exact = super::super::history_api::query(
            &mut index.db,
            &exact_options,
            &exact_request,
            &BTreeMap::new(),
        )
        .unwrap();
        let summary = super::super::history_api::query(
            &mut index.db,
            &summary_options,
            &request,
            &BTreeMap::new(),
        )
        .unwrap();
        for field in ["days", "countStatus", "classificationStatus", "scopedUsage"] {
            assert_eq!(summary[field], exact[field], "{field}, request {request}");
        }
    }
}

#[test]
fn aligned_time_bounds_use_daily_summary_but_partial_days_remain_exact() {
    use chrono::TimeZone;

    let directory = tempfile::tempdir().unwrap();
    let mut index = PersistentIndex::open(&directory.path().join("usage.sqlite")).unwrap();
    let start = chrono::Local
        .with_ymd_and_hms(2026, 11, 10, 0, 0, 0)
        .single()
        .unwrap()
        .timestamp_millis();
    let end = chrono::Local
        .with_ymd_and_hms(2026, 11, 11, 0, 0, 0)
        .single()
        .unwrap()
        .timestamp_millis();
    index
        .commit_batch(
            &mut batch("source-a", 0, 20, vec![observation(5, 1, 10, Some(2))]),
            |tx, sessions| {
                let mut events = resolve(tx, sessions)?;
                events[0].timestamp_ms = start + 1_000;
                Ok(events)
            },
        )
        .unwrap();
    let today = chrono::NaiveDate::from_ymd_opt(2026, 11, 10).unwrap();
    let partial_request = json!({"startAt":start + 1,"endAt":end});
    let partial_options = super::super::Query::parse(&partial_request, today).unwrap();
    let exact = super::super::history_api::query(
        &mut index.db,
        &partial_options,
        &partial_request,
        &BTreeMap::new(),
    )
    .unwrap();
    index
        .db
        .execute_batch("ALTER TABLE usage_events RENAME TO usage_events_unavailable;")
        .unwrap();
    let request = json!({"startAt":start,"endAt":end});
    let options = super::super::Query::parse(&request, today).unwrap();
    let summary =
        super::super::history_api::query(&mut index.db, &options, &request, &BTreeMap::new())
            .unwrap();
    assert_eq!(summary["days"], exact["days"]);
    assert_eq!(summary["rangeApplied"], true);
    assert!(
        super::super::history_api::query(
            &mut index.db,
            &partial_options,
            &partial_request,
            &BTreeMap::new(),
        )
        .is_err()
    );
}

#[test]
fn previous_daily_schema_rebuilds_from_committed_events() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("usage.sqlite");
    let mut index = PersistentIndex::open(&path).unwrap();
    index
        .commit_batch(
            &mut batch("source-a", 0, 20, vec![observation(5, 1, 10, Some(2))]),
            resolve,
        )
        .unwrap();
    index
        .db
        .execute_batch(
            "DROP TABLE usage_daily;
             CREATE TABLE usage_daily(day TEXT, session TEXT, model TEXT, provider TEXT,
                 provider_source TEXT, role TEXT, input INTEGER, cached INTEGER,
                 output INTEGER, reasoning INTEGER, records INTEGER, uncertain_events INTEGER);
             INSERT INTO usage_daily VALUES ('2026-11-10', 'session-a', 'old', 'old', '',
                 'main', 999, NULL, 0, NULL, 1, 0);
             PRAGMA user_version = 5;",
        )
        .unwrap();
    drop(index);

    let index = PersistentIndex::open(&path).unwrap();
    assert_eq!(daily(&index), (10, Some(2), 1));
    assert_eq!(source(&index, "source-a").0, 20);
    assert_eq!(index.revision().unwrap(), 1);
    let version: i64 = index
        .db
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, SCHEMA_VERSION);
}

#[test]
fn expiry_index_migration_preserves_committed_data() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("usage.sqlite");
    let mut index = PersistentIndex::open(&path).unwrap();
    index
        .commit_batch(
            &mut batch("source-a", 0, 20, vec![observation(5, 1, 10, Some(2))]),
            resolve,
        )
        .unwrap();
    index
        .db
        .execute_batch("DROP INDEX executions_detail_expiry; PRAGMA user_version = 6;")
        .unwrap();
    drop(index);

    let index = PersistentIndex::open(&path).unwrap();
    let migrated: bool = index
        .db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master
             WHERE type = 'index' AND name = 'executions_detail_expiry')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(migrated);
    assert_eq!(daily(&index), (10, Some(2), 1));
    assert_eq!(source(&index, "source-a").0, 20);
    assert_eq!(index.revision().unwrap(), 1);
}

#[test]
fn oversized_previous_schema_still_allows_committed_reads() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("usage.sqlite");
    let mut index = PersistentIndex::open(&path).unwrap();
    index
        .commit_batch(
            &mut batch("source-a", 0, 20, vec![observation(5, 1, 10, Some(2))]),
            resolve,
        )
        .unwrap();
    index.db.pragma_update(None, "user_version", 5).unwrap();
    let pages: u64 = index
        .db
        .pragma_query_value(None, "page_count", |row| row.get(0))
        .unwrap();
    let page_size: u64 = index
        .db
        .pragma_query_value(None, "page_size", |row| row.get(0))
        .unwrap();
    drop(index);

    let mut read_only =
        PersistentIndex::open_with_budget(&path, (pages - 1) * page_size * 3).unwrap();
    assert!(read_only.budget_blocked);
    let request = json!({});
    let options = super::super::Query::parse(
        &request,
        chrono::NaiveDate::from_ymd_opt(2026, 11, 10).unwrap(),
    )
    .unwrap();
    let result =
        super::super::history_api::query(&mut read_only.db, &options, &request, &BTreeMap::new())
            .unwrap();
    assert_eq!(result["days"][0]["usage"]["inputTokens"], 10);
    assert_eq!(result["revision"], 1);
}

#[test]
fn full_daily_migration_keeps_previous_schema_readable_until_retry() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("usage.sqlite");
    let mut index = PersistentIndex::open(&path).unwrap();
    index
        .commit_batch(
            &mut batch("source-a", 0, 20, vec![observation(5, 1, 10, Some(2))]),
            resolve,
        )
        .unwrap();
    index
        .db
        .execute_batch(
            "WITH RECURSIVE sequence(n) AS (
                 VALUES(1) UNION ALL SELECT n + 1 FROM sequence WHERE n < 4096
             )
             INSERT INTO usage_events
             SELECT CAST(printf('migration-%08d', n) AS BLOB),
                    'session-a', '2026-11-10', n, printf('model-%04d', (n - 1) / 4),
                    'provider-a', NULL, 'main', (n - 1) % 4, 0, 0, 10, 2, 1, 0
             FROM sequence;
             DROP TABLE usage_daily;
             CREATE TABLE usage_daily (
                 day TEXT NOT NULL, session TEXT NOT NULL, model TEXT NOT NULL,
                 provider TEXT NOT NULL, provider_source TEXT NOT NULL, role TEXT NOT NULL,
                 input INTEGER NOT NULL, cached INTEGER, output INTEGER NOT NULL,
                 reasoning INTEGER, records INTEGER NOT NULL, uncertain_events INTEGER NOT NULL,
                 PRIMARY KEY(day, session, model, provider, provider_source, role)
             ) WITHOUT ROWID;
             INSERT INTO usage_daily
             SELECT day, session, model, provider, COALESCE(provider_source, ''), role,
                    SUM(input), SUM(cached), SUM(output), SUM(reasoning), COUNT(*), 0
             FROM usage_events GROUP BY day, session, model, provider, provider_source, role;
             PRAGMA user_version = 5;",
        )
        .unwrap();
    let pages: u64 = index
        .db
        .pragma_query_value(None, "page_count", |row| row.get(0))
        .unwrap();
    let page_size: u64 = index
        .db
        .pragma_query_value(None, "page_size", |row| row.get(0))
        .unwrap();
    drop(index);

    let mut read_only = PersistentIndex::open_with_budget(&path, pages * page_size * 3).unwrap();
    assert!(read_only.budget_blocked);
    assert_eq!(
        read_only.source_state(b"source-a").unwrap().unwrap().offset,
        20
    );
    assert_eq!(read_only.revision().unwrap(), 1);
    let version: i64 = read_only
        .db
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 5);
    let request = json!({});
    let options = super::super::Query::parse(
        &request,
        chrono::NaiveDate::from_ymd_opt(2026, 11, 10).unwrap(),
    )
    .unwrap();
    let result =
        super::super::history_api::query(&mut read_only.db, &options, &request, &BTreeMap::new())
            .unwrap();
    assert_eq!(result["days"][0]["usage"]["inputTokens"], 40_970);
    drop(read_only);

    let mut reopened = PersistentIndex::open(&path).unwrap();
    assert!(!reopened.budget_blocked);
    let version: i64 = reopened
        .db
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, SCHEMA_VERSION);
    let recovered =
        super::super::history_api::query(&mut reopened.db, &options, &request, &BTreeMap::new())
            .unwrap();
    assert_eq!(recovered["days"], result["days"]);
    assert_eq!(recovered["countStatus"], result["countStatus"]);
    assert_eq!(
        recovered["classificationStatus"],
        result["classificationStatus"]
    );
}

#[test]
fn full_execution_session_index_migration_keeps_previous_schema_readable() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("usage.sqlite");
    let mut index = PersistentIndex::open(&path).unwrap();
    index
        .commit_batch(
            &mut batch("source-a", 0, 20, vec![observation(5, 1, 10, Some(2))]),
            resolve,
        )
        .unwrap();
    index
        .db
        .execute_batch(
            "DROP INDEX execution_daily_session;
             WITH RECURSIVE sequence(n) AS (
                 VALUES(1) UNION ALL SELECT n + 1 FROM sequence WHERE n < 4096
             )
             INSERT INTO execution_daily(day, session, count, partial)
             SELECT '2026-01-01', printf('session-%06d', n), 1, 0 FROM sequence;
             PRAGMA user_version = 7;",
        )
        .unwrap();
    let pages: u64 = index
        .db
        .pragma_query_value(None, "page_count", |row| row.get(0))
        .unwrap();
    let page_size: u64 = index
        .db
        .pragma_query_value(None, "page_size", |row| row.get(0))
        .unwrap();
    drop(index);

    let read_only = PersistentIndex::open_with_budget(&path, pages * page_size * 3).unwrap();
    assert!(read_only.budget_blocked);
    assert_eq!(
        read_only
            .db
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        7
    );
    assert_eq!(source(&read_only, "source-a").0, 20);
    assert_eq!(daily(&read_only), (10, Some(2), 1));
    assert_eq!(
        read_only
            .db
            .query_row("SELECT COUNT(*) FROM execution_daily", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        4096
    );
    drop(read_only);

    let reopened = PersistentIndex::open(&path).unwrap();
    assert!(!reopened.budget_blocked);
    let plan: String = reopened
        .db
        .query_row(
            "EXPLAIN QUERY PLAN SELECT 1 FROM execution_daily WHERE session = ?1",
            ["session-000001"],
            |row| row.get(3),
        )
        .unwrap();
    assert!(plan.contains("execution_daily_session"), "{plan}");
    assert_eq!(daily(&reopened), (10, Some(2), 1));
    assert_eq!(source(&reopened, "source-a").0, 20);
}

#[test]
fn daily_migration_reports_non_capacity_errors() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("usage.sqlite");
    let index = PersistentIndex::open(&path).unwrap();
    index
        .db
        .execute_batch("DROP TABLE usage_daily; PRAGMA user_version = 5;")
        .unwrap();
    drop(index);

    let error = match PersistentIndex::open(&path) {
        Ok(_) => panic!("missing daily table must not be treated as a capacity failure"),
        Err(error) => error,
    };
    assert!(format!("{error:#}").contains("no such table"));
}

#[test]
fn tool_classification_migration_keeps_counts_and_marks_legacy_classification_unknown() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("usage.sqlite");
    let mut index = PersistentIndex::open(&path).unwrap();
    index.commit_batch(
        &mut batch("source-a", 0, 20, vec![observation(5, 1, 10, Some(2))]), resolve,
    ).unwrap();
    index.db.execute_batch(
        "ALTER TABLE execution_daily DROP COLUMN skill;
         ALTER TABLE execution_daily DROP COLUMN mcp;
         ALTER TABLE execution_daily DROP COLUMN tool;
         INSERT INTO execution_daily(day, session, count, partial) VALUES('2026-01-01', 'session', 4, 0);
         PRAGMA user_version = 10;",
    ).unwrap();
    drop(index);
    let reopened = PersistentIndex::open(&path).unwrap();
    assert_eq!(source(&reopened, "source-a").0, 20);
    assert_eq!(daily(&reopened), (10, Some(2), 1));
    let day = chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let summary = execution_index::daily_summary(&reopened.db, day, day.succ_opt().unwrap(), None).unwrap();
    assert_eq!(summary[0].count, 4);
    assert!(!summary[0].classification_complete);
    assert_eq!(summary[0].skill + summary[0].mcp + summary[0].tool, 0);
}

#[test]
fn nearly_full_classification_migration_keeps_maintenance_and_ingestion_available() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("usage.sqlite");
    let mut index = PersistentIndex::open(&path).unwrap();
    index.commit_batch(
        &mut batch("source-a", 0, 20, vec![observation(5, 1, 10, Some(2))]), resolve,
    ).unwrap();
    index.db.execute_batch(
        "ALTER TABLE execution_daily DROP COLUMN skill;
         ALTER TABLE execution_daily DROP COLUMN mcp;
         ALTER TABLE execution_daily DROP COLUMN tool;
         WITH RECURSIVE sequence(n) AS (
             VALUES(1) UNION ALL SELECT n + 1 FROM sequence WHERE n < 8192
         )
         INSERT INTO execution_observations
             (source, start, session, call_id, timestamp_ms, is_output)
         SELECT CAST('source-a' AS BLOB), n, 'session-a', printf('old-call-%05d', n), n, 0
         FROM sequence;
         PRAGMA user_version = 10;",
    ).unwrap();
    reclaim_free_pages(&index.db).unwrap();
    let pages: u64 = index.db.pragma_query_value(None, "page_count", |row| row.get(0)).unwrap();
    let page_size: u64 = index.db.pragma_query_value(None, "page_size", |row| row.get(0)).unwrap();
    let budget = (pages + 13) * page_size * 3;
    let before_revision = index.revision().unwrap();
    assert_eq!(
        index.db.pragma_query_value(None, "freelist_count", |row| row.get::<_, u64>(0)).unwrap(),
        0
    );
    drop(index);

    let mut reopened = PersistentIndex::open_with_budget(&path, budget).unwrap();
    assert!(!reopened.budget_blocked);
    assert_eq!(source(&reopened, "source-a").0, 20);
    assert_eq!(daily(&reopened), (10, Some(2), 1));
    assert_eq!(reopened.revision().unwrap(), before_revision);
    assert_eq!(
        reopened.db.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0)).unwrap(),
        SCHEMA_VERSION
    );
    let mut cursor = None;
    loop {
        let compacted = reopened.compact_execution_times(1_800_000_000_000, cursor.as_ref(), false).unwrap();
        cursor = compacted.next;
        if cursor.is_none() { break; }
    }
    let total: i64 = reopened.db.query_row(
        "SELECT (SELECT COUNT(*) FROM execution_observations WHERE is_output = 0)
             + COALESCE((SELECT SUM(count) FROM execution_daily), 0)",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(total, 8192);
    reclaim_free_pages(&reopened.db).unwrap();
    assert!(
        reopened.db.pragma_query_value(None, "page_count", |row| row.get::<_, u64>(0)).unwrap() < pages
    );
    reopened.commit_batch(
        &mut batch("source-a", 20, 30, vec![observation(25, 2, 20, Some(3))]), resolve,
    ).unwrap();
    assert_eq!(source(&reopened, "source-a").0, 30);
    assert_eq!(daily(&reopened), (30, Some(5), 2));
    assert!(std::fs::metadata(path).unwrap().len() <= budget);
}

#[test]
fn appended_daily_summary_matches_full_rebuild_without_deleting_prior_rows() {
    fn shaped(
        tx: &Transaction<'_>,
        sessions: &BTreeSet<String>,
    ) -> anyhow::Result<Vec<ResolvedEvent>> {
        let mut events = resolve(tx, sessions)?;
        for event in &mut events {
            match event.id[0] {
                1 => event.reasoning = Some(1),
                2 => {
                    event.cached = None;
                    event.reasoning = None;
                }
                3 => event.count_uncertain = true,
                4 => {
                    event.input = 0;
                    event.cached = Some(0);
                    event.output = 0;
                    event.reasoning = Some(0);
                    event.context_conflicts = 1;
                }
                5 => {
                    event.model = "model-b".into();
                    event.provider = "provider-b".into();
                    event.provider_source = Some("request".into());
                    event.reasoning = Some(1);
                }
                6 => {
                    event.context_conflicts = 1;
                    event.reasoning = Some(1);
                }
                7 => event.reasoning = Some(1),
                _ => unreachable!(),
            }
        }
        Ok(events)
    }

    let directory = tempfile::tempdir().unwrap();
    let mut fast = PersistentIndex::open(&directory.path().join("fast.sqlite")).unwrap();
    let mut full = PersistentIndex::open(&directory.path().join("full.sqlite")).unwrap();
    for index in [&mut fast, &mut full] {
        index
            .commit_batch(
                &mut batch("source-a", 0, 20, vec![observation(5, 1, 10, Some(2))]),
                shaped,
            )
            .unwrap();
    }
    fast.db
        .execute_batch(
            "CREATE TRIGGER forbid_daily_rebuild BEFORE DELETE ON usage_daily
             BEGIN SELECT RAISE(ABORT, 'append rebuilt existing daily summary'); END;",
        )
        .unwrap();
    let request = json!({});
    let options = super::super::Query::parse(
        &request,
        chrono::NaiveDate::from_ymd_opt(2026, 11, 10).unwrap(),
    )
    .unwrap();
    for (expected, next, values) in [
        (20, 40, vec![(25, 2, 20, None), (30, 3, 30, Some(3))]),
        (40, 60, vec![(45, 4, 0, Some(0)), (50, 5, 50, Some(5))]),
        (60, 80, vec![(65, 6, 60, Some(6)), (70, 7, 70, Some(7))]),
    ] {
        let make_batch = || {
            batch(
                "source-a",
                expected,
                next,
                values
                    .iter()
                    .map(|&(position, id, input, cached)| observation(position, id, input, cached))
                    .collect(),
            )
        };
        fast.commit_batch_resolved(&mut make_batch(), |tx, sessions, pending| {
            let mut events = shaped(tx, sessions)?;
            events.retain(|event| {
                pending
                    .observations
                    .iter()
                    .any(|observation| observation.event_id == event.id)
            });
            Ok(Resolution::Append(events))
        })
        .unwrap();
        full.commit_batch(&mut make_batch(), shaped).unwrap();
        let actual =
            super::super::history_api::query(&mut fast.db, &options, &request, &BTreeMap::new())
                .unwrap();
        let expected =
            super::super::history_api::query(&mut full.db, &options, &request, &BTreeMap::new())
                .unwrap();
        for field in ["days", "countStatus", "classificationStatus"] {
            assert_eq!(actual[field], expected[field], "{field}");
        }
    }
}

#[test]
fn appended_daily_overflow_rolls_back_all_committed_state() {
    for (column, uncertain) in [
        ("input", false),
        ("cached", false),
        ("cached_known_sum", false),
        ("output", false),
        ("reasoning", false),
        ("records", false),
        ("uncertain_events", true),
        ("total_events", false),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let mut index = PersistentIndex::open(&directory.path().join("usage.sqlite")).unwrap();
        index
            .commit_batch(
                &mut batch("source-a", 0, 20, vec![observation(5, 1, 10, Some(2))]),
                |tx, sessions| {
                    let mut events = resolve(tx, sessions)?;
                    events[0].reasoning = Some(1);
                    Ok(events)
                },
            )
            .unwrap();
        index
            .db
            .execute(&format!("UPDATE usage_daily SET {column} = ?1"), [i64::MAX])
            .unwrap();
        let before: (i64, String) = index
            .db
            .query_row(
                &format!("SELECT {column}, typeof({column}) FROM usage_daily"),
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let mut pending = batch("source-a", 20, 30, vec![observation(25, 2, 20, Some(2))]);
        let error = index
            .commit_batch_resolved(&mut pending, |tx, sessions, _| {
                let mut events = resolve(tx, sessions)?;
                events.retain(|event| event.id == [2; 32]);
                events[0].reasoning = Some(1);
                events[0].count_uncertain = uncertain;
                Ok(Resolution::Append(events))
            })
            .unwrap_err();
        assert!(
            format!("{error:#}").contains("daily usage summary exceeds SQLite integer"),
            "{column}: {error:#}"
        );
        assert_eq!(pending.observations.len(), 1, "{column}");
        assert_eq!(source(&index, "source-a").0, 20, "{column}");
        assert_eq!(index.revision().unwrap(), 1, "{column}");
        let counts: (i64, i64) = index
            .db
            .query_row(
                "SELECT (SELECT COUNT(*) FROM usage_observations),
                        (SELECT COUNT(*) FROM usage_events)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(counts, (1, 1), "{column}");
        let after: (i64, String) = index
            .db
            .query_row(
                &format!("SELECT {column}, typeof({column}) FROM usage_daily"),
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(after, before, "{column}");
        assert_eq!(after.1, "integer", "{column}");
    }
}

#[test]
fn duplicate_appended_event_rolls_back_without_counting_it_twice() {
    let directory = tempfile::tempdir().unwrap();
    let mut index = PersistentIndex::open(&directory.path().join("usage.sqlite")).unwrap();
    index
        .commit_batch(
            &mut batch("source-a", 0, 20, vec![observation(5, 1, 10, Some(2))]),
            resolve,
        )
        .unwrap();
    let mut pending = batch("source-a", 20, 30, vec![observation(25, 1, 10, Some(2))]);
    assert!(
        index
            .commit_batch_resolved(&mut pending, |tx, sessions, _| {
                Ok(Resolution::Append(resolve(tx, sessions)?))
            })
            .is_err()
    );
    assert_eq!(pending.observations.len(), 1);
    assert_eq!(source(&index, "source-a").0, 20);
    assert_eq!(index.revision().unwrap(), 1);
    assert_eq!(daily(&index), (10, Some(2), 1));
}

#[test]
fn unchanged_events_and_unaffected_days_are_not_rewritten() {
    let directory = tempfile::tempdir().unwrap();
    let mut index = PersistentIndex::open(&directory.path().join("usage.sqlite")).unwrap();
    let resolve_days = |tx: &Transaction<'_>, sessions: &BTreeSet<String>| {
        let mut events = resolve(tx, sessions)?;
        for event in &mut events {
            if event.id[0] == 1 {
                event.day = "2026-11-09".into();
            }
        }
        Ok(events)
    };
    index
        .commit_batch(
            &mut batch(
                "source-a",
                0,
                20,
                vec![observation(5, 1, 10, Some(2)), observation(10, 2, 20, None)],
            ),
            resolve_days,
        )
        .unwrap();
    index
        .db
        .execute_batch(
            "CREATE TABLE writes(kind TEXT NOT NULL, value TEXT NOT NULL);
             CREATE TRIGGER event_insert AFTER INSERT ON usage_events BEGIN
               INSERT INTO writes VALUES('event-insert', hex(NEW.id));
             END;
             CREATE TRIGGER event_update AFTER UPDATE ON usage_events BEGIN
               INSERT INTO writes VALUES('event-update', hex(NEW.id));
             END;
             CREATE TRIGGER event_delete AFTER DELETE ON usage_events BEGIN
               INSERT INTO writes VALUES('event-delete', hex(OLD.id));
             END;
             CREATE TRIGGER daily_insert AFTER INSERT ON usage_daily BEGIN
               INSERT INTO writes VALUES('daily-insert', NEW.day);
             END;
             CREATE TRIGGER daily_delete AFTER DELETE ON usage_daily BEGIN
               INSERT INTO writes VALUES('daily-delete', OLD.day);
             END;",
        )
        .unwrap();
    index
        .commit_batch(
            &mut batch("source-a", 20, 30, vec![observation(25, 3, 30, Some(3))]),
            resolve_days,
        )
        .unwrap();
    let writes: Vec<(String, String)> = index
        .db
        .prepare("SELECT kind, value FROM writes ORDER BY kind")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        writes,
        vec![
            ("daily-delete".into(), "2026-11-10".into()),
            ("daily-insert".into(), "2026-11-10".into()),
            ("event-insert".into(), "03".repeat(32)),
        ]
    );
    index.db.execute("DELETE FROM writes", []).unwrap();
    index
        .commit_batch(
            &mut batch("source-b", 0, 20, vec![observation(5, 1, 10, Some(2))]),
            resolve_days,
        )
        .unwrap();
    let count: usize = index
        .db
        .query_row("SELECT COUNT(*) FROM writes", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn changed_event_rebuilds_both_old_and_new_days() {
    let directory = tempfile::tempdir().unwrap();
    let mut index = PersistentIndex::open(&directory.path().join("usage.sqlite")).unwrap();
    index
        .commit_batch(
            &mut batch("source-a", 0, 20, vec![observation(5, 1, 10, Some(2))]),
            resolve,
        )
        .unwrap();
    index
        .commit_batch(
            &mut batch("source-a", 20, 30, vec![observation(25, 2, 20, None)]),
            |tx, sessions| {
                let mut events = resolve(tx, sessions)?;
                events
                    .iter_mut()
                    .find(|event| event.id[0] == 1)
                    .unwrap()
                    .day = "2026-11-09".into();
                Ok(events)
            },
        )
        .unwrap();
    let rows: Vec<(String, i64, Option<i64>, i64)> = index
        .db
        .prepare("SELECT day, input, cached, records FROM usage_daily ORDER BY day")
        .unwrap()
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        rows,
        vec![
            ("2026-11-09".into(), 10, Some(2), 1),
            ("2026-11-10".into(), 20, None, 1),
        ]
    );
}

#[test]
fn previous_index_schema_migrates_without_losing_committed_usage() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("usage.sqlite");
    let mut index = PersistentIndex::open(&path).unwrap();
    index
        .commit_batch(
            &mut batch("source-a", 0, 20, vec![observation(5, 1, 10, Some(2))]),
            resolve,
        )
        .unwrap();
    index
        .db
        .execute_batch(
            "ALTER TABLE execution_observations DROP COLUMN process_id;
         ALTER TABLE usage_sources DROP COLUMN usage_cutoff;
         DROP TABLE execution_daily;
         DROP TABLE execution_compacted_sources;
         DROP TABLE usage_collision_families;
         DROP TABLE usage_count_gaps;
         DROP TABLE usage_unindexed_checkpoints;
         DROP INDEX usage_events_day_timestamp_session;
         PRAGMA user_version = 2;",
        )
        .unwrap();
    let state = checkpoint::save(&FileState {
        session: "session-a".into(),
        ..FileState::default()
    })
    .unwrap();
    index
        .db
        .execute(
            "UPDATE usage_sources SET parser_state = ?1 WHERE source = ?2",
            params![state, b"source-a"],
        )
        .unwrap();
    drop(index);

    let index = PersistentIndex::open(&path).unwrap();
    assert_eq!(daily(&index), (10, Some(2), 1));
    assert_eq!(source(&index, "source-a").0, 20);
    let version: i64 = index
        .db
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, SCHEMA_VERSION);
    let count: i64 = index.db.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('execution_observations') WHERE name = 'process_id'",
        [],
        |row| row.get(0),
    ).unwrap();
    assert_eq!(count, 1);
    let count: i64 = index
        .db
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN
            ('usage_collision_families', 'execution_daily', 'execution_compacted_sources')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 3);
}

#[test]
fn migration_restores_gap_scope_from_checkpoint_and_observations() {
    use crate::usage_history::structured::InheritedUsageContext;

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("usage.sqlite");
    let mut index = PersistentIndex::open(&path).unwrap();
    let mut source_batch = batch("source-a", 0, 20, vec![observation(5, 1, 10, Some(2))]);
    source_batch.parser_state = checkpoint::save(&FileState {
        session: "session-a".into(),
        pending_count_gap: true,
        inherited_contexts: vec![InheritedUsageContext {
            session: "session-parent".into(),
            ..InheritedUsageContext::default()
        }],
        ..FileState::default()
    })
    .unwrap();
    source_batch.count_gap_sessions = Some(BTreeSet::from([
        "session-a".into(),
        "session-parent".into(),
    ]));
    index.commit_batch(&mut source_batch, resolve).unwrap();
    index
        .db
        .execute_batch(
            "DROP TABLE usage_count_gaps;
             DROP TABLE usage_unindexed_checkpoints;
             DROP INDEX usage_events_day_timestamp_session;
             ALTER TABLE usage_sources DROP COLUMN usage_cutoff;
             PRAGMA user_version = 3;",
        )
        .unwrap();
    drop(index);

    let index = PersistentIndex::open(&path).unwrap();
    let sessions: Vec<String> = index
        .db
        .prepare("SELECT session FROM usage_count_gaps ORDER BY session")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(sessions, vec!["session-a", "session-parent"]);
    assert_eq!(source(&index, "source-a").0, 20);
    assert_eq!(daily(&index), (10, Some(2), 1));
    let index_exists: bool = index
        .db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master
             WHERE type = 'index' AND name = 'usage_events_day_timestamp_session')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(index_exists);
}

#[test]
fn migration_keeps_committed_usage_when_checkpoint_is_invalid() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("usage.sqlite");
    let mut index = PersistentIndex::open(&path).unwrap();
    index
        .commit_batch(
            &mut batch("source-a", 0, 20, vec![observation(5, 1, 10, Some(2))]),
            resolve,
        )
        .unwrap();
    index
        .db
        .execute_batch(
            "DROP TABLE usage_count_gaps;
             DROP TABLE usage_unindexed_checkpoints;
             DROP INDEX usage_events_day_timestamp_session;
             ALTER TABLE usage_sources DROP COLUMN usage_cutoff;
             PRAGMA user_version = 3;",
        )
        .unwrap();
    drop(index);

    let mut index = PersistentIndex::open(&path).unwrap();
    assert_eq!(daily(&index), (10, Some(2), 1));
    let error: String = index
        .db
        .query_row(
            "SELECT error FROM usage_unindexed_checkpoints WHERE source = ?1",
            [b"source-a"],
            |row| row.get(0),
        )
        .unwrap();
    assert!(error.contains("invalid usage parser state"));
    let mut next = batch("source-a", 20, 30, vec![observation(25, 2, 20, None)]);
    let error = index
        .commit_batch(&mut next, |tx, sessions| {
            super::resolve::resolve(tx, sessions, &BTreeMap::new())
        })
        .unwrap_err();
    assert!(format!("{error:#}").contains("requires checkpoint recovery"));
    assert_eq!(source(&index, "source-a").0, 20);
    assert_eq!(daily(&index), (10, Some(2), 1));
    assert_eq!(next.observations.len(), 1);
}

#[test]
fn detail_expiry_commits_without_source_changes_and_advances_revision_once() {
    use crate::usage_history::structured::execution_index::ObservationKind;

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("usage.sqlite");
    let mut index = PersistentIndex::open(&path).unwrap();
    let mut source_batch = batch("source-a", 0, 20, vec![observation(5, 1, 10, Some(2))]);
    source_batch
        .execution_observations
        .push(ExecutionObservation {
            session: "session-a".into(),
            id: "call-1".into(),
            timestamp_ms: 1_000,
            start: 10,
            end: 15,
            kind: ObservationKind::Call {
                name: "functions.exec_command".into(),
                namespace: String::new(),
                kind: "tool".into(),
                process_id: None,
            },
        });
    index.commit_batch(&mut source_batch, resolve).unwrap();
    assert_eq!(index.revision().unwrap(), 1);
    assert!(
        !index
            .purge_execution_details(1_000 + 7 * 86_400_000 - 1)
            .unwrap()
    );
    assert_eq!(index.revision().unwrap(), 1);
    assert!(index.purge_execution_details(1_000 + 7 * 86_400_000).unwrap());
    assert_eq!(index.revision().unwrap(), 2);
    assert!(!index.purge_execution_details(1_000 + 7 * 86_400_000).unwrap());
    assert_eq!(source(&index, "source-a").0, 20);
    assert_eq!(daily(&index), (10, Some(2), 1));
    drop(index);
    let index = PersistentIndex::open(&path).unwrap();
    let detail: (Option<i64>, Option<String>) = index
        .db
        .query_row(
            "SELECT end, name FROM execution_observations WHERE call_id = 'call-1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(detail, (None, Some("functions.exec_command".into())));
    assert_eq!(index.revision().unwrap(), 2);
}

#[test]
fn old_event_retention_preserves_summaries_across_append_replacement_and_restart() {
    use chrono::{Local, NaiveDate, TimeZone};

    fn shaped(
        tx: &Transaction<'_>,
        sessions: &BTreeSet<String>,
    ) -> anyhow::Result<Vec<ResolvedEvent>> {
        let mut events = resolve(tx, sessions)?;
        for event in &mut events {
            let day = if matches!(event.id[0], 1 | 2 | 5 | 6) {
                NaiveDate::from_ymd_opt(2026, 8, 1).unwrap()
            } else {
                NaiveDate::from_ymd_opt(2026, 9, 9).unwrap()
            };
            event.day = day.to_string();
            event.timestamp_ms = Local
                .from_local_datetime(&day.and_hms_opt(12, 0, event.id[0] as u32).unwrap())
                .single()
                .unwrap()
                .timestamp_millis();
            event.count_uncertain = event.id[0] == 2;
            event.context_conflicts = u8::from(event.id[0] == 5);
            event.reasoning = (event.id[0] % 2 == 1).then_some(0);
        }
        Ok(events)
    }

    fn snapshot(index: &mut PersistentIndex, request: &serde_json::Value) -> serde_json::Value {
        let options = super::super::Query::parse(
            request,
            NaiveDate::from_ymd_opt(2026, 11, 10).unwrap(),
        )
        .unwrap();
        super::super::history_api::query(&mut index.db, &options, request, &BTreeMap::new())
            .unwrap()
    }

    fn compare(left: &mut PersistentIndex, right: &mut PersistentIndex) {
        let request = json!({"period":"all"});
        let left = snapshot(left, &request);
        let right = snapshot(right, &request);
        for field in ["days", "countStatus", "classificationStatus"] {
            assert_eq!(left[field], right[field], "{field}");
        }
    }

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("retained.sqlite");
    let mut retained = PersistentIndex::open(&path).unwrap();
    let mut control = PersistentIndex::open(&directory.path().join("control.sqlite")).unwrap();
    for index in [&mut retained, &mut control] {
        let mut first = batch(
            "source-a",
            0,
            20,
            vec![
                observation(5, 1, 10, Some(2)),
                observation(10, 2, 20, None),
                observation(15, 3, 30, Some(3)),
            ],
        );
        first.complete = false;
        index.commit_batch(&mut first, shaped).unwrap();
    }
    let now = Local
        .with_ymd_and_hms(2026, 11, 10, 12, 0, 0)
        .single()
        .unwrap()
        .timestamp_millis();
    let first = retained.compact_usage_events(now, None).unwrap();
    assert_eq!(first.deleted, 2);
    assert!(first.precision_changed);
    assert_eq!(retained.revision().unwrap(), 2);
    let counts: (i64, i64) = retained
        .db
        .query_row(
            "SELECT (SELECT COUNT(*) FROM usage_observations),
                    (SELECT COUNT(*) FROM usage_events)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(counts, (3, 1));
    compare(&mut retained, &mut control);
    assert_eq!(snapshot(&mut retained, &json!({"period":"all"}))["coverage"]["precision"], "day");

    let old = Local.with_ymd_and_hms(2026, 8, 1, 10, 0, 0).single().unwrap();
    let old_partial = json!({
        "startAt": old.timestamp_millis(),
        "endAt": (old + chrono::Duration::hours(4)).timestamp_millis()
    });
    let old_result = snapshot(&mut retained, &old_partial);
    assert_eq!(old_result["coverage"]["precision"], "day");
    assert_eq!(old_result["days"], json!([]));
    let recent = Local.with_ymd_and_hms(2026, 9, 9, 10, 0, 0).single().unwrap();
    let recent_partial = json!({
        "startAt": recent.timestamp_millis(),
        "endAt": (recent + chrono::Duration::hours(4)).timestamp_millis()
    });
    assert_eq!(snapshot(&mut retained, &recent_partial)["coverage"]["precision"], "exact");
    assert_eq!(snapshot(&mut retained, &recent_partial)["days"][0]["usage"]["inputTokens"], 30);

    drop(retained);
    let mut retained = PersistentIndex::open(&path).unwrap();
    for index in [&mut retained, &mut control] {
        let mut appended = batch("source-a", 20, 30, vec![observation(25, 4, 40, Some(4))]);
        index
            .commit_batch_resolved(&mut appended, |tx, sessions, _| {
                let mut events = shaped(tx, sessions)?;
                events.retain(|event| event.id == [4; 32]);
                Ok(Resolution::Append(events))
            })
            .unwrap();
    }
    compare(&mut retained, &mut control);

    for index in [&mut retained, &mut control] {
        index
            .commit_batch(
                &mut batch(
                    "source-b",
                    0,
                    20,
                    vec![observation(5, 1, 10, Some(2)), observation(10, 5, 50, None)],
                ),
                shaped,
            )
            .unwrap();
    }
    compare(&mut retained, &mut control);
    assert_eq!(
        retained
            .db
            .query_row(
                "SELECT COUNT(*) FROM usage_events WHERE day = '2026-08-01'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );

    for index in [&mut retained, &mut control] {
        let mut replacement = batch(
            "source-a",
            30,
            20,
            vec![observation(5, 3, 30, Some(3)), observation(10, 6, 60, Some(6))],
        );
        replacement.replace = true;
        replacement.identity = b"replacement".to_vec();
        index.commit_batch(&mut replacement, shaped).unwrap();
    }
    compare(&mut retained, &mut control);
    assert_eq!(source(&retained, "source-a").0, 20);
}

#[test]
fn old_event_retention_is_bounded_and_rolls_back_its_precision_marker() {
    use chrono::{Local, TimeZone};

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("retained.sqlite");
    let mut index = PersistentIndex::open(&path).unwrap();
    let observations = (0u16..600)
        .map(|number| {
            let mut value = observation(u64::from(number), 0, 1, Some(0));
            value.event_id[..2].copy_from_slice(&number.to_le_bytes());
            value
        })
        .collect();
    index
        .commit_batch(&mut batch("source-a", 0, 601, observations), |tx, sessions| {
            let mut events = resolve(tx, sessions)?;
            for event in &mut events {
                event.day = "2026-08-01".into();
            }
            Ok(events)
        })
        .unwrap();
    let now = Local
        .with_ymd_and_hms(2026, 11, 10, 12, 0, 0)
        .single()
        .unwrap()
        .timestamp_millis();
    index
        .db
        .execute_batch(
            "CREATE TRIGGER fail_usage_retention BEFORE INSERT ON usage_day_only
             BEGIN SELECT RAISE(ABORT, 'synthetic retention failure'); END;",
        )
        .unwrap();
    assert!(
        format!("{:#}", index.compact_usage_events(now, None).err().unwrap())
            .contains("synthetic retention failure")
    );
    assert_eq!(index.revision().unwrap(), 1);
    assert_eq!(
        index.db.query_row("SELECT COUNT(*) FROM usage_events", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        600
    );
    assert_eq!(daily(&index), (600, Some(0), 600));
    assert_eq!(source(&index, "source-a").0, 601);
    index.db.execute_batch("DROP TRIGGER fail_usage_retention").unwrap();

    let first = index.compact_usage_events(now, None).unwrap();
    assert_eq!(first.deleted, 256);
    assert!(first.precision_changed);
    assert_eq!(index.revision().unwrap(), 2);
    let second = index.compact_usage_events(now, first.next.as_ref()).unwrap();
    assert_eq!(second.deleted, 256);
    assert!(!second.precision_changed);
    assert_eq!(index.revision().unwrap(), 2);
    let third = index.compact_usage_events(now, second.next.as_ref()).unwrap();
    assert_eq!(third.deleted, 88);
    assert!(!third.precision_changed);
    assert_eq!(index.revision().unwrap(), 2);
    assert_eq!(daily(&index), (600, Some(0), 600));
    assert_eq!(
        index.db.query_row("SELECT COUNT(*) FROM usage_observations", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        600
    );
    drop(index);
    let index = PersistentIndex::open(&path).unwrap();
    assert_eq!(daily(&index), (600, Some(0), 600));
    assert_eq!(index.revision().unwrap(), 2);
}

#[test]
fn full_index_can_reuse_old_usage_event_pages_without_raising_the_limit() {
    use chrono::{Local, TimeZone};

    fn items(range: std::ops::Range<u16>, padding: usize) -> Vec<Observation> {
        range
            .map(|number| {
                let mut value = observation(u64::from(number), 0, 1, Some(0));
                value.event_id[..2].copy_from_slice(&number.to_le_bytes());
                value.payload = serde_json::to_vec(&json!({
                    "input":1, "cached":0, "padding":"x".repeat(padding)
                }))
                .unwrap();
                value
            })
            .collect()
    }

    let directory = tempfile::tempdir().unwrap();
    let mut index =
        PersistentIndex::open_with_budget(&directory.path().join("usage.sqlite"), 2 * 1024 * 1024)
            .unwrap();
    index
        .commit_batch(&mut batch("source-a", 0, 601, items(0..600, 0)), |tx, sessions| {
            let mut events = resolve(tx, sessions)?;
            for event in &mut events {
                event.day = "2026-08-01".into();
            }
            Ok(events)
        })
        .unwrap();
    let pages: u64 = index.db.pragma_query_value(None, "page_count", |row| row.get(0)).unwrap();
    index.db.pragma_update(None, "max_page_count", pages).unwrap();
    let mut large = batch("source-a", 601, 1000, items(600..900, 512));
    assert!(index.commit_batch(&mut large, resolve).is_err());
    assert_eq!(large.observations.len(), 300);
    assert_eq!(source(&index, "source-a").0, 601);
    assert_eq!(daily(&index), (600, Some(0), 600));

    let now = Local
        .with_ymd_and_hms(2026, 11, 10, 12, 0, 0)
        .single()
        .unwrap()
        .timestamp_millis();
    let mut cursor = None;
    loop {
        let result = index.compact_usage_events(now, cursor.as_ref()).unwrap();
        cursor = result.next;
        if cursor.is_none() {
            break;
        }
    }
    let limit: u64 =
        index.db.pragma_query_value(None, "max_page_count", |row| row.get(0)).unwrap();
    assert_eq!(limit, pages);
    assert_eq!(daily(&index), (600, Some(0), 600));

    let mut small = batch("source-a", 601, 700, items(600..640, 128));
    index
        .commit_batch_resolved(&mut small, |tx, sessions, pending| {
            let mut events = resolve(tx, sessions)?;
            events.retain(|event| {
                pending.observations.iter().any(|item| item.event_id == event.id)
            });
            Ok(Resolution::Append(events))
        })
        .unwrap();
    assert_eq!(source(&index, "source-a").0, 700);
    let totals: (i64, i64) = index
        .db
        .query_row("SELECT SUM(input), SUM(records) FROM usage_daily", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap();
    assert_eq!(totals, (640, 640));
}
