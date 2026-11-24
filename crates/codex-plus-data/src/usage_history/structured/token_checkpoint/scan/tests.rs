use super::*;
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

    fs::rename(&source, &moved).unwrap();
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
