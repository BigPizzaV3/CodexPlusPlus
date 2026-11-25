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
