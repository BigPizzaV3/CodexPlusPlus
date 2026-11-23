use super::store::{Store, projection::SourceUpdate};
use crate::usage_history::structured::{FileState, UsageEvent, index_budget::Budget};
use chrono::{DateTime, NaiveDate};
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

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
            "CREATE TEMP TRIGGER fail_token_projection BEFORE INSERT ON token_resolved
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
