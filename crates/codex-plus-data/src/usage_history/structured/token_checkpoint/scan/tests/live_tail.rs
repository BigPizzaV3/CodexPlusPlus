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
fn occupied_reader_slots_defer_live_sources_without_reporting_corruption() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let root = dir.path().canonicalize()?;
    let sessions = root.join("sessions");
    fs::create_dir(&sessions)?;
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let names = BTreeMap::new();
    let live = sessions.join(format!("rollout-{FIRST}.jsonl"));
    fs::write(&live, rollout(FIRST))?;
    let mut store = Store::open(Connection::open_in_memory()?)?;
    let mut scan = Scan::default();
    assert!(scan.round(&mut store, &root, cutoff, &names, LIMIT).complete);
    let padding = format!("{{\"type\":\"ignored\",\"payload\":\"{}\"}}\n", "x".repeat(65536));
    for (i, slot) in [&mut scan.active, &mut scan.deferred].into_iter().enumerate() {
        let path = root.join(format!("history-{i}.jsonl"));
        fs::write(&path, format!("{}{}", rollout(SECOND), padding.repeat(96)))?;
        let mut reader = SourceReader::restore(&store, &path)?;
        let mut budget = Budget::new(std::iter::empty());
        budget.read_left = 65536;
        assert!(!reader.step(&mut store, cutoff, &mut budget, &names, LIMIT)?);
        assert!(reader.has_open_reader());
        *slot = Some(reader);
    }
    writeln!(fs::OpenOptions::new().append(true).open(&live)?, "{}", next_count())?;
    let report = scan.round(&mut store, &root, cutoff, &names, LIMIT);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(report.issues, 0, "waiting for a reader is not a source error");
    assert!(scan.remaining.contains(&live), "the deferred append must remain queued");
    for _ in 0..16 {
        let report = scan.round(&mut store, &root, cutoff, &names, LIMIT);
        assert_eq!(report.issues, 0);
        if store.checkpoint(&live)?.unwrap().1.offset == live.metadata()?.len() {
            return Ok(());
        }
    }
    anyhow::bail!("deferred live append did not resume")
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
fn legacy_prefix_recheck_leaves_the_live_slot_available_for_other_appends() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let root = dir.path().canonicalize()?;
    let sessions = root.join("sessions");
    fs::create_dir(&sessions)?;
    let first = sessions.join(format!("rollout-{FIRST}.jsonl"));
    let second = sessions.join(format!("rollout-{SECOND}.jsonl"));
    let padding = format!("{{\"type\":\"ignored\",\"payload\":\"{}\"}}\n", "x".repeat(65536));
    fs::write(&first, format!("{}{}", rollout(FIRST), padding.repeat(512)))?;
    fs::write(&second, format!("{}{}", rollout(SECOND), padding.repeat(160)))?;
    let mut store = Store::open(Connection::open_in_memory()?)?;
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let names = BTreeMap::new();
    let mut scan = Scan::default();
    assert!((0..32).any(|_| scan.round(&mut store, &root, cutoff, &names, LIMIT).complete));
    store.db.execute(
        "UPDATE token_sources SET checkpoint=CAST(json_set(CAST(checkpoint AS TEXT),
         '$.legacy_identity_prefix',offset) AS BLOB)",
        [],
    )?;
    writeln!(fs::OpenOptions::new().append(true).open(&first)?, "{}", next_count())?;
    let mut scan = Scan::default();
    start_archive(&root, &mut scan, &mut store, cutoff)?;
    assert_eq!(store.checkpoint(&first)?.unwrap().1.offset, first.metadata()?.len());
    let probing = scan.round(&mut store, &root, cutoff, &names, LIMIT);
    assert!(probing.errors.is_empty() && !probing.complete);
    writeln!(fs::OpenOptions::new().append(true).open(&second)?, "{}", next_count())?;
    let appended = scan.round(&mut store, &root, cutoff, &names, LIMIT);
    assert!(appended.errors.is_empty(), "{:?}", appended.errors);
    assert_eq!(store.checkpoint(&second)?.unwrap().1.offset, second.metadata()?.len());
    assert!(scan.active.as_ref().is_some_and(|reader| reader.source == second)
        || scan.remaining.front() == Some(&second),
        "a closed live prefix must precede unrelated queued history");
    for _ in 0..24 {
        if scan.round(&mut store, &root, cutoff, &names, LIMIT).complete { break; }
    }
    assert_eq!(store.checkpoint(&first)?.unwrap().1.legacy_identity_prefix, 0);
    Ok(())
}

#[test]
fn legacy_prefix_attribution_follows_only_the_linked_sessions() -> anyhow::Result<()> {
    use crate::usage_history::structured::token_checkpoint::store::projection::SourceUpdate;

    const PHYSICAL: &str = "00000000-0000-4000-8000-000000000071";
    const CHILD: &str = "00000000-0000-4000-8000-000000000072";
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let names = BTreeMap::new();
    let mut divergences = Vec::new();
    for case in [
        "same-session",
        "history-base",
        "tools-only",
        "replaced",
        "missing",
        "replaced-during-probe",
        "unknown-header",
        "partial-line",
    ] {
        let dir = tempfile::tempdir()?;
        let root = dir.path().canonicalize()?;
        let sessions = root.join("sessions");
        fs::create_dir(&sessions)?;
        let parent = sessions.join(format!("rollout-{PHYSICAL}.jsonl"));
        let parent_rows: Vec<_> = rollout(FIRST)
            .lines()
            .enumerate()
            .map(|(i, line)| {
                let mut row: serde_json::Value = serde_json::from_str(line).unwrap();
                row["ordinal"] = json!(i);
                format!("{row}\n")
            })
            .collect();
        fs::write(&parent, parent_rows.concat())?;
        fs::write(
            sessions.join(format!("rollout-{SECOND}.jsonl")),
            rollout(SECOND),
        )?;
        let source = sessions.join(format!("rollout-fixture-{CHILD}.jsonl"));
        let owner = if case == "history-base" { CHILD } else { FIRST };
        let mut meta = json!({"type":"session_meta","ordinal":2,"timestamp":"2026-01-02T12:02:00Z",
            "payload":{"id":owner,"model_provider":"custom"}});
        if case == "history-base" {
            meta["payload"]["history_base"] = json!({"thread_id":PHYSICAL,
                "end_ordinal_exclusive":2,"end_byte_offset":parent.metadata()?.len()});
        }
        let context = json!({"type":"turn_context","ordinal":3,"timestamp":"2026-01-02T12:02:01Z",
            "payload":{"turn_id":"tail","model":"fixture"}});
        let padding = format!(
            "{{\"type\":\"ignored\",\"payload\":\"{}\"}}\n",
            "x".repeat(65536)
        );
        let long_id = "r".repeat(257);
        let counts = json!({"input_tokens":125,"cached_input_tokens":25,"output_tokens":12});
        let terminal = json!({"type":"token_usage_record","timestamp":"2026-01-02T12:03:00Z",
            "payload":{"thread_id":owner,"turn_id":"tail","response_id":long_id,
            "usage":counts,"thread_token_usage":counts}});
        let tools_only = case == "tools-only";
        let mut content = format!("{meta}\n{context}\n{}", padding.repeat(145));
        if tools_only {
            let tool = json!({"type":"response_item","timestamp":"2026-01-02T12:03:00Z",
                "payload":{"type":"function_call","name":"exec_command","call_id":"large",
                "arguments": "a".repeat(1100 << 10)}});
            content.push_str(&format!("{tool}\n"));
        } else {
            content.push_str(&format!("{terminal}\n"));
        }
        if case == "unknown-header" {
            let header = json!({"type":"session_meta","timestamp":"2026-01-02T12:04:00Z",
                "payload":{"id":SECOND,"padding":"x".repeat(1100 << 10)}});
            content.push_str(&format!("{header}\n"));
        }
        fs::write(&source, content)?;
        let mut old = FileState::default();
        while !old.read_with_budget(&source, cutoff, &mut Budget::new(std::iter::empty()))? {}
        if case == "unknown-header" {
            // 当前解析器读出第二个会话；旧解析器跳过了整个超长头，只留下非计数诊断。
            assert_eq!(old.session, FIRST);
            assert!(old.identity_conflict && old.invalid_counts && old.issues == 1);
            old.identity_conflict = false;
            old.invalid_counts = false;
            old.issues = 0;
        }
        assert_eq!(old.issues, 0);
        if !tools_only {
            assert!(
                old.events
                    .iter()
                    .any(|event| event.terminal_response.as_deref() == Some(&long_id))
            );
            old.events.retain(|event| event.terminal_response.is_none());
        }
        let mut store = Store::open(Connection::open_in_memory()?)?;
        let update = SourceUpdate {
            source: &source,
            state: &old,
            previous_revision: None,
            replace: false,
        };
        let prepared = store.prepare(update, cutoff, names.clone(), LIMIT)?;
        store.publish(prepared)?;
        let mut scan = Scan::default();
        assert!(
            scan.round(&mut store, &root, cutoff, &names, LIMIT)
                .complete
        );
        for session in [FIRST, SECOND] {
            store
                .reclassify_session(session, LIMIT, &BTreeSet::new(), |events, _| {
                    for event in events {
                        event.provider = "verified-relay".into();
                        event.provider_source = Some("request-log-exact".into());
                    }
                    Ok(())
                })?
                .unwrap();
        }
        let exact_sessions = |store: &mut Store| -> anyhow::Result<BTreeSet<String>> {
            let mut exact = BTreeSet::new();
            Store::read_committed(&mut store.db, i64::MIN, i64::MAX, |event| {
                if event.provider_source.as_deref() == Some("request-log-exact") {
                    exact.insert(event.session);
                }
                Ok(())
            })?;
            Ok(exact)
        };
        let both = BTreeSet::from([FIRST.to_owned(), SECOND.to_owned()]);
        assert_eq!(exact_sessions(&mut store)?, both);
        let (rev, _) = store.checkpoint(&source)?.unwrap();
        if tools_only {
            store.db.execute(
                "UPDATE token_sources SET checkpoint=CAST(json_set(checkpoint,
                '$.activity_truncated',json('true'),'$.large_tools_checked',json('false')) AS BLOB)
                WHERE revision=?",
                [rev],
            )?;
        } else {
            store.db.execute(
                "UPDATE token_sources SET checkpoint=CAST(json_set(json_remove(checkpoint,
                '$.legacy_identity_prefix'),'$.issues',?) AS BLOB) WHERE revision=?",
                rusqlite::params![usize::from(case == "unknown-header"), rev],
            )?;
        }
        if case == "partial-line" {
            let mut file = fs::OpenOptions::new().append(true).open(&source)?;
            file.write_all(b"{\"type\":\"session_meta\",\"payload\":{\"id\":\"")?;
        }
        if case == "replaced" || case == "missing" {
            fs::rename(&source, root.join("old.jsonl"))?;
            let path = if case == "replaced" {
                source.clone()
            } else {
                sessions.join("rollout-new.jsonl")
            };
            // 新来源的会话头位于首批数据之后，扫描器尚不能断言其归属范围。
            fs::write(path, format!("{}{}", padding.repeat(145), rollout(SECOND)))?;
        }
        let mut scan = Scan::default();
        let report = scan.round(&mut store, &root, cutoff, &names, LIMIT);
        assert!(
            !report.complete && report.errors.is_empty(),
            "{case}: {:?}",
            report.errors
        );
        let expected = match case {
            "tools-only" | "same-session" | "history-base" | "replaced-during-probe" => both,
            "missing" => BTreeSet::from([SECOND.to_owned()]),
            "replaced" | "unknown-header" | "partial-line" => BTreeSet::new(),
            _ => unreachable!("{case}"),
        };
        let actual = exact_sessions(&mut store)?;
        if actual != expected {
            divergences.push((case, expected, actual));
        }
        if case == "replaced-during-probe" {
            fs::rename(&source, root.join("old.jsonl"))?;
            fs::write(&source, format!("{}{}", padding.repeat(145), rollout(SECOND)))?;
            let report = scan.round(&mut store, &root, cutoff, &names, LIMIT);
            assert!(!report.complete);
            assert!(exact_sessions(&mut store)?.is_empty());
        }
        if case == "partial-line" {
            assert!((0..16).any(|_| {
                let report = scan.round(&mut store, &root, cutoff, &names, LIMIT);
                assert!(!report.complete && report.errors.is_empty());
                scan.waiting_for_newline.contains_key(&source)
            }));
            let waiting = scan.round(&mut store, &root, cutoff, &names, LIMIT);
            assert!(!waiting.complete && waiting.errors.is_empty());
            assert!(exact_sessions(&mut store)?.is_empty());
        }
        if matches!(case, "same-session" | "history-base") {
            assert!((0..16).any(|_| {
                scan.round(&mut store, &root, cutoff, &names, LIMIT)
                    .complete
            }));
            let exists = store.db.query_row(
                "SELECT EXISTS(SELECT 1 FROM token_observations
                WHERE json_extract(CAST(payload AS TEXT),'$.terminal_response')=?)",
                [&long_id],
                |row| row.get::<_, bool>(0),
            )?;
            assert!(exists, "{case}");
        }
    }
    assert!(divergences.is_empty(), "{divergences:?}");
    Ok(())
}

#[test]
fn recent_session_rechecks_linked_archive_without_live_growth() -> anyhow::Result<()> {
    use std::time::{Duration, UNIX_EPOCH};

    let dir = tempfile::tempdir()?;
    let root = dir.path().canonicalize()?;
    let sessions = root.join("sessions");
    let archives = root.join("archived_sessions");
    fs::create_dir(&sessions)?;
    fs::create_dir(&archives)?;
    let live = sessions.join("rollout-live.jsonl");
    let retired = sessions.join("rollout-retired.jsonl");
    let linked = archives.join("rollout-linked.jsonl.zst");
    let unrelated = archives.join("rollout-unrelated.jsonl.zst");
    let now = chrono::Utc::now();
    fs::write(&retired, rollout("00000000-0000-4000-8000-000000000099"))?;
    fs::write(
        &live,
        rollout(FIRST)
            .replace("2026-01-02T12:00:00Z", &(now - chrono::TimeDelta::minutes(2)).to_rfc3339())
            .replace("2026-01-02T12:01:00Z", &(now - chrono::TimeDelta::minutes(1)).to_rfc3339())
            .replace(r#""input_tokens":100"#, r#""input_tokens":200"#),
    )?;
    let padding = format!(
        "{{\"type\":\"ignored\",\"payload\":\"{}\"}}\n",
        "x".repeat(65_536)
    );
    for (path, session, count) in [(&linked, FIRST, 0), (&unrelated, SECOND, 144)] {
        let mut file = fs::File::create(path)?;
        zstd::stream::copy_encode(
            format!("{}{}", rollout(session), padding.repeat(count)).as_bytes(),
            &mut file,
            1,
        )?;
        if path == &linked {
            file.set_modified(UNIX_EPOCH + Duration::from_secs(1_770_000_000))?;
        }
    }
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let names = BTreeMap::new();
    let mut store = Store::open(Connection::open_in_memory()?)?;
    let mut scan = Scan::default();
    assert!((0..16).any(|_| scan.round(&mut store, &root, cutoff, &names, LIMIT).complete));
    assert_eq!(
        store.recent_sessions(now.timestamp_millis() - super::super::super::RECENT_CLASSIFICATION_MS)?
            .into_iter()
            .map(|(session, _)| session)
            .collect::<Vec<_>>(),
        [FIRST],
    );
    fs::rename(&retired, root.join("not-discovered.jsonl"))?;
    for path in [&linked, &unrelated] {
        store.db.execute(
            "UPDATE token_sources SET checkpoint=CAST(json_remove(checkpoint,'$.legacy_identity_prefix') AS BLOB)
             WHERE revision=?",
            [store.checkpoint(path)?.unwrap().0],
        )?;
    }
    let mut scan = Scan::default();
    let report = scan.round(&mut store, &root, cutoff, &names, LIMIT);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(report.can_classify && !report.complete);
    assert!(scan.missing.iter().any(|(path, _)| path == &retired));
    assert_eq!(store.checkpoint(&linked)?.unwrap().1.legacy_identity_prefix, 0);
    assert_ne!(store.checkpoint(&unrelated)?.unwrap().1.legacy_identity_prefix, 0);
    let first_admitted = store.db.query_row(
        "SELECT NOT EXISTS(SELECT 1 FROM json_each(blocked_sessions)
         WHERE value=(SELECT id FROM token_sessions WHERE session=?1))
         FROM token_attribution_coverage",
        [FIRST],
        |row| row.get::<_, bool>(0),
    )?;
    assert!(first_admitted);
    assert!(!scan.remaining.contains(&unrelated), "unrelated history must also start");
    Ok(())
}

#[test]
fn growing_session_prioritizes_linked_prefixes_and_keeps_other_history_moving() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let root = dir.path().canonicalize()?;
    let sessions = root.join("sessions");
    fs::create_dir(&sessions)?;
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let paths: Vec<_> = (0..5).map(|n| sessions.join(format!("rollout-fixture-{n}.jsonl"))).collect();
    let padding = format!("{{\"type\":\"ignored\",\"payload\":\"{}\"}}\n", "x".repeat(65536)).repeat(65);
    for (i, path) in paths.iter().enumerate() {
        let session = match i {
            0 | 1 => FIRST.to_owned(),
            n => format!("00000000-0000-4000-8000-{:012}", n + 100),
        };
        let mut rows = rollout(&session);
        rows.push_str(&padding);
        if i == 0 { rows.push_str(&padding); }
        fs::write(path, rows)?;
    }
    let mut store = Store::open(Connection::open_in_memory()?)?;
    let mut scan = Scan::default();
    assert!((0..32).any(|_| scan.round(&mut store, &root, cutoff, &BTreeMap::new(), LIMIT).complete));
    for (i, path) in paths.iter().enumerate() {
        let revision = store.checkpoint(path)?.unwrap().0;
        store.db.execute(
            "UPDATE token_sources SET checkpoint=CAST(json_remove(checkpoint,'$.legacy_identity_prefix') AS BLOB)
             WHERE revision=?", [revision],
        )?;
        if i == 1 || i == 2 {
            // 第二个前缀只能经首个前缀的物理续段关联到增长源。
            store.db.execute(
                "INSERT INTO token_source_links SELECT id,'r:00000000-0000-4000-8000-000000000199'
                 FROM token_sources WHERE revision=?", [revision],
            )?;
        }
    }
    let mut ordered: VecDeque<_> = [3, 2, 0, 4, 1].map(|i| paths[i].clone()).into();
    store.prioritize_queue(&BTreeMap::from([(paths[0].clone(), 1)]), None, &mut ordered)?;
    assert_eq!(ordered, [0, 2, 3, 1, 4].map(|i| paths[i].clone()));

    let mut scan = Scan::default();
    let mut active = fs::OpenOptions::new().append(true).open(&paths[0])?;
    for i in 1..=32 {
        let mut count: serde_json::Value = serde_json::from_str(&next_count())?;
        count["timestamp"] = json!(format!("2026-01-02T12:02:{i:02}Z"));
        count["payload"]["info"]["total_token_usage"]["input_tokens"] = json!(100 + 102 * i);
        writeln!(active, "{count}")?;
        let report = scan.round(&mut store, &root, cutoff, &BTreeMap::new(), LIMIT);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        let checkpoint = store.checkpoint(&paths[0])?.unwrap().1;
        assert_eq!(checkpoint.offset, active.metadata()?.len());
        if i == 1 { assert_ne!(checkpoint.legacy_identity_prefix, 0); }
        let appended: usize = store.db.query_row(
            "SELECT count(*) FROM token_resolved WHERE json_extract(CAST(payload AS TEXT),'$.delta.i')=102",
            [], |row| row.get(0),
        )?;
        assert_eq!(appended, i);
        if report.complete { break; }
    }
    for path in &paths {
        assert_eq!(store.checkpoint(path)?.unwrap().1.legacy_identity_prefix, 0);
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
fn pending_newline_on_known_append_keeps_published_supplier_proofs() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let root = fs::canonicalize(dir.path())?;
    let sessions = root.join("sessions");
    fs::create_dir(&sessions)?;
    let source = sessions.join(format!("rollout-{FIRST}.jsonl"));
    fs::write(&source, rollout(FIRST))?;
    fs::write(sessions.join(format!("rollout-{SECOND}.jsonl")), rollout(SECOND))?;
    let mut store = Store::open(Connection::open_in_memory()?)?;
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let names = BTreeMap::new();
    assert!(Scan::default().round(&mut store, &root, cutoff, &names, LIMIT).complete);
    for session in [FIRST, SECOND] {
        assert!(store.reclassify_session(session, LIMIT, &BTreeSet::new(), |events, _| {
            for event in events {
                event.provider = "direct-supplier".into();
                event.provider_source = Some("request-log-exact".into());
            }
            Ok(())
        })?.is_some());
    }
    let mut file = fs::OpenOptions::new().append(true).open(&source)?;
    write!(file, "{}", next_count())?;
    let mut scan = Scan::default();
    for _ in 0..2 {
        let report = scan.round(&mut store, &root, cutoff, &names, LIMIT);
        assert!(!report.complete && report.pending_sources > 0);
        assert_eq!(report.issues, 0, "{:?}", report.errors);
        let mut actual = BTreeMap::new();
        Store::read_committed(&mut store.db, i64::MIN, i64::MAX, |event| {
            actual.insert(event.session, (event.provider, event.delta.input_tokens));
            Ok(())
        })?;
        assert_eq!(actual, BTreeMap::from([
            (FIRST.to_owned(), ("direct-supplier".to_owned(), 100)),
            (SECOND.to_owned(), ("direct-supplier".to_owned(), 100)),
        ]));
        assert!(report.can_classify);
        let mut blocked = store.db.prepare(
            "SELECT s.session FROM token_attribution_coverage a
             JOIN json_each(a.blocked_sessions) b JOIN token_sessions s ON s.id = b.value",
        )?;
        let blocked = blocked.query_map(
            [],
            |r| r.get::<_, String>(0),
        )?.collect::<rusqlite::Result<Vec<_>>>()?;
        assert_eq!(blocked, [FIRST]);
    }
    writeln!(file)?;
    assert!(scan.round(&mut store, &root, cutoff, &names, LIMIT).complete);
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
