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
    let request = json!({"includeActivity":true});
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
