use super::*;
use chrono::Local;
use serde_json::json;
use std::io::Write;

const SESSION: &str = "00000000-0000-4000-8000-000000000010";

fn usage(timestamp: &str, input: u64) -> String {
    json!({"type":"event_msg","timestamp":timestamp,"payload":{
        "type":"token_count","info":{
            "total_token_usage":{"input_tokens":input,"cached_input_tokens":0,"output_tokens":input / 10},
            "last_token_usage":{"input_tokens":10,"cached_input_tokens":0,"output_tokens":1}
        }
    }})
    .to_string()
}

fn tick(controller: &mut Controller, instant: Instant, now_ms: i64) -> ScanReport {
    controller
        .tick(
            instant,
            now_ms,
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            &BTreeMap::new(),
        )
        .unwrap()
        .unwrap()
}

fn total(controller: &Controller) -> i64 {
    controller
        .index
        .db
        .query_row("SELECT SUM(input) FROM usage_daily", [], |row| row.get(0))
        .unwrap()
}

#[test]
fn scans_only_changed_sources_and_reports_partial_coverage() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    fs::create_dir_all(home.join("sessions")).unwrap();
    let path = home.join("sessions/rollout-test.jsonl");
    let timestamp = Local::now().to_rfc3339();
    let header = json!({"type":"session_meta","payload":{"id":SESSION}});
    fs::write(&path, format!("{header}\n{}\n", usage(&timestamp, 10))).unwrap();
    let mut controller = Controller::open(&home, &root.path().join("index.sqlite")).unwrap();
    let start = Instant::now();
    let now_ms = Local::now().timestamp_millis();
    let first = tick(&mut controller, start, now_ms);
    assert!(first.failures.is_empty());
    assert!(first.discovery_finished);
    assert!(!first.coverage_complete);
    assert_eq!(first.processed_sources, 1);
    assert_eq!(total(&controller), 10);
    assert!(
        controller
            .tick(
                start + Duration::from_secs(4),
                now_ms,
                NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
                &BTreeMap::new()
            )
            .unwrap()
            .is_none()
    );
    let unchanged = tick(&mut controller, start + Duration::from_secs(5), now_ms);
    assert_eq!(unchanged.revision, first.revision);
    assert_eq!(unchanged.bytes_read, 0);
    assert_eq!(total(&controller), 10);
    let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
    writeln!(file, "{}", usage(&timestamp, 20)).unwrap();
    drop(file);
    let changed = tick(&mut controller, start + Duration::from_secs(10), now_ms);
    assert!(changed.failures.is_empty());
    assert!(changed.revision > unchanged.revision);
    assert!(changed.bytes_read > 0);
    assert_eq!(total(&controller), 20);
}

#[test]
fn failed_source_keeps_committed_data_and_is_retried() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    fs::create_dir_all(home.join("sessions")).unwrap();
    let path = home.join("sessions/rollout-test.jsonl");
    let timestamp = Local::now().to_rfc3339();
    let header = json!({"type":"session_meta","payload":{"id":SESSION}});
    fs::write(&path, format!("{header}\n{}\n", usage(&timestamp, 10))).unwrap();
    let mut controller = Controller::open(&home, &root.path().join("index.sqlite")).unwrap();
    let start = Instant::now();
    let now_ms = Local::now().timestamp_millis();
    let first = tick(&mut controller, start, now_ms);
    let (offset, checkpoint): (i64, Vec<u8>) = controller
        .index
        .db
        .query_row(
            "SELECT offset, parser_state FROM usage_sources",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    controller
        .index
        .db
        .execute("UPDATE usage_sources SET parser_state = X'FF'", [])
        .unwrap();
    let failed = tick(&mut controller, start + Duration::from_secs(5), now_ms);
    assert_eq!(failed.failures.len(), 1);
    assert!(!failed.coverage_complete);
    assert_eq!(failed.revision, first.revision);
    assert_eq!(total(&controller), 10);
    let stored: i64 = controller
        .index
        .db
        .query_row("SELECT offset FROM usage_sources", [], |row| row.get(0))
        .unwrap();
    assert_eq!(stored, offset);
    controller
        .index
        .db
        .execute("UPDATE usage_sources SET parser_state = ?1", [&checkpoint])
        .unwrap();
    let retried = tick(&mut controller, start + Duration::from_secs(10), now_ms);
    assert!(retried.failures.is_empty());
    assert_eq!(retried.revision, first.revision);
    assert_eq!(total(&controller), 10);
}

#[test]
fn oversized_index_reports_paused_ingestion_and_keeps_committed_data() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    fs::create_dir_all(home.join("sessions")).unwrap();
    let path = home.join("sessions/rollout-test.jsonl");
    let timestamp = Local::now().to_rfc3339();
    let header = json!({"type":"session_meta","payload":{"id":SESSION}});
    fs::write(&path, format!("{header}\n{}\n", usage(&timestamp, 10))).unwrap();
    let database = root.path().join("index.sqlite");
    let mut controller = Controller::open(&home, &database).unwrap();
    let start = Instant::now();
    let now_ms = Local::now().timestamp_millis();
    let first = tick(&mut controller, start, now_ms);
    let baseline_pages: u64 = controller
        .index
        .db
        .pragma_query_value(None, "page_count", |row| row.get(0))
        .unwrap();
    let page_size: u64 = controller
        .index
        .db
        .pragma_query_value(None, "page_size", |row| row.get(0))
        .unwrap();
    controller
        .index
        .db
        .execute_batch(
            "CREATE TABLE synthetic_padding(value BLOB);
             INSERT INTO synthetic_padding VALUES (zeroblob(1048576));",
        )
        .unwrap();
    let padded_pages: u64 = controller
        .index
        .db
        .pragma_query_value(None, "page_count", |row| row.get(0))
        .unwrap();
    assert!(padded_pages > baseline_pages + 1);
    let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
    writeln!(file, "{}", usage(&timestamp, 20)).unwrap();
    drop(file);

    let budget = (baseline_pages + 1) * page_size * 3;
    controller.index = PersistentIndex::open_with_budget(&database, budget).unwrap();
    let paused = tick(&mut controller, start + Duration::from_secs(5), now_ms);
    assert!(paused.storage_error.as_deref().unwrap().contains("paused"));
    assert_eq!(paused.revision, first.revision);
    assert_eq!(paused.processed_sources, 0);
    assert_eq!(total(&controller), 10);

    let reader = rusqlite::Connection::open(&database).unwrap();
    reader
        .execute_batch("DROP TABLE synthetic_padding; VACUUM;")
        .unwrap();
    drop(reader);
    let waiting = tick(&mut controller, start + Duration::from_secs(10), now_ms);
    assert!(waiting.storage_error.is_some());
    assert!(controller.index.budget_blocked);
    let resumed = tick(&mut controller, start + Duration::from_secs(65), now_ms);
    assert!(resumed.storage_error.is_none());
    assert!(resumed.failures.is_empty());
    assert_eq!(total(&controller), 20);
    assert!(resumed.revision > paused.revision);
    assert_eq!(controller.index.budget_bytes, budget);
}

#[test]
fn full_index_pauses_the_round_and_retries_the_same_source() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    fs::create_dir_all(home.join("sessions")).unwrap();
    let path = home.join("sessions/rollout-test.jsonl");
    let now = Local::now();
    let header = json!({"type":"session_meta","payload":{"id":SESSION}});
    fs::write(&path, format!("{header}\n{}\n", usage(&now.to_rfc3339(), 10))).unwrap();
    let database = root.path().join("index.sqlite");
    let mut controller = Controller::open(&home, &database).unwrap();
    let start = Instant::now();
    let first = tick(&mut controller, start, now.timestamp_millis());
    let source = scan::encode_source(&fs::canonicalize(&path).unwrap());
    let offset = controller.index.source_state(&source).unwrap().unwrap().offset;
    let max_pages: u64 = controller
        .index
        .db
        .pragma_query_value(None, "max_page_count", |row| row.get(0))
        .unwrap();
    let pages: u64 = controller
        .index
        .db
        .pragma_query_value(None, "page_count", |row| row.get(0))
        .unwrap();
    controller
        .index
        .db
        .pragma_update(None, "max_page_count", pages)
        .unwrap();
    let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
    for item in 1..=500 {
        let timestamp = (now + chrono::Duration::milliseconds(item)).to_rfc3339();
        writeln!(file, "{}", usage(&timestamp, 10 + 10 * item as u64)).unwrap();
    }
    drop(file);

    let paused = tick(&mut controller, start + Duration::from_secs(5), now.timestamp_millis());
    assert!(paused.storage_error.is_some());
    assert_eq!(paused.failures.len(), 1);
    assert_eq!(paused.revision, first.revision);
    assert_eq!(controller.index.source_state(&source).unwrap().unwrap().offset, offset);
    assert_eq!(total(&controller), 10);
    assert!(controller.active.is_some());
    assert_eq!(controller.retry_read_bytes, Some(2 * 1024 * 1024));

    controller
        .index
        .db
        .pragma_update(None, "max_page_count", max_pages)
        .unwrap();
    let resumed = tick(&mut controller, start + Duration::from_secs(10), now.timestamp_millis());
    assert!(resumed.storage_error.is_none());
    assert!(resumed.failures.is_empty());
    assert!(resumed.revision > first.revision);
    assert_eq!(total(&controller), 5_010);
    assert!(controller.index.source_state(&source).unwrap().unwrap().offset > offset);
}

#[test]
fn storage_retry_does_not_pause_expired_execution_compaction() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    fs::create_dir_all(home.join("sessions")).unwrap();
    let path = home.join("sessions/rollout-test.jsonl");
    let now = Local::now();
    let old = now - chrono::Duration::days(70);
    let header = json!({"type":"session_meta","payload":{"id":SESSION}});
    let call = json!({"type":"response_item","timestamp":old.to_rfc3339(),"payload":{
        "type":"function_call","call_id":"old-call","name":"functions.exec_command"
    }});
    fs::write(&path, format!("{header}\n{}\n{call}\n", usage(&now.to_rfc3339(), 10))).unwrap();
    let mut controller = Controller::open(&home, &root.path().join("index.sqlite")).unwrap();
    let start = Instant::now();
    tick(&mut controller, start, now.timestamp_millis());
    let source = scan::encode_source(&fs::canonicalize(&path).unwrap());
    let offset = controller.index.source_state(&source).unwrap().unwrap().offset;
    controller.last_compact = None;
    controller.storage_error = Some("synthetic full index".into());
    controller.storage_retry_at = Some(start + Duration::from_secs(60));

    let report = tick(&mut controller, start + Duration::from_secs(5), now.timestamp_millis());
    assert!(report.storage_error.is_some());
    assert_eq!(report.bytes_read, 0);
    assert_eq!(controller.index.source_state(&source).unwrap().unwrap().offset, offset);
    assert_eq!(total(&controller), 10);
    assert_eq!(
        controller.index.db.query_row(
            "SELECT SUM(count) FROM execution_daily",
            [],
            |row| row.get::<_, i64>(0),
        ).unwrap(),
        1
    );
}

#[test]
fn full_detail_reclaim_does_not_block_other_maintenance() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    fs::create_dir_all(home.join("sessions")).unwrap();
    let path = home.join("sessions/rollout-test.jsonl");
    let now = Local::now();
    let old = now - chrono::Duration::days(70);
    let header = json!({"type":"session_meta","payload":{"id":SESSION}});
    let old_call = json!({"type":"response_item","timestamp":old.to_rfc3339(),"payload":{
        "type":"function_call","call_id":"old-call","name":"functions.exec_command"
    }});
    let call = json!({"type":"response_item","timestamp":now.to_rfc3339(),"payload":{
        "type":"function_call","call_id":"reclaim-call","name":"functions.exec_command",
        "arguments":"{\"cmd\":\"pwd\"}"
    }});
    let output = json!({"type":"response_item","timestamp":now.to_rfc3339(),"payload":{
        "type":"function_call_output","call_id":"reclaim-call","output":"done"
    }});
    fs::write(&path, format!(
        "{header}\n{}\n{old_call}\n{call}\n{output}\n", usage(&now.to_rfc3339(), 10)
    )).unwrap();
    let mut controller = Controller::open(&home, &root.path().join("index.sqlite")).unwrap();
    let start = Instant::now();
    let now_ms = now.timestamp_millis();
    assert!(tick(&mut controller, start, now_ms).failures.is_empty());
    let source = scan::encode_source(&fs::canonicalize(&path).unwrap());
    let offset = controller.index.source_state(&source).unwrap().unwrap().offset;
    controller.index.db.execute_batch("CREATE TABLE synthetic_padding(value BLOB)").unwrap();
    let pages: u64 = controller.index.db.pragma_query_value(None, "page_count", |row| row.get(0)).unwrap();
    controller.index.db.pragma_update(None, "max_page_count", pages + 32).unwrap();
    controller.index.db.execute_batch(
        "CREATE TEMP TRIGGER force_reclaim_full BEFORE UPDATE OF end ON execution_observations
         WHEN OLD.call_id = 'reclaim-call' AND EXISTS (
             SELECT 1 FROM execution_observations
             WHERE session = OLD.session AND call_id = OLD.call_id AND is_output = 1
         )
         BEGIN INSERT INTO synthetic_padding VALUES (zeroblob(1048576)); END;"
    ).unwrap();
    controller.payload_compaction_done = true;
    controller.last_purge = None;
    controller.last_compact = None;
    controller.storage_error = Some("synthetic full index".into());
    controller.storage_retry_at = Some(start + Duration::from_secs(60));
    let report = tick(
        &mut controller,
        start + Duration::from_secs(5),
        now_ms + chrono::Duration::days(8).num_milliseconds(),
    );
    assert!(report.storage_error.as_deref().unwrap().contains("full"));
    assert_eq!(report.bytes_read, 0);
    assert_eq!(controller.detail_reclaim_retry_at, Some(start + Duration::from_secs(65)));
    assert_eq!(controller.storage_retry_at, Some(start + Duration::from_secs(65)));
    assert_eq!(controller.index.source_state(&source).unwrap().unwrap().offset, offset);
    assert_eq!(total(&controller), 10);
    assert_eq!(controller.index.db.query_row(
        "SELECT COUNT(*) FROM execution_observations WHERE call_id = 'reclaim-call' AND is_output = 1",
        [], |row| row.get::<_, i64>(0)
    ).unwrap(), 0);
    assert_eq!(controller.index.db.query_row(
        "SELECT SUM(count) FROM execution_daily", [], |row| row.get::<_, i64>(0)
    ).unwrap(), 1);
}

#[test]
fn storage_pressure_restarts_compaction_before_retrying_an_incomplete_source() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    fs::create_dir_all(home.join("sessions")).unwrap();
    let path = home.join("sessions/rollout-pressure.jsonl");
    let now = Local::now();
    let old = now - chrono::Duration::days(70);
    let header = json!({"type":"session_meta","payload":{"id":SESSION}});
    let call = json!({"type":"response_item","timestamp":old.to_rfc3339(),"payload":{
        "type":"function_call","call_id":"old-call","name":"functions.exec_command"
    }});
    fs::write(&path, format!("{header}\n{}\n{call}\n", usage(&now.to_rfc3339(), 10))).unwrap();
    let mut controller = Controller::open(&home, &root.path().join("index.sqlite")).unwrap();
    let start = Instant::now();
    tick(&mut controller, start, now.timestamp_millis());
    let source = scan::encode_source(&fs::canonicalize(&path).unwrap());
    let offset = controller.index.source_state(&source).unwrap().unwrap().offset;
    controller.index.db.execute("UPDATE usage_sources SET complete = 0 WHERE source = ?1", [&source]).unwrap();
    controller.last_compact = Some(start);
    controller.storage_error = Some("synthetic full index".into());
    controller.storage_retry_at = Some(start + Duration::from_secs(60));

    let report = tick(&mut controller, start + Duration::from_secs(5), now.timestamp_millis());
    assert!(report.storage_error.is_some());
    assert_eq!(report.bytes_read, 0);
    assert_eq!(total(&controller), 10);
    assert_eq!(controller.index.source_state(&source).unwrap().unwrap().offset, offset);
    let (count, partial): (i64, bool) = controller.index.db.query_row(
        "SELECT SUM(count), MAX(partial) FROM execution_daily", [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(count, 1);
    assert!(partial);
}

#[test]
fn full_index_recovers_by_compacting_incomplete_old_tools_before_resuming_ingestion() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    fs::create_dir_all(home.join("sessions")).unwrap();
    let path = home.join("sessions/rollout-full-pressure.jsonl");
    let now = Local::now();
    let old = now - chrono::Duration::days(70);
    let header = json!({"type":"session_meta","payload":{"id":SESSION}});
    let mut file = fs::File::create(&path).unwrap();
    writeln!(file, "{header}").unwrap();
    writeln!(file, "{}", usage(&now.to_rfc3339(), 10)).unwrap();
    for index in 0..2_000 {
        let call = json!({"type":"response_item","timestamp":old.to_rfc3339(),"payload":{
            "type":"function_call","call_id":format!("old-call-{index}"),"name":"functions.exec_command"
        }});
        writeln!(file, "{call}").unwrap();
    }
    drop(file);
    let database = root.path().join("index.sqlite");
    let mut controller = Controller::open(&home, &database).unwrap();
    let start = Instant::now();
    let first = tick(&mut controller, start, now.timestamp_millis());
    assert!(first.failures.is_empty());
    let source = scan::encode_source(&fs::canonicalize(&path).unwrap());
    controller.index.db.execute("UPDATE usage_sources SET complete = 0 WHERE source = ?1", [&source]).unwrap();
    let offset = controller.index.source_state(&source).unwrap().unwrap().offset;
    let pages: u64 = controller.index.db.pragma_query_value(None, "page_count", |row| row.get(0)).unwrap();
    controller.index.db.pragma_update(None, "max_page_count", pages).unwrap();
    let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
    for index in 1..=500 {
        let timestamp = (now + chrono::Duration::milliseconds(index)).to_rfc3339();
        writeln!(file, "{}", usage(&timestamp, 10 + 10 * index as u64)).unwrap();
    }
    drop(file);

    let paused = tick(&mut controller, start + Duration::from_secs(5), now.timestamp_millis());
    assert!(paused.storage_error.is_some());
    assert_eq!(controller.index.source_state(&source).unwrap().unwrap().offset, offset);
    assert_eq!(total(&controller), 10);
    let mut recovered = false;
    for round in 2..80 {
        let report = tick(&mut controller, start + Duration::from_secs(round * 5), now.timestamp_millis());
        if total(&controller) == 5_010 {
            assert!(report.storage_error.is_none());
            recovered = true;
            break;
        }
    }
    assert!(recovered, "old partial summaries should release pages for the committed source");
    assert!(controller.index.source_state(&source).unwrap().unwrap().offset > offset);
    let (count, partial): (i64, bool) = controller.index.db.query_row(
        "SELECT SUM(count), MAX(partial) FROM execution_daily", [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert!(count > 0);
    assert!(partial);
    drop(controller);
    let reopened = Controller::open(&home, &database).unwrap();
    assert_eq!(total(&reopened), 5_010);
}

#[test]
fn full_index_preserves_recent_tool_locations_while_ingestion_is_paused() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    fs::create_dir_all(home.join("sessions")).unwrap();
    let path = home.join("sessions/rollout-full-recent-tools.jsonl");
    let now = Local::now();
    let old = now - chrono::Duration::days(2);
    let mut file = fs::File::create(&path).unwrap();
    writeln!(file, "{}", json!({"type":"session_meta","payload":{"id":SESSION}})).unwrap();
    writeln!(file, "{}", usage(&now.to_rfc3339(), 10)).unwrap();
    for index in 0..2_000 {
        let id = format!("tool-{index}");
        writeln!(file, "{}", json!({"type":"response_item","timestamp":old.to_rfc3339(),
            "payload":{"type":"function_call","call_id":id,"name":"functions.exec_command",
                "arguments":"{\"cmd\":\"pwd\"}"}})).unwrap();
        writeln!(file, "{}", json!({"type":"response_item","timestamp":old.to_rfc3339(),
            "payload":{"type":"function_call_output","call_id":id,"output":"done"}})).unwrap();
    }
    drop(file);
    let database = root.path().join("index.sqlite");
    let mut controller = Controller::open(&home, &database).unwrap();
    let start = Instant::now();
    let source = scan::encode_source(&fs::canonicalize(&path).unwrap());
    for step in 0..20 {
        tick(&mut controller, start + Duration::from_secs(step * 5), now.timestamp_millis());
        if controller.index.source_state(&source).unwrap().is_some_and(|state| state.complete) {
            break;
        }
    }
    let offset = controller.index.source_state(&source).unwrap().unwrap().offset;
    assert_eq!(offset, fs::metadata(&path).unwrap().len());
    let calls: i64 = controller.index.db.query_row(
        "SELECT COUNT(*) FROM execution_observations WHERE is_output = 0", [], |row| row.get(0)
    ).unwrap();
    assert_eq!(calls, 2_000);
    let pages: u64 = controller.index.db.pragma_query_value(None, "page_count", |row| row.get(0)).unwrap();
    controller.index.db.pragma_update(None, "max_page_count", pages).unwrap();
    let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
    for index in 1..=500 {
        writeln!(file, "{}", usage(&(now + chrono::Duration::milliseconds(index)).to_rfc3339(),
            10 + 10 * index as u64)).unwrap();
    }
    drop(file);

    let paused = tick(&mut controller, start + Duration::from_secs(105), now.timestamp_millis());
    assert!(paused.storage_error.is_some());
    assert_eq!(controller.index.source_state(&source).unwrap().unwrap().offset, offset);
    assert_eq!(total(&controller), 10);
    controller.storage_retry_at = Some(start + Duration::from_secs(600));
    for step in 22..30 {
        let report = tick(&mut controller, start + Duration::from_secs(step * 5), now.timestamp_millis());
        assert!(report.storage_error.is_some());
        assert_eq!(controller.index.source_state(&source).unwrap().unwrap().offset, offset);
        assert_eq!(total(&controller), 10);
    }
    let (remaining_calls, call_locations, output_locations): (i64, i64, i64) = controller.index.db.query_row(
        "SELECT SUM(is_output = 0), SUM(is_output = 0 AND end IS NOT NULL),
                SUM(is_output = 1 AND end IS NOT NULL) FROM execution_observations",
        [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))
    ).unwrap();
    assert_eq!(remaining_calls, calls);
    assert_eq!(call_locations, calls);
    assert_eq!(output_locations, calls);
    drop(controller);
    let reopened = Controller::open(&home, &database).unwrap();
    assert_eq!(total(&reopened), 10);
}

#[cfg(windows)]
#[test]
fn legacy_source_migration_resumes_a_real_session_after_restart() {
    use std::os::windows::ffi::OsStrExt;

    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    fs::create_dir_all(home.join("sessions")).unwrap();
    let path = home.join("sessions/rollout-test.jsonl");
    let now = Local::now();
    let header = json!({"type":"session_meta","payload":{"id":SESSION}});
    let call = json!({"type":"response_item","timestamp":now.to_rfc3339(),"payload":{
        "type":"function_call","call_id":"tool-call","name":"functions.exec_command",
        "arguments":"{\"cmd\":\"pwd\"}"
    }});
    fs::write(&path, format!("{header}\n{}\n{call}\n", usage(&now.to_rfc3339(), 10))).unwrap();
    let database = root.path().join("index.sqlite");
    let mut controller = Controller::open(&home, &database).unwrap();
    let start = Instant::now();
    let first = tick(&mut controller, start, now.timestamp_millis());
    assert!(first.failures.is_empty());
    let path = fs::canonicalize(path).unwrap();
    let new = scan::encode_source(&path);
    let mut old = vec![b'W'];
    for unit in path.as_os_str().encode_wide() {
        old.extend_from_slice(&unit.to_le_bytes());
    }
    let offset = controller.index.source_state(&new).unwrap().unwrap().offset;
    {
        let tx = controller.index.db.transaction().unwrap();
        tx.pragma_update(None, "defer_foreign_keys", true).unwrap();
        for table in [
            "usage_observations", "execution_observations", "usage_scope_parents",
            "usage_scope_aliases", "usage_count_gaps", "usage_unindexed_checkpoints",
            "usage_sealed_sessions", "execution_compacted_sources", "usage_sources",
        ] {
            tx.execute(
                &format!("UPDATE {table} SET source = ?1 WHERE source = ?2"),
                rusqlite::params![old, new],
            ).unwrap();
        }
        tx.execute_batch("DROP TABLE usage_source_migration; PRAGMA user_version = 12;").unwrap();
        tx.commit().unwrap();
    }
    drop(controller);
    let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
    writeln!(file, "{}", usage(&(now + chrono::Duration::seconds(1)).to_rfc3339(), 20)).unwrap();
    drop(file);

    let mut reopened = Controller::open(&home, &database).unwrap();
    for step in 1..=30 {
        let report = tick(
            &mut reopened,
            start + Duration::from_secs(step * 5),
            now.timestamp_millis(),
        );
        assert!(report.failures.is_empty());
        assert!(report.storage_error.is_none());
        if reopened.index.source_state(&new).unwrap().is_some_and(|saved| saved.offset > offset) {
            break;
        }
    }
    assert!(!reopened.migrating_sources());
    assert!(reopened.index.source_state(&old).unwrap().is_none());
    assert!(reopened.index.source_state(&new).unwrap().unwrap().offset > offset);
    assert_eq!(total(&reopened), 20);
    assert_eq!(reopened.index.db.query_row(
        "SELECT COUNT(*) FROM execution_observations WHERE is_output = 0",
        [],
        |row| row.get::<_, i64>(0),
    ).unwrap(), 1);
}

#[cfg(windows)]
#[test]
fn full_legacy_source_migration_compresses_before_moving_keys() {
    use std::os::windows::ffi::OsStrExt;

    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    fs::create_dir_all(home.join("sessions")).unwrap();
    let path = home.join("sessions/rollout-test.jsonl");
    let now = Local::now();
    let mut contents = format!("{}\n", json!({"type":"session_meta","payload":{"id":SESSION}}));
    for item in 1..=1200 {
        contents.push_str(&usage(
            &(now + chrono::Duration::milliseconds(item)).to_rfc3339(),
            item as u64 * 10,
        ));
        contents.push('\n');
    }
    fs::write(&path, contents).unwrap();
    let path = fs::canonicalize(path).unwrap();
    let database = root.path().join("index.sqlite");
    let mut controller = Controller::open(&home, &database).unwrap();
    let start = Instant::now();
    let first = tick(&mut controller, start, now.timestamp_millis());
    assert!(first.failures.is_empty());
    assert_eq!(total(&controller), 12_000);
    let new = scan::encode_source(&path);
    let mut old = vec![b'W'];
    for unit in path.as_os_str().encode_wide() {
        old.extend_from_slice(&unit.to_le_bytes());
    }
    let offset = controller.index.source_state(&new).unwrap().unwrap().offset;
    let observations = controller.index.db.prepare(
        "SELECT position, payload FROM usage_observations WHERE source = ?1 ORDER BY position",
    ).unwrap().query_map([&new], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?))
    }).unwrap().collect::<Result<Vec<_>, _>>().unwrap();
    assert_eq!(observations.len(), 1200);
    {
        let tx = controller.index.db.transaction().unwrap();
        tx.pragma_update(None, "defer_foreign_keys", true).unwrap();
        for (position, encoded) in observations {
            assert_eq!(encoded[0], 0);
            let length = u32::from_le_bytes(encoded[1..5].try_into().unwrap()) as usize;
            let plain = zstd::bulk::decompress(&encoded[5..], length).unwrap();
            tx.execute(
                "UPDATE usage_observations SET payload = ?3 WHERE source = ?1 AND position = ?2",
                rusqlite::params![new, position, plain],
            ).unwrap();
        }
        for table in [
            "usage_observations", "execution_observations", "usage_scope_parents",
            "usage_scope_aliases", "usage_count_gaps", "usage_unindexed_checkpoints",
            "usage_sealed_sessions", "execution_compacted_sources", "usage_sources",
        ] {
            tx.execute(
                &format!("UPDATE {table} SET source = ?1 WHERE source = ?2"),
                rusqlite::params![old, new],
            ).unwrap();
        }
        tx.execute(
            "INSERT INTO execution_observations
             (source, start, session, call_id, timestamp_ms, is_output)
             VALUES (?1, 1, ?2, 'expired-output', ?3, 1)",
            rusqlite::params![old, SESSION, now.timestamp_millis() - 8 * 86_400_000],
        ).unwrap();
        tx.execute(
            "INSERT INTO usage_source_migration VALUES (1, X'', X'')",
            [],
        ).unwrap();
        tx.execute_batch(
            "CREATE TABLE synthetic_padding(value BLOB); PRAGMA user_version = 13;",
        ).unwrap();
        tx.commit().unwrap();
    }
    controller.index.repack_usage_payloads().unwrap();
    let pages: u64 = controller.index.db.pragma_query_value(None, "page_count", |row| row.get(0)).unwrap();
    assert_eq!(controller.index.db.pragma_query_value(None, "freelist_count", |row| row.get::<_, u64>(0)).unwrap(), 0);
    drop(controller);
    let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
    writeln!(file, "{}", usage(&(now + chrono::Duration::seconds(2)).to_rfc3339(), 12_010)).unwrap();
    drop(file);

    let mut reopened = Controller::open(&home, &database).unwrap();
    reopened.index.db.pragma_update(None, "max_page_count", pages).unwrap();
    let error = reopened.index.db.execute(
        "INSERT INTO synthetic_padding VALUES (zeroblob(1048576))",
        [],
    ).unwrap_err();
    assert!(matches!(error, rusqlite::Error::SqliteFailure(ref inner, _)
        if inner.code == rusqlite::ErrorCode::DiskFull));
    reopened.index.db.execute_batch(
        "CREATE TEMP TRIGGER force_full BEFORE UPDATE OF payload ON usage_observations
         BEGIN INSERT INTO synthetic_padding VALUES (zeroblob(1048576)); END;",
    ).unwrap();
    let failed = tick(&mut reopened, start + Duration::from_secs(1), now.timestamp_millis());
    assert!(failed.storage_error.is_some());
    assert!(reopened.payload_compaction_cursor.is_none());
    assert_eq!(reopened.payload_compaction_retry_at, Some(start + Duration::from_secs(61)));
    reopened.index.db.execute_batch("DROP TRIGGER force_full").unwrap();
    let waiting = tick(&mut reopened, start + Duration::from_secs(6), now.timestamp_millis());
    assert!(waiting.storage_error.is_some());
    assert!(reopened.payload_compaction_cursor.is_none());
    assert_eq!(reopened.index.db.query_row(
        "SELECT COUNT(*) FROM execution_observations WHERE call_id = 'expired-output'",
        [],
        |row| row.get::<_, i64>(0),
    ).unwrap(), 0);
    let report = tick(&mut reopened, start + Duration::from_secs(61), now.timestamp_millis());
    assert!(report.migrating_sources);
    assert!(reopened.payload_compaction_cursor.is_some());
    assert!(reopened.index.db.query_row(
        "SELECT COUNT(*) FROM usage_observations WHERE substr(payload, 1, 1) = X'00'",
        [],
        |row| row.get::<_, i64>(0),
    ).unwrap() > 0);
    assert_eq!(reopened.index.source_state(&old).unwrap().unwrap().offset, offset);
    assert_eq!(total(&reopened), 12_000);

    for step in 1..=80 {
        let report = tick(&mut reopened, start + Duration::from_secs(61 + step * 5), now.timestamp_millis());
        assert!(report.failures.is_empty(), "migration failures: {}", report.failures.len());
        if reopened.index.source_state(&new).unwrap().is_some_and(|saved| saved.offset > offset) {
            assert!(report.storage_error.is_none(), "{:?}", report.storage_error);
            break;
        }
    }
    assert!(!reopened.migrating_sources());
    assert!(reopened.index.source_state(&old).unwrap().is_none());
    assert!(reopened.index.source_state(&new).unwrap().unwrap().offset > offset);
    assert_eq!(total(&reopened), 12_010);
    assert!(reopened.index.db.pragma_query_value(None, "page_count", |row| row.get::<_, u64>(0)).unwrap() <= pages);

    drop(reopened);
    let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
    writeln!(file, "{}", usage(&(now + chrono::Duration::seconds(3)).to_rfc3339(), 12_020)).unwrap();
    drop(file);
    let mut cold = Controller::open(&home, &database).unwrap();
    assert!(!cold.migrating_sources());
    let report = tick(&mut cold, start + Duration::from_secs(500), now.timestamp_millis());
    assert!(report.failures.is_empty());
    assert!(!cold.payload_compaction_done);
    assert_eq!(total(&cold), 12_020);
}

#[cfg(windows)]
#[test]
fn source_migration_respects_short_interval_and_storage_backoff() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    fs::create_dir_all(home.join("sessions")).unwrap();
    let mut controller = Controller::open(&home, &root.path().join("index.sqlite")).unwrap();
    let start = Instant::now();
    controller.index.db.execute(
        "INSERT INTO usage_source_migration VALUES (1, X'', X'')",
        [],
    ).unwrap();
    controller.index.source_migration_complete = false;
    controller.payload_compaction_done = true;
    controller.storage_error = Some("synthetic full index".into());
    controller.storage_retry_at = Some(start + Duration::from_secs(60));
    let waiting = tick(&mut controller, start, Local::now().timestamp_millis());
    assert!(waiting.migrating_sources);
    assert!(waiting.storage_error.is_some());
    assert!(controller.tick(
        start + Duration::from_secs(1),
        Local::now().timestamp_millis(),
        NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        &BTreeMap::new(),
    ).unwrap().is_none());
    controller.storage_retry_at = None;
    let resumed = tick(&mut controller, start + Duration::from_secs(1), Local::now().timestamp_millis());
    assert!(!resumed.migrating_sources);
}

#[test]
fn full_batch_shrinks_until_a_committed_prefix_fits() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    fs::create_dir_all(home.join("sessions")).unwrap();
    let path = home.join("sessions/rollout-test.jsonl");
    let now = Local::now();
    let header = json!({"type":"session_meta","payload":{"id":SESSION}});
    fs::write(&path, format!("{header}\n{}\n", usage(&now.to_rfc3339(), 10))).unwrap();
    let database = root.path().join("index.sqlite");
    let mut controller = Controller::open(&home, &database).unwrap();
    let start = Instant::now();
    let first = tick(&mut controller, start, now.timestamp_millis());
    let source = scan::encode_source(&fs::canonicalize(&path).unwrap());
    let original = controller.index.source_state(&source).unwrap().unwrap().offset;
    let pages: u64 = controller
        .index
        .db
        .pragma_query_value(None, "page_count", |row| row.get(0))
        .unwrap();
    controller
        .index
        .db
        .pragma_update(None, "max_page_count", pages + 32)
        .unwrap();
    let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
    for item in 1..=2_000 {
        let timestamp = (now + chrono::Duration::milliseconds(item)).to_rfc3339();
        writeln!(file, "{}", usage(&timestamp, 10 + 10 * item as u64)).unwrap();
    }
    drop(file);

    let mut successful = false;
    let mut previous_limit = 4 * 1024 * 1024;
    for round in 1..=12 {
        let report = tick(
            &mut controller,
            start + Duration::from_secs(5 * round),
            now.timestamp_millis(),
        );
        if report.storage_error.is_none() && report.revision > first.revision {
            successful = true;
            break;
        }
        assert!(report.storage_error.is_some());
        assert_eq!(report.revision, first.revision);
        assert_eq!(controller.index.source_state(&source).unwrap().unwrap().offset, original);
        let limit = controller.retry_read_bytes.unwrap();
        assert!(limit <= previous_limit);
        previous_limit = limit;
    }
    assert!(successful, "the smaller batch did not commit into available pages");
    assert!(previous_limit < 4 * 1024 * 1024);
    assert!(controller.index.source_state(&source).unwrap().unwrap().offset > original);
    assert!(total(&controller) > 10);
}

#[test]
fn larger_budget_resumes_usage_and_tool_ingestion_after_full_index() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    fs::create_dir_all(home.join("sessions")).unwrap();
    let path = home.join("sessions/rollout-test.jsonl");
    let database = root.path().join("index.sqlite");
    let now = Local::now();
    let header = json!({"type":"session_meta","payload":{"id":SESSION}});
    fs::write(&path, format!("{header}\n{}\n", usage(&now.to_rfc3339(), 10))).unwrap();
    let start = Instant::now();
    let mut controller = Controller::open(&home, &database).unwrap();
    tick(&mut controller, start, now.timestamp_millis());
    let source = scan::encode_source(&fs::canonicalize(&path).unwrap());
    let offset = controller.index.source_state(&source).unwrap().unwrap().offset;
    let pages: u64 = controller.index.db.pragma_query_value(None, "page_count", |row| row.get(0)).unwrap();
    controller.index.db.pragma_update(None, "max_page_count", pages).unwrap();
    let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
    for item in 1..=2_000 {
        let timestamp = (now + chrono::Duration::milliseconds(item)).to_rfc3339();
        writeln!(file, "{}", usage(&timestamp, 10 + 10 * item as u64)).unwrap();
    }
    writeln!(file, "{}", json!({"type":"response_item","timestamp":now.to_rfc3339(),"payload":{
        "type":"function_call","call_id":"new-tool","name":"functions.exec_command"
    }})).unwrap();
    drop(file);
    let stopped = tick(&mut controller, start + Duration::from_secs(5), now.timestamp_millis());
    assert!(stopped.storage_error.is_some());
    assert_eq!(controller.index.source_state(&source).unwrap().unwrap().offset, offset);
    assert_eq!(total(&controller), 10);
    drop(controller);

    let mut reopened = Controller::open(&home, &database).unwrap();
    for round in 0..30 {
        let report = tick(&mut reopened, start + Duration::from_secs(10 + round * 5), now.timestamp_millis());
        assert!(report.storage_error.is_none());
        if reopened.index.source_state(&source).unwrap().unwrap().offset == fs::metadata(&path).unwrap().len() {
            break;
        }
    }
    assert_eq!(reopened.index.source_state(&source).unwrap().unwrap().offset, fs::metadata(&path).unwrap().len());
    assert_eq!(total(&reopened), 20_010);
    let tools: i64 = reopened.index.db.query_row(
        "SELECT COUNT(*) FROM execution_observations WHERE call_id = 'new-tool' AND is_output = 0",
        [],
        |row| row.get(0),
    ).unwrap();
    assert_eq!(tools, 1);
}

#[test]
fn storage_warning_uses_reusable_pages_and_clears_with_headroom() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    fs::create_dir_all(home.join("sessions")).unwrap();
    let mut controller = Controller::open(&home, &root.path().join("index.sqlite")).unwrap();
    let pages: u64 = controller.index.db.pragma_query_value(None, "page_count", |row| row.get(0)).unwrap();
    let size: u64 = controller.index.db.pragma_query_value(None, "page_size", |row| row.get(0)).unwrap();
    let start = Instant::now();
    let now_ms = Local::now().timestamp_millis();
    controller.index.budget_bytes = pages * size * 3;
    assert!(tick(&mut controller, start, now_ms).storage_near_limit);
    controller.index.budget_bytes = pages * size * 3 * 4;
    assert!(!tick(&mut controller, start + Duration::from_secs(5), now_ms).storage_near_limit);
}

#[test]
fn storage_pressure_starts_before_full_and_clears_below_target() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    fs::create_dir_all(home.join("sessions")).unwrap();
    let mut controller = Controller::open(&home, &root.path().join("index.sqlite")).unwrap();
    let start = Instant::now();
    let now_ms = Local::now().timestamp_millis();
    tick(&mut controller, start, now_ms);

    let pages: u64 = controller.index.db.pragma_query_value(None, "page_count", |row| row.get(0)).unwrap();
    let free: u64 = controller.index.db.pragma_query_value(None, "freelist_count", |row| row.get(0)).unwrap();
    let size: u64 = controller.index.db.pragma_query_value(None, "page_size", |row| row.get(0)).unwrap();
    let used = pages - free;
    controller.index.budget_bytes = used * size * 3 * 100 / 91;
    tick(&mut controller, start + Duration::from_secs(5), now_ms);
    assert!(controller.compaction_pressure);

    controller.index.budget_bytes = used * size * 3 * 100 / 80;
    tick(&mut controller, start + Duration::from_secs(10), now_ms);
    assert!(controller.compaction_pressure);

    controller.index.budget_bytes = used * size * 3 * 100 / 70;
    tick(&mut controller, start + Duration::from_secs(15), now_ms);
    assert!(!controller.compaction_pressure);
}

#[test]
fn periodically_expires_tool_details_without_source_changes() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    fs::create_dir_all(home.join("sessions")).unwrap();
    let path = home.join("sessions/rollout-test.jsonl");
    let now = Local::now();
    let timestamp = now.to_rfc3339();
    let header = json!({"type":"session_meta","payload":{"id":SESSION}});
    let call = json!({"type":"response_item","timestamp":timestamp,"payload":{
        "type":"function_call","call_id":"call-1","name":"functions.exec_command"
    }});
    fs::write(
        &path,
        format!("{header}\n{}\n{call}\n", usage(&timestamp, 10)),
    )
    .unwrap();
    let mut controller = Controller::open(&home, &root.path().join("index.sqlite")).unwrap();
    let start = Instant::now();
    let now_ms = now.timestamp_millis();
    let first = tick(&mut controller, start, now_ms);
    let before: Option<String> = controller
        .index
        .db
        .query_row("SELECT name FROM execution_observations", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(before.as_deref(), Some("functions.exec_command"));
    let later = tick(
        &mut controller,
        start + Duration::from_secs(60),
        now_ms + 7 * 86_400_000,
    );
    assert!(later.failures.is_empty());
    assert_eq!(later.revision, first.revision + 1);
    let after: (Option<String>, Option<i64>) = controller
        .index
        .db
        .query_row("SELECT name, end FROM execution_observations", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap();
    assert_eq!(after, (Some("functions.exec_command".into()), None));
    assert_eq!(total(&controller), 10);
}

#[test]
fn continues_expiry_on_each_refresh_until_backlog_is_cleared() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    fs::create_dir_all(home.join("sessions")).unwrap();
    let path = home.join("sessions/rollout-test.jsonl");
    let now = Local::now();
    let timestamp = now.to_rfc3339();
    let header = json!({"type":"session_meta","payload":{"id":SESSION}});
    let call = json!({"type":"response_item","timestamp":timestamp,"payload":{
        "type":"function_call","call_id":"call-1","name":"functions.exec_command"
    }});
    fs::write(
        &path,
        format!("{header}\n{}\n{call}\n", usage(&timestamp, 10)),
    )
    .unwrap();
    let mut controller = Controller::open(&home, &root.path().join("index.sqlite")).unwrap();
    let start = Instant::now();
    let now_ms = now.timestamp_millis();
    let first = tick(&mut controller, start, now_ms);
    controller
        .index
        .db
        .execute(
            "WITH RECURSIVE sequence(n) AS (
                 VALUES(1) UNION ALL SELECT n + 1 FROM sequence WHERE n < 600
             )
             INSERT INTO execution_observations(
                 source, start, end, session, call_id, timestamp_ms, is_output
             )
             SELECT source, 100000 + n, 100001 + n, ?1, 'call-1', ?2, 1
             FROM usage_sources, sequence",
            rusqlite::params![SESSION, now_ms],
        )
        .unwrap();
    let mut previous = 600;
    let mut revision = first.revision;
    for round in 0..4 {
        let report = tick(
            &mut controller,
            start + Duration::from_secs(60 + round * 5),
            now_ms + 7 * 86_400_000,
        );
        assert!(report.failures.is_empty());
        let remaining: i64 = controller
            .index
            .db
            .query_row(
                "SELECT COUNT(*) FROM execution_observations WHERE is_output = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        if previous > 0 {
            assert!(remaining < previous);
            assert!(previous - remaining <= 256);
            assert_eq!(report.revision, revision + i64::from(remaining == 0));
        } else {
            assert_eq!(report.revision, revision);
        }
        previous = remaining;
        revision = report.revision;
    }
    assert_eq!(previous, 0);
    let after: (Option<String>, Option<i64>) = controller
        .index
        .db
        .query_row(
            "SELECT name, end FROM execution_observations WHERE is_output = 0",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(after, (Some("functions.exec_command".into()), None));
    assert_eq!(total(&controller), 10);
}

#[test]
fn old_execution_compaction_recovers_from_failure_and_resumes_after_restart() {
    use crate::usage_history::structured::execution_index::{self, ExecutionQuery, PageLookup};

    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    fs::create_dir_all(home.join("sessions")).unwrap();
    let path = home.join("sessions/rollout-test.jsonl");
    let now = Local::now();
    let old = now - chrono::Duration::days(70);
    let now_ms = now.timestamp_millis();
    let old_ms = old.timestamp_millis();
    let header = json!({"type":"session_meta","payload":{"id":SESSION}});
    let calls = (0..300)
        .map(|i| {
            json!({"type":"response_item","timestamp":old.to_rfc3339(),"payload":{
                "type":"function_call","call_id":format!("old-{i:03}"),
                "name":"functions.exec_command"
            }})
            .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(
        &path,
        format!("{header}\n{}\n{calls}\n", usage(&now.to_rfc3339(), 10)),
    )
    .unwrap();
    let database = root.path().join("index.sqlite");
    let mut controller = Controller::open(&home, &database).unwrap();
    let start = Instant::now();
    let first = tick(&mut controller, start, now_ms);
    let source = scan::encode_source(&fs::canonicalize(&path).unwrap());
    let offset = controller.index.source_state(&source).unwrap().unwrap().offset;
    let check_count = |controller: &Controller| {
        let request = ExecutionQuery {
            start_ms: old_ms - 1,
            end_ms: now_ms + 1,
            sessions: None,
            limit: 1,
            cursor: None,
            revision: None,
            search: "",
            kind: None,
            now_ms,
        };
        let PageLookup::Ready(page) = execution_index::query(&controller.index.db, &request).unwrap()
        else {
            panic!("execution page was unexpectedly stale");
        };
        let daily = execution_index::daily_summary(
            &controller.index.db,
            old.date_naive(),
            old.date_naive().succ_opt().unwrap(),
            None,
        )
        .unwrap();
        page.total + daily.iter().map(|item| item.count).sum::<u64>()
    };
    assert_eq!(check_count(&controller), 300);
    controller
        .index
        .db
        .execute_batch(
            "CREATE TRIGGER fail_compaction BEFORE INSERT ON execution_daily
             BEGIN SELECT RAISE(ABORT, 'synthetic compaction failure'); END;",
        )
        .unwrap();
    let failed = controller.tick(
        start + Duration::from_secs(60),
        now_ms,
        NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        &BTreeMap::new(),
    );
    assert!(format!("{:#}", failed.err().unwrap()).contains("synthetic compaction failure"));
    assert_eq!(controller.index.revision().unwrap(), first.revision);
    assert_eq!(controller.index.source_state(&source).unwrap().unwrap().offset, offset);
    assert_eq!(check_count(&controller), 300);
    controller
        .index
        .db
        .execute_batch("DROP TRIGGER fail_compaction")
        .unwrap();
    let compacted = tick(&mut controller, start + Duration::from_secs(65), now_ms);
    assert_eq!(compacted.revision, first.revision + 1);
    assert_eq!(check_count(&controller), 300);
    assert_eq!(
        controller
            .index
            .db
            .query_row("SELECT COUNT(*) FROM execution_observations", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        44
    );

    drop(controller);
    let mut controller = Controller::open(&home, &database).unwrap();
    let resumed = tick(&mut controller, start + Duration::from_secs(70), now_ms);
    assert_eq!(resumed.revision, compacted.revision + 1);
    assert_eq!(controller.index.source_state(&source).unwrap().unwrap().offset, offset);
    assert_eq!(check_count(&controller), 300);
    assert_eq!(
        controller
            .index
            .db
            .query_row("SELECT COUNT(*) FROM execution_observations", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        0
    );
    assert_eq!(tick(&mut controller, start + Duration::from_secs(75), now_ms).revision, resumed.revision);
}

#[test]
fn compaction_controller_resumes_scanning_and_retries_a_failed_batch() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    fs::create_dir_all(home.join("sessions")).unwrap();
    let mut controller = Controller::open(&home, &root.path().join("index.sqlite")).unwrap();
    let now_ms = Local::now().timestamp_millis();
    let old_ms = now_ms - 70 * 24 * 60 * 60 * 1000;
    controller
        .index
        .db
        .execute_batch(
            "INSERT INTO usage_sources(
                 source, identity, offset, parser_state, complete, execution_indexed, execution_valid, scope_valid
             ) VALUES
                 (X'61', X'01', 0, X'', 1, 1, 1, 1),
                 (X'62', X'02', 0, X'', 0, 1, 1, 1);",
        )
        .unwrap();
    controller
        .index
        .db
        .execute(
            "WITH RECURSIVE sequence(n) AS (
                 VALUES(1) UNION ALL SELECT n + 1 FROM sequence WHERE n < 2500
             )
             INSERT INTO execution_observations(
                 source, start, session, call_id, timestamp_ms, is_output
             )
             SELECT X'62', n, ?1, printf('pending-%05d', n), ?2 + n, 0
             FROM sequence",
            rusqlite::params![SESSION, old_ms],
        )
        .unwrap();
    controller
        .index
        .db
        .execute(
            "INSERT INTO execution_observations(
                 source, start, session, call_id, timestamp_ms, is_output
             ) VALUES (X'61', 0, ?1, 'completed', ?2, 0)",
            rusqlite::params![SESSION, old_ms + 3_000],
        )
        .unwrap();
    let start = Instant::now();
    let first = tick(&mut controller, start, now_ms);
    assert!(controller.compaction_cursor.is_some());
    let second = tick(&mut controller, start + Duration::from_secs(5), now_ms);
    assert_eq!(second.revision, first.revision);
    assert!(controller.compaction_cursor.is_some());

    controller
        .index
        .db
        .execute_batch(
            "CREATE TRIGGER fail_compaction BEFORE INSERT ON execution_daily
             BEGIN SELECT RAISE(ABORT, 'synthetic compaction failure'); END;",
        )
        .unwrap();
    let failed = controller.tick(
        start + Duration::from_secs(10),
        now_ms,
        NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        &BTreeMap::new(),
    );
    assert!(format!("{:#}", failed.err().unwrap()).contains("synthetic compaction failure"));
    assert_eq!(controller.index.revision().unwrap(), first.revision);
    assert!(controller.compaction_cursor.is_some());
    controller
        .index
        .db
        .execute_batch("DROP TRIGGER fail_compaction")
        .unwrap();
    let resumed = tick(&mut controller, start + Duration::from_secs(15), now_ms);
    assert_eq!(resumed.revision, first.revision + 1);
    assert!(controller.compaction_cursor.is_none());

    controller
        .index
        .db
        .execute("UPDATE usage_sources SET complete = 1 WHERE source = X'62'", [])
        .unwrap();
    for round in 0..12 {
        tick(
            &mut controller,
            start + Duration::from_secs(75 + round * 5),
            now_ms,
        );
        if controller.compaction_cursor.is_none() {
            break;
        }
    }
    let (retained, summarized): (u64, u64) = controller
        .index
        .db
        .query_row(
            "SELECT (SELECT COUNT(*) FROM execution_observations),
                    (SELECT COALESCE(SUM(count), 0) FROM execution_daily)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!((retained, summarized), (0, 2_501));
}

#[test]
fn oversized_duplicate_cleanup_retries_without_invalidating_unchanged_pages() {
    use crate::usage_history::structured::execution_index::{self, ExecutionQuery, PageLookup};

    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    fs::create_dir_all(home.join("sessions")).unwrap();
    let mut controller = Controller::open(&home, &root.path().join("index.sqlite")).unwrap();
    let now_ms = Local::now().timestamp_millis();
    let old_ms = now_ms - 70 * 24 * 60 * 60 * 1000;
    controller
        .index
        .db
        .execute_batch(
            "INSERT INTO usage_sources(
                 source, identity, offset, parser_state, complete, execution_indexed, execution_valid, scope_valid
             ) VALUES
                 (X'61', X'01', 0, X'', 1, 1, 1, 1),
                 (X'62', X'02', 0, X'', 1, 1, 1, 1);",
        )
        .unwrap();
    controller
        .index
        .db
        .execute(
            "WITH RECURSIVE sequence(n) AS (
                 VALUES(1) UNION ALL SELECT n + 1 FROM sequence WHERE n < 600
             )
             INSERT INTO execution_observations(
                 source, start, session, call_id, timestamp_ms, is_output
             )
             SELECT X'61', n, ?1, 'duplicate', ?2, 0 FROM sequence",
            rusqlite::params![SESSION, old_ms],
        )
        .unwrap();
    controller
        .index
        .db
        .execute(
            "INSERT INTO execution_observations(
                 source, start, session, call_id, timestamp_ms, is_output, end, name, call_kind
             ) VALUES(X'62', 1, ?1, 'recent', ?2, 0, 2, 'tool', 'tool')",
            rusqlite::params![SESSION, now_ms],
        )
        .unwrap();
    let request = ExecutionQuery {
        start_ms: old_ms - 1,
        end_ms: now_ms + 1,
        sessions: None,
        limit: 1,
        cursor: None,
        revision: Some(0),
        search: "",
        kind: None,
        now_ms,
    };
    let start = Instant::now();
    let first = tick(&mut controller, start, now_ms);
    assert_eq!(first.revision, 0);
    let PageLookup::Ready(page) = execution_index::query(&controller.index.db, &request).unwrap()
    else {
        panic!("identical duplicate cleanup invalidated an unchanged page");
    };
    assert_eq!(page.total, 2);
    assert_eq!(page.items[0].id, "recent");
    let remaining = |controller: &Controller| -> u64 {
        controller
            .index
            .db
            .query_row(
                "SELECT COUNT(*) FROM execution_observations WHERE call_id = 'duplicate'",
                [],
                |row| row.get(0),
            )
            .unwrap()
    };
    assert_eq!(remaining(&controller), 344);
    let sweep = tick(&mut controller, start + Duration::from_secs(5), now_ms);
    assert_eq!(sweep.revision, 0);
    assert_eq!(remaining(&controller), 344);

    controller
        .index
        .db
        .execute_batch(
            "CREATE TRIGGER fail_duplicate_cleanup BEFORE DELETE ON execution_observations
             WHEN OLD.call_id = 'duplicate'
             BEGIN SELECT RAISE(ABORT, 'synthetic duplicate cleanup failure'); END;",
        )
        .unwrap();
    let failed = controller.tick(
        start + Duration::from_secs(10),
        now_ms,
        NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        &BTreeMap::new(),
    );
    assert!(format!("{:#}", failed.err().unwrap()).contains("synthetic duplicate cleanup failure"));
    assert_eq!(remaining(&controller), 344);
    assert_eq!(controller.index.revision().unwrap(), 0);
    controller
        .index
        .db
        .execute_batch("DROP TRIGGER fail_duplicate_cleanup")
        .unwrap();
    for round in 3..12 {
        tick(
            &mut controller,
            start + Duration::from_secs(round * 5),
            now_ms,
        );
        if remaining(&controller) == 0 {
            break;
        }
    }
    assert_eq!(remaining(&controller), 0);
    assert_eq!(
        controller
            .index
            .db
            .query_row("SELECT SUM(count) FROM execution_daily", [], |row| {
                row.get::<_, u64>(0)
            })
            .unwrap(),
        1
    );
    assert_eq!(controller.index.revision().unwrap(), 1);
}

#[test]
fn full_index_reuses_compacted_execution_pages_without_raising_the_limit() {
    assert_full_index_recovers_execution_pages(false);
}

#[test]
fn full_index_recovers_when_completed_logs_have_missing_tool_output() {
    assert_full_index_recovers_execution_pages(true);
}

fn assert_full_index_recovers_execution_pages(incomplete: bool) {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    fs::create_dir_all(home.join("sessions")).unwrap();
    let path = home.join("sessions/rollout-test.jsonl");
    let now = Local::now();
    let old = now - chrono::Duration::days(70);
    let header = json!({"type":"session_meta","payload":{"id":SESSION}});
    let calls = (0..6_000)
        .map(|i| {
            json!({"type":"response_item","timestamp":old.to_rfc3339(),"payload":{
                "type":"function_call","call_id":format!("old-{i:05}"),
                "name":"functions.exec_command"
            }})
            .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(
        &path,
        format!("{header}\n{}\n{calls}\n", usage(&now.to_rfc3339(), 10)),
    )
    .unwrap();
    if incomplete {
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file, "{}", json!({"type":"response_item","timestamp":old.to_rfc3339(),
            "payload":{"type":"function_call_output","output":"missing call identity"}})).unwrap();
    }
    let mut controller = Controller::open(&home, &root.path().join("index.sqlite")).unwrap();
    let start = Instant::now();
    let now_ms = now.timestamp_millis();
    tick(&mut controller, start, now_ms);
    let source = scan::encode_source(&fs::canonicalize(&path).unwrap());
    let offset = controller.index.source_state(&source).unwrap().unwrap().offset;
    let pages: u64 = controller
        .index
        .db
        .pragma_query_value(None, "page_count", |row| row.get(0))
        .unwrap();
    controller
        .index
        .db
        .pragma_update(None, "max_page_count", pages)
        .unwrap();
    let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
    for i in 1..=2_000 {
        let timestamp = (now + chrono::Duration::milliseconds(i)).to_rfc3339();
        writeln!(file, "{}", usage(&timestamp, 10 + 10 * i as u64)).unwrap();
    }
    drop(file);
    let paused = tick(&mut controller, start + Duration::from_secs(5), now_ms);
    assert!(paused.storage_error.is_some());
    assert_eq!(controller.index.source_state(&source).unwrap().unwrap().offset, offset);
    assert_eq!(total(&controller), 10);

    let mut resumed = false;
    for seconds in (60..=180).step_by(5) {
        let report = tick(&mut controller, start + Duration::from_secs(seconds), now_ms);
        let current = controller.index.source_state(&source).unwrap().unwrap().offset;
        if current > offset {
            assert!(report.storage_error.is_none());
            resumed = true;
            break;
        }
    }
    assert!(resumed, "compacted pages did not allow a committed prefix");
    let current_limit: u64 = controller
        .index
        .db
        .pragma_query_value(None, "max_page_count", |row| row.get(0))
        .unwrap();
    assert_eq!(current_limit, pages);
    let retained: u64 = controller
        .index
        .db
        .query_row(
            "SELECT COUNT(DISTINCT call_id) FROM execution_observations WHERE is_output = 0",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let summarized: u64 = controller
        .index
        .db
        .query_row("SELECT COALESCE(SUM(count), 0) FROM execution_daily", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(retained + summarized, 6_000);
    let partial: bool = controller.index.db.query_row(
        "SELECT EXISTS(SELECT 1 FROM execution_daily WHERE partial = 1)", [], |row| row.get(0),
    ).unwrap();
    assert_eq!(partial, incomplete);
}

#[test]
fn expired_output_cleanup_does_not_invalidate_an_execution_page() {
    use crate::usage_history::structured::execution_index::{self, ExecutionQuery, PageLookup};

    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    fs::create_dir_all(home.join("sessions")).unwrap();
    let path = home.join("sessions/rollout-test.jsonl");
    let now = Local::now();
    let timestamp = now.to_rfc3339();
    let header = json!({"type":"session_meta","payload":{"id":SESSION}});
    let call = |id| {
        json!({"type":"response_item","timestamp":timestamp,"payload":{
            "type":"function_call","call_id":id,"name":"functions.exec_command"
        }})
    };
    fs::write(
        &path,
        format!("{header}\n{}\n{}\n{}\n", call("a"), call("b"), call("c")),
    )
    .unwrap();
    let mut controller = Controller::open(&home, &root.path().join("index.sqlite")).unwrap();
    let start = Instant::now();
    let now_ms = now.timestamp_millis();
    let first = tick(&mut controller, start, now_ms);
    controller
        .index
        .db
        .execute(
            "WITH RECURSIVE sequence(n) AS (
                 VALUES(1) UNION ALL SELECT n + 1 FROM sequence WHERE n < 600
             )
             INSERT INTO execution_observations(
                 source, start, end, session, call_id, timestamp_ms, is_output
             )
             SELECT source, 100000 + n, 100001 + n, ?1, 'a', ?2, 1
             FROM usage_sources, sequence",
            rusqlite::params![SESSION, now_ms - 7 * 86_400_000],
        )
        .unwrap();
    let mut request = ExecutionQuery {
        start_ms: now_ms - 86_400_000,
        end_ms: now_ms + 1,
        sessions: None,
        limit: 1,
        cursor: None,
        revision: Some(first.revision),
        search: "",
        kind: None,
        now_ms,
    };
    let PageLookup::Ready(page) = execution_index::query(&controller.index.db, &request).unwrap()
    else {
        panic!("first page was unexpectedly stale");
    };
    let cursor = page.next_cursor.unwrap();
    let report = tick(&mut controller, start + Duration::from_secs(60), now_ms);
    assert!(report.failures.is_empty());
    request.cursor = Some(&cursor);
    assert!(matches!(
        execution_index::query(&controller.index.db, &request).unwrap(),
        PageLookup::Ready(_)
    ));
}

#[test]
fn compressed_source_keeps_its_reader_across_bounded_rounds() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    fs::create_dir_all(home.join("sessions")).unwrap();
    let path = home.join("sessions/rollout-test.jsonl.zst");
    let timestamp = Local::now().to_rfc3339();
    let header = json!({"type":"session_meta","payload":{"id":SESSION}});
    let filler = format!(
        "{}\n",
        json!({"type":"response_item","payload":{"type":"message","content":"x".repeat(1024)}})
    );
    let contents = format!(
        "{header}\n{}\n{}{}\n",
        usage(&timestamp, 10),
        filler.repeat(4300),
        usage(&timestamp, 20)
    );
    fs::write(
        &path,
        zstd::stream::encode_all(contents.as_bytes(), 1).unwrap(),
    )
    .unwrap();
    let mut controller = Controller::open(&home, &root.path().join("index.sqlite")).unwrap();
    let start = Instant::now();
    let now_ms = Local::now().timestamp_millis();
    let first = tick(&mut controller, start, now_ms);
    assert!(first.failures.is_empty());
    assert!(controller.active.is_some());
    assert_eq!(total(&controller), 10);
    let second = tick(&mut controller, start + Duration::from_secs(5), now_ms);
    assert!(second.failures.is_empty());
    assert!(controller.active.is_none());
    assert_eq!(total(&controller), 20);
}

#[test]
fn priority_source_preempts_and_resumes_a_compressed_reader() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    fs::create_dir_all(home.join("sessions")).unwrap();
    let background = home.join("sessions/rollout-background.jsonl.zst");
    let current = home.join("sessions/rollout-current.jsonl");
    let timestamp = Local::now().to_rfc3339();
    let header = json!({"type":"session_meta","payload":{"id":SESSION}});
    let filler = format!(
        "{}\n",
        json!({"type":"response_item","payload":{"type":"message","content":"x".repeat(1024)}})
    );
    let contents = format!(
        "{header}\n{}\n{}{}\n",
        usage(&timestamp, 10),
        filler.repeat(4500),
        usage(&timestamp, 20)
    );
    fs::write(
        &background,
        zstd::stream::encode_all(contents.as_bytes(), 1).unwrap(),
    )
    .unwrap();
    let current_header = json!({"type":"session_meta","payload":{
        "id":"00000000-0000-4000-8000-000000000011"
    }});
    fs::write(
        &current,
        format!("{current_header}\n{}\n", usage(&timestamp, 10)),
    )
    .unwrap();
    let mut controller = Controller::open(&home, &root.path().join("index.sqlite")).unwrap();
    let start = Instant::now();
    let now_ms = Local::now().timestamp_millis();
    controller.prioritize_source(&background).unwrap();
    let first = tick(&mut controller, start, now_ms);
    assert!(first.failures.is_empty());
    assert!(controller.active.as_ref().unwrap().1.reader.is_some());
    assert_eq!(total(&controller), 10);

    controller.prioritize_source(&current).unwrap();
    let second = tick(&mut controller, start + Duration::from_secs(5), now_ms);
    assert!(second.failures.is_empty());
    assert!(second.bytes_read < 2 * 1024 * 1024);
    assert!(controller.suspended.is_none());
    assert_eq!(total(&controller), 30);
}

#[test]
fn full_priority_source_does_not_block_new_current_source() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    fs::create_dir_all(home.join("sessions")).unwrap();
    let background = home.join("sessions/rollout-background.jsonl.zst");
    let failing = home.join("sessions/rollout-failing.jsonl");
    let current = home.join("sessions/rollout-current.jsonl");
    let timestamp = Local::now().to_rfc3339();
    let filler = format!(
        "{}\n",
        json!({"type":"response_item","payload":{"type":"message","content":"x".repeat(1024)}})
    );
    let header = json!({"type":"session_meta","payload":{"id":SESSION}});
    let contents = format!(
        "{header}\n{}\n{}{}\n",
        usage(&timestamp, 10),
        filler.repeat(4500),
        usage(&timestamp, 20)
    );
    fs::write(&background, zstd::stream::encode_all(contents.as_bytes(), 1).unwrap()).unwrap();
    let failing_id = "00000000-0000-4000-8000-000000000011";
    let failing_header = json!({"type":"session_meta","payload":{"id":failing_id}});
    fs::write(&failing, format!("{failing_header}\n{}\n", usage(&timestamp, 10))).unwrap();
    let current_header = json!({"type":"session_meta","payload":{
        "id":"00000000-0000-4000-8000-000000000012"
    }});
    fs::write(&current, format!("{current_header}\n{}\n", usage(&timestamp, 10))).unwrap();
    let background = fs::canonicalize(background).unwrap();
    let failing = fs::canonicalize(failing).unwrap();
    let current = fs::canonicalize(current).unwrap();

    let mut controller = Controller::open(&home, &root.path().join("index.sqlite")).unwrap();
    let start = Instant::now();
    let now_ms = Local::now().timestamp_millis();
    controller.prioritize_source(&background).unwrap();
    assert!(tick(&mut controller, start, now_ms).failures.is_empty());
    let background_key = scan::encode_source(&background);
    let background_offset = controller.index.source_state(&background_key).unwrap().unwrap().offset;
    assert_eq!(total(&controller), 10);
    assert!(controller.active.is_some());

    controller.index.db.execute_batch("CREATE TABLE synthetic_padding(value BLOB)").unwrap();
    let pages: u64 = controller.index.db.pragma_query_value(None, "page_count", |row| row.get(0)).unwrap();
    controller.index.db.pragma_update(None, "max_page_count", pages + 32).unwrap();
    controller.index.db.execute_batch(&format!(
        "CREATE TEMP TRIGGER force_full BEFORE INSERT ON usage_observations
         WHEN NEW.session = '{failing_id}'
         BEGIN INSERT INTO synthetic_padding VALUES (zeroblob(1048576)); END;"
    )).unwrap();
    controller.prioritize_source(&failing).unwrap();
    let failed = tick(&mut controller, start + Duration::from_secs(5), now_ms);
    assert!(failed.storage_error.is_some());
    assert!(controller.suspended.is_none());
    assert_eq!(controller.deferred.iter().filter(|path| *path == &background).count(), 1);
    assert_eq!(controller.index.source_state(&background_key).unwrap().unwrap().offset, background_offset);
    assert!(controller.index.source_state(&scan::encode_source(&failing)).unwrap().is_none());

    controller.prioritize_source(&current).unwrap();
    controller.prioritize_source(&current).unwrap();
    assert_eq!(controller.priority.iter().filter(|path| *path == &current).count(), 1);
    tick(&mut controller, start + Duration::from_secs(10), now_ms);
    assert!(controller.index.source_state(&scan::encode_source(&current)).unwrap().is_some());
    assert_eq!(total(&controller), 20);

    controller.prioritize_source(&background).unwrap();
    controller.prioritize_source(&background).unwrap();
    assert!(!controller.deferred.contains(&background));
    assert_eq!(controller.priority.iter().filter(|path| *path == &background).count(), 1);
    controller.index.db.execute_batch("DROP TRIGGER force_full").unwrap();
    controller.index.db.pragma_update(None, "max_page_count", pages + 512).unwrap();
    for step in 3..=12 {
        tick(&mut controller, start + Duration::from_secs(step * 5), now_ms);
        if total(&controller) == 40 {
            break;
        }
    }
    assert_eq!(total(&controller), 40);
    assert!(controller.index.source_state(&background_key).unwrap().unwrap().offset > background_offset);
    assert!(controller.index.source_state(&scan::encode_source(&failing)).unwrap().is_some());
    tick(&mut controller, start + Duration::from_secs(70), now_ms);
    assert_eq!(total(&controller), 40);
}

#[test]
fn priority_sources_preserve_caller_order() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    fs::create_dir_all(home.join("sessions")).unwrap();
    let timestamp = Local::now().to_rfc3339();
    let mut paths = Vec::new();
    for index in 10..13 {
        let path = home.join(format!("sessions/rollout-{index}.jsonl"));
        let id = format!("00000000-0000-4000-8000-{index:012}");
        let header = json!({"type":"session_meta","payload":{"id":id}});
        fs::write(&path, format!("{header}\n{}\n", usage(&timestamp, 10))).unwrap();
        paths.push(fs::canonicalize(path).unwrap());
    }
    let mut controller = Controller::open(&home, &root.path().join("index.sqlite")).unwrap();
    controller.index.db.execute_batch(
        "CREATE TEMP TABLE inserted_sources(source BLOB);
         CREATE TEMP TRIGGER record_source AFTER INSERT ON usage_sources
         BEGIN INSERT INTO inserted_sources VALUES (NEW.source); END;"
    ).unwrap();
    for path in &paths {
        controller.prioritize_source(path).unwrap();
    }
    let report = tick(&mut controller, Instant::now(), Local::now().timestamp_millis());
    assert!(report.failures.is_empty());
    let inserted: Vec<Vec<u8>> = controller.index.db.prepare(
        "SELECT source FROM inserted_sources ORDER BY rowid"
    ).unwrap().query_map([], |row| row.get(0)).unwrap().collect::<Result<_, _>>().unwrap();
    assert_eq!(inserted, paths.iter().map(|path| scan::encode_source(path)).collect::<Vec<_>>());
}

#[test]
fn priority_source_rejects_paths_outside_session_roots() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    fs::create_dir_all(home.join("sessions")).unwrap();
    let outside = root.path().join("rollout-outside.jsonl");
    fs::write(&outside, "{}\n").unwrap();
    let mut controller = Controller::open(&home, &root.path().join("index.sqlite")).unwrap();
    assert!(controller.prioritize_source(&outside).is_err());
    assert!(controller.priority.is_empty());
}

#[test]
fn priority_source_skips_unchanged_commits_but_accepts_new_bytes() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    fs::create_dir_all(home.join("sessions")).unwrap();
    let source = home.join("sessions/rollout-current.jsonl");
    let timestamp = Local::now().to_rfc3339();
    let header = json!({"type":"session_meta","payload":{"id":SESSION}});
    fs::write(&source, format!("{header}\n{}\n", usage(&timestamp, 10))).unwrap();
    let mut controller = Controller::open(&home, &root.path().join("index.sqlite")).unwrap();
    let start = Instant::now();
    let now_ms = Local::now().timestamp_millis();
    tick(&mut controller, start, now_ms);
    controller.prioritize_source(&source).unwrap();
    assert!(controller.priority.is_empty());

    let mut file = fs::OpenOptions::new().append(true).open(&source).unwrap();
    writeln!(file, "{}", usage(&timestamp, 20)).unwrap();
    drop(file);
    controller.prioritize_source(&source).unwrap();
    controller.prioritize_source(&source).unwrap();
    assert_eq!(controller.priority.len(), 1);
    let changed = tick(&mut controller, start + Duration::from_secs(5), now_ms);
    assert!(changed.failures.is_empty());
    assert_eq!(total(&controller), 20);
}

#[test]
fn usage_retention_preserves_real_parser_results_through_restart_and_source_changes() {
    use chrono::TimeZone;

    fn compare(left: &mut Controller, right: &mut Controller) {
        let request = json!({"period":"all"});
        let options = crate::usage_history::structured::Query::parse(
            &request,
            NaiveDate::from_ymd_opt(2026, 11, 10).unwrap(),
        )
        .unwrap();
        let left = crate::usage_history::structured::history_api::query(
            &mut left.index.db,
            &options,
            &request,
            &BTreeMap::new(),
        )
        .unwrap();
        let right = crate::usage_history::structured::history_api::query(
            &mut right.index.db,
            &options,
            &request,
            &BTreeMap::new(),
        )
        .unwrap();
        for field in ["days", "countStatus", "classificationStatus"] {
            assert_eq!(left[field], right[field], "{field}");
        }
    }

    let root = tempfile::tempdir().unwrap();
    let homes = [root.path().join("retained"), root.path().join("control")];
    let databases = [root.path().join("retained.sqlite"), root.path().join("control.sqlite")];
    let header = json!({"type":"session_meta","payload":{"id":SESSION}});
    let old = usage("2026-08-01T12:00:00+08:00", 10);
    let recent = usage("2026-09-09T12:00:00+08:00", 20);
    let appended = usage("2026-09-10T12:00:00+08:00", 30);
    for home in &homes {
        fs::create_dir_all(home.join("sessions")).unwrap();
        fs::write(
            home.join("sessions/rollout-primary.jsonl"),
            format!("{header}\n{old}\n{recent}\n"),
        )
        .unwrap();
    }
    let now = Local
        .with_ymd_and_hms(2026, 11, 10, 12, 0, 0)
        .single()
        .unwrap()
        .timestamp_millis();
    let control_now = Local
        .with_ymd_and_hms(2026, 9, 1, 12, 0, 0)
        .single()
        .unwrap()
        .timestamp_millis();
    let start = Instant::now();
    let mut retained = Controller::open(&homes[0], &databases[0]).unwrap();
    let mut control = Controller::open(&homes[1], &databases[1]).unwrap();
    assert!(tick(&mut retained, start, now).failures.is_empty());
    assert!(tick(&mut control, start, control_now).failures.is_empty());
    assert!(tick(&mut retained, start + Duration::from_secs(65), now).failures.is_empty());
    assert!(tick(&mut control, start + Duration::from_secs(65), control_now).failures.is_empty());
    assert_eq!(
        retained.index.db.query_row("SELECT COUNT(*) FROM usage_events", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        control.index.db.query_row("SELECT COUNT(*) FROM usage_events", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        2
    );
    compare(&mut retained, &mut control);
    drop(retained);
    drop(control);

    for home in &homes {
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(home.join("sessions/rollout-primary.jsonl"))
            .unwrap();
        writeln!(file, "{appended}").unwrap();
    }
    let mut retained = Controller::open(&homes[0], &databases[0]).unwrap();
    let mut control = Controller::open(&homes[1], &databases[1]).unwrap();
    assert!(tick(&mut retained, start + Duration::from_secs(70), now).failures.is_empty());
    assert!(tick(&mut control, start + Duration::from_secs(70), control_now).failures.is_empty());
    compare(&mut retained, &mut control);
    assert_eq!(total(&retained), 30);

    for home in &homes {
        fs::write(home.join("sessions/rollout-copy.jsonl"), format!("{header}\n{old}\n")).unwrap();
    }
    assert!(tick(&mut retained, start + Duration::from_secs(75), now).failures.is_empty());
    assert!(tick(&mut control, start + Duration::from_secs(75), control_now).failures.is_empty());
    compare(&mut retained, &mut control);
    assert_eq!(total(&retained), 30);
    assert_eq!(
        retained.index.db.query_row(
            "SELECT COUNT(*) FROM usage_events WHERE day = '2026-08-01'",
            [],
            |row| row.get::<_, i64>(0)
        ).unwrap(),
        0
    );

    for home in &homes {
        fs::write(home.join("sessions/rollout-primary.jsonl"), format!("{header}\n{recent}\n")).unwrap();
    }
    assert!(tick(&mut retained, start + Duration::from_secs(80), now).failures.is_empty());
    assert!(tick(&mut control, start + Duration::from_secs(80), control_now).failures.is_empty());
    compare(&mut retained, &mut control);
}

#[test]
fn usage_retention_resumes_a_partially_read_real_source() {
    use chrono::TimeZone;
    use std::fmt::Write as _;

    fn snapshot(controller: &mut Controller) -> serde_json::Value {
        let request = json!({"period":"all"});
        let options = crate::usage_history::structured::Query::parse(
            &request,
            NaiveDate::from_ymd_opt(2026, 11, 10).unwrap(),
        )
        .unwrap();
        crate::usage_history::structured::history_api::query(
            &mut controller.index.db,
            &options,
            &request,
            &BTreeMap::new(),
        )
        .unwrap()
    }

    let root = tempfile::tempdir().unwrap();
    let homes = [root.path().join("retained"), root.path().join("control")];
    let databases = [root.path().join("retained.sqlite"), root.path().join("control.sqlite")];
    let header = json!({"type":"session_meta","payload":{"id":SESSION}});
    let start_time = chrono::DateTime::parse_from_rfc3339("2026-08-01T12:00:00+08:00").unwrap();
    let first_row = usage(&start_time.to_rfc3339(), 10);
    let mut contents = format!("{header}\n{first_row}\n");
    for number in 2..=900 {
        let timestamp = (start_time + chrono::Duration::seconds(number - 1)).to_rfc3339();
        writeln!(contents, "{}", usage(&timestamp, number as u64 * 10)).unwrap();
    }
    let correction_time = (start_time + chrono::Duration::seconds(900)).to_rfc3339();
    let correction = json!({"type":"event_msg","timestamp":correction_time,"payload":{
        "type":"token_count","info":{
            "total_token_usage":{"input_tokens":9000,"cached_input_tokens":50,"output_tokens":900},
            "last_token_usage":{"input_tokens":10,"cached_input_tokens":0,"output_tokens":1}
        }
    }});
    let duplicate_time = (start_time + chrono::Duration::seconds(898)).to_rfc3339();
    let late_duplicate = usage(&duplicate_time, 8_990);
    writeln!(contents, "{correction}\n{late_duplicate}").unwrap();
    assert!(contents.len() > 16 * 1024);
    for home in &homes {
        fs::create_dir_all(home.join("sessions")).unwrap();
        fs::write(home.join("sessions/rollout-primary.jsonl"), &contents).unwrap();
    }

    let now = Local.with_ymd_and_hms(2026, 11, 10, 12, 0, 0).single().unwrap().timestamp_millis();
    let start = Instant::now();
    let mut retained = Controller::open(&homes[0], &databases[0]).unwrap();
    let mut control = Controller::open(&homes[1], &databases[1]).unwrap();
    for (controller, home) in [(&mut retained, &homes[0]), (&mut control, &homes[1])] {
        controller.retry_read_bytes = Some(16 * 1024);
        let report = tick(controller, start, now);
        assert!(report.failures.is_empty());
        let path = fs::canonicalize(home.join("sessions/rollout-primary.jsonl")).unwrap();
        let source = controller.index.source_state(&scan::encode_source(&path)).unwrap().unwrap();
        assert!(!source.complete);
        assert!(source.offset > 0 && source.offset < contents.len() as u64);
    }
    let compacted = retained.index.compact_usage_events(now, None).unwrap();
    assert!(compacted.deleted > 0);
    drop(retained);
    drop(control);

    let mut retained = Controller::open(&homes[0], &databases[0]).unwrap();
    let mut control = Controller::open(&homes[1], &databases[1]).unwrap();
    for (controller, home) in [(&mut retained, &homes[0]), (&mut control, &homes[1])] {
        let report = tick(controller, start + Duration::from_secs(5), now);
        assert!(report.failures.is_empty(), "{}", report.failures.len());
        let path = fs::canonicalize(home.join("sessions/rollout-primary.jsonl")).unwrap();
        let source = controller.index.source_state(&scan::encode_source(&path)).unwrap().unwrap();
        assert!(source.complete);
        assert_eq!(source.offset, contents.len() as u64);
        let observations: i64 = controller.index.db
            .query_row("SELECT COUNT(*) FROM usage_observations", [], |row| row.get(0))
            .unwrap();
        assert_eq!(observations, 902);
    }
    let before_copy = snapshot(&mut retained);
    assert!(!before_copy["days"].as_array().unwrap().is_empty());
    for field in ["days", "countStatus", "classificationStatus"] {
        assert_eq!(before_copy[field], snapshot(&mut control)[field], "{field}");
    }
    for home in &homes {
        fs::write(
            home.join("sessions/rollout-copy.jsonl"),
            format!("{header}\n{first_row}\n"),
        )
        .unwrap();
    }
    for (controller, home) in [(&mut retained, &homes[0]), (&mut control, &homes[1])] {
        let report = tick(controller, start + Duration::from_secs(10), now);
        assert!(report.failures.is_empty(), "{}", report.failures.len());
        let path = fs::canonicalize(home.join("sessions/rollout-copy.jsonl")).unwrap();
        assert!(controller.index.source_state(&scan::encode_source(&path)).unwrap().unwrap().complete);
    }
    for field in ["days", "countStatus", "classificationStatus"] {
        assert_eq!(snapshot(&mut retained)[field], snapshot(&mut control)[field], "{field}");
        assert_eq!(snapshot(&mut retained)[field], before_copy[field], "{field}");
    }
}
