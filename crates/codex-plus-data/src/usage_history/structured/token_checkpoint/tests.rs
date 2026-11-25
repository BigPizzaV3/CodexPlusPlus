use super::*;
use crate::usage_history::structured::{Index, Query, RETENTION_DAYS, index_budget::Budget};
use chrono::{DateTime, Local};
use rusqlite::{Connection, ErrorCode};
use serde_json::json;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

const SESSION: &str = "00000000-0000-4000-8000-000000000010";

#[test]
fn nul_padding_requires_adjacent_consecutive_records_across_reopen() {
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let record =
        |ordinal| format!("{{\"type\":\"note\",\"ordinal\":{ordinal},\"payload\":{{}}}}\n");
    let feed = |state: &mut FileState, line: &[u8]| {
        state.offset += line.len() as u64;
        state.accept(line, cutoff);
    };
    for (middle, next, expected) in [
        (b"\0\0\n".as_slice(), 1, 0),
        (b"\0\0\n".as_slice(), 2, 1),
        (b"\0?\n".as_slice(), 1, 1),
    ] {
        let mut state = FileState::default();
        feed(&mut state, record(0).as_bytes());
        feed(&mut state, middle);
        assert_eq!(state.issues, 1);
        let (offset, checkpoint) = Checkpoint::capture(&state);
        let encoded = serde_json::to_vec(&checkpoint).unwrap();
        let mut reopened = serde_json::from_slice::<Checkpoint>(&encoded)
            .unwrap()
            .restore(offset);
        feed(&mut reopened, record(next).as_bytes());
        assert_eq!(reopened.issues, expected);
    }
    let mut state = FileState::default();
    feed(&mut state, b"\0\n");
    feed(&mut state, record(0).as_bytes());
    feed(&mut state, b"\0\n");
    feed(&mut state, b"\0\n");
    feed(&mut state, record(1).as_bytes());
    assert_eq!(state.issues, 3);
}

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
        .restore(offset)
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
                    .into_event(event.id, event.session)
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
    let key = Path::new("rollout.jsonl");
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    fs::write(&path, rows.concat()).unwrap();
    let mut state = FileState::default();
    let mut budget = Budget::new(std::iter::empty());
    budget.read_left = rows[..3].iter().map(Vec::len).sum::<usize>() as u64;
    assert!(!state.read_with_budget(&path, cutoff, &mut budget).unwrap());
    let mut store = store::Store::open(Connection::open(&db).unwrap()).unwrap();
    let first = store.save(key, None, &state, false).unwrap();
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
    let failure = store.save(key, Some(first), &tail, false).unwrap_err();
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
    let mut store = store::Store::open(Connection::open(&db).unwrap()).unwrap();
    store
        .db
        .pragma_update(None, "max_page_count", page_count + 100)
        .unwrap();
    assert_eq!(
        store.save(key, Some(first), &tail, false).unwrap(),
        first + 1
    );
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
    let key = Path::new("rollout.jsonl");
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    if let Some(home) = std::env::var_os(TEST_HOME) {
        let home = std::path::PathBuf::from(home);
        let mut store =
            store::Store::open(Connection::open(home.join("tokens.sqlite3")).unwrap()).unwrap();
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
        store::Store::stage(&tx, key, Some(revision), &state, false).unwrap();
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
    let mut store = store::Store::open(Connection::open(&db).unwrap()).unwrap();
    let first = store.save(key, None, &prefix, false).unwrap();
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
        let store = store::Store::open(Connection::open(&db).unwrap()).unwrap();
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
fn replacing_a_source_replaces_only_its_observations_and_rejects_a_stale_batch() {
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
    let mut store =
        store::Store::open(Connection::open(root.join("tokens.sqlite3")).unwrap()).unwrap();
    let a = Path::new("source-a");
    let b = Path::new("source-b");
    let original = store.save(a, None, &state, false).unwrap();
    store.save(b, None, &state, false).unwrap();

    fs::write(&path, rows[..3].concat()).unwrap();
    let mut replacement = FileState::default();
    assert!(
        replacement
            .read_with_budget(&path, cutoff, &mut Budget::new(std::iter::empty()))
            .unwrap()
    );
    store.save(a, Some(original), &replacement, true).unwrap();
    assert!(store.save(a, Some(original), &state, false).is_err());
    assert!(store.observations(a).unwrap() == replacement.events);
    assert!(store.observations(b).unwrap() == state.events);
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
    let mut store = store::Store::open(Connection::open(&db).unwrap()).unwrap();
    for name in sources {
        let path = sessions.join(name);
        let mut state = FileState::default();
        let mut budget = Budget::new(std::iter::empty());
        budget.read_left = rows[..5].iter().map(Vec::len).sum::<usize>() as u64;
        assert!(!state.read_with_budget(&path, cutoff, &mut budget).unwrap());
        let revision = store.save(Path::new(name), None, &state, false).unwrap();
        let (_, mut state) = store.checkpoint(Path::new(name)).unwrap().unwrap();
        assert!(
            state
                .read_with_budget(&path, cutoff, &mut Budget::new(std::iter::empty()))
                .unwrap()
        );
        store
            .save(Path::new(name), Some(revision), &state, false)
            .unwrap();
    }
    drop(store);

    let store = store::Store::open(Connection::open(&db).unwrap()).unwrap();
    let mut index = make_index("restored-evidence");
    for name in sources {
        let (_, mut state) = store.checkpoint(Path::new(name)).unwrap().unwrap();
        state.events = store.observations(Path::new(name)).unwrap();
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
fn bounded_batches_resolve_beyond_the_in_memory_observation_limit() {
    use crate::usage_history::structured::MAX_EVENTS;
    use std::io::BufWriter;
    use store::projection::SourceUpdate;
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let path = root.join("long.jsonl");
    let db = root.join("tokens.sqlite3");
    let key = path.as_path();
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let mut file = BufWriter::new(fs::File::create(&path).unwrap());
    file.write_all(&source_rows()[0]).unwrap();
    let at = DateTime::parse_from_rfc3339("2026-01-02T12:00:00Z").unwrap();
    for n in 1..=MAX_EVENTS + 1 {
        let timestamp = at + chrono::Duration::milliseconds(n as i64);
        let row = json!({"type":"event_msg","ordinal":n,"timestamp":timestamp.to_rfc3339(),
            "payload":{"type":"token_count","info":{"total_token_usage":{
                "input_tokens":n,"cached_input_tokens":0,"output_tokens":n},
                "last_token_usage":{"input_tokens":1,"cached_input_tokens":0,"output_tokens":1}}}});
        writeln!(file, "{row}").unwrap();
    }
    file.flush().unwrap();
    drop(file);

    let mut store = store::Store::open(Connection::open(&db).unwrap()).unwrap();
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
        let prepared = store
            .prepare(
                SourceUpdate {
                    source: key,
                    state: &state,
                    previous_revision: revision,
                    replace: false,
                },
                cutoff,
                BTreeMap::new(),
                512 * 1024,
            )
            .unwrap_or_else(|error| panic!("offset {old_offset}: {error:#}"));
        previous = Some(store.publish(prepared).unwrap());
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
    let mut db = db;
    let mut resolved = 0;
    let revision = store::Store::read_committed(&mut db, i64::MIN, i64::MAX, |event| {
        assert_eq!(event.delta, Tokens::new(1, 0, 1));
        assert!(!event.count_uncertain && !event.count_incomplete);
        resolved += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!(Some(revision), previous);
    assert_eq!(resolved, count);
    let day = at.with_timezone(&chrono::Local).date_naive();
    let mut summary_records = 0;
    let mut summary_tokens = 0;
    let summary_revision =
        store::Store::read_days(&mut db, day, day.succ_opt().unwrap(), |_, _, groups| {
            for group in groups {
                summary_records += group.totals.records;
                summary_tokens += group.totals.counts.0.total_tokens;
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(summary_revision, revision);
    assert_eq!(summary_records, count as u64);
    assert_eq!(summary_tokens, count as u64 * 2);
}
