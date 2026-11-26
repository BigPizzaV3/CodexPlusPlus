use super::ingest::SourceReader;
use super::store::{Coverage, Store, projection::SourceUpdate};
use crate::usage_history::Query;
use crate::usage_history::structured::{FileState, UsageEvent, index_budget::Budget};
use chrono::{DateTime, Local, NaiveDate, TimeZone};
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashSet};
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
fn historical_candidates_stream_all_rows_at_one_timestamp() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let source = dir.path().canonicalize()?.join(format!("rollout-{PARENT}.jsonl"));
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let rows: Vec<_> = std::iter::once(header(PARENT, 0))
        .chain((1..=33).map(|n| count(n, 1, n as u64, true)))
        .collect();
    let state = parse(&source, &rows, cutoff);
    let expected: BTreeSet<_> = state.events.iter().map(|event| event.id).collect();
    assert_eq!(expected.len(), 33);
    let mut store = Store::open(Connection::open_in_memory()?)?;
    let prepared = store.prepare(new_source(&source, &state), cutoff, BTreeMap::new(), 1 << 20)?;
    store.publish(prepared)?;
    let tx = store.db.transaction()?;
    // 每条记录单独可读，同一毫秒的合法 JSON 合计超过原先的 32 MiB。
    tx.execute(
        "UPDATE token_resolved SET payload = CAST(payload || ?1 AS BLOB)",
        [" ".repeat(1 << 20)],
    )?;
    let at = DateTime::parse_from_rfc3339(&state.events[0].timestamp)?.timestamp_millis();
    let mut visited = BTreeSet::new();
    Store::historical_candidates(&tx, &BTreeSet::from([at]), |event| {
        assert!(visited.insert(event.id));
    })?;
    assert_eq!(visited, expected);
    Ok(())
}

#[test]
fn reconciles_linked_rollouts_beyond_4096_sources_within_the_input_budget() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let root = dir.path().canonicalize()?;
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let prior_path = root.join("rollout-prior.jsonl");
    let new_path = root.join("rollout-new.jsonl");
    let prior = parse(
        &prior_path,
        &[header(PARENT, 0), count(1, 1, 1, true)],
        cutoff,
    );
    let incoming = parse(
        &new_path,
        &[header(PARENT, 0), count(1, 2, 2, true)],
        cutoff,
    );
    let mut store = Store::open(Connection::open_in_memory()?)?;
    let tx = store.db.transaction()?;
    let header_only = FileState {
        session: PARENT.to_owned(),
        ..FileState::default()
    };
    for index in 0..4096 {
        let path = root.join(format!("rollout-{index}.jsonl"));
        Store::stage(&tx, &path, None, &header_only, false)?;
    }
    let before = Store::stage(&tx, &prior_path, None, &prior, false)?;
    tx.commit()?;

    let error = store
        .prepare(
            new_source(&new_path, &incoming),
            cutoff,
            BTreeMap::new(),
            64 * 1024,
        )
        .err()
        .expect("the component must exhaust the low input budget");
    assert!(error.is::<super::store::projection::ComponentInputLimit>());
    assert!(store.checkpoint(&new_path)?.is_none());
    let (revision, events) = read(&mut store.db, i64::MIN, i64::MAX);
    assert_eq!(revision, before);
    assert!(events.is_empty());

    let prepared = store.prepare(
        new_source(&new_path, &incoming),
        cutoff,
        BTreeMap::new(),
        32 << 20,
    )?;
    assert_eq!(store.publish(prepared)?, before + 1);
    let (_, events) = read(&mut store.db, i64::MIN, i64::MAX);
    assert_eq!(
        events
            .iter()
            .map(|event| (event.id, event.total))
            .collect::<Vec<_>>(),
        [&prior.events[0], &incoming.events[0]].map(|event| (event.id, event.total))
    );
    Ok(())
}

#[test]
fn full_component_reads_clear_flags_once_and_keeps_the_stored_observations() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let root = dir.path().canonicalize()?;
    let first = root.join(format!("rollout-{PARENT}.jsonl"));
    let peer = root.join(format!("rollout-{OTHER}.jsonl"));
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let rows: Vec<_> = std::iter::once(header(PARENT, 0))
        .chain((1..=40).map(|n| count(n, n, n as u64, true)))
        .collect();
    let state = parse(&first, &rows, cutoff);
    let names = BTreeMap::new();
    let mut store = Store::open(Connection::open_in_memory()?)?;
    let prepared = store.prepare(new_source(&first, &state), cutoff, names.clone(), 1 << 20)?;
    store.publish(prepared)?;
    let state = parse(&peer, &[header(PARENT, 0), count(1, 41, 41, true)], cutoff);
    let raw_bytes: usize = store.db.query_row(
        "SELECT sum(length(payload)) FROM token_observations",
        [],
        |row| row.get(0),
    )?;
    assert!(
        store
            .prepare(new_source(&peer, &state), cutoff, names.clone(), 1024)
            .is_err()
    );
    let prepared = store.prepare(new_source(&peer, &state), cutoff, names.clone(), raw_bytes)?;
    drop(prepared);
    let explicit: usize = store.db.query_row(
        "SELECT count(*) FROM token_observations WHERE json_type(CAST(payload AS TEXT),'$.count_pending')='false'",
        [], |row| row.get(0)
    )?;
    assert_eq!(explicit, 40);
    store.db.execute(
        "UPDATE token_observations
         SET payload=CAST(json_set(CAST(payload AS TEXT),'$.unrecognized',1) AS BLOB)",
        [],
    )?;
    let error = store
        .prepare(new_source(&peer, &state), cutoff, names.clone(), 1 << 20)
        .err()
        .unwrap();
    assert!(error.to_string().contains("unknown field"));
    store.db.execute(
        "UPDATE token_observations
         SET payload=CAST(json_remove(CAST(payload AS TEXT),'$.unrecognized') || x'006a756e6b' AS BLOB)",
        [],
    )?;
    let error = store
        .prepare(new_source(&peer, &state), cutoff, names, 1 << 20)
        .err()
        .expect("NUL and trailing bytes must reach the strict observation decoder");
    assert!(error.to_string().contains("trailing characters"));
    Ok(())
}

#[test]
fn full_component_folds_only_identical_last_counts_under_input_limit() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let root = dir.path().canonicalize()?;
    let first = root.join(format!("rollout-{PARENT}.jsonl"));
    let peer = root.join(format!("rollout-{OTHER}.jsonl"));
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let rows: Vec<_> = std::iter::once(header(PARENT, 0))
        .chain((1..=120).map(|n| {
            let mut row = count(n, n % 60, n as u64, true);
            row["timestamp"] = json!(format!("2026-01-02T12:{:02}:{:02}Z", n / 60, n % 60));
            row
        }))
        .collect();
    let mut prior = parse(&first, &rows, cutoff);
    prior.events[1].last = None;
    prior.events[2].last.as_mut().unwrap().cached_known = false;
    prior.events[3].last.as_mut().unwrap().reasoning_output_tokens = Some(0);
    let mut store = Store::open(Connection::open_in_memory()?)?;
    let initial = store.prepare(new_source(&first, &prior), cutoff, BTreeMap::new(), 1 << 20)?;
    store.publish(initial)?;
    let limit = store.db.query_row(
        "SELECT sum(length(json_remove(CAST(payload AS TEXT), '$.count_uncertain',
         '$.count_incomplete', '$.count_pending', '$.inherited', '$.context_conflicts')))
         FROM token_observations", [], |row| row.get::<_, usize>(0),
    )?;
    let mut next = count(1, 1, 121, true);
    next["timestamp"] = json!("2026-01-02T12:02:01Z");
    let peer_state = parse(&peer, &[header(PARENT, 0), next], cutoff);
    let prepared = store.prepare(new_source(&peer, &peer_state), cutoff, BTreeMap::new(), limit)?;
    store.publish(prepared)?;
    let (_, resolved) = read(&mut store.db, i64::MIN, i64::MAX);
    assert_eq!(resolved.len(), 121);
    for (expected, observed) in prior.events.iter().zip(&resolved) {
        assert_eq!(observed.id, expected.id);
        assert_eq!(observed.last, expected.last);
    }
    // 重复字段也必须经过完整的反序列化检查。
    store.db.execute(
        "UPDATE token_observations SET payload=CAST('{\"last\":' ||
         json_extract(CAST(payload AS TEXT),'$.last') || ',' || substr(CAST(payload AS TEXT),2) AS BLOB)
         WHERE position=8", [],
    )?;
    let other = root.join("another.jsonl");
    let error = store.prepare(new_source(&other, &peer_state), cutoff, BTreeMap::new(), 1 << 20)
        .err().expect("duplicate last must be rejected");
    assert!(error.to_string().contains("duplicate field"), "{error}");
    Ok(())
}

#[test]
fn registered_compressed_prefix_can_introduce_another_sessions_counts_in_its_suffix()
-> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path().canonicalize()?;
    let archive = root.join(format!("rollout-{CHILD}.jsonl.zst"));
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let first = format!("{}\n{}\n", header(CHILD, 0), count(1, 1, 1, true));
    let mut fork = header(CHILD, 2);
    fork["payload"]["forked_from_id"] = json!(PARENT);
    fork["payload"]["subagent_history_start_ordinal"] = json!(5);
    let full = format!(
        "{first}{fork}\n{}\n{}\n",
        header(PARENT, 3),
        count(4, 4, 1, true)
    );
    fs::write(&archive, zstd::stream::encode_all(full.as_bytes(), 1)?)?;
    let mut store = Store::open(Connection::open_in_memory()?)?;
    let mut reader = SourceReader::restore(&store, &archive)?;
    let mut first_budget = Budget::new(std::iter::empty());
    first_budget.read_left = first.len() as u64 + 1;
    assert!(!reader.step(
        &mut store,
        cutoff,
        &mut first_budget,
        &BTreeMap::new(),
        1 << 20
    )?);
    let (_, first_checkpoint) = store.checkpoint(&archive)?.unwrap();
    assert!(first_checkpoint.resume_pending);
    assert_eq!(first_checkpoint.session, CHILD);
    assert_eq!(first_checkpoint.offset, first.len() as u64);
    assert_eq!(
        first_checkpoint.stamp,
        Some(FileState::source_stamp(&archive)?)
    );
    assert!(!first_checkpoint.identity_conflict && !first_checkpoint.invalid_counts);
    let link_count = |store: &Store| -> rusqlite::Result<i64> {
        store.db.query_row(
            "SELECT count(*) FROM token_source_links WHERE link=?",
            [format!("s:{PARENT}")],
            |row| row.get(0),
        )
    };
    assert_eq!(link_count(&store)?, 0);
    let queued = std::slice::from_ref(&archive);
    assert!(store.sealed_queued_sources(queued)?.0.is_empty());
    let mut reopened = SourceReader::restore(&store, &archive)?;
    assert!(reopened.step(
        &mut store,
        cutoff,
        &mut Budget::new(std::iter::empty()),
        &BTreeMap::new(),
        1 << 20
    )?);
    let (_, closed) = store.checkpoint(&archive)?.unwrap();
    assert!(!closed.resume_pending && !closed.identity_conflict && !closed.invalid_counts);
    assert_eq!(closed.session, CHILD);
    assert_eq!(closed.stamp, first_checkpoint.stamp);
    assert!(store.sealed_queued_sources(queued)?.0.contains_key(&archive));
    assert_eq!(link_count(&store)?, 1);
    assert!(store.db.query_row(
        "SELECT EXISTS(SELECT 1 FROM token_observations o JOIN token_sessions s ON s.id=o.session_id
         WHERE s.session=?1)",
        [PARENT], |row| row.get::<_, bool>(0)
    )?);
    Ok(())
}

#[test]
fn cross_session_parent_prefix_requires_a_unique_committed_native_anchor() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path().canonicalize()?;
    let parent = root.join(format!("rollout-{PHYSICAL_PREFIX}.jsonl"));
    let child = root.join(format!("rollout-{CHILD}.jsonl"));
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let parent_rows = std::iter::once(header(PARENT, 0))
        .chain((1..=48).map(|n| count(n, n, n as u64, true)))
        .collect::<Vec<_>>();
    let origin = parse(&parent, &parent_rows, cutoff);
    let offset = origin.events[46].rollout_position.unwrap().2;
    for case in [
        "unique",
        "cache alternate",
        "physical copy",
        "overlapping boundary",
        "graph rejoins",
    ] {
        let db = root.join(format!("{case}.sqlite"));
        let mut store = Store::open(Connection::open(&db)?)?;
        let ready = store.prepare(
            new_source(&parent, &origin),
            cutoff,
            BTreeMap::new(),
            1 << 20,
        )?;
        store.publish(ready)?;
        if matches!(case, "cache alternate" | "physical copy" | "graph rejoins") {
            let other = if case == "physical copy" {
                root.join(format!("copy-{PHYSICAL_PREFIX}.jsonl"))
            } else {
                root.join(format!("other-{OTHER}.jsonl"))
            };
            let mut counter = count(1, 47, 47, true);
            counter["payload"]["info"]["total_token_usage"]["cached_input_tokens"] = json!(2);
            let mut meta = header(PARENT, 0);
            if case == "graph rejoins" {
                meta = header(CHILD, 48);
                meta["payload"]["history_base"] = json!({"thread_id":PHYSICAL_PREFIX,
                    "end_ordinal_exclusive":48,"end_byte_offset":offset});
                counter = count(49, 50, 50, true);
            }
            let competing = parse(&other, &[meta, counter], cutoff);
            let ready = store.prepare(
                new_source(&other, &competing),
                cutoff,
                BTreeMap::new(),
                1 << 20,
            )?;
            store.publish(ready)?;
        }
        let mut meta = header(CHILD, 48);
        meta["payload"]["history_base"] = json!({"thread_id":PHYSICAL_PREFIX,
            "end_ordinal_exclusive":48,"end_byte_offset":if case=="overlapping boundary" {offset-1} else {offset}});
        let incoming = parse(&child, &[meta, count(49, 49, 49, false)], cutoff);
        let update = new_source(&child, &incoming);
        let small = store.prepare(update, cutoff, BTreeMap::new(), 12_000);
        if case == "unique" {
            let mut reference = Store::open(Connection::open_in_memory()?)?;
            let ready = reference.prepare(
                new_source(&parent, &origin),
                cutoff,
                BTreeMap::new(),
                1 << 20,
            )?;
            reference.publish(ready)?;
            let full = reference.prepare(
                SourceUpdate {
                    replace: true,
                    ..update
                },
                cutoff,
                BTreeMap::new(),
                1 << 20,
            )?;
            reference.publish(full)?;
            store.publish(small?)?;
            drop(store);
            let mut reopened = Store::open(Connection::open(&db)?)?;
            let actual = read(&mut reopened.db, i64::MIN, i64::MAX).1;
            assert_eq!(actual.len(), 49);
            assert!(actual == read(&mut reference.db, i64::MIN, i64::MAX).1);
            assert_eq!(
                actual
                    .iter()
                    .find(|e| e.session == CHILD)
                    .unwrap()
                    .delta
                    .total_tokens,
                220
            );
        } else {
            assert!(
                small
                    .err()
                    .is_some_and(|e| e.to_string().contains("component exceeds")),
                "{case}"
            );
            assert!(store.checkpoint(&child)?.is_none());
            let ready = store.prepare(update, cutoff, BTreeMap::new(), 1 << 20)?;
            store.publish(ready)?;
        }
    }
    Ok(())
}

#[test]
fn tiny_parent_still_fits_the_original_full_read_budget() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path().canonicalize()?;
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let parent = root.join(format!("rollout-{PHYSICAL_PREFIX}.jsonl"));
    let other = root.join("prior.jsonl");
    let child = root.join(format!("rollout-{CHILD}.jsonl"));
    let mut store = Store::open(Connection::open_in_memory()?)?;
    let origin = parse(&parent, &[header(PARENT, 0), count(1, 1, 1, true)], cutoff);
    let prior = parse(&other, &[header(CHILD, 0), count(1, 3, 2, true)], cutoff);
    for (path, state) in [(&parent, &origin), (&other, &prior)] {
        let ready = store.prepare(new_source(path, state), cutoff, BTreeMap::new(), 1 << 20)?;
        store.publish(ready)?;
    }
    let mut meta = header(CHILD, 2);
    meta["payload"]["history_base"] = json!({"thread_id":PHYSICAL_PREFIX,
        "end_ordinal_exclusive":2,"end_byte_offset":origin.offset});
    let incoming = parse(&child, &[meta, count(3, 4, 3, false)], cutoff);
    let update = new_source(&child, &incoming);
    let full = SourceUpdate {
        replace: true,
        ..update
    };
    let (mut low, mut high) = (0, 1 << 20);
    while low < high {
        let middle = (low + high) / 2;
        match store.prepare(full, cutoff, BTreeMap::new(), middle) {
            Ok(_) => high = middle,
            Err(error) if error.to_string().contains("input budget") => low = middle + 1,
            Err(error) => return Err(error),
        }
    }
    let ready = store.prepare(update, cutoff, BTreeMap::new(), high)?;
    store.publish(ready)?;
    assert_eq!(read(&mut store.db, i64::MIN, i64::MAX).1.len(), 3);
    Ok(())
}

#[test]
fn long_continuation_appends_within_budget_after_reopen_and_refuses_a_child() -> anyhow::Result<()>
{
    let dir = tempfile::tempdir()?;
    let root = dir.path().canonicalize()?;
    let file = root.join("index.sqlite");
    let parent = root.join(format!("rollout-{PHYSICAL_PREFIX}.jsonl"));
    let leaf = root.join(format!("rollout-{CHILD}.jsonl"));
    let child = root.join(format!("rollout-{OTHER}.jsonl"));
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let names = BTreeMap::new();
    let source_budget = 1 << 20;
    let append_budget = 8192;
    let head = std::iter::once(header(PARENT, 0))
        .chain((1..=48).map(|n| count(n, n, n as u64, true)))
        .collect::<Vec<_>>();
    let base = parse(&parent, &head, cutoff);
    let mut meta = header(PARENT, 49);
    meta["payload"]["history_base"] = json!({"thread_id":PHYSICAL_PREFIX,
        "end_ordinal_exclusive":49,"end_byte_offset":base.offset});
    let own = [meta, count(50, 49, 49, true)];
    let first = parse(&leaf, &own, cutoff);
    let mut store = Store::open(Connection::open(&file)?)?;
    for (path, state) in [(&parent, &base), (&leaf, &first)] {
        let batch = store.prepare(
            new_source(path, state),
            cutoff,
            names.clone(),
            source_budget,
        )?;
        store.publish(batch)?;
    }
    let append = |row: &Value| -> std::io::Result<()> {
        writeln!(fs::OpenOptions::new().append(true).open(&leaf)?, "{row}")
    };
    let duplicate = count(51, 50, 49, true);
    append(&duplicate)?;
    let mut reader = SourceReader::restore(&store, &leaf)?;
    assert!(reader.step(
        &mut store,
        cutoff,
        &mut Budget::new(std::iter::empty()),
        &names,
        append_budget
    )?);
    assert_eq!(read(&mut store.db, i64::MIN, i64::MAX).1.len(), 49);
    let (committed_revision, committed) = store.checkpoint(&leaf)?.unwrap();
    drop(store);

    let new_count = count(52, 51, 50, true);
    append(&new_count)?;
    let mut store = Store::open(Connection::open(&file)?)?;
    let mut reader = SourceReader::restore(&store, &leaf)?;
    store.db.execute_batch(
        "CREATE TEMP TRIGGER reject_append BEFORE UPDATE ON token_sources
         BEGIN SELECT RAISE(ABORT, 'fixture append rollback'); END;",
    )?;
    let append_next = |reader: &mut SourceReader, store: &mut Store| {
        reader.step(
            store,
            cutoff,
            &mut Budget::new(std::iter::empty()),
            &names,
            append_budget,
        )
    };
    let error = append_next(&mut reader, &mut store).unwrap_err();
    assert!(error.to_string().contains("fixture append rollback"));
    let (stored_revision, stored) = store.checkpoint(&leaf)?.unwrap();
    assert_eq!(
        (stored_revision, stored.offset),
        (committed_revision, committed.offset)
    );
    store.db.execute_batch("DROP TRIGGER reject_append")?;
    assert!(append_next(&mut reader, &mut store)?);
    let (latest_revision, latest) = store.checkpoint(&leaf)?.unwrap();
    let (_, actual) = read(&mut store.db, i64::MIN, i64::MAX);
    assert_eq!(actual.len(), 50);

    let reference_file = root.join("reference.jsonl");
    let reference = parse(
        &reference_file,
        &[
            own[0].clone(),
            own[1].clone(),
            duplicate.clone(),
            new_count.clone(),
        ],
        cutoff,
    );
    let mut full = Store::open(Connection::open_in_memory()?)?;
    for (path, state) in [(&parent, &base), (&leaf, &reference)] {
        let batch = full.prepare(
            new_source(path, state),
            cutoff,
            names.clone(),
            source_budget,
        )?;
        full.publish(batch)?;
    }
    let (_, expected) = read(&mut full.db, i64::MIN, i64::MAX);
    assert!(actual == expected);

    let mut correction = count(53, 52, 50, true);
    correction["payload"]["info"]["total_token_usage"]["cached_input_tokens"] = json!(999);
    let mut backdated = count(53, 52, 51, true);
    backdated["timestamp"] = json!("2026-01-02T13:00:52+02:00"); // 11:00:52Z, before the anchor.
    for rejected in [correction, backdated] {
        let mut changed = parse(
            &reference_file,
            &[
                own[0].clone(),
                own[1].clone(),
                duplicate.clone(),
                new_count.clone(),
                rejected,
            ],
            cutoff,
        );
        changed.events.drain(..3);
        let error = store
            .prepare(
                SourceUpdate {
                    source: &leaf,
                    state: &changed,
                    previous_revision: Some(latest_revision),
                    replace: false,
                },
                cutoff,
                names.clone(),
                append_budget,
            )
            .err()
            .unwrap();
        assert!(error.to_string().contains("Token component exceeds"));
        let (retained_revision, retained) = store.checkpoint(&leaf)?.unwrap();
        assert_eq!(
            (retained_revision, retained.offset),
            (latest_revision, latest.offset)
        );
        assert!(read(&mut store.db, i64::MIN, i64::MAX).1 == actual);
    }

    let mut child_meta = header(PARENT, 53);
    child_meta["payload"]["history_base"] = json!({"thread_id":CHILD,
        "end_ordinal_exclusive":53,"end_byte_offset":latest.offset});
    let competing = parse(&child, &[child_meta, count(54, 52, 51, true)], cutoff);
    let batch = store.prepare(
        new_source(&child, &competing),
        cutoff,
        names.clone(),
        source_budget,
    )?;
    store.publish(batch)?;
    store.record_coverage(
        Coverage {
            pending_sources: 0,
            issues: 0,
            maintenance_blocked: false,
        },
        false,
    )?;
    let old = read(&mut store.db, i64::MIN, i64::MAX)
        .1
        .into_iter()
        .min_by_key(|event| event.timestamp.clone())
        .unwrap();
    let proof = |events: &mut [UsageEvent], _: &BTreeMap<String, String>| {
        for event in events {
            event.provider = "verified-supplier".into();
            event.model = "verified-model".into();
            event.provider_source = Some("request-log-exact".into());
        }
        Ok(())
    };
    assert!(
        store
            .reclassify_session(PARENT, source_budget, &BTreeSet::new(), proof)?
            .is_some()
    );
    let mut late = count(53, 53, 52, true);
    late["timestamp"] = json!("2026-01-02T15:00:53Z");
    append(&late)?;
    assert!(!append_next(&mut reader, &mut store)?);
    let (retained_revision, retained) = store.checkpoint(&leaf)?.unwrap();
    assert_eq!(
        (retained_revision, retained.offset),
        (latest_revision, latest.offset)
    );
    let read_leaf = |reader: &mut SourceReader, store: &mut Store| {
        reader.step(
            store,
            cutoff,
            &mut Budget::new(std::iter::empty()),
            &names,
            source_budget,
        )
    };
    let before_failure = read(&mut store.db, i64::MIN, i64::MAX);
    store.db.execute_batch(
        "CREATE TEMP TRIGGER abort_day BEFORE INSERT ON token_days
        BEGIN SELECT RAISE(ABORT, 'fixture day rollback'); END;",
    )?;
    assert!(
        read_leaf(&mut reader, &mut store)
            .unwrap_err()
            .to_string()
            .contains("fixture day rollback")
    );
    assert_eq!(store.checkpoint(&leaf)?.unwrap().1.offset, latest.offset);
    assert!(read(&mut store.db, i64::MIN, i64::MAX) == before_failure);
    store.db.execute_batch("DROP TRIGGER abort_day")?;
    assert!(read_leaf(&mut reader, &mut store)?);
    let (_, rows) = read(&mut store.db, i64::MIN, i64::MAX);
    let retained = rows.iter().find(|event| event.id == old.id).unwrap();
    assert_eq!(retained.delta, old.delta);
    assert_eq!(retained.total, old.total);
    assert_eq!(
        retained.provider_source.as_deref(),
        Some("request-log-exact")
    );
    assert_eq!(
        (&*retained.provider, &*retained.model),
        ("verified-supplier", "verified-model")
    );
    assert!(
        rows.iter()
            .any(|event| event.timestamp == "2026-01-02T15:00:53.000Z")
    );
    Ok(())
}

#[test]
fn reprojected_request_proofs_follow_competing_counts_and_receipt_corrections() -> anyhow::Result<()>
{
    let dir = tempfile::tempdir()?;
    let root = dir.path().canonicalize()?;
    let source = root.join("rollout-counts.jsonl");
    let peer = root.join("rollout-peer.jsonl");
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let names = BTreeMap::new();
    let limit = 1 << 20;
    let mut store = Store::open(Connection::open_in_memory()?)?;
    let usage = |n| json!({"input_tokens":100*n,"cached_input_tokens":20*n,"output_tokens":10*n});
    let counter = |at: &str, n, cached: bool| {
        let mut row = json!({"type":"event_msg","timestamp":at,"payload":{"type":"token_count",
            "info":{"last_token_usage":usage(1),"total_token_usage":usage(n)}}});
        if !cached {
            for key in ["last_token_usage", "total_token_usage"] {
                row["payload"]["info"][key]
                    .as_object_mut()
                    .unwrap()
                    .remove("cached_input_tokens");
            }
        }
        row
    };
    let receipt = |id| {
        json!({"type":"token_usage_record","timestamp":"2026-01-02T15:00:01Z",
        "payload":{"thread_id":PARENT,"turn_id":"same","response_id":id,"usage":usage(1),"thread_token_usage":usage(2)}})
    };
    let header = json!({"type":"session_meta","timestamp":"2026-01-02T12:00:00Z",
        "payload":{"id":PARENT,"model_provider":"supplier-a"}});
    let initial = parse(
        &source,
        &[
            header.clone(),
            json!({"type":"turn_context","timestamp":"2026-01-02T12:00:00Z",
            "payload":{"turn_id":"same","model":"fixture"}}),
            counter("2026-01-02T12:00:01Z", 1, true),
            counter("2026-01-02T15:00:00Z", 2, false),
            receipt("claim-one"),
            counter("2026-01-02T18:00:00Z", 3, true),
        ],
        cutoff,
    );
    let other = parse(&peer, &[header], cutoff);
    for (path, state) in [(&source, &initial), (&peer, &other)] {
        let batch = store.prepare(new_source(path, state), cutoff, names.clone(), limit)?;
        store.publish(batch)?;
    }
    store.record_coverage(
        Coverage {
            pending_sources: 0,
            issues: 0,
            maintenance_blocked: false,
        },
        false,
    )?;
    assert!(
        store
            .reclassify_session(PARENT, limit, &BTreeSet::new(), |rows, _| {
                for event in rows {
                    event.provider = "verified-supplier".into();
                    event.provider_source = Some("request-log-exact".into());
                }
                Ok(())
            })?
            .is_some()
    );
    let before = read(&mut store.db, i64::MIN, i64::MAX).1;
    assert_eq!(before.len(), 3);
    assert!(before[1].delta.cached_known);
    let mut reader = SourceReader::restore(&store, &source)?;
    let mut append = |row: &Value, store: &mut Store| -> anyhow::Result<_> {
        writeln!(fs::OpenOptions::new().append(true).open(&source)?, "{row}")?;
        assert!(reader.step(
            store,
            cutoff,
            &mut Budget::new(std::iter::empty()),
            &names,
            limit
        )?);
        Ok(read(&mut store.db, i64::MIN, i64::MAX).1)
    };
    let nearby = append(&counter("2026-01-02T18:04:00Z", 4, true), &mut store)?;
    assert_eq!(nearby.len(), 4);
    assert!(nearby[0..2] == before[0..2]);
    assert_eq!(nearby[2].id, before[2].id);
    assert_eq!(nearby[2].provider_source, None);
    let corrected = append(&receipt("claim-two"), &mut store)?;
    assert!(corrected[0] == before[0]);
    assert_eq!(corrected[1].id, before[1].id);
    assert_eq!(
        corrected[1].delta.total_tokens,
        before[1].delta.total_tokens
    );
    assert!(!corrected[1].delta.cached_known);
    assert_eq!(corrected[1].provider_source, None);
    let query = Query::parse(&json!({}), cutoff).unwrap();
    let days = Store::read_selection(&mut store.db, &query, None, None)?.days;
    let totals: u64 = days
        .iter()
        .map(|day| day["usage"]["totalTokens"].as_u64().unwrap())
        .sum();
    assert_eq!(totals, 440);
    let verified: u64 = days
        .iter()
        .flat_map(|day| day["breakdowns"].as_array().unwrap())
        .filter(|group| group["provider"] == "verified-supplier")
        .map(|group| group["recordCount"].as_u64().unwrap())
        .sum();
    assert_eq!(verified, 1);
    Ok(())
}

#[test]
fn continuation_receipts_require_reconciliation_in_the_incoming_or_prior_batch()
-> anyhow::Result<()> {
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let names = BTreeMap::new();
    let full_budget = 1 << 20;
    let append_budget = 8192;
    for (prior_receipt, incoming_receipt) in [(false, false), (false, true), (true, false)] {
        let dir = tempfile::tempdir()?;
        let root = dir.path().canonicalize()?;
        let parent = root.join(format!("rollout-{PHYSICAL_PREFIX}.jsonl"));
        let leaf = root.join(format!("rollout-{CHILD}.jsonl"));
        let head = std::iter::once(header(PARENT, 0))
            .chain((1..=48).map(|n| count(n, n, n as u64, true)))
            .collect::<Vec<_>>();
        let base = parse(&parent, &head, cutoff);
        let mut meta = header(PARENT, 49);
        meta["payload"]["history_base"] = json!({"thread_id":PHYSICAL_PREFIX,
            "end_ordinal_exclusive":49,"end_byte_offset":base.offset});
        let mut rows = vec![
            meta,
            json!({"type":"turn_context","ordinal":50,"timestamp":"2026-01-02T12:00:48Z",
                "payload":{"turn_id":"tail","model":"fixture"}}),
        ];
        let receipt = |ordinal: usize, second: usize| {
            let counter = count(ordinal, second, 50, true);
            let info = &counter["payload"]["info"];
            json!({"type":"token_usage_record","ordinal":ordinal,"timestamp":counter["timestamp"],
                "payload":{"thread_id":PARENT,"turn_id":"tail","response_id":"tail-response",
                    "usage":info["last_token_usage"],"thread_token_usage":info["total_token_usage"]}})
        };
        if prior_receipt {
            // The receipt precedes the committed anchor: only the raw claim survives.
            rows.push(receipt(51, 49));
        }
        rows.push(count(49 + rows.len(), 50, 49, true));
        let first = parse(&leaf, &rows, cutoff);
        let mut store = Store::open(Connection::open_in_memory()?)?;
        for (source, state) in [(&parent, &base), (&leaf, &first)] {
            let prepared = store.prepare(
                new_source(source, state),
                cutoff,
                names.clone(),
                full_budget,
            )?;
            store.publish(prepared)?;
        }
        let before = read(&mut store.db, i64::MIN, i64::MAX);
        assert_eq!(before.1.len(), 49);
        assert!(before.1.iter().all(|e| e.terminal_response.is_none()));
        assert_eq!(
            store
                .observations(&leaf)?
                .iter()
                .filter(|e| e.terminal_response.is_some())
                .count(),
            usize::from(prior_receipt)
        );
        let (revision, checkpoint) = store.checkpoint(&leaf)?.unwrap();
        let ordinal = 49 + rows.len();
        let mut native = count(ordinal, 51, 50, true);
        for key in ["last_token_usage", "total_token_usage"] {
            native["payload"]["info"][key]
                .as_object_mut()
                .unwrap()
                .remove("cached_input_tokens");
        }
        {
            let mut file = fs::OpenOptions::new().append(true).open(&leaf)?;
            writeln!(file, "{native}")?;
            if incoming_receipt {
                writeln!(file, "{}", receipt(ordinal + 1, 52))?;
            }
        }
        let mut reader = SourceReader::restore(&store, &leaf)?;
        let step = |reader: &mut SourceReader, store: &mut Store, limit| {
            reader.step(
                store,
                cutoff,
                &mut Budget::new(std::iter::empty()),
                &names,
                limit,
            )
        };
        let bounded = step(&mut reader, &mut store, append_budget);
        if prior_receipt || incoming_receipt {
            assert!(!bounded?);
            assert!(read(&mut store.db, i64::MIN, i64::MAX) == before);
            let (unchanged, retained) = store.checkpoint(&leaf)?.unwrap();
            assert_eq!((unchanged, retained.offset), (revision, checkpoint.offset));
            assert!(step(&mut reader, &mut store, append_budget)?);
        } else {
            assert!(bounded?);
        }
        let actual = read(&mut store.db, i64::MIN, i64::MAX).1;
        assert_eq!(actual.len(), 50);
        let last = actual.last().unwrap();
        assert!(last.terminal_response.is_none());
        assert_eq!(last.delta.total_tokens, 110);
        assert_eq!(last.delta.cached_known, prior_receipt || incoming_receipt);
        if last.delta.cached_known {
            assert_eq!(last.delta.cached_input_tokens, 20);
        }
        assert_eq!(
            store.checkpoint(&leaf)?.unwrap().1.offset,
            fs::metadata(&leaf)?.len()
        );

        let mut replay = Store::open(Connection::open_in_memory()?)?;
        for source in [&parent, &leaf] {
            let mut reader = SourceReader::restore(&replay, source)?;
            assert!(step(&mut reader, &mut replay, full_budget)?);
        }
        assert!(actual == read(&mut replay.db, i64::MIN, i64::MAX).1);
    }
    Ok(())
}

#[test]
fn legacy_identity_prefix_repairs_zero_issue_rows_and_marks_clean_sources_once() -> anyhow::Result<()> {
    for affected in ["response", "model", "provider", "unrelated"] {
        let root = tempfile::tempdir()?;
        let source = root.path().canonicalize()?.join("rollout-legacy.jsonl");
        let database = root.path().join("usage.sqlite");
        let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let long = "x".repeat(310);
        let context = json!({"type":"turn_context", "ordinal":1, "timestamp":"2026-01-02T12:00:01Z",
            "payload":{"turn_id":"turn", "model":if affected=="model" {&long} else {"normal"},
                "model_provider":if affected=="provider" {&long} else {"relay"}}});
        let second = count(3, 3, 2, true);
        let info = &second["payload"]["info"];
        let receipt = json!({"type":"token_usage_record","ordinal":3,"timestamp":"2026-01-02T12:00:03Z",
            "payload":{"thread_id":PARENT,"turn_id":"turn","response_id":&long,
                "usage":info["last_token_usage"],"thread_token_usage":info["total_token_usage"]}});
        let mut rows = vec![header(PARENT, 0), context, count(2, 2, 1, true)];
        if affected == "response" {
            rows.push(receipt);
        } else {
            rows.push(json!({"type":"response_item","ordinal":3,"timestamp":"2026-01-02T12:00:03Z",
                "payload":{"type":"message","model":&long,"response_id":&long}}));
        }
        let mut old = parse(&source, &rows, cutoff);
        if affected == "response" {
            old.events.retain(|event| event.terminal_response.is_none());
        } else if affected == "model" {
            old.model.clear();
            old.model_unavailable = true;
            old.events[0].model.clear();
            old.events[0].context_conflicts.model = true;
        } else if affected == "provider" {
            old.provider = "supplier-a".into();
            old.events[0].provider = old.provider.clone();
        }
        assert_eq!(old.issues, 0);
        let mut store = Store::open(Connection::open(&database)?)?;
        let prepared = store.prepare(new_source(&source, &old), cutoff, BTreeMap::new(), 1 << 20)?;
        store.publish(prepared)?;
        store.db.execute("UPDATE token_sources SET checkpoint=CAST(json_remove(checkpoint, '$.legacy_identity_prefix') AS BLOB)", [])?;
        let before = read(&mut store.db, i64::MIN, i64::MAX);
        assert!(store.sealed_queued_sources(std::slice::from_ref(&source))?.0.is_empty());
        drop(store);
        let mut store = Store::open(Connection::open(&database)?)?;
        let mut reader = SourceReader::restore(&store, &source)?;
        let step = |reader: &mut SourceReader, store: &mut Store, bytes| {
            let mut budget = Budget::new(std::iter::empty());
            budget.read_left = bytes;
            reader.step(store, cutoff, &mut budget, &BTreeMap::new(), 1 << 20)
        };
        assert!(!step(&mut reader, &mut store, 100)?);
        assert!(before == read(&mut store.db, i64::MIN, i64::MAX));
        store.db.pragma_update(None, "query_only", true)?;
        assert!(step(&mut reader, &mut store, 1 << 20).is_err());
        assert!(before == read(&mut store.db, i64::MIN, i64::MAX));
        store.db.pragma_update(None, "query_only", false)?;
        assert!(step(&mut reader, &mut store, 1 << 20)?);
        let after = read(&mut store.db, i64::MIN, i64::MAX);
        let final_state = store.checkpoint(&source)?.unwrap().1;
        assert_eq!(final_state.legacy_identity_prefix, 0);
        assert!(store.sealed_queued_sources(std::slice::from_ref(&source))?.0.contains_key(&source));
        match affected {
            "response" => assert!(after.1.iter().any(|e| e.terminal_response.as_deref() == Some(&long))),
            "model" => assert_eq!(after.1[0].model, long),
            "provider" => assert_eq!(after.1[0].provider, long),
            _ => assert!(before == after),
        }
        drop(store);
        let mut store = Store::open(Connection::open(&database)?)?;
        let mut reader = SourceReader::restore(&store, &source)?;
        assert!(step(&mut reader, &mut store, 1)?);
        assert!(after == read(&mut store.db, i64::MIN, i64::MAX));
    }
    Ok(())
}

#[test]
fn legacy_identity_prefix_allows_append_during_check_and_compressed_cold_reopen() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let path = root.path().canonicalize()?.join("rollout-old.jsonl");
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    for compressed in [false, true] {
        let rows = [header(PARENT, 0), count(1, 1, 1, true),
            json!({"type":"response_item","ordinal":2,"payload":{"type":"message","content":"x".repeat(1500)}})];
        let state = parse(&path, &rows, cutoff);
        let source = if compressed {
            let archive = path.with_extension("jsonl.zst");
            fs::write(&archive, zstd::encode_all(fs::File::open(&path)?, 1)?)?;
            archive
        } else { path.clone() };
        let mut state = state;
        state.stamp = Some(FileState::source_stamp(&source)?);
        let end = state.offset;
        let db = root.path().join(if compressed {"compressed.sqlite"} else {"plain.sqlite"});
        let mut store = Store::open(Connection::open(&db)?)?;
        let prepared = store.prepare(new_source(&source, &state), cutoff, BTreeMap::new(), 1 << 20)?;
        store.publish(prepared)?;
        store.db.execute("UPDATE token_sources SET checkpoint=CAST(json_remove(checkpoint, '$.legacy_identity_prefix') AS BLOB)", [])?;
        let mut reader = SourceReader::restore(&store, &source)?;
        let step = |r: &mut SourceReader, s: &mut Store, size| {
            let mut budget = Budget::new(std::iter::empty());
            budget.read_left = size;
            r.step(s, cutoff, &mut budget, &BTreeMap::new(), 1 << 20)
        };
        assert!(!step(&mut reader, &mut store, 128)?);
        if !compressed {
            writeln!(fs::OpenOptions::new().append(true).open(&source)?, "{}", count(3, 3, 2, true))?;
            assert!(!step(&mut reader, &mut store, 2048)?);
            assert_eq!(read(&mut store.db, i64::MIN, i64::MAX).1.len(), 2);
            assert_eq!(store.checkpoint(&source)?.unwrap().1.legacy_identity_prefix, end);
        }
        let before_finish = read(&mut store.db, i64::MIN, i64::MAX);
        for _ in 0..32 {
            if step(&mut reader, &mut store, 128)? { break; }
        }
        assert_eq!(store.checkpoint(&source)?.unwrap().1.legacy_identity_prefix, 0);
        drop(store);
        let mut store = Store::open(Connection::open(&db)?)?;
        let mut reader = SourceReader::restore(&store, &source)?;
        assert!(step(&mut reader, &mut store, 1)?);
        assert!(before_finish == read(&mut store.db, i64::MIN, i64::MAX));
    }
    Ok(())
}

#[test]
fn legacy_identity_prefix_scan_commits_live_growth_while_an_archive_is_unverified() -> anyhow::Result<()> {
    use super::scan::Scan;

    let dir = tempfile::tempdir()?;
    let root = dir.path().canonicalize()?;
    let sessions = root.join("sessions");
    let archived = root.join("archived_sessions");
    fs::create_dir(&sessions)?;
    fs::create_dir(&archived)?;
    let live = sessions.join(format!("rollout-{PARENT}.jsonl"));
    let archive = archived.join(format!("rollout-{OTHER}.jsonl.zst"));
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    parse(&live, &[header(PARENT, 0), count(1, 1, 1, true)], cutoff);
    let padding = "x".repeat(90 * 1024);
    let mut z = zstd::Encoder::new(fs::File::create(&archive)?, 1)?;
    writeln!(z, "{}", header(OTHER, 0))?;
    writeln!(z, "{}", count(1, 1, 1, true))?;
    for i in 2..165 {
        writeln!(z, "{}", json!({"type":"response_item","ordinal":i,
            "payload":{"type":"message","content":padding}}))?;
    }
    z.finish()?;
    let mut store = Store::open(Connection::open_in_memory()?)?;
    let mut scan = Scan::default();
    let round = |scan: &mut Scan, store: &mut Store| {
        let report = scan.round(store, &root, cutoff, &BTreeMap::new(), 1 << 20);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert!(report.maintenance_error.is_none(), "{:?}", report.maintenance_error);
        report.complete
    };
    for _ in 0..12 {
        if round(&mut scan, &mut store) { break; }
    }
    assert_eq!(read(&mut store.db, i64::MIN, i64::MAX).1.len(), 2);
    store.db.execute("UPDATE token_sources SET checkpoint=CAST(json_remove(checkpoint, '$.legacy_identity_prefix') AS BLOB)", [])?;
    let mut scan = Scan::default();
    assert!(!round(&mut scan, &mut store));
    for n in 2..=3 {
        writeln!(fs::OpenOptions::new().append(true).open(&live)?, "{}", count(n, n, n as u64, true))?;
        assert!(!round(&mut scan, &mut store));
        assert_eq!(read(&mut store.db, i64::MIN, i64::MAX).1.len(), n + 1);
        assert_ne!(store.checkpoint(&archive)?.unwrap().1.legacy_identity_prefix, 0);
        assert!(store.sealed_queued_sources(std::slice::from_ref(&archive))?.0.is_empty());
    }
    for _ in 0..12 {
        if round(&mut scan, &mut store) { break; }
    }
    assert_eq!(store.checkpoint(&archive)?.unwrap().1.legacy_identity_prefix, 0);
    assert_eq!(read(&mut store.db, i64::MIN, i64::MAX).1.len(), 4);
    Ok(())
}

#[test]
fn legacy_large_line_issue_replays_once_and_replaces_the_old_snapshot_atomically()
-> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let source = root.path().canonicalize()?.join("rollout-legacy.jsonl");
    let database = root.path().join("usage.sqlite");
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let mut second = count(2, 2, 2, true);
    second["payload"]["extra"] = json!("x".repeat(2 << 20));
    let mut state = parse(
        &source,
        &[header(PARENT, 0), count(1, 1, 1, true), second],
        cutoff,
    );
    state.events.pop();
    state.previous = Some(state.events[0].total);
    state.issues = 1;
    state.rollout_out_of_order = true;
    let mut store = Store::open(Connection::open(&database)?)?;
    let prepared = store.prepare(
        new_source(&source, &state),
        cutoff,
        BTreeMap::new(),
        1 << 20,
    )?;
    store.publish(prepared)?;
    store.db.execute("UPDATE token_sources SET checkpoint = CAST(json_remove(checkpoint, '$.large_meter_checked') AS BLOB)", [])?;
    let (original_revision, original) = read(&mut store.db, i64::MIN, i64::MAX);
    assert_eq!(original.len(), 1);
    drop(store);
    let mut store = Store::open(Connection::open(&database)?)?;
    let mut reader = SourceReader::restore(&store, &source)?;
    store.db.pragma_update(None, "query_only", true)?;
    assert!(
        reader
            .step(
                &mut store,
                cutoff,
                &mut Budget::new(std::iter::empty()),
                &BTreeMap::new(),
                1 << 20
            )
            .is_err()
    );
    assert!(read(&mut store.db, i64::MIN, i64::MAX) == (original_revision, original));
    store.db.pragma_update(None, "query_only", false)?;
    assert!(reader.step(
        &mut store,
        cutoff,
        &mut Budget::new(std::iter::empty()),
        &BTreeMap::new(),
        1 << 20
    )?);
    let (revision, events) = read(&mut store.db, i64::MIN, i64::MAX);
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].delta.input_tokens, 100);
    assert_eq!(store.checkpoint(&source)?.unwrap().1.issues, 0);
    let mut reader = SourceReader::restore(&store, &source)?;
    assert!(reader.step(
        &mut store,
        cutoff,
        &mut Budget::new(std::iter::empty()),
        &BTreeMap::new(),
        1 << 20
    )?);
    assert!(read(&mut store.db, i64::MIN, i64::MAX) == (revision, events));
    Ok(())
}

#[test]
fn oversized_unfinished_meter_reopens_at_its_start_and_publishes_only_once() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let home = root.path().canonicalize()?;
    let path = home.join("rollout-partial.jsonl");
    let database = home.join("usage.sqlite");
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let prefix = format!("{}\n{}\n", header(PARENT, 0), count(1, 1, 1, true));
    let mut second = count(2, 2, 2, false);
    second["payload"]["extra"] = json!("x".repeat(5 << 20));
    fs::write(&path, format!("{prefix}{second}"))?;
    let mut store = Store::open(Connection::open(&database)?)?;
    let mut reader = SourceReader::restore(&store, &path)?;
    for _ in 0..2 {
        assert!(!reader.step(
            &mut store,
            cutoff,
            &mut Budget::new(std::iter::empty()),
            &BTreeMap::new(),
            1 << 20
        )?);
    }
    assert!(reader.waiting_for_newline());
    assert_eq!(
        store.checkpoint(&path)?.unwrap().1.offset,
        prefix.len() as u64
    );
    assert_eq!(read(&mut store.db, i64::MIN, i64::MAX).1.len(), 1);
    drop(reader);
    drop(store);
    writeln!(fs::OpenOptions::new().append(true).open(&path)?)?;
    let mut store = Store::open(Connection::open(&database)?)?;
    let mut reader = SourceReader::restore(&store, &path)?;
    for _ in 0..3 {
        if reader.step(
            &mut store,
            cutoff,
            &mut Budget::new(std::iter::empty()),
            &BTreeMap::new(),
            1 << 20,
        )? {
            break;
        }
    }
    let events = read(&mut store.db, i64::MIN, i64::MAX).1;
    assert_eq!(events.len(), 2);
    assert_eq!(
        events[1].rollout_position,
        Some((2, prefix.len() as u64, path.metadata()?.len()))
    );
    assert_eq!(events[1].total.input_tokens, 200);
    assert_eq!(events[1].delta.input_tokens, 100);
    assert_eq!(
        store.checkpoint(&path)?.unwrap().1.offset,
        path.metadata()?.len()
    );
    let mut reader = SourceReader::restore(&store, &path)?;
    assert!(reader.step(
        &mut store,
        cutoff,
        &mut Budget::new(std::iter::empty()),
        &BTreeMap::new(),
        1 << 20
    )?);
    assert!(events == read(&mut store.db, i64::MIN, i64::MAX).1);
    Ok(())
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
    let response_id = "resp-a".repeat(100);
    let mut first = terminal(2, "turn-a", response_id.as_str(), usage(10, 3, 2), usage(10, 3, 2));
    first["payload"]["extra"] = json!("x".repeat(1100 * 1024));
    let mut wrong_turn = first.clone();
    wrong_turn["payload"]["turn_id"] = json!("turn-b");
    wrong_turn["payload"]["response_id"] = json!("wrong-turn");
    append(&[head.clone(), context(1, "turn-a"), wrong_turn, first.clone()]);
    let initial = step(&rollout);
    assert_eq!(initial.len(), 1);
    assert_eq!(initial[0].terminal_response.as_deref(), Some(response_id.as_str()));
    assert!(step(&rollout) == initial);
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
                .record_coverage(
                    Coverage {
                        pending_sources: 0,
                        issues: 0,
                        maintenance_blocked: false,
                    },
                    false,
                )
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
fn compatibility_fingerprints_bind_the_selected_events_and_allow_a_summary_only_read() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().canonicalize().unwrap();
    let first_path = home.join("first.jsonl");
    let second_path = home.join("second.jsonl");
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let mut store = Store::open(Connection::open_in_memory().unwrap()).unwrap();
    for (path, session, shift) in [(&first_path, PARENT, 0), (&second_path, CHILD, 4)] {
        let mut metadata = header(session, 0);
        if session == CHILD {
            metadata["payload"]["model_provider"] = json!("supplier-b");
        }
        let state = parse(
            path,
            &[
                metadata,
                count(1, 1 + shift, 1, true),
                count(2, 2 + shift, 2, true),
            ],
            cutoff,
        );
        let staged = store
            .prepare(new_source(path, &state), cutoff, BTreeMap::new(), 8192)
            .unwrap();
        store.publish(staged).unwrap();
    }
    let events = read(&mut store.db, i64::MIN, i64::MAX).1;
    let day = events[0].date;
    let end = day.succ_opt().unwrap();
    let midnight = |day: NaiveDate| {
        Local
            .from_local_datetime(&day.and_hms_opt(0, 0, 0).unwrap())
            .single()
            .unwrap()
            .timestamp_millis()
    };
    let old = crate::usage_history::structured::Snapshot {
        count_cache_available: true,
        provider_names: BTreeMap::from([(events[0].provider.clone(), "Renamed relay".into())]),
        events,
        ..Default::default()
    };
    let names = &old.provider_names;
    let from = old.events[0]
        .timestamp
        .parse::<DateTime<chrono::FixedOffset>>()
        .unwrap()
        .timestamp_millis();
    for (request, bounds) in [
        (json!({"from":day.to_string(),"to":day.to_string()}), None),
        (
            json!({"startAt":midnight(day),"endAt":midnight(end)}),
            Some((midnight(day), midnight(end))),
        ),
        (
            json!({"startAt":from+1,"endAt":from+5000,"includeTrend":true}),
            Some((from + 1, from + 5000)),
        ),
    ] {
        let query = Query::parse(&request, cutoff).unwrap();
        let projected =
            Store::read_response(&mut store.db, &query, &request, Some(PARENT), names).unwrap();
        assert_eq!(projected["scan"]["retentionDays"], 60);
        assert_eq!(projected["scan"]["preciseFrom"], cutoff.to_string());
        assert!(projected["scan"].get("bytesRead").is_none());
        assert!(projected["scan"].get("indexedFiles").is_none());
        let selected = HashSet::from([PARENT.to_owned()]);
        for (actual, expected) in [
            (&projected["days"][0], old.usage_days(&query, bounds, None)),
            (
                &projected["scopedUsage"]["days"][0],
                old.usage_days(&query, bounds, Some(&selected)),
            ),
        ] {
            assert_eq!(actual["fingerprintCoverage"], "events");
            assert_eq!(
                actual["accountingFingerprint"],
                expected[0]["accountingFingerprint"]
            );
            let breakdowns = |day: &Value| {
                day["breakdowns"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|row| {
                        (
                            row["provider"].as_str().unwrap().to_owned(),
                            (
                                row["membershipFingerprint"].clone(),
                                row["providerName"].clone(),
                            ),
                        )
                    })
                    .collect::<BTreeMap<_, _>>()
            };
            assert_eq!(breakdowns(actual), breakdowns(&expected[0]));
            assert_eq!(actual["recordCount"], expected[0]["recordCount"]);
        }
        let mut fast = request;
        fast["includeFingerprints"] = json!(false);
        let result =
            Store::read_response(&mut store.db, &query, &fast, Some(PARENT), names).unwrap();
        assert!(result["days"][0].get("fingerprintCoverage").is_none());
        assert_eq!(result["days"][0]["usage"], projected["days"][0]["usage"]);
    }

    store.advance_retention(end).unwrap().unwrap();
    let request = json!({"from":day.to_string(),"to":day.to_string()});
    let query = Query::parse(&request, cutoff).unwrap();
    let after = Store::read_response(&mut store.db, &query, &request, None, names).unwrap();
    assert_eq!(after["scan"]["preciseFrom"], end.to_string());
    assert_eq!(after["days"][0]["usage"]["totalTokens"], 440);
    assert_eq!(after["days"][0]["fingerprintCoverage"], "unavailable");
    assert!(after["days"][0].get("accountingFingerprint").is_none());
    assert!(
        after["days"][0]["breakdowns"][0]
            .get("membershipFingerprint")
            .is_none()
    );
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
fn context_only_fork_reconciles_within_budget_until_it_affects_parent_counts() -> anyhow::Result<()>
{
    use crate::usage_history::structured::{Index, Snapshot};

    let dir = tempfile::tempdir()?;
    let root = fs::canonicalize(dir.path())?;
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let parent_path = root.join("rollout-parent.jsonl");
    let child_path = root.join("rollout-child.jsonl");
    let rows = std::iter::once(header(PARENT, 0))
        .chain((1..=24).map(|n| count(n, n, n as u64, true)))
        .collect::<Vec<_>>();
    let parent = parse(&parent_path, &rows, cutoff);
    let mut fork = header(CHILD, 0);
    fork["payload"]["forked_from_id"] = json!(PARENT);
    fork["payload"]["subagent_history_start_ordinal"] = json!(3);
    let copied_header = header(PARENT, 1);
    let child = parse(
        &child_path,
        &[fork.clone(), copied_header.clone(), count(3, 53, 1, true)],
        cutoff,
    );
    assert_eq!(child.inherited_contexts[0].session, PARENT);
    assert_eq!(child.events.len(), 1);
    assert_eq!(child.events[0].session, CHILD);

    let mut index = Index::default();
    index.files.insert(parent_path.clone(), parent);
    index.files.insert(child_path.clone(), child);
    let mut reference = Snapshot::default();
    index.select_observations(cutoff, &mut reference, &mut BTreeMap::new(), usize::MAX);
    index.resolve_counts(&mut reference);
    let parent = &index.files[&parent_path];
    let child = &index.files[&child_path];

    let file = root.join("fork.sqlite");
    let mut store = Store::open(Connection::open(&file)?)?;
    let ready = store.prepare(
        new_source(&parent_path, parent),
        cutoff,
        BTreeMap::new(),
        1 << 20,
    )?;
    store.publish(ready)?;
    let ready = store.prepare(
        new_source(&child_path, child),
        cutoff,
        BTreeMap::new(),
        8192,
    )?;
    let mut revision = store.publish(ready)?;
    drop(store);
    let mut store = Store::open(Connection::open(&file)?)?;
    let (_, before) = read(&mut store.db, i64::MIN, i64::MAX);
    assert_eq!(before.len(), 25);
    assert!(before == reference.events);
    assert!(
        before
            .iter()
            .all(|event| event.delta.total_tokens == 110 && !event.count_incomplete)
    );
    assert_eq!(
        store.db.query_row(
            "SELECT count(*) FROM token_source_links WHERE link = ?1",
            [format!("s:{PARENT}")],
            |row| row.get::<_, usize>(0)
        )?,
        2
    );
    let mut malformed = count(2, 49, 25, true);
    malformed["payload"]["info"]["total_token_usage"]["input_tokens"] = json!("bad");
    for (inherited_row, is_bad) in [(count(2, 49, 25, true), false), (malformed, true)] {
        let contributed = parse(
            &child_path,
            &[
                fork.clone(),
                copied_header.clone(),
                inherited_row,
                count(3, 53, 1, true),
            ],
            cutoff,
        );
        assert_eq!(contributed.invalid_counts, is_bad);
        assert_eq!(
            contributed
                .events
                .iter()
                .any(|event| event.session == PARENT && event.inherited),
            !is_bad
        );
        let update = SourceUpdate {
            source: &child_path,
            state: &contributed,
            previous_revision: Some(revision),
            replace: true,
        };
        assert!(
            store
                .prepare(update, cutoff, BTreeMap::new(), 8192)
                .err()
                .unwrap()
                .to_string()
                .contains("Token component exceeds")
        );
        assert!(read(&mut store.db, i64::MIN, i64::MAX) == (revision, before.clone()));
        let ready = store.prepare(update, cutoff, BTreeMap::new(), 1 << 20)?;
        let changed = store.publish(ready)?;
        let (_, events) = read(&mut store.db, i64::MIN, i64::MAX);
        assert_eq!(
            events
                .iter()
                .filter(|event| event.session == PARENT)
                .count(),
            if is_bad { 24 } else { 25 }
        );
        assert_eq!(
            events
                .iter()
                .find(|event| event.session == PARENT)
                .unwrap()
                .count_incomplete,
            is_bad
        );
        let restore = SourceUpdate {
            source: &child_path,
            state: child,
            previous_revision: Some(changed),
            replace: true,
        };
        assert!(
            store
                .prepare(restore, cutoff, BTreeMap::new(), 8192)
                .err()
                .unwrap()
                .to_string()
                .contains("Token component exceeds")
        );
        let ready = store.prepare(restore, cutoff, BTreeMap::new(), 1 << 20)?;
        revision = store.publish(ready)?;
        assert!(read(&mut store.db, i64::MIN, i64::MAX).1 == before);
    }
    store.record_coverage(
        Coverage {
            pending_sources: 0,
            issues: 0,
            maintenance_blocked: false,
        },
        false,
    )?;
    assert!(
        store
            .reclassify_session(PARENT, 1 << 20, &BTreeSet::new(), |events, _| {
                events[0].provider = "fixture-proof".into();
                events[0].provider_source = Some("request-log-exact".into());
                Ok(())
            })?
            .is_some()
    );
    let parent_day = |db: &Connection| {
        db.query_row("SELECT groups FROM token_days d JOIN token_sessions s ON s.id = d.session_id WHERE s.session = ?1", [PARENT], |row| row.get::<_, Vec<u8>>(0))
    };
    let day = parent_day(&store.db)?;
    let (_, classified) = read(&mut store.db, i64::MIN, i64::MAX);
    store.retire_source(&child_path, revision, cutoff, BTreeMap::new())?;
    let (_, remaining) = read(&mut store.db, i64::MIN, i64::MAX);
    assert!(remaining == classified[..24]);
    assert_eq!(parent_day(&store.db)?, day);
    let ready = store.prepare(
        new_source(&child_path, child),
        cutoff,
        BTreeMap::new(),
        8192,
    )?;
    store.publish(ready)?;
    assert!(read(&mut store.db, i64::MIN, i64::MAX).1 == classified);
    assert_eq!(parent_day(&store.db)?, day);
    Ok(())
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
fn oversized_non_usage_lines_preserve_physical_prefix_across_batches() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    for (case, ordinal, pending) in [("ordered", 2, false), ("gap", 3, true)] {
        let parent_path = root.join(format!("{case}-prefix-{PHYSICAL_PREFIX}.jsonl"));
        let child_path = root.join(format!("{case}-child.jsonl"));
        let initial = parse(
            &parent_path,
            &[header(PARENT, 0), count(1, 1, 1, true)],
            cutoff,
        );
        let long_line = format!(
            "{{\"ordinal\":{ordinal},\"type\":\"event_msg\",\"payload\":{{\"type\":\"agent_message\",\"message\":\"{}\"}}}}\n",
            "x".repeat(crate::usage_history::structured::MAX_LINE + 128)
        );
        let mut bytes = fs::read(&parent_path).unwrap();
        bytes.extend_from_slice(long_line.as_bytes());
        fs::write(&parent_path, &bytes).unwrap();
        let file = root.join(format!("{case}.sqlite3"));
        let mut store = Store::open(Connection::open(&file).unwrap()).unwrap();
        let mut reader = SourceReader::restore(&store, &parent_path).unwrap();
        let mut budget = Budget::new(std::iter::empty());
        budget.read_left = initial.offset + crate::usage_history::structured::MAX_LINE as u64 + 1;
        assert!(
            !reader
                .step(&mut store, cutoff, &mut budget, &BTreeMap::new(), 8192)
                .unwrap()
        );
        assert!(
            store.checkpoint(&parent_path).unwrap().is_none(),
            "the skipped row must not commit before its newline"
        );
        assert!(
            reader
                .step(
                    &mut store,
                    cutoff,
                    &mut Budget::new(std::iter::empty()),
                    &BTreeMap::new(),
                    8192,
                )
                .unwrap()
        );
        drop(store);

        let mut store = Store::open(Connection::open(&file).unwrap()).unwrap();
        let (_, checkpoint) = store.checkpoint(&parent_path).unwrap().unwrap();
        assert_eq!(
            checkpoint.rollout_extent,
            Some((0, ordinal as u64, bytes.len() as u64))
        );
        assert_eq!(checkpoint.rollout_out_of_order, pending);
        assert_eq!(checkpoint.issues, 0);

        let mut child_header = header(CHILD, ordinal + 1);
        child_header["payload"]["history_base"] = json!({"thread_id":PHYSICAL_PREFIX,
            "end_ordinal_exclusive":ordinal + 1,"end_byte_offset":bytes.len()});
        let child = parse(
            &child_path,
            &[child_header, count(ordinal + 2, 2, 2, false)],
            cutoff,
        );
        let prepared = store
            .prepare(
                new_source(&child_path, &child),
                cutoff,
                BTreeMap::new(),
                8192,
            )
            .unwrap();
        store.publish(prepared).unwrap();
        let (_, events) = read(&mut store.db, i64::MIN, i64::MAX);
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].count_pending, pending);
        if !pending {
            assert_eq!(events[1].delta.total_tokens, 110);
        }
    }
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
fn history_session_model_uses_context_time_across_sources_after_reopen() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let root = dir.path().canonicalize()?;
    let database = root.join("tokens.sqlite");
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let names = BTreeMap::new();
    let request = json!({"from":"2026-01-02","to":"2026-01-02"});
    let query = Query::parse(&request, cutoff).unwrap();
    let model = "long-model/".repeat(32);
    for (file, second, value) in [
        ("rollout-z.jsonl", 1, "first"),
        ("rollout-a.jsonl", 2, model.as_str()),
        ("rollout-b.jsonl", 2, "conflicting"),
        ("rollout-c.jsonl", 3, "last"),
    ] {
        let path = root.join(file);
        let context = json!({"type":"turn_context", "ordinal":1,
            "timestamp":format!("2026-01-02T12:00:{second:02}Z"),
            "payload":{"model":value,"model_provider":"supplier-a"}});
        let state = parse(&path, &[header(PARENT, 0), context], cutoff);
        let mut store = Store::open(Connection::open(&database)?)?;
        let prepared = store.prepare(new_source(&path, &state), cutoff, names.clone(), 1 << 16)?;
        store.publish(prepared)?;
        drop(store);
        let mut reader = Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let result = Store::read_response(&mut reader, &query, &request, Some(PARENT), &names)?;
        let expected = if value == "conflicting" { "" } else { value };
        assert_eq!(result["sessionInfo"][PARENT]["model"], expected, "{file}");
    }
    Ok(())
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
fn restored_supplier_evidence_keeps_inference_separate_without_changing_counts()
-> anyhow::Result<()> {
    for source in ["model-namespace", "session-context-inferred"] {
        restored_supplier_evidence_for_source(source)?;
    }
    Ok(())
}

#[test]
fn mixed_supplier_inference_preserves_inferred_flag() {
    use crate::usage_history::structured::ClassificationEvidence as Evidence;
    for (a, b) in [
        (Evidence::NamespaceInferred, Evidence::SessionContextInferred),
        (Evidence::SessionContextInferred, Evidence::NamespaceInferred),
    ] {
        let mut row = json!({});
        a.intersect(b).write_to(&mut row);
        assert_eq!(row, json!({"providerInferred":true}));
        assert_eq!(a.intersect(Evidence::RequestExact), Evidence::None);
    }
}

fn restored_supplier_evidence_for_source(inferred_source: &str) -> anyhow::Result<()> {
    use super::historical::{EvidenceVerifier, Historical};
    use std::sync::Mutex;

    let dir = tempfile::tempdir()?;
    let root = fs::canonicalize(dir.path())?;
    let database = root.join("tokens.sqlite");
    let settings = root.join("settings.json");
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let mut store = Store::open(Connection::open(&database)?)?;
    let mut records = Vec::new();
    for (session, supplier, source) in [
        (PARENT, "custom", inferred_source),
        (CHILD, "custom", "request-log-recovered-supplier-only"),
        (OTHER, "supplier-b", inferred_source),
    ] {
        let path = root.join(format!("rollout-{session}.jsonl"));
        let mut state = parse(&path, &[header(session, 0), count(1, 1, 1, true)], cutoff);
        let event = &mut state.events[0];
        event.model = if inferred_source == "session-context-inferred" {
            "fixture".into()
        } else {
            "supplier-a/fixture".into()
        };
        event.provider = supplier.into();
        event.provider_source = (supplier == "supplier-b").then(|| "request-log-exact".into());
        let prepared = store.prepare(new_source(&path, &state), cutoff, BTreeMap::new(), 8192)?;
        store.publish(prepared)?;
        let event = read(&mut store.db, i64::MIN, i64::MAX)
            .1
            .into_iter()
            .find(|event| event.session == session)
            .unwrap();
        records.push(json!({
            "eventId":event.id.map(|byte| format!("{byte:02x}")).concat(),
            "session":event.session,"timestamp":event.timestamp,"model":event.model,
            "role":event.role,"input":event.delta.input_tokens,"output":event.delta.output_tokens,
            "cached":event.delta.cached_input_tokens,"provider":"supplier-a",
            "providerName":"Supplier A","source":source
        }));
    }
    let query = Query::parse(&json!({}), cutoff).unwrap();
    let request = json!({});
    let names = BTreeMap::new();
    let before = Store::read_response(&mut store.db, &query, &request, None, &names)?;
    fs::write(
        settings.with_file_name("usage-provider-evidence.json"),
        serde_json::to_vec(&json!({"version":1,"records":records}))?,
    )?;
    let with_overlay = |store: &mut Store| -> anyhow::Result<Value> {
        let verifier = Mutex::new(EvidenceVerifier::default());
        let overlay = Historical::refresh(None, store, &verifier, &settings)?.unwrap();
        assert_eq!(overlay.overlay.legacy_status["appliedEvents"], 2);
        let (result, _) = Store::read_response_with_archive(
            &mut store.db,
            &query,
            &request,
            None,
            &names,
            None,
            Some(&overlay),
        )?;
        Ok(result)
    };
    let after = with_overlay(&mut store)?;
    assert_eq!(after["days"][0]["usage"], before["days"][0]["usage"]);
    assert_eq!(after["days"][0]["recordCount"], 3);
    let rows = after["days"][0]["breakdowns"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    assert!(
        rows.iter()
            .any(|row| row["provider"] == "supplier-a" && row["providerInferred"] == true)
    );
    assert!(rows.iter().any(|row| row["provider"] == "supplier-a"
        && row["providerInferred"].is_null()
        && row["supplierClassificationEvidence"] == "request-log-recovered-supplier-only"));
    assert!(rows.iter().any(|row| row["provider"] == "supplier-b"
        && row["classificationEvidence"] == "request-log-exact"));
    drop(store);
    let mut reopened = Store::open(Connection::open(&database)?)?;
    assert_eq!(with_overlay(&mut reopened)?["days"], after["days"]);

    reopened.db.pragma_update(None, "journal_mode", "WAL")?;
    let overlay = Historical::refresh(
        None, &mut reopened, &Mutex::new(EvidenceVerifier::default()), &settings,
    )?.unwrap();
    let reader = Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    reader.execute_batch("BEGIN")?;
    reader.query_row("SELECT revision FROM token_revision", [], |row| row.get::<_, i64>(0))?;
    let path = root.join("rollout-appended.jsonl");
    let state = parse(&path, &[header(PHYSICAL_PREFIX, 0), count(1, 2, 1, true)], cutoff);
    let prepared = reopened.prepare(new_source(&path, &state), cutoff, BTreeMap::new(), 8192)?;
    reopened.publish(prepared)?;

    for request in [&request, &json!({"includeFingerprints":false})] {
        let (during, _) =
            Store::read_snapshot(&reader, &query, request, None, &names, None, Some(&overlay))?;
        assert_eq!(during["days"][0]["recordCount"], 3);
        assert!(during["days"][0]["breakdowns"].as_array().unwrap().iter()
            .any(|row| row["providerInferred"] == true));
    }
    let updated = with_overlay(&mut reopened)?;
    assert_eq!(updated["days"][0]["recordCount"], 4);
    assert_eq!(updated["days"][0]["usage"]["totalTokens"], 440);
    fs::write(settings.with_file_name("usage-provider-evidence.json"), r#"{"version":1,"records":[]}"#)?;
    let (revoked, _) = Store::read_snapshot(&reader, &query, &json!({}), None, &names, None, Some(&overlay))?;
    assert_eq!(revoked["days"][0]["recordCount"], 3);
    assert!(revoked["providerEvidence"].is_null());
    Ok(())
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
fn newly_unidentified_source_withdraws_published_classifications_with_their_days() {
    use super::scan::Scan;

    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let sessions = root.join("sessions");
    fs::create_dir(&sessions).unwrap();
    let db = root.join("index.sqlite");
    let source = sessions.join("rollout-parent.jsonl");
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    parse(&source, &[header(PARENT, 0), count(1, 1, 1, true)], cutoff);
    let mut store = Store::open(Connection::open(&db).unwrap()).unwrap();
    let mut scan = Scan::default();
    assert!(
        scan.round(&mut store, &root, cutoff, &BTreeMap::new(), 8192)
            .complete
    );
    let native = read(&mut store.db, i64::MIN, i64::MAX).1.pop().unwrap();
    let revision = store
        .reclassify_session(PARENT, 8192, &BTreeSet::new(), |events, _| {
            events[0].provider = "relay-proof".into();
            events[0].model = "final-model".into();
            events[0].provider_source = Some("request-log-exact".into());
            Ok(())
        })
        .unwrap()
        .unwrap();
    fs::write(sessions.join("rollout-unidentified.jsonl.zst"), b"unread").unwrap();
    store
        .db
        .execute_batch(
            "CREATE TRIGGER fail_day BEFORE UPDATE ON token_days
        BEGIN SELECT RAISE(ABORT,'failed day write'); END;",
        )
        .unwrap();
    let failed = scan.round(&mut store, &root, cutoff, &BTreeMap::new(), 8192);
    assert!(
        failed
            .maintenance_error
            .unwrap()
            .to_string()
            .contains("failed day write")
    );
    let (after_failed, kept) = read(&mut store.db, i64::MIN, i64::MAX);
    assert_eq!(after_failed, revision);
    assert_eq!(kept[0].provider, "relay-proof");
    store.db.execute_batch("DROP TRIGGER fail_day").unwrap();
    let report = scan.round(&mut store, &root, cutoff, &BTreeMap::new(), 8192);
    assert_eq!((report.pending_sources, report.issues), (0, 1));
    let (next, restored) = read(&mut store.db, i64::MIN, i64::MAX);
    assert!(next > revision);
    assert_eq!(restored[0].id, native.id);
    assert_eq!(restored[0].provider, native.provider);
    assert_eq!(restored[0].provider_source, native.provider_source);
    assert_eq!(restored[0].model, native.model);
    assert_eq!(restored[0].delta.value(), native.delta.value());
    drop(store);
    let mut store = Store::open(Connection::open(&db).unwrap()).unwrap();
    let report = Scan::default().round(&mut store, &root, cutoff, &BTreeMap::new(), 8192);
    assert_eq!((report.pending_sources, report.issues), (0, 1));
    let query = Query::parse(&json!({}), cutoff).unwrap();
    let view = Store::read_selection(&mut store.db, &query, None, None).unwrap();
    assert_eq!(view.revision, next);
    let day = &view.days[0];
    assert_eq!(day["recordCount"], 1);
    assert_eq!(day["usage"]["totalTokens"], 110);
    assert_ne!(day["breakdowns"][0]["provider"], "relay-proof");
    assert!(day["breakdowns"][0].get("classificationEvidence").is_none());
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
fn durable_future_partial_request_needs_ordered_gateway_and_paired_native_response()
-> anyhow::Result<()> {
    use super::scan::Scan;
    use crate::usage_history::structured::{RequestEvidence, RequestEvidenceCache, provider_cache};
    let dir = tempfile::tempdir()?;
    let root = dir.path().canonicalize()?;
    fs::create_dir(root.join("sessions"))?;
    let cutoff = Local::now().date_naive().pred_opt().unwrap();
    let at = Local::now().timestamp_millis();
    let time = |delta| {
        DateTime::from_timestamp_millis(at + delta)
            .unwrap()
            .to_rfc3339()
    };
    let mut head = header(PARENT, 0);
    head["payload"]["model_provider"] = json!("custom");
    head["timestamp"] = json!(time(-60_000));
    let context = json!({"type":"turn_context","timestamp":time(-50_000),"ordinal":1,"payload":{"turn_id":"t","model":"gpt-fixture"}});
    let mut counter = count(3, 0, 1, true);
    counter["timestamp"] = json!(time(0));
    let usage = &counter["payload"]["info"]["last_token_usage"];
    let receipt = json!({"type":"token_usage_record","ordinal":2,"timestamp":time(-1),"payload":{"thread_id":PARENT,"turn_id":"t","response_id":"id","usage":usage,"thread_token_usage":usage}});
    parse(
        &root.join("sessions/rollout-ref.jsonl"),
        &[head, context, receipt, counter],
        cutoff,
    );
    let db = root.join("index.sqlite");
    let mut store = Store::open(Connection::open(&db)?)?;
    assert!(
        Scan::default()
            .round(&mut store, &root, cutoff, &BTreeMap::new(), 1 << 20)
            .complete
    );
    let before = read(&mut store.db, at, at + 1).1.remove(0);
    drop(store);
    let mut proof_db = provider_cache::ProviderCache::default();
    proof_db.open(&root.join("requests"), cutoff)?;
    for (start, end, proof_end, response, expected) in [
        (102, 12_309, 5, "id", true),
        (-24_696, -12_489, 5, "id", false),
        (0, 12_309, 5, "id", false),
        (1, 12_309, 5, "id", false),
        (102, 12_309, 5, "other", false),
        (12_310, 12_309, 5, "id", false),
        (102, 12_309, -60_000, "id", false),
    ] {
        let mut store = Store::open(Connection::open(&db)?)?;
        let proof: RequestEvidence = serde_json::from_value(
            json!({"id":"complete","threadId":PARENT,"model":"gpt-fixture","upstreamModel":"gpt-fixture","relayId":"proved","success":true,"state":"success","statusCode":200,
            "timestampMs":at+proof_end,"startedAtMs":at-70_000,"attempts":1,"autoRetries":0,"finalResponseEvidence":"final-response-v1","finalResponseId":response,
            "usage":{"inputTokens":100,"cachedTokens":20,"outputTokens":10}}),
        )?;
        let mut other = proof.clone();
        other.id = "partial".into();
        other.started_at_ms = if start == 0 { 0 } else { at + start };
        other.timestamp_ms = at + end;
        other.final_response_id = None;
        other.final_response_evidence = None;
        other.usage = None;
        let mut evidence = RequestEvidenceCache {
            records: vec![proof, other],
            ..Default::default()
        };
        store.reclassify_window(
            PARENT,
            Some(at..at + 1),
            1 << 20,
            &BTreeSet::new(),
            0,
            |events, aliases, scope| {
                evidence.attribute_with_cache_scoped(
                    events,
                    &mut BTreeMap::new(),
                    aliases,
                    &proof_db,
                    Some(scope),
                );
                assert!(evidence.captured_id_check_error.is_none());
                Ok(())
            },
        )?;
        drop(store);
        let after = read(&mut Connection::open(&db)?, at, at + 1).1.remove(0);
        assert_eq!(
            after.provider == "proved",
            expected,
            "start={start} end={end} proof={proof_end} id={response}"
        );
        assert_eq!((after.id, after.delta), (before.id, before.delta));
    }
    Ok(())
}

#[test]
fn bounded_classification_preserves_day_totals_and_counts_competitors_across_the_boundary() {
    use super::scan::Scan;
    use crate::usage_history::structured::{RequestEvidence, RequestEvidenceCache, provider_cache};

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::create_dir(root.join("sessions")).unwrap();
    let source = root.join("sessions/rollout-81.jsonl");
    let day = Local::now().date_naive();
    let cutoff = day.pred_opt().unwrap();
    let point = day
        .and_hms_opt(12, 0, 0)
        .unwrap()
        .and_local_timezone(Local)
        .single()
        .unwrap();
    let at = |seconds| point + chrono::TimeDelta::seconds(seconds);
    let mut head = header(PARENT, 0);
    head["timestamp"] = json!(at(-4000).to_rfc3339());
    head["payload"]["model_provider"] = json!("custom");
    let context = json!({"type":"turn_context","ordinal":1,"timestamp":at(-4000).to_rfc3339(),
        "payload":{"turn_id":"turn","model":"gpt-fixture"}});
    let counter = |index: usize, seconds, input| {
        let mut row = count(index + 1, 0, input, true);
        row["timestamp"] = json!(at(seconds).to_rfc3339());
        row
    };
    let mut rows = vec![head, context];
    rows.extend((1..=80).map(|i| counter(i, -3800 + i as i64, i as u64)));
    rows.extend([counter(81, -230, 81), counter(82, 0, 82)]);
    parse(&source, &rows, cutoff);
    let mut store = Store::open(Connection::open_in_memory().unwrap()).unwrap();
    let names = BTreeMap::new();
    let gaps = BTreeSet::new();
    assert!(
        Scan::default()
            .round(&mut store, &root, cutoff, &names, 1 << 20)
            .complete
    );
    const LIMIT: usize = 8192;
    let rejected = store
        .reclassify_session(PARENT, LIMIT, &gaps, |_, _| {
            panic!("the full session should exceed the fixture budget")
        })
        .unwrap_err();
    assert!(rejected.to_string().contains("exceeds input budget"));

    let mut proof_db = provider_cache::ProviderCache::default();
    proof_db.open(&root.join("proofs"), cutoff).unwrap();
    let request = |id, relay, seconds| -> RequestEvidence {
        let time = at(seconds).timestamp_millis();
        serde_json::from_value(json!({"id":id,"threadId":PARENT,"model":"gpt-fixture",
            "upstreamModel":"gpt-fixture","relayId":relay,"success":true,"state":"success",
            "statusCode":200,"timestampMs":time,"startedAtMs":time - 100,"attempts":1,
            "autoRetries":0,"usage":{"inputTokens":100,"cachedTokens":20,"outputTokens":10}
        }))
        .unwrap()
    };
    let mut evidence = RequestEvidenceCache::default();
    let inspect = |store: &mut Store| {
        read(
            &mut store.db,
            at(-240).timestamp_millis(),
            at(1).timestamp_millis(),
        )
        .1
    };
    let native = inspect(&mut store);
    assert_eq!(native.len(), 2);
    assert_ne!(native[1].provider, "recent-relay");
    let classify = |store: &mut Store, evidence: &mut RequestEvidenceCache, seconds| {
        let start = at(seconds).timestamp_millis();
        store
            .reclassify_window(
                PARENT,
                Some(start..start + 1),
                LIMIT,
                &gaps,
                0,
                |events, aliases, scope| {
                    assert_eq!(scope.targets.len(), 1);
                    assert_eq!(events.len(), 2);
                    evidence.attribute_with_cache_scoped(
                        events,
                        &mut BTreeMap::new(),
                        aliases,
                        &proof_db,
                        Some(scope),
                    );
                    Ok(())
                },
            )
            .unwrap()
    };
    let neighbor = request("neighbor", "neighbor-relay", -230);
    evidence.records = vec![neighbor.clone()];
    assert!(classify(&mut store, &mut evidence, -230).is_some());
    assert_eq!(inspect(&mut store)[0].provider, "neighbor-relay");
    evidence.records = vec![
        neighbor.clone(),
        request("other-neighbor", "another-relay", -230),
        request("recent", "recent-relay", 0),
    ];
    assert!(classify(&mut store, &mut evidence, 0).is_some());
    let assigned = inspect(&mut store);
    assert_eq!(assigned[0].provider, "neighbor-relay");
    assert_eq!(assigned[1].provider, "recent-relay");
    assert!(!evidence.verified[&native[0].id].rejected);

    let mut disputed = RequestEvidenceCache {
        records: vec![request("ambiguous", "ambiguous-relay", -115)],
        ..Default::default()
    };
    assert!(classify(&mut store, &mut disputed, 0).is_some());
    let result = inspect(&mut store);
    assert_eq!(result[0].provider, "neighbor-relay");
    assert_eq!(result[1].provider, native[1].provider);
    assert!(disputed.verified.is_empty());
    assert_eq!(
        result
            .iter()
            .map(|e| (e.id, e.delta.total_tokens))
            .collect::<Vec<_>>(),
        native
            .iter()
            .map(|e| (e.id, e.delta.total_tokens))
            .collect::<Vec<_>>()
    );
    let request = json!({"from":day.to_string(),"to":day.to_string(),"includeFingerprints":false});
    let query = Query::parse(&request, day).unwrap();
    let summary = Store::read_response(&mut store.db, &query, &request, None, &names).unwrap();
    assert_eq!(summary["days"][0]["recordCount"], 82);
    assert_eq!(summary["days"][0]["usage"]["totalTokens"], 82 * 110);
    let oldest = cutoff
        .and_hms_opt(0, 5, 0)
        .unwrap()
        .and_local_timezone(Local)
        .earliest()
        .unwrap()
        .timestamp_millis();
    assert!(
        store
            .reclassify_window(
                PARENT,
                Some(oldest..oldest + 1),
                LIMIT,
                &gaps,
                0,
                |_, _, _| { panic!("cannot accept a clipped retention boundary") }
            )
            .unwrap()
            .is_none()
    );
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
