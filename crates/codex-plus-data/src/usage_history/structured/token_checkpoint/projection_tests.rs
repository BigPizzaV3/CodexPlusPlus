use super::ingest::SourceReader;
use super::store::{Coverage, Store, projection::SourceUpdate};
use crate::usage_history::Query;
use crate::usage_history::structured::{FileState, UsageEvent, index_budget::Budget};
use chrono::{DateTime, Local, NaiveDate, TimeZone};
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::{fs, io::Write};

const PARENT: &str = "00000000-0000-4000-8000-000000000071";
const CHILD: &str = "00000000-0000-4000-8000-000000000072";
const OTHER: &str = "00000000-0000-4000-8000-000000000073";
const PHYSICAL_PREFIX: &str = "00000000-0000-4000-8000-000000000074";

fn header(session: &str, ordinal: usize) -> Value {
    json!({"type":"session_meta","ordinal":ordinal,"timestamp":"2026-01-02T12:00:00Z",
        "payload":{"id":session,"history_mode":"paginated","model_provider":"supplier-a"}})
}

fn count(ordinal: usize, second: usize, n: u64, last: bool) -> Value {
    let counts =
        |n| json!({"input_tokens":100*n, "cached_input_tokens":20*n, "output_tokens":10*n});
    json!({"type":"event_msg","ordinal":ordinal,"timestamp":format!("2026-01-02T12:00:{second:02}Z"),
    "payload":{"type":"token_count","info":{
        "total_token_usage":counts(n),
        "last_token_usage":if last { counts(1) } else { Value::Null }
    }}})
}

fn parse(path: &Path, rows: &[Value], cutoff: NaiveDate) -> FileState {
    let source = rows
        .iter()
        .map(|row| format!("{row}\n"))
        .collect::<String>();
    fs::write(path, source).unwrap();
    let mut parsed = FileState::default();
    assert!(
        parsed
            .read_with_budget(path, cutoff, &mut Budget::new(std::iter::empty()))
            .unwrap()
    );
    parsed
}

fn read(db: &mut Connection, start: i64, end: i64) -> (i64, Vec<UsageEvent>) {
    let mut events = Vec::new();
    let revision = Store::read_committed(db, start, end, |event| {
        events.push(event);
        Ok(())
    })
    .unwrap();
    (revision, events)
}

fn new_source<'a>(source: &'a Path, state: &'a FileState) -> SourceUpdate<'a> {
    SourceUpdate {
        source,
        state,
        previous_revision: None,
        replace: false,
    }
}

#[test]
fn terminal_usage_persists_across_restart_and_late_count_without_rebasing_the_next_turn() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().canonicalize().unwrap();
    let database = home.join("usage.sqlite");
    let rollout = home.join("rollout-fixture.jsonl");
    let copied = home.join("rollout-copy.jsonl");
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let at = |second| format!("2026-01-02T12:00:{second:02}Z");
    let usage = |input, cached, output| json!({"input_tokens":input,"cached_input_tokens":cached,"output_tokens":output});
    let context = |second, turn| json!({"type":"turn_context","timestamp":at(second),"payload":{"turn_id":turn,"model":"fixture"}});
    let terminal = |second, turn, response, last, total| {
        json!({"type":"token_usage_record","timestamp":at(second),
        "payload":{"thread_id":PARENT,"turn_id":turn,"response_id":response,"usage":last,"thread_token_usage":total}})
    };
    let counter = |second, last, total| {
        json!({"type":"event_msg","timestamp":at(second),
        "payload":{"type":"token_count","info":{"last_token_usage":last,"total_token_usage":total}}})
    };
    let append = |rows: &[Value]| {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&rollout)
            .unwrap();
        for row in rows {
            writeln!(file, "{row}").unwrap();
        }
    };
    let step = |source: &Path| {
        let mut store = Store::open(Connection::open(&database).unwrap()).unwrap();
        let mut reader = SourceReader::restore(&store, source).unwrap();
        assert!(
            reader
                .step(
                    &mut store,
                    cutoff,
                    &mut Budget::new(std::iter::empty()),
                    &BTreeMap::new(),
                    1 << 20
                )
                .unwrap()
        );
        read(&mut store.db, i64::MIN, i64::MAX).1
    };
    let head = json!({"type":"session_meta","timestamp":at(0),"payload":{"id":PARENT,"model_provider":"fixture-supplier"}});
    let first = terminal(2, "turn-a", "resp-a", usage(10, 3, 2), usage(10, 3, 2));
    append(&[head.clone(), context(1, "turn-a"), first.clone()]);
    let initial = step(&rollout);
    assert_eq!(initial.len(), 1);
    assert_eq!(
        initial[0].delta,
        crate::usage_history::Tokens::new(10, 3, 2)
    );

    let partial = json!({"input_tokens":10,"output_tokens":2});
    append(&[counter(3, partial.clone(), partial)]);
    let paired = step(&rollout);
    assert_eq!(paired.len(), 1);
    assert_eq!(paired[0].delta, initial[0].delta);
    assert!(paired[0].terminal_response.is_none());

    // A new response with the same individual token counts still counts.
    append(&[terminal(
        4,
        "turn-a",
        "resp-b",
        usage(10, 3, 2),
        usage(20, 6, 4),
    )]);
    let second = step(&rollout);
    assert_eq!(second.len(), 2);
    assert_eq!(second[1].terminal_response.as_deref(), Some("resp-b"));

    // The subsequent native delta includes the unreported cumulative portion.
    // The earlier terminal receipt cannot be added again or become its baseline.
    append(&[
        context(5, "turn-b"),
        counter(6, usage(95, 10, 4), usage(105, 13, 6)),
    ]);
    let last = step(&rollout);
    assert_eq!(last.len(), 2);
    assert_eq!(last[1].delta, crate::usage_history::Tokens::new(95, 10, 4));
    fs::write(
        &copied,
        format!("{head}\n{}\n{first}\n", context(1, "turn-a")),
    )
    .unwrap();
    let duplicate = step(&copied);
    assert!(duplicate == last);
}

#[test]
fn reconciled_terminal_pairs_append_within_budget_after_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let source = root.join("rollout-long.jsonl");
    let database = root.join("tokens.sqlite3");
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let base = DateTime::parse_from_rfc3339("2026-01-02T12:00:00Z").unwrap();
    let pair = |n: usize, response: String, second_receipt: bool| {
        let at = |extra| (base + chrono::Duration::seconds(n as i64 * 4 + extra)).to_rfc3339();
        let turn = format!("turn-{n}");
        let total = count(0, 0, n as u64, true);
        let info = &total["payload"]["info"];
        let mut receipt = json!({
            "type":"token_usage_record","timestamp":at(2),
            "payload":{"thread_id":PARENT,"turn_id":turn,"response_id":response,
                       "usage":info["last_token_usage"],"thread_token_usage":info["total_token_usage"]}
        });
        let mut rows = vec![
            json!({"type":"turn_context","timestamp":at(0),"payload":{"model":"fixture","turn_id":turn}}),
            json!({"type":"event_msg","timestamp":at(1),"payload":total["payload"]}),
            receipt.clone(),
        ];
        if second_receipt {
            receipt["timestamp"] = json!(at(3));
            receipt["payload"]["response_id"] = json!("another-claim");
            rows.push(receipt);
        }
        for (index, row) in rows.iter_mut().enumerate() {
            row["ordinal"] = json!((n - 1) * 3 + index + 1);
        }
        rows
    };
    let mut rows = vec![header(PARENT, 0)];
    for n in 1..=120 {
        rows.extend(pair(n, format!("receipt-{n}"), false));
    }
    let original = parse(&source, &rows, cutoff);
    let mut store = Store::open(Connection::open(&database).unwrap()).unwrap();
    let seed = store
        .prepare(
            new_source(&source, &original),
            cutoff,
            BTreeMap::new(),
            1 << 20,
        )
        .unwrap();
    let seeded_revision = store.publish(seed).unwrap();
    let certificate: i64 = store
        .db
        .query_row("SELECT revision FROM token_linear_sources", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(certificate, seeded_revision);
    let used: usize = store
        .db
        .query_row(
            "SELECT sum(length(payload)) FROM token_observations",
            [],
            |row| row.get(0),
        )
        .unwrap();
    const APPEND_BUDGET: usize = 16 * 1024;
    assert!(used > APPEND_BUDGET);

    for (n, response, another_claim, should_append) in [
        (121, "receipt-121", false, true),
        (122, "receipt-122", false, true),
        (123, "receipt-123", true, false),
        (123, "receipt-1", false, false),
    ] {
        drop(store);
        let start = fs::metadata(&source).unwrap().len();
        let batch = pair(n, response.into(), another_claim);
        let mut file = fs::OpenOptions::new().append(true).open(&source).unwrap();
        for row in &batch {
            writeln!(file, "{row}").unwrap();
        }
        store = Store::open(Connection::open(&database).unwrap()).unwrap();
        let (revision, mut state) = store.checkpoint(&source).unwrap().unwrap();
        assert_eq!(state.offset, start);
        assert!(
            state
                .read_with_budget(&source, cutoff, &mut Budget::new(std::iter::empty()))
                .unwrap()
        );
        let update = SourceUpdate {
            source: &source,
            state: &state,
            previous_revision: Some(revision),
            replace: false,
        };
        let prepared = store.prepare(update, cutoff, BTreeMap::new(), APPEND_BUDGET);
        if !should_append {
            assert!(
                prepared
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("component exceeds reconciliation input budget")
            );
            assert_eq!(store.checkpoint(&source).unwrap().unwrap().1.offset, start);
            drop(file);
            fs::OpenOptions::new()
                .write(true)
                .open(&source)
                .unwrap()
                .set_len(start)
                .unwrap();
            continue;
        }
        let committed = store.publish(prepared.unwrap()).unwrap();
        assert_eq!(
            store.checkpoint(&source).unwrap().unwrap().1.offset,
            fs::metadata(&source).unwrap().len()
        );
        let certificate: i64 = store
            .db
            .query_row("SELECT revision FROM token_linear_sources", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(certificate, committed);
        rows.extend(batch);
    }

    let selected = read(&mut store.db, i64::MIN, i64::MAX).1;
    assert_eq!(selected.len(), 122);
    assert!(selected.iter().all(|event| event.delta.total_tokens == 110));
    let mut replay = Store::open(Connection::open_in_memory().unwrap()).unwrap();
    let full = parse(&source, &rows, cutoff);
    let prepared = replay
        .prepare(new_source(&source, &full), cutoff, BTreeMap::new(), 1 << 20)
        .unwrap();
    replay.publish(prepared).unwrap();
    assert!(selected == read(&mut replay.db, i64::MIN, i64::MAX).1);

    let day = full.events[0].date;
    let retained = day.succ_opt().unwrap();
    store.advance_retention(retained).unwrap().unwrap();
    let mut cursor = (0, 0);
    while let (Some(next), _) = store
        .reclaim_source_observations(&source, cursor, 32)
        .unwrap()
    {
        cursor = next;
    }
    let historical_tail = store.observations(&source).unwrap();
    assert_eq!(historical_tail.len(), 1);
    assert!(historical_tail[0].terminal_response.is_none());
    let (previous_revision, mut next) = store.checkpoint(&source).unwrap().unwrap();
    {
        let mut file = fs::OpenOptions::new().append(true).open(&source).unwrap();
        for mut row in pair(123, "next-day".into(), false) {
            let at = DateTime::parse_from_rfc3339(row["timestamp"].as_str().unwrap()).unwrap();
            row["timestamp"] = json!((at + chrono::Duration::days(1)).to_rfc3339());
            writeln!(file, "{row}").unwrap();
        }
    }
    assert!(
        next.read_with_budget(&source, retained, &mut Budget::new(std::iter::empty()))
            .unwrap()
    );
    let staged = store
        .prepare(
            SourceUpdate {
                source: &source,
                state: &next,
                previous_revision: Some(previous_revision),
                replace: false,
            },
            retained,
            BTreeMap::new(),
            APPEND_BUDGET,
        )
        .unwrap();
    store.publish(staged).unwrap();
    let latest = read(&mut store.db, i64::MIN, i64::MAX).1;
    assert_eq!(latest.len(), 1);
    assert_eq!(latest[0].delta.total_tokens, 110);
    let mut archive = Vec::new();
    Store::read_days(&mut store.db, day, retained, |_, _, groups| {
        archive.push(groups[0].totals.counts.0.total_tokens);
        Ok(())
    })
    .unwrap();
    assert_eq!(archive, vec![122 * 110]);
}

#[test]
fn delayed_counters_replace_committed_receipts_without_reloading_the_long_prefix() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().canonicalize().unwrap();
    let source = home.join("rollout-ordered.jsonl");
    let database = home.join("tokens.sqlite3");
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let base = DateTime::parse_from_rfc3339("2026-01-02T12:00:00Z").unwrap();
    let at = |second| (base + chrono::Duration::seconds(second)).to_rfc3339();
    let usage = |n| json!({"input_tokens":100*n,"cached_input_tokens":20*n,"output_tokens":10*n});
    let context = |second, turn| json!({"type":"turn_context","timestamp":at(second),"payload":{"turn_id":turn,"model":"fixture"}});
    let counter = |second, last, total| {
        json!({"type":"event_msg","timestamp":at(second),
        "payload":{"type":"token_count","info":{"last_token_usage":last,"total_token_usage":total}}})
    };
    let receipt = |second, n, response| {
        json!({"type":"token_usage_record","timestamp":at(second),
        "payload":{"thread_id":PARENT,"turn_id":"tail","response_id":response,
                   "usage":usage(1),"thread_token_usage":usage(n)}})
    };
    let mut prefix = vec![
        json!({"type":"session_meta","timestamp":at(0),
        "payload":{"id":PARENT,"model_provider":"supplier-a"}}),
        context(0, "prefix"),
    ];
    prefix.extend((1..=64).map(|n| counter(n, usage(1), usage(n))));
    let initial = parse(&source, &prefix, cutoff);
    let mut store = Store::open(Connection::open(&database).unwrap()).unwrap();
    let seed = store
        .prepare(
            new_source(&source, &initial),
            cutoff,
            BTreeMap::new(),
            1 << 20,
        )
        .unwrap();
    let first = store.publish(seed).unwrap();
    let source_bytes: usize = store
        .db
        .query_row(
            "SELECT sum(length(payload)) FROM token_observations",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let budget = 8192;
    assert!(source_bytes > budget);
    drop(store);
    let append = |rows: &[Value]| {
        let mut file = fs::OpenOptions::new().append(true).open(&source).unwrap();
        for row in rows {
            writeln!(file, "{row}").unwrap();
        }
    };
    let step = |store: &mut Store| {
        let mut reader = SourceReader::restore(store, &source).unwrap();
        reader.step(
            store,
            cutoff,
            &mut Budget::new(std::iter::empty()),
            &BTreeMap::new(),
            budget,
        )
    };
    let first_receipt = receipt(65, 65, "receipt-a");
    let second_receipt = receipt(68, 66, "receipt-b");
    let noise = json!({"type":"event_msg","timestamp":at(66),"payload":{"type":"agent_message","message":"synthetic"}});
    for (batch, expected) in [
        (vec![context(65, "tail"), first_receipt], 65),
        (vec![noise], 65),
        (vec![second_receipt], 66),
    ] {
        append(&batch);
        let mut store = Store::open(Connection::open(&database).unwrap()).unwrap();
        assert!(step(&mut store).unwrap());
        assert_eq!(read(&mut store.db, i64::MIN, i64::MAX).1.len(), expected);
        let anchor: i64 = store
            .db
            .query_row("SELECT revision FROM token_linear_sources", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(
            anchor, first,
            "receipts and cursor-only writes cannot replace the anchor"
        );
        prefix.extend(batch);
    }

    let late = counter(
        67,
        json!({"input_tokens":100,"output_tokens":10}),
        json!({"input_tokens":6500,"output_tokens":650}),
    );
    let mut store = Store::open(Connection::open(&database).unwrap()).unwrap();
    let committed = store.checkpoint(&source).unwrap().unwrap().1.offset;
    let before = read(&mut store.db, i64::MIN, i64::MAX);
    append(std::slice::from_ref(&late));
    prefix.push(late);
    store
        .db
        .execute_batch(
            "CREATE TRIGGER fail_late_count BEFORE UPDATE ON token_days
         BEGIN SELECT RAISE(ABORT, 'test rollback'); END;",
        )
        .unwrap();
    assert!(
        step(&mut store)
            .unwrap_err()
            .to_string()
            .contains("test rollback")
    );
    assert_eq!(
        store.checkpoint(&source).unwrap().unwrap().1.offset,
        committed
    );
    assert!(before == read(&mut store.db, i64::MIN, i64::MAX));
    store
        .db
        .execute_batch("DROP TRIGGER fail_late_count")
        .unwrap();
    drop(store);
    let mut store = Store::open(Connection::open(&database).unwrap()).unwrap();
    assert!(step(&mut store).unwrap());
    let (_, paired) = read(&mut store.db, i64::MIN, i64::MAX);
    assert_eq!(paired.len(), 66);
    assert!(paired[64].terminal_response.is_none());
    assert_eq!(paired[64].delta.cached_input_tokens, 20);
    assert_eq!(paired[65].terminal_response.as_deref(), Some("receipt-b"));

    let next = counter(69, usage(2), usage(67));
    append(std::slice::from_ref(&next));
    prefix.push(next);
    assert!(step(&mut store).unwrap());
    let actual = read(&mut store.db, i64::MIN, i64::MAX).1;
    assert_eq!(actual.len(), 66);
    assert_eq!(actual.last().unwrap().delta.total_tokens, 220);
    assert!(actual.iter().all(|event| event.terminal_response.is_none()));

    let mut replay = Store::open(Connection::open_in_memory().unwrap()).unwrap();
    let full = parse(&source, &prefix, cutoff);
    let staged = replay
        .prepare(new_source(&source, &full), cutoff, BTreeMap::new(), 1 << 20)
        .unwrap();
    replay.publish(staged).unwrap();
    assert!(actual == read(&mut replay.db, i64::MIN, i64::MAX).1);
    let query = Query::parse(&json!({}), cutoff).unwrap();
    let days = Store::read_selection(&mut store.db, &query, None, None)
        .unwrap()
        .days;
    assert_eq!(
        days,
        Store::read_selection(&mut replay.db, &query, None, None)
            .unwrap()
            .days
    );
    assert_eq!(days[0]["usage"]["totalTokens"], 67 * 110);
}

#[test]
fn late_receipt_repairs_a_sealed_counter_and_competition_revokes_only_its_cache() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().canonicalize().unwrap();
    let source = home.join("rollout-sealed.jsonl");
    let db = home.join("tokens.sqlite3");
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let base = DateTime::parse_from_rfc3339("2026-01-02T12:00:00Z").unwrap();
    let at = |second| (base + chrono::Duration::seconds(second)).to_rfc3339();
    let usage = |n| json!({"input_tokens":100*n,"cached_input_tokens":20*n,"output_tokens":10*n});
    let mut input = vec![
        json!({"type":"session_meta","timestamp":at(0),"payload":{"id":PARENT,"model_provider":"supplier-a"}}),
        json!({"type":"turn_context","timestamp":at(0),"payload":{"turn_id":"same-turn","model":"fixture"}}),
    ];
    for n in 1..=66 {
        let mut last = usage(1);
        let mut total = usage(n);
        if n == 65 {
            last.as_object_mut().unwrap().remove("cached_input_tokens");
            total.as_object_mut().unwrap().remove("cached_input_tokens");
        }
        input.push(
            json!({"type":"event_msg","timestamp":at(n),"payload":{"type":"token_count",
            "info":{"last_token_usage":last,"total_token_usage":total}}}),
        );
    }
    let initial = parse(&source, &input, cutoff);
    let mut store = Store::open(Connection::open(&db).unwrap()).unwrap();
    let staged = store
        .prepare(
            new_source(&source, &initial),
            cutoff,
            BTreeMap::new(),
            1 << 20,
        )
        .unwrap();
    let native_revision = store.publish(staged).unwrap();
    let size: usize = store
        .db
        .query_row(
            "SELECT sum(length(payload)) FROM token_observations",
            [],
            |row| row.get(0),
        )
        .unwrap();
    const BUDGET: usize = 8192;
    assert!(size > BUDGET);
    let native = read(&mut store.db, i64::MIN, i64::MAX).1;
    assert!(!native[64].delta.cached_known);
    assert!(native[65].delta.cached_known);
    drop(store);

    for n in 1..=3 {
        let receipt = json!({"type":"token_usage_record","timestamp":at(66+n),
            "payload":{"thread_id":PARENT,"turn_id":"same-turn","response_id":format!("claim-{n}"),
                       "usage":usage(1),"thread_token_usage":usage(65)}});
        writeln!(
            fs::OpenOptions::new().append(true).open(&source).unwrap(),
            "{receipt}"
        )
        .unwrap();
        input.push(receipt);
        let mut store = Store::open(Connection::open(&db).unwrap()).unwrap();
        let mut reader = SourceReader::restore(&store, &source).unwrap();
        assert!(
            reader
                .step(
                    &mut store,
                    cutoff,
                    &mut Budget::new(std::iter::empty()),
                    &BTreeMap::new(),
                    BUDGET
                )
                .unwrap()
        );
        let certificate: i64 = store
            .db
            .query_row("SELECT revision FROM token_linear_sources", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(certificate, native_revision);
        let actual = read(&mut store.db, i64::MIN, i64::MAX).1;
        assert_eq!(actual.len(), 66);
        assert_eq!(actual[64].id, native[64].id);
        assert_eq!(actual[64].delta.total_tokens, 110);
        assert_eq!(actual[64].delta.cached_known, n == 1);
        let mut replay = Store::open(Connection::open_in_memory().unwrap()).unwrap();
        let copy = home.join("rollout-replay.jsonl");
        let full = parse(&copy, &input, cutoff);
        let staged = replay
            .prepare(new_source(&copy, &full), cutoff, BTreeMap::new(), 1 << 20)
            .unwrap();
        replay.publish(staged).unwrap();
        assert!(actual == read(&mut replay.db, i64::MIN, i64::MAX).1);
        let query = Query::parse(&json!({}), cutoff).unwrap();
        let daily = Store::read_selection(&mut store.db, &query, None, None)
            .unwrap()
            .days;
        assert_eq!(
            daily,
            Store::read_selection(&mut replay.db, &query, None, None)
                .unwrap()
                .days
        );
        assert_eq!(daily[0]["usage"]["totalTokens"], 66 * 110);
        if n == 1 {
            store
                .record_coverage(Coverage {
                    pending_sources: 0,
                    issues: 0,
                    maintenance_blocked: false,
                })
                .unwrap();
            assert!(
                store
                    .reclassify_session(PARENT, 1 << 20, &BTreeSet::new(), |rows, _| {
                        for row in &mut rows[64..] {
                            row.provider = "fixture-proof".into();
                            row.provider_source = Some("request-log-exact".into());
                        }
                        Ok(())
                    })
                    .unwrap()
                    .is_some()
            );
        }
    }
}

#[test]
fn source_ingest_retries_a_failed_batch_then_completes_the_split_line_after_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let source = root.join("rollout-a.jsonl");
    let database = root.join("tokens.sqlite3");
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let prefix = format!("{}\n{}\n", header(PARENT, 0), count(1, 1, 1, true));
    let tail = count(2, 2, 2, true).to_string();
    fs::write(&source, format!("{prefix}{}", &tail[..20])).unwrap();
    let mut store = Store::open(Connection::open(&database).unwrap()).unwrap();
    let mut reader = SourceReader::restore(&store, &source).unwrap();
    let mut budget = Budget::new(std::iter::empty());
    let names = BTreeMap::new();
    store
        .db
        .execute_batch(
            "CREATE TRIGGER fail_batch BEFORE INSERT ON token_days
             BEGIN SELECT RAISE(ABORT, 'test publish failure'); END;",
        )
        .unwrap();
    let error = reader
        .step(&mut store, cutoff, &mut budget, &names, 8192)
        .unwrap_err();
    assert!(error.to_string().contains("test publish failure"));
    assert!(store.checkpoint(&source).unwrap().is_none());
    assert!(read(&mut store.db, i64::MIN, i64::MAX).1.is_empty());
    store.db.execute_batch("DROP TRIGGER fail_batch").unwrap();
    let bytes_left = budget.read_left;
    assert!(
        !reader
            .step(&mut store, cutoff, &mut budget, &names, 8192)
            .unwrap()
    );
    assert_eq!(budget.read_left, bytes_left);
    assert_eq!(
        store.checkpoint(&source).unwrap().unwrap().1.offset,
        prefix.len() as u64
    );
    let (_, observed) = read(&mut store.db, i64::MIN, i64::MAX);
    assert_eq!(observed.len(), 1);
    assert_eq!(observed[0].delta.total_tokens, 110);
    drop(store);

    writeln!(
        fs::OpenOptions::new().append(true).open(&source).unwrap(),
        "{}",
        &tail[20..]
    )
    .unwrap();
    let mut store = Store::open(Connection::open(&database).unwrap()).unwrap();
    let mut reader = SourceReader::restore(&store, &source).unwrap();
    let mut budget = Budget::new(std::iter::empty());
    assert!(
        reader
            .step(&mut store, cutoff, &mut budget, &names, 8192)
            .unwrap()
    );
    let (revision, observed) = read(&mut store.db, i64::MIN, i64::MAX);
    assert_eq!(observed.len(), 2);
    assert!(observed.iter().all(|event| event.delta.total_tokens == 110));
    assert!(
        reader
            .step(&mut store, cutoff, &mut budget, &names, 8192)
            .unwrap()
    );
    assert_eq!(read(&mut store.db, i64::MIN, i64::MAX).0, revision);
}

#[test]
fn source_ingest_keeps_committed_results_until_a_replacement_is_complete() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let source = root.join("rollout-a.jsonl");
    let database = root.join("tokens.sqlite3");
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    fs::write(
        &source,
        format!(
            "{}\n{}\n{}\n",
            header(PARENT, 0),
            count(1, 1, 1, true),
            count(2, 2, 2, true)
        ),
    )
    .unwrap();
    let mut store = Store::open(Connection::open(&database).unwrap()).unwrap();
    let mut reader = SourceReader::restore(&store, &source).unwrap();
    let names = BTreeMap::new();
    let mut budget = Budget::new(std::iter::empty());
    assert!(
        reader
            .step(&mut store, cutoff, &mut budget, &names, 8192)
            .unwrap()
    );
    let (before, observed) = read(&mut store.db, i64::MIN, i64::MAX);
    assert_eq!(observed.len(), 2);
    let replacement = format!("{}\n{}\n", header(OTHER, 0), count(1, 3, 1, true));
    fs::write(&source, replacement).unwrap();
    let mut budget = Budget::new(std::iter::empty());
    budget.read_left = 150;
    assert!(
        !reader
            .step(&mut store, cutoff, &mut budget, &names, 8192)
            .unwrap()
    );
    let (revision, still_visible) = read(&mut store.db, i64::MIN, i64::MAX);
    assert_eq!(revision, before);
    assert!(still_visible == observed);
    let mut budget = Budget::new(std::iter::empty());
    assert!(
        reader
            .step(&mut store, cutoff, &mut budget, &names, 8192)
            .unwrap()
    );
    let (after, revised) = read(&mut store.db, i64::MIN, i64::MAX);
    assert_eq!(after, before + 1);
    assert_eq!(revised.len(), 1);
    assert_eq!(revised[0].session, OTHER);
}

#[test]
fn source_ingest_resumes_a_compressed_prefix_across_bounded_rounds() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let source = root.join("rollout-a.jsonl.zst");
    let database = root.join("tokens.sqlite3");
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let prefix = format!("{}\n{}\n", header(PARENT, 0), count(1, 1, 1, true));
    let full = format!("{prefix}{}\n", count(2, 2, 2, true));
    fs::write(
        &source,
        zstd::stream::encode_all(full.as_bytes(), 1).unwrap(),
    )
    .unwrap();
    let names = BTreeMap::new();
    let mut store = Store::open(Connection::open(&database).unwrap()).unwrap();
    let mut reader = SourceReader::restore(&store, &source).unwrap();
    let mut budget = Budget::new(std::iter::empty());
    budget.read_left = prefix.len() as u64;
    assert!(
        !reader
            .step(&mut store, cutoff, &mut budget, &names, 8192)
            .unwrap()
    );
    let (before, observed) = read(&mut store.db, i64::MIN, i64::MAX);
    assert_eq!(observed.len(), 1);
    drop(store);

    let mut store = Store::open(Connection::open(&database).unwrap()).unwrap();
    let mut reader = SourceReader::restore(&store, &source).unwrap();
    let mut budget = Budget::new(std::iter::empty());
    budget.read_left = 80;
    assert!(
        !reader
            .step(&mut store, cutoff, &mut budget, &names, 8192)
            .unwrap()
    );
    assert_eq!(budget.read_left, 0);
    assert_eq!(read(&mut store.db, i64::MIN, i64::MAX).0, before);
    let mut complete = false;
    for _ in 0..full.len().div_ceil(80) {
        let mut budget = Budget::new(std::iter::empty());
        budget.read_left = 80;
        complete = reader
            .step(&mut store, cutoff, &mut budget, &names, 8192)
            .unwrap();
        if complete {
            break;
        }
    }
    assert!(complete);
    let (_, observed) = read(&mut store.db, i64::MIN, i64::MAX);
    assert_eq!(observed.len(), 2);
    assert!(observed.iter().all(|event| event.delta.total_tokens == 110));
    let (_, checkpoint) = store.checkpoint(&source).unwrap().unwrap();
    assert_eq!(checkpoint.offset, full.len() as u64);
    assert!(!checkpoint.resume_pending);
}

#[test]
fn late_physical_prefix_repairs_only_affected_sources_and_preserves_reader_revision() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let parent_path = root.join(format!("rollout-prefix-{PHYSICAL_PREFIX}.jsonl"));
    let child_path = root.join("rollout-child.jsonl");
    let other_path = root.join("rollout-other.jsonl");
    let parent = parse(
        &parent_path,
        &[header(PARENT, 0), count(1, 1, 1, true)],
        cutoff,
    );
    let mut child_header = header(CHILD, 2);
    child_header["payload"]["history_base"] = json!({"thread_id":PHYSICAL_PREFIX,
        "end_ordinal_exclusive":2,"end_byte_offset":parent.offset});
    let child = parse(&child_path, &[child_header, count(3, 2, 2, false)], cutoff);
    let mut other = parse(
        &other_path,
        &[header(OTHER, 0), count(1, 3, 1, true)],
        cutoff,
    );
    other.events[0].model = "large-fixture".repeat(4_000);

    let file = root.join("tokens.sqlite3");
    let mut store = Store::open(Connection::open(&file).unwrap()).unwrap();
    let mut reader = Connection::open_with_flags(&file, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let prepared = store
        .prepare(
            new_source(&other_path, &other),
            cutoff,
            BTreeMap::new(),
            1 << 20,
        )
        .unwrap();
    store.publish(prepared).unwrap();
    let prepared = store
        .prepare(
            new_source(&child_path, &child),
            cutoff,
            BTreeMap::new(),
            8192,
        )
        .unwrap();
    let before = store.publish(prepared).unwrap();

    let pending = store
        .prepare(
            new_source(&parent_path, &parent),
            cutoff,
            BTreeMap::new(),
            8192,
        )
        .unwrap();
    let mut pending = Some(pending);
    let mut seen = Vec::new();
    let revision = Store::read_committed(&mut reader, i64::MIN, i64::MAX, |event| {
        if let Some(prepared) = pending.take() {
            assert_eq!(store.publish(prepared).unwrap(), before + 1);
        }
        seen.push(event);
        Ok(())
    })
    .unwrap();
    assert_eq!(revision, before);
    assert_eq!(seen.len(), 2);
    assert!(seen[0].count_pending);
    assert_eq!(seen[1].session, OTHER);

    let (after, events) = read(&mut reader, i64::MIN, i64::MAX);
    assert_eq!(after, before + 1);
    assert_eq!(events.len(), 3);
    assert_eq!(events[0].session, PARENT);
    assert_eq!(events[1].session, CHILD);
    assert_eq!(events[1].delta.total_tokens, 110);
    assert!(!events[1].count_pending);
    assert!(!events[1].count_uncertain);
    assert!(events[2] == seen[1]);

    let child_ms = DateTime::parse_from_rfc3339(&events[1].timestamp)
        .unwrap()
        .timestamp_millis();
    let (_, exact) = read(&mut reader, child_ms, child_ms + 1);
    assert!(exact == events[1..2]);
    assert!(read(&mut reader, child_ms + 1, child_ms + 1).1.is_empty());
}

#[test]
fn replacing_scope_uses_old_and_new_links_and_failed_publication_keeps_the_old_view() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let file = root.join("tokens.sqlite3");
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let mut store = Store::open(Connection::open(&file).unwrap()).unwrap();
    let mut reader = Connection::open_with_flags(&file, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let a = root.join("rollout-a.jsonl");
    let b = root.join("rollout-b.jsonl");
    let c = root.join("rollout-c.jsonl");
    let original = parse(&a, &[header(PARENT, 0), count(1, 1, 1, true)], cutoff);
    let duplicate = parse(&b, &[header(PARENT, 0), count(1, 1, 1, true)], cutoff);
    let new_component = parse(&c, &[header(CHILD, 0), count(1, 2, 1, true)], cutoff);
    for (path, state) in [(&a, &original), (&b, &duplicate), (&c, &new_component)] {
        let next = store
            .prepare(new_source(path, state), cutoff, BTreeMap::new(), 8192)
            .unwrap();
        store.publish(next).unwrap();
    }
    let (old, before) = read(&mut reader, i64::MIN, i64::MAX);
    assert_eq!(before.len(), 2);
    let day = before[0].date;
    let summaries = |db: &mut Connection| {
        let mut groups = Vec::new();
        let revision = Store::read_days(db, day, day.succ_opt().unwrap(), |session, _, rows| {
            groups.extend(rows.into_iter().map(|group| {
                assert_eq!(group.provider, "supplier-a");
                assert_eq!(group.conflicts, 0);
                (session.clone(), group.totals.records, group.totals.counts.0)
            }));
            Ok(())
        })
        .unwrap();
        (revision, groups)
    };
    let (summary_revision, before_summary) = summaries(&mut reader);
    assert_eq!(summary_revision, old);
    assert_eq!(before_summary.len(), 2);
    for (event, (session, records, counts)) in before.iter().zip(&before_summary) {
        assert_eq!(&event.session, session);
        assert_eq!(*records, 1);
        assert_eq!(*counts, event.delta);
    }
    let (a_revision, old_checkpoint) = store.checkpoint(&a).unwrap().unwrap();

    let replacement = parse(&a, &[header(CHILD, 0), count(1, 3, 2, true)], cutoff);
    let update = SourceUpdate {
        source: &a,
        state: &replacement,
        previous_revision: Some(a_revision),
        replace: true,
    };
    let too_small = store.prepare(update, cutoff, BTreeMap::new(), 128);
    assert!(too_small.is_err());
    assert_eq!(read(&mut reader, i64::MIN, i64::MAX).0, old);
    let pending = store
        .prepare(update, cutoff, BTreeMap::new(), 8192)
        .unwrap();
    store
        .db
        .execute_batch(
            "CREATE TEMP TRIGGER fail_token_projection BEFORE INSERT ON token_days
         BEGIN SELECT RAISE(ABORT, 'projection write failed'); END;",
        )
        .unwrap();
    assert!(
        store
            .publish(pending)
            .unwrap_err()
            .to_string()
            .contains("projection write failed")
    );
    assert_eq!(store.checkpoint(&a).unwrap().unwrap().0, a_revision);
    assert_eq!(
        store.checkpoint(&a).unwrap().unwrap().1.offset,
        old_checkpoint.offset
    );
    let (after_error, events) = read(&mut reader, i64::MIN, i64::MAX);
    assert_eq!(after_error, old);
    assert!(events == before);
    assert_eq!(summaries(&mut reader), (old, before_summary));
    store
        .db
        .execute_batch("DROP TRIGGER fail_token_projection")
        .unwrap();

    let prepared = store
        .prepare(update, cutoff, BTreeMap::new(), 8192)
        .unwrap();
    let stale = store
        .prepare(update, cutoff, BTreeMap::new(), 8192)
        .unwrap();
    let revision = store.publish(prepared).unwrap();
    assert!(
        store
            .publish(stale)
            .unwrap_err()
            .to_string()
            .contains("changed during reconciliation")
    );
    let rejected_old_cursor = store.prepare(update, cutoff, BTreeMap::new(), 8192);
    assert!(
        rejected_old_cursor
            .err()
            .unwrap()
            .to_string()
            .contains("changed before reconciliation")
    );
    let (after, events) = read(&mut reader, i64::MIN, i64::MAX);
    assert_eq!(after, revision);
    assert_eq!(after, old + 1);
    assert_eq!(
        events
            .iter()
            .map(|event| event.session.as_str())
            .collect::<Vec<_>>(),
        vec![PARENT, CHILD, CHILD]
    );
    assert!(events[0] == before[0]);
    let (summary_revision, groups) = summaries(&mut reader);
    assert_eq!(summary_revision, revision);
    assert_eq!(groups.iter().map(|(_, count, _)| count).sum::<u64>(), 3);
    assert_eq!(
        groups
            .iter()
            .map(|(_, _, counts)| counts.total_tokens)
            .sum::<u64>(),
        events
            .iter()
            .map(|event| event.delta.total_tokens)
            .sum::<u64>(),
    );
}

#[test]
fn copies_sharing_one_session_resolve_once_without_affecting_other_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let source = root.join("rollout-original.jsonl");
    let original = parse(&source, &[header(PARENT, 0), count(1, 1, 1, true)], cutoff);
    let other_path = root.join("rollout-other.jsonl");
    let other = parse(
        &other_path,
        &[header(OTHER, 0), count(1, 4, 1, true)],
        cutoff,
    );
    let mut store = Store::open(Connection::open_in_memory().unwrap()).unwrap();
    let prepared = store
        .prepare(
            new_source(&other_path, &other),
            cutoff,
            BTreeMap::new(),
            8192,
        )
        .unwrap();
    store.publish(prepared).unwrap();
    for copy in 0..96 {
        store
            .save(
                &root.join(format!("rollout-copy-{copy}.jsonl")),
                None,
                &original,
                false,
            )
            .unwrap();
    }
    let (before, events) = read(&mut store.db, i64::MIN, i64::MAX);
    assert_eq!(events.len(), 1);
    assert!(matches!(
        store.prepare(new_source(&source, &original), cutoff, BTreeMap::new(), 8192),
        Err(error) if error.to_string().contains("reconciliation input budget")
    ));
    assert_eq!(read(&mut store.db, i64::MIN, i64::MAX).0, before);
    assert!(store.checkpoint(&source).unwrap().is_none());

    let prepared = store
        .prepare(
            new_source(&source, &original),
            cutoff,
            BTreeMap::new(),
            1 << 20,
        )
        .unwrap();
    assert_eq!(store.publish(prepared).unwrap(), before + 1);
    let (_, resolved) = read(&mut store.db, i64::MIN, i64::MAX);
    assert_eq!(resolved.len(), 2);
    assert_eq!(resolved[0].session, PARENT);
    assert_eq!(resolved[0].delta.total_tokens, 110);
    assert!(resolved[1] == events[0]);
}

#[test]
fn ordered_append_matches_full_resolution_after_reopen_and_late_copy() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let a = root.join("rollout-a.jsonl");
    let copy = root.join("rollout-copy.jsonl");
    let full = root.join("rollout-full.jsonl");
    let file = root.join("tokens.sqlite3");
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let head = [header(PARENT, 0), count(1, 1, 1, true)];
    let initial = parse(&a, &head, cutoff);
    let mut store = Store::open(Connection::open(&file).unwrap()).unwrap();
    let first = store
        .prepare(new_source(&a, &initial), cutoff, BTreeMap::new(), 8192)
        .unwrap();
    store.publish(first).unwrap();
    drop(store);

    let additional = count(2, 2, 2, true);
    writeln!(
        fs::OpenOptions::new().append(true).open(&a).unwrap(),
        "{additional}"
    )
    .unwrap();
    let mut store = Store::open(Connection::open(&file).unwrap()).unwrap();
    let (at, mut state) = store.checkpoint(&a).unwrap().unwrap();
    assert!(
        state
            .read_with_budget(&a, cutoff, &mut Budget::new(std::iter::empty()))
            .unwrap()
    );
    let update = SourceUpdate {
        source: &a,
        state: &state,
        previous_revision: Some(at),
        replace: false,
    };
    let next = store
        .prepare(update, cutoff, BTreeMap::new(), 8192)
        .unwrap();
    let revision = store.publish(next).unwrap();
    let (_, appended) = read(&mut store.db, i64::MIN, i64::MAX);

    let whole = parse(
        &full,
        &[head[0].clone(), head[1].clone(), additional],
        cutoff,
    );
    let mut reference = Store::open(Connection::open_in_memory().unwrap()).unwrap();
    let next = reference
        .prepare(new_source(&full, &whole), cutoff, BTreeMap::new(), 8192)
        .unwrap();
    reference.publish(next).unwrap();
    assert!(read(&mut reference.db, i64::MIN, i64::MAX).1 == appended);
    assert_eq!(appended.len(), 2);

    let duplicate = parse(&copy, &head, cutoff);
    assert!(
        store
            .prepare(new_source(&copy, &duplicate), cutoff, BTreeMap::new(), 128)
            .is_err()
    );
    assert_eq!(read(&mut store.db, i64::MIN, i64::MAX).0, revision);
    let next = store
        .prepare(new_source(&copy, &duplicate), cutoff, BTreeMap::new(), 8192)
        .unwrap();
    store.publish(next).unwrap();
    assert!(read(&mut store.db, i64::MIN, i64::MAX).1 == appended);
    let proofs: u64 = store
        .db
        .query_row("SELECT count(*) FROM token_linear_sources", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(
        proofs, 0,
        "a competing source revokes the cheap append certificate"
    );
}

#[test]
fn advancing_retention_does_not_rehydrate_an_ordered_source() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let source = root.join("rollout-retention.jsonl");
    let file = root.join("tokens.sqlite3");
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let head: Vec<_> = std::iter::once(header(PARENT, 0))
        .chain((1..=50).map(|pos| count(pos, pos, pos as u64, true)))
        .collect();
    let initial = parse(&source, &head, cutoff);
    let expired = initial.events[0].date;
    let retained = expired.succ_opt().unwrap();
    let mut store = Store::open(Connection::open(&file).unwrap()).unwrap();
    assert_eq!(store.reclaim_expired_details(7).unwrap(), 0);
    let first = store
        .prepare(
            new_source(&source, &initial),
            cutoff,
            BTreeMap::new(),
            1 << 20,
        )
        .unwrap();
    let first_revision = store.publish(first).unwrap();
    drop(store);

    let mut row = count(51, 51, 51, true);
    row["timestamp"] = "2026-01-03T12:00:51Z".into();
    writeln!(
        fs::OpenOptions::new().append(true).open(&source).unwrap(),
        "{row}"
    )
    .unwrap();
    let mut store = Store::open(Connection::open(&file).unwrap()).unwrap();
    let (base, mut tail) = store.checkpoint(&source).unwrap().unwrap();
    assert_eq!(base, first_revision);
    assert!(
        tail.read_with_budget(&source, retained, &mut Budget::new(std::iter::empty()))
            .unwrap()
    );
    let update = SourceUpdate {
        source: &source,
        state: &tail,
        previous_revision: Some(base),
        replace: false,
    };
    let next = store
        .prepare(update, retained, BTreeMap::new(), 8192)
        .unwrap();
    let mut reader = Connection::open_with_flags(&file, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    store
        .db
        .execute_batch(
            "CREATE TRIGGER reject_retention BEFORE UPDATE OF precise_from ON token_revision
         BEGIN SELECT RAISE(ABORT, 'retention write failed'); END;",
        )
        .unwrap();
    assert!(
        store
            .publish(next)
            .unwrap_err()
            .to_string()
            .contains("retention write failed")
    );
    let (unchanged, previous) = read(&mut reader, i64::MIN, i64::MAX);
    assert_eq!(unchanged, first_revision);
    assert_eq!(previous.len(), 50);
    assert_eq!(
        store.checkpoint(&source).unwrap().unwrap().0,
        first_revision
    );
    store
        .db
        .execute_batch("DROP TRIGGER reject_retention")
        .unwrap();
    let mut next = Some(
        store
            .prepare(update, retained, BTreeMap::new(), 8192)
            .unwrap(),
    );
    let mut seen = 0;
    let snapshot = Store::read_committed(&mut reader, i64::MIN, i64::MAX, |_| {
        if let Some(next) = next.take() {
            assert_eq!(store.publish(next).unwrap(), first_revision + 1);
            assert_eq!(store.reclaim_expired_details(7).unwrap(), 7);
        }
        seen += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!((snapshot, seen), (first_revision, 50));
    let count_details = |store: &Store| {
        store
            .db
            .query_row("SELECT COUNT(*) FROM token_resolved", [], |row| {
                row.get::<_, usize>(0)
            })
            .unwrap()
    };
    assert_eq!(count_details(&store), 44);
    store
        .db
        .execute_batch(
            "CREATE TRIGGER reject_detail_reclaim BEFORE DELETE ON token_resolved
             BEGIN SELECT RAISE(ABORT, 'detail reclaim failed'); END;",
        )
        .unwrap();
    assert!(
        store
            .reclaim_expired_details(7)
            .unwrap_err()
            .to_string()
            .contains("detail reclaim failed")
    );
    assert_eq!(count_details(&store), 44);
    store
        .db
        .execute_batch("DROP TRIGGER reject_detail_reclaim")
        .unwrap();
    assert_eq!(store.reclaim_expired_details(7).unwrap(), 7);
    assert_eq!(store.reclaim_expired_details(100).unwrap(), 36);
    assert_eq!(store.reclaim_expired_details(100).unwrap(), 0);
    assert_eq!(count_details(&store), 1);
    assert_eq!(store.observations(&source).unwrap().len(), 51);
    assert_eq!(
        store.checkpoint(&source).unwrap().unwrap().1.offset,
        tail.offset
    );
    let (actual_revision, events) = read(&mut reader, i64::MIN, i64::MAX);
    assert_eq!(actual_revision, first_revision + 1);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].date, retained);
    assert_eq!(events[0].delta.total_tokens, 110);
    let mut summaries = Vec::new();
    let summary_revision = Store::read_days(
        &mut reader,
        expired,
        retained.succ_opt().unwrap(),
        |_, day, groups| {
            summaries.push((
                day,
                groups[0].totals.records,
                groups[0].totals.counts.0.total_tokens,
            ));
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(summary_revision, actual_revision);
    assert_eq!(summaries, vec![(expired, 50, 5500), (retained, 1, 110)]);
}

#[test]
fn reclaiming_archived_observations_unblocks_a_referenced_continuation() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let source = root.join(format!("rollout-prefix-{PHYSICAL_PREFIX}.jsonl"));
    let old_cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let head: Vec<_> = std::iter::once(header(PARENT, 0))
        .chain((1..=50).map(|pos| count(pos, pos, pos as u64, true)))
        .collect();
    let parent = parse(&source, &head, old_cutoff);
    let old_day = parent.events[0].date;
    let cutoff = old_day.succ_opt().unwrap();
    let old_offset = parent.offset;
    let file = root.join("tokens.sqlite3");
    let mut store = Store::open(Connection::open(&file).unwrap()).unwrap();
    let initial = store
        .prepare(
            new_source(&source, &parent),
            old_cutoff,
            BTreeMap::new(),
            1 << 20,
        )
        .unwrap();
    let parent_revision = store.publish(initial).unwrap();

    let child_path = root.join("rollout-child.jsonl");
    let mut child_header = header(CHILD, 51);
    child_header["payload"]["history_base"] = json!({"thread_id":PHYSICAL_PREFIX,
        "end_ordinal_exclusive":51,"end_byte_offset":old_offset});
    let mut child_count = count(52, 52, 51, false);
    child_count["timestamp"] = Local
        .from_local_datetime(&cutoff.and_hms_opt(12, 0, 0).unwrap())
        .single()
        .unwrap()
        .to_rfc3339()
        .into();
    let child = parse(&child_path, &[child_header, child_count], cutoff);
    let stale = store
        .prepare(
            new_source(&child_path, &child),
            old_cutoff,
            BTreeMap::new(),
            1 << 20,
        )
        .unwrap();
    // 没有其他新来源时也能推进保留线，并拒绝基于旧保留线准备的写入。
    let before = store.advance_retention(cutoff).unwrap().unwrap();
    assert_eq!(before, parent_revision + 1);
    assert_eq!(store.advance_retention(cutoff).unwrap(), None);
    assert_eq!(store.advance_retention(old_cutoff).unwrap(), None);
    assert!(matches!(
        store.publish(stale),
        Err(error) if error.to_string().contains("index changed during reconciliation")
    ));
    assert!(store.checkpoint(&child_path).unwrap().is_none());
    let (_, mut stale_parser) = store.checkpoint(&source).unwrap().unwrap();
    assert!(matches!(
        store.prepare(new_source(&child_path, &child), cutoff, BTreeMap::new(), 8192),
        Err(error) if error.to_string().contains("reconciliation input budget")
    ));

    store
        .db
        .execute_batch(
            "CREATE TRIGGER fail_reclaim BEFORE UPDATE OF checkpoint ON token_sources
         BEGIN SELECT RAISE(ABORT, 'compaction checkpoint failed'); END;",
        )
        .unwrap();
    assert!(matches!(
        store.reclaim_source_observations(&source, (0, 0), 7),
        Err(error) if error.to_string().contains("compaction checkpoint failed")
    ));
    assert_eq!(store.observations(&source).unwrap().len(), 50);
    assert!(
        store
            .checkpoint(&source)
            .unwrap()
            .unwrap()
            .1
            .expired_count
            .is_none()
    );
    store.db.execute_batch("DROP TRIGGER fail_reclaim").unwrap();

    let mut cursor = (0, 0);
    let mut removed = 0;
    for _ in 0..8 {
        let (next, batch) = store
            .reclaim_source_observations(&source, cursor, 7)
            .unwrap();
        assert!(batch <= 7);
        removed += batch;
        if let Some(next) = next {
            cursor = next;
        } else {
            break;
        }
    }
    assert_eq!(removed, 49);
    assert_eq!(store.observations(&source).unwrap().len(), 1);
    let (source_revision, checkpoint) = store.checkpoint(&source).unwrap().unwrap();
    assert_eq!(
        (source_revision, checkpoint.offset),
        (parent_revision, old_offset)
    );
    assert_eq!(checkpoint.expired_count.unwrap().0.0, 49);
    assert_eq!(read(&mut store.db, i64::MIN, i64::MAX).0, before);

    drop(store);
    let mut store = Store::open(Connection::open(&file).unwrap()).unwrap();
    writeln!(
        fs::OpenOptions::new().append(true).open(&source).unwrap(),
        "{}",
        json!({"type":"event_msg","ordinal":51,"timestamp":"2026-01-02T12:00:51Z",
            "payload":{"type":"task_complete"}})
    )
    .unwrap();
    assert!(
        stale_parser
            .read_with_budget(&source, cutoff, &mut Budget::new(std::iter::empty()))
            .unwrap()
    );
    assert!(stale_parser.expired_count.is_none());
    let updated_parent = store
        .prepare(
            SourceUpdate {
                source: &source,
                state: &stale_parser,
                previous_revision: Some(parent_revision),
                replace: false,
            },
            cutoff,
            BTreeMap::new(),
            8192,
        )
        .unwrap();
    let before = store.publish(updated_parent).unwrap();
    let (_, checkpoint) = store.checkpoint(&source).unwrap().unwrap();
    assert!(checkpoint.offset > old_offset);
    assert_eq!(checkpoint.expired_count.unwrap().0.0, 49);

    let ready = store
        .prepare(
            new_source(&child_path, &child),
            cutoff,
            BTreeMap::new(),
            8192,
        )
        .unwrap();
    assert_eq!(store.publish(ready).unwrap(), before + 1);
    let (_, events) = read(&mut store.db, i64::MIN, i64::MAX);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].session, CHILD);
    assert_eq!(events[0].delta.total_tokens, 110);
    assert!(!events[0].count_pending && !events[0].count_uncertain);
    let mut archived = 0;
    Store::read_days(&mut store.db, old_day, cutoff, |session, day, groups| {
        archived += 1;
        assert_eq!((session.as_str(), day), (PARENT, old_day));
        assert_eq!(
            (
                groups[0].totals.records,
                groups[0].totals.counts.0.total_tokens
            ),
            (50, 5500)
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(archived, 1);
}

#[test]
fn cross_retention_range_reads_one_revision_without_inventing_partial_days() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let source = root.join("rollout-range.jsonl");
    let file = root.join("tokens.sqlite3");
    let archived = NaiveDate::from_ymd_opt(2026, 1, 2).unwrap();
    let retained = archived.succ_opt().unwrap();
    let at = |day: NaiveDate, hour, minute| {
        Local
            .from_local_datetime(&day.and_hms_opt(hour, minute, 0).unwrap())
            .single()
            .unwrap()
    };
    let timed_count = |day, minute, ordinal| {
        let mut event = count(ordinal, ordinal, ordinal as u64, true);
        event["timestamp"] = at(day, 12, minute).to_rfc3339().into();
        event
    };
    let initial = parse(
        &source,
        &[
            header(PARENT, 0),
            timed_count(archived, 0, 1),
            timed_count(retained, 0, 2),
        ],
        archived,
    );
    let mut store = Store::open(Connection::open(&file).unwrap()).unwrap();
    let initial = store
        .prepare(
            new_source(&source, &initial),
            archived,
            BTreeMap::new(),
            8192,
        )
        .unwrap();
    store.publish(initial).unwrap();
    let later_source = root.join("rollout-advance-retention.jsonl");
    let later = parse(
        &later_source,
        &[
            header(OTHER, 0),
            timed_count(retained.succ_opt().unwrap(), 0, 1),
        ],
        retained,
    );
    let advance = store
        .prepare(
            new_source(&later_source, &later),
            retained,
            BTreeMap::new(),
            8192,
        )
        .unwrap();
    let revision = store.publish(advance).unwrap();
    writeln!(
        fs::OpenOptions::new().append(true).open(&source).unwrap(),
        "{}",
        timed_count(retained, 30, 3)
    )
    .unwrap();
    let (source_revision, mut tail) = store.checkpoint(&source).unwrap().unwrap();
    assert!(
        tail.read_with_budget(&source, retained, &mut Budget::new(std::iter::empty()))
            .unwrap()
    );
    let append = store
        .prepare(
            SourceUpdate {
                source: &source,
                state: &tail,
                previous_revision: Some(source_revision),
                replace: false,
            },
            retained,
            BTreeMap::new(),
            8192,
        )
        .unwrap();
    let mut append = Some(append);
    let mut reader = Connection::open_with_flags(&file, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let start = at(archived, 0, 0).timestamp_millis();
    let end = at(retained, 13, 0).timestamp_millis();
    let mut exact = Vec::new();
    let mut days = Vec::new();
    let snapshot = Store::read_range(
        &mut reader,
        start,
        end,
        |event| {
            exact.push((event.date, event.delta.total_tokens));
            Ok(())
        },
        |session, day, groups| {
            assert_eq!(session, PARENT);
            assert_eq!(groups.len(), 1);
            days.push((
                day,
                groups[0].totals.records,
                groups[0].totals.counts.0.total_tokens,
            ));
            assert_eq!(store.publish(append.take().unwrap()).unwrap(), revision + 1);
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(snapshot, (revision, true));
    assert_eq!(days, vec![(archived, 1, 110)]);
    assert_eq!(exact, vec![(retained, 110)]);

    for (from, to, missing_exact, expected_events) in [
        (at(retained, 12, 0), at(retained, 12, 30), false, 1),
        (at(archived, 12, 0), at(retained, 13, 0), true, 2),
        (at(archived, 12, 0), at(archived, 13, 0), true, 0),
    ] {
        let mut detail = 0;
        let snapshot = Store::read_range(
            &mut reader,
            from.timestamp_millis(),
            to.timestamp_millis(),
            |event| {
                assert_eq!(event.date, retained);
                assert_eq!(event.delta.total_tokens, 110);
                detail += 1;
                Ok(())
            },
            |_, _, _| panic!("must not emit an out-of-range daily summary"),
        )
        .unwrap();
        assert_eq!(snapshot, (revision + 1, missing_exact));
        assert_eq!(detail, expected_events);
    }

    let scoped = std::collections::HashSet::from([PARENT.to_owned()]);
    let query = crate::usage_history::Query::parse(
        &json!({"from":archived.to_string(),"to":retained.to_string()}),
        retained,
    )
    .unwrap();
    let summaries = Store::read_selection(&mut reader, &query, None, Some(&scoped)).unwrap();
    assert_eq!(summaries.revision, revision + 1);
    assert_eq!(summaries.unresolved, 0);
    assert!(summaries.archived && summaries.needs_source_recovery);
    assert_eq!(summaries.days.len(), 2);
    assert_eq!(summaries.days[0]["usage"]["totalTokens"], 110);
    assert_eq!(summaries.days[1]["usage"]["totalTokens"], 220);
    let exact = Store::read_selection(
        &mut reader,
        &query,
        Some((at(archived, 12, 0).timestamp_millis(), end)),
        Some(&scoped),
    )
    .unwrap();
    assert_eq!(exact.revision, summaries.revision);
    assert!(!exact.archived && exact.needs_source_recovery);
    assert_eq!(exact.days, summaries.days[1..]);
    let excluded = crate::usage_history::Query::parse(
        &json!({"sessionId":OTHER,"model":"missing","search":"missing"}),
        retained,
    )
    .unwrap();
    assert!(
        Store::read_selection(&mut reader, &excluded, None, Some(&scoped))
            .unwrap()
            .days
            .is_empty()
    );
}

#[test]
fn reopened_scope_uses_committed_aliases_and_only_unambiguous_descendants() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let db = root.join("tokens.sqlite3");
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let unlinked = "00000000-0000-4000-8000-000000000080";
    let child_path = root.join("rollout-child.jsonl");
    let mut store = Store::open(Connection::open(&db).unwrap()).unwrap();
    for (name, session, parent, second) in [
        (
            format!("rollout-parent-{PHYSICAL_PREFIX}.jsonl"),
            PARENT,
            None,
            1,
        ),
        (
            "rollout-child.jsonl".into(),
            CHILD,
            Some(PHYSICAL_PREFIX),
            2,
        ),
        ("rollout-descendant.jsonl".into(), OTHER, Some(CHILD), 3),
        ("rollout-unlinked.jsonl".into(), unlinked, None, 4),
    ] {
        let path = root.join(name);
        let mut meta = header(session, 0);
        if let Some(parent) = parent {
            meta["payload"]["source"] =
                json!({"subagent":{"thread_spawn":{"parent_thread_id":parent}}});
        }
        let state = parse(&path, &[meta, count(1, second, 1, true)], cutoff);
        let prepared = store
            .prepare(new_source(&path, &state), cutoff, BTreeMap::new(), 1 << 16)
            .unwrap();
        store.publish(prepared).unwrap();
    }
    drop(store);
    let query = Query::parse(&json!({}), cutoff).unwrap();
    let mut reader = Connection::open_with_flags(&db, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let (all, scoped) =
        Store::read_scoped_selection(&mut reader, &query, None, PHYSICAL_PREFIX).unwrap();
    assert_eq!(scoped.session_id, PARENT);
    assert_eq!(scoped.session_ids, [PARENT, CHILD, OTHER]);
    assert_eq!(all.revision, scoped.usage.revision);
    assert_eq!(all.days[0]["recordCount"], 4);
    assert_eq!(scoped.usage.days[0]["recordCount"], 3);

    let mut writer = Store::open(Connection::open(&db).unwrap()).unwrap();
    let (revision, _) = writer.checkpoint(&child_path).unwrap().unwrap();
    let mut replaced_header = header(CHILD, 0);
    replaced_header["payload"]["source"] =
        json!({"subagent":{"thread_spawn":{"parent_thread_id":OTHER}}});
    let replaced = parse(
        &child_path,
        &[replaced_header, count(1, 2, 1, true)],
        cutoff,
    );
    let prepared = writer
        .prepare(
            SourceUpdate {
                source: &child_path,
                state: &replaced,
                previous_revision: Some(revision),
                replace: true,
            },
            cutoff,
            BTreeMap::new(),
            1 << 16,
        )
        .unwrap();
    let revision = writer.publish(prepared).unwrap();
    let (all, scoped) =
        Store::read_scoped_selection(&mut reader, &query, None, PHYSICAL_PREFIX).unwrap();
    assert_eq!(all.revision, revision);
    assert_eq!(all.days[0]["recordCount"], 4);
    assert_eq!(scoped.session_ids, [PARENT]);
    assert_eq!(scoped.usage.days[0]["recordCount"], 1);
}

#[test]
fn requested_session_cursors_follow_the_same_committed_batch_after_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let path = root.join("rollout-parent.jsonl");
    let database = root.join("tokens.sqlite");
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let names = BTreeMap::new();
    parse(&path, &[header(PARENT, 0), count(1, 1, 1, true)], cutoff);
    let mut store = Store::open(Connection::open(&database).unwrap()).unwrap();
    let mut source = SourceReader::restore(&store, &path).unwrap();
    assert!(
        source
            .step(
                &mut store,
                cutoff,
                &mut Budget::new(std::iter::empty()),
                &names,
                8192
            )
            .unwrap()
    );
    let mut reader =
        Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let query = Query::parse(&json!({"includeSessionCursors":true}), cutoff).unwrap();
    let first = Store::read_selection(&mut reader, &query, None, None).unwrap();
    let first_cursor = &first.session_cursors.as_ref().unwrap()[0];
    assert_eq!(first_cursor["sessionId"], PARENT);
    assert_eq!(first_cursor["total"]["totalTokens"], 110);

    writeln!(
        fs::OpenOptions::new().append(true).open(&path).unwrap(),
        "{}",
        count(2, 2, 2, true)
    )
    .unwrap();
    store
        .db
        .execute_batch(
            "CREATE TRIGGER reject_cursor_batch BEFORE INSERT ON token_days
         BEGIN SELECT RAISE(ABORT, 'cursor batch write failed'); END;",
        )
        .unwrap();
    let retry = source.step(
        &mut store,
        cutoff,
        &mut Budget::new(std::iter::empty()),
        &names,
        8192,
    );
    assert!(
        retry
            .unwrap_err()
            .to_string()
            .contains("cursor batch write failed")
    );
    let failed = Store::read_selection(&mut reader, &query, None, None).unwrap();
    assert_eq!(failed.revision, first.revision);
    assert_eq!(failed.session_cursors, first.session_cursors);
    store
        .db
        .execute_batch("DROP TRIGGER reject_cursor_batch")
        .unwrap();
    assert!(
        source
            .step(
                &mut store,
                cutoff,
                &mut Budget::new(std::iter::empty()),
                &names,
                8192
            )
            .unwrap()
    );
    drop(store);
    let mut reader =
        Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let (all, scoped) = Store::read_scoped_selection(&mut reader, &query, None, PARENT).unwrap();
    assert!(all.revision > first.revision);
    assert_eq!(all.revision, scoped.usage.revision);
    assert_eq!(all.days[0]["recordCount"], 2);
    assert_eq!(all.session_cursors, scoped.usage.session_cursors);
    assert_eq!(all.session_cursors.unwrap()[0]["total"]["totalTokens"], 220);
    let plain = Query::parse(&json!({}), cutoff).unwrap();
    assert!(
        Store::read_selection(&mut reader, &plain, None, None)
            .unwrap()
            .session_cursors
            .is_none()
    );
}

#[test]
fn scoped_turn_selection_uses_retained_events_without_returning_archived_day_totals() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let database = root.join("tokens.sqlite3");
    let start = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 4).unwrap();
    let retained = cutoff.succ_opt().unwrap();
    let mut store = Store::open(Connection::open(&database).unwrap()).unwrap();
    for (session, turn, include_archive) in [(PARENT, "selected", true), (OTHER, "other", false)] {
        let path = root.join(format!("rollout-{session}.jsonl"));
        let mut rows = vec![
            header(session, 0),
            json!({"type":"turn_context","ordinal":1,
                "timestamp":"2026-01-02T12:00:01Z",
                "payload":{"turn_id":turn,"model":"gpt-fixture"}}),
        ];
        if include_archive {
            rows.push(count(2, 2, 1, true));
        }
        let mut recent = count(3, 3, if include_archive { 2 } else { 1 }, true);
        recent["timestamp"] = json!(format!("{retained}T12:00:03Z"));
        rows.push(recent);
        let state = parse(&path, &rows, start);
        let prepared = store
            .prepare(new_source(&path, &state), start, BTreeMap::new(), 1 << 16)
            .unwrap();
        store.publish(prepared).unwrap();
    }
    let revision = store.advance_retention(cutoff).unwrap().unwrap();
    drop(store);
    let mut reader =
        Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let query = Query::parse(&json!({"turnId":"selected"}), retained).unwrap();
    let midnight = |day: NaiveDate| {
        Local
            .from_local_datetime(&day.and_hms_opt(0, 0, 0).unwrap())
            .single()
            .unwrap()
            .timestamp_millis()
    };
    let bounds = Some((midnight(start), midnight(retained.succ_opt().unwrap())));
    for bounds in [None, bounds] {
        let (all, scoped) =
            Store::read_scoped_selection(&mut reader, &query, bounds, PARENT).unwrap();
        assert_eq!(all.revision, revision);
        assert_eq!(scoped.usage.revision, revision);
        assert_eq!(all.days, scoped.usage.days);
        assert_eq!(all.days.len(), 1);
        assert_eq!(all.days[0]["recordCount"], 1);
        assert_eq!(all.days[0]["usage"]["totalTokens"], 110);
        assert!(!all.archived);
        assert!(all.needs_source_recovery);
    }
    let query = Query::parse(
        &json!({"period":"today","turnId":"selected","sessionId":PARENT}),
        retained,
    )
    .unwrap();
    let current = Store::read_selection(&mut reader, &query, None, None).unwrap();
    assert_eq!(current.days[0]["recordCount"], 1);
    assert!(!current.needs_source_recovery);
}

#[test]
fn selection_preserves_the_known_priceable_subset_across_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let mut store = Store::open(Connection::open_in_memory().unwrap()).unwrap();
    for (session, cache_known) in [(PARENT, true), (CHILD, false)] {
        let path = root.join(format!("rollout-{session}.jsonl"));
        let mut item = count(1, 1, 1, true);
        if !cache_known {
            for key in ["total_token_usage", "last_token_usage"] {
                item["payload"]["info"][key]
                    .as_object_mut()
                    .unwrap()
                    .remove("cached_input_tokens");
            }
        }
        let state = parse(&path, &[header(session, 0), item], cutoff);
        let prepared = store
            .prepare(new_source(&path, &state), cutoff, BTreeMap::new(), 8192)
            .unwrap();
        store.publish(prepared).unwrap();
    }
    let query = Query::parse(&json!({}), cutoff).unwrap();
    let result = Store::read_selection(&mut store.db, &query, None, None).unwrap();
    assert_eq!(result.days.len(), 1);
    let day = &result.days[0];
    assert_eq!(day["recordCount"], 2);
    assert_eq!(day["usage"]["totalTokens"], 220);
    assert_eq!(day["usage"]["cachedKnown"], false);
    assert_eq!(day["breakdowns"].as_array().unwrap().len(), 1);
    for row in [day, &day["models"][0], &day["breakdowns"][0]] {
        assert_eq!(row["priceableUsage"]["inputTokens"], 100);
        assert_eq!(row["priceableUsage"]["cachedInputTokens"], 20);
        assert_eq!(row["priceableUsage"]["outputTokens"], 10);
    }
}

#[test]
fn archived_classification_matches_live_rollups_without_promoting_mixed_evidence() {
    use crate::usage_history::structured::{NativeResponseIdentity, REQUEST_SUPPLIER_PROOF, Snapshot};

    const RECOVERED: &str = "00000000-0000-4000-8000-000000000075";

    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let database = root.join("tokens.sqlite3");
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let archive = cutoff + chrono::Days::new(4);
    let mut store = Store::open(Connection::open(&database).unwrap()).unwrap();
    for (session, provider, evidence) in [
        (PARENT, "supplier-a", Some("request-log-exact")),
        (CHILD, "supplier-a", None),
        (
            OTHER,
            "supplier-b",
            Some("upstream-counter-bidirectional-supplier-only"),
        ),
        (RECOVERED, "supplier-c", Some(REQUEST_SUPPLIER_PROOF)),
    ] {
        let path = root.join(format!("rollout-{session}.jsonl"));
        let mut state = parse(&path, &[header(session, 0), count(1, 1, 1, true)], cutoff);
        state.events[0].provider = provider.to_owned();
        state.events[0].provider_source = evidence.map(str::to_owned);
        if session == RECOVERED {
            state.events[0].native_response_id = Some(NativeResponseIdentity {
                id: "resp_recovered_supplier".into(),
                full_counts: false,
            });
        }
        let prepared = store
            .prepare(new_source(&path, &state), cutoff, BTreeMap::new(), 8192)
            .unwrap();
        store.publish(prepared).unwrap();
    }
    let snapshot = Snapshot {
        events: read(&mut store.db, i64::MIN, i64::MAX).1,
        count_cache_available: true,
        ..Default::default()
    };
    let recovered = snapshot
        .events
        .iter()
        .find(|event| event.session == RECOVERED)
        .unwrap();
    let proof = recovered.native_response_id.as_ref().unwrap();
    assert_eq!(&*proof.id, "resp_recovered_supplier");
    assert!(!proof.full_counts);
    store.advance_retention(archive).unwrap();
    assert_eq!(store.reclaim_expired_details(10).unwrap(), 4);
    drop(store);
    let mut reader =
        Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    for session in [None, Some(PARENT), Some(OTHER), Some(RECOVERED)] {
        let query = Query::parse(&json!({"sessionId":session}), archive).unwrap();
        let live = snapshot.usage_days(&query, None, None);
        let archived = Store::read_selection(&mut reader, &query, None, None).unwrap();
        assert!(archived.archived);
        for (left, right) in live.iter().zip(&archived.days) {
            assert_eq!(left["usage"], right["usage"]);
            assert_eq!(left["recordCount"], right["recordCount"]);
            let live_rows = left["breakdowns"].as_array().unwrap();
            let archived_rows = right["breakdowns"].as_array().unwrap();
            assert_eq!(live_rows.len(), archived_rows.len());
            for (live_row, archive_row) in live_rows.iter().zip(archived_rows) {
                for field in [
                    "provider",
                    "usage",
                    "classificationEvidence",
                    "supplierClassificationEvidence",
                ] {
                    assert_eq!(live_row[field], archive_row[field]);
                }
            }
        }
        let first = &archived.days[0]["breakdowns"][0];
        assert_eq!(
            first["classificationEvidence"],
            if session == Some(PARENT) {
                json!("request-log-exact")
            } else {
                Value::Null
            }
        );
        if session == Some(OTHER) {
            assert_eq!(
                first["supplierClassificationEvidence"],
                "upstream-counter-bidirectional-supplier-only"
            );
        }
        if session == Some(RECOVERED) {
            assert_eq!(
                first["supplierClassificationEvidence"],
                REQUEST_SUPPLIER_PROOF
            );
        }
    }
}

#[test]
fn attribution_still_waits_for_count_gaps_in_related_or_unidentified_sources() {
    use super::scan::Scan;

    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    for (owner, alias, unparsed) in [
        (PARENT, false, false),
        (OTHER, true, false),
        (OTHER, false, true),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let sessions = root.join("sessions");
        fs::create_dir(&sessions).unwrap();
        let parent = sessions.join(format!("rollout-parent-{PHYSICAL_PREFIX}.jsonl"));
        let parent_state = parse(&parent, &[header(PARENT, 0), count(1, 1, 1, true)], cutoff);
        let gap = sessions.join("rollout-gap.jsonl");
        let mut meta = header(owner, if alias { 2 } else { 0 });
        if alias {
            meta["payload"]["history_base"] = json!({
                "thread_id":PHYSICAL_PREFIX,"end_ordinal_exclusive":2,
                "end_byte_offset":parent_state.offset
            });
        }
        let mut malformed = count(if alias { 3 } else { 1 }, 3, 2, true);
        malformed["payload"]["info"]["total_token_usage"]["input_tokens"] = json!("invalid");
        if unparsed {
            fs::write(&gap, b"{not a JSON envelope}\n").unwrap();
        } else {
            parse(&gap, &[meta, malformed], cutoff);
        }
        let mut store = Store::open(Connection::open_in_memory().unwrap()).unwrap();
        let mut scan = Scan::default();
        let report = scan.round(&mut store, &root, cutoff, &BTreeMap::new(), 8192);
        assert_eq!(report.pending_sources, 0);
        assert_eq!(report.issues, 1);
        assert_eq!(scan.scoped_count_gaps().contains(&gap), !unparsed);
        assert_eq!(
            store
                .reclassify_session(PARENT, 8192, scan.scoped_count_gaps(), |_, _| {
                    panic!("unresolved source was admitted: {owner}, alias={alias}, unparsed={unparsed}")
                })
                .unwrap(),
            None
        );
    }
}

#[test]
fn late_gateway_evidence_reclassifies_committed_rows_and_days_atomically() {
    use super::scan::Scan;
    use crate::usage_history::structured::{RequestEvidence, RequestEvidenceCache, provider_cache};

    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let sessions = root.join("sessions");
    fs::create_dir(&sessions).unwrap();
    let source = sessions.join("rollout-parent.jsonl");
    let database = root.join("tokens.sqlite3");
    let now = chrono::Utc::now();
    let cutoff = now.with_timezone(&Local).date_naive();
    let mut meta = header(PARENT, 0);
    meta["timestamp"] = json!(now.to_rfc3339());
    meta["payload"]["model_provider"] = json!("custom");
    let turn = json!({"type":"turn_context","ordinal":1,"timestamp":now.to_rfc3339(),
        "payload":{"turn_id":"turn-one","model":"gpt-fixture"}});
    let mut usage = count(2, 2, 1, true);
    usage["timestamp"] = json!(now.to_rfc3339());
    parse(&source, &[meta, turn, usage], cutoff);
    let mut store = Store::open(Connection::open(&database).unwrap()).unwrap();
    assert!(
        Scan::default()
            .round(&mut store, &root, cutoff, &BTreeMap::new(), 8192)
            .complete
    );

    let bounds = (now.timestamp_millis() - 1, now.timestamp_millis() + 1);
    let mut reader =
        Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let (_, before) = read(&mut reader, bounds.0, bounds.1);
    assert_eq!(before.len(), 1);
    let checkpoint = store.checkpoint(&source).unwrap().unwrap();
    let original = before[0].provider.clone();
    let provider_dir = root.join("evidence");
    let mut proof_db = provider_cache::ProviderCache::default();
    proof_db.open(&provider_dir, cutoff).unwrap();
    let request: RequestEvidence = serde_json::from_value(json!({
        "id":"test-request","threadId":PARENT,"model":"gpt-fixture",
        "upstreamModel":"gpt-fixture","relayId":"relay-proof","relayName":"Relay Proof",
        "success":true,"state":"success","statusCode":200,"timestampMs":now.timestamp_millis(),
        "startedAtMs":now.timestamp_millis()-100,"attempts":1,"autoRetries":0,
        "usage":{"inputTokens":100,"cachedTokens":20,"outputTokens":10}
    }))
    .unwrap();
    let mut evidence = RequestEvidenceCache {
        records: vec![request.clone()],
        ..Default::default()
    };
    let mut classify = |events: &mut [UsageEvent], aliases: &BTreeMap<String, String>| {
        evidence.attribute_with_cache(events, &mut BTreeMap::new(), aliases, &proof_db);
        Ok(())
    };
    assert!(
        store
            .reclassify_session(PARENT, 1, &BTreeSet::new(), &mut classify)
            .is_err()
    );
    store
        .db
        .execute_batch(
            "CREATE TRIGGER abort_classification BEFORE UPDATE ON token_days
        BEGIN SELECT RAISE(ABORT, 'classification test write failure'); END;",
        )
        .unwrap();
    let error = store
        .reclassify_session(PARENT, 8192, &BTreeSet::new(), &mut classify)
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("classification test write failure")
    );
    assert_eq!(
        read(&mut reader, bounds.0, bounds.1).1[0].provider,
        original
    );
    store
        .db
        .execute_batch("DROP TRIGGER abort_classification")
        .unwrap();

    let revision = store
        .reclassify_session(PARENT, 8192, &BTreeSet::new(), &mut classify)
        .unwrap()
        .unwrap();
    let query = Query::parse(&json!({"period":"today"}), cutoff).unwrap();
    let view = Store::read_selection(&mut reader, &query, None, None).unwrap();
    assert_eq!(view.revision, revision);
    assert_eq!(view.days[0]["breakdowns"][0]["provider"], "relay-proof");
    assert_eq!(
        view.days[0]["breakdowns"][0]["classificationEvidence"],
        "request-log-exact"
    );
    assert_eq!(view.days[0]["usage"]["totalTokens"], 110);
    let latest = read(&mut reader, bounds.0, bounds.1).1.pop().unwrap();
    assert_eq!(latest.id, before[0].id);
    assert_eq!(latest.timestamp, before[0].timestamp);
    assert_eq!(latest.total.value(), before[0].total.value());
    assert_eq!(latest.delta.value(), before[0].delta.value());

    proof_db.save(&evidence.verified, &BTreeMap::new()).unwrap();
    drop(store);
    drop(proof_db);
    let mut store = Store::open(Connection::open(&database).unwrap()).unwrap();
    let mut proof_db = provider_cache::ProviderCache::default();
    proof_db.open(&provider_dir, cutoff).unwrap();
    let mut evidence = RequestEvidenceCache::default();
    evidence.restore_verified(proof_db.load_verified(cutoff).unwrap());
    let mut classify = |events: &mut [UsageEvent], aliases: &BTreeMap<String, String>| {
        evidence.attribute_with_cache(events, &mut BTreeMap::new(), aliases, &proof_db);
        Ok(())
    };
    assert!(
        store
            .reclassify_session(PARENT, 8192, &BTreeSet::new(), &mut classify)
            .unwrap()
            .is_none()
    );

    let mut conflict = request.clone();
    conflict.id = "other-request".into();
    conflict.relay_id = "unverified-relay".into();
    evidence.records = vec![request, conflict];
    let next = store
        .reclassify_session(PARENT, 8192, &BTreeSet::new(), |events, aliases| {
            evidence.attribute_with_cache(events, &mut BTreeMap::new(), aliases, &proof_db);
            Ok(())
        })
        .unwrap()
        .unwrap();
    assert!(next > revision);
    let view = Store::read_selection(&mut reader, &query, None, None).unwrap();
    assert_eq!(view.days[0]["breakdowns"][0]["provider"], original);
    assert!(
        view.days[0]["breakdowns"][0]
            .get("classificationEvidence")
            .is_none()
    );
    assert_eq!(view.days[0]["recordCount"], 1);
    assert_eq!(view.days[0]["usage"]["totalTokens"], 110);
    let after = store.checkpoint(&source).unwrap().unwrap();
    assert_eq!(after.0, checkpoint.0);
    assert_eq!(after.1.offset, checkpoint.1.offset);
}

#[test]
fn append_retracts_only_nearby_proofs_in_the_same_transaction_as_the_new_count() {
    use super::scan::Scan;

    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let sessions = root.join("sessions");
    fs::create_dir(&sessions).unwrap();
    let source = sessions.join("rollout-parent.jsonl");
    let mut store = Store::open(Connection::open_in_memory().unwrap()).unwrap();
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let names = BTreeMap::new();
    let point = DateTime::parse_from_rfc3339("2026-01-02T12:00:00Z").unwrap();
    let counts_at = |n, seconds| {
        let mut row = count(n, n, n as u64, true);
        row["timestamp"] = json!((point + chrono::TimeDelta::seconds(seconds)).to_rfc3339());
        row
    };
    parse(&source, &[header(PARENT, 0), counts_at(1, 1)], cutoff);
    let mut scan = Scan::default();
    assert!(scan.round(&mut store, &root, cutoff, &names, 8192).complete);
    let proof = |events: &mut [UsageEvent], _: &BTreeMap<String, String>| {
        for event in events {
            event.provider = "relay-proof".into();
            event.provider_source = Some("request-log-exact".into());
        }
        Ok(())
    };
    store
        .reclassify_session(PARENT, 8192, &BTreeSet::new(), proof)
        .unwrap()
        .unwrap();
    writeln!(
        fs::OpenOptions::new().append(true).open(&source).unwrap(),
        "{}",
        counts_at(2, 600)
    )
    .unwrap();
    assert!(scan.round(&mut store, &root, cutoff, &names, 8192).complete);
    let (_, distant) = read(&mut store.db, i64::MIN, i64::MAX);
    assert_eq!(distant.len(), 2);
    assert_eq!(distant[0].provider, "relay-proof");
    assert_ne!(distant[1].provider, "relay-proof");
    store
        .reclassify_session(PARENT, 8192, &BTreeSet::new(), proof)
        .unwrap()
        .unwrap();

    writeln!(
        fs::OpenOptions::new().append(true).open(&source).unwrap(),
        "{}",
        counts_at(3, 605)
    )
    .unwrap();
    store
        .db
        .execute_batch(
            "CREATE TRIGGER abort_append BEFORE INSERT ON token_days
         BEGIN SELECT RAISE(ABORT, 'append test write failure'); END;",
        )
        .unwrap();
    let failed = scan.round(&mut store, &root, cutoff, &names, 8192);
    assert!(!failed.complete);
    assert!(
        failed.errors[0]
            .1
            .to_string()
            .contains("append test write failure")
    );
    let (_, unchanged) = read(&mut store.db, i64::MIN, i64::MAX);
    assert_eq!(unchanged.len(), 2);
    assert_eq!(unchanged[1].provider, "relay-proof");
    store.db.execute_batch("DROP TRIGGER abort_append").unwrap();
    assert!(scan.round(&mut store, &root, cutoff, &names, 8192).complete);
    let (_, updated) = read(&mut store.db, i64::MIN, i64::MAX);
    assert_eq!(updated.len(), 3);
    assert_eq!(updated[0].provider, "relay-proof");
    assert_eq!(
        updated[0].provider_source.as_deref(),
        Some("request-log-exact")
    );
    assert_eq!(updated[1].provider, distant[1].provider);
    assert_eq!(updated[2].provider, distant[1].provider);
    assert_eq!(
        updated
            .iter()
            .map(|event| event.delta.total_tokens)
            .sum::<u64>(),
        330
    );
    let query = Query::parse(&json!({}), cutoff).unwrap();
    let day = &Store::read_selection(&mut store.db, &query, None, None)
        .unwrap()
        .days[0];
    assert_eq!(day["recordCount"], 3);
    assert_eq!(day["usage"]["totalTokens"], 330);
    let groups = day["breakdowns"].as_array().unwrap();
    assert_eq!(groups.len(), 2);
    let exact = groups
        .iter()
        .find(|row| row["provider"] == "relay-proof")
        .unwrap();
    assert_eq!(exact["recordCount"], 1);
}

#[test]
fn replacing_recent_details_keeps_older_daily_totals_and_unknown_cache() {
    let dir = tempfile::tempdir().unwrap();
    let source = fs::canonicalize(dir.path())
        .unwrap()
        .join("rollout-a.jsonl");
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let mut usage = count(1, 1, 1, true);
    for key in ["total_token_usage", "last_token_usage"] {
        usage["payload"]["info"][key]
            .as_object_mut()
            .unwrap()
            .remove("cached_input_tokens");
    }
    let original = parse(&source, &[header(PARENT, 0), usage], cutoff);
    let day = original.events[0].date;
    let end = day.succ_opt().unwrap();
    let mut store = Store::open(Connection::open_in_memory().unwrap()).unwrap();
    let first = store
        .prepare(
            new_source(&source, &original),
            cutoff,
            BTreeMap::new(),
            8192,
        )
        .unwrap();
    let first_revision = store.publish(first).unwrap();
    store
        .db
        .execute_batch(
            "CREATE TRIGGER fail_archived_detail_delete BEFORE DELETE ON token_resolved
             BEGIN SELECT RAISE(ABORT, 'archived detail must be reclaimed separately'); END;",
        )
        .unwrap();
    let replacement = parse(&source, &[header(PARENT, 0)], end);
    let second = store
        .prepare(
            SourceUpdate {
                source: &source,
                state: &replacement,
                previous_revision: Some(first_revision),
                replace: true,
            },
            end,
            BTreeMap::new(),
            8192,
        )
        .unwrap();
    let latest = store.publish(second).unwrap();
    assert!(read(&mut store.db, i64::MIN, i64::MAX).1.is_empty());
    let mut visited = false;
    let summary_rev = Store::read_days(&mut store.db, day, end, |session, date, groups| {
        assert_eq!(session, PARENT);
        assert_eq!(date, day);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].totals.records, 1);
        assert_eq!(groups[0].totals.counts.0.total_tokens, 110);
        assert!(!groups[0].totals.counts.0.cached_known);
        assert_eq!(groups[0].totals.priceable, None);
        visited = true;
        Ok(())
    })
    .unwrap();
    assert!(visited);
    assert_eq!(summary_rev, latest);
    store
        .db
        .execute_batch("DROP TRIGGER fail_archived_detail_delete")
        .unwrap();
    assert_eq!(store.reclaim_expired_details(1).unwrap(), 1);
    assert_eq!(store.reclaim_expired_details(1).unwrap(), 0);
}

#[test]
fn daily_pricing_subset_survives_append_reopen_and_detail_reclamation() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let source = root.join("rollout-a.jsonl");
    let db = root.join("tokens.sqlite3");
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let known = count(1, 1, 1, true);
    let mut unknown = count(2, 2, 2, true);
    let mut latest = count(3, 3, 3, true);
    for key in ["total_token_usage", "last_token_usage"] {
        unknown["payload"]["info"][key]
            .as_object_mut()
            .unwrap()
            .remove("cached_input_tokens");
    }
    latest["payload"]["info"]["last_token_usage"]["cached_input_tokens"] = json!(0);
    latest["payload"]["info"]["total_token_usage"]["cached_input_tokens"] = json!(40);
    let original = parse(&source, &[header(PARENT, 0), known], cutoff);
    let day = original.events[0].date;
    let end = day.succ_opt().unwrap();
    let inspect = |store: &mut Store, records, total, priceable| {
        let mut visited = false;
        Store::read_days(&mut store.db, day, end, |session, _, groups| {
            assert_eq!(session, PARENT);
            assert_eq!(groups.len(), 1);
            assert_eq!(groups[0].totals.records, records);
            assert_eq!(groups[0].totals.counts.0.total_tokens, total);
            assert_eq!(groups[0].totals.counts.0.cached_known, records == 1);
            assert_eq!(groups[0].totals.priceable, priceable);
            visited = true;
            Ok(())
        })
        .unwrap();
        assert!(visited);
    };
    let mut store = Store::open(Connection::open(&db).unwrap()).unwrap();
    let first = store
        .prepare(
            new_source(&source, &original),
            cutoff,
            BTreeMap::new(),
            8192,
        )
        .unwrap();
    store.publish(first).unwrap();
    inspect(&mut store, 1, 110, None);

    for (line, records, priceable) in [(unknown, 2, [100, 20, 10]), (latest, 3, [200, 20, 20])] {
        drop(store);
        writeln!(
            fs::OpenOptions::new().append(true).open(&source).unwrap(),
            "{line}"
        )
        .unwrap();
        store = Store::open(Connection::open(&db).unwrap()).unwrap();
        let (revision, mut state) = store.checkpoint(&source).unwrap().unwrap();
        assert!(
            state
                .read_with_budget(&source, cutoff, &mut Budget::new(std::iter::empty()))
                .unwrap()
        );
        let staged = store
            .prepare(
                SourceUpdate {
                    source: &source,
                    state: &state,
                    previous_revision: Some(revision),
                    replace: false,
                },
                cutoff,
                BTreeMap::new(),
                8192,
            )
            .unwrap();
        let committed = store.publish(staged).unwrap();
        let linear: i64 = store
            .db
            .query_row("SELECT revision FROM token_linear_sources", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(
            linear, committed,
            "append must retain its linear source certificate"
        );
        inspect(&mut store, records, records * 110, Some(priceable));
    }

    let next_source = root.join("rollout-next-day.jsonl");
    let next_day = json!({"type":"event_msg","ordinal":1,"timestamp":"2026-01-04T12:00:01Z",
        "payload":count(1, 1, 1, true)["payload"]});
    let next = parse(&next_source, &[header(OTHER, 0), next_day], end);
    let staged = store
        .prepare(new_source(&next_source, &next), end, BTreeMap::new(), 8192)
        .unwrap();
    store.publish(staged).unwrap();
    assert_eq!(store.reclaim_expired_details(2).unwrap(), 2);
    drop(store);
    let mut store = Store::open(Connection::open(&db).unwrap()).unwrap();
    assert_eq!(store.reclaim_expired_details(2).unwrap(), 1);
    assert_eq!(store.reclaim_expired_details(2).unwrap(), 0);
    inspect(&mut store, 3, 330, Some([200, 20, 20]));
}

#[test]
fn a_distinct_turn_collision_keeps_its_identity_after_source_replacement() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let a = root.join("rollout-a.jsonl");
    let b = root.join("rollout-b.jsonl");
    let turn = |id| {
        json!({"type":"turn_context","ordinal":1,
        "timestamp":"2026-01-02T12:00:01Z","payload":{"turn_id":id,"model":"gpt-fixture"}})
    };
    let first = parse(
        &a,
        &[header(PARENT, 0), turn("turn-a"), count(2, 2, 1, true)],
        cutoff,
    );
    let second = parse(
        &b,
        &[header(PARENT, 0), turn("turn-b"), count(2, 2, 1, true)],
        cutoff,
    );
    assert_eq!(first.events[0].original_id, second.events[0].original_id);
    let file = root.join("tokens.sqlite3");
    let mut store = Store::open(Connection::open(&file).unwrap()).unwrap();
    let mut reader = Connection::open_with_flags(&file, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    for (path, state) in [(&a, &first), (&b, &second)] {
        let next = store
            .prepare(new_source(path, state), cutoff, BTreeMap::new(), 8192)
            .unwrap();
        store.publish(next).unwrap();
    }
    let (_, before) = read(&mut reader, i64::MIN, i64::MAX);
    assert_eq!(before.len(), 2);
    assert_ne!(before[0].id, before[1].id);
    let turn_a = before.iter().find(|event| event.turn == "turn-a").unwrap();
    let (b_revision, _) = store.checkpoint(&b).unwrap().unwrap();
    let cleared = parse(&b, &[header(PARENT, 0)], cutoff);
    drop(store);

    let mut reopened = Store::open(Connection::open(&file).unwrap()).unwrap();
    let update = SourceUpdate {
        source: &b,
        state: &cleared,
        previous_revision: Some(b_revision),
        replace: true,
    };
    let next = reopened
        .prepare(update, cutoff, BTreeMap::new(), 8192)
        .unwrap();
    reopened.publish(next).unwrap();
    let (_, after) = read(&mut reader, i64::MIN, i64::MAX);
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].id, turn_a.id);
    assert_eq!(after[0].delta, turn_a.delta);
}
