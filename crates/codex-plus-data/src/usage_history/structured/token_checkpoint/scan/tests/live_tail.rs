use super::*;
use std::io::Write;

const LIMIT: usize = 32 << 20;

fn start_archive(
    root: &Path,
    scan: &mut Scan,
    store: &mut Store,
    cutoff: NaiveDate,
) -> anyhow::Result<()> {
    let archive = root.join("archived_sessions");
    fs::create_dir(&archive)?;
    let mut writer = zstd::stream::write::Encoder::new(
        fs::File::create(archive.join("rollout-history.jsonl.zst"))?,
        1,
    )?;
    writer.write_all(rollout(SECOND).as_bytes())?;
    let padding = format!(
        "{{\"type\":\"ignored\",\"payload\":\"{}\"}}\n",
        "x".repeat(65536)
    );
    for _ in 0..320 {
        writer.write_all(padding.as_bytes())?;
    }
    writer.finish()?;
    let report = scan.round(store, root, cutoff, &BTreeMap::new(), LIMIT);
    assert!(!report.complete && report.errors.is_empty());
    assert!(scan.active.is_some());
    Ok(())
}

fn next_count() -> String {
    json!({"type":"event_msg","timestamp":"2026-01-02T12:02:00Z",
    "payload":{"type":"token_count","info":{
        "total_token_usage":{"input_tokens":202,"cached_input_tokens":20,"output_tokens":10},
        "last_token_usage":{"input_tokens":102,"cached_input_tokens":0,"output_tokens":0}
    }}})
    .to_string()
}

#[test]
fn parked_reader_charges_only_retained_state_after_publishing() -> anyhow::Result<()> {
    use crate::usage_history::structured::index_budget::{Budget, CURSOR_BYTES, Meter};

    let dir = tempfile::tempdir()?;
    let source = fs::canonicalize(dir.path())?.join(format!("rollout-{FIRST}.jsonl"));
    let prefix = rollout(FIRST);
    fs::write(&source, format!("{prefix}{}\n", next_count()))?;
    let mut store = Store::open(Connection::open_in_memory()?)?;
    let mut reader = SourceReader::restore(&store, &source)?;
    let mut budget = Budget::new(std::iter::empty());
    budget.read_left = prefix.len() as u64 + 1;
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    assert!(!reader.step(&mut store, cutoff, &mut budget, &BTreeMap::new(), LIMIT)?);
    assert!(reader.has_open_reader() && reader.state.events.is_empty());
    assert_eq!(
        store.checkpoint(&source)?.unwrap().1.offset,
        prefix.len() as u64
    );
    let charge = reader.state.retained_charge;
    Meter::new(&reader.state).refresh(&mut reader.state, CURSOR_BYTES);
    assert_eq!(charge, reader.state.retained_charge);
    assert!(reader.can_yield_plain_batch());
    drop(reader);
    let mut resumed = SourceReader::restore(&store, &source)?;
    assert!(resumed.step(
        &mut store,
        cutoff,
        &mut Budget::new(std::iter::empty()),
        &BTreeMap::new(),
        LIMIT
    )?);
    let mut deltas = Vec::new();
    Store::read_committed(&mut store.db, i64::MIN, i64::MAX, |event| {
        deltas.push(event.delta.input_tokens);
        Ok(())
    })?;
    assert_eq!(deltas, [100, 102]);
    Ok(())
}

#[test]
fn incomplete_live_line_waits_for_one_newline_without_accumulating_issues() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let root = fs::canonicalize(dir.path())?;
    fs::create_dir(root.join("sessions"))?;
    let source = root.join("sessions").join(format!("rollout-{FIRST}.jsonl"));
    fs::write(&source, rollout(FIRST))?;
    let mut store = Store::open(Connection::open_in_memory()?)?;
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let names = BTreeMap::new();
    let mut scan = Scan::default();
    assert!(
        scan.round(&mut store, &root, cutoff, &names, LIMIT)
            .complete
    );
    let before = store.checkpoint(&source)?.unwrap().1.offset;
    start_archive(&root, &mut scan, &mut store, cutoff)?;
    let mut file = fs::OpenOptions::new().append(true).open(&source)?;
    write!(file, "{}", next_count())?;
    for _ in 0..8 {
        let report = scan.round(&mut store, &root, cutoff, &names, LIMIT);
        assert!(!report.complete && report.pending_sources > 0);
        assert_eq!(report.issues, 0, "{:?}", report.errors);
        assert_eq!(store.checkpoint(&source)?.unwrap().1.offset, before);
    }
    writeln!(file)?;
    for _ in 0..3 {
        let report = scan.round(&mut store, &root, cutoff, &names, LIMIT);
        assert_eq!(report.issues, 0, "{:?}", report.errors);
        if report.complete {
            break;
        }
    }
    assert!(
        scan.round(&mut store, &root, cutoff, &names, LIMIT)
            .complete
    );
    assert_eq!(
        store.checkpoint(&source)?.unwrap().1.offset,
        file.metadata()?.len()
    );
    let mut deltas = Vec::new();
    Store::read_committed(&mut store.db, i64::MIN, i64::MAX, |event| {
        if event.session == FIRST {
            deltas.push(event.delta.input_tokens);
        }
        Ok(())
    })?;
    assert_eq!(deltas, [100, 102]);
    Ok(())
}

#[test]
fn unchanged_live_tail_preserves_its_malformed_count_diagnostic() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let root = fs::canonicalize(dir.path())?;
    let sessions = root.join("sessions");
    fs::create_dir(&sessions)?;
    let source = sessions.join(format!("rollout-{FIRST}.jsonl"));
    let malformed = json!({"type":"event_msg","timestamp":"2026-01-02T12:02:00Z",
        "payload":{"type":"token_count","info":{"total_token_usage":"bad count"}}});
    fs::write(&source, format!("{}{malformed}\n{{", rollout(FIRST)))?;
    let mut store = Store::open(Connection::open_in_memory()?)?;
    let mut scan = Scan::default();
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    for _ in 0..3 {
        let report = scan.round(&mut store, &root, cutoff, &BTreeMap::new(), LIMIT);
        assert!(!report.complete);
        assert_eq!(report.issues, 1);
        assert!(scan.scoped_count_gaps().contains(&source));
    }
    Ok(())
}

#[test]
fn moved_live_append_recovers_after_a_failed_publication() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let root = fs::canonicalize(dir.path())?;
    fs::create_dir(root.join("sessions"))?;
    let filename = format!("rollout-{FIRST}.jsonl");
    let source = root.join("sessions").join(&filename);
    let moved = root.join("archived_sessions").join(filename);
    fs::write(&source, rollout(FIRST))?;
    let mut store = Store::open(Connection::open_in_memory()?)?;
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let names = BTreeMap::new();
    let mut scan = Scan::default();
    assert!(
        scan.round(&mut store, &root, cutoff, &names, LIMIT)
            .complete
    );
    let (revision, before) = store.checkpoint(&source)?.unwrap();
    start_archive(&root, &mut scan, &mut store, cutoff)?;
    store.db.execute_batch(&format!(
        "CREATE TRIGGER fail_append BEFORE UPDATE ON token_sources WHEN OLD.revision={revision}
         BEGIN SELECT RAISE(ABORT, 'rejected live append'); END;"
    ))?;
    writeln!(
        fs::OpenOptions::new().append(true).open(&source)?,
        "{}",
        next_count()
    )?;
    let failed = scan.round(&mut store, &root, cutoff, &names, LIMIT);
    assert!(failed.errors.iter().any(|(path, error)| {
        path == &source && error.to_string().contains("rejected live append")
    }));
    assert_eq!(store.checkpoint(&source)?.unwrap().1.offset, before.offset);
    store.db.execute_batch("DROP TRIGGER fail_append")?;
    fs::rename(&source, &moved)?;
    for _ in 0..12 {
        if scan
            .round(&mut store, &root, cutoff, &names, LIMIT)
            .complete
        {
            break;
        }
    }
    assert!(
        scan.round(&mut store, &root, cutoff, &names, LIMIT)
            .complete
    );
    assert!(store.checkpoint(&source)?.is_none());
    assert_eq!(
        store.checkpoint(&moved)?.unwrap().1.offset,
        moved.metadata()?.len()
    );
    let mut deltas = Vec::new();
    Store::read_committed(&mut store.db, i64::MIN, i64::MAX, |event| {
        if event.session == FIRST {
            deltas.push(event.delta.input_tokens);
        }
        Ok(())
    })?;
    assert_eq!(deltas, [100, 102]);
    Ok(())
}
