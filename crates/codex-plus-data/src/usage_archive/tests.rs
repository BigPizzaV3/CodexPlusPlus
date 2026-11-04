use super::*;
use rusqlite::types::Value as SqlValue;
use std::sync::{Arc, Barrier};

const SESSION: &str = "00000000-0000-4000-8000-000000000001";
const CHILD: &str = "00000000-0000-4000-8000-000000000002";
const TODAY: &str = "2026-11-04";

fn metrics(input: u64, output: u64) -> Value {
    json!({
        "input":input,"output":output,"cached":0,"reasoning":0,"total":input+output,
        "calls":1,"turns":1,"cachedKnown":true,"reasoningKnown":true,
        "inputKnown":true,"outputKnown":true,"totalKnown":true,"usageUncertain":false
    })
}

fn summary(input: u64, output: u64) -> Value {
    let mut model = metrics(input, output);
    model["model"] = json!("model");
    let mut breakdown = model.clone();
    breakdown["provider"] = json!("provider");
    breakdown["role"] = json!("main");
    let mut summary = metrics(input, output);
    summary["models"] = json!([model]);
    summary["breakdowns"] = json!([breakdown]);
    summary
}

fn entry(day: &str, detail: &str) -> Value {
    let summary = summary(10, 5);
    let mut core = json!({
        "all":summary,"sessions":[{"sessionId":SESSION,"summary":summary}],
        "sessionInfo":{},"aliases":{},"pendingLiveEvents":0
    });
    core["sessionInfo"][SESSION] = json!({"role":"main"});
    core["aliases"][SESSION] = json!(SESSION);
    json!({"version":1,"date":day,"core":core,"detail":detail,"protectedDetail":false})
}

fn request(entry: Value, expected: Option<&str>) -> Value {
    json!({"operation":"put","expectedRevision":expected,"entry":entry})
}

fn response(result: Result<Value>) -> Value {
    match result {
        Ok(value) => value,
        Err(ArchiveError::Unavailable(code)) => json!({"status":"unavailable","code":code}),
        Err(ArchiveError::Failed(code)) => json!({"status":"failed","code":code}),
    }
}

struct Fixture {
    _root: tempfile::TempDir,
    home: PathBuf,
    settings: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let state = root.path().join("state");
        fs::create_dir(&home).unwrap();
        fs::create_dir(&state).unwrap();
        Self {
            home,
            settings: state.join("settings.json"),
            _root: root,
        }
    }

    fn call(&self, payload: &Value) -> Value {
        self.at(payload, TODAY, MAX_TOTAL_DETAIL_BYTES)
    }

    fn at(&self, payload: &Value, today: &str, limit: usize) -> Value {
        response(dispatch(
            &self.home,
            &self.settings,
            payload,
            date(today).unwrap(),
            limit,
        ))
    }

    fn database(&self) -> PathBuf {
        Location::new(&self.home, &self.settings)
            .unwrap()
            .database(false)
            .unwrap()
    }

    fn read(&self, day: &str) -> Value {
        self.call(&json!({"operation":"read","date":day}))
    }
}

fn entry_revision(value: &Value) -> String {
    let entry: Entry = serde_json::from_value(value.clone()).unwrap();
    format!(
        "{:x}",
        Sha256::digest(canonical_json(&entry).unwrap().as_bytes())
    )
}

fn seed_legacy_text(fixture: &Fixture, value: &Value) -> String {
    let location = Location::new(&fixture.home, &fixture.settings).unwrap();
    let db = Connection::open(location.database(true).unwrap()).unwrap();
    db.execute_batch(
        "CREATE TABLE archive_owner(
            singleton INTEGER PRIMARY KEY CHECK(singleton=1),
            home_binding TEXT NOT NULL, state_binding TEXT NOT NULL, client TEXT NOT NULL
        );
        CREATE TABLE archive_days(
            client TEXT NOT NULL, date TEXT NOT NULL, revision TEXT NOT NULL,
            core TEXT NOT NULL, detail TEXT,
            protected_detail INTEGER NOT NULL CHECK(protected_detail IN (0,1)),
            PRIMARY KEY(client,date)
        ) WITHOUT ROWID;
        PRAGMA user_version=1;",
    )
    .unwrap();
    db.execute(
        "INSERT INTO archive_owner VALUES(1,?1,?2,?3)",
        params![location.home_binding, location.state_binding, CLIENT],
    )
    .unwrap();
    let entry: Entry = serde_json::from_value(value.clone()).unwrap();
    let revision = entry_revision(value);
    db.execute(
        "INSERT INTO archive_days VALUES(?1,?2,?3,?4,?5,?6)",
        params![
            CLIENT,
            entry.date,
            revision,
            canonical_json(&entry.core).unwrap(),
            entry.detail,
            entry.protected_detail
        ],
    )
    .unwrap();
    revision
}

fn dense_like_detail(chars: usize) -> String {
    let mut state = 0x7e57_a11c_u32;
    let mut detail = String::from("{\"codec\":\"opaque-dense16\",\"payload\":\"");
    for _ in 0..chars {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        let digit = state % 63_230;
        let code = if digit < 55_040 {
            digit + 0x100
        } else {
            digit - 55_040 + 0xe000
        };
        detail.push(char::from_u32(code).unwrap());
    }
    detail.push_str("\"}");
    detail
}

#[test]
fn large_dense_unicode_uses_smaller_zstd_blob_and_round_trips_from_sqlite_file() {
    let fixture = Fixture::new();
    let detail = dense_like_detail(400_000);
    assert!(detail.len() > 1024 * 1024);
    let value = entry(TODAY, &detail);
    let payload = request(value.clone(), None);
    let saved = fixture.call(&payload);
    assert_eq!(saved["status"], "ok");
    assert_eq!(saved["entry"]["revision"], entry_revision(&value));
    let db = Connection::open(fixture.database()).unwrap();
    let (kind, blob): (String, Vec<u8>) = db
        .query_row(
            "SELECT typeof(detail),detail FROM archive_days WHERE date=?1",
            [TODAY],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(kind, "blob");
    assert_eq!(&blob[..4], &[0x28, 0xb5, 0x2f, 0xfd]);
    assert!(
        blob.len() * 100 < detail.len() * 95,
        "dense-like UTF-8 should recover encoding overhead: {} -> {} bytes",
        detail.len(),
        blob.len()
    );
    drop(db);
    let before = fs::read(fixture.database()).unwrap();
    assert_eq!(fixture.read(TODAY)["entry"], saved["entry"]);
    assert_eq!(saved["entry"]["detail"], detail);
    assert_eq!(fixture.call(&payload), saved);
    assert_eq!(fs::read(fixture.database()).unwrap(), before);

    let legacy = Fixture::new();
    seed_legacy_text(&legacy, &value);
    assert!(
        fs::metadata(fixture.database()).unwrap().len()
            < fs::metadata(legacy.database()).unwrap().len(),
        "the SQLite file itself should be smaller than the equivalent TEXT archive"
    );
}

#[test]
fn legacy_text_reads_are_read_only_and_updates_use_blob_with_original_string_cas() {
    let fixture = Fixture::new();
    let value = entry(TODAY, "legacy\0\u{4e2d}\u{1f600}");
    let original_revision = seed_legacy_text(&fixture, &value);
    let before = fs::read(fixture.database()).unwrap();
    let files_before = files(fixture.settings.parent().unwrap());
    let saved = fixture.read(TODAY);
    assert_eq!(saved["status"], "ok");
    assert_eq!(saved["entry"]["detail"], value["detail"]);
    assert_eq!(saved["entry"]["revision"], original_revision);
    assert_eq!(fixture.call(&json!({"operation":"list"}))["status"], "ok");
    assert_eq!(fs::read(fixture.database()).unwrap(), before);
    assert_eq!(files(fixture.settings.parent().unwrap()), files_before);
    assert_eq!(fixture.call(&request(value, None)), saved);
    let db = Connection::open(fixture.database()).unwrap();
    let kind: String = db
        .query_row("SELECT typeof(detail) FROM archive_days", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(
        kind, "text",
        "idempotent retries must not rewrite legacy rows"
    );

    let changed = entry(TODAY, "updated\0\u{4e2d}\u{1f600}");
    let stale = fixture.call(&request(changed.clone(), None));
    assert_eq!(stale["status"], "conflict");
    assert_eq!(stale["revision"], original_revision);
    let updated = fixture.call(&request(changed.clone(), Some(&original_revision)));
    assert_eq!(updated["status"], "ok");
    assert_eq!(updated["entry"]["revision"], entry_revision(&changed));
    assert_eq!(updated["entry"]["detail"], changed["detail"]);
    assert_eq!(fixture.read(TODAY)["entry"], updated["entry"]);
    let kind: String = db
        .query_row("SELECT typeof(detail) FROM archive_days", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(kind, "blob");
}

#[test]
fn compressed_detail_corruption_and_expansion_limits_fail_without_writing() {
    let fixture = Fixture::new();
    let saved = fixture.call(&request(entry(TODAY, "kept"), None));
    assert_eq!(saved["status"], "ok");
    let db = Connection::open(fixture.database()).unwrap();
    let original: Vec<u8> = db
        .query_row("SELECT detail FROM archive_days", [], |row| row.get(0))
        .unwrap();
    let mut checksum_error = original.clone();
    *checksum_error.last_mut().unwrap() ^= 1;
    let mut trailing_garbage = original.clone();
    trailing_garbage.push(0xff);
    let expanded = zstd::stream::encode_all(&vec![b'x'; MAX_DETAIL_BYTES + 1][..], 1).unwrap();
    let invalid_utf8 = zstd::stream::encode_all(&[0xff_u8][..], 1).unwrap();
    for corrupt in [
        SqlValue::Blob(b"not a zstd frame".to_vec()),
        SqlValue::Blob(original[..original.len() - 1].to_vec()),
        SqlValue::Blob(checksum_error),
        SqlValue::Blob(trailing_garbage),
        SqlValue::Blob(expanded),
        SqlValue::Blob(invalid_utf8),
        SqlValue::Blob(vec![0; MAX_COMPRESSED_DETAIL_BYTES + 1]),
        SqlValue::Text("x".repeat(MAX_DETAIL_BYTES + 1)),
        SqlValue::Integer(42),
    ] {
        db.execute("UPDATE archive_days SET detail=?1", [corrupt])
            .unwrap();
        let before = fs::read(fixture.database()).unwrap();
        let read = fixture.read(TODAY);
        assert_eq!(read["status"], "failed");
        assert_eq!(read["code"], "invalid-stored-entry");
        // list 只读 core，不解压大明细；失败的 put 也不能顺带提交其他变更。
        assert_eq!(fixture.call(&json!({"operation":"list"}))["status"], "ok");
        let rejected = fixture.call(&request(entry("2026-11-03", "new"), None));
        assert_eq!(rejected["status"], "failed");
        assert_eq!(rejected["code"], "invalid-stored-entry");
        assert_eq!(fs::read(fixture.database()).unwrap(), before);
        assert_eq!(fixture.read("2026-11-03")["status"], "unavailable");
    }
    db.execute("UPDATE archive_days SET detail=?1", [original])
        .unwrap();
    assert_eq!(fixture.read(TODAY)["entry"], saved["entry"]);
}

#[test]
fn compressed_details_keep_utf8_input_and_logical_quota_boundaries() {
    let fixture = Fixture::new();
    let mut detail = "\u{4e2d}".repeat(MAX_DETAIL_BYTES / 3);
    detail.push_str(&"x".repeat(MAX_DETAIL_BYTES - detail.len()));
    assert_eq!(detail.len(), MAX_DETAIL_BYTES);
    let value = entry(TODAY, &detail);
    let saved = fixture.at(&request(value.clone(), None), TODAY, MAX_DETAIL_BYTES);
    assert_eq!(saved["status"], "ok");
    assert_eq!(saved["entry"]["detail"], detail);
    assert_eq!(fixture.read(TODAY)["entry"], saved["entry"]);
    let db = Connection::open(fixture.database()).unwrap();
    let stored_bytes: usize = db
        .query_row("SELECT length(detail) FROM archive_days", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert!(stored_bytes < MAX_DETAIL_BYTES / 100);
    let before = fs::read(fixture.database()).unwrap();
    let rejected = fixture.at(
        &request(entry("2026-11-03", "x"), None),
        TODAY,
        MAX_DETAIL_BYTES,
    );
    assert_eq!(rejected["status"], "failed");
    assert_eq!(rejected["code"], "detail-budget-exceeded");
    let mut oversized = value;
    oversized["detail"] = json!(format!("{detail}x"));
    let rejected = fixture.call(&request(oversized, saved["entry"]["revision"].as_str()));
    assert_eq!(rejected["status"], "failed");
    assert_eq!(rejected["code"], "detail-limit");
    assert_eq!(fs::read(fixture.database()).unwrap(), before);
}

#[test]
fn canonical_entry_retries_are_idempotent_and_cas_rejects_stale_writes() {
    let fixture = Fixture::new();
    let payload = request(entry(TODAY, "opaque"), None);
    let first = fixture.call(&payload);
    assert_eq!(first["status"], "ok");
    let revision = first["entry"]["revision"].as_str().unwrap();
    let mut reordered = payload.clone();
    let map = reordered["entry"]["core"].as_object_mut().unwrap();
    let all = map.remove("all").unwrap();
    map.insert("all".into(), all);
    assert_eq!(fixture.call(&reordered), first);
    assert_eq!(fixture.read(TODAY)["entry"], first["entry"]);
    let listed = fixture.call(&json!({"operation":"list"}));
    assert_eq!(listed["entries"].as_array().unwrap().len(), 1);
    assert!(listed["entries"][0].get("detail").is_none());
    assert_eq!(listed["entries"][0]["detailAvailable"], true);

    let changed = entry(TODAY, "updated");
    let rejected = fixture.call(&request(changed.clone(), None));
    assert_eq!(rejected["status"], "conflict");
    assert_eq!(rejected["revision"], revision);
    let updated = fixture.call(&request(changed.clone(), Some(revision)));
    assert_eq!(updated["status"], "ok");
    assert_ne!(updated["entry"]["revision"], revision);
    assert_eq!(fixture.call(&request(changed, Some(revision))), updated);
    assert_eq!(
        fixture.call(&payload)["status"],
        "conflict",
        "an older complete entry cannot overwrite the newer revision"
    );
    let missing = fixture.call(&request(entry("2026-11-03", ""), Some(revision)));
    assert_eq!(missing["status"], "conflict");
    assert_eq!(missing["revision"], Value::Null);
}

#[test]
fn concurrent_compare_and_swap_has_exactly_one_winner() {
    let fixture = Fixture::new();
    let first = fixture.call(&request(entry(TODAY, "initial"), None));
    let expected = first["entry"]["revision"].as_str().unwrap().to_owned();
    let barrier = Arc::new(Barrier::new(3));
    let workers: Vec<_> = ["writer-a", "writer-b"]
        .into_iter()
        .map(|detail| {
            let barrier = barrier.clone();
            let home = fixture.home.clone();
            let settings = fixture.settings.clone();
            let expected = expected.clone();
            std::thread::spawn(move || {
                barrier.wait();
                response(dispatch(
                    &home,
                    &settings,
                    &request(entry(TODAY, detail), Some(&expected)),
                    date(TODAY).unwrap(),
                    MAX_TOTAL_DETAIL_BYTES,
                ))
            })
        })
        .collect();
    barrier.wait();
    let results: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    assert_eq!(
        results.iter().filter(|row| row["status"] == "ok").count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(|row| row["status"] == "conflict")
            .count(),
        1
    );
    let winner = results.iter().find(|row| row["status"] == "ok").unwrap();
    assert_eq!(fixture.read(TODAY)["entry"], winner["entry"]);
}

#[test]
fn missing_reads_do_not_create_paths_or_claim_empty_success() {
    let fixture = Fixture::new();
    for payload in [
        json!({"operation":"list"}),
        json!({"operation":"read","date":TODAY}),
    ] {
        let result = query_archive(&fixture.home, &fixture.settings, &payload);
        assert_eq!(result["status"], "unavailable");
        assert_eq!(result["code"], "archive-missing");
    }
    assert_eq!(
        fs::read_dir(fixture.settings.parent().unwrap())
            .unwrap()
            .count(),
        0
    );
    let home = fixture._root.path().join("missing-home");
    assert_eq!(
        query_archive(&home, &fixture.settings, &json!({"operation":"list"}))["status"],
        "unavailable"
    );
    assert!(!home.exists());
}

fn files(directory: &Path) -> BTreeMap<PathBuf, (u64, Option<std::time::SystemTime>)> {
    let mut result = BTreeMap::new();
    for entry in fs::read_dir(directory).unwrap() {
        let entry = entry.unwrap();
        let metadata = entry.metadata().unwrap();
        if metadata.is_dir() {
            result.extend(files(&entry.path()));
        } else {
            result.insert(entry.path(), (metadata.len(), metadata.modified().ok()));
        }
    }
    result
}

#[test]
fn existing_reads_neither_write_nor_migrate_or_prune() {
    let fixture = Fixture::new();
    let old = "2026-09-01";
    assert_eq!(
        fixture.at(&request(entry(old, "old"), None), old, 100)["status"],
        "ok"
    );
    let database = fixture.database();
    let bytes = fs::read(&database).unwrap();
    let before = files(fixture.settings.parent().unwrap());
    assert_eq!(fixture.call(&json!({"operation":"list"}))["status"], "ok");
    assert_eq!(fixture.read(old)["entry"]["detail"], "old");
    assert_eq!(fixture.read(TODAY)["status"], "unavailable");
    assert_eq!(fs::read(&database).unwrap(), bytes);
    assert_eq!(files(fixture.settings.parent().unwrap()), before);

    let db = Connection::open(&database).unwrap();
    db.execute_batch("PRAGMA user_version=99;").unwrap();
    drop(db);
    let before = fs::read(&database).unwrap();
    for payload in [
        json!({"operation":"list"}),
        json!({"operation":"read","date":old}),
    ] {
        assert_eq!(fixture.call(&payload)["status"], "failed");
    }
    assert_eq!(fs::read(&database).unwrap(), before);
}

#[test]
fn retention_prunes_only_unprotected_details_older_than_thirty_local_days() {
    let fixture = Fixture::new();
    let today = date(TODAY).unwrap();
    let old = (today - Days::days(31)).to_string();
    let boundary = (today - Days::days(30)).to_string();
    let protected_day = (today - Days::days(32)).to_string();
    let old_result = fixture.at(&request(entry(&old, "expired"), None), &old, 100);
    let boundary_result = fixture.at(&request(entry(&boundary, "boundary"), None), &boundary, 100);
    let mut protected = entry(&protected_day, "protected");
    protected["protectedDetail"] = json!(true);
    protected["core"]["pendingLiveEvents"] = json!(1);
    let protected_result = fixture.at(&request(protected.clone(), None), &protected_day, 100);
    assert_eq!(fixture.read(&old)["entry"]["detail"], "expired");
    assert_eq!(
        fixture.call(&request(entry(TODAY, "today"), None))["status"],
        "ok"
    );
    let old_saved = fixture.read(&old);
    assert_eq!(old_saved["entry"]["detail"], Value::Null);
    assert_eq!(old_saved["entry"]["detailAvailable"], false);
    assert_eq!(
        old_saved["entry"]["revision"],
        old_result["entry"]["revision"]
    );
    assert_eq!(old_saved["entry"]["core"], old_result["entry"]["core"]);
    assert_eq!(fixture.read(&boundary)["entry"], boundary_result["entry"]);
    assert_eq!(
        fixture.read(&protected_day)["entry"],
        protected_result["entry"]
    );
    // 原请求重试不会把已经到期的 detail 恢复成热明细。
    assert_eq!(
        fixture.call(&request(entry(&old, "expired"), None))["entry"],
        old_saved["entry"]
    );
    protected["protectedDetail"] = json!(false);
    protected["core"]["pendingLiveEvents"] = json!(0);
    let released = fixture.call(&request(
        protected,
        protected_result["entry"]["revision"].as_str(),
    ));
    assert_eq!(released["status"], "ok");
    assert_eq!(released["entry"]["detailAvailable"], false);
    assert_eq!(
        fixture.call(&json!({"operation":"list"}))["entries"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
}

#[test]
fn quota_failure_rolls_back_new_entry_and_pruning_without_deleting_protected_data() {
    let fixture = Fixture::new();
    let old = "2026-09-01";
    assert_eq!(
        fixture.at(&request(entry(old, "1234"), None), old, 12)["status"],
        "ok"
    );
    let mut protected = entry("2026-11-03", "12345678");
    protected["protectedDetail"] = json!(true);
    assert_eq!(
        fixture.at(&request(protected, None), old, 12)["status"],
        "ok"
    );
    let before = fs::read(fixture.database()).unwrap();
    let rejected = fixture.at(&request(entry(TODAY, "12345678"), None), TODAY, 12);
    assert_eq!(rejected["code"], "detail-budget-exceeded");
    assert_eq!(fs::read(fixture.database()).unwrap(), before);
    assert_eq!(fixture.read(old)["entry"]["detail"], "1234");
    assert_eq!(fixture.read("2026-11-03")["entry"]["detail"], "12345678");
    assert_eq!(fixture.read(TODAY)["status"], "unavailable");
}

#[test]
fn database_errors_and_corruption_fail_without_overwriting_existing_data() {
    let fixture = Fixture::new();
    let first = fixture.call(&request(entry(TODAY, "kept"), None));
    let db = Connection::open(fixture.database()).unwrap();
    db.execute_batch(
        "CREATE TRIGGER reject_archive BEFORE INSERT ON archive_days
         BEGIN SELECT RAISE(ABORT,'synthetic'); END;",
    )
    .unwrap();
    assert_eq!(
        fixture.call(&request(entry("2026-11-03", "new"), None))["status"],
        "failed"
    );
    assert_eq!(fixture.read(TODAY)["entry"], first["entry"]);
    db.execute("UPDATE archive_days SET core='{}'", []).unwrap();
    assert_eq!(fixture.read(TODAY)["status"], "failed");
    assert_eq!(
        fixture.call(&json!({"operation":"list"}))["status"],
        "failed"
    );
}

#[test]
fn fixed_schema_rejects_unknown_fields_counts_non_uuid_ids_and_oversized_payloads() {
    let fixture = Fixture::new();
    let base = request(entry(TODAY, "opaque"), None);
    let mut bad = Vec::new();
    for pointer in [
        "",
        "/entry",
        "/entry/core",
        "/entry/core/all",
        "/entry/core/all/models/0",
        "/entry/core/all/breakdowns/0",
        "/entry/core/sessions/0",
        &format!("/entry/core/sessionInfo/{SESSION}"),
    ] {
        let mut value = base.clone();
        value.pointer_mut(pointer).unwrap()["unexpected"] = json!(true);
        bad.push(value);
    }
    for (pointer, replacement) in [
        ("/entry/version", json!(2)),
        ("/entry/core/all/total", json!(16)),
        ("/entry/core/all/cached", json!(11)),
        ("/entry/core/all/reasoning", json!(6)),
        ("/entry/core/all/input", json!(MAX_INTEGER + 1)),
        ("/entry/core/all/calls", json!(-1)),
        ("/entry/core/all/turns", json!(1.5)),
        ("/entry/core/all/models/0/input", json!(9)),
        ("/entry/core/sessions/0/summary/calls", json!(2)),
        ("/entry/core/sessions/0/sessionId", json!("../home")),
        ("/entry/core/pendingLiveEvents", json!(1)),
        ("/entry/core/all/models/0/model", json!("x".repeat(257))),
        ("/entry/date", json!("2026-02-30")),
    ] {
        let mut value = base.clone();
        *value.pointer_mut(pointer).unwrap() = replacement;
        bad.push(value);
    }
    let mut mismatched = base.clone();
    mismatched["date"] = json!("2026-11-03");
    bad.push(mismatched);
    let mut too_many = base.clone();
    too_many["entry"]["core"]["sessions"] = json!(vec![
        base["entry"]["core"]["sessions"][0].clone();
        MAX_SESSIONS + 1
    ]);
    bad.push(too_many);
    let mut too_large = base.clone();
    too_large["entry"]["detail"] = json!("x".repeat(MAX_DETAIL_BYTES + 1));
    bad.push(too_large);
    for payload in bad {
        assert_eq!(fixture.call(&payload)["status"], "failed");
    }
    assert_eq!(
        fs::read_dir(fixture.settings.parent().unwrap())
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn roles_accept_only_declared_values_without_silent_normalization() {
    for role in ["main", "subagent", "unknown"] {
        let fixture = Fixture::new();
        let mut value = entry(TODAY, "opaque");
        value["core"]["sessionInfo"][SESSION]["role"] = json!(role);
        value["core"]["all"]["breakdowns"][0]["role"] = json!(role);
        value["core"]["sessions"][0]["summary"]["breakdowns"][0]["role"] = json!(role);
        let saved = fixture.call(&request(value.clone(), None));
        assert_eq!(saved["status"], "ok");
        assert_eq!(saved["entry"]["core"], value["core"]);
    }
    let fixture = Fixture::new();
    for pointer in [
        format!("/core/sessionInfo/{SESSION}/role"),
        "/core/all/breakdowns/0/role".into(),
        "/core/sessions/0/summary/breakdowns/0/role".into(),
    ] {
        for role in ["", "MAIN", "child", "sub_agent", " main", "main "] {
            let mut value = entry(TODAY, "opaque");
            *value.pointer_mut(&pointer).unwrap() = json!(role);
            let rejected = fixture.call(&request(value, None));
            assert_eq!(rejected["status"], "failed");
            assert_eq!(rejected["code"], "invalid-role");
        }
    }
    assert_eq!(
        fs::read_dir(fixture.settings.parent().unwrap())
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn declared_optional_classification_fields_round_trip_without_promoting_them_to_proof() {
    let fixture = Fixture::new();
    let mut value = entry(TODAY, "not parsed or executed");
    value["core"]["sourceAccountingFingerprint"] = json!("consumer supplied");
    for summary in [&mut value["core"]["all"]] {
        summary["models"][0]["modelInferred"] = json!(true);
        summary["models"][0]["inferredModelTokens"] = json!(15);
        let row = &mut summary["breakdowns"][0];
        for key in [
            "modelConflict",
            "providerConflict",
            "providerInferred",
            "modelInferred",
        ] {
            row[key] = json!(true);
        }
        for key in ["inferredTokens", "inferredModelTokens"] {
            row[key] = json!(15);
        }
        for key in ["providerName", "providerSource", "modelSource"] {
            row[key] = json!("consumer supplied");
        }
    }
    let stored = fixture.call(&request(value.clone(), None));
    assert_eq!(stored["status"], "ok");
    assert_eq!(stored["entry"]["core"], value["core"]);
    assert_eq!(stored["entry"]["detail"], value["detail"]);
}

#[test]
fn homes_are_isolated_and_copied_databases_cannot_change_owner() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture.call(&request(entry(TODAY, "original"), None))["status"],
        "ok"
    );
    let other_home = fixture._root.path().join("other-home");
    fs::create_dir(&other_home).unwrap();
    let payload = json!({"operation":"list"});
    assert_eq!(
        query_archive(&other_home, &fixture.settings, &payload)["status"],
        "unavailable"
    );
    let other_db = Location::new(&other_home, &fixture.settings)
        .unwrap()
        .database(true)
        .unwrap();
    fs::copy(fixture.database(), &other_db).unwrap();
    assert_eq!(
        query_archive(&other_home, &fixture.settings, &payload)["code"],
        "owner-mismatch"
    );
    assert_eq!(
        query_archive(
            &other_home,
            &fixture.settings,
            &request(entry(TODAY, "overwrite"), None)
        )["code"],
        "owner-mismatch"
    );
    assert_eq!(fixture.read(TODAY)["entry"]["detail"], "original");

    let other_state = fixture._root.path().join("other-state");
    fs::create_dir(&other_state).unwrap();
    let other_settings = other_state.join("settings.json");
    let destination = Location::new(&fixture.home, &other_settings)
        .unwrap()
        .database(true)
        .unwrap();
    fs::copy(fixture.database(), destination).unwrap();
    assert_eq!(
        query_archive(&fixture.home, &other_settings, &payload)["code"],
        "owner-mismatch"
    );
}

#[test]
fn ancestry_and_aliases_are_typed_and_cycles_are_rejected() {
    let fixture = Fixture::new();
    let mut value = entry(TODAY, "opaque");
    value["core"]["sessionInfo"][CHILD] = json!({"role":"subagent","parentSessionId":SESSION});
    let saved = fixture.call(&request(value.clone(), None));
    assert_eq!(saved["status"], "ok");
    assert_eq!(
        saved["entry"]["core"]["sessionInfo"][CHILD]["parentSessionId"],
        SESSION
    );
    value["core"]["sessionInfo"][SESSION]["parentSessionId"] = json!(CHILD);
    assert_eq!(
        fixture.call(&request(value, saved["entry"]["revision"].as_str()))["code"],
        "parent-cycle"
    );
    let mut value = entry(TODAY, "opaque");
    value["core"]["aliases"][CHILD] = json!("local:00000000-0000-4000-8000-000000000001");
    assert_eq!(
        fixture.call(&request(value, saved["entry"]["revision"].as_str()))["code"],
        "invalid-session-id"
    );
}
