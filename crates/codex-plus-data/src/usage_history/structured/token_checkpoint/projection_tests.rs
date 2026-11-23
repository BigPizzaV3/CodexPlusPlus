use super::store::{Store, projection::SourceUpdate};
use crate::usage_history::structured::{FileState, UsageEvent, index_budget::Budget};
use chrono::{DateTime, Local, NaiveDate, TimeZone};
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
fn copies_sharing_one_session_resolve_once_without_affecting_other_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let source = root.join("rollout-original.jsonl");
    let original = parse(&source, &[header(PARENT, 0), count(1, 1, 1, true)], cutoff);
    let other_path = root.join("rollout-other.jsonl");
    let other = parse(&other_path, &[header(OTHER, 0), count(1, 4, 1, true)], cutoff);
    let mut store = Store::open(Connection::open_in_memory().unwrap()).unwrap();
    let prepared = store
        .prepare(new_source(&other_path, &other), cutoff, BTreeMap::new(), 8192)
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
        .prepare(new_source(&source, &original), cutoff, BTreeMap::new(), 1 << 20)
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
    // 由无关空来源推进保留线，保留已提交的父来源日汇总。
    let other_path = root.join("rollout-other.jsonl");
    let other = parse(&other_path, &[header(OTHER, 0)], cutoff);
    let advance = store
        .prepare(
            new_source(&other_path, &other),
            cutoff,
            BTreeMap::new(),
            8192,
        )
        .unwrap();
    let before = store.publish(advance).unwrap();
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
    assert_eq!(snapshot, (revision, false));
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
