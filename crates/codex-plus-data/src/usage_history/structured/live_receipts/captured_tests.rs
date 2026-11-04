use super::completion_lag_tests::{positive, receipt, request};
use super::*;
use rusqlite::Connection;

fn fixture() -> (tempfile::TempDir, provider_cache::ProviderCache, i64) {
    let root = tempfile::tempdir().unwrap();
    let mut cache = provider_cache::ProviderCache::default();
    cache
        .open(root.path(), Local::now().date_naive() - Days::days(1))
        .unwrap();
    (root, cache, Local::now().timestamp_millis())
}

fn check(root: &Path, records: &[RequestEvidence], input: &Value) -> Value {
    match_captured_receipts(root, records, &BTreeMap::new(), input)
}

#[test]
fn cache_only_receipts_survive_rotation_without_scanned_events_or_writes() {
    let (root, mut cache, at) = fixture();
    let record = request("one", at, 97, 10, 2, 3);
    cache.capture_proxy(&[record.clone()]).unwrap();
    let input = json!([receipt("live", at, 10, 2, 3)]);
    let first = check(root.path(), &[], &input);
    assert!(positive(&first), "{first}");
    // 当前快照和缓存的同一条请求不是两个竞争请求。
    assert_eq!(check(root.path(), &[record], &input), first);
    let mut saved = input;
    for key in ["model", "provider", "requestIds", "requestSignatures"] {
        saved[0][key] = first[0][key].clone();
    }
    assert_eq!(check(root.path(), &[], &saved), first);
    let reader = provider_cache::ProviderCache::read_snapshot(
        root.path(),
        Local::now().date_naive() - Days::days(1),
    )
    .unwrap();
    assert!(reader.rejected_proxy_ids_read_only(&[]).unwrap().is_empty());
    let db = Connection::open(provider_cache::database_path(root.path()).unwrap()).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM rejected_proxy_ids", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn versioned_retry_contract_survives_cache_reload_and_saved_proof_revalidation() {
    let (root, mut cache, at) = fixture();
    let mut record = request("retried", at, -10, 10, 2, 3);
    record.attempts = Some(2);
    record.auto_retries = Some(1);
    record.final_response_evidence = Some("final-response-v1".into());
    cache.capture_proxy(&[record]).unwrap();
    drop(cache);
    let mut input = json!([receipt("live", at, 10, 2, 3)]);
    let proof = check(root.path(), &[], &input);
    assert!(positive(&proof), "{proof}");
    for key in ["model", "provider", "requestIds", "requestSignatures"] {
        input[0][key] = proof[0][key].clone();
    }
    assert_eq!(check(root.path(), &[], &input), proof);
}

#[test]
fn delayed_final_response_survives_cache_reload_and_retains_competitors() {
    let (root, mut cache, at) = fixture();
    let mut record = request("delayed", at, -63_925, 172115, 179, 171904);
    record.final_response_evidence = Some("final-response-v1".into());
    cache.capture_proxy(&[record]).unwrap();
    let mut input = json!([receipt("live", at, 172115, 179, 171904)]);
    let proof = check(root.path(), &[], &input);
    assert!(positive(&proof), "{proof}");
    for key in ["model", "provider", "requestIds", "requestSignatures"] {
        input[0][key] = proof[0][key].clone();
    }
    assert_eq!(check(root.path(), &[], &input), proof);
    cache
        .capture_proxy(&[request("legacy", at, -90_000, 172115, 179, 171904)])
        .unwrap();
    assert!(!positive(&check(root.path(), &[], &input)));
}

#[test]
fn cached_competitors_persisted_rejections_and_changed_signatures_are_not_hidden() {
    let (root, mut cache, at) = fixture();
    let a = request("a", at, -10, 10, 2, 3);
    let input = json!([receipt("live", at, 10, 2, 3)]);
    cache.capture_proxy(&[a.clone()]).unwrap();
    let proof = check(root.path(), &[], &input);
    assert!(positive(&proof));
    let mut saved = input.clone();
    for key in ["model", "provider", "requestIds", "requestSignatures"] {
        saved[0][key] = proof[0][key].clone();
    }
    let b = request("b", at, 97, 10, 2, 3);
    cache.capture_proxy(&[b]).unwrap();
    assert!(!positive(&check(root.path(), &[a.clone()], &input)));
    assert!(!positive(&check(root.path(), &[], &saved)));

    let (other, mut other_cache, _) = fixture();
    other_cache.capture_proxy(&[a.clone()]).unwrap();
    let mut changed = a.clone();
    changed.relay_name = "Changed".into();
    assert!(!positive(&check(other.path(), &[changed], &saved)));
    let db = Connection::open(provider_cache::database_path(other.path()).unwrap()).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM rejected_proxy_ids", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0,
        "read-only checks must not persist newly discovered conflicts"
    );
    db.execute("INSERT INTO rejected_proxy_ids VALUES('a',?1)", [at])
        .unwrap();
    assert!(!positive(&check(other.path(), &[a], &input)));
}

#[test]
fn persisted_aliases_include_all_competitors_and_conflicting_aliases_fail_closed() {
    let (root, mut cache, at) = fixture();
    let mut record = request("a", at, 0, 10, 2, 3);
    record.thread_id = "alias".into();
    cache.capture_proxy(&[record]).unwrap();
    cache
        .save(
            &HashMap::new(),
            &BTreeMap::from([("alias".into(), "session".into())]),
        )
        .unwrap();
    let input = json!([receipt("live", at, 10, 2, 3)]);
    assert!(positive(&check(root.path(), &[], &input)));
    cache
        .capture_proxy(&[request("direct", at, 97, 10, 2, 3)])
        .unwrap();
    let conflict = BTreeMap::from([("alias".into(), "another-session".into())]);
    assert!(!positive(&match_captured_receipts(
        root.path(),
        &[],
        &conflict,
        &input
    )));
    cache
        .capture_proxy(&[request("b", at, 97, 10, 2, 3)])
        .unwrap();
    assert!(!positive(&check(root.path(), &[], &input)));
}

#[test]
fn incomplete_competing_receipt_cannot_release_its_remaining_request_to_another_receipt() {
    let (root, mut cache, at) = fixture();
    let floor = at - 120_000;
    cache
        .capture_proxy(&[request("current", at, -1_000, 10, 2, 3)])
        .unwrap();
    let db = Connection::open(provider_cache::database_path(root.path()).unwrap()).unwrap();
    db.execute(
        "INSERT INTO capture_retention VALUES('completed_proxy_requests',?1)",
        [floor - 1],
    )
    .unwrap();
    let input = json!([
        receipt("incomplete", at - 1_000, 20, 4, 6),
        receipt("complete", at, 10, 2, 3)
    ]);
    let result = check(root.path(), &[], &input);
    assert!(!positive(&result), "{result}");
    assert_eq!(result.as_array().unwrap().len(), 2);
    assert!(
        result
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["rejected"] == true)
    );
}

#[test]
fn retention_and_complete_millisecond_limits_reject_partial_windows() {
    let (root, mut cache, at) = fixture();
    let record = request("a", at, -10, 10, 2, 3);
    cache.capture_proxy(&[record]).unwrap();
    let input = json!([receipt("live", at, 10, 2, 3)]);
    let db = Connection::open(provider_cache::database_path(root.path()).unwrap()).unwrap();
    db.execute(
        "INSERT INTO capture_retention VALUES('completed_proxy_requests',?1)",
        [at - DELAYED_REQUEST_MATCH_WINDOW_MS],
    )
    .unwrap();
    let limited = check(root.path(), &[], &input);
    assert_eq!(limited[0]["rejected"], true);
    db.execute("DELETE FROM capture_retention", []).unwrap();
    let rows: Vec<_> = (0..=MAX_REQUEST_EVIDENCE)
        .map(|index| request(&format!("competitor-{index}"), at, -20, 1, 1, 0))
        .collect();
    cache.capture_proxy(&rows).unwrap();
    let limited = check(root.path(), &[], &input);
    assert_eq!(limited[0]["rejected"], true, "{limited}");
}

#[test]
fn reads_use_one_snapshot_and_never_create_or_migrate_storage() {
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("missing");
    let at = Local::now().timestamp_millis();
    let input = json!([receipt("live", at, 10, 2, 3)]);
    assert_eq!(check(&missing, &[], &input), json!([]));
    assert!(!missing.exists());
    let (root, mut cache, at) = fixture();
    let a = request("a", at, 0, 10, 2, 3);
    cache.capture_proxy(&[a]).unwrap();
    let cutoff = Local::now().date_naive() - Days::days(1);
    let snapshot = provider_cache::ProviderCache::read_snapshot(root.path(), cutoff).unwrap();
    cache
        .capture_proxy(&[request("b", at, 0, 10, 2, 3)])
        .unwrap();
    assert_eq!(
        snapshot
            .captured_proxy("session", at - 1, at + 1)
            .unwrap()
            .rows
            .len(),
        1
    );
    drop(snapshot);
    let db = Connection::open(provider_cache::database_path(root.path()).unwrap()).unwrap();
    db.pragma_update(None, "user_version", 1).unwrap();
    assert_eq!(check(root.path(), &[], &input), json!([]));
    assert_eq!(
        db.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
    db.pragma_update(None, "user_version", 3).unwrap();
    assert_eq!(check(root.path(), &[], &input), json!([]));
}

#[test]
fn cached_requests_cannot_be_reused_by_two_receipts_or_cross_the_batch_limit() {
    let (root, mut cache, at) = fixture();
    cache
        .capture_proxy(&[request("a", at, 0, 10, 2, 3)])
        .unwrap();
    let input = json!([receipt("one", at, 10, 2, 3), receipt("two", at, 10, 2, 3)]);
    assert!(!positive(&check(root.path(), &[], &input)));
    let too_many: Vec<_> = (0..=MAX_RECEIPTS)
        .map(|index| receipt(&format!("live-{index}"), at, 10, 2, 3))
        .collect();
    assert_eq!(check(root.path(), &[], &json!(too_many)), json!([]));
}
