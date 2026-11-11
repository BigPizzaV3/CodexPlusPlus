use super::*;

const SESSION: &str = "00000000-0000-4000-8000-000000000010";

fn event(at: i64) -> UsageEvent {
    UsageEvent {
        id: [7; 32],
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
    assert!(!unknown_model("cpx-model"));
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
fn supplier_identity_survives_missing_or_renamed_display_name() {
    let at = Local::now().timestamp_millis();
    let mut record = request(at);
    record.relay_name.clear();
    let mut names = BTreeMap::new();
    let mut events = vec![event(at)];
    let mut cache = RequestEvidenceCache::default();
    cache.records.push(record.clone());
    let before = events[0].delta;
    assert_eq!(
        cache.attribute(&mut events, &mut names, &BTreeMap::new())["recoveredEvents"],
        1
    );
    assert_eq!(events[0].provider, "supplier-a");
    assert_eq!(events[0].delta, before);
    assert_eq!(names["supplier-a"], "supplier-a");

    record.relay_name = "Renamed supplier".into();
    cache.records[0] = record;
    let mut events = vec![event(at)];
    assert_eq!(attribute(&mut cache, &mut events)["recoveredEvents"], 1);
    assert_eq!(events[0].provider, "supplier-a");
    assert!(!cache.verified[&events[0].id].rejected);

    cache.records[0].relay_name = "x".repeat(300);
    let mut events = vec![event(at)];
    assert_eq!(attribute(&mut cache, &mut events)["recoveredEvents"], 1);
}

#[test]
fn captured_supplier_rename_does_not_invalidate_request_identity() {
    let at = Local::now().timestamp_millis();
    let dir = tempfile::tempdir().unwrap();
    let mut disk = provider_cache::ProviderCache::default();
    disk.open(dir.path(), Local::now().date_naive() - Days::days(1))
        .unwrap();
    let mut record = request(at);
    disk.capture_proxy(&[record.clone()]).unwrap();
    record.relay_name = "Renamed supplier".into();
    disk.capture_proxy(&[record.clone()]).unwrap();
    assert!(
        disk.rejected_proxy_ids([&record]).unwrap().is_empty(),
        "changing a display name must not reject the stable request identity"
    );
    let mut cache = RequestEvidenceCache::default();
    cache.records.push(record);
    let mut events = vec![event(at)];
    assert_eq!(
        cache.attribute_with_cache(&mut events, &mut BTreeMap::new(), &BTreeMap::new(), &disk)
            ["recoveredEvents"],
        1
    );
    assert_eq!(events[0].provider, "supplier-a");
}

#[test]
fn uncertain_events_only_compete_with_requests_in_their_window() {
    let at = Local::now().timestamp_millis();
    let mut record = request(at);
    record.timestamp_ms = at - DELAYED_REQUEST_MATCH_WINDOW_MS + 10;
    record.started_at_ms = record.timestamp_ms - 100;
    for (offset, time_valid, same_session, expected) in [
        (-DELAYED_REQUEST_MATCH_WINDOW_MS, true, true, 0),
        (-DELAYED_REQUEST_MATCH_WINDOW_MS - 1, true, true, 1),
        (0, false, true, 0),
        (0, true, false, 1),
    ] {
        let mut other = event(at);
        other.id = [8; 32];
        other.count_uncertain = true;
        other.delta = Tokens::new(20, 0, 2);
        other.timestamp = if time_valid {
            DateTime::from_timestamp_millis(record.timestamp_ms + offset)
                .unwrap()
                .to_rfc3339()
        } else {
            "not-a-timestamp".into()
        };
        if !same_session {
            other.session = "other-session".into();
        }
        let mut events = vec![event(at), other];
        let mut cache = RequestEvidenceCache::default();
        cache.records.push(record.clone());
        let result = attribute(&mut cache, &mut events);
        assert_eq!(
            result["recoveredEvents"], expected,
            "offset={offset}, time_valid={time_valid}, same_session={same_session}"
        );
        assert_eq!(events[0].delta, event(at).delta);
    }
}

#[test]
fn cached_zero_requires_presence_evidence_without_changing_counts() {
    let at = Local::now().timestamp_millis();
    for (known, strong, expected) in [
        (Some(false), false, 0),
        (None, false, 0),
        (Some(true), false, 1),
        (None, true, 1),
    ] {
        let mut encoded = serde_json::to_value(request(at)).unwrap();
        encoded["usage"]["cachedTokens"] = json!(0);
        if let Some(known) = known {
            encoded["usage"]["cachedTokensKnown"] = json!(known);
        }
        if strong {
            encoded["finalResponseEvidence"] = json!("final-response-v1");
        }
        let record: RequestEvidence = serde_json::from_value(encoded).unwrap();
        let mut original = event(at);
        original.delta = Tokens::new(100, 0, 10);
        let mut events = vec![original.clone()];
        let mut cache = RequestEvidenceCache::default();
        cache.records.push(record);
        assert_eq!(
            attribute(&mut cache, &mut events)["recoveredEvents"],
            expected,
            "known={known:?}, strong={strong}"
        );
        assert_eq!(events[0].delta, original.delta);
    }
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
    assert!(!unknown_model("cpx-member/model-a"));

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
