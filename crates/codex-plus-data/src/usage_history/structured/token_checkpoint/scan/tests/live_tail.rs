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
fn older_open_logs_publish_concurrent_appends_while_history_advances() -> anyhow::Result<()> {
    use std::time::{Duration, UNIX_EPOCH};

    for cold in [false, true] {
        let dir = tempfile::tempdir()?;
        let root = dir.path().canonicalize()?;
        let sessions = root.join("sessions");
        fs::create_dir(&sessions)?;
        let old = UNIX_EPOCH + Duration::from_secs(1_770_000_000);
        let mut writers = Vec::new();
        for i in 0..12 {
            let session = format!("00000000-0000-4000-8000-{:012}", i + 100);
            let source = sessions.join(format!("rollout-{session}.jsonl"));
            let mut file = fs::File::create(&source)?;
            file.write_all(rollout(&session).as_bytes())?;
            if i < 3 {
                file.set_modified(old)?;
                writers.push((source, file));
            } else {
                file.set_modified(old + Duration::from_secs(86_400))?;
            }
        }
        let mut store = Store::open(Connection::open_in_memory()?)?;
        let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let mut scan = Scan::default();
        assert!(scan.round(&mut store, &root, cutoff, &BTreeMap::new(), LIMIT).complete);
        if cold {
            scan = Scan::default();
        }
        start_archive(&root, &mut scan, &mut store, cutoff)?;
        let before = scan.active.as_ref().unwrap().state.offset;
        for (index, (_, file)) in writers.iter_mut().enumerate() {
            for n in 1..=[8, 13, 12][index] {
                let mut count: serde_json::Value = serde_json::from_str(&next_count())?;
                count["timestamp"] = json!(format!("2026-01-02T12:02:{n:02}Z"));
                count["payload"]["info"]["total_token_usage"]["input_tokens"] = json!(100 + 102 * n);
                writeln!(file, "{count}")?;
            }
            file.set_modified(old)?;
        }
        let round = scan.round(&mut store, &root, cutoff, &BTreeMap::new(), LIMIT);
        assert!(round.errors.is_empty(), "{:?}", round.errors);
        for (path, file) in &writers {
            assert_eq!(store.checkpoint(path)?.unwrap().1.offset, file.metadata()?.len(), "cold={cold}");
        }
        assert!(scan.active.as_ref().unwrap().state.offset > before);
        let mut total = 0;
        Store::read_committed(&mut store.db, i64::MIN, i64::MAX, |event| {
            total += usize::from(event.delta.input_tokens == 102);
            Ok(())
        })?;
        assert_eq!(total, 33);
    }
    Ok(())
}

#[test]
fn concurrent_appends_beyond_one_round_stay_pending_and_rotate() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let root = dir.path().canonicalize()?;
    let sessions = root.join("sessions");
    fs::create_dir(&sessions)?;
    let mut sources = Vec::new();
    for i in 0..SOURCES_PER_ROUND + 3 {
        let session = format!("00000000-0000-4000-8000-{:012}", i + 100);
        let path = sessions.join(format!("rollout-{session}.jsonl"));
        fs::write(&path, rollout(&session))?;
        sources.push(path);
    }
    let mut store = Store::open(Connection::open_in_memory()?)?;
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let names = BTreeMap::new();
    let mut scan = Scan::default();
    scan.round(&mut store, &root, cutoff, &names, LIMIT);
    assert!(scan.round(&mut store, &root, cutoff, &names, LIMIT).complete);
    start_archive(&root, &mut scan, &mut store, cutoff)?;
    for source in &sources {
        writeln!(fs::OpenOptions::new().append(true).open(source)?, "{}", next_count())?;
    }
    let round = scan.round(&mut store, &root, cutoff, &names, LIMIT);
    assert!(round.errors.is_empty(), "{:?}", round.errors);
    let pending: Vec<_> = sources.iter().filter(|source| scan.remaining.contains(source)).collect();
    assert_eq!(pending.len(), sources.len() - (SOURCES_PER_ROUND - 1));
    assert!(pending.iter().all(|source| !scan.sealed_queue.contains(*source)));
    // 已得到名额的源再次增长，尚未读取的尾部仍然先得到下一轮名额。
    for source in &sources {
        writeln!(fs::OpenOptions::new().append(true).open(source)?, "{{\"type\":\"ignored\"}}")?;
    }
    let round = scan.round(&mut store, &root, cutoff, &names, LIMIT);
    assert!(round.errors.is_empty(), "{:?}", round.errors);
    for source in pending {
        assert_eq!(store.checkpoint(source)?.unwrap().1.offset, source.metadata()?.len());
    }
    Ok(())
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
fn unfinished_line_allows_oversized_prefix_to_retry_its_write() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let root = fs::canonicalize(dir.path())?;
    let sessions = root.join("sessions");
    fs::create_dir(&sessions)?;
    let source = sessions.join(format!("rollout-{FIRST}.jsonl"));
    fs::write(&source, rollout(FIRST))?;
    let mut store = Store::open(Connection::open_in_memory()?)?;
    let mut scan = Scan::default();
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let names = BTreeMap::new();
    assert!(scan.round(&mut store, &root, cutoff, &names, LIMIT).complete);
    let previous = store.checkpoint(&source)?.unwrap().1.offset;
    let mut file = fs::OpenOptions::new().append(true).open(&source)?;
    writeln!(file, "{}", next_count())?;
    let boundary = file.metadata()?.len();
    file.write_all(b"{")?;
    let deferred = scan.round(&mut store, &root, cutoff, &names, 128);
    assert!(deferred.errors.is_empty() && !deferred.complete);
    assert_eq!(store.checkpoint(&source)?.unwrap().1.offset, previous);

    store.db.execute_batch("CREATE TEMP TRIGGER block_prefix BEFORE INSERT ON token_days
        BEGIN SELECT RAISE(ABORT, 'blocked prefix'); END;")?;
    let rejected = scan.round(&mut store, &root, cutoff, &names, 128);
    assert!(rejected.errors.iter().any(|(_, error)| error.to_string().contains("blocked prefix")));
    assert_eq!(store.checkpoint(&source)?.unwrap().1.offset, previous);
    store.db.execute_batch("DROP TRIGGER block_prefix")?;
    let recovered = scan.round(&mut store, &root, cutoff, &names, 128);
    assert!(recovered.errors.is_empty() && !recovered.complete);
    assert_eq!(store.checkpoint(&source)?.unwrap().1.offset, boundary);
    assert!(scan.waiting_for_newline.contains_key(&source));
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
