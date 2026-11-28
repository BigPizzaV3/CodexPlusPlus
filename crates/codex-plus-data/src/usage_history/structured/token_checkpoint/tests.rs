use super::*;
use crate::usage_history::structured::{Index, Query, RETENTION_DAYS, index_budget::Budget};
use chrono::{DateTime, Local};
use rusqlite::{Connection, ErrorCode, config::DbConfig};
use serde_json::json;
use std::fs::{self, OpenOptions};
use std::io::Write;

const SESSION: &str = "00000000-0000-4000-8000-000000000010";

fn source_rows() -> Vec<Vec<u8>> {
    source_rows_on(NaiveDate::from_ymd_opt(2026, 1, 2).unwrap())
}

fn source_rows_on(date: NaiveDate) -> Vec<Vec<u8>> {
    let at = |minute| format!("{date}T12:{minute:02}:00Z");
    let counts = |[input, read, write, output, total]: [u64; 5]| {
        json!({
            "input_tokens": input, "cached_input_tokens": read, "cache_write_input_tokens": write,
            "output_tokens": output, "total_tokens": total,
        })
    };
    [
        json!({"type":"session_meta","timestamp":at(0),"ordinal":0,
            "payload":{"id":SESSION,"model_provider":"custom"}}),
        json!({"type":"turn_context","timestamp":at(1),"ordinal":1,
            "payload":{"model":"model-a","turn_id":"first"}}),
        json!({"type":"event_msg","timestamp":at(2),"ordinal":2,
            "payload":{"type":"token_count","info":{"total_token_usage":counts([10,40,3,5,58])}}}),
        json!({"type":"turn_context","timestamp":at(3),"ordinal":3,
            "payload":{"model":"model-b","turn_id":"second"}}),
        json!({"type":"event_msg","timestamp":at(4),"ordinal":4,
            "payload":{"type":"token_count","info":{"total_token_usage":counts([110,120,7,15,168])}}}),
        json!({"type":"event_msg","timestamp":at(5),"ordinal":5,
            "payload":{"type":"token_count","info":{"total_token_usage":counts([110,120,7,15,168])}}}),
        json!({"type":"turn_context","timestamp":at(6),"ordinal":6,
            "payload":{"model":"model-b\u{00e9}","turn_id":"third"}}),
        json!({"type":"event_msg","timestamp":at(7),"ordinal":7,
            "payload":{"type":"token_count","info":{"total_token_usage":{
                "input_tokens":3,"output_tokens":2},"last_token_usage":{
                "input_tokens":3,"output_tokens":2}}}}),
        json!({"type":"event_msg","timestamp":at(8),"ordinal":8,
            "payload":{"type":"token_count","info":{"total_token_usage":{
                "input_tokens":5,"cached_input_tokens":0,"output_tokens":3},"last_token_usage":{
                "input_tokens":2,"cached_input_tokens":0,"output_tokens":1}}}}),
    ]
    .into_iter()
    .map(|row| format!("{row}\n").into_bytes())
    .collect()
}

fn round_trip(state: &FileState) -> FileState {
    let (offset, checkpoint) = Checkpoint::capture(state);
    let bytes = serde_json::to_vec(&checkpoint).unwrap();
    serde_json::from_slice::<Checkpoint>(&bytes)
        .unwrap()
        .restore(offset, state.source_generation)
}

#[test]
fn unknown_snapshot_version_is_not_modified() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("token.sqlite");
    let db = Connection::open(&path).unwrap();
    db.pragma_update(None, "user_version", 2).unwrap();
    drop(db);
    let before = fs::read(&path).unwrap();
    let error = store::Store::open(&path).err().unwrap();
    assert_eq!(error.to_string(), "unsupported Token snapshot version 2");
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn unknown_wal_version_does_not_checkpoint_unmerged_data_on_open() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("token.sqlite");
    let wal = directory.path().join("token.sqlite-wal");
    let db = Connection::open(&path).unwrap();
    db.pragma_update(None, "journal_mode", "WAL").unwrap();
    db.set_db_config(DbConfig::SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE, true)
        .unwrap();
    db.execute_batch(
        "CREATE TABLE future (entry TEXT);
         INSERT INTO future VALUES ('uncheckpointed');
         PRAGMA user_version = 2;",
    )
    .unwrap();
    drop(db);

    let main_before = fs::read(&path).unwrap();
    let wal_before = fs::read(&wal).unwrap();
    assert!(wal_before.len() > 32);
    let error = store::Store::open(&path).err().unwrap();
    assert_eq!(error.to_string(), "unsupported Token snapshot version 2");
    assert_eq!(fs::read(&path).unwrap(), main_before);
    assert_eq!(fs::read(&wal).unwrap(), wal_before);
}

#[test]
fn unversioned_database_with_existing_schema_is_not_modified() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("token.sqlite");
    let db = Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE unrelated (entry TEXT);")
        .unwrap();
    drop(db);
    let before = fs::read(&path).unwrap();
    let error = store::Store::open(&path).err().unwrap();
    assert_eq!(error.to_string(), "unversioned Token snapshot is not empty");
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn additive_payload_fields_do_not_change_counts_or_pending_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let path = fs::canonicalize(dir.path()).unwrap().join("rollout.jsonl");
    fs::write(&path, source_rows()[..3].concat()).unwrap();
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let mut state = FileState::default();
    assert!(
        state
            .read_with_budget(&path, cutoff, &mut Budget::new(std::iter::empty()))
            .unwrap()
    );
    let event = &mut state.events[0];
    event.native_response_id = Some(NativeResponseIdentity {
        id: "fixture-response".into(),
        full_counts: false,
    });
    let mut observation = serde_json::to_value(Observation::capture(event)).unwrap();
    observation["future_metadata"] = json!({"present":true});
    let decoded: Observation = serde_json::from_value(observation).unwrap();
    let restored = decoded
        .into_event(
            event.id,
            event.original_id,
            event.session.clone(),
            event.source_end,
        )
        .unwrap();
    assert!(restored == *event);

    let (offset, checkpoint) = Checkpoint::capture(&state);
    let mut data = serde_json::to_value(checkpoint).unwrap();
    data.as_object_mut()
        .unwrap()
        .remove("scope_parent_candidates");
    data["future_metadata"] = json!(true);
    let decoded: Checkpoint = serde_json::from_value(data).unwrap();
    let restored = decoded.restore(offset, state.source_generation);
    assert!(restored.scope_parent_candidates.is_empty());
    assert_eq!(restored.offset, offset);
    assert_eq!(restored.previous, state.previous);
}

#[test]
#[ignore = "requires a generated synthetic session directory"]
fn synthetic_token_snapshot_baseline() {
    let source =
        std::env::var_os("CODEX_USAGE_TOKEN_FIXTURE").expect("synthetic fixture directory");
    let mut paths = fs::read_dir(fs::canonicalize(source).unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    paths.sort_unstable();
    assert!(!paths.is_empty());
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("token.sqlite");
    let scan_started = std::time::Instant::now();
    let mut store = store::Store::open(&path).unwrap();
    for source in &paths {
        let mut state = FileState::default();
        let mut budget = Budget::new(std::iter::empty());
        assert!(state.read_with_budget(source, cutoff, &mut budget).unwrap());
        assert_eq!(state.events.len(), 1);
        store
            .save(source.file_name().unwrap().as_encoded_bytes(), None, &state)
            .unwrap();
    }
    let wal_bytes = fs::metadata(directory.path().join("token.sqlite-wal"))
        .unwrap()
        .len();
    drop(store);
    let scan = scan_started.elapsed();
    let read_started = std::time::Instant::now();
    let store = store::Store::open(&path).unwrap();
    let mut totals = [0_u64; 4];
    for source in &paths {
        let key = source.file_name().unwrap().as_encoded_bytes();
        let (_, checkpoint) = store.checkpoint(key).unwrap().unwrap();
        assert_eq!(checkpoint.offset, fs::metadata(source).unwrap().len());
        let rows = store.observations(key).unwrap();
        assert_eq!(rows.len(), 1);
        let usage = rows[0].delta;
        for (total, value) in totals.iter_mut().zip([
            usage.input_tokens,
            usage.cached_input_tokens,
            usage.output_tokens,
            usage.total_tokens,
        ]) {
            *total += value;
        }
    }
    let sessions = paths.len() as u64;
    assert_eq!(
        totals,
        [10 * sessions, 4 * sessions, 2 * sessions, 12 * sessions]
    );
    println!(
        "{sessions} sessions: totals={totals:?}, scan/save={scan:?}, reopen/read={:?}, sqlite_bytes={}, wal_bytes={wal_bytes}",
        read_started.elapsed(),
        fs::metadata(path).unwrap().len(),
    );
}

#[test]
fn each_complete_line_checkpoint_resumes_with_identical_events() {
    let rows = source_rows();
    let source = rows.concat();
    let dir = tempfile::tempdir().unwrap();
    let path = fs::canonicalize(dir.path()).unwrap().join("rollout.jsonl");
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    fs::write(&path, &source).unwrap();
    let mut continuous = FileState::default();
    assert!(
        continuous
            .read_with_budget(&path, cutoff, &mut Budget::new(std::iter::empty()))
            .unwrap()
    );
    let expected = continuous.events;
    let counts = expected
        .iter()
        .map(|event| {
            (
                event.delta.input_tokens,
                event.delta.cached_known,
                event.delta.output_tokens,
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        counts,
        vec![
            (53, true, 5),
            (100, true, 10),
            (0, true, 0),
            (3, false, 2),
            (2, true, 1)
        ]
    );

    let mut end = 0;
    for (row, next) in rows.iter().zip(rows.iter().skip(1)) {
        end += row.len();
        // 留下一段未完成行，包含 UTF-8 字符的半个编码。
        let partial = next
            .iter()
            .position(|byte| *byte == 0xc3)
            .map_or(2, |i| i + 1);
        fs::write(&path, &source[..end + partial]).unwrap();
        let mut prefix = FileState::default();
        assert!(
            !prefix
                .read_with_budget(&path, cutoff, &mut Budget::new(std::iter::empty()))
                .unwrap()
        );
        let mut resumed = round_trip(&prefix);
        assert_eq!(resumed.offset, end as u64);
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(&source[end + partial..]).unwrap();
        drop(file);
        let mut combined: Vec<_> = prefix
            .events
            .into_iter()
            .map(|event| {
                let bytes = serde_json::to_vec(&Observation::capture(&event)).unwrap();
                serde_json::from_slice::<Observation>(&bytes)
                    .unwrap()
                    .into_event(event.id, event.original_id, event.session, event.source_end)
                    .unwrap()
            })
            .collect();
        assert!(
            resumed
                .read_with_budget(&path, cutoff, &mut Budget::new(std::iter::empty()))
                .unwrap()
        );
        combined.extend(resumed.events);
        assert!(combined == expected, "event mismatch after byte {end}");
        assert_eq!(resumed.rollout_extent, continuous.rollout_extent);
        assert_eq!(resumed.previous, continuous.previous);
    }
}

#[test]
fn checkpoint_does_not_treat_an_unread_suffix_as_end_of_an_unchanged_file() {
    let source = source_rows().concat();
    let dir = tempfile::tempdir().unwrap();
    let path = fs::canonicalize(dir.path()).unwrap().join("rollout.jsonl");
    fs::write(&path, &source).unwrap();
    let cutoff = DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
        .unwrap()
        .with_timezone(&Local)
        .date_naive();
    let mut prefix = FileState::default();
    let mut budget = Budget::new(std::iter::empty());
    budget.read_left = source_rows()[..3].iter().map(Vec::len).sum::<usize>() as u64;
    assert!(!prefix.read_with_budget(&path, cutoff, &mut budget).unwrap());
    assert_eq!(prefix.events.len(), 1);
    let mut resumed = round_trip(&prefix);
    assert!(
        resumed
            .read_with_budget(&path, cutoff, &mut Budget::new(std::iter::empty()))
            .unwrap()
    );
    assert_eq!(resumed.events.len(), 4);
}

#[test]
fn restart_replays_a_compressed_prefix_without_reparsing_committed_events() {
    let rows = source_rows();
    let dir = tempfile::tempdir().unwrap();
    let path = fs::canonicalize(dir.path())
        .unwrap()
        .join("rollout.jsonl.zst");
    fs::write(&path, zstd::bulk::compress(&rows.concat(), 1).unwrap()).unwrap();
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let mut state = FileState::default();
    let mut budget = Budget::new(std::iter::empty());
    budget.read_left = rows[..5].iter().map(Vec::len).sum::<usize>() as u64;
    assert!(!state.read_with_budget(&path, cutoff, &mut budget).unwrap());
    assert_eq!(state.events.len(), 2);
    assert!(state.offset > fs::metadata(&path).unwrap().len());

    let mut resumed = round_trip(&state);
    let mut budget = Budget::new(std::iter::empty());
    budget.read_left = 32;
    assert!(
        !resumed
            .read_with_budget(&path, cutoff, &mut budget)
            .unwrap()
    );
    assert_eq!(resumed.offset, state.offset);
    assert!(resumed.events.is_empty());
    assert_eq!(resumed.replay_remaining, state.offset - 32);
    assert!(
        resumed
            .read_with_budget(&path, cutoff, &mut Budget::new(std::iter::empty()))
            .unwrap()
    );
    state.events.append(&mut resumed.events);

    let mut continuous = FileState::default();
    assert!(
        continuous
            .read_with_budget(&path, cutoff, &mut Budget::new(std::iter::empty()))
            .unwrap()
    );
    assert!(state.events == continuous.events);
    assert!(!round_trip(&resumed).resume_pending);
}

#[test]
fn restart_preserves_continuation_base_and_expired_original_watermark() {
    let mut rows = source_rows();
    rows[0] = format!(
        "{}\n",
        json!({"type":"session_meta","timestamp":"2026-01-02T12:00:00Z","ordinal":20,
            "payload":{"id":SESSION,"history_mode":"paginated","history_base":{
                "thread_id":"00000000-0000-4000-8000-000000000081",
                "end_ordinal_exclusive":20,"end_byte_offset":128}}})
    )
    .into_bytes();
    let dir = tempfile::tempdir().unwrap();
    let path = fs::canonicalize(dir.path()).unwrap().join("rollout.jsonl");
    fs::write(&path, rows[..3].concat()).unwrap();
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 3).unwrap();
    let mut prefix = FileState::default();
    assert!(
        prefix
            .read_with_budget(&path, cutoff, &mut Budget::new(std::iter::empty()))
            .unwrap()
    );
    assert!(prefix.events.is_empty());
    assert!(prefix.rollout_out_of_order);
    let base = prefix.history_base.as_ref().unwrap();
    let resumed = round_trip(&prefix);
    let restored_base = resumed.history_base.unwrap();
    assert_eq!(restored_base.rollout, base.rollout);
    assert_eq!(restored_base.end_ordinal, 20);
    assert_eq!(restored_base.end_offset, 128);
    assert!(resumed.continuation);
    assert!(resumed.rollout_out_of_order);
    assert_eq!(resumed.rollout_extent, prefix.rollout_extent);
    assert_eq!(resumed.expired_count, prefix.expired_count);
    assert_eq!(resumed.expired_count.unwrap().1, Tokens::new(53, 40, 5));
}

#[test]
fn sqlite_full_rolls_back_cursor_observations_and_revision_then_reopens() {
    let rows = source_rows();
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let path = root.join("rollout.jsonl");
    let db = root.join("tokens.sqlite3");
    let key = b"rollout.jsonl";
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    fs::write(&path, rows.concat()).unwrap();
    let mut state = FileState::default();
    let mut budget = Budget::new(std::iter::empty());
    budget.read_left = rows[..3].iter().map(Vec::len).sum::<usize>() as u64;
    assert!(!state.read_with_budget(&path, cutoff, &mut budget).unwrap());
    let mut store = store::Store::open(&db).unwrap();
    let first = store.save(key, None, &state).unwrap();
    let reader = store::Store {
        db: Connection::open(&db).unwrap(),
    };
    reader.db.execute_batch("BEGIN").unwrap();
    let (_, old) = reader.checkpoint(key).unwrap().unwrap();
    let (_, mut tail) = store.checkpoint(key).unwrap().unwrap();
    assert!(
        tail.read_with_budget(&path, cutoff, &mut Budget::new(std::iter::empty()))
            .unwrap()
    );
    let page_count: i64 = store
        .db
        .pragma_query_value(None, "page_count", |r| r.get(0))
        .unwrap();
    store
        .db
        .pragma_update(None, "max_page_count", page_count)
        .unwrap();
    let saved_model = std::mem::replace(&mut tail.events[0].model, "x".repeat(65536));
    let failure = store.save(key, Some(first), &tail).unwrap_err();
    assert_eq!(
        failure
            .downcast_ref::<rusqlite::Error>()
            .unwrap()
            .sqlite_error_code(),
        Some(ErrorCode::DiskFull)
    );
    assert_eq!(store.checkpoint(key).unwrap().unwrap().1.offset, old.offset);
    assert!(store.observations(key).unwrap() == state.events);
    drop(store);

    tail.events[0].model = saved_model;
    let mut store = store::Store::open(&db).unwrap();
    store
        .db
        .pragma_update(None, "max_page_count", page_count + 100)
        .unwrap();
    assert_eq!(store.save(key, Some(first), &tail).unwrap(), first + 1);
    assert_eq!(
        reader.checkpoint(key).unwrap().unwrap().1.offset,
        old.offset
    );
    assert!(reader.observations(key).unwrap() == state.events);
    reader.db.execute_batch("COMMIT").unwrap();
    assert_eq!(
        reader.checkpoint(key).unwrap().unwrap().1.offset,
        tail.offset
    );
    state.events.extend(tail.events);
    assert!(reader.observations(key).unwrap() == state.events);
}

#[test]
fn abrupt_process_exit_keeps_only_committed_batches() {
    use rusqlite::TransactionBehavior;
    use std::process::Command;

    const TEST_HOME: &str = "CODEX_PLUS_TEST_TOKEN_CHECKPOINT_CRASH_HOME";
    const TEST_COMMIT: &str = "CODEX_PLUS_TEST_TOKEN_CHECKPOINT_CRASH_COMMIT";
    let key = b"rollout.jsonl";
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    if let Some(home) = std::env::var_os(TEST_HOME) {
        let home = std::path::PathBuf::from(home);
        let mut store = store::Store::open(&home.join("tokens.sqlite3")).unwrap();
        let (revision, mut state) = store.checkpoint(key).unwrap().unwrap();
        assert!(
            state
                .read_with_budget(
                    &home.join("rollout.jsonl"),
                    cutoff,
                    &mut Budget::new(std::iter::empty())
                )
                .unwrap()
        );
        let tx = store
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        store::Store::stage(&tx, key, Some(revision), &state).unwrap();
        tx.cache_flush().unwrap();
        if std::env::var_os(TEST_COMMIT).is_some() {
            tx.commit().unwrap();
        }
        // 直接退出，不运行 Transaction/Connection 的析构。
        std::process::exit(37);
    }

    let rows = source_rows();
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let path = root.join("rollout.jsonl");
    let db = root.join("tokens.sqlite3");
    let source = rows.concat();
    fs::write(&path, &source).unwrap();
    let mut prefix = FileState::default();
    let mut budget = Budget::new(std::iter::empty());
    budget.read_left = rows[..3].iter().map(Vec::len).sum::<usize>() as u64;
    assert!(!prefix.read_with_budget(&path, cutoff, &mut budget).unwrap());
    let mut store = store::Store::open(&db).unwrap();
    let first = store.save(key, None, &prefix).unwrap();
    drop(store);

    for committed in [false, true] {
        let mut child = Command::new(std::env::current_exe().unwrap());
        let module = module_path!().split_once("::").unwrap().1;
        child
            .arg("--exact")
            .arg(format!(
                "{module}::abrupt_process_exit_keeps_only_committed_batches"
            ))
            .env(TEST_HOME, &root)
            .env_remove(TEST_COMMIT);
        if committed {
            child.env(TEST_COMMIT, "1");
        }
        assert_eq!(child.status().unwrap().code(), Some(37));
        let store = store::Store::open(&db).unwrap();
        let (revision, state) = store.checkpoint(key).unwrap().unwrap();
        assert_eq!(revision, first + i64::from(committed));
        assert_eq!(
            state.offset,
            if committed {
                source.len() as u64
            } else {
                prefix.offset
            }
        );
        assert_eq!(
            store.observations(key).unwrap().len(),
            if committed { 5 } else { 1 }
        );
    }
}

#[test]
fn appending_to_the_same_state_writes_only_new_source_positions() {
    let rows = source_rows();
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let path = root.join("rollout.jsonl");
    let db = root.join("tokens.sqlite3");
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let mut legacy: serde_json::Value = serde_json::from_slice(&rows[2]).unwrap();
    legacy.as_object_mut().unwrap().remove("ordinal");
    let legacy = format!("{legacy}\n");
    fs::write(
        &path,
        [rows[..2].concat(), legacy.as_bytes().to_vec()].concat(),
    )
    .unwrap();
    let mut state = FileState::default();
    assert!(
        state
            .read_with_budget(&path, cutoff, &mut Budget::new(std::iter::empty()))
            .unwrap()
    );
    let mut store = store::Store::open(&db).unwrap();
    let first = store.save(b"source", None, &state).unwrap();
    let second = store.save(b"source", Some(first), &state).unwrap();
    assert_eq!(store.observations(b"source").unwrap().len(), 1);

    let mut file = OpenOptions::new().append(true).open(&path).unwrap();
    file.write_all(legacy.as_bytes()).unwrap();
    drop(file);
    assert!(
        state
            .read_with_budget(&path, cutoff, &mut Budget::new(std::iter::empty()))
            .unwrap()
    );
    assert_eq!(state.events.len(), 2);
    assert_eq!(state.events[0].id, state.events[1].id);
    assert!(state.events[0].source_end < state.events[1].source_end);
    store.save(b"source", Some(second), &state).unwrap();
    drop(store);
    let store = store::Store::open(&db).unwrap();
    assert!(store.observations(b"source").unwrap() == state.events);
}

#[test]
fn replacing_a_source_auto_resets_observations_and_removes_only_orphan_sessions() {
    let rows = source_rows();
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let path = root.join("rollout.jsonl");
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    fs::write(&path, rows.concat()).unwrap();
    let mut state = FileState::default();
    assert!(
        state
            .read_with_budget(&path, cutoff, &mut Budget::new(std::iter::empty()))
            .unwrap()
    );
    let mut store = store::Store::open(&root.join("tokens.sqlite3")).unwrap();
    let original = store.save(b"source-a", None, &state).unwrap();
    store.save(b"source-b", None, &state).unwrap();

    const NEW_SESSION: &str = "00000000-0000-4000-8000-000000000011";
    fs::rename(&path, root.join("previous-rollout.jsonl")).unwrap();
    let text = String::from_utf8(rows.concat())
        .unwrap()
        .replace(SESSION, NEW_SESSION);
    fs::write(&path, format!("{text}\n")).unwrap();
    let (_, mut replacement) = store.checkpoint(b"source-a").unwrap().unwrap();
    assert!(
        replacement
            .read_with_budget(&path, cutoff, &mut Budget::new(std::iter::empty()))
            .unwrap()
    );
    assert!(replacement.offset > state.offset);
    store
        .save(b"source-a", Some(original), &replacement)
        .unwrap();
    assert!(store.save(b"source-a", Some(original), &state).is_err());
    assert!(store.observations(b"source-a").unwrap() == replacement.events);
    assert!(store.observations(b"source-b").unwrap() == state.events);
    store
        .db
        .execute(
            "DELETE FROM token_sources WHERE source = ?1",
            [b"source-b".as_slice()],
        )
        .unwrap();
    let retained_session: String = store
        .db
        .query_row("SELECT session FROM token_sessions", [], |row| row.get(0))
        .unwrap();
    assert_eq!(retained_session, NEW_SESSION);
    assert!(store.observations(b"source-b").unwrap().is_empty());
    assert!(store.observations(b"source-a").unwrap() == replacement.events);
}

#[test]
fn reopened_duplicate_rollouts_match_the_live_resolver_and_daily_totals() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let sessions = root.join("sessions");
    fs::create_dir(&sessions).unwrap();
    let settings = root.join("settings.json");
    let log = root.join("request-log.jsonl");
    fs::write(
        &settings,
        r#"{"usageHistoryEnabled":true,"relayProfiles":[]}"#,
    )
    .unwrap();
    fs::write(&log, "").unwrap();
    let make_index = |cache| Index {
        settings_path: Some(settings.clone()),
        storage_dir: Some(root.join(cache)),
        ..Index::default()
    };
    let today = Local::now().date_naive();
    let date = today.pred_opt().unwrap();
    let mut rows = source_rows_on(date);
    for (ordinal, count) in [(2, 1), (4, 2), (5, 2)] {
        let usage =
            |n| json!({"input_tokens":100*n,"cached_input_tokens":20*n,"output_tokens":10*n});
        let row = json!({"type":"event_msg","timestamp":format!("{date}T12:{ordinal:02}:00Z"),
            "ordinal":ordinal,"payload":{"type":"token_count","info":{
                "total_token_usage":usage(count),
                "last_token_usage":usage(1)}}});
        rows[ordinal] = format!("{row}\n").into_bytes();
    }
    let content = rows[..6].concat();
    let sources = ["rollout-a.jsonl", "rollout-b.jsonl"];
    for name in sources {
        fs::write(sessions.join(name), &content).unwrap();
    }
    let mut live = make_index("live-evidence");
    let expected = live.update_with_request_log(&root, &log);
    assert!(expected.bytes_read > 0);
    assert_eq!(expected.events.len(), 2);

    let db = root.join("tokens.sqlite3");
    let cutoff = today - chrono::Days::new((RETENTION_DAYS - 1) as u64);
    let mut store = store::Store::open(&db).unwrap();
    for name in sources {
        let path = sessions.join(name);
        let mut state = FileState::default();
        let mut budget = Budget::new(std::iter::empty());
        budget.read_left = rows[..5].iter().map(Vec::len).sum::<usize>() as u64;
        assert!(!state.read_with_budget(&path, cutoff, &mut budget).unwrap());
        let revision = store.save(name.as_bytes(), None, &state).unwrap();
        let (_, mut state) = store.checkpoint(name.as_bytes()).unwrap().unwrap();
        assert!(
            state
                .read_with_budget(&path, cutoff, &mut Budget::new(std::iter::empty()))
                .unwrap()
        );
        store.save(name.as_bytes(), Some(revision), &state).unwrap();
    }
    drop(store);

    let store = store::Store::open(&db).unwrap();
    let mut index = make_index("restored-evidence");
    for name in sources {
        let (_, mut state) = store.checkpoint(name.as_bytes()).unwrap().unwrap();
        state.events = store.observations(name.as_bytes()).unwrap();
        index.files.insert(sessions.join(name), state);
    }
    let restored = index.update_with_request_log(&root, &log);
    assert_eq!(restored.bytes_read, 0);
    assert!(restored.events == expected.events);
    let query = Query::parse(&json!({"period":"all"}), today).unwrap();
    let gold = expected.response_page(&query, false, 1, None, None);
    let observed = restored.response_page(&query, false, 1, None, None);
    assert_eq!(observed["days"], gold["days"]);
    assert_eq!(observed["days"][0]["recordCount"], 2);
    assert_eq!(observed["days"][0]["usage"]["totalTokens"], 220);
    assert_eq!(observed["days"][0]["usage"]["cachedKnown"], true);
}

#[test]
fn bounded_batches_commit_beyond_the_in_memory_observation_limit() {
    use crate::usage_history::structured::MAX_EVENTS;
    use std::io::BufWriter;
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let path = root.join("long.jsonl");
    let db = root.join("tokens.sqlite3");
    let key = b"long.jsonl";
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let mut file = BufWriter::new(fs::File::create(&path).unwrap());
    file.write_all(&source_rows()[0]).unwrap();
    let at = DateTime::parse_from_rfc3339("2026-01-02T12:00:00Z").unwrap();
    for n in 1..=MAX_EVENTS + 1 {
        let timestamp = at + chrono::Duration::milliseconds(n as i64);
        let row = json!({"type":"event_msg","timestamp":timestamp.to_rfc3339(),
            "payload":{"type":"token_count","info":{"total_token_usage":{
                "input_tokens":n,"cached_input_tokens":0,"output_tokens":n}}}});
        writeln!(file, "{row}").unwrap();
    }
    file.flush().unwrap();
    drop(file);

    let mut store = store::Store::open(&db).unwrap();
    let mut previous = None;
    let size = fs::metadata(&path).unwrap().len();
    loop {
        let (revision, mut state) = store
            .checkpoint(key)
            .unwrap()
            .map_or((None, FileState::default()), |(revision, state)| {
                (Some(revision), state)
            });
        assert_eq!(revision, previous);
        let old_offset = state.offset;
        let mut budget = Budget::new(std::iter::empty());
        budget.read_left = 128 * 1024;
        let complete = state.read_with_budget(&path, cutoff, &mut budget).unwrap();
        assert_eq!(state.issues, 0);
        assert!(state.events.len() <= 1024);
        let (next, _) = Checkpoint::capture(&state);
        assert!(next > old_offset);
        previous = Some(store.save(key, revision, &state).unwrap());
        if complete {
            assert_eq!(next, size);
            break;
        }
    }
    drop(store);
    let db = Connection::open(&db).unwrap();
    let count: usize = db
        .query_row("SELECT count(*) FROM token_observations", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, MAX_EVENTS + 1);
}
