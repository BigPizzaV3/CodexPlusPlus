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
fn long_identifiers_survive_capture_reopen_and_late_competition() {
    let (root, mut cache, at) = fixture();
    let mut row = request(&"request".repeat(100), at, -10, 10, 2, 3);
    row.thread_id = "session".repeat(70);
    row.model = "模型".repeat(130);
    row.upstream_model = row.model.clone();
    row.relay_id = "supplier".repeat(64);
    row.relay_name = "供应商".repeat(64);
    row.final_response_id = Some("response".repeat(80));
    let mut input = json!([receipt(&"receipt".repeat(100), at, 10, 2, 3)]);
    input[0]["sessionId"] = json!(row.thread_id);
    cache.capture_proxy(std::slice::from_ref(&row)).unwrap();
    drop(cache);
    let proof = check(root.path(), &[], &input);
    assert_eq!(proof[0]["id"], input[0]["id"]);
    assert_eq!(proof[0]["model"], row.upstream_model);
    assert_eq!(proof[0]["provider"], row.relay_id);
    assert_eq!(proof[0]["providerName"], row.relay_name);
    assert_eq!(proof[0]["requestIds"], json!([row.id]));
    for key in ["model", "provider", "requestIds", "requestSignatures"] {
        input[0][key] = proof[0][key].clone();
    }
    assert_eq!(check(root.path(), &[], &input), proof);
    let mut cache = provider_cache::ProviderCache::default();
    cache.open(root.path(), Local::now().date_naive() - Days::days(1)).unwrap();
    row.id.push('2');
    row.final_response_id = None;
    row.usage = None;
    cache.capture_proxy(&[row]).unwrap();
    drop(cache);
    assert_eq!(check(root.path(), &[], &input)[0]["rejected"], true);
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
fn fallback_receipt_revalidates_original_model_after_cache_reload() {
    let (root, mut cache, at) = fixture();
    let mut record = request("fallback", at, -10, 10, 2, 3);
    record.model = "supplier-a/model-a".into();
    record.upstream_model = "model-a".into();
    record.relay_id = "supplier-b".into();
    record.relay_name = "Supplier B".into();
    record.attempts = Some(2);
    record.final_response_evidence = Some("final-response-v1".into());
    cache.capture_proxy(&[record]).unwrap();
    drop(cache);

    let mut input = json!([receipt("live", at, 10, 2, 3)]);
    input[0]["model"] = json!("supplier-a/model-a");
    let proof = check(root.path(), &[], &input);
    assert_eq!(proof[0]["provider"], "supplier-b");
    assert_eq!(proof[0]["model"], "model-a");
    for key in ["provider", "requestIds", "requestSignatures"] {
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
    changed.relay_id = "different-supplier".into();
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
fn display_name_changes_do_not_invalidate_current_receipts_or_cached_requests() {
    let (root, mut cache, at) = fixture();
    let mut record = request("a", at, -10, 10, 2, 3);
    record.relay_name.clear();
    cache.capture_proxy(&[record.clone()]).unwrap();
    let input = json!([receipt("live", at, 10, 2, 3)]);
    let first = check(root.path(), &[record.clone()], &input);
    assert!(positive(&first), "{first}");
    assert_eq!(first[0]["providerName"], "supplier");
    let mut saved = input;
    for key in ["model", "provider", "requestIds", "requestSignatures"] {
        saved[0][key] = first[0][key].clone();
    }

    record.relay_name = "Renamed supplier".into();
    cache.capture_proxy(&[record.clone()]).unwrap();
    drop(cache);
    let renamed = check(root.path(), &[record.clone()], &saved);
    assert!(positive(&renamed), "{renamed}");
    assert_eq!(renamed[0]["providerName"], "Renamed supplier");
    assert_eq!(renamed[0]["input"], 10);
    assert_eq!(renamed[0]["output"], 2);
    assert_eq!(renamed[0]["cached"], 3);
    assert!(!positive(&check(
        root.path(),
        &[record.clone(), record],
        &saved
    )));
}

#[test]
fn legacy_signature_after_rename_requires_the_original_cached_record() {
    let (root, mut cache, at) = fixture();
    let record = request("legacy", at, -10, 10, 2, 3);
    cache.capture_proxy(&[record.clone()]).unwrap();
    let mut saved = json!([receipt("live", at, 10, 2, 3)]);
    let proof = check(root.path(), &[record.clone()], &saved);
    for key in ["model", "provider", "requestIds"] {
        saved[0][key] = proof[0][key].clone();
    }
    saved[0]["requestSignatures"] = json!({
        "legacy": format!("{:x}", Sha256::digest(serde_json::to_vec(&record).unwrap()))
    });
    let mut renamed = record.clone();
    renamed.relay_name = "Renamed supplier".into();
    assert!(!positive(&match_receipts(
        &[renamed.clone()],
        &BTreeMap::new(),
        &saved,
    )));
    assert!(positive(&check(root.path(), &[renamed.clone()], &saved)));

    let mut conflict = renamed;
    conflict.usage.as_mut().unwrap().output_tokens = Some(4);
    assert!(!positive(&check(root.path(), &[conflict], &saved)));
}

#[test]
fn cached_extra_evidence_is_still_a_conflict_after_name_normalization() {
    let (root, mut cache, at) = fixture();
    let record = request("a", at, -10, 10, 2, 3);
    cache.capture_proxy(&[record.clone()]).unwrap();
    let mut stored = serde_json::to_value(&record).unwrap();
    stored["additionalEvidence"] = json!("different");
    let db = Connection::open(provider_cache::database_path(root.path()).unwrap()).unwrap();
    db.execute(
        "UPDATE completed_proxy_requests SET data=?1 WHERE id='a'",
        [stored.to_string()],
    )
    .unwrap();
    let input = json!([receipt("live", at, 10, 2, 3)]);
    assert!(!positive(&check(root.path(), &[record], &input)));
}

#[test]
fn pre_enrichment_receipt_signature_survives_a_rename_but_not_a_changed_response_id() {
    let (root, mut cache, at) = fixture();
    let mut record = request("old-format", at, -10, 10, 2, 3);
    let mut input = json!([receipt("live", at, 10, 2, 3)]);
    input[0]["model"] = json!(record.model);
    input[0]["provider"] = json!(record.relay_id);
    input[0]["requestIds"] = json!([record.id]);
    input[0]["requestSignatures"] = json!({record.id.clone(): legacy_request_signature(&record)});
    let mut legacy = record.clone();
    legacy.relay_name.clear();
    let previous_signature = legacy_request_signature(&legacy);
    cache.capture_proxy(std::slice::from_ref(&record)).unwrap();
    record.final_response_id = Some("response-a".into());
    record.usage.as_mut().unwrap().cached_tokens_known = Some(true);
    cache.capture_proxy(std::slice::from_ref(&record)).unwrap();
    record.relay_name = "Renamed".into();
    let proof = check(root.path(), std::slice::from_ref(&record), &input);
    assert!(positive(&proof), "{proof}");
    assert_eq!(
        proof[0]["requestSignatures"][&record.id],
        previous_signature
    );
    input[0]["requestSignatures"] = proof[0]["requestSignatures"].clone();
    record.final_response_id = Some("response-b".into());
    assert!(!positive(&check(root.path(), &[record], &input)));
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
fn physical_retention_rejects_missing_windows_but_dense_complete_milliseconds_are_read() {
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
    db.execute("DELETE FROM capture_retention WHERE kind='completed_proxy_requests'", []).unwrap();
    let rows: Vec<_> = (0..20_001)
        .map(|index| request(&format!("other-{index}"), at, -5, 100, 20, 30))
        .collect();
    cache.capture_proxy(&rows).unwrap();
    let aliases = (0..17).map(|i| (format!("alias-{i}"), "session".to_owned())).collect();
    let unique = match_captured_receipts(root.path(), &[], &aliases, &input);
    assert!(positive(&unique), "{unique}");
    assert_eq!(unique[0]["requestIds"], json!(["a"]));
    let mut competitor = request("b", at, -12, 10, 2, 3);
    competitor.thread_id = "alias-16".into();
    cache.capture_proxy(&[competitor]).unwrap();
    let conflict = match_captured_receipts(root.path(), &[], &aliases, &input);
    assert!(!positive(&conflict), "{conflict}");
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
fn older_cache_without_observation_extras_remains_readable() {
    let (root, mut cache, at) = fixture();
    cache
        .capture_proxy(&[request("legacy", at, 0, 10, 2, 3)])
        .unwrap();
    drop(cache);
    let db = Connection::open(provider_cache::database_path(root.path()).unwrap()).unwrap();
    db.execute_batch(
        "DROP TRIGGER completed_proxy_extra_delete;
         DROP TRIGGER completed_proxy_extra_invalidate;
         DROP TABLE completed_proxy_request_extra;",
    )
    .unwrap();
    let input = json!([receipt("live", at, 10, 2, 3)]);
    assert!(positive(&check(root.path(), &[], &input)));
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name='completed_proxy_request_extra'",
            [],
            |row| row.get::<_, usize>(0),
        )
        .unwrap(),
        0
    );
}

#[test]
fn large_receipt_batch_keeps_unique_proofs_and_rejects_competitors_at_both_ends() {
    let (root, mut cache, at) = fixture();
    let requests: Vec<_> = (0..129)
        .map(|index| request(&format!("request-{index}"), at, 0, 10 + index, 2, 3))
        .collect();
    cache.capture_proxy(&requests).unwrap();
    let mut receipts: Vec<_> = (0..129)
        .map(|index| receipt(&format!("live-{index}"), at, 10 + index, 2, 3))
        .collect();
    receipts.push(receipt("last", at, 10, 2, 3));
    let result = check(root.path(), &[], &json!(receipts));
    let rows = result.as_array().unwrap();
    assert_eq!(rows.len(), 130);
    assert_eq!(rows[0]["rejected"], true);
    assert_eq!(rows[129]["rejected"], true);
    for index in 1..129 {
        assert_eq!(rows[index]["id"], format!("live-{index}"));
        assert_eq!(rows[index]["requestIds"], json!([format!("request-{index}")]));
    }
}

#[test]
fn nine_sequential_requests_revalidate_after_reopen_and_yield_to_competition() {
    let (root, mut cache, at) = fixture();
    let requests: Vec<_> = (0..9)
        .map(|index| {
            let mut row = request(&format!("request-{index}"), at, index * 100 - 900, 10, 2, 3);
            row.started_at_ms = row.timestamp_ms - 50;
            row
        })
        .collect();
    cache.capture_proxy(&requests).unwrap();
    let mut input = json!([receipt("nine", at, 90, 18, 27)]);
    let proof = check(root.path(), &[], &input);
    assert!(positive(&proof), "{proof}");
    let ids: Vec<_> = requests.iter().rev().map(|row| &row.id).collect();
    assert_eq!(proof[0]["requestIds"], json!(ids));
    for key in ["model", "provider", "requestIds", "requestSignatures"] {
        input[0][key] = proof[0][key].clone();
    }
    drop(cache);
    assert_eq!(check(root.path(), &[], &input), proof);
    let mut reopened = provider_cache::ProviderCache::default();
    reopened.open(root.path(), Local::now().date_naive() - Days::days(1)).unwrap();
    reopened.capture_proxy(&[request("competing", at, 0, 90, 18, 27)]).unwrap();
    assert_eq!(check(root.path(), &[], &input)[0]["rejected"], true);
}
