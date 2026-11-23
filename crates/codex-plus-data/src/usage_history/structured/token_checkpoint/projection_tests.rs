use super::store::{Store, projection::SourceUpdate};
use crate::usage_history::structured::{FileState, UsageEvent, index_budget::Budget};
use chrono::{DateTime, NaiveDate};
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};
use std::collections::BTreeMap;
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
        }
        seen += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!((snapshot, seen), (first_revision, 50));
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
        visited = true;
        Ok(())
    })
    .unwrap();
    assert!(visited);
    assert_eq!(summary_rev, latest);
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
