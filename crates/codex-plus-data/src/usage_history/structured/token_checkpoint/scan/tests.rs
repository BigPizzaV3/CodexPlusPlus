use super::*;
use crate::usage_history::structured::FileState;
use rusqlite::Connection;
use serde_json::json;
use std::fs;

const FIRST: &str = "00000000-0000-4000-8000-000000000001";
const SECOND: &str = "00000000-0000-4000-8000-000000000002";

fn rollout(session: &str) -> String {
    let header = json!({"type":"session_meta","timestamp":"2026-01-02T12:00:00Z",
        "payload":{"id":session,"model_provider":"custom"}});
    let usage = json!({"input_tokens":100,"cached_input_tokens":20,"output_tokens":10});
    let count = json!({"type":"event_msg","timestamp":"2026-01-02T12:01:00Z",
    "payload":{"type":"token_count","info":{"total_token_usage":{
        "input_tokens":100,"cached_input_tokens":20,"output_tokens":10
        },"last_token_usage":usage}}});
    format!("{header}\n{count}\n")
}

#[test]
fn blocked_source_does_not_delay_other_sources_and_can_retry_without_rescan() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let sessions = root.join("sessions");
    fs::create_dir(&sessions).unwrap();
    let blocked = sessions.join("rollout-01.jsonl");
    let healthy = sessions.join("rollout-02.jsonl");
    fs::write(&healthy, rollout(SECOND)).unwrap();
    fs::write(&blocked, rollout(FIRST)).unwrap();
    let mut store = Store::open(Connection::open_in_memory().unwrap()).unwrap();
    store
        .db
        .execute_batch(&format!(
            "CREATE TRIGGER block_source BEFORE INSERT ON token_days WHEN EXISTS
                (SELECT 1 FROM token_sessions WHERE id=NEW.session_id AND session='{FIRST}')
             BEGIN SELECT RAISE(ABORT, 'source is blocked'); END;"
        ))
        .unwrap();
    let mut scan = Scan::default();
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let names = BTreeMap::new();
    let first = scan.round(&mut store, &root, cutoff, &names, 8192);
    assert!(!first.complete);
    assert_eq!(first.pending_sources, 1);
    assert_eq!(first.issues, 1);
    assert_eq!(first.errors[0].0, blocked);
    assert!(first.errors[0].1.to_string().contains("source is blocked"));
    assert!(store.checkpoint(&blocked).unwrap().is_none());
    assert!(store.checkpoint(&healthy).unwrap().is_some());

    store.db.execute_batch("DROP TRIGGER block_source").unwrap();
    let recovered = scan.round(&mut store, &root, cutoff, &names, 8192);
    assert!(recovered.complete);
    assert_eq!(recovered.pending_sources, 0);
    assert!(recovered.errors.is_empty());
    let mut sessions = Vec::new();
    Store::read_committed(&mut store.db, i64::MIN, i64::MAX, |event| {
        assert_eq!(event.delta.total_tokens, 110);
        sessions.push(event.session);
        Ok(())
    })
    .unwrap();
    sessions.sort();
    assert_eq!(sessions, [FIRST, SECOND]);
}

#[test]
fn deferred_batch_is_discarded_when_its_source_is_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let sessions = root.join("sessions");
    fs::create_dir(&sessions).unwrap();
    let source = sessions.join("rollout-01.jsonl");
    fs::write(&source, rollout(FIRST)).unwrap();
    let mut store = Store::open(Connection::open_in_memory().unwrap()).unwrap();
    store
        .db
        .execute_batch(
            "CREATE TRIGGER block_source BEFORE INSERT ON token_days
             BEGIN SELECT RAISE(ABORT, 'source is blocked'); END;",
        )
        .unwrap();
    let mut scan = Scan::default();
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let names = BTreeMap::new();
    assert_eq!(
        scan.round(&mut store, &root, cutoff, &names, 8192)
            .pending_sources,
        1
    );
    fs::rename(&source, root.join("replaced.jsonl")).unwrap();
    fs::write(&source, rollout(SECOND)).unwrap();
    store.db.execute_batch("DROP TRIGGER block_source").unwrap();
    assert!(scan.round(&mut store, &root, cutoff, &names, 8192).complete);
    let mut seen = Vec::new();
    Store::read_committed(&mut store.db, i64::MIN, i64::MAX, |event| {
        seen.push(event.session);
        Ok(())
    })
    .unwrap();
    assert_eq!(seen, [SECOND]);
}

#[test]
fn source_round_limits_work_and_reports_unfinished_discovery_queue() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let sessions = root.join("sessions");
    fs::create_dir(&sessions).unwrap();
    for index in 0..SOURCES_PER_ROUND + 2 {
        fs::write(sessions.join(format!("rollout-{index}.jsonl")), "").unwrap();
    }
    let mut store = Store::open(Connection::open_in_memory().unwrap()).unwrap();
    let mut scan = Scan::default();
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let names = BTreeMap::new();
    let first = scan.round(&mut store, &root, cutoff, &names, 8192);
    assert!(!first.complete);
    assert_eq!(first.pending_sources, 2);
    assert!(first.errors.is_empty());
    let last = scan.round(&mut store, &root, cutoff, &names, 8192);
    assert!(last.complete);
    assert_eq!(last.pending_sources, 0);
    assert!(last.errors.is_empty());
}

#[test]
fn failed_retention_batch_leaves_archived_totals_and_recent_ingestion_available() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let sessions = root.join("sessions");
    fs::create_dir(&sessions).unwrap();
    fs::write(sessions.join("rollout-01.jsonl"), rollout(FIRST)).unwrap();
    let mut store = Store::open(Connection::open_in_memory().unwrap()).unwrap();
    let mut scan = Scan::default();
    let start = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let names = BTreeMap::new();
    assert!(scan.round(&mut store, &root, start, &names, 8192).complete);
    store
        .db
        .execute_batch(
            "CREATE TRIGGER fail_reclaim BEFORE DELETE ON token_resolved
             BEGIN SELECT RAISE(ABORT, 'test reclamation failure'); END;",
        )
        .unwrap();
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 4).unwrap();
    fs::write(
        sessions.join("rollout-02.jsonl"),
        rollout(SECOND).replace("2026-01-02", "2026-01-05"),
    )
    .unwrap();
    let partial = scan.round(&mut store, &root, cutoff, &names, 8192);
    assert!(!partial.complete);
    assert!(partial.errors.is_empty());
    assert!(
        partial
            .maintenance_error
            .unwrap()
            .to_string()
            .contains("test reclamation failure")
    );
    let mut recent = Vec::new();
    let before = Store::read_committed(&mut store.db, i64::MIN, i64::MAX, |event| {
        recent.push(event.session);
        Ok(())
    })
    .unwrap();
    assert_eq!(recent, [SECOND]);

    store.db.execute_batch("DROP TRIGGER fail_reclaim").unwrap();
    let recovered = scan.round(&mut store, &root, cutoff, &names, 8192);
    assert!(recovered.complete);
    assert!(recovered.maintenance_error.is_none());
    assert_eq!(
        store
            .db
            .query_row("SELECT count(*) FROM token_resolved", [], |row| row
                .get::<_, u64>(0))
            .unwrap(),
        1
    );
    let mut archived = Vec::new();
    let after = Store::read_days(&mut store.db, start, cutoff, |session, _, groups| {
        archived.push((session, groups[0].totals.counts.0.total_tokens));
        Ok(())
    })
    .unwrap();
    assert_eq!(archived, [(FIRST.to_owned(), 110)]);
    assert_eq!(before, after);
}

#[test]
fn source_move_and_removal_keep_archived_days_without_duplicating_recent_usage() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let sessions = root.join("sessions");
    let archives = root.join("archived_sessions");
    fs::create_dir(&sessions).unwrap();
    fs::create_dir(&archives).unwrap();
    let source = sessions.join("rollout-01.jsonl");
    let moved = archives.join("rollout-01.jsonl");
    let old = archives.join("rollout-02.jsonl");
    fs::write(&source, rollout(FIRST).replace("2026-01-02", "2026-01-05")).unwrap();
    fs::write(&old, rollout(SECOND)).unwrap();
    let db = dir.path().join("usage.sqlite");
    let mut store = Store::open(Connection::open(&db).unwrap()).unwrap();
    let mut scan = Scan::default();
    let start = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 4).unwrap();
    let names = BTreeMap::new();
    assert!(scan.round(&mut store, &root, start, &names, 8192).complete);

    // A new physical file still exercises ingestion-before-retirement, not relocation.
    fs::copy(&source, &moved).unwrap();
    fs::remove_file(&source).unwrap();
    store
        .db
        .execute_batch(
            "CREATE TRIGGER block_retirement BEFORE DELETE ON token_sources
         BEGIN SELECT RAISE(ABORT, 'test retirement failure'); END;",
        )
        .unwrap();
    let failed = scan.round(&mut store, &root, cutoff, &names, 8192);
    assert!(!failed.complete);
    assert_eq!(failed.errors[0].0, source);
    assert!(store.checkpoint(&source).unwrap().is_some());
    assert!(store.checkpoint(&moved).unwrap().is_some());
    let mut rows = Vec::new();
    Store::read_committed(&mut store.db, i64::MIN, i64::MAX, |event| {
        rows.push((event.session, event.delta.total_tokens));
        Ok(())
    })
    .unwrap();
    assert_eq!(rows, [(FIRST.into(), 110)]);

    store
        .db
        .execute_batch("DROP TRIGGER block_retirement")
        .unwrap();
    drop(store);
    let mut store = Store::open(Connection::open(&db).unwrap()).unwrap();
    let mut scan = Scan::default();
    assert!(scan.round(&mut store, &root, cutoff, &names, 8192).complete);
    assert!(store.checkpoint(&source).unwrap().is_none());
    assert!(store.checkpoint(&moved).unwrap().is_some());
    fs::remove_file(&moved).unwrap();
    fs::remove_file(&old).unwrap();
    assert!(scan.round(&mut store, &root, cutoff, &names, 8192).complete);
    let mut recent = Vec::new();
    Store::read_committed(&mut store.db, i64::MIN, i64::MAX, |event| {
        recent.push(event);
        Ok(())
    })
    .unwrap();
    assert!(recent.is_empty());
    let mut archived = Vec::new();
    Store::read_days(&mut store.db, start, cutoff, |session, _, groups| {
        archived.push((session, groups[0].totals.counts.0.total_tokens));
        Ok(())
    })
    .unwrap();
    assert_eq!(archived, [(SECOND.into(), 110)]);
}

#[test]
fn physical_archive_move_keeps_a_long_sources_committed_index_and_append_baseline() {
    use std::io::Write;

    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let active = root.join("sessions");
    let archived = root.join("archived_sessions");
    fs::create_dir(&active).unwrap();
    fs::create_dir(&archived).unwrap();
    let name = format!("rollout-{FIRST}.jsonl");
    let source = active.join(&name);
    let moved = archived.join(&name);
    let initial = rollout(FIRST);
    let line = |n: i64| {
        let at = chrono::DateTime::parse_from_rfc3339("2026-01-02T12:01:00Z").unwrap()
            + chrono::Duration::seconds(n);
        json!({"type":"event_msg","timestamp":at.to_rfc3339(),
        "payload":{"type":"token_count","info":{
          "total_token_usage":{"input_tokens":100*n,"cached_input_tokens":20*n,"output_tokens":10*n},
          "last_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":10}
        }}})
    };
    fs::write(&source, initial).unwrap();
    let mut file = fs::OpenOptions::new().append(true).open(&source).unwrap();
    for n in 2..=80 {
        writeln!(file, "{}", line(n)).unwrap();
    }
    drop(file);
    let db = root.join("usage.sqlite");
    let mut store = Store::open(Connection::open(&db).unwrap()).unwrap();
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let names = BTreeMap::new();
    assert!(
        Scan::default()
            .round(&mut store, &root, cutoff, &names, 1 << 20)
            .complete
    );
    let checkpoint = store.checkpoint(&source).unwrap().unwrap();
    let source_id: i64 = store
        .db
        .query_row("SELECT id FROM token_sources", [], |row| row.get(0))
        .unwrap();
    let observation_bytes: u64 = store
        .db
        .query_row(
            "SELECT sum(length(payload)) FROM token_observations",
            [],
            |row| row.get(0),
        )
        .unwrap();
    const BUDGET: usize = 8192;
    assert!(observation_bytes > BUDGET as u64);
    let read = |store: &mut Store| {
        let mut counts = Vec::new();
        let revision = Store::read_committed(&mut store.db, i64::MIN, i64::MAX, |event| {
            counts.push((event.id, event.delta.total_tokens));
            Ok(())
        })
        .unwrap();
        (revision, counts)
    };
    let before = read(&mut store);
    assert_eq!(before.1.len(), 80);

    fs::rename(&source, &moved).unwrap();
    store
        .db
        .execute_batch(
            "CREATE TRIGGER stop_move BEFORE UPDATE OF source ON token_sources
         BEGIN SELECT RAISE(ABORT, 'test relocation failure'); END;",
        )
        .unwrap();
    let failed = Scan::default().round(&mut store, &root, cutoff, &names, BUDGET);
    assert_eq!(failed.errors.len(), 1);
    assert!(
        failed.errors[0]
            .1
            .to_string()
            .contains("test relocation failure")
    );
    assert_eq!(read(&mut store), before);
    assert!(store.checkpoint(&source).unwrap().is_some());
    assert!(store.checkpoint(&moved).unwrap().is_none());
    store.db.execute_batch("DROP TRIGGER stop_move").unwrap();
    drop(store);
    let mut store = Store::open(Connection::open(&db).unwrap()).unwrap();

    let mut scan = Scan::default();
    let report = scan.round(&mut store, &root, cutoff, &names, BUDGET);
    assert!(report.complete, "{:?}", report.errors);
    assert!(store.checkpoint(&source).unwrap().is_none());
    let (source_revision, state) = store.checkpoint(&moved).unwrap().unwrap();
    assert_eq!(
        (source_revision, state.offset),
        (checkpoint.0, checkpoint.1.offset)
    );
    let moved_id: i64 = store
        .db
        .query_row("SELECT id FROM token_sources", [], |row| row.get(0))
        .unwrap();
    assert_eq!(moved_id, source_id);
    let relocated = read(&mut store);
    assert_eq!(relocated.0, before.0 + 1);
    assert_eq!(relocated.1, before.1);

    writeln!(
        fs::OpenOptions::new().append(true).open(&moved).unwrap(),
        "{}",
        line(81)
    )
    .unwrap();
    let report = scan.round(&mut store, &root, cutoff, &names, BUDGET);
    assert!(report.complete, "{:?}", report.errors);
    let after = read(&mut store).1;
    assert_eq!(after.len(), 81);
    assert_eq!(after[..80], before.1);
    assert_eq!(after[80].1, 110);
}

#[test]
fn identical_archive_copy_must_not_stall_the_original_append() {
    use std::io::Write;

    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let active = root.join("sessions");
    let archived = root.join("archived_sessions");
    fs::create_dir(&active).unwrap();
    fs::create_dir(&archived).unwrap();
    let name = format!("rollout-{FIRST}.jsonl");
    let source = active.join(&name);
    let copy = archived.join(&name);
    let initial = rollout(FIRST);
    let (_, prototype) = initial.split_once('\n').unwrap();
    let count = |n: i64| {
        let mut event: serde_json::Value = serde_json::from_str(prototype).unwrap();
        let at = chrono::DateTime::parse_from_rfc3339("2026-01-02T12:01:00Z").unwrap()
            + chrono::Duration::seconds(n);
        event["timestamp"] = json!(at.to_rfc3339());
        event["payload"]["info"]["total_token_usage"] =
            json!({"input_tokens":100*n,"cached_input_tokens":20*n,"output_tokens":10*n});
        event
    };
    fs::write(&source, initial.as_bytes()).unwrap();
    let mut file = fs::OpenOptions::new().append(true).open(&source).unwrap();
    for n in 2..=80 {
        writeln!(file, "{}", count(n)).unwrap();
    }
    drop(file);
    let db = root.join("usage.sqlite");
    let mut store = Store::open(Connection::open(&db).unwrap()).unwrap();
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let names = BTreeMap::new();
    let mut scan = Scan::default();
    assert!(
        scan.round(&mut store, &root, cutoff, &names, 1 << 20)
            .complete
    );
    fs::copy(&source, &copy).unwrap();
    assert_ne!(
        FileState::source_stamp(&source).unwrap().2,
        FileState::source_stamp(&copy).unwrap().2,
    );
    store
        .db
        .execute_batch(
            "CREATE TRIGGER refuse_copy BEFORE INSERT ON token_source_copies
         BEGIN SELECT RAISE(ABORT, 'test copy registration failure'); END;",
        )
        .unwrap();
    const BUDGET: usize = 8192;
    let failed = scan.round(&mut store, &root, cutoff, &names, BUDGET);
    assert!(!failed.complete);
    assert_eq!(failed.errors.len(), 1);
    assert!(
        failed.errors[0]
            .1
            .to_string()
            .contains("test copy registration failure")
    );
    assert!(store.checkpoint(&copy).unwrap().is_none());
    store.db.execute_batch("DROP TRIGGER refuse_copy").unwrap();
    let registered = scan.round(&mut store, &root, cutoff, &names, BUDGET);
    assert!(registered.complete, "{:?}", registered.errors);
    assert!(store.checkpoint(&copy).unwrap().is_none());
    let source_id: i64 = store
        .db
        .query_row("SELECT id FROM token_sources", [], |row| row.get(0))
        .unwrap();
    let read = |store: &mut Store| {
        let mut counts = Vec::new();
        Store::read_committed(&mut store.db, i64::MIN, i64::MAX, |event| {
            counts.push((event.id, event.delta.total_tokens));
            Ok(())
        })
        .unwrap();
        counts
    };
    let prior = read(&mut store);
    assert_eq!(prior.len(), 80);
    drop(store);

    writeln!(
        fs::OpenOptions::new().append(true).open(&source).unwrap(),
        "{}",
        count(81)
    )
    .unwrap();
    let mut store = Store::open(Connection::open(&db).unwrap()).unwrap();
    let mut scan = Scan::default();
    let report = scan.round(&mut store, &root, cutoff, &names, BUDGET);
    assert!(report.complete, "{:?}", report.errors);
    let after = read(&mut store);
    assert_eq!(after.len(), 81);
    assert_eq!(after[..80], prior);
    assert_eq!(after[80].1, 110);

    fs::copy(&source, &copy).unwrap();
    let verified = scan.round(&mut store, &root, cutoff, &names, BUDGET);
    assert!(verified.complete, "{:?}", verified.errors);
    fs::remove_file(&source).unwrap();
    drop(store);
    let mut store = Store::open(Connection::open(&db).unwrap()).unwrap();
    let mut scan = Scan::default();
    let promoted = scan.round(&mut store, &root, cutoff, &names, BUDGET);
    assert!(promoted.complete, "{:?}", promoted.errors);
    assert_eq!(read(&mut store), after);
    assert!(store.checkpoint(&source).unwrap().is_none());
    assert!(store.checkpoint(&copy).unwrap().is_some());
    assert_eq!(
        store
            .db
            .query_row("SELECT id FROM token_sources", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        source_id,
    );
    writeln!(
        fs::OpenOptions::new().append(true).open(&copy).unwrap(),
        "{}",
        count(82)
    )
    .unwrap();
    let appended = scan.round(&mut store, &root, cutoff, &names, BUDGET);
    assert!(appended.complete, "{:?}", appended.errors);
    let final_counts = read(&mut store);
    assert_eq!(final_counts[..81], after);
    assert_eq!(final_counts.len(), 82);
    assert_eq!(final_counts[81].1, 110);

    fs::copy(&copy, &source).unwrap();
    writeln!(
        fs::OpenOptions::new().append(true).open(&source).unwrap(),
        "{}",
        count(83)
    )
    .unwrap();
    let extended = scan.round(&mut store, &root, cutoff, &names, BUDGET);
    assert!(extended.complete, "{:?}", extended.errors);
    assert!(store.checkpoint(&copy).unwrap().is_none());
    assert!(store.checkpoint(&source).unwrap().is_some());
    let extended_counts = read(&mut store);
    assert_eq!(extended_counts[..82], final_counts);
    assert_eq!(extended_counts.len(), 83);
    assert_eq!(extended_counts[82].1, 110);

    fs::remove_file(&copy).unwrap();
    fs::rename(&source, &copy).unwrap();
    let renamed = scan.round(&mut store, &root, cutoff, &names, BUDGET);
    assert!(renamed.complete, "{:?}", renamed.errors);
    assert_eq!(read(&mut store), extended_counts);
    assert_eq!(
        store
            .db
            .query_row("SELECT count(*) FROM token_source_copies", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );

    // If discovery first sees the copy after the original is gone, the file
    // name and size alone must not let it inherit the committed cursor.
    fs::copy(&copy, &source).unwrap();
    fs::remove_file(&copy).unwrap();
    drop(store);
    let mut store = Store::open(Connection::open(&db).unwrap()).unwrap();
    let unavailable = Scan::default().round(&mut store, &root, cutoff, &names, BUDGET);
    assert!(!unavailable.complete);
    assert!(store.checkpoint(&source).unwrap().is_none());
    assert!(store.checkpoint(&copy).unwrap().is_some());
    assert_eq!(read(&mut store), extended_counts);

    let reread = Scan::default().round(&mut store, &root, cutoff, &names, 1 << 20);
    assert!(reread.complete, "{:?}", reread.errors);
    assert!(store.checkpoint(&source).unwrap().is_some());
    assert!(store.checkpoint(&copy).unwrap().is_none());
    assert_eq!(read(&mut store), extended_counts);
}

#[test]
fn previously_materialized_copies_recover_a_verified_tail_without_republishing_counts() {
    use std::io::Write;

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let active = root.join("sessions");
    let archive = root.join("archived_sessions");
    fs::create_dir(&active).unwrap();
    fs::create_dir(&archive).unwrap();
    let name = format!("rollout-{FIRST}.jsonl");
    let source = active.join(&name);
    let copy = archive.join(&name);
    let at = chrono::DateTime::parse_from_rfc3339("2026-01-02T12:01:00Z").unwrap();
    let row = |n| {
        json!({"type":"event_msg","timestamp":(at+chrono::Duration::seconds(n)).to_rfc3339(),
        "payload":{"type":"token_count","info":{
            "total_token_usage":{"input_tokens":100*n,"cached_input_tokens":20*n,"output_tokens":10*n},
            "last_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":10}
        }}})
    };
    let head = rollout(FIRST);
    fs::write(&source, &head).unwrap();
    let mut file = fs::OpenOptions::new().append(true).open(&source).unwrap();
    for n in 2..=80 {
        writeln!(file, "{}", row(n)).unwrap();
    }
    drop(file);
    let db = root.join("usage.sqlite");
    let mut store = Store::open(Connection::open(&db).unwrap()).unwrap();
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let names = BTreeMap::new();
    fs::copy(&source, &copy).unwrap();
    for path in [&source, &copy] {
        // Reproduce the old scanner, which ingested both paths before copy proof existed.
        assert!(
            SourceReader::restore(&store, path)
                .unwrap()
                .step(
                    &mut store,
                    cutoff,
                    &mut Budget::new(std::iter::empty()),
                    &names,
                    1 << 20
                )
                .unwrap()
        );
    }
    let count_table = |store: &Store, query: &str| {
        store
            .db
            .query_row(query, [], |row| row.get::<_, u64>(0))
            .unwrap()
    };
    assert_eq!(count_table(&store, "SELECT count(*) FROM token_sources"), 2);
    assert_eq!(
        count_table(&store, "SELECT count(*) FROM token_linear_sources"),
        0
    );
    let rows = |store: &mut Store| {
        let mut counts = Vec::new();
        Store::read_committed(&mut store.db, i64::MIN, i64::MAX, |e| {
            counts.push((e.id, e.delta));
            Ok(())
        })
        .unwrap();
        counts
    };
    let before = rows(&mut store);
    assert_eq!(before.len(), 80);
    writeln!(
        fs::OpenOptions::new().append(true).open(&source).unwrap(),
        "{}",
        row(81)
    )
    .unwrap();
    store
        .db
        .execute_batch(
            "CREATE TRIGGER refuse_collapse BEFORE DELETE ON token_sources
         BEGIN SELECT RAISE(ABORT, 'test collapse failure'); END;",
        )
        .unwrap();
    let failed = Scan::default().round(&mut store, &root, cutoff, &names, 8192);
    assert!(!failed.complete);
    assert!(
        failed
            .errors
            .iter()
            .any(|(_, error)| error.to_string().contains("test collapse failure"))
    );
    assert_eq!(count_table(&store, "SELECT count(*) FROM token_sources"), 2);
    assert_eq!(
        count_table(&store, "SELECT count(*) FROM token_linear_sources"),
        0
    );
    assert_eq!(rows(&mut store), before);
    store
        .db
        .execute_batch("DROP TRIGGER refuse_collapse")
        .unwrap();
    drop(store);
    let mut store = Store::open(Connection::open(&db).unwrap()).unwrap();
    let mut scan = Scan::default();
    let recovered = scan.round(&mut store, &root, cutoff, &names, 8192);
    assert!(recovered.complete, "{:?}", recovered.errors);
    assert_eq!(count_table(&store, "SELECT count(*) FROM token_sources"), 1);
    assert_eq!(
        count_table(&store, "SELECT count(*) FROM token_source_copies"),
        1
    );
    let after = rows(&mut store);
    assert_eq!(after.len(), 81);
    assert_eq!(after[..80], before);
    assert_eq!(after[80].1.total_tokens, 110);

    let mut fork = row(82);
    fork["payload"]["info"]["total_token_usage"]["input_tokens"] = json!(9000);
    writeln!(
        fs::OpenOptions::new().append(true).open(&copy).unwrap(),
        "{fork}"
    )
    .unwrap();
    writeln!(
        fs::OpenOptions::new().append(true).open(&source).unwrap(),
        "{}",
        row(82)
    )
    .unwrap();
    let divergent = scan.round(&mut store, &root, cutoff, &names, 8192);
    assert!(!divergent.complete);
    assert_eq!(count_table(&store, "SELECT count(*) FROM token_sources"), 1);
    let after_fork = rows(&mut store);
    assert_eq!(after_fork[..81], after);
    assert_eq!(after_fork.len(), 82);
    assert_eq!(after_fork[81].1.total_tokens, 110);
}

#[test]
fn large_copy_yields_to_other_sessions_and_a_changed_prefix_is_not_admitted_as_a_copy() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let active = root.join("sessions");
    let archived = root.join("archived_sessions");
    fs::create_dir(&active).unwrap();
    fs::create_dir(&archived).unwrap();
    let name = format!("rollout-{FIRST}.jsonl");
    let source = active.join(&name);
    let copy = archived.join(&name);
    let independent = active.join(format!("rollout-{SECOND}.jsonl"));
    let padding = format!("{}\n", " ".repeat(512 * 1024)).repeat(3);
    let bytes = format!("{}{}", rollout(FIRST), padding);
    fs::write(&source, &bytes).unwrap();
    let mut store = Store::open(Connection::open_in_memory().unwrap()).unwrap();
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let names = BTreeMap::new();
    let mut scan = Scan::default();
    const BUDGET: usize = 8192;
    assert!(
        scan.round(&mut store, &root, cutoff, &names, BUDGET)
            .complete
    );

    fs::copy(&source, &copy).unwrap();
    fs::write(&independent, rollout(SECOND)).unwrap();
    let pending = scan.round(&mut store, &root, cutoff, &names, BUDGET);
    assert_eq!(pending.pending_sources, 1);
    assert!(pending.errors.is_empty());
    assert!(store.checkpoint(&independent).unwrap().is_some());
    assert!(store.checkpoint(&copy).unwrap().is_none());
    let ready = scan.round(&mut store, &root, cutoff, &names, BUDGET);
    assert!(ready.complete, "{:?}", ready.errors);
    assert!(store.checkpoint(&copy).unwrap().is_none());

    fs::rename(&copy, archived.join("previous.jsonl")).unwrap();
    fs::write(
        &copy,
        bytes.replacen(
            "\"cached_input_tokens\":20",
            "\"cached_input_tokens\":19",
            1,
        ),
    )
    .unwrap();
    let diverged = scan.round(&mut store, &root, cutoff, &names, BUDGET);
    assert!(diverged.complete, "{:?}", diverged.errors);
    assert!(store.checkpoint(&source).unwrap().is_some());
    assert!(store.checkpoint(&copy).unwrap().is_some());
    assert_eq!(
        store
            .db
            .query_row("SELECT count(*) FROM token_source_copies", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn missing_sources_are_retired_in_batches_and_reappearance_is_not_a_deletion() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let sessions = root.join("sessions");
    fs::create_dir(&sessions).unwrap();
    let mut store = Store::open(Connection::open_in_memory().unwrap()).unwrap();
    let mut scan = Scan::default();
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let names = BTreeMap::new();

    let removed: Vec<_> = (0..SOURCES_PER_ROUND + 2)
        .map(|index| sessions.join(format!("rollout-{index:03}.jsonl")))
        .collect();
    for path in &removed {
        fs::write(path, "").unwrap();
    }
    assert!(!scan.round(&mut store, &root, cutoff, &names, 8192).complete);
    assert!(scan.round(&mut store, &root, cutoff, &names, 8192).complete);
    for path in &removed {
        assert!(store.checkpoint(path).unwrap().is_some());
        fs::remove_file(path).unwrap();
    }
    // A full ingestion round leaves retirement pending after discovery.
    for index in 0..SOURCES_PER_ROUND {
        fs::write(sessions.join(format!("rollout-new-{index:03}.jsonl")), "").unwrap();
    }
    assert!(!scan.round(&mut store, &root, cutoff, &names, 8192).complete);
    let reappeared = scan.missing.front().unwrap().0.clone();
    fs::write(&reappeared, "").unwrap();
    let first_retirement = scan.round(&mut store, &root, cutoff, &names, 8192);
    assert!(!first_retirement.complete);
    assert!(first_retirement.errors.is_empty());
    assert!(store.checkpoint(&reappeared).unwrap().is_some());
    assert_eq!(
        removed
            .iter()
            .filter(|path| store.checkpoint(path).unwrap().is_some())
            .count(),
        3
    );
    assert!(!scan.round(&mut store, &root, cutoff, &names, 8192).complete);
    assert!(scan.round(&mut store, &root, cutoff, &names, 8192).complete);
    for path in removed {
        assert_eq!(
            store.checkpoint(&path).unwrap().is_some(),
            path == reappeared
        );
    }
}

#[test]
fn incomplete_discovery_cannot_retire_a_committed_source() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let sessions = root.join("sessions");
    fs::create_dir(&sessions).unwrap();
    let mut store = Store::open(Connection::open_in_memory().unwrap()).unwrap();
    let mut scan = Scan::default();
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let names = BTreeMap::new();
    let source = sessions.join("rollout-01.jsonl");
    fs::write(&source, rollout(FIRST)).unwrap();
    assert!(scan.round(&mut store, &root, cutoff, &names, 8192).complete);

    fs::remove_file(&source).unwrap();
    let archives = root.join("archived_sessions");
    // An inaccessible discovery root could hide the replacement.
    fs::write(&archives, "").unwrap();
    let incomplete = scan.round(&mut store, &root, cutoff, &names, 8192);
    assert!(!incomplete.complete);
    assert!(incomplete.issues > 0);
    assert!(store.checkpoint(&source).unwrap().is_some());

    fs::remove_file(&archives).unwrap();
    assert!(scan.round(&mut store, &root, cutoff, &names, 8192).complete);
    assert!(store.checkpoint(&source).unwrap().is_none());
}

#[test]
fn an_interrupted_round_cannot_reuse_coverage_for_a_newer_snapshot() {
    use crate::usage_history::Query;
    use rusqlite::OpenFlags;

    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let sessions = root.join("sessions");
    fs::create_dir(&sessions).unwrap();
    let database = root.join("tokens.sqlite");
    let mut store = Store::open(Connection::open(&database).unwrap()).unwrap();
    let mut scan = Scan::default();
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let names = BTreeMap::new();
    fs::write(sessions.join("rollout-01.jsonl"), rollout(FIRST)).unwrap();
    assert!(scan.round(&mut store, &root, cutoff, &names, 8192).complete);
    let mut reader =
        Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let query = Query::parse(&json!({}), cutoff).unwrap();
    let first = Store::read_selection(&mut reader, &query, None, None).unwrap();
    assert_eq!(first.days[0]["recordCount"], 1);
    assert_eq!(
        first.coverage,
        Some(Coverage {
            pending_sources: 0,
            issues: 0,
            maintenance_blocked: false,
        })
    );

    let path = sessions.join("rollout-02.jsonl");
    fs::write(&path, rollout(SECOND)).unwrap();
    let mut source = SourceReader::restore(&store, &path).unwrap();
    let mut budget = Budget::new(std::iter::empty());
    assert!(
        source
            .step(&mut store, cutoff, &mut budget, &names, 8192)
            .unwrap()
    );
    drop(store);
    let (all, scoped) = Store::read_scoped_selection(&mut reader, &query, None, FIRST).unwrap();
    assert!(all.revision > first.revision);
    assert_eq!(all.revision, scoped.usage.revision);
    assert_eq!(all.days[0]["recordCount"], 2);
    assert_eq!(scoped.usage.days[0]["recordCount"], 1);
    assert!(all.coverage.is_none());
    assert!(scoped.usage.coverage.is_none());

    let mut store = Store::open(Connection::open(&database).unwrap()).unwrap();
    let mut scan = Scan::default();
    assert!(scan.round(&mut store, &root, cutoff, &names, 8192).complete);
    let settled = Store::read_selection(&mut reader, &query, None, None).unwrap();
    assert_eq!(settled.revision, all.revision);
    assert_eq!(settled.days, all.days);
    assert_eq!(settled.coverage, first.coverage);
}

#[test]
fn cold_index_does_not_report_an_empty_archived_day_as_exact() {
    use crate::usage_history::Query;
    use chrono::{Local, TimeZone};

    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let sessions = root.join("sessions");
    fs::create_dir(&sessions).unwrap();
    fs::write(sessions.join("rollout-01.jsonl"), rollout(FIRST)).unwrap();
    let db = root.join("tokens.sqlite");
    let mut store = Store::open(Connection::open(&db).unwrap()).unwrap();
    let mut scan = Scan::default();
    let archived = NaiveDate::from_ymd_opt(2026, 1, 2).unwrap();
    let cutoff = archived + chrono::Days::new(2);
    assert!(
        scan.round(&mut store, &root, cutoff, &BTreeMap::new(), 8192)
            .complete
    );
    drop(store);
    let mut reader =
        Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();

    let midnight = |day: NaiveDate| {
        Local
            .from_local_datetime(&day.and_hms_opt(0, 0, 0).unwrap())
            .single()
            .unwrap()
            .timestamp_millis()
    };
    let old = Query::parse(
        &json!({"from":archived.to_string(),"to":archived.to_string()}),
        cutoff,
    )
    .unwrap();
    for bounds in [
        None,
        Some((midnight(archived), midnight(archived.succ_opt().unwrap()))),
    ] {
        let selection = Store::read_selection(&mut reader, &old, bounds, None).unwrap();
        assert!(selection.days.is_empty());
        assert!(selection.needs_source_recovery);
    }
    let current = Query::parse(&json!({"period":"today"}), cutoff).unwrap();
    let selection = Store::read_selection(&mut reader, &current, None, None).unwrap();
    assert!(selection.days.is_empty());
    assert!(!selection.needs_source_recovery);
}

#[test]
fn execution_locators_commit_with_the_source_and_expire_without_losing_the_call() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let sessions = root.join("sessions");
    fs::create_dir(&sessions).unwrap();
    let source = sessions.join(format!("rollout-{FIRST}.jsonl"));
    let now = chrono::Local::now();
    let at = now - chrono::Duration::seconds(10);
    let timestamp = at.to_rfc3339();
    let template = rollout(FIRST);
    let (meta, token) = template.split_once('\n').unwrap();
    let token = token.replace("2026-01-02T12:01:00Z", &timestamp);
    let call = json!({"type":"response_item","timestamp":timestamp,"payload":{
        "type":"function_call","name":"exec_command","namespace":"functions",
        "call_id":"example","arguments":"{\"cmd\":\"fixture-body\"}"
    }});
    let output = json!({"type":"response_item","timestamp":timestamp,"payload":{
        "type":"function_call_output","call_id":"example","output":"fixture-output"
    }});
    let content = format!("{meta}\n{token}{call}\n{output}\n");
    fs::write(&source, &content).unwrap();
    let db = root.join("usage.sqlite");
    let mut store = Store::open(Connection::open(&db).unwrap()).unwrap();
    store
        .db
        .execute_batch(
            "CREATE TRIGGER block_execution BEFORE INSERT ON execution_observations
         BEGIN SELECT RAISE(ABORT, 'execution write failed'); END;",
        )
        .unwrap();
    let names = BTreeMap::new();
    let cutoff = at.date_naive();
    let failed = Scan::default().round(&mut store, &root, cutoff, &names, 8192);
    assert!(!failed.complete);
    assert!(
        failed.errors[0]
            .1
            .to_string()
            .contains("execution write failed")
    );
    assert!(store.checkpoint(&source).unwrap().is_none());
    let mut committed = 0;
    Store::read_committed(&mut store.db, i64::MIN, i64::MAX, |_| {
        committed += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!(committed, 0);
    store
        .db
        .execute_batch("DROP TRIGGER block_execution")
        .unwrap();
    drop(store);

    let mut store = Store::open(Connection::open(&db).unwrap()).unwrap();
    let report = Scan::default().round(&mut store, &root, cutoff, &names, 8192);
    assert!(report.complete, "{:?}", report.errors);
    assert_eq!(
        store.checkpoint(&source).unwrap().unwrap().1.offset,
        content.len() as u64
    );
    {
        let mut query = store
            .db
            .prepare(
                "SELECT call_id, position, end, name FROM execution_observations ORDER BY position",
            )
            .unwrap();
        let mut rows = query.query([]).unwrap();
        for (expected, name) in [(call, Some("exec_command")), (output, None)] {
            let row = rows.next().unwrap().unwrap();
            let id: String = row.get(0).unwrap();
            let start: usize = row.get(1).unwrap();
            let end: usize = row.get(2).unwrap();
            assert_eq!(id, "example");
            assert_eq!(row.get::<_, Option<String>>(3).unwrap().as_deref(), name);
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&content[start..end]).unwrap(),
                expected
            );
        }
        assert!(rows.next().unwrap().is_none());
    }
    let retained: u64 = store
        .db
        .query_row("SELECT count(*) FROM token_activity", [], |row| row.get(0))
        .unwrap();
    assert_eq!(retained, 0);
    Store::read_committed(&mut store.db, i64::MIN, i64::MAX, |event| {
        assert_eq!(event.delta.total_tokens, 110);
        committed += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!(committed, 1);
    use crate::usage_history::structured::token_checkpoint::{DetailQuery, ExecutionQuery};
    let page_request = ExecutionQuery::parse(&json!({
        "startAt":at.timestamp_millis(),"endAt":now.timestamp_millis(),"limit":1,
        "scopeSessionId":FIRST,
    }))
    .unwrap();
    let page = Store::read_execution_page(
        &mut store.db,
        &page_request,
        now.timestamp_millis(),
        false,
        None,
    )
    .unwrap();
    assert_eq!(page["status"], "ok", "{page}");
    assert_eq!(page["summary"]["tool"], 1);
    assert_eq!(page["items"][0]["id"], "example");
    assert!(page["nextCursor"].is_null());
    let detail_request = DetailQuery::parse(&json!({
        "sessionId":FIRST,"id":"example","revision":page["revision"]
    }))
    .unwrap();
    let read_detail = |store: &mut Store, now| {
        Store::read_execution_detail(&mut store.db, &root, &detail_request, || now)
    };
    let detail = read_detail(&mut store, now.timestamp_millis()).unwrap();
    assert_eq!(detail["status"], "partial"); // No terminal shell receipt.
    assert_eq!(detail["detail"]["input"]["cmd"], "fixture-body");
    assert_eq!(detail["detail"]["output"], json!(["fixture-output"]));
    assert_eq!(
        read_detail(&mut store, at.timestamp_millis() + 86_400_000).unwrap()["status"],
        "not_found"
    );
    fs::rename(&source, root.join("original.jsonl")).unwrap();
    fs::write(&source, &content).unwrap();
    assert!(read_detail(&mut store, now.timestamp_millis()).is_err());
    store
        .expire_execution_locators(
            at.timestamp_millis()
                + crate::usage_history::structured::execution_observations::DETAIL_MS,
            256,
        )
        .unwrap();
    let remaining: (u32, u32) = store
        .db
        .query_row(
            "SELECT count(*), count(end) FROM execution_observations",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(remaining, (1, 0));
}

#[test]
fn detail_follows_a_verified_shell_process_and_rejects_reused_process_ids() {
    use crate::usage_history::structured::token_checkpoint::DetailQuery;
    let dir = tempfile::tempdir().unwrap();
    let home = fs::canonicalize(dir.path()).unwrap();
    let sessions = home.join("sessions");
    fs::create_dir(&sessions).unwrap();
    let path = sessions.join(format!("rollout-{FIRST}.jsonl"));
    let now = chrono::Local::now();
    let start = now - chrono::Duration::seconds(60);
    let row = |offset: i64, payload| {
        json!({
            "type":"response_item","timestamp":(start + chrono::Duration::milliseconds(offset)).to_rfc3339(),"payload":payload,
        })
    };
    let base = rollout(FIRST);
    let rows = [
        row(
            0,
            json!({"type":"function_call","name":"functions.exec_command","call_id":"open","arguments":"{\"cmd\":\"example\"}"}),
        ),
        row(
            10,
            json!({"type":"function_call","name":"functions.write_stdin","call_id":"before","arguments":"{\"session_id\":831}"}),
        ),
        row(
            20,
            json!({"type":"function_call_output","call_id":"before","output":"Process exited with code 0\nOutput:\nnot owned"}),
        ),
        row(
            30,
            json!({"type":"function_call_output","call_id":"open","output":format!(
                "Chunk ID: x\nProcess running with session ID 831\nOutput:\n{}", "x".repeat(8192)
            )}),
        ),
        row(
            50,
            json!({"type":"function_call","name":"functions.write_stdin","call_id":"other-poll","arguments":{"session_id":557}}),
        ),
        row(
            60,
            json!({"type":"function_call_output","call_id":"other-poll","output":"Process exited with code 0\nOutput:\nnot associated"}),
        ),
        row(
            500,
            json!({"type":"function_call","name":"functions.write_stdin","call_id":"finish","arguments":{"session_id":831}}),
        ),
        row(
            700,
            json!({"type":"function_call_output","call_id":"finish","output":"Process exited with code 0\nOutput:\nfinished"}),
        ),
        row(
            900,
            json!({"type":"function_call","name":"functions.write_stdin","call_id":"after","arguments":{"session_id":831}}),
        ),
        row(
            990,
            json!({"type":"function_call_output","call_id":"after","output":"Process exited with code 0\nOutput:\nnot active"}),
        ),
    ];
    let encode = |rows: &[serde_json::Value]| {
        rows.iter()
            .map(|row| format!("{row}\n"))
            .collect::<String>()
    };
    fs::write(&path, format!("{base}{}", encode(&rows))).unwrap();
    let db = home.join("usage.sqlite");
    let mut store = Store::open(Connection::open(&db).unwrap()).unwrap();
    let cutoff = start.date_naive() - chrono::Days::new(2);
    let mut scan = Scan::default();
    let report = scan.round(&mut store, &home, cutoff, &BTreeMap::new(), 1 << 20);
    assert!(report.complete, "{:?}", report.errors);
    drop(store);
    let mut store = Store::open(Connection::open(&db).unwrap()).unwrap();
    let read = |store: &mut Store| {
        let revision: i64 = store
            .db
            .query_row("SELECT revision FROM token_revision", [], |r| r.get(0))
            .unwrap();
        let request =
            DetailQuery::parse(&json!({"sessionId":FIRST,"id":"open","revision":revision}))
                .unwrap();
        Store::read_execution_detail(&mut store.db, &home, &request, || now.timestamp_millis())
            .unwrap()
    };
    let detail = read(&mut store);
    assert_eq!(detail["status"], "ok", "{detail}");
    assert_eq!(detail["detail"]["durationMs"], 700);
    let parts = detail["detail"]["output"].as_array().unwrap();
    assert_eq!(parts.len(), 2);
    assert!(parts[1].as_str().unwrap().ends_with("finished"));
    let competing = [
        row(
            1000,
            json!({"type":"function_call","name":"functions.exec_command","call_id":"other","arguments":"{\"cmd\":\"another\"}"}),
        ),
        row(
            1100,
            json!({"type":"function_call_output","call_id":"other","output":"Process running with session ID 831\nOutput:\nother"}),
        ),
    ];
    fs::write(
        &path,
        format!("{base}{}{}", encode(&rows), encode(&competing)),
    )
    .unwrap();
    let report = scan.round(&mut store, &home, cutoff, &BTreeMap::new(), 1 << 20);
    assert!(report.complete, "{:?}", report.errors);
    let ambiguous = read(&mut store);
    assert_eq!(ambiguous["status"], "partial");
    assert!(ambiguous["detail"]["durationMs"].is_null());
    assert_eq!(ambiguous["detail"]["output"].as_array().unwrap().len(), 1);
}

#[test]
fn activity_checkpoint_restarts_polls_and_publishes_only_committed_receipts() {
    use crate::usage_history::Query;
    use std::io::Write;

    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let sessions = root.join("sessions");
    fs::create_dir(&sessions).unwrap();
    let source = sessions.join("rollout-01.jsonl");
    let db = root.join("tokens.sqlite");
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let tool = |time: &str, payload| {
        json!({
            "type":"response_item","timestamp":format!("2026-01-02T12:{time}Z"),"payload":payload,
        })
    };
    let head = rollout(FIRST);
    let skill = tool(
        "02:00",
        json!({"type":"function_call","call_id":"skill",
        "name":"functions.exec_command","arguments":"{\"cmd\":\"cat /skills/pdf/SKILL.md\"}"}),
    );
    let running = tool(
        "02:01",
        json!({"type":"function_call_output","call_id":"skill",
        "output":"Process running with session ID 42\nname: pdf"}),
    );
    let poll = tool(
        "02:02",
        json!({"type":"function_call","call_id":"poll",
        "name":"functions.write_stdin","arguments":"{\"session_id\":42,\"chars\":\"\"}"}),
    );
    fs::write(&source, format!("{head}{skill}\n{running}\n{poll}\n")).unwrap();
    let names = BTreeMap::from([("custom".into(), "Example Relay".into())]);
    let request = json!({"includeActivity":true,"includeFingerprints":false});
    let query = Query::parse(&request, cutoff).unwrap();
    let mut store = Store::open(Connection::open(&db).unwrap()).unwrap();
    assert!(
        Scan::default()
            .round(&mut store, &root, cutoff, &names, 8192)
            .complete
    );
    drop(store);
    let mut store = Store::open(Connection::open(&db).unwrap()).unwrap();
    let mut read =
        Connection::open_with_flags(&db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let before = Store::read_response(&mut read, &query, &request, None, &names).unwrap();
    assert_eq!(before["activityPagination"]["total"], 0);
    assert_eq!(before["days"][0]["usage"]["totalTokens"], 110);

    let mcp = tool(
        "01:35",
        json!({"type":"function_call","call_id":"mcp",
        "name":"mcp__demo__lookup","arguments":"{}"}),
    );
    let mcp_done = tool(
        "01:36",
        json!({"type":"function_call_output","call_id":"mcp","output":"ok"}),
    );
    let done = tool(
        "02:03",
        json!({"type":"function_call_output","call_id":"poll",
        "output":"Process exited with code 0\n# Skill content observed"}),
    );
    let mut file = fs::OpenOptions::new().append(true).open(&source).unwrap();
    writeln!(file, "{done}\n{mcp}\n{mcp_done}").unwrap();
    drop(file);
    store
        .db
        .execute_batch(
            "CREATE TRIGGER refuse_activity BEFORE INSERT ON token_activity
         BEGIN SELECT RAISE(ABORT, 'activity write failed'); END;",
        )
        .unwrap();
    let mut scan = Scan::default();
    let blocked = scan.round(&mut store, &root, cutoff, &names, 8192);
    assert_eq!(blocked.pending_sources, 1);
    assert_eq!(blocked.errors.len(), 1);
    let during = Store::read_response(&mut read, &query, &request, None, &names).unwrap();
    assert_eq!(during["revision"], before["revision"]);
    assert_eq!(during["activityPagination"]["total"], 0);
    store
        .db
        .execute_batch("DROP TRIGGER refuse_activity")
        .unwrap();
    assert!(scan.round(&mut store, &root, cutoff, &names, 8192).complete);
    let after = Store::read_response(&mut read, &query, &request, Some(FIRST), &names).unwrap();
    assert!(after["updatedAt"].as_i64().unwrap() > before["updatedAt"].as_i64().unwrap());
    assert!(after["updatedAt"].as_i64().unwrap() > 1_700_000_000_000);
    assert_eq!(after["activityPagination"]["total"], 2);
    assert_eq!(after["activityRecords"][0]["call"]["call_id"], "mcp");
    assert_eq!(after["activityRecords"][1]["call"]["call_id"], "skill");
    assert_eq!(
        after["activityRecords"][1]["call"]["arguments"]["singleRead"],
        false
    );
    assert_eq!(after["providerNames"]["custom"], "Example Relay");
    assert_eq!(after["scopedUsage"]["days"], before["days"]);
    let unchanged = json!({"activityRevision":after["updatedAt"]});
    let same = Store::read_response(&mut read, &query, &unchanged, None, &names).unwrap();
    assert_eq!(same["activityRecords"], json!([]));
    assert_eq!(same["activityPagination"]["unchanged"], true);

    let archives = root.join("archived_sessions");
    fs::create_dir(&archives).unwrap();
    let copy = archives.join("rollout-01.jsonl");
    fs::copy(&source, &copy).unwrap();
    assert!(scan.round(&mut store, &root, cutoff, &names, 8192).complete);
    let copied = Store::read_response(&mut read, &query, &request, None, &names).unwrap();
    assert_eq!(copied["activityRecords"], after["activityRecords"]);
    fs::remove_file(&source).unwrap();
    assert!(scan.round(&mut store, &root, cutoff, &names, 8192).complete);
    let moved = Store::read_response(&mut read, &query, &request, None, &names).unwrap();
    assert_eq!(moved["activityRecords"], after["activityRecords"]);

    let next = NaiveDate::from_ymd_opt(2026, 1, 4).unwrap();
    assert!(scan.round(&mut store, &root, next, &names, 8192).complete);
    let count: i64 = store
        .db
        .query_row("SELECT count(*) FROM token_activity", [], |row| row.get(0))
        .unwrap();
    let expired = Store::read_response(&mut read, &query, &request, None, &names).unwrap();
    assert_eq!(count, 0);
    assert_eq!(expired["activityRecords"], json!([]));
    assert_eq!(expired["days"], after["days"]);
}

#[test]
fn response_includes_scoped_trend_and_preserves_archived_day_totals() {
    use crate::usage_history::Query;
    use chrono::Local;

    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let sessions = root.join("sessions");
    fs::create_dir(&sessions).unwrap();
    let date = NaiveDate::from_ymd_opt(2026, 1, 2).unwrap();
    let time = |day: NaiveDate, hour, minute| {
        day.and_hms_opt(hour, minute, 0)
            .unwrap()
            .and_local_timezone(Local)
            .single()
            .unwrap()
    };
    fs::write(
        sessions.join("rollout-01.jsonl"),
        rollout(FIRST).replace("2026-01-02T12:01:00Z", &time(date, 12, 1).to_rfc3339()),
    )
    .unwrap();
    fs::write(
        sessions.join("rollout-02.jsonl"),
        rollout(SECOND).replace("2026-01-02T12:01:00Z", &time(date, 12, 45).to_rfc3339()),
    )
    .unwrap();
    let mut store = Store::open(Connection::open_in_memory().unwrap()).unwrap();
    let mut scan = Scan::default();
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let names = BTreeMap::new();
    assert!(scan.round(&mut store, &root, cutoff, &names, 8192).complete);
    let request = json!({"startAt":time(date, 12, 0).timestamp_millis(),
        "endAt":time(date, 13, 0).timestamp_millis(), "includeTrend":true});
    let query = Query::parse(&request, cutoff).unwrap();
    let result =
        Store::read_response(&mut store.db, &query, &request, Some(FIRST), &names).unwrap();
    assert_eq!(result["days"][0]["usage"]["totalTokens"], 220);
    assert_eq!(
        result["scopedUsage"]["days"][0]["usage"]["totalTokens"],
        110
    );
    assert_eq!(result["coverage"]["precision"], "exact");
    assert_eq!(result["coverage"]["days"].as_object().unwrap().len(), 1);
    assert_eq!(result["trend"]["intervalMs"], 1_800_000);
    assert_eq!(result["trend"]["buckets"][0]["usage"]["totalTokens"], 110);
    assert_eq!(result["trend"]["buckets"][1]["usage"]["totalTokens"], 110);
    assert_eq!(
        result["scopedUsage"]["trend"]["buckets"][1]["usage"]["totalTokens"],
        0
    );
    assert_eq!(result["trend"]["buckets"][1]["endAt"], request["endAt"]);
    assert!(result.get("activityRecords").is_none());

    let day_request = json!({
        "startAt":time(date, 0, 0).timestamp_millis(),
        "endAt":time(date.succ_opt().unwrap(), 0, 0).timestamp_millis(),
    });
    let day =
        Store::read_response(&mut store.db, &query, &day_request, Some(FIRST), &names).unwrap();
    assert_eq!(day["coverage"]["precision"], "exact");
    assert_eq!(day["days"], result["days"]);
    assert_eq!(day["scopedUsage"]["days"], result["scopedUsage"]["days"]);
    assert!(day.get("trend").is_none());

    let next = NaiveDate::from_ymd_opt(2026, 1, 4).unwrap();
    assert!(scan.round(&mut store, &root, next, &names, 8192).complete);
    let expired =
        Store::read_response(&mut store.db, &query, &request, Some(FIRST), &names).unwrap();
    assert_eq!(expired["coverage"]["precision"], "day");
    assert_eq!(expired["archiveRecovery"]["status"], "pending");
    assert_eq!(expired["countStatus"]["complete"], false);
    assert_eq!(expired["trend"], json!(null));
    let calendar =
        Store::read_response(&mut store.db, &query, &day_request, Some(FIRST), &names).unwrap();
    assert_eq!(calendar["coverage"]["precision"], "day");
    assert_eq!(calendar["rangeApplied"], true);
    assert_eq!(calendar["days"][0]["usage"]["totalTokens"], 220);
}
