use super::*;

fn sandbox() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

fn event(number: u8) -> UsageEvent {
    let counts = Tokens::new(100, 80, 7);
    UsageEvent {
        native_response_id: None,
        terminal_response: None,
        id: [number; 32],
        original_id: [number; 32],
        session: "00000000-0000-4000-8000-000000000001".into(),
        turn: String::new(),
        model: "recorded-model".into(),
        timestamp: "2026-10-30T00:00:00Z".into(),
        date: NaiveDate::from_ymd_opt(2026, 10, 30).unwrap(),
        total: counts,
        delta: counts,
        has_file_baseline: true,
        rollout_position: None,
        count_pending: false,
        last: Some(counts),
        count_uncertain: false,
        count_incomplete: false,
        provider: "custom".into(),
        provider_source: None,
        role: "main".into(),
        inherited: false,
        legacy_identity: None,
        context_conflicts: ContextConflicts::default(),
    }
}

fn row(event: &UsageEvent, number: u64) -> Value {
    let id: String = event.id.iter().map(|byte| format!("{byte:02x}")).collect();
    let upstream = format!("fixture-supplier:bill/{number}");
    json!({
        "event":{"eventId":id,"session":event.session,"timestamp":event.timestamp,
            "recordedModel":event.model,"role":event.role,"input":event.delta.input_tokens,
            "output":event.delta.output_tokens,"cached":event.delta.cached_input_tokens},
        "supplierEvidence":{"provider":"supplier-test","providerName":"Test supplier","source":SOURCE,
            "upstream":{"supplier":"fixture-supplier","id":upstream,
                "requestId":format!("opaque/request/{number}"),
                "sessionId":event.session,"timestamp":event.timestamp,"model":"upstream-model",
                "input":event.delta.input_tokens,"output":event.delta.output_tokens,"cached":event.delta.cached_input_tokens},
            "sessionBinding":{"field":"usage.items[].session_id","value":event.session,
                "provenance":"direct-sanitized-scalar-copy","headerPopulationVerified":false},
            "competition":{"primaryWindowMs":30000,"guardMs":300000,"nativeCount":2,
                "extraLocalCount":1,"upstreamCount":3,"localIds":[id],"upstreamIds":[upstream],"modelFilter":false}},
        "modelEvidence":{"status":"conflict","recordedModel":event.model,
            "upstreamModel":"upstream-model","action":"preserve-recorded"}
    })
}

fn settings(directory: &Path, rows: Vec<Value>) -> PathBuf {
    fs::write(
        directory.join("usage-supplier-evidence-v2.json"),
        serde_json::to_vec(&json!({"version":2,"records":rows})).unwrap(),
    )
    .unwrap();
    directory.join("settings.json")
}

fn apply(cache: &mut Cache, settings: &Path, events: Vec<UsageEvent>) -> Snapshot {
    let mut snapshot = Snapshot {
        events,
        ready: true,
        count_cache_available: true,
        ..Snapshot::default()
    };
    cache.supplement(settings, &mut snapshot);
    snapshot
}

fn assert_only_supplier(original: &UsageEvent, result: &UsageEvent) {
    let restored = UsageEvent {
        provider: original.provider.clone(),
        provider_source: original.provider_source.clone(),
        ..result.clone()
    };
    assert!(
        original == &restored,
        "supplier import changed native identity/model/counts"
    );
}

#[test]
fn positive_preserves_native_and_exposes_model_conflict() {
    let directory = sandbox();
    let original = event(1);
    let path = settings(directory.path(), vec![row(&original, 1)]);
    let mut cache = Cache::default();
    let result = apply(&mut cache, &path, vec![original.clone()]);
    assert_eq!(result.events[0].provider, "supplier-test");
    assert_eq!(result.events[0].provider_source.as_deref(), Some(SOURCE));
    assert_only_supplier(&original, &result.events[0]);
    assert_eq!(result.supplier_evidence["appliedEvents"], 1);
    assert_eq!(result.supplier_evidence["modelRepairs"], 0);
    assert_eq!(
        result.supplier_evidence["runtimeCompetitionRecomputed"],
        false
    );
    assert_eq!(
        result.supplier_evidence["trustBoundary"],
        "external-offline-record-bound"
    );
    assert_eq!(
        result.supplier_evidence["modelEvidence"][0]["recordedModel"],
        "recorded-model"
    );
    assert_eq!(
        result.supplier_evidence["modelEvidence"][0]["upstreamModel"],
        "upstream-model"
    );
    let query = Query::parse(&json!({"period":"all"}), original.date).unwrap();
    let response = result.response_page(&query, false, 1, None, None);
    assert_eq!(response["supplierEvidence"]["modelConflictEvents"], 1);
}

#[test]
fn native_signature_fields_and_uncertain_flags_are_strict() {
    let original = event(1);
    let mut variants = Vec::new();
    for field in 0..12 {
        let mut changed = original.clone();
        match field {
            0 => changed.session = "00000000-0000-4000-8000-000000000002".into(),
            1 => changed.timestamp = "2026-10-30T00:00:01Z".into(),
            2 => changed.model = "different".into(),
            3 => changed.role = "subagent".into(),
            4 => changed.delta.input_tokens += 1,
            5 => changed.delta.output_tokens += 1,
            6 => changed.delta.cached_input_tokens += 1,
            7 => changed.delta.cached_known = false,
            8 => changed.total.cached_known = false,
            9 => changed.count_uncertain = true,
            10 => changed.count_incomplete = true,
            _ => changed.context_conflicts.model = true,
        }
        variants.push(changed);
    }
    for changed in variants {
        let directory = sandbox();
        let path = settings(directory.path(), vec![row(&original, 1)]);
        let result = apply(&mut Cache::default(), &path, vec![changed.clone()]);
        assert!(result.events[0] == changed);
        assert_eq!(result.supplier_evidence["appliedEvents"], 0);
    }
}

#[test]
fn upstream_fields_competition_schema_and_model_forgery_reject() {
    let original = event(1);
    let base = row(&original, 1);
    let cases = [
        ("/supplierEvidence/upstream/input", json!(101)),
        ("/supplierEvidence/upstream/cached", json!(0)),
        (
            "/supplierEvidence/upstream/timestamp",
            json!("2026-10-30T00:00:30.000001Z"),
        ),
        ("/supplierEvidence/upstream/sessionId", json!("other")),
        ("/supplierEvidence/upstream/requestId", json!("")),
        (
            "/supplierEvidence/upstream/requestId",
            json!("bad\nrequest"),
        ),
        (
            "/supplierEvidence/upstream/id",
            json!("other-namespace:bill/1"),
        ),
        ("/supplierEvidence/upstream/supplier", json!("bad/supplier")),
        (
            "/supplierEvidence/sessionBinding/field",
            json!("usage..session"),
        ),
        (
            "/supplierEvidence/sessionBinding/headerPopulationVerified",
            json!(true),
        ),
        ("/supplierEvidence/competition/modelFilter", json!(true)),
        ("/supplierEvidence/competition/guardMs", json!(300001)),
        (
            "/supplierEvidence/competition/primaryWindowMs",
            json!(30001),
        ),
        ("/supplierEvidence/competition/nativeCount", json!(0)),
        ("/supplierEvidence/competition/localIds", json!([])),
        ("/modelEvidence/status", json!("agrees")),
        ("/modelEvidence/upstreamModel", json!("recorded-model")),
        ("/supplierEvidence/upstream/model", json!("recorded-model")),
    ];
    for (pointer, value) in cases {
        let mut changed = base.clone();
        *changed.pointer_mut(pointer).unwrap() = value;
        let directory = sandbox();
        let path = settings(directory.path(), vec![changed]);
        let result = apply(&mut Cache::default(), &path, vec![original.clone()]);
        assert!(result.events[0] == original, "accepted {pointer}");
        assert_eq!(result.supplier_evidence["invalid"], 1);
    }
    let mut changed = base.clone();
    changed["supplierEvidence"]["competition"]["verified"] = json!(true);
    let directory = sandbox();
    let path = settings(directory.path(), vec![changed]);
    let result = apply(&mut Cache::default(), &path, vec![original.clone()]);
    assert_eq!(result.supplier_evidence["status"], "rejected");
    assert!(result.events[0] == original);
}

#[test]
fn exact_thirty_second_boundary_is_accepted() {
    for at in ["2026-10-30T00:00:30Z", "2026-10-29T23:59:30Z"] {
        let original = event(1);
        let mut record = row(&original, 1);
        record["supplierEvidence"]["upstream"]["timestamp"] = json!(at);
        let directory = sandbox();
        let path = settings(directory.path(), vec![record]);
        let result = apply(&mut Cache::default(), &path, vec![original]);
        assert_eq!(result.supplier_evidence["appliedEvents"], 1);
    }
}

#[test]
fn duplicate_native_event_and_evidence_ids_do_not_apply() {
    let original = event(1);
    for (rows, events) in [
        (
            vec![row(&original, 1)],
            vec![original.clone(), original.clone()],
        ),
        (
            vec![row(&original, 1), row(&original, 1)],
            vec![original.clone()],
        ),
    ] {
        let directory = sandbox();
        let path = settings(directory.path(), rows);
        let result = apply(&mut Cache::default(), &path, events);
        assert!(result.events.iter().all(|item| item == &original));
        assert_eq!(result.supplier_evidence["appliedEvents"], 0);
    }
}

#[test]
fn upstream_and_request_reuse_blocks_both_and_survives_removal() {
    let first = event(1);
    let second = event(2);
    for key in ["id", "requestId"] {
        let directory = sandbox();
        let first_row = row(&first, 1);
        let mut second_row = row(&second, 2);
        second_row["supplierEvidence"]["upstream"][key] =
            first_row["supplierEvidence"]["upstream"][key].clone();
        if key == "id" {
            second_row["supplierEvidence"]["competition"]["upstreamIds"] =
                json!(["fixture-supplier:bill/1"]);
        }
        let mut cache = Cache::default();
        let path = settings(directory.path(), vec![first_row.clone()]);
        assert_eq!(
            apply(&mut cache, &path, vec![first.clone()]).supplier_evidence["appliedEvents"],
            1
        );
        settings(directory.path(), vec![first_row.clone(), second_row]);
        let result = apply(&mut cache, &path, vec![first.clone(), second.clone()]);
        assert_eq!(result.supplier_evidence["appliedEvents"], 0);
        settings(directory.path(), vec![first_row]);
        assert_eq!(
            apply(&mut cache, &path, vec![first.clone()]).supplier_evidence["appliedEvents"],
            0
        );
    }
}

#[test]
fn signature_rewrite_revoke_and_reinstate_protection_are_independent_of_v1() {
    let original = event(1);
    let directory = sandbox();
    let base = row(&original, 1);
    let path = settings(directory.path(), vec![base.clone()]);
    let mut cache = Cache::default();
    assert_eq!(
        apply(&mut cache, &path, vec![original.clone()]).supplier_evidence["appliedEvents"],
        1
    );
    fs::remove_file(directory.path().join("usage-supplier-evidence-v2.json")).unwrap();
    let absent = apply(&mut cache, &path, vec![original.clone()]);
    assert_eq!(absent.supplier_evidence["status"], "absent");
    assert!(absent.events[0] == original);
    let mut changed = base.clone();
    changed["supplierEvidence"]["providerName"] = json!("changed");
    settings(directory.path(), vec![changed]);
    assert_eq!(
        apply(&mut cache, &path, vec![original.clone()]).supplier_evidence["invalid"],
        1
    );
    settings(directory.path(), vec![base]);
    assert_eq!(
        apply(&mut cache, &path, vec![original]).supplier_evidence["invalid"],
        1
    );
}

#[test]
fn known_supplier_keeps_original_source_and_other_supplier_rejects() {
    let base = event(1);
    let directory = sandbox();
    let path = settings(directory.path(), vec![row(&base, 1)]);
    for provider in ["supplier-test", "supplier-other"] {
        let mut known = base.clone();
        known.provider = provider.into();
        known.provider_source = Some("original-source".into());
        let result = apply(&mut Cache::default(), &path, vec![known.clone()]);
        assert!(result.events[0] == known);
        assert_eq!(result.supplier_evidence["appliedEvents"], 0);
        assert_eq!(
            result.supplier_evidence["unchangedEvents"],
            usize::from(provider == "supplier-test")
        );
    }
}

#[test]
fn both_evidence_imports_match_after_the_old_file_record_and_cache_limits() -> anyhow::Result<()> {
    use std::io::{BufWriter, Write};

    let directory = sandbox();
    let path = directory.path().join("settings.json");
    let original = event(1);
    let legacy = json!({"eventId":"01".repeat(32),"session":original.session,
        "timestamp":original.timestamp,"model":original.model,"role":"main","input":100,
        "output":7,"cached":80,"provider":"supplier-test","providerName":"Test supplier",
        "source":"upstream-exact-bidirectional"});
    for (version, file, record, id_path) in [
        (1, "usage-provider-evidence.json", legacy, "/eventId"),
        (
            2,
            "usage-supplier-evidence-v2.json",
            row(&original, 1),
            "/event/eventId",
        ),
    ] {
        let evidence = directory.path().join(file);
        let mut output = BufWriter::new(File::create(&evidence)?);
        write!(output, "{{\"version\":{version},\"records\":[")?;
        let mut unrelated = record.clone();
        *unrelated.pointer_mut(id_path).unwrap() = json!("x".repeat(64));
        let mut line = serde_json::to_vec(&unrelated)?;
        line.extend_from_slice(b",\n");
        for _ in 0..MAX_EVENTS {
            output.write_all(&line)?;
        }
        serde_json::to_writer(&mut output, &record)?;
        output.write_all(b"]}")?;
        output.flush()?;
        drop(output);
        assert!(fs::metadata(&evidence)?.len() > 32 << 20);

        let mut snapshot = Snapshot {
            events: vec![original.clone()],
            ..Snapshot::default()
        };
        let report = if version == 1 {
            let mut cache = LegacyEvidenceCache {
                signatures: (0..MAX_EVENTS)
                    .map(|n| {
                        let mut id = [0; 32];
                        id[24..].copy_from_slice(&(n as u64).to_be_bytes());
                        (id, None)
                    })
                    .collect(),
            };
            cache.apply(LegacyEvidenceDocument::read(&path), &mut snapshot);
            assert_eq!(cache.signatures.len(), MAX_EVENTS + 1);
            snapshot.provider_evidence
        } else {
            let stale_owners: HashMap<_, _> =
                (0..MAX_EVENTS).map(|n| (n.to_string(), None)).collect();
            let mut cache = Cache {
                signatures: (0..MAX_EVENTS).map(|n| (format!("{n:064x}"), None)).collect(),
                upstream_owners: stale_owners.clone(),
                request_owners: stale_owners,
            };
            cache.supplement(&path, &mut snapshot);
            assert_eq!(cache.signatures.len(), MAX_EVENTS + 1);
            assert_eq!(cache.upstream_owners.len(), MAX_EVENTS + 1);
            assert_eq!(cache.request_owners.len(), MAX_EVENTS + 1);
            snapshot.supplier_evidence
        };
        assert_eq!(report["status"], "ok");
        assert_eq!(report["invalid"], MAX_EVENTS);
        assert_eq!(report["appliedEvents"], 1);
        assert_eq!(snapshot.events[0].provider, "supplier-test");
        assert_only_supplier(&original, &snapshot.events[0]);
    }
    Ok(())
}

#[test]
fn v1_schema_and_model_guard_remain_strict_and_sources_do_not_mix() {
    let original = event(1);
    let directory = sandbox();
    let legacy = json!({"eventId":"01".repeat(32),"session":original.session,
        "timestamp":original.timestamp,"model":"wrong-model","role":"main","input":100,
        "output":7,"cached":80,"provider":"supplier-other","providerName":"Other",
        "source":"upstream-exact-bidirectional"});
    let legacy_path = directory.path().join("usage-provider-evidence.json");
    fs::write(
        &legacy_path,
        serde_json::to_vec(&json!({"version":1,"records":[legacy.clone()]})).unwrap(),
    )
    .unwrap();
    let path = settings(directory.path(), vec![row(&original, 1)]);
    let mut index = Index::default();
    let mut snapshot = Snapshot {
        events: vec![original.clone()],
        ..Snapshot::default()
    };
    index.supplement_legacy_evidence(&path, &mut snapshot);
    assert!(snapshot.events[0] == original);
    assert_eq!(snapshot.provider_evidence["signatureMismatches"], 1);
    index.supplier_evidence.supplement(&path, &mut snapshot);
    assert_eq!(snapshot.events[0].provider, "supplier-test");
    let mut extra = legacy;
    extra["upstreamModel"] = json!("extra");
    assert!(serde_json::from_value::<LegacyProviderEvidence>(extra).is_err());
    fs::remove_file(directory.path().join("usage-supplier-evidence-v2.json")).unwrap();
    let mut fresh = Snapshot {
        events: vec![original.clone()],
        ..Snapshot::default()
    };
    index.supplement_legacy_evidence(&path, &mut fresh);
    index.supplier_evidence.supplement(&path, &mut fresh);
    assert!(fresh.events[0] == original);
}

#[test]
fn another_supplier_and_another_field_path_are_accepted() {
    let original = event(1);
    let mut record = row(&original, 1);
    record["supplierEvidence"]["upstream"]["supplier"] = json!("Example_2");
    record["supplierEvidence"]["upstream"]["id"] = json!("Example_2:opaque:bill-X");
    record["supplierEvidence"]["competition"]["upstreamIds"] = json!(["Example_2:opaque:bill-X"]);
    record["supplierEvidence"]["upstream"]["requestId"] = json!("another opaque request");
    record["supplierEvidence"]["sessionBinding"]["field"] = json!("response.rows[].session_uuid");
    let directory = sandbox();
    let path = settings(directory.path(), vec![record]);
    let result = apply(&mut Cache::default(), &path, vec![original]);
    assert_eq!(result.supplier_evidence["appliedEvents"], 1);
}

#[test]
fn different_suppliers_can_share_the_same_opaque_request_id() {
    let first = event(1);
    let second = event(2);
    let first_row = row(&first, 1);
    let mut second_row = row(&second, 2);
    second_row["supplierEvidence"]["upstream"]["supplier"] = json!("other_fixture");
    second_row["supplierEvidence"]["upstream"]["id"] = json!("other_fixture:bill/2");
    second_row["supplierEvidence"]["competition"]["upstreamIds"] = json!(["other_fixture:bill/2"]);
    second_row["supplierEvidence"]["upstream"]["requestId"] =
        first_row["supplierEvidence"]["upstream"]["requestId"].clone();
    let directory = sandbox();
    let path = settings(directory.path(), vec![first_row, second_row]);
    let result = apply(&mut Cache::default(), &path, vec![first, second]);
    assert_eq!(result.supplier_evidence["appliedEvents"], 2);
    assert_eq!(result.supplier_evidence["conflictedEvents"], 0);
}

#[test]
fn missing_fields_trailing_json_and_empty_supplier_document_reject_before_mutation() {
    let original = event(1);
    let directory = sandbox();
    let mut missing = row(&original, 1);
    missing["supplierEvidence"]["upstream"]
        .as_object_mut()
        .unwrap()
        .remove("requestId");
    let path = settings(directory.path(), vec![missing]);
    assert_eq!(
        apply(&mut Cache::default(), &path, vec![original.clone()]).supplier_evidence["status"],
        "rejected"
    );
    let mut with_trailing_json =
        serde_json::to_vec(&json!({"version":2,"records":[row(&original, 1)]})).unwrap();
    with_trailing_json.extend_from_slice(b"{}");
    fs::write(
        directory.path().join("usage-supplier-evidence-v2.json"),
        with_trailing_json,
    )
    .unwrap();
    let mut cache = Cache::default();
    let rejected = apply(&mut cache, &path, vec![original.clone()]);
    assert_eq!(rejected.supplier_evidence["status"], "rejected");
    assert!(rejected.events[0] == original && cache.signatures.is_empty());
    assert!(cache.upstream_owners.is_empty() && cache.request_owners.is_empty());
    settings(directory.path(), vec![]);
    assert_eq!(
        apply(&mut Cache::default(), &path, vec![original]).supplier_evidence["status"],
        "rejected"
    );
}

#[test]
fn synthetic_reader_replay_preserves_v1_proofs_counts_and_revokes_v2() {
    let directory = sandbox();
    let home = directory.path().join("home");
    fs::create_dir_all(home.join("sessions")).unwrap();
    let home = fs::canonicalize(home).unwrap();
    let path = directory.path().join("settings.json");
    fs::write(&path, r#"{"usageHistoryEnabled":true,"relayProfiles":[]}"#).unwrap();
    let request_log = directory.path().join("requests.jsonl");
    fs::write(&request_log, "").unwrap();
    let timestamp = (Local::now() - Days::seconds(60)).to_rfc3339();
    for number in 1..=2 {
        let session = format!("00000000-0000-4000-8000-{number:012}");
        let rows = [
            json!({"type":"session_meta","timestamp":timestamp,
                "payload":{"id":session,"model_provider":"custom"}}),
            json!({"type":"turn_context","timestamp":timestamp,
                "payload":{"model":"recorded-model"}}),
            json!({"type":"event_msg","timestamp":timestamp,
                "payload":{"type":"token_count","info":{
                    "total_token_usage":{"input_tokens":100,"output_tokens":7,"cached_input_tokens":80},
                    "last_token_usage":{"input_tokens":100,"output_tokens":7,"cached_input_tokens":80}}}}),
        ];
        fs::write(
            home.join("sessions")
                .join(format!("rollout-{session}.jsonl")),
            rows.iter()
                .map(|value| format!("{value}\n"))
                .collect::<String>(),
        )
        .unwrap();
    }
    let mut index = Index {
        settings_path: Some(path.clone()),
        storage_dir: Some(directory.path().join("cache")),
        ..Index::default()
    };
    let native = index.update_with_request_log(&home, &request_log);
    assert_eq!(native.errors, 0);
    assert_eq!(native.issues, 0);
    assert_eq!(native.events.len(), 2);
    let original = native
        .events
        .iter()
        .find(|item| item.session.ends_with('1'))
        .unwrap()
        .clone();
    let legacy = native
        .events
        .iter()
        .find(|item| item.session.ends_with('2'))
        .unwrap();
    fs::write(path.with_file_name("usage-provider-evidence.json"),
        serde_json::to_vec(&json!({"version":1,"records":[{
            "eventId":legacy.id.iter().map(|byte| format!("{byte:02x}")).collect::<String>(),
            "session":legacy.session,"timestamp":legacy.timestamp,"model":legacy.model,
            "role":legacy.role,"input":legacy.delta.input_tokens,"output":legacy.delta.output_tokens,
            "cached":legacy.delta.cached_input_tokens,"provider":"legacy-test","providerName":"Legacy",
            "source":"upstream-exact-bidirectional"
        }]})).unwrap()).unwrap();
    let baseline = index.update_with_request_log(&home, &request_log);
    assert_eq!(baseline.provider_evidence["appliedEvents"], 1);
    settings(directory.path(), vec![row(&original, 1)]);
    let supplied = index.update_with_request_log(&home, &request_log);
    assert_eq!(supplied.supplier_evidence["appliedEvents"], 1);
    let storage = directory.path().join("persistent");
    let mut worker = token_checkpoint::Runtime::new(storage.clone(), path.clone());
    let state = worker.update(&home);
    let at = DateTime::parse_from_rfc3339(&timestamp)
        .unwrap()
        .timestamp_millis();
    let request =
        json!({"startAt":at-1, "endAt":at+1,"includeTrend":true,"includeSessionCursors":true});
    let bounded = Query::parse(&request, Local::now().date_naive()).unwrap();
    let detailed = state.read(&storage, &bounded, &request, Some(&original.session), false);
    let calendar = state.read(&storage, &bounded, &json!({}), None, false);
    assert_eq!(detailed["providerEvidence"]["appliedEvents"], 1);
    assert_eq!(detailed["supplierEvidence"]["appliedEvents"], 1);
    assert_eq!(calendar["days"], detailed["days"]);
    assert_eq!(detailed["days"][0]["usage"]["totalTokens"], 214);
    let scoped = &detailed["scopedUsage"]["days"][0]["breakdowns"][0];
    assert_eq!(scoped["model"], "recorded-model");
    assert_eq!(scoped["provider"], "supplier-test");
    assert_eq!(scoped["supplierClassificationEvidence"], SOURCE);
    assert!(scoped.get("classificationEvidence").is_none());
    assert_eq!(detailed["trend"]["buckets"][0]["usage"]["totalTokens"], 214);
    assert!(detailed["liveAttributions"].as_array().unwrap().is_empty());

    let supplier_path = path.with_file_name("usage-supplier-evidence-v2.json");
    let saved = supplier_path.with_extension("saved");
    fs::rename(&supplier_path, &saved).unwrap();
    let withdrawn = state.read(&storage, &bounded, &request, Some(&original.session), false);
    assert_eq!(withdrawn["revision"], detailed["revision"]);
    assert_eq!(withdrawn["countStatus"], detailed["countStatus"]);
    assert_eq!(withdrawn["sessionCursors"], detailed["sessionCursors"]);
    assert_eq!(withdrawn["days"][0]["usage"], detailed["days"][0]["usage"]);
    assert_ne!(
        withdrawn["scopedUsage"]["days"][0]["breakdowns"][0]["provider"],
        "supplier-test"
    );
    let refreshed = worker
        .update(&home)
        .read(&storage, &bounded, &request, None, false);
    assert_eq!(refreshed["providerEvidence"]["appliedEvents"], 1);
    assert_eq!(refreshed["supplierEvidence"]["status"], "absent");
    fs::rename(saved, supplier_path).unwrap();
    for (before, after) in baseline.events.iter().zip(&supplied.events) {
        assert_only_supplier(before, after);
    }
    assert!(index.request_evidence.verified.is_empty());
    assert!(index.native_evidence.verified.is_empty());
    assert!(
        index
            .provider_cache
            .load_verified(NaiveDate::MIN)
            .unwrap()
            .is_empty()
    );
    assert!(
        index
            .provider_cache
            .load_native(NaiveDate::MIN)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        supplied.session_info[&original.session]["verifiedUsage"],
        Value::Null
    );
    assert_eq!(
        supplied.session_info[&legacy.session]["verifiedUsage"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let query = Query::parse(&json!({"period":"all"}), original.date).unwrap();
    let before_days = baseline.usage_days(&query, None, None);
    let after_days = supplied.usage_days(&query, None, None);
    for (before, after) in before_days.iter().zip(&after_days) {
        for field in [
            "date",
            "usage",
            "recordCount",
            "models",
            "accountingFingerprint",
        ] {
            assert_eq!(before[field], after[field], "changed {field}");
        }
    }
    let response = supplied.response_page(&query, false, 1, None, None);
    let rows = response["days"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|day| day["breakdowns"].as_array().unwrap());
    let supplier_rows: Vec<_> = rows
        .filter(|item| item["provider"] == "supplier-test")
        .collect();
    assert_eq!(supplier_rows.len(), 1);
    assert_eq!(supplier_rows[0]["classificationEvidence"], Value::Null);
    assert_eq!(supplier_rows[0]["supplierClassificationEvidence"], SOURCE);
    assert!(before_days.iter().all(|day| {
        day["breakdowns"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item.get("supplierClassificationEvidence").is_none())
    }));
    let evidence = path.with_file_name("usage-supplier-evidence-v2.json");
    let opened = File::open(&evidence).unwrap();
    let identity = codex_plus_core::codex_sqlite::opened_file_identity(&opened).unwrap();
    let before = opened.metadata().unwrap();
    assert_eq!(
        observed_file_identity(&evidence, &before).unwrap(),
        identity
    );
    let bytes = fs::read(&evidence).unwrap();
    fs::rename(&evidence, evidence.with_extension("old")).unwrap();
    fs::write(&evidence, bytes).unwrap();
    File::options()
        .write(true)
        .open(&evidence)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(before.modified().unwrap()))
        .unwrap();
    let replaced = fs::metadata(&evidence).unwrap();
    assert_eq!(replaced.len(), before.len());
    assert_eq!(replaced.modified().unwrap(), before.modified().unwrap());
    assert_ne!(
        observed_file_identity(&evidence, &replaced).unwrap(),
        identity
    );
    assert_eq!(
        codex_plus_core::codex_sqlite::opened_file_identity(&opened).unwrap(),
        identity
    );
    assert_eq!(
        index
            .update_with_request_log(&home, &request_log)
            .supplier_evidence["appliedEvents"],
        1
    );
    fs::remove_file(&evidence).unwrap();
    let revoked = index.update_with_request_log(&home, &request_log);
    assert_eq!(revoked.supplier_evidence["status"], "absent");
    assert!(baseline.events == revoked.events);
    assert_eq!(
        baseline.usage_days(&query, None, None),
        revoked.usage_days(&query, None, None)
    );
    let revoked_response = revoked.response_page(&query, false, 1, None, None);
    assert!(
        revoked_response["days"]
            .as_array()
            .unwrap()
            .iter()
            .all(|day| day["breakdowns"]
                .as_array()
                .unwrap()
                .iter()
                .all(|item| item.get("supplierClassificationEvidence").is_none()))
    );
}

#[test]
fn supplier_classification_requires_all_members_and_no_context_conflicts() {
    let first = UsageEvent {
        provider: "supplier-test".into(),
        provider_source: Some(SOURCE.into()),
        ..event(1)
    };
    let query = Query::parse(&json!({"period":"all"}), first.date).unwrap();
    let group = |events| {
        let snapshot = Snapshot {
            events,
            ready: true,
            count_cache_available: true,
            ..Snapshot::default()
        };
        let response = snapshot.response_page(&query, false, 1, None, None);
        let groups = response["days"][0]["breakdowns"].as_array().unwrap();
        assert_eq!(groups.len(), 1);
        groups[0].clone()
    };
    let mut second = UsageEvent {
        id: [2; 32],
        ..first.clone()
    };
    let supplied = group(vec![first.clone(), second.clone()]);
    assert_eq!(supplied["supplierClassificationEvidence"], SOURCE);
    assert_eq!(supplied["classificationEvidence"], Value::Null);
    assert_eq!(supplied["model"], first.model);
    assert_eq!(supplied["role"], first.role);
    for source in [
        None,
        Some("request-log-exact"),
        Some("upstream-exact-bidirectional"),
        Some("user-provider-classification"),
    ] {
        second.provider_source = source.map(str::to_owned);
        for id in [0, 2] {
            second.id = [id; 32];
            let mixed = group(vec![first.clone(), second.clone()]);
            assert!(mixed.get("supplierClassificationEvidence").is_none());
            assert_eq!(mixed["classificationEvidence"], Value::Null);
        }
    }
    for field in 0..4 {
        let mut conflicted = first.clone();
        match field {
            0 => conflicted.context_conflicts.model = true,
            1 => conflicted.context_conflicts.provider = true,
            2 => conflicted.context_conflicts.role = true,
            _ => conflicted.context_conflicts.turn = true,
        }
        let row = group(vec![conflicted]);
        assert!(row.get("supplierClassificationEvidence").is_none());
        assert_eq!(row["classificationEvidence"], Value::Null);
    }
    let exact = group(vec![UsageEvent {
        provider_source: Some("request-log-exact".into()),
        ..first
    }]);
    assert_eq!(exact["classificationEvidence"], "request-log-exact");
    assert!(exact.get("supplierClassificationEvidence").is_none());
}
