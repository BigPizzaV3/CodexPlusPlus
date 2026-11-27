use super::*;

const SESSION: &str = "00000000-0000-4000-8000-000000000010";

fn event(at: i64) -> UsageEvent {
    UsageEvent {
        native_response_id: None,
        terminal_response: None,
        id: [7; 32],
        original_id: [7; 32],
        session: SESSION.into(),
        turn: "turn-a".into(),
        model: "model-a".into(),
        timestamp: DateTime::from_timestamp_millis(at).unwrap().to_rfc3339(),
        date: DateTime::from_timestamp_millis(at)
            .unwrap()
            .with_timezone(&Local)
            .date_naive(),
        total: Tokens::new(100, 20, 10),
        delta: Tokens::new(100, 20, 10),
        has_file_baseline: false,
        rollout_position: None,
        count_pending: false,
        last: None,
        count_uncertain: false,
        count_incomplete: false,
        provider: String::new(),
        provider_source: None,
        role: "main".into(),
        inherited: false,
        legacy_identity: None,
        context_conflicts: ContextConflicts::default(),
    }
}

fn request(at: i64) -> RequestEvidence {
    RequestEvidence {
        id: "request-a".into(),
        thread_id: SESSION.into(),
        model: "model-a".into(),
        upstream_model: "model-a".into(),
        relay_id: "supplier-a".into(),
        relay_name: "Supplier A".into(),
        success: true,
        state: "success".into(),
        status_code: 200,
        timestamp_ms: at - 10,
        started_at_ms: at - 100,
        attempts: Some(1),
        auto_retries: Some(0),
        final_response_evidence: None,
        final_response_id: None,
        usage: Some(RequestEvidenceUsage {
            input_tokens: Some(100),
            output_tokens: Some(10),
            cached_tokens: Some(20),
            cached_tokens_known: None,
        }),
        agent: String::new(),
    }
}

fn attribute(cache: &mut RequestEvidenceCache, events: &mut [UsageEvent]) -> Value {
    cache.attribute(events, &mut BTreeMap::new(), &BTreeMap::new())
}

#[test]
fn scoped_namespace_infers_only_known_suppliers_without_session_requests() -> anyhow::Result<()> {
    let at = Local::now().timestamp_millis() - 600_000;
    let mut cache = RequestEvidenceCache::default();
    let mut names = BTreeMap::from([
        ("supplier-a".to_owned(), "Supplier A".to_owned()),
        ("group".to_owned(), "Routing group".to_owned()),
        ("custom".to_owned(), "Custom".to_owned()),
    ]);
    let scope = AttributionScope {
        targets: 0..1,
        gateway_blocked: false,
        native_blocked: false,
        paired_only: false,
        resolve_terminal: None,
    };
    for (model, expected) in [
        ("supplier-a/model-a", "supplier-a"),
        ("supplier-a/vendor/model-a", "supplier-a"),
        ("missing/model-a", ""),
        ("group/model-a", ""),
        ("custom/model-a", ""),
        ("supplier-a/unknown", ""),
        ("supplier-a/", ""),
        ("model-a", ""),
    ] {
        let mut original = event(at);
        original.model = model.into();
        let mut events = [original.clone()];
        cache.infer_scoped(&mut events, &mut names, &BTreeMap::new(), &scope)?;
        assert_eq!(events[0].provider, expected, "{model}");
        assert_eq!(events[0].provider_source.as_deref(),
            (!expected.is_empty()).then_some("model-namespace"), "{model}");
        events[0].provider.clear();
        events[0].provider_source = None;
        assert!(events[0] == original, "{model}: changed raw usage");
    }
    let mut unrelated = request(at);
    unrelated.thread_id = "another-session".into();
    cache.records.push(unrelated);
    let mut events = [event(at)];
    events[0].model = "supplier-a/model-a".into();
    cache.infer_scoped(&mut events, &mut names, &BTreeMap::new(), &scope)?;
    assert_eq!(events[0].provider_source.as_deref(), Some("model-namespace"));
    for field in ["provider", "context", "uncertain", "incomplete", "pending"] {
        let mut original = event(at);
        original.model = "supplier-a/model-a".into();
        match field {
            "provider" => original.provider = "supplier-b".into(),
            "context" => original.context_conflicts.provider = true,
            "uncertain" => original.count_uncertain = true,
            "incomplete" => original.count_incomplete = true,
            "pending" => original.count_pending = true,
            _ => unreachable!(),
        }
        let mut events = [original.clone()];
        cache.infer_scoped(&mut events, &mut names, &BTreeMap::new(), &scope)?;
        assert!(events[0] == original, "{field}");
    }
    Ok(())
}

#[test]
fn scoped_namespace_does_not_override_request_context_or_conflicts() -> anyhow::Result<()> {
    let at = Local::now().timestamp_millis() - 600_000;
    let mut names = BTreeMap::from([("supplier-a".to_owned(), "Supplier A".to_owned())]);
    let mut original = event(at);
    original.model = "supplier-a/model-a".into();
    let mut cache = RequestEvidenceCache::default();
    let mut routed = request(at);
    routed.relay_id = "supplier-b".into();
    routed.final_response_id = Some("response-b".into());
    cache.records.push(routed.clone());
    let resolve = |event: &UsageEvent| Ok(event.terminal_response.clone());
    let scope = AttributionScope {
        targets: 0..1,
        gateway_blocked: false,
        native_blocked: false,
        paired_only: false,
        resolve_terminal: Some(&resolve),
    };
    let mut events = [original.clone()];
    cache.infer_scoped(&mut events, &mut names, &BTreeMap::new(), &scope)?;
    assert_eq!(events[0].provider, "supplier-b");
    assert_eq!(events[0].provider_source.as_deref(), Some("session-context-inferred"));
    events[0] = original.clone();
    events[0].terminal_response = Some("response-b".into());
    cache.infer_scoped(&mut events, &mut names, &BTreeMap::new(), &scope)?;
    assert_eq!(events[0].provider, "supplier-b");
    assert_eq!(events[0].provider_source.as_deref(), Some("request-log-recovered-supplier-only"));
    cache.captured_rejected.insert(routed.id.clone());
    events[0] = original.clone();
    cache.infer_scoped(&mut events, &mut names, &BTreeMap::new(), &scope)?;
    assert!(events[0] == original);
    cache.records[0] = RequestEvidence {
        id: routed.id.clone(),
        thread_id: SESSION.into(),
        timestamp_ms: at,
        ..RequestEvidence::default()
    };
    cache.infer_scoped(&mut events, &mut names, &BTreeMap::new(), &scope)?;
    assert!(events[0] == original, "rejected capture marker is not missing evidence");
    cache.records[0] = routed.clone();
    cache.captured_rejected.clear();
    routed.id = "competing".into();
    routed.relay_id = "supplier-c".into();
    cache.records.push(routed);
    events[0] = original.clone();
    cache.infer_scoped(&mut events, &mut names, &BTreeMap::new(), &scope)?;
    assert!(events[0] == original);
    cache.records.clear();
    cache.collision_families.insert(original.original_id);
    cache.infer_scoped(&mut events, &mut names, &BTreeMap::new(), &scope)?;
    assert!(events[0] == original);
    cache.collision_families.clear();
    cache.reject_conflicting_event(&original);
    cache.infer_scoped(&mut events, &mut names, &BTreeMap::new(), &scope)?;
    assert!(events[0] == original);
    Ok(())
}

#[test]
fn scoped_supplier_resolves_terminal_once_per_event() -> anyhow::Result<()> {
    let at = Local::now().timestamp_millis() - 600_000;
    let mut cache = RequestEvidenceCache::default();
    for n in 0..2048 {
        let mut proof = request(at - n * 1000);
        proof.id = format!("request-{n}");
        proof.final_response_id = Some(format!("response-{n}"));
        proof.relay_id = format!("supplier-{}", n % 2);
        cache.records.push(proof);
    }
    let lookups = std::cell::Cell::new(0);
    let resolve = |_: &UsageEvent| {
        lookups.set(lookups.get() + 1);
        Ok(Some(Box::from("response-2047")))
    };
    let scope = AttributionScope {
        targets: 0..1,
        gateway_blocked: false,
        native_blocked: false,
        paired_only: true,
        resolve_terminal: Some(&resolve),
    };
    let original = event(at);
    let mut events = [original.clone()];
    cache.infer_scoped(&mut events, &mut BTreeMap::new(), &BTreeMap::new(), &scope)?;
    assert_eq!(events[0].provider, "supplier-1");
    assert_eq!(events[0].provider_source.as_deref(), Some("request-log-recovered-supplier-only"));
    events[0].provider.clear();
    events[0].provider_source = None;
    assert!(events[0] == original);
    assert_eq!(lookups.get(), 1, "resolve the native receipt once, not once per request");
    events[0].inherited = true;
    cache.infer_scoped(&mut events, &mut BTreeMap::new(), &BTreeMap::new(), &scope)?;
    assert_eq!(lookups.get(), 1, "inherited records cannot bind the parent's receipt");
    assert_eq!(events[0].provider_source.as_deref(), Some("session-context-inferred"));
    for request in &mut cache.records {
        request.final_response_id = None;
    }
    events[0] = original;
    cache.infer_scoped(&mut events, &mut BTreeMap::new(), &BTreeMap::new(), &scope)?;
    assert_eq!(lookups.get(), 1, "requests without response identities need no receipt lookup");
    assert_eq!(events[0].provider_source.as_deref(), Some("session-context-inferred"));
    Ok(())
}

#[test]
fn scoped_supplier_identity_does_not_require_cache_accounting() -> anyhow::Result<()> {
    let at = Local::now().timestamp_millis() - 600_000;
    let mut proof = request(at);
    proof.final_response_id = Some("response-a".into());
    proof.final_response_evidence = Some("final-response-supplier-v1".into());
    proof.usage.as_mut().unwrap().cached_tokens = None;
    proof.usage.as_mut().unwrap().cached_tokens_known = Some(false);
    let mut cache = RequestEvidenceCache::default();
    cache.records.push(proof.clone());
    let mut events = [event(at)];
    let original_counts = events[0].delta;
    let resolve = |event: &UsageEvent| Ok(event.terminal_response.clone());
    let scope = AttributionScope {
        targets: 0..1,
        gateway_blocked: false,
        native_blocked: false,
        paired_only: true,
        resolve_terminal: Some(&resolve),
    };
    cache.infer_scoped(&mut events, &mut BTreeMap::new(), &BTreeMap::new(), &scope)?;
    assert_eq!(events[0].provider, "supplier-a");
    assert_eq!(events[0].provider_source.as_deref(), Some("session-context-inferred"));
    events[0].provider.clear();
    events[0].terminal_response = Some("response-a".into());
    cache.infer_scoped(&mut events, &mut BTreeMap::new(), &BTreeMap::new(), &scope)?;
    assert_eq!(events[0].provider_source.as_deref(), Some("request-log-recovered-supplier-only"));
    assert!(events[0].delta == original_counts);
    assert_eq!(cache.records[0].known_cached(), None);

    let mut other = proof;
    other.id = "request-b".into();
    other.relay_id = "supplier-b".into();
    other.final_response_id = Some("response-b".into());
    cache.records.push(other);
    events[0].provider.clear();
    cache.infer_scoped(&mut events, &mut BTreeMap::new(), &BTreeMap::new(), &scope)?;
    assert_eq!(events[0].provider, "supplier-a");
    cache.records[1].final_response_id = Some("response-a".into());
    events[0].provider.clear();
    events[0].provider_source = None;
    cache.infer_scoped(&mut events, &mut BTreeMap::new(), &BTreeMap::new(), &scope)?;
    assert!(unknown_provider(&events[0].provider));
    Ok(())
}

#[test]
fn scoped_context_uses_canonical_session_and_nearest_provider_switch() -> anyhow::Result<()> {
    let at = Local::now().timestamp_millis() - 600_000;
    let mut before = request(at - 300_000);
    before.thread_id = "session-alias".into();
    let mut after = request(at + 360_000);
    after.id = "request-b".into();
    after.relay_id = "supplier-b".into();
    let mut cache = RequestEvidenceCache::default();
    cache.records = vec![before, after];
    let aliases = BTreeMap::from([("session-alias".to_owned(), SESSION.to_owned())]);
    let scope = AttributionScope {
        targets: 0..1,
        gateway_blocked: false,
        native_blocked: false,
        paired_only: false,
        resolve_terminal: None,
    };
    let mut events = [event(at)];
    events[0].session = "session-alias".into();
    cache.infer_scoped(&mut events, &mut BTreeMap::new(), &aliases, &scope)?;
    assert_eq!(events[0].provider, "supplier-a");
    assert_eq!(events[0].provider_source.as_deref(), Some("session-context-inferred"));
    events[0] = event(at + 330_000);
    cache.infer_scoped(&mut events, &mut BTreeMap::new(), &aliases, &scope)?;
    assert_eq!(events[0].provider, "supplier-b");
    events[0] = event(at + 330_000);
    events[0].model = "unknown".into();
    cache.infer_scoped(&mut events, &mut BTreeMap::new(), &aliases, &scope)?;
    assert_eq!(events[0].provider, "supplier-b");
    assert_eq!(events[0].provider_source.as_deref(), Some("session-context-inferred"));
    cache.captured_rejected.insert("request-b".into());
    events[0].provider.clear();
    cache.infer_scoped(&mut events, &mut BTreeMap::new(), &aliases, &scope)?;
    assert!(unknown_provider(&events[0].provider));
    Ok(())
}

#[test]
fn scoped_context_survives_reopen_without_the_request_log() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let at = Local::now().timestamp_millis() - 600_000;
    let cutoff = Local::now().date_naive();
    let mut disk = provider_cache::ProviderCache::default();
    disk.open(root.path(), cutoff)?;
    let mut before = request(at - 300_000);
    before.final_response_id = Some("response-a".into());
    let mut after = request(at + 360_000);
    after.id = "request-b".into();
    after.relay_id = "supplier-b".into();
    disk.capture_proxy(&[before, after])?;
    drop(disk);
    let mut disk = provider_cache::ProviderCache::default();
    disk.open(root.path(), cutoff)?;
    assert!(disk.captured_proxy(SESSION, at - 120_000, at + 120_000)?.rows.is_empty());
    let scope = AttributionScope {
        targets: 0..1,
        gateway_blocked: false,
        native_blocked: false,
        paired_only: false,
        resolve_terminal: None,
    };
    let mut events = [event(at)];
    let mut cache = RequestEvidenceCache::default();
    cache.supplement_captured_scoped(&events, &BTreeMap::new(), &disk, Some(&scope));
    assert_eq!(cache.records.len(), 2);
    assert!(cache.records.iter().any(|r| r.final_response_id.as_deref() == Some("response-a")));
    cache.attribute_with_cache_scoped(&mut events, &mut BTreeMap::new(), &BTreeMap::new(), &disk, Some(&scope));
    assert!(cache.captured_id_check_error.is_none());
    cache.infer_scoped(&mut events, &mut BTreeMap::new(), &BTreeMap::new(), &scope)?;
    assert_eq!(events[0].provider, "supplier-a");
    assert_eq!(events[0].provider_source.as_deref(), Some("session-context-inferred"));
    Ok(())
}

#[test]
fn legacy_proxy_replay_recovers_long_competitors_and_keeps_rotated_windows_unproven() {
    let root = tempfile::tempdir().unwrap();
    let cutoff = Local::now().date_naive();
    let at = Local::now().timestamp_millis() - 600_000;
    let rotated_at = at - 1_000_000;
    let original = request(at);
    let mut unique = request(at + 300_000);
    unique.id = "unique".into();
    let mut rotated = request(rotated_at);
    rotated.id = "rotated".into();
    rotated.final_response_id = Some("terminal-a".into());
    rotated.final_response_evidence = Some("final-response-v1".into());
    let mut disk = provider_cache::ProviderCache::default();
    disk.open(root.path(), cutoff).unwrap();
    disk.capture_proxy(&[original.clone(), unique.clone(), rotated.clone()])
        .unwrap();
    let mut old = RequestEvidenceCache::default();
    old.records.push(rotated.clone());
    let mut earlier = event(rotated_at);
    earlier.id = [3; 32];
    attribute(&mut old, std::slice::from_mut(&mut earlier));
    assert_eq!(earlier.provider, "supplier-a");
    disk.save(&old.verified, &BTreeMap::new()).unwrap();
    drop(disk);
    let db =
        rusqlite::Connection::open(provider_cache::database_path(root.path()).unwrap()).unwrap();
    db.execute_batch("DELETE FROM capture_retention WHERE kind='proxy_legacy_until'; DROP TABLE proxy_replayed_windows;").unwrap();
    let receipt = |t| {
        json!([{
            "id":"live","sessionId":SESSION,"timestamp":t,
            "before":{"input":0,"output":0,"cached":0},"after":{"input":100,"output":10,"cached":20}
        }])
    };
    let check =
        |t| live_receipts::match_captured_receipts(root.path(), &[], &BTreeMap::new(), &receipt(t));
    assert_eq!(check(at)[0]["rejected"], true);
    let mut disk = provider_cache::ProviderCache::default();
    disk.open(root.path(), cutoff).unwrap();
    assert_eq!(check(rotated_at)[0]["rejected"], true);

    let mut competitor = original.clone();
    competitor.id = "long-line".into();
    competitor.relay_id = "supplier-b".into();
    let mut first = request(at - 130_000);
    first.thread_id = "other-session".into();
    first.id = "first".into();
    let mut last = request(at + 430_000);
    last.thread_id = "other-session".into();
    last.id = "last".into();
    let mut long = serde_json::to_value(&competitor).unwrap();
    long["displayOnly"] = json!("x".repeat(17_000));
    let mut rows = [
        json!(first),
        json!(original),
        long,
        json!(unique),
        json!(last),
    ];
    let path = root.path().join("request-log.jsonl");
    let mut classifier = RequestEvidenceCache::default();
    for stamped in [false, true] {
        for row in &mut rows {
            if stamped {
                row["appendedAtMs"] = row["timestampMs"].clone();
            }
        }
        let raw: String = rows.iter().map(|v| v.to_string() + "\n").collect();
        fs::write(&path, raw).unwrap();
        classifier.refresh(&path);
        assert_eq!(classifier.replay_window.is_some(), stamped);
        disk.capture_proxy_log(&classifier.records, classifier.replay_window)
            .unwrap();
        assert_eq!(check(at)[0]["rejected"], true);
        assert_eq!(check(at + 300_000)[0]["provider"] == "supplier-a", stamped);
    }
    fs::remove_file(&path).unwrap();
    classifier.refresh(&path);
    drop(disk);
    let mut disk = provider_cache::ProviderCache::default();
    disk.open(root.path(), cutoff).unwrap();
    assert_eq!(check(at)[0]["rejected"], true);
    assert_eq!(check(at + 300_000)[0]["provider"], "supplier-a");
    assert_eq!(check(rotated_at)[0]["rejected"], true);
    let uncovered = receipt(rotated_at)[0].clone();
    let mut covered = receipt(at + 300_000)[0].clone();
    covered["id"] = json!("independent-live");
    let mixed = live_receipts::match_captured_receipts(
        root.path(), &[], &BTreeMap::new(), &json!([uncovered, covered]),
    );
    let mixed = mixed.as_array().unwrap();
    assert!(mixed.iter().any(|r| r["id"] == "live" && r["rejected"] == true));
    assert!(mixed.iter().any(|r| r["id"] == "independent-live" && r["provider"] == "supplier-a"));

    classifier.restore_verified(disk.load_verified(cutoff).unwrap());
    earlier.provider.clear();
    earlier.provider_source = None;
    for response in [None, Some("terminal-a")] {
        let mut row = earlier.clone();
        row.terminal_response = response.map(Into::into);
        classifier.supplement_captured(std::slice::from_ref(&row), &BTreeMap::new(), &disk);
        classifier.attribute_with_cache(
            std::slice::from_mut(&mut row),
            &mut BTreeMap::new(),
            &BTreeMap::new(),
            &disk,
        );
        assert!(unknown_provider(&row.provider));
        assert!(!classifier.verified[&row.id].rejected);
    }
    let mut competing = rotated;
    competing.id = "another-rotated".into();
    competing.final_response_id = Some("another-terminal".into());
    competing.relay_id = "supplier-b".into();
    disk.capture_proxy(&[competing]).unwrap();
    earlier.terminal_response = Some("terminal-a".into());
    classifier.supplement_captured(std::slice::from_ref(&earlier), &BTreeMap::new(), &disk);
    classifier.attribute_with_cache(
        std::slice::from_mut(&mut earlier),
        &mut BTreeMap::new(),
        &BTreeMap::new(),
        &disk,
    );
    assert!(unknown_provider(&earlier.provider));
    assert!(classifier.verified[&earlier.id].rejected);
}

#[test]
fn concurrent_collector_preserves_fresh_rejections_and_retries_new_ones() {
    let root = tempfile::tempdir().unwrap();
    let cutoff = Local::now().date_naive();
    let now = Local::now().timestamp_millis();
    let mut disk = provider_cache::ProviderCache::default();
    disk.open(root.path(), cutoff).unwrap();
    let writer =
        rusqlite::Connection::open(provider_cache::database_path(root.path()).unwrap()).unwrap();
    let first = request(now);
    let mut second = request(now + 1);
    second.id = "second".into();
    disk.capture_proxy(&[first.clone(), second.clone()]).unwrap();
    let mut changed_first = first.clone();
    changed_first.relay_id = "conflict".into();
    let mut changed_second = second.clone();
    changed_second.relay_id = "conflict".into();
    let initial = disk.rejected_proxy_ids([&changed_first]).unwrap();
    assert_eq!(initial, HashSet::from([first.id.clone()]));
    writer.execute_batch("BEGIN IMMEDIATE").unwrap();
    assert_eq!(disk.rejected_proxy_ids([&first]).unwrap(), initial);
    let error = disk.rejected_proxy_ids([&first, &changed_second]).unwrap_err();
    assert_eq!(
        error.downcast_ref::<rusqlite::Error>().unwrap().sqlite_error_code(),
        Some(rusqlite::ErrorCode::DatabaseBusy)
    );
    assert_eq!(disk.rejected_proxy_ids_read_only([&first, &second]).unwrap(), initial);
    writer.execute_batch("COMMIT").unwrap();
    let both = HashSet::from([first.id.clone(), second.id.clone()]);
    assert_eq!(disk.rejected_proxy_ids([&first, &changed_second]).unwrap(), both);
    drop(disk);
    let reopened = provider_cache::ProviderCache::read_snapshot(root.path(), cutoff).unwrap();
    assert_eq!(reopened.rejected_proxy_ids_read_only([&first, &second]).unwrap(), both);
}

#[test]
fn legacy_proxy_coverage_read_error_uses_existing_validation_diagnostic() {
    let root = tempfile::tempdir().unwrap();
    let cutoff = Local::now().date_naive();
    let at = Local::now().timestamp_millis();
    let mut disk = provider_cache::ProviderCache::default();
    disk.open(root.path(), cutoff).unwrap();
    disk.capture_proxy(&[request(at)]).unwrap();
    let db =
        rusqlite::Connection::open(provider_cache::database_path(root.path()).unwrap()).unwrap();
    db.execute_batch(
        "UPDATE capture_retention SET floor=1 WHERE kind='proxy_legacy_until';
         ALTER TABLE proxy_replayed_windows RENAME TO unavailable;",
    )
    .unwrap();
    let mut classifier = RequestEvidenceCache::default();
    let mut row = event(at);
    for unavailable in [true, false] {
        if !unavailable {
            db.execute_batch("ALTER TABLE unavailable RENAME TO proxy_replayed_windows;")
                .unwrap();
        }
        let (_, limited) =
            classifier.supplement_captured(std::slice::from_ref(&row), &BTreeMap::new(), &disk);
        assert_eq!(limited, unavailable);
        classifier.attribute_with_cache(
            std::slice::from_mut(&mut row),
            &mut BTreeMap::new(),
            &BTreeMap::new(),
            &disk,
        );
        assert_eq!(classifier.captured_id_check_error.is_some(), unavailable);
        if let Some(error) = &classifier.captured_id_check_error {
            assert!(error.contains("read proxy coverage") && error.contains("proxy_replayed_windows"));
        }
        assert_eq!(unknown_provider(&row.provider), unavailable);
    }
}

#[test]
fn legacy_proxy_replay_rejects_out_of_order_physical_rows() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("request-log.jsonl");
    let at = Local::now().timestamp_millis();
    let mut later = json!(request(at + 10_000));
    later["id"] = json!("later");
    later["appendedAtMs"] = json!(at + 10_000);
    let mut earlier = json!(request(at));
    earlier["id"] = json!("earlier");
    earlier["appendedAtMs"] = json!(at);
    fs::write(
        &path,
        format!(
            "{}\n{}\n",
            serde_json::to_string(&later).unwrap(),
            serde_json::to_string(&earlier).unwrap()
        ),
    )
    .unwrap();
    let mut cache = RequestEvidenceCache::default();
    cache.refresh(&path);
    assert_eq!(cache.records.len(), 2);
    assert_eq!(cache.replay_window, None);
}

#[test]
fn captured_gateway_includes_all_target_sessions_and_their_aliases() {
    let at = Local::now().timestamp_millis();
    let root = tempfile::tempdir().unwrap();
    let mut disk = provider_cache::ProviderCache::default();
    disk.open(root.path(), Local::now().date_naive()).unwrap();
    let mut events = Vec::new();
    let mut requests = Vec::new();
    for i in 0..257u32 {
        let mut event = event(at);
        event.id[..4].copy_from_slice(&i.to_le_bytes());
        event.original_id = event.id;
        event.session = format!("thread-{i}");
        let mut completed = request(at);
        completed.thread_id = event.session.clone();
        completed.id = format!("request-{i}");
        events.push(event);
        requests.push(completed);
    }
    requests[0].thread_id = "alias-16".into();
    let aliases = (0..17).map(|i| (format!("alias-{i}"), "thread-0".to_owned())).collect();
    disk.capture_proxy(&requests).unwrap();
    let original = events.clone();
    let mut classifier = RequestEvidenceCache::default();
    assert_eq!(classifier.supplement_captured(&events, &aliases, &disk), (257, false));
    classifier.attribute_with_cache(&mut events, &mut BTreeMap::new(), &aliases, &disk);
    assert!(events.iter().all(|event| event.provider == "supplier-a"));
    let mut competitor = request(at);
    competitor.thread_id = "alias-0".into();
    competitor.relay_id = "supplier-b".into();
    disk.capture_proxy(&[competitor]).unwrap();
    let mut events = original;
    assert_eq!(classifier.supplement_captured(&events, &aliases, &disk), (258, false));
    classifier.attribute_with_cache(&mut events, &mut BTreeMap::new(), &aliases, &disk);
    assert!(unknown_provider(&events[0].provider));
    assert!(events[1..].iter().all(|event| event.provider == "supplier-a"));
}

#[test]
fn request_log_reads_all_finished_rows_and_leaves_the_incomplete_tail_for_retry() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("request-log.jsonl");
    let mut log = format!("{{\"id\":\"first\",\"error\":\"{}\"}}\n", "x".repeat(4 * 1024 * 1024));
    for i in 0..8_192 {
        log.push_str(&format!("{{\"id\":\"{i}\"}}\n"));
    }
    log.push_str("{\"id\":\"malformed\",\"timestampMs\":false}\n{\"id\":\"tail\"}");
    fs::write(&path, &log).unwrap();
    let mut cache = RequestEvidenceCache::default();
    cache.refresh(&path);
    assert_eq!(cache.bytes_read, log.len() as u64);
    assert_eq!(cache.records.len(), 8_194);
    assert_eq!(cache.records.first().unwrap().id, "first");
    assert_eq!(cache.records.last().unwrap().id, "malformed");
    assert!(!cache.records.last().unwrap().valid_capture());
    log.push('\n');
    fs::write(&path, log).unwrap();
    cache.refresh(&path);
    assert_eq!(cache.records.len(), 8_195);
    assert_eq!(cache.records.last().unwrap().id, "tail");
}

#[test]
fn final_response_model_requires_compatible_native_context_and_a_complete_receipt() {
    let at = Local::now().timestamp_millis();
    let archived = tempfile::tempdir().unwrap();
    let mut disk = provider_cache::ProviderCache::default();
    disk.open(archived.path(), Local::now().date_naive()).unwrap();
    for (index, (provider, model, marker, response_id, expected)) in [
        ("", "model-a", true, true, "served-model"),
        ("supplier-a", "model-a", true, true, "served-model"),
        ("", "model-a", true, false, "model-a"),
        ("", "model-a", false, true, "model-a"),
        ("", "another-native", true, true, "another-native"),
        ("supplier-b", "model-a", true, true, "model-a"),
    ]
    .into_iter()
    .enumerate()
    {
        let time = at - index as i64 * 300_000;
        let mut completed = request(time);
        completed.id = format!("terminal-request-{index}");
        completed.upstream_model = "served-model".into();
        completed.final_response_evidence = marker.then(|| "final-response-v1".into());
        completed.final_response_id = response_id.then(|| format!("response-{index}"));
        disk.capture_proxy(&[completed]).unwrap();
        let mut classifier = RequestEvidenceCache::default();
        let mut native = event(time);
        native.provider = provider.into();
        native.model = model.into();
        let counts = native.delta;
        let mut events = [native];
        classifier.supplement_captured(&events, &BTreeMap::new(), &disk);
        classifier.attribute_with_cache(&mut events, &mut BTreeMap::new(), &BTreeMap::new(), &disk);
        assert_eq!(events[0].model, expected, "case {index}");
        assert_eq!(events[0].delta, counts);
        if model == "another-native" || provider == "supplier-b" {
            assert_ne!(events[0].provider_source.as_deref(), Some("request-log-exact"));
        } else {
            assert_eq!(events[0].provider, "supplier-a");
            assert_eq!(events[0].provider_source.as_deref(), Some("request-log-exact"));
        }
    }
}

#[test]
fn successful_request_without_usage_still_blocks_a_competing_proof_after_rotation() {
    let root = tempfile::tempdir().unwrap();
    let at = Local::now().timestamp_millis();
    let cutoff = Local::now().date_naive();
    let exact = request(at);
    let mut competitor = request(at + 1);
    competitor.id = "request-without-counts".into();
    competitor.relay_id = "supplier-b".into();
    competitor.usage = None;
    competitor.final_response_id = Some("response-without-counts".into());
    let mut disk = provider_cache::ProviderCache::default();
    disk.open(root.path(), cutoff).unwrap();
    assert!(
        disk.rejected_proxy_ids_read_only([&competitor])
            .unwrap()
            .is_empty()
    );
    disk.capture_proxy(&[exact.clone(), competitor.clone()])
        .unwrap();
    drop(disk);

    let mut disk = provider_cache::ProviderCache::default();
    disk.open(root.path(), cutoff).unwrap();
    assert!(
        disk.rejected_proxy_ids([&exact, &competitor])
            .unwrap()
            .is_empty()
    );
    let saved = disk.captured_proxy(SESSION, at - 1000, at + 1000).unwrap();
    assert!(saved.rows.contains(&competitor));
    let original = event(at);
    let mut events = [original.clone()];
    let mut cache = RequestEvidenceCache::default();
    cache.supplement_captured(&events, &BTreeMap::new(), &disk);
    cache.attribute_with_cache(&mut events, &mut BTreeMap::new(), &BTreeMap::new(), &disk);
    assert!(unknown_provider(&events[0].provider));
    assert!(cache.verified.is_empty());
    assert_eq!(events[0].delta, original.delta);

    competitor.relay_id = "conflicting-supplier".into();
    disk.capture_proxy(std::slice::from_ref(&competitor))
        .unwrap();
    assert!(
        disk.rejected_proxy_ids_read_only([&competitor])
            .unwrap()
            .contains(&competitor.id)
    );
}

#[test]
fn enriched_disk_observation_keeps_legacy_projection_after_reopen() {
    let root = tempfile::tempdir().unwrap();
    let at = Local::now().timestamp_millis();
    let cutoff = Local::now().date_naive() - Days::days(1);
    let legacy = request(at);
    let mut wire = serde_json::to_value(&legacy).unwrap();
    wire["finalResponseId"] = json!("response-a");
    wire["usage"]["cachedTokensKnown"] = json!(true);
    let mut disk = provider_cache::ProviderCache::default();
    disk.open(root.path(), cutoff).unwrap();
    disk.capture_proxy(std::slice::from_ref(&legacy)).unwrap();
    let current: RequestEvidence = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(
        disk.capture_proxy(std::slice::from_ref(&current)).unwrap(),
        1
    );
    drop(disk);
    let db =
        rusqlite::Connection::open(provider_cache::database_path(root.path()).unwrap()).unwrap();
    let legacy_data: String = db
        .query_row("SELECT data FROM completed_proxy_requests", [], |row| {
            row.get(0)
        })
        .unwrap();
    // 旧程序去掉 relayName 后比较两个完整的 JSON 对象。
    assert_eq!(
        serde_json::from_str::<Value>(&legacy_data).unwrap(),
        serde_json::to_value(&legacy).unwrap()
    );
    assert_eq!(
        db.execute(
            "INSERT OR IGNORE INTO completed_proxy_requests VALUES(?1,?2,?3,?4)",
            rusqlite::params![
                legacy.id,
                legacy.timestamp_ms,
                legacy.thread_id,
                legacy_data
            ],
        )
        .unwrap(),
        0
    );
    drop(db);

    let mut disk = provider_cache::ProviderCache::default();
    disk.open(root.path(), cutoff).unwrap();
    assert!(
        disk.rejected_proxy_ids_read_only([&current])
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        disk.capture_proxy(std::slice::from_ref(&current)).unwrap(),
        0
    );
    assert_eq!(disk.capture_proxy(&[legacy]).unwrap(), 0);
    let mut cache = RequestEvidenceCache::default(); // 轮转后仅剩 SQLite 证据。
    let original = event(at);
    let mut events = [original.clone()];
    cache.supplement_captured(&events, &BTreeMap::new(), &disk);
    cache.attribute_with_cache(&mut events, &mut BTreeMap::new(), &BTreeMap::new(), &disk);
    assert_eq!(events[0].provider, "supplier-a");
    assert_eq!(events[0].id, original.id);
    assert_eq!(events[0].delta, original.delta);

    wire["finalResponseId"] = json!("response-b");
    let conflicting: RequestEvidence = serde_json::from_value(wire).unwrap();
    disk.capture_proxy(&[conflicting]).unwrap();
    assert!(
        disk.rejected_proxy_ids_read_only([&current])
            .unwrap()
            .contains(&current.id)
    );
    let db =
        rusqlite::Connection::open(provider_cache::database_path(root.path()).unwrap()).unwrap();
    let extra: usize = db
        .query_row(
            "SELECT count(*) FROM completed_proxy_request_extra",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(extra, 0);
}

#[test]
fn rotated_legacy_verified_zero_is_reused_only_by_its_original_event() {
    let at = Local::now().timestamp_millis();
    let root = tempfile::tempdir().unwrap();
    let cutoff = Local::now().date_naive() - Days::days(1);
    let mut disk = provider_cache::ProviderCache::default();
    disk.open(root.path(), cutoff).unwrap();
    let mut legacy = request(at);
    legacy.usage.as_mut().unwrap().cached_tokens = Some(0);
    let mut original = event(at);
    original.delta = Tokens::new(100, 0, 10);
    original.total = original.delta;
    let saved = VerifiedAttribution {
        key: RequestMatchKey {
            session: original.session.clone(),
            model: original.model.clone(),
            input: 100,
            output: 10,
            cached: 0,
        },
        at,
        request: legacy.clone(),
        rejected: false,
        version: 1,
    };
    disk.capture_proxy(std::slice::from_ref(&legacy)).unwrap();
    disk.save(&HashMap::from([(original.id, saved)]), &BTreeMap::new())
        .unwrap();
    drop(disk);

    let mut disk = provider_cache::ProviderCache::default();
    disk.open(root.path(), cutoff).unwrap();
    let mut cache = RequestEvidenceCache::default();
    cache.restore_verified(disk.load_verified(cutoff).unwrap());
    let mut events = [original.clone()];
    cache.supplement_captured(&events, &BTreeMap::new(), &disk);
    let apply = |cache: &mut RequestEvidenceCache, events: &mut [UsageEvent]| {
        cache.attribute_with_cache(events, &mut BTreeMap::new(), &BTreeMap::new(), &disk);
    };
    apply(&mut cache, &mut events);
    assert_eq!(
        events[0].provider_source.as_deref(),
        Some("request-log-exact")
    );
    assert_eq!(events[0].provider, "supplier-a");
    assert_eq!(events[0].delta, original.delta);
    assert!(!cache.verified[&original.id].rejected);
    assert!(
        cache.verified[&original.id]
            .request
            .known_cached()
            .is_none()
    );

    let mut other = original.clone();
    other.id = [8; 32];
    other.original_id = other.id;
    let mut events = [other];
    apply(&mut cache, &mut events);
    assert!(unknown_provider(&events[0].provider));
    assert!(!cache.verified.contains_key(&events[0].id));

    let mut conflict = legacy;
    let usage = conflict.usage.as_mut().unwrap();
    usage.cached_tokens = Some(1);
    usage.cached_tokens_known = Some(true);
    cache.records = vec![conflict];
    let mut events = [original.clone()];
    apply(&mut cache, &mut events);
    assert!(unknown_provider(&events[0].provider));
    assert!(cache.verified[&original.id].rejected);
    disk.save(&cache.verified, &BTreeMap::new()).unwrap();
    drop(disk);

    let mut disk = provider_cache::ProviderCache::default();
    disk.open(root.path(), cutoff).unwrap();
    let mut cache = RequestEvidenceCache::default();
    cache.restore_verified(disk.load_verified(cutoff).unwrap());
    assert!(cache.verified[&original.id].rejected);
}

#[test]
fn missing_cache_waits_for_evidence_and_never_replaces_a_known_count() {
    let at = Local::now().timestamp_millis();
    for exact in [0, 20] {
        let root = tempfile::tempdir().unwrap();
        let cutoff = Local::now().date_naive() - Days::days(1);
        let mut disk = provider_cache::ProviderCache::default();
        disk.open(root.path(), cutoff).unwrap();
        let mut pending = request(at);
        pending.usage.as_mut().unwrap().cached_tokens = Some(0);
        let mut confirmed = pending.clone();
        confirmed.usage.as_mut().unwrap().cached_tokens = Some(exact);
        confirmed.usage.as_mut().unwrap().cached_tokens_known = Some(true);
        confirmed.final_response_id = Some("response-a".into());
        let mut original = event(at);
        original.delta = Tokens::new(100, exact, 10);
        original.total = original.delta;
        for flag in [None, Some(false)] {
            pending.usage.as_mut().unwrap().cached_tokens_known = flag;
            assert!(pending.keys().is_empty());
            disk.capture_proxy(std::slice::from_ref(&pending)).unwrap();
            let mut cache = RequestEvidenceCache::default();
            let mut events = [original.clone()];
            cache.supplement_captured(&events, &BTreeMap::new(), &disk);
            cache.attribute_with_cache(&mut events, &mut BTreeMap::new(), &BTreeMap::new(), &disk);
            assert!(unknown_provider(&events[0].provider));
            assert!(
                disk.rejected_proxy_ids_read_only([&pending])
                    .unwrap()
                    .is_empty()
            );
        }
        disk.capture_proxy(std::slice::from_ref(&confirmed))
            .unwrap();
        drop(disk);
        let mut disk = provider_cache::ProviderCache::default();
        disk.open(root.path(), cutoff).unwrap();
        assert_eq!(
            disk.capture_proxy(std::slice::from_ref(&pending)).unwrap(),
            0
        );
        let mut cache = RequestEvidenceCache::default();
        cache.records.push(pending.clone());
        let mut events = [original.clone()];
        cache.supplement_captured(&events, &BTreeMap::new(), &disk);
        cache.attribute_with_cache(&mut events, &mut BTreeMap::new(), &BTreeMap::new(), &disk);
        assert_eq!(
            events[0].provider_source.as_deref(),
            Some("request-log-exact")
        );
        assert_eq!(events[0].id, original.id);
        assert_eq!(events[0].delta, original.delta);
        assert_eq!(cache.records.len(), 1);
        let mut conflict = confirmed.clone();
        conflict.usage.as_mut().unwrap().cached_tokens = Some(exact + 1);
        assert!(
            disk.rejected_proxy_ids([&conflict, &pending])
                .unwrap()
                .contains(&pending.id)
        );
        assert!(
            disk.rejected_proxy_ids_read_only([&confirmed])
                .unwrap()
                .contains(&pending.id)
        );
    }
}

#[test]
fn sparse_observation_cannot_bridge_contradictory_receipts() {
    let at = Local::now().timestamp_millis();
    let root = tempfile::tempdir().unwrap();
    let mut disk = provider_cache::ProviderCache::default();
    disk.open(root.path(), Local::now().date_naive()).unwrap();
    let sparse = request(at);
    let mut a = sparse.clone();
    a.final_response_id = Some("response-a".into());
    let mut b = sparse.clone();
    b.final_response_id = Some("response-b".into());
    for rows in [[&a, &sparse, &b], [&sparse, &b, &a], [&b, &a, &sparse]] {
        assert!(
            disk.rejected_proxy_ids_read_only(rows)
                .unwrap()
                .contains(&a.id)
        );
    }
    assert!(disk.rejected_proxy_ids_read_only([&a]).unwrap().is_empty());
    disk.capture_proxy(&[a.clone(), a.clone()]).unwrap();
    assert!(
        disk.rejected_proxy_ids_read_only([&a])
            .unwrap()
            .contains(&a.id)
    );
}

#[test]
fn ordinary_model_namespace_is_not_provider_evidence() {
    let names = BTreeMap::from([
        ("namespace".to_owned(), "Namespace".to_owned()),
        ("relay/one".to_owned(), "Relay One".to_owned()),
    ]);
    assert!(unknown_provider(usage_provider(
        "namespace/model",
        "custom",
        &names
    )));
    assert_eq!(
        usage_provider("namespace/model", "supplier-b", &names),
        "supplier-b"
    );
    assert_eq!(usage_provider("namespace/model", "openai", &names), "");

    let aggregate =
        codex_plus_core::aggregate_catalog::aggregate_model_id("relay/one", "upstream/model");
    assert!(unknown_provider(usage_provider(
        &aggregate, "custom", &names
    )));
    assert_eq!(
        usage_provider(&aggregate, "supplier-b", &names),
        "supplier-b"
    );
    assert!(!unknown_model(&aggregate));
    assert!(!unknown_model("cpx-member/group/auto"));
    assert!(unknown_provider(usage_provider(
        "cpx-removed/model",
        "custom",
        &names
    )));
}

#[test]
fn routed_supplier_requires_successful_request_evidence() {
    let at = Local::now().timestamp_millis();
    let route = codex_plus_core::aggregate_catalog::aggregate_model_id("supplier-a", "model-a");
    let names = BTreeMap::from([("supplier-a".to_owned(), "Supplier A".to_owned())]);
    let mut routed = event(at);
    routed.model = route;
    routed.provider = usage_provider(&routed.model, "custom", &names).to_owned();
    assert!(unknown_provider(&routed.provider));

    let before = routed.delta;
    let mut successful = request(at);
    successful.model.clone_from(&routed.model);
    successful.relay_id = "supplier-b".into();
    successful.relay_name = "Supplier B".into();
    let mut cache = RequestEvidenceCache::default();
    cache.records.push(successful);
    let mut events = vec![routed];
    assert_eq!(attribute(&mut cache, &mut events)["recoveredEvents"], 1);
    assert_eq!(events[0].provider, "supplier-b");
    assert_eq!(events[0].delta, before);
}

#[test]
fn provider_placeholders_can_be_recovered_only_by_exact_request() {
    let at = Local::now().timestamp_millis();
    for provider in [
        "",
        "unknown",
        "CUSTOM",
        "codex_plus_managed",
        " Codex-Plus-Gateway ",
        " Null ",
        "UNDEFINED",
    ] {
        assert!(unknown_provider(provider), "{provider}");
        let mut cache = RequestEvidenceCache::default();
        cache.records.push(request(at));
        let mut candidate = event(at);
        candidate.provider = provider.into();
        let mut events = vec![candidate.clone()];
        assert_eq!(attribute(&mut cache, &mut events)["recoveredEvents"], 1);
        assert_eq!(events[0].provider, "supplier-a");

        candidate.delta.cached_known = false;
        let mut unverified = vec![candidate];
        let mut fresh = RequestEvidenceCache::default();
        fresh.records.push(request(at));
        assert_eq!(attribute(&mut fresh, &mut unverified)["recoveredEvents"], 0);
        assert!(unknown_provider(&unverified[0].provider));
    }
    assert!(!unknown_provider("supplier-a"));
}

#[test]
fn model_placeholders_are_not_exact_matching_keys() {
    let at = Local::now().timestamp_millis();
    for model in [
        "",
        " Unknown ",
        "DEFAULT",
        "null",
        "undefined",
        "AUTO",
        "Group/daily",
    ] {
        assert!(unknown_model(model), "{model}");
        let mut first = event(at);
        first.model = model.into();
        let known = event(at);
        first.merge_duplicate_context(&known, &BTreeMap::new());
        assert_eq!(first.model, "model-a");
        assert!(!first.context_conflicts.any());

        let mut ambiguous = event(at);
        ambiguous.id = [8; 32];
        ambiguous.model = model.into();
        let mut events = vec![event(at), ambiguous];
        let mut cache = RequestEvidenceCache::default();
        cache.records.push(request(at));
        assert_eq!(attribute(&mut cache, &mut events)["recoveredEvents"], 0);
        assert!(events.iter().all(|event| unknown_provider(&event.provider)));

        let mut invalid_request = request(at);
        invalid_request.model = model.into();
        invalid_request.upstream_model = model.into();
        assert!(invalid_request.keys().is_empty());
    }
    assert!(!unknown_model("namespace/model"));

    let mut routed = request(at);
    routed.model = "group/daily".into();
    assert_eq!(routed.keys().len(), 2);
    assert_eq!(routed.keys()[0].model, "model-a");
    assert_eq!(routed.keys()[1].model, "");
}

#[test]
fn exact_event_evidence_fills_placeholder_model_only_when_counts_match() {
    let dir = tempfile::tempdir().unwrap();
    let settings = dir.path().join("settings.json");
    let mut original = event(Local::now().timestamp_millis());
    original.model = "group/daily".into();
    original.provider = "codex-plus-gateway".into();
    let mut proof = json!({
        "eventId": original.id.iter().map(|byte| format!("{byte:02x}")).collect::<String>(),
        "session": original.session, "timestamp": original.timestamp, "model": "model-a",
        "role": original.role, "input": 100, "output": 10, "cached": 20,
        "provider": "supplier-a", "providerName": "Supplier A",
        "source": "upstream-exact-bidirectional"
    });
    for valid in [true, false] {
        proof["input"] = json!(if valid { 100 } else { 101 });
        fs::write(
            settings.with_file_name("usage-provider-evidence.json"),
            serde_json::to_vec(&json!({"version": 1, "records": [proof.clone()]})).unwrap(),
        )
        .unwrap();
        let mut index = Index::default();
        let mut snapshot = Snapshot {
            events: vec![original.clone()],
            ..Snapshot::default()
        };
        index.supplement_legacy_evidence(&settings, &mut snapshot);
        assert_eq!(
            snapshot.provider_evidence["appliedEvents"],
            usize::from(valid)
        );
        if valid {
            assert_eq!(snapshot.events[0].model, "model-a");
            assert_eq!(snapshot.events[0].provider, "supplier-a");
        } else {
            assert!(unknown_model(&snapshot.events[0].model));
            assert!(unknown_provider(&snapshot.events[0].provider));
        }
    }
}

#[test]
fn duplicate_provider_conflicts_cannot_be_masked_by_model_namespace() {
    let names = BTreeMap::from([("namespace".to_owned(), "Namespace".to_owned())]);
    let mut first = event(Local::now().timestamp_millis());
    first.model = "namespace/model".into();
    first.provider = "supplier-a".into();
    let mut second = first.clone();
    second.provider = "supplier-b".into();

    for (mut left, right) in [(first.clone(), second.clone()), (second, first)] {
        left.merge_duplicate_context(&right, &names);
        assert!(left.context_conflicts.provider);
        assert_eq!(left.provider, "");
        left.merge_duplicate_context(&right, &names);
        assert!(left.context_conflicts.provider);
        assert_eq!(left.provider, "");
    }
}

#[test]
fn conflicting_saved_attribution_stays_rejected_after_reopen() {
    let at = Local::now().timestamp_millis();
    let cutoff = Local::now().date_naive() - Days::days(RETENTION_DAYS - 1);
    for case in ["model", "provider", "role", "turn", "recorded-provider"] {
        let dir = tempfile::tempdir().unwrap();
        let mut disk = provider_cache::ProviderCache::default();
        disk.open(dir.path(), cutoff).unwrap();
        let mut cache = RequestEvidenceCache::default();
        cache.records.push(request(at));
        let mut original = vec![event(at)];
        assert_eq!(attribute(&mut cache, &mut original)["recoveredEvents"], 1);
        disk.save(&cache.verified, &BTreeMap::new()).unwrap();

        let mut conflicting = event(at);
        match case {
            "model" => conflicting.context_conflicts.model = true,
            "provider" => conflicting.context_conflicts.provider = true,
            "role" => conflicting.context_conflicts.role = true,
            "turn" => conflicting.context_conflicts.turn = true,
            "recorded-provider" => conflicting.provider = "supplier-b".into(),
            _ => unreachable!(),
        }
        let mut events = vec![conflicting];
        assert_eq!(
            attribute(&mut cache, &mut events)["recoveredEvents"],
            0,
            "{case}"
        );
        assert!(cache.verified[&events[0].id].rejected, "{case}");
        if case == "recorded-provider" {
            assert!(events[0].provider.is_empty());
            assert!(events[0].context_conflicts.provider);
        }
        disk.save(&cache.verified, &BTreeMap::new()).unwrap();
        drop(disk);

        let mut reopened = provider_cache::ProviderCache::default();
        reopened.open(dir.path(), cutoff).unwrap();
        let mut restored = RequestEvidenceCache::default();
        restored.restore_verified(reopened.load_verified(cutoff).unwrap());
        restored.records.push(request(at));
        let mut later = vec![event(at)];
        assert_eq!(
            attribute(&mut restored, &mut later)["recoveredEvents"],
            0,
            "{case}"
        );
        assert!(unknown_provider(&later[0].provider), "{case}");
        assert!(restored.verified[&later[0].id].rejected, "{case}");
    }
}

#[test]
fn unloaded_conflict_and_failed_save_do_not_restore_an_old_proof() {
    let at = Local::now().timestamp_millis();
    let cutoff = Local::now().date_naive() - Days::days(RETENTION_DAYS - 1);
    let dir = tempfile::tempdir().unwrap();
    let mut disk = provider_cache::ProviderCache::default();
    disk.open(dir.path(), cutoff).unwrap();
    let mut original = RequestEvidenceCache::default();
    original.records.push(request(at));
    assert_eq!(
        attribute(&mut original, &mut [event(at)])["recoveredEvents"],
        1
    );
    disk.save(&original.verified, &BTreeMap::new()).unwrap();

    let mut pending = RequestEvidenceCache::default();
    let mut conflicting = event(at);
    conflicting.context_conflicts.model = true;
    attribute(&mut pending, &mut [conflicting]);
    assert!(pending.verified[&event(at).id].rejected);
    let mut unavailable = provider_cache::ProviderCache::default();
    assert!(
        unavailable
            .save(&pending.verified, &BTreeMap::new())
            .is_err()
    );

    pending.restore_verified(disk.load_verified(cutoff).unwrap());
    assert!(pending.verified[&event(at).id].rejected);
    disk.save(&pending.verified, &BTreeMap::new()).unwrap();
    drop(disk);

    let mut reopened = provider_cache::ProviderCache::default();
    reopened.open(dir.path(), cutoff).unwrap();
    let mut restored = RequestEvidenceCache::default();
    restored.restore_verified(reopened.load_verified(cutoff).unwrap());
    restored.records.push(request(at));
    let mut later = vec![event(at)];
    assert_eq!(attribute(&mut restored, &mut later)["recoveredEvents"], 0);
    assert!(unknown_provider(&later[0].provider));
    assert!(restored.verified[&later[0].id].rejected);
}
