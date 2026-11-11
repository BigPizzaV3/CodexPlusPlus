use super::*;

#[path = "actual_attribution_tests.rs"]
mod actual_attribution_tests;

#[test]
fn local_large_json_strings_keep_only_safe_skill_receipts() {
    let dense = format!("[{}0]", "0,".repeat(40_000));
    assert!(dense.len() > tool_receipts::MAX_JSON_TEXT_BYTES);
    assert_eq!(tool_receipts::compact(&json!(dense), 0, false), json!(""));
    let receipt = format!(
        "Wall time: 0.1 seconds\nProcess exited with code 0\nname: pdf\n# Skill\n{}",
        "private body ".repeat(7_000)
    );
    let compact = tool_receipts::compact(&json!(receipt), 0, true);
    let text = compact.as_str().unwrap();
    assert!(text.contains("Process exited with code 0"));
    assert!(text.contains("name: pdf"));
    assert!(text.contains("# Skill content observed"));
    assert!(!text.contains("private body"));
    let wrapped = json!({"success":true, "content":[{"type":"text", "text":"name: pdf\n# Skill"}]})
        .to_string();
    let compact = tool_receipts::compact(&json!(wrapped), 0, true);
    assert_eq!(compact["success"], true);
    assert!(
        compact["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("name: pdf")
    );
    let call = tool_receipts::call(&json!({"type":"function_call", "call_id":"skill",
        "name":"functions.exec_command", "arguments":"{\"cmd\":\"cat /skills/pdf/SKILL.md\"}"}));
    assert_eq!(call["arguments"]["skillNames"], json!(["pdf"]));
    assert_eq!(call["arguments"]["singleRead"], true);
}

#[test]
fn local_skipping_unterminated_line_releases_buffer_and_resumes_tokens() {
    use std::io::Write;
    let fixture = Fixture::new();
    let root = fs::canonicalize(fixture._root.path()).unwrap();
    let path = root.join("oversized-tail.jsonl");
    let head = json!({"type":"session_meta", "payload":{"id":SESSION}});
    fs::write(&path, format!("{head}\n{}", "x".repeat(MAX_LINE + 64))).unwrap();
    let cutoff = Local::now().date_naive() - Days::days(RETENTION_DAYS - 1);
    let mut state = FileState::default();
    let mut budget = index_budget::Budget::new(std::iter::empty());
    assert!(!state.read_with_budget(&path, cutoff, &mut budget).unwrap());
    assert!(state.skipping);
    assert!(state.reader.is_none());
    assert_eq!(state.pending.capacity(), 0);
    let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
    writeln!(file, "\n{}", fixture.usage(1_000, 1)).unwrap();
    drop(file);
    let mut budget = index_budget::Budget::new(std::iter::empty());
    assert!(state.read_with_budget(&path, cutoff, &mut budget).unwrap());
    assert!(!state.skipping);
    assert_eq!(state.events.len(), 1);
    assert_eq!(state.pending.capacity(), 0);
}

#[test]
fn local_evidence_placeholder_is_never_a_verified_model() {
    for model in ["default", "null", "undefined", "auto", "group/routing"] {
        let mut fixture = Fixture::new();
        fixture.write_segment(
            "rollout-a.jsonl",
            vec![fixture.context(500, "Unknown"), fixture.usage(1_000, 1)],
        );
        let original = fixture.snapshot();
        fixture.write_evidence(vec![evidence(&original.events[0], model)]);
        reopen_index(&mut fixture);
        let repaired = fixture.snapshot();
        assert!(unknown_provider(&repaired.events[0].provider), "{model}");
        assert_eq!(repaired.provider_evidence["appliedEvents"], 0, "{model}");
        assert_eq!(
            snapshot_response(&repaired)["days"][0]["usage"]["totalTokens"],
            110
        );
    }
}

#[test]
fn local_missing_initial_baseline_does_not_assign_lifetime_usage_to_today() {
    let mut fixture = Fixture::new();
    let mut segment = fixture.usage(2_000, 100);
    segment["payload"]["info"]["last_token_usage"] = Value::Null;
    fixture.write_continuation_segment("rollout-b.jsonl", vec![segment]);
    let response = snapshot_response(&fixture.snapshot());
    assert_eq!(response["complete"], false);
    assert_eq!(response["days"], json!([]));
    fixture.write_segment("rollout-a.jsonl", vec![fixture.usage(1_000, 99)]);
    let response = snapshot_response(&fixture.snapshot());
    assert_eq!(response["complete"], true);
    assert_eq!(response["days"][0]["usage"]["totalTokens"], 220);
}

#[test]
fn local_competing_cache_and_reasoning_are_unknown_without_losing_usage() {
    let mut fixture = Fixture::new();
    let mut second = fixture.usage(2_000, 2);
    second["payload"]["info"]["last_token_usage"]["cached_input_tokens"] = json!(90);
    second["payload"]["info"]["last_token_usage"]["reasoning_output_tokens"] = json!(9);
    fixture.write_segment("rollout-a.jsonl", vec![fixture.usage(1_000, 1), second]);
    let snapshot = fixture.snapshot();
    let response = snapshot_response(&snapshot);
    let usage = &response["days"][0]["usage"];
    assert_eq!(usage["totalTokens"], 220);
    assert_eq!(usage["cachedKnown"], false);
    assert_eq!(usage["reasoningKnown"], false);
    assert!(!snapshot.events[1].delta.cached_known);
}

#[test]
fn local_component_corrections_change_proof_without_replaying_consumption() {
    let mut fixture = Fixture::new();
    let first = fixture.usage(1_000, 1);
    fixture.write_segment("rollout-a.jsonl", vec![first.clone()]);
    let original = snapshot_response(&fixture.snapshot());
    let mut correction = first.clone();
    correction["timestamp"] = json!(fixture.at(2_000));
    correction["payload"]["info"]["total_token_usage"]["cached_input_tokens"] = json!(80);
    correction["payload"]["info"]["total_token_usage"]["reasoning_output_tokens"] = json!(8);
    fixture.write_segment("rollout-a.jsonl", vec![first, correction]);
    reopen_index(&mut fixture);
    let snapshot = fixture.snapshot();
    let response = snapshot_response(&snapshot);
    assert_eq!(response["days"][0]["usage"]["totalTokens"], 110);
    assert_eq!(response["days"][0]["usage"]["cachedKnown"], false);
    assert_eq!(response["days"][0]["usage"]["reasoningKnown"], false);
    assert_ne!(
        original["days"][0]["accountingFingerprint"],
        response["days"][0]["accountingFingerprint"]
    );
}

#[test]
fn local_out_of_range_components_are_not_clamped_into_known_counts() {
    let parsed = Tokens::parse(&json!({
        "input_tokens": 100, "cached_input_tokens": 101,
        "output_tokens": 10, "reasoning_output_tokens": 11
    }))
    .unwrap();
    assert!(!parsed.cached_known);
    assert_eq!(parsed.reasoning_output_tokens, None);
    let delta = Tokens::parse(&json!({
        "input_tokens": 200, "cached_input_tokens": 180,
        "output_tokens": 20, "reasoning_output_tokens": 20
    }))
    .unwrap()
    .usage_delta(Some(Tokens::new(100, 0, 10)), None);
    assert_eq!(delta.total_tokens, 110);
    assert!(!delta.cached_known);
    let before = Tokens::parse(&json!({
        "input_tokens": 100, "cached_input_tokens": 80,
        "output_tokens": 10, "reasoning_output_tokens": 8
    }))
    .unwrap();
    let next = Tokens::parse(&json!({
        "input_tokens": 200, "cached_input_tokens": 20,
        "output_tokens": 20, "reasoning_output_tokens": 2
    }))
    .unwrap();
    assert!(next.is_monotonic_after(before));
    assert!(!next.requires_last_after(before));
    let delta = next.usage_delta(Some(before), Some(Tokens::new(1, 0, 1)));
    assert_eq!(delta.total_tokens, 110);
    assert!(!delta.cached_known);
    assert_eq!(delta.reasoning_output_tokens, None);
}

#[test]
fn local_invalid_usage_time_is_partial_and_recovers_after_source_repair() {
    for timestamp in [Value::Null, json!("invalid-time")] {
        let mut fixture = Fixture::new();
        let first = fixture.usage(1_000, 1);
        let mut second = fixture.usage(2_000, 2);
        second["timestamp"] = timestamp;
        fixture.write_segment("rollout-a.jsonl", vec![first.clone(), second]);
        let snapshot = fixture
            .index
            .update_with_request_log(&fixture.home, &fixture.log);
        let response = snapshot_response(&snapshot);
        assert_eq!(response["complete"], false);
        assert!(snapshot.issues > 0);
        assert_eq!(response["days"], json!([]));
        fixture.write_segment("rollout-a.jsonl", vec![first, fixture.usage(2_000, 2)]);
        let recovered = fixture.snapshot();
        let response = snapshot_response(&recovered);
        assert_eq!(response["complete"], true);
        assert_eq!(response["days"][0]["usage"]["totalTokens"], 220);
    }
}

#[test]
fn local_membership_proofs_bind_counts_and_allow_exact_evidence_reclassification() {
    let mut fixture = Fixture::new();
    let row = fixture.usage(1_000, 1);
    fixture.write_segment(
        "rollout-a.jsonl",
        vec![fixture.context(500, "Unknown"), row],
    );
    let initial = fixture.snapshot();
    let original = snapshot_response(&initial)["days"][0].clone();
    assert!(original["accountingFingerprint"].as_str().unwrap().len() == 64);
    fixture.write_evidence(vec![evidence(&initial.events[0], "model-a")]);
    reopen_index(&mut fixture);
    let repaired = snapshot_response(&fixture.snapshot())["days"][0].clone();
    assert_eq!(
        original["accountingFingerprint"],
        repaired["accountingFingerprint"]
    );
    assert_eq!(
        original["breakdowns"][0]["membershipFingerprint"],
        repaired["breakdowns"][0]["membershipFingerprint"]
    );
    assert_eq!(
        repaired["breakdowns"][0]["classificationEvidence"],
        "upstream-exact-bidirectional"
    );
    assert_eq!(repaired["breakdowns"][0]["model"], "model-a");
    fs::rename(
        fixture
            .settings
            .with_file_name("usage-provider-evidence.json"),
        fixture.settings.with_file_name("saved-evidence.json"),
    )
    .unwrap();
    reopen_index(&mut fixture);
    let revoked = snapshot_response(&fixture.snapshot())["days"][0].clone();
    assert_eq!(
        original["accountingFingerprint"],
        revoked["accountingFingerprint"]
    );
    assert_eq!(
        original["breakdowns"][0]["membershipFingerprint"],
        revoked["breakdowns"][0]["membershipFingerprint"]
    );
    assert!(revoked["breakdowns"][0]["classificationEvidence"].is_null());
    fixture.write_segment(
        "rollout-a.jsonl",
        vec![fixture.context(500, "Unknown"), fixture.usage(2_000, 1)],
    );
    reopen_index(&mut fixture);
    let replaced = snapshot_response(&fixture.snapshot())["days"][0].clone();
    assert_eq!(
        original["usage"]["totalTokens"],
        replaced["usage"]["totalTokens"]
    );
    assert_ne!(
        original["accountingFingerprint"],
        replaced["accountingFingerprint"]
    );
    assert_ne!(
        original["breakdowns"][0]["membershipFingerprint"],
        replaced["breakdowns"][0]["membershipFingerprint"]
    );
}

#[test]
fn local_unfinished_lines_release_readers_and_resume_without_replay() {
    use std::io::Write;
    let fixture = Fixture::new();
    let root = fs::canonicalize(fixture._root.path()).unwrap();
    let cutoff = Local::now().date_naive() - Days::days(RETENTION_DAYS - 1);
    let head = json!({"type":"session_meta", "timestamp":fixture.at(0),
        "payload":{"id":SESSION,"model_provider":"custom"}});
    let mut states = BTreeMap::<PathBuf, FileState>::new();
    for n in 0..21 {
        let path = root.join(format!("partial-{n:02}.jsonl"));
        fs::write(&path, format!("{head}\n{{")).unwrap();
        states.insert(path, FileState::default());
    }
    let valid_path = root.join("z-valid.jsonl");
    fs::write(
        &valid_path,
        format!("{head}\n{}\n", fixture.usage(1_000, 1)),
    )
    .unwrap();
    states.insert(valid_path.clone(), FileState::default());
    let mut budget = index_budget::Budget::new(states.values());
    for (path, state) in &mut states {
        let before = index_budget::Usage::of(state);
        budget.replace(before, index_budget::Usage::default());
        let complete = state.read_with_budget(path, cutoff, &mut budget).unwrap();
        if path == &valid_path {
            assert!(complete);
            assert_eq!(state.events.len(), 1);
        } else {
            assert!(!complete);
            assert!(state.reader.is_none());
        }
        budget.replace(
            index_budget::Usage::default(),
            index_budget::Usage::of(state),
        );
    }
    let resume_path = root.join("partial-00.jsonl");
    let complete_line = fixture.usage(2_000, 1).to_string();
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(&resume_path)
        .unwrap();
    writeln!(file, "{}", &complete_line[1..]).unwrap();
    drop(file);
    for _ in 0..2 {
        let mut budget = index_budget::Budget::new(states.values());
        let state = states.get_mut(&resume_path).unwrap();
        budget.replace(
            index_budget::Usage::of(state),
            index_budget::Usage::default(),
        );
        assert!(
            state
                .read_with_budget(&resume_path, cutoff, &mut budget)
                .unwrap()
        );
        assert_eq!(state.events.len(), 1);
        assert_eq!(state.events[0].delta.total_tokens, 110);
    }
}

#[test]
fn local_expired_capacity_and_calls_release_without_losing_watermark() {
    let fixture = Fixture::new();
    let root = fs::canonicalize(fixture._root.path()).unwrap();
    let path = root.join("expired.jsonl");
    let old_cutoff = Local::now().date_naive() - Days::days(RETENTION_DAYS - 1);
    let new_cutoff = Local::now().date_naive() + Days::days(1);
    let head = json!({"type":"session_meta", "timestamp":fixture.at(0),
        "payload":{"id":SESSION,"model_provider":"custom"}});
    let records = (1..=80)
        .map(|n| fixture.usage(n * 100, n as u64).to_string())
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&path, format!("{head}\n{records}\n")).unwrap();
    let mut state = FileState::default();
    let mut budget = index_budget::Budget::new(std::iter::empty::<&FileState>());
    assert!(
        state
            .read_with_budget(&path, old_cutoff, &mut budget)
            .unwrap()
    );
    assert_eq!(state.events.len(), 80);
    let watermark = state.previous;
    let before = state.retained_charge;
    state.calls.insert(
        "old-call".to_owned(),
        Call {
            timestamp: (Local::now() - Days::days(RETENTION_DAYS + 1)).to_rfc3339(),
            session: SESSION.to_owned(),
            payload: json!({}),
        },
    );
    let mut budget = index_budget::Budget::new(std::iter::empty::<&FileState>());
    assert!(
        state
            .read_with_budget(&path, new_cutoff, &mut budget)
            .unwrap()
    );
    assert!(state.events.is_empty());
    assert!(state.calls.is_empty());
    assert!(state.retained_charge < before);
    assert_eq!(state.previous, watermark);
}

#[test]
fn local_missing_cache_preserves_cumulative_counts_across_segments() {
    for split in [false, true] {
        let mut fixture = Fixture::new();
        let first = fixture.usage(1_000, 1);
        let mut second = fixture.usage(2_000, 2);
        second["payload"]["info"]["total_token_usage"]
            .as_object_mut()
            .unwrap()
            .remove("cached_input_tokens");
        second["payload"]["info"]["last_token_usage"] = json!({
            "input_tokens": 5, "output_tokens": 1, "cached_input_tokens": 1
        });
        let third = fixture.usage(3_000, 3);
        if split {
            fixture.write_segment("rollout-a.jsonl", vec![first]);
            fixture.write_continuation_segment("rollout-b.jsonl", vec![second, third]);
        } else {
            fixture.write_segment("rollout-a.jsonl", vec![first, second, third]);
        }
        let snapshot = fixture.snapshot();
        let response = snapshot_response(&snapshot);
        assert_eq!(response["complete"], true, "split={split}");
        assert_eq!(response["days"][0]["usage"]["totalTokens"], 330);
        assert_eq!(response["days"][0]["usage"]["cachedInputTokens"], 40);
        assert_eq!(response["days"][0]["usage"]["cachedKnown"], false);
        assert!(!snapshot.events[1].delta.cached_known);
        assert_eq!(snapshot.events[1].delta.total_tokens, 110);
        assert!(snapshot.events[2].delta.cached_known);
        assert_eq!(snapshot.events[2].delta.cached_input_tokens, 20);
    }
}

#[test]
fn local_matching_last_recovers_missing_cached_delta() {
    let mut fixture = Fixture::new();
    let first = fixture.usage(1_000, 1);
    let mut second = fixture.usage(2_000, 2);
    second["payload"]["info"]["total_token_usage"]
        .as_object_mut()
        .unwrap()
        .remove("cached_input_tokens");
    fixture.write_segment("rollout-a.jsonl", vec![first]);
    fixture.write_segment("rollout-b.jsonl", vec![second]);
    let snapshot = fixture.snapshot();
    let response = snapshot_response(&snapshot);
    assert_eq!(response["days"][0]["usage"]["totalTokens"], 220);
    assert_eq!(response["days"][0]["usage"]["cachedInputTokens"], 40);
    assert_eq!(response["days"][0]["usage"]["cachedKnown"], true);
    assert_eq!(response["days"][0]["usage"]["reasoningKnown"], true);
}

#[test]
fn local_unknown_cache_keeps_legacy_id_but_rejects_zero_cache_evidence() {
    let mut fixture = Fixture::new();
    let mut row = fixture.usage(1_000, 1);
    for name in ["total_token_usage", "last_token_usage"] {
        row["payload"]["info"][name]
            .as_object_mut()
            .unwrap()
            .remove("cached_input_tokens");
    }
    let timestamp = DateTime::parse_from_rfc3339(row["timestamp"].as_str().unwrap())
        .unwrap()
        .with_timezone(&chrono::Utc)
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let old_tokens = json!({"inputTokens":100, "cachedInputTokens":0,
        "outputTokens":10, "totalTokens":110, "reasoningOutputTokens":2});
    let mut digest = Sha256::new();
    digest.update(SESSION.as_bytes());
    digest.update(timestamp.as_bytes());
    digest.update(serde_json::to_vec(&json!([old_tokens.clone(), old_tokens])).unwrap());
    let expected: [u8; 32] = digest.finalize().into();
    fixture.write_segment("rollout-a.jsonl", vec![row]);
    let initial = fixture.snapshot();
    assert_eq!(initial.events[0].id, expected);
    assert!(!initial.events[0].delta.cached_known);
    fixture.write_evidence(vec![evidence(&initial.events[0], "model-a")]);
    reopen_index(&mut fixture);
    let restored = fixture.snapshot();
    assert!(unknown_provider(&restored.events[0].provider));
    assert_eq!(restored.provider_evidence["appliedEvents"], 0);
}

#[test]
fn local_reset_with_unknown_cached_count_preserves_known_input_and_output() {
    for split in [false, true] {
        let mut fixture = Fixture::new();
        let first = fixture.usage(1_000, 1);
        let mut reset = fixture.usage(2_000, 1);
        reset["payload"]["info"]["total_token_usage"] = json!({"input_tokens":5,"output_tokens":1});
        reset["payload"]["info"]["last_token_usage"] = json!({"input_tokens":5,"output_tokens":1});
        if split {
            fixture.write_segment("rollout-a.jsonl", vec![first]);
            fixture.write_segment("rollout-b.jsonl", vec![reset]);
        } else {
            fixture.write_segment("rollout-a.jsonl", vec![first, reset]);
        }
        let snapshot = fixture.snapshot();
        let response = snapshot_response(&snapshot);
        assert_eq!(response["complete"], true);
        assert_eq!(response["days"][0]["usage"]["totalTokens"], 116);
        assert_eq!(response["days"][0]["usage"]["cachedKnown"], false);
        assert_eq!(snapshot.events[1].delta.total_tokens, 6);
        assert!(!snapshot.events[1].delta.cached_known);
    }
}

#[test]
fn local_unknown_cache_blocks_competing_proxy_attribution() {
    let mut fixture = Fixture::new();
    fixture.write_classified_segment("rollout-a.jsonl", "model-a", "custom", "main");
    let mut event = fixture.snapshot().events.remove(0);
    let at = DateTime::parse_from_rfc3339(&event.timestamp)
        .unwrap()
        .timestamp_millis();
    let mut proof = request();
    proof.started_at_ms = at - 100;
    proof.timestamp_ms = at - 10;
    let mut missing = event.clone();
    missing.id = [34; 32];
    missing.delta.cached_known = false;
    event.provider.clear();
    let mut events = vec![event, missing];
    let mut cache = RequestEvidenceCache::default();
    cache.records.push(proof);
    let result = cache.attribute(&mut events, &mut BTreeMap::new(), &BTreeMap::new());
    assert_eq!(result["recoveredEvents"], 0);
    assert!(unknown_provider(&events[0].provider));
}

#[test]
fn local_missing_required_counts_are_partial_and_recover_without_a_durable_conflict() {
    for missing in ["input_tokens", "output_tokens"] {
        for split in [false, true] {
            let mut fixture = Fixture::new();
            let first = fixture.usage(1_000, 1);
            let mut invalid = fixture.usage(2_000, 2);
            invalid["payload"]["info"]["total_token_usage"]
                .as_object_mut()
                .unwrap()
                .remove(missing);
            let third = fixture.usage(3_000, 3);
            if split {
                fixture.write_segment("rollout-a.jsonl", vec![first.clone()]);
                fixture.write_segment("rollout-b.jsonl", vec![invalid, third.clone()]);
            } else {
                fixture.write_segment(
                    "rollout-a.jsonl",
                    vec![first.clone(), invalid, third.clone()],
                );
            }
            let partial = fixture
                .index
                .update_with_request_log(&fixture.home, &fixture.log);
            assert!(partial.issues > 0, "{missing}, split={split}");
            assert!(partial.events.iter().all(|event| event.count_incomplete));
            let response = snapshot_response(&partial);
            assert_eq!(response["complete"], false);
            assert_eq!(response["days"], json!([]));
            let valid = fixture.usage(2_000, 2);
            if split {
                fixture.write_segment("rollout-b.jsonl", vec![valid, third]);
            } else {
                fixture.write_segment("rollout-a.jsonl", vec![first, valid, third]);
            }
            reopen_index(&mut fixture);
            let restored = snapshot_response(&fixture.snapshot());
            assert_eq!(restored["complete"], true);
            assert_eq!(restored["days"][0]["usage"]["totalTokens"], 330);
        }
    }
}

#[test]
fn local_competing_last_values_are_incomplete_in_both_orders() {
    for short_first in [false, true] {
        for metadata_variant in 0..3 {
            let mut fixture = Fixture::new();
            let full = fixture.usage(1_000, 1);
            let mut short = full.clone();
            short["payload"]["info"]["last_token_usage"] = json!({
                "input_tokens": 20, "output_tokens": 2, "cached_input_tokens": 4,
                "reasoning_output_tokens": 1
            });
            if metadata_variant == 1 {
                short["payload"]["info"]["total_token_usage"]["reasoning_output_tokens"] =
                    Value::Null;
            } else if metadata_variant == 2 {
                short["payload"]["info"]["total_token_usage"]["cached_input_tokens"] = json!(19);
            }
            let (first, second) = if short_first {
                (short, full)
            } else {
                (full, short)
            };
            fixture.write_segment(
                "rollout-a.jsonl",
                vec![first, fixture.usage(2_000, 2), fixture.usage(3_000, 3)],
            );
            fixture.write_segment("rollout-b.jsonl", vec![second]);
            let snapshot = fixture.snapshot();
            let response = snapshot_response(&snapshot);
            let uncertain = if metadata_variant == 0 { 2 } else { 3 };
            assert_eq!(response["complete"], false);
            assert_eq!(response["scan"]["uncertainCountEvents"], uncertain);
            assert_eq!(
                response["days"][0]["usage"]["totalTokens"],
                if metadata_variant == 0 { 220 } else { 110 }
            );
            assert!(!snapshot.events.last().unwrap().count_uncertain);
            let db = rusqlite::Connection::open(
                fixture
                    ._root
                    .path()
                    .join("cache/usage-provider-cache.sqlite"),
            )
            .unwrap();
            for event in snapshot.events.iter().take(2) {
                let data: String = db
                    .query_row(
                        "SELECT data FROM native_attributions WHERE id=?1",
                        [event.id.as_slice()],
                        |row| row.get(0),
                    )
                    .unwrap();
                assert_eq!(
                    serde_json::from_str::<Value>(&data).unwrap()["count_conflict"],
                    2
                );
            }
        }
    }
}

#[test]
fn local_original_event_id_retains_legacy_provider_evidence() {
    let mut fixture = Fixture::new();
    let row = fixture.usage(1_000, 1);
    let total = Tokens::parse(&row["payload"]["info"]["total_token_usage"]).unwrap();
    let last = Tokens::parse(&row["payload"]["info"]["last_token_usage"]);
    let timestamp = DateTime::parse_from_rfc3339(row["timestamp"].as_str().unwrap())
        .unwrap()
        .with_timezone(&chrono::Utc)
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let mut hash = Sha256::new();
    hash.update(SESSION.as_bytes());
    hash.update(timestamp.as_bytes());
    hash.update(serde_json::to_vec(&json!([total, last])).unwrap());
    let legacy_id: [u8; 32] = hash.finalize().into();
    fixture.write_segment("rollout-a.jsonl", vec![row]);
    let initial = fixture.snapshot();
    assert_eq!(initial.events[0].id, legacy_id);
    let mut record = evidence(&initial.events[0], "model-a");
    record["eventId"] = json!(
        legacy_id
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    fixture.write_evidence(vec![record]);
    reopen_index(&mut fixture);
    let restored = fixture.snapshot();
    assert_eq!(restored.events[0].id, legacy_id);
    assert_eq!(restored.events[0].provider, "supplier-a");
    assert_eq!(restored.provider_evidence["appliedEvents"], 1);
}

#[test]
fn local_missing_last_after_reset_does_not_claim_complete() {
    for split in [false, true] {
        let mut fixture = Fixture::new();
        let first = fixture.usage(1_000, 1);
        let mut reset = fixture.usage(2_000, 1);
        reset["payload"]["info"]["total_token_usage"] =
            json!({"input_tokens": 5, "output_tokens": 1});
        reset["payload"]["info"]["last_token_usage"] = Value::Null;
        let mut later = fixture.usage(3_000, 1);
        later["payload"]["info"]["total_token_usage"] = json!({
            "input_tokens": 105, "output_tokens": 11, "cached_input_tokens": 20,
            "reasoning_output_tokens": 2
        });
        if split {
            fixture.write_segment("rollout-a.jsonl", vec![first]);
            fixture.write_continuation_segment("rollout-b.jsonl", vec![reset, later]);
        } else {
            fixture.write_segment("rollout-a.jsonl", vec![first, reset, later]);
        }
        let snapshot = fixture.snapshot();
        let response = snapshot_response(&snapshot);
        assert_eq!(response["complete"], false, "split={split}");
        assert_eq!(response["scan"]["uncertainCountEvents"], 1);
        assert_eq!(response["days"][0]["usage"]["totalTokens"], 220);
        assert!(snapshot.events[1].count_uncertain);
        assert!(!snapshot.events[2].count_uncertain);
    }
}

#[test]
fn local_separate_indexes_reload_persisted_conflicts() {
    let mut fixture = Fixture::new();
    let event = fixture.usage(1_000, 1);
    fixture.write_segment("rollout-a.jsonl", vec![event.clone()]);
    assert_eq!(snapshot_response(&fixture.snapshot())["complete"], true);
    let mut other = Index {
        settings_path: Some(fixture.settings.clone()),
        storage_dir: Some(fixture._root.path().join("cache")),
        ..Index::default()
    };
    assert_eq!(
        snapshot_response(&other.update_with_request_log(&fixture.home, &fixture.log))["complete"],
        true
    );
    let mut previous = fixture.usage(0, 1);
    previous["timestamp"] = json!((Local::now() - Days::days(RETENTION_DAYS + 2)).to_rfc3339());
    previous["payload"]["info"]["total_token_usage"] = json!({
        "input_tokens": 80, "output_tokens": 5, "cached_input_tokens": 10, "reasoning_output_tokens": 1
    });
    fixture.write_segment("rollout-b.jsonl", vec![previous, event]);
    assert_eq!(snapshot_response(&fixture.snapshot())["complete"], false);
    fixture.write_segment("rollout-b.jsonl", vec![]);
    let response = snapshot_response(&other.update_with_request_log(&fixture.home, &fixture.log));
    assert_eq!(response["complete"], false);
    assert_eq!(response["scan"]["uncertainCountEvents"], 1);
    drop(other);
    reopen_index(&mut fixture);
    assert_eq!(snapshot_response(&fixture.snapshot())["complete"], false);
}

#[test]
fn local_final_availability_rechecks_late_global_guard() {
    let mut fixture = Fixture::new();
    fixture.write_segment("rollout-a.jsonl", vec![fixture.usage(1_000, 1)]);
    assert_eq!(snapshot_response(&fixture.snapshot())["complete"], true);
    let cutoff = Local::now().date_naive() - Days::days(RETENTION_DAYS - 1);
    fixture.index.sync_native_evidence(cutoff).unwrap();
    let db = rusqlite::Connection::open(
        fixture
            ._root
            .path()
            .join("cache/usage-provider-cache.sqlite"),
    )
    .unwrap();
    let at = Local::now().timestamp_millis();
    db.execute(
        "INSERT INTO structured_count_guard(singleton,at) VALUES(1,?1)",
        [at],
    )
    .unwrap();
    assert!(
        !fixture
            .index
            .native_evidence_available(&fixture.home, cutoff)
            .unwrap()
    );
    assert_eq!(
        db.query_row(
            "SELECT at FROM structured_count_guard WHERE singleton=1",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        at
    );
    let response = snapshot_response(&fixture.snapshot());
    assert_eq!(response["complete"], false);
    assert_eq!(response["scan"]["countCacheAvailable"], false);
    assert_eq!(response["days"], json!([]));
}

const SESSION: &str = "00000000-0000-4000-8000-000000000010";

struct Fixture {
    _root: tempfile::TempDir,
    home: PathBuf,
    settings: PathBuf,
    log: PathBuf,
    index: Index,
    start: i64,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let settings = root.path().join("settings.json");
        let log = root.path().join("requests.jsonl");
        fs::create_dir_all(home.join("sessions")).unwrap();
        let home = fs::canonicalize(home).unwrap();
        fs::write(
            &settings,
            r#"{"usageHistoryEnabled":true,"relayProfiles":[]}"#,
        )
        .unwrap();
        fs::write(&log, "").unwrap();
        let index = Index {
            settings_path: Some(settings.clone()),
            storage_dir: Some(root.path().join("cache")),
            ..Index::default()
        };
        Self {
            _root: root,
            home,
            settings,
            log,
            index,
            start: Local::now().timestamp_millis() - 60_000,
        }
    }

    fn at(&self, offset: i64) -> String {
        DateTime::from_timestamp_millis(self.start + offset)
            .unwrap()
            .to_rfc3339()
    }

    fn usage(&self, offset: i64, ordinal: u64) -> Value {
        json!({
            "type": "event_msg", "timestamp": self.at(offset),
            "payload": {"type": "token_count", "info": {
                "total_token_usage": {
                    "input_tokens": ordinal * 100, "output_tokens": ordinal * 10,
                    "cached_input_tokens": ordinal * 20, "reasoning_output_tokens": ordinal * 2
                },
                "last_token_usage": {
                    "input_tokens": 100, "output_tokens": 10,
                    "cached_input_tokens": 20, "reasoning_output_tokens": 2
                }
            }}
        })
    }

    fn context(&self, offset: i64, model: &str) -> Value {
        json!({
            "type": "turn_context", "timestamp": self.at(offset),
            "payload": {"model": model}
        })
    }

    fn write_segment(&self, name: &str, rows: Vec<Value>) {
        let head = json!({
            "type": "session_meta", "timestamp": self.at(0),
            "payload": {"id": SESSION, "model_provider": "custom"}
        });
        let text = std::iter::once(head)
            .chain(rows)
            .map(|row| format!("{row}\n"))
            .collect::<String>();
        fs::write(self.home.join("sessions").join(name), text).unwrap();
    }

    fn write_continuation_segment(&self, name: &str, mut rows: Vec<Value>) {
        rows.insert(
            0,
            json!({"type":"session_meta","payload":{
                "id":SESSION,"history_base":{"end_ordinal_exclusive":1}
            }}),
        );
        self.write_segment(name, rows);
    }

    fn write_classified_segment(&self, name: &str, model: &str, provider: &str, role: &str) {
        let head = json!({
            "type": "session_meta", "timestamp": self.at(0),
            "payload": {"id": SESSION, "model_provider": provider,
                "source": if role == "subagent" { json!({"subagent": {}}) } else { json!("vscode") }}
        });
        let mut context = self.context(500, model);
        context["payload"]["model_provider"] = json!(provider);
        let text = [head, context, self.usage(1_000, 1)]
            .into_iter()
            .map(|row| format!("{row}\n"))
            .collect::<String>();
        fs::write(self.home.join("sessions").join(name), text).unwrap();
    }

    fn write_inherited_segment(&self, name: &str, model: &str, provider: &str) {
        let child = json!({"type": "session_meta", "payload": {
            "id": "00000000-0000-4000-8000-000000000011", "forked_from_id": SESSION,
            "subagent_history_start_ordinal": 3, "source": {"subagent": {}}
        }});
        let parent = json!({"type": "session_meta", "ordinal": 0,
            "payload": {"id": SESSION, "model_provider": provider}});
        let mut context = self.context(500, model);
        context["ordinal"] = json!(1);
        context["payload"]["model_provider"] = json!(provider);
        let mut usage = self.usage(1_000, 1);
        usage["ordinal"] = json!(2);
        let text = [child, parent, context, usage]
            .into_iter()
            .map(|row| format!("{row}\n"))
            .collect::<String>();
        fs::write(self.home.join("sessions").join(name), text).unwrap();
    }

    fn write_evidence(&self, records: Vec<Value>) {
        fs::write(
            self.settings.with_file_name("usage-provider-evidence.json"),
            serde_json::to_vec(&json!({"version": 1, "records": records})).unwrap(),
        )
        .unwrap();
    }

    fn snapshot(&mut self) -> Snapshot {
        let snapshot = self.index.update_with_request_log(&self.home, &self.log);
        assert_eq!(snapshot.errors, 0);
        assert_eq!(snapshot.issues, 0);
        snapshot
    }
}

fn evidence(event: &UsageEvent, model: &str) -> Value {
    json!({
        "eventId": event.id.iter().map(|byte| format!("{byte:02x}")).collect::<String>(),
        "session": event.session, "timestamp": event.timestamp, "model": model,
        "role": event.role, "input": event.delta.input_tokens,
        "output": event.delta.output_tokens, "cached": event.delta.cached_input_tokens,
        "provider": "supplier-a", "providerName": "Supplier A",
        "source": "upstream-exact-bidirectional"
    })
}

fn assert_accounting(snapshot: &Snapshot, count: usize) {
    assert_eq!(snapshot.events.len(), count);
    let query = Query::parse(&json!({}), Local::now().date_naive()).unwrap();
    let scoped = snapshot.scoped_usage(SESSION, &query, None);
    for days in [
        json!(snapshot.usage_days(&query, None, None)),
        scoped["days"].clone(),
    ] {
        for (field, amount) in [
            ("inputTokens", 100),
            ("outputTokens", 10),
            ("cachedInputTokens", 20),
            ("totalTokens", 110),
            ("reasoningOutputTokens", 2),
        ] {
            let total: u64 = days
                .as_array()
                .unwrap()
                .iter()
                .map(|day| day["usage"][field].as_u64().unwrap())
                .sum();
            assert_eq!(total, count as u64 * amount, "{field}");
        }
    }
}

#[test]
fn model_lengths_are_indexed_and_filtered_by_utf8_bytes() {
    let mut fixture = Fixture::new();
    let models = [
        "a".repeat(200),
        "b".repeat(201),
        "c".repeat(256),
        format!("{}a", "模".repeat(85)),
    ];
    let mut rows = Vec::new();
    for (position, model) in models.iter().enumerate() {
        let offset = (position as i64 + 1) * 1_000;
        rows.push(fixture.context(offset, model));
        rows.push(fixture.usage(offset + 500, position as u64 + 1));
    }
    fixture.write_segment("rollout-lengths.jsonl", rows);
    for phase in ["fresh", "reopened"] {
        if phase == "reopened" {
            reopen_index(&mut fixture);
        }
        let snapshot = fixture.snapshot();
        assert_eq!(snapshot.events.len(), models.len(), "{phase}");
        assert_accounting(&snapshot, models.len());
        for model in &models {
            let query = Query::parse(&json!({"model":model}), Local::now().date_naive()).unwrap();
            let days = snapshot.usage_days(&query, None, None);
            assert_eq!(days[0]["recordCount"], 1, "{phase} {}", model.len());
        }
        assert_eq!(
            snapshot.session_info[SESSION]["model"], models[3],
            "{phase}"
        );
    }
    assert!(Query::parse(&json!({"model":"a".repeat(257)}), Local::now().date_naive()).is_none());
    assert!(
        Query::parse(
            &json!({"turnId":"a".repeat(201)}),
            Local::now().date_naive()
        )
        .is_none()
    );
    assert!(
        Query::parse(
            &json!({"search":"a".repeat(201)}),
            Local::now().date_naive()
        )
        .is_none()
    );
}

#[test]
fn oversized_model_blocks_old_context_evidence_and_duplicate_recovery() {
    let mut fixture = Fixture::new();
    let oversized = "x".repeat(257);
    let rows = vec![
        fixture.context(500, "old-model"),
        fixture.usage(1_000, 1),
        fixture.context(1_500, &oversized),
        fixture.usage(2_000, 2),
    ];
    fixture.write_segment("rollout-a.jsonl", rows);
    let mut duplicate = fixture.usage(2_000, 2);
    duplicate["payload"]["info"]["total_token_usage"]["reasoning_output_tokens"] = json!(4);
    fixture.write_segment(
        "rollout-b.jsonl",
        vec![fixture.context(500, "old-model"), duplicate],
    );
    let mut stale = request();
    stale.model = "old-model".into();
    stale.upstream_model = "old-model".into();
    stale.started_at_ms = fixture.start + 1_800;
    stale.timestamp_ms = fixture.start + 1_900;
    fs::write(
        &fixture.log,
        format!("{}\n", serde_json::to_string(&stale).unwrap()),
    )
    .unwrap();
    for phase in ["fresh", "reopened"] {
        if phase == "reopened" {
            reopen_index(&mut fixture);
        }
        let snapshot = fixture.snapshot();
        assert_accounting(&snapshot, 2);
        let mut events = snapshot.events.clone();
        events.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
        assert_eq!(events[0].model, "old-model", "{phase}");
        assert!(events[1].model.is_empty(), "{phase}");
        assert!(events[1].context_conflicts.model, "{phase}");
        assert_eq!(snapshot.session_info[SESSION]["model"], "", "{phase}");
    }
}

#[test]
fn oversized_context_recovers_on_a_later_continuation() {
    let mut fixture = Fixture::new();
    fixture.write_segment(
        "rollout-a.jsonl",
        vec![
            fixture.context(500, "old-model"),
            fixture.usage(1_000, 1),
            fixture.context(1_500, &"x".repeat(257)),
            fixture.usage(2_000, 2),
        ],
    );
    fixture.write_continuation_segment(
        "rollout-b.jsonl",
        vec![fixture.context(2_500, "new-model"), fixture.usage(3_000, 3)],
    );
    for phase in ["fresh", "reopened"] {
        if phase == "reopened" {
            reopen_index(&mut fixture);
        }
        let snapshot = fixture.snapshot();
        assert_accounting(&snapshot, 3);
        let mut events = snapshot.events.clone();
        events.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
        assert_eq!(events[0].model, "old-model", "{phase}");
        assert!(events[1].model.is_empty(), "{phase}");
        assert_eq!(events[2].model, "new-model", "{phase}");
        assert_eq!(
            snapshot.session_info[SESSION]["model"], "new-model",
            "{phase}"
        );
    }
}

fn assert_models(snapshot: &Snapshot, expected: &[&str]) {
    assert_eq!(
        snapshot
            .events
            .iter()
            .map(|event| event.model.as_str())
            .collect::<Vec<_>>(),
        expected
    );
}

fn request() -> RequestEvidence {
    RequestEvidence {
        id: "request-1".into(),
        thread_id: SESSION.into(),
        model: "model-a".into(),
        upstream_model: "model-a".into(),
        relay_id: "supplier-a".into(),
        relay_name: "Supplier A".into(),
        success: true,
        state: "success".into(),
        status_code: 200,
        timestamp_ms: 100_000,
        started_at_ms: 99_000,
        attempts: Some(1),
        auto_retries: Some(0),
        final_response_evidence: None,
        usage: Some(RequestEvidenceUsage {
            input_tokens: Some(100),
            output_tokens: Some(10),
            cached_tokens: Some(20),
        }),
        agent: String::new(),
    }
}

#[test]
fn request_routing_recovers_actual_classification_without_changing_accounting() {
    for model in ["", "unknown", "auto", "group/pool", "cpx-route/model-a"] {
        let mut fixture = Fixture::new();
        fixture.write_classified_segment("rollout-a.jsonl", model, "custom", "main");
        let original = fixture.snapshot().events.remove(0);
        let at = DateTime::parse_from_rfc3339(&original.timestamp)
            .unwrap()
            .timestamp_millis();
        let mut proof = request();
        proof.model = if model.is_empty() {
            "auto".into()
        } else {
            model.into()
        };
        proof.upstream_model = "actual-model".into();
        proof.timestamp_ms = at - 10;
        proof.started_at_ms = at - 100;
        let mut cache = RequestEvidenceCache::default();
        cache.records.push(proof);
        let mut events = vec![original.clone()];
        let result = cache.attribute(&mut events, &mut BTreeMap::new(), &BTreeMap::new());
        assert_eq!(result["recoveredEvents"], 1, "{model}");
        assert_eq!(events[0].model, "actual-model", "{model}");
        assert_eq!(events[0].provider, "supplier-a", "{model}");
        assert_eq!(
            events[0].provider_source.as_deref(),
            Some("request-log-exact")
        );
        assert_eq!(events[0].id, original.id);
        assert_eq!(events[0].total, original.total);
        assert_eq!(events[0].delta, original.delta);
        let query = Query::parse(&json!({}), Local::now().date_naive()).unwrap();
        let before = Snapshot {
            events: vec![original],
            count_cache_available: true,
            ..Snapshot::default()
        }
        .usage_days(&query, None, None);
        let after = Snapshot {
            events,
            count_cache_available: true,
            ..Snapshot::default()
        }
        .usage_days(&query, None, None);
        assert_eq!(
            before[0]["accountingFingerprint"],
            after[0]["accountingFingerprint"]
        );
        assert_eq!(before[0]["usage"], after[0]["usage"]);
        assert_eq!(
            after[0]["breakdowns"][0]["classificationEvidence"],
            "request-log-exact"
        );
    }
}

#[test]
fn request_routing_independent_unknown_model_does_not_block_exact_counts() {
    let mut fixture = Fixture::new();
    fixture.write_classified_segment("rollout-a.jsonl", "cpx-route/model-a", "custom", "main");
    let original = fixture.snapshot().events.remove(0);
    let mut other = original.clone();
    other.id = [42; 32];
    other.model = "group/pool".into();
    other.delta = Tokens::new(200, 30, 20);
    let at = DateTime::parse_from_rfc3339(&original.timestamp)
        .unwrap()
        .timestamp_millis();
    let mut proof = request();
    proof.model = original.model.clone();
    proof.upstream_model = "actual-model".into();
    proof.timestamp_ms = at - 10;
    proof.started_at_ms = at - 100;
    let mut cache = RequestEvidenceCache::default();
    cache.records.push(proof);
    let mut events = vec![original, other];
    let result = cache.attribute(&mut events, &mut BTreeMap::new(), &BTreeMap::new());
    assert_eq!(result["recoveredEvents"], 1);
    assert_eq!(events[0].provider, "supplier-a");
    assert!(unknown_provider(&events[1].provider));
}

#[test]
fn request_routing_competes_with_known_and_unknown_events_in_both_directions() {
    for competitor_model in ["actual-model", "group/pool", "unknown"] {
        let mut fixture = Fixture::new();
        fixture.write_classified_segment("rollout-a.jsonl", "auto", "custom", "main");
        let original = fixture.snapshot().events.remove(0);
        let mut other = original.clone();
        other.id = [43; 32];
        other.model = competitor_model.into();
        let at = DateTime::parse_from_rfc3339(&original.timestamp)
            .unwrap()
            .timestamp_millis();
        let mut proof = request();
        proof.model = "auto".into();
        proof.upstream_model = "actual-model".into();
        proof.timestamp_ms = at - 10;
        proof.started_at_ms = at - 100;
        let mut cache = RequestEvidenceCache::default();
        cache.records.push(proof.clone());
        let mut events = vec![original.clone(), other];
        let result = cache.attribute(&mut events, &mut BTreeMap::new(), &BTreeMap::new());
        assert_eq!(result["recoveredEvents"], 0, "{competitor_model}");
        let mut second = proof;
        second.id = "competing-request".into();
        cache.records.push(second);
        let mut events = vec![original];
        let result = cache.attribute(&mut events, &mut BTreeMap::new(), &BTreeMap::new());
        assert_eq!(result["recoveredEvents"], 0, "{competitor_model}");
    }
}

#[test]
fn request_routing_retains_and_revokes_actual_classification_after_log_rotation() {
    let mut fixture = Fixture::new();
    fixture.write_classified_segment("rollout-a.jsonl", "group/pool", "custom", "main");
    let original = fixture.snapshot().events.remove(0);
    let at = DateTime::parse_from_rfc3339(&original.timestamp)
        .unwrap()
        .timestamp_millis();
    let mut proof = request();
    proof.model = "group/pool".into();
    proof.timestamp_ms = at - 10;
    proof.started_at_ms = at - 100;
    let mut cache = RequestEvidenceCache::default();
    cache.records.push(proof.clone());
    let mut events = vec![original.clone()];
    cache.attribute(&mut events, &mut BTreeMap::new(), &BTreeMap::new());
    assert_eq!(events[0].provider, "supplier-a");
    cache.records.clear();
    let mut events = vec![original.clone()];
    cache.attribute(&mut events, &mut BTreeMap::new(), &BTreeMap::new());
    assert_eq!(events[0].provider, "supplier-a");
    assert_eq!(events[0].model, "model-a");
    let mut competitor = proof;
    competitor.id = "late-competitor".into();
    cache.records.push(competitor);
    let mut events = vec![original];
    cache.attribute(&mut events, &mut BTreeMap::new(), &BTreeMap::new());
    assert!(unknown_provider(&events[0].provider));
    assert_eq!(events[0].model, "group/pool");
    assert!(cache.verified[&events[0].id].rejected);
}

#[test]
fn request_routing_restores_legacy_aggregate_proof() {
    let mut fixture = Fixture::new();
    fixture.write_classified_segment("rollout-a.jsonl", "cpx-route/model-a", "custom", "main");
    let original = fixture.snapshot().events.remove(0);
    let at = DateTime::parse_from_rfc3339(&original.timestamp)
        .unwrap()
        .timestamp_millis();
    let mut proof = request();
    proof.model = original.model.clone();
    proof.upstream_model = "actual-model".into();
    proof.timestamp_ms = at - 10;
    proof.started_at_ms = at - 100;
    let mut cache = RequestEvidenceCache::default();
    cache.verified.insert(
        original.id,
        VerifiedAttribution {
            key: RequestMatchKey {
                session: SESSION.into(),
                model: "model-a".into(),
                input: 100,
                output: 10,
                cached: 20,
            },
            at,
            request: proof,
            rejected: false,
            version: 0,
        },
    );
    let mut events = vec![original];
    cache.attribute(&mut events, &mut BTreeMap::new(), &BTreeMap::new());
    assert_eq!(events[0].provider, "supplier-a");
    assert_eq!(events[0].model, "actual-model");
    assert!(!cache.verified[&events[0].id].rejected);
}

#[test]
fn cpx_request_proofs_require_the_complete_model_and_preserve_upstream_names() {
    let mut fixture = Fixture::new();
    fixture.write_classified_segment("rollout-cpx.jsonl", "cpx-a/model", "custom", "main");
    let original = fixture.snapshot().events.remove(0);
    let at = DateTime::parse_from_rfc3339(&original.timestamp)
        .unwrap()
        .timestamp_millis();
    let mut record = request();
    record.timestamp_ms = at - 10;
    record.started_at_ms = at - 100;
    record.model = "cpx-b/model".into();
    record.upstream_model = "model".into();

    let mut wrong = vec![original.clone()];
    let mut cache = RequestEvidenceCache::default();
    cache.records.push(record.clone());
    cache.attribute(&mut wrong, &mut BTreeMap::new(), &BTreeMap::new());
    assert!(unknown_provider(&wrong[0].provider));
    assert_eq!(wrong[0].model, original.model);

    record.model = original.model.clone();
    record.upstream_model = "cpx-native/model".into();
    let mut correct = vec![original.clone()];
    let mut cache = RequestEvidenceCache::default();
    cache.records.push(record);
    cache.attribute(&mut correct, &mut BTreeMap::new(), &BTreeMap::new());
    assert_eq!(correct[0].provider, "supplier-a");
    assert_eq!(correct[0].model, "cpx-native/model");
    assert_eq!(correct[0].delta, original.delta);
    assert_eq!(cache.verified[&original.id].version, 1);
    cache.records.clear();
    let mut restored = vec![original.clone()];
    cache.attribute(&mut restored, &mut BTreeMap::new(), &BTreeMap::new());
    assert_eq!(restored[0].model, "cpx-native/model");
}

#[test]
fn aggregate_original_request_identity_blocks_upstream_only_event_matches() {
    let mut recovered = Vec::new();
    for (model, requested) in [
        ("cpx-a/model", "cpx-b/model"),
        ("model", "cpx-b/model"),
        ("cpx-a/model", "other"),
        ("cpx-a/model", "group/pool"),
    ] {
        let mut fixture = Fixture::new();
        fixture.write_classified_segment("rollout-cpx.jsonl", model, "custom", "main");
        let original = fixture.snapshot().events.remove(0);
        let at = DateTime::parse_from_rfc3339(&original.timestamp)
            .unwrap()
            .timestamp_millis();
        let mut record = request();
        record.timestamp_ms = at - 10;
        record.started_at_ms = at - 100;
        record.model = requested.into();
        record.upstream_model = model.into();
        let mut cache = RequestEvidenceCache::default();
        cache.records.push(record.clone());
        let mut events = vec![original.clone()];
        let result = cache.attribute(&mut events, &mut BTreeMap::new(), &BTreeMap::new());
        recovered.push(result["recoveredEvents"].as_u64().unwrap());
        assert_eq!(events[0].id, original.id);
        assert_eq!(events[0].delta, original.delta);
        assert_eq!(events[0].model, original.model);
        assert!(!cache.verified.contains_key(&original.id));

        cache.verified.insert(
            original.id,
            VerifiedAttribution {
                key: RequestMatchKey {
                    session: SESSION.into(),
                    model: original.model.clone(),
                    input: 100,
                    output: 10,
                    cached: 20,
                },
                at,
                request: record,
                rejected: false,
                version: 1,
            },
        );
        let mut replay = vec![original.clone()];
        cache.attribute(&mut replay, &mut BTreeMap::new(), &BTreeMap::new());
        assert!(cache.verified[&original.id].rejected);
        assert!(unknown_provider(&replay[0].provider));
    }
    assert_eq!(recovered, [0, 0, 0, 0]);
}

#[test]
fn aggregate_original_request_identity_blocks_upstream_only_live_receipts() {
    let mut attributed = Vec::new();
    for model in ["cpx-a/model", "model"] {
        let mut record = request();
        record.model = "cpx-b/model".into();
        record.upstream_model = model.into();
        let mut row = receipt("cpx-mismatch");
        row["model"] = json!(model);
        let result = match_receipts(&[record.clone()], json!([row.clone()]));
        attributed.push(result[0]["provider"] == "supplier-a");

        row["model"] = json!("cpx-b/model");
        let proof = match_receipts(&[record.clone()], json!([row.clone()]));
        assert_eq!(proof[0]["model"], model);
        for key in ["model", "provider", "requestIds", "requestSignatures"] {
            row[key] = proof[0][key].clone();
        }
        let replay = match_receipts(&[record], json!([row]));
        assert_eq!(replay[0]["provider"], "supplier-a");
        assert_eq!(replay[0]["model"], model);
    }
    assert_eq!(attributed, [false, false]);
}

#[test]
fn aggregate_upstream_identity_cannot_replace_original_live_request() {
    for requested in ["other", "group/pool"] {
        let mut record = request();
        record.model = requested.into();
        record.upstream_model = "cpx-a/model".into();
        let mut row = receipt("cpx-mismatch");
        row["model"] = json!("cpx-a/model");
        let result = match_receipts(&[record], json!([row]));
        assert_ne!(result[0]["provider"], "supplier-a", "{requested}");
    }

    let mut record = request();
    record.model = "alias/cpx-a/model".into();
    record.upstream_model = "cpx-a/model".into();
    let mut row = receipt("cpx-saved");
    row["model"] = json!(record.model);
    let proof = match_receipts(&[record.clone()], json!([row.clone()]));
    assert_eq!(proof[0]["provider"], "supplier-a");
    assert_eq!(proof[0]["model"], "cpx-a/model");
    for key in ["model", "provider", "requestIds", "requestSignatures"] {
        row[key] = proof[0][key].clone();
    }
    let replay = match_receipts(&[record], json!([row]));
    assert_eq!(replay[0]["provider"], "supplier-a");
    assert_eq!(replay[0]["model"], "cpx-a/model");
}

#[test]
fn session_model_supplement_preserves_legacy_display_and_full_name_uniqueness() {
    let now = Local::now().timestamp_millis();
    let mut record = request();
    record.model = "cpx-a/model".into();
    record.upstream_model = "model".into();
    record.timestamp_ms = now - 10;
    record.started_at_ms = now - 100;
    let mut cache = RequestEvidenceCache::default();
    cache.records.push(record.clone());
    let aliases = RequestSessionAliases::new(&BTreeMap::new());
    let mut sessions = BTreeMap::from([(SESSION.into(), json!({"model":"","role":"main"}))]);
    cache.supplement_missing_session_models(&mut sessions, &HashSet::new(), &aliases, now);
    assert_eq!(sessions[SESSION]["model"], "model");
    assert_eq!(sessions[SESSION]["modelSource"], "request-log-single-model");

    record.id = "request-2".into();
    record.model = "cpx-b/model".into();
    cache.records.push(record);
    let mut sessions = BTreeMap::from([(SESSION.into(), json!({"model":"","role":"main"}))]);
    cache.supplement_missing_session_models(&mut sessions, &HashSet::new(), &aliases, now);
    assert_eq!(sessions[SESSION]["model"], "");

    cache.records.truncate(1);
    cache.records[0].model = "cpx-a/auto".into();
    let mut sessions = BTreeMap::from([(SESSION.into(), json!({"model":"","role":"main"}))]);
    cache.supplement_missing_session_models(&mut sessions, &HashSet::new(), &aliases, now);
    assert_eq!(sessions[SESSION]["model"], "");
}

#[test]
fn legacy_proof_with_bare_event_restores_after_reopen() {
    let mut fixture = Fixture::new();
    fixture.write_classified_segment("rollout-cpx.jsonl", "model", "custom", "main");
    let original = fixture.snapshot().events.remove(0);
    let at = DateTime::parse_from_rfc3339(&original.timestamp)
        .unwrap()
        .timestamp_millis();
    let mut record = request();
    record.timestamp_ms = at - 10;
    record.started_at_ms = at - 100;
    record.model = "cpx-a/model".into();
    record.upstream_model = "actual".into();
    let saved = VerifiedAttribution {
        key: RequestMatchKey {
            session: SESSION.into(),
            model: "model".into(),
            input: 100,
            output: 10,
            cached: 20,
        },
        at,
        request: record,
        rejected: false,
        version: 0,
    };
    let root = tempfile::tempdir().unwrap();
    let cutoff = Local::now().date_naive() - Days::days(RETENTION_DAYS - 1);
    let mut disk = provider_cache::ProviderCache::default();
    disk.open(root.path(), cutoff).unwrap();
    disk.save(
        &HashMap::from([(original.id, saved.clone())]),
        &BTreeMap::new(),
    )
    .unwrap();
    drop(disk);
    let mut disk = provider_cache::ProviderCache::default();
    disk.open(root.path(), cutoff).unwrap();
    let mut cache = RequestEvidenceCache::default();
    cache.restore_verified(disk.load_verified(cutoff).unwrap());
    let mut events = vec![original.clone()];
    cache.attribute(&mut events, &mut BTreeMap::new(), &BTreeMap::new());
    assert_eq!(events[0].provider, "supplier-a");
    assert_eq!(events[0].model, original.model);
    assert_eq!(events[0].delta, original.delta);
    assert!(cache.verified[&original.id] == saved);
}

#[test]
fn legacy_cpx_request_proof_keeps_its_original_v2_result() {
    let mut fixture = Fixture::new();
    fixture.write_classified_segment("rollout-cpx.jsonl", "cpx-a/model", "custom", "main");
    let original = fixture.snapshot().events.remove(0);
    let at = DateTime::parse_from_rfc3339(&original.timestamp)
        .unwrap()
        .timestamp_millis();
    let mut record = request();
    record.timestamp_ms = at - 10;
    record.started_at_ms = at - 100;
    record.model = "cpx-b/model".into();
    record.upstream_model = "cpx-native/actual".into();
    let saved = VerifiedAttribution {
        key: RequestMatchKey {
            session: SESSION.into(),
            model: "model".into(),
            input: 100,
            output: 10,
            cached: 20,
        },
        at,
        request: record,
        rejected: false,
        version: 0,
    };
    let mut cache = RequestEvidenceCache::default();
    let root = tempfile::tempdir().unwrap();
    let cutoff = Local::now().date_naive() - Days::days(RETENTION_DAYS - 1);
    let mut disk = provider_cache::ProviderCache::default();
    disk.open(root.path(), cutoff).unwrap();
    assert!(
        serde_json::to_value(&saved)
            .unwrap()
            .get("version")
            .is_none()
    );
    disk.save(
        &HashMap::from([(original.id, saved.clone())]),
        &BTreeMap::new(),
    )
    .unwrap();
    drop(disk);
    let mut disk = provider_cache::ProviderCache::default();
    disk.open(root.path(), cutoff).unwrap();
    cache.restore_verified(disk.load_verified(cutoff).unwrap());
    let mut historical = vec![original.clone()];
    cache.attribute(&mut historical, &mut BTreeMap::new(), &BTreeMap::new());
    assert_eq!(historical[0].provider, "supplier-a");
    assert_eq!(historical[0].model, "actual");
    assert!(cache.verified[&original.id] == saved);
    assert_eq!(historical[0].delta, original.delta);
}

#[test]
fn legacy_cpx_request_proof_does_not_attribute_an_unknown_upstream_model() {
    let mut fixture = Fixture::new();
    fixture.write_classified_segment("rollout-cpx.jsonl", "cpx-a/model", "custom", "main");
    let original = fixture.snapshot().events.remove(0);
    let at = DateTime::parse_from_rfc3339(&original.timestamp)
        .unwrap()
        .timestamp_millis();
    let mut record = request();
    record.timestamp_ms = at - 10;
    record.started_at_ms = at - 100;
    record.model = "cpx-a/model".into();
    record.upstream_model = "cpx-native/auto".into();
    let mut cache = RequestEvidenceCache::default();
    cache.verified.insert(
        original.id,
        VerifiedAttribution {
            key: RequestMatchKey {
                session: SESSION.into(),
                model: "model".into(),
                input: 100,
                output: 10,
                cached: 20,
            },
            at,
            request: record,
            rejected: false,
            version: 0,
        },
    );
    let mut events = vec![original];
    cache.attribute(&mut events, &mut BTreeMap::new(), &BTreeMap::new());
    assert!(unknown_provider(&events[0].provider));
    assert_eq!(events[0].model, "cpx-a/model");
}

#[test]
fn live_cpx_receipts_require_the_full_request_model_and_preserve_legacy_proof() {
    let mut record = request();
    record.model = "cpx-b/model".into();
    record.upstream_model = "cpx-native/model".into();
    let mut row = receipt("cpx");
    row["model"] = json!("cpx-a/model");
    assert_no_attribution(&match_receipts(&[record.clone()], json!([row.clone()])));

    row["model"] = json!("cpx-b/model");
    let proof = match_receipts(&[record.clone()], json!([row.clone()]));
    assert_eq!(proof[0]["model"], "cpx-native/model");
    row["model"] = json!("model");
    row["provider"] = proof[0]["provider"].clone();
    row["requestIds"] = proof[0]["requestIds"].clone();
    row["requestSignatures"] = proof[0]["requestSignatures"].clone();
    let historical = match_receipts(&[record], json!([row]));
    assert_eq!(historical[0]["model"], "model");
    assert_eq!(historical[0]["provider"], "supplier-a");
}

#[test]
fn request_routing_fills_missing_model_for_an_already_recorded_provider() {
    let mut fixture = Fixture::new();
    fixture.write_classified_segment("rollout-a.jsonl", "auto", "supplier-a", "main");
    let original = fixture.snapshot().events.remove(0);
    let at = DateTime::parse_from_rfc3339(&original.timestamp)
        .unwrap()
        .timestamp_millis();
    let mut proof = request();
    proof.model = "auto".into();
    proof.upstream_model = "actual-model".into();
    proof.timestamp_ms = at - 10;
    proof.started_at_ms = at - 100;
    fs::write(
        &fixture.log,
        format!("{}\n", serde_json::to_string(&proof).unwrap()),
    )
    .unwrap();
    let after = fixture.snapshot();
    assert_eq!(after.events[0].model, "actual-model");
    assert_eq!(after.events[0].provider, "supplier-a");
    assert_eq!(after.events[0].delta, original.delta);
}

#[test]
fn port_exact_request_proof_reaches_live_history_after_rotation_and_reopen() {
    for (model, upstream, expected_model) in [
        ("model-a", "model-a", "model-a"),
        ("auto", "model-a", "model-a"),
        ("group/pool", "model-a", "model-a"),
        ("cpx-route/model-a", "model-a", "model-a"),
        ("supplier-a/model-a", "model-a", "supplier-a/model-a"),
        ("requested-model", "actual-model", "requested-model"),
    ] {
        let mut fixture = Fixture::new();
        fixture.write_classified_segment("rollout-a.jsonl", model, "custom", "main");
        let before = fixture.snapshot();
        let original = &before.events[0];
        let at = DateTime::parse_from_rfc3339(&original.timestamp)
            .unwrap()
            .timestamp_millis();
        let mut proof = request();
        proof.model = model.into();
        proof.upstream_model = upstream.into();
        proof.timestamp_ms = at - 10;
        proof.started_at_ms = at - 100;
        fs::write(
            &fixture.log,
            format!("{}\n", serde_json::to_string(&proof).unwrap()),
        )
        .unwrap();
        let event_id = original
            .id
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let before_days = snapshot_response(&before)["days"].clone();
        for phase in ["fresh", "rotated", "reopened"] {
            if phase != "fresh" {
                fs::write(&fixture.log, "").unwrap();
            }
            if phase == "reopened" {
                reopen_index(&mut fixture);
            }
            let after = fixture.snapshot();
            assert_eq!(after.events[0].provider, "supplier-a", "{model}/{phase}");
            assert_eq!(after.events[0].model, expected_model, "{model}/{phase}");
            let response = snapshot_response(&after);
            let verified = response["sessionInfo"][SESSION]["verifiedUsage"]
                .as_array()
                .expect("exact request proof must reach the live-history consumer");
            assert_eq!(verified.len(), 1, "{model}/{phase}");
            assert_eq!(verified[0]["sourceEventId"], event_id);
            assert_eq!(verified[0]["providerSource"], "request-log-exact");
            assert_eq!(verified[0]["provider"], "supplier-a");
            assert_eq!(verified[0]["model"], expected_model);
            assert_eq!(verified[0]["total"], original.total.value());
            assert_eq!(verified[0]["delta"], original.delta.value());
            assert_eq!(response["days"][0]["usage"], before_days[0]["usage"]);
            assert_eq!(
                response["days"][0]["accountingFingerprint"],
                before_days[0]["accountingFingerprint"]
            );
        }
        let mut competing = proof;
        competing.id = "competing-request".into();
        competing.relay_id = "supplier-b".into();
        fs::write(
            &fixture.log,
            format!("{}\n", serde_json::to_string(&competing).unwrap()),
        )
        .unwrap();
        let revoked = fixture.snapshot();
        assert!(unknown_provider(&revoked.events[0].provider));
        let response = snapshot_response(&revoked);
        assert!(response["sessionInfo"][SESSION]["verifiedUsage"].is_null());
        assert_eq!(response["days"][0]["usage"], before_days[0]["usage"]);
        assert_eq!(
            response["days"][0]["accountingFingerprint"],
            before_days[0]["accountingFingerprint"]
        );
    }
}

#[test]
fn port_session_id_child_exports_only_its_own_or_inherited_owners_proof() {
    const CHILD: &str = "00000000-0000-4000-8000-000000000011";
    let mut fixture = Fixture::new();
    let head = json!({"type": "session_meta", "payload": {
        "session_id": CHILD, "model_provider": "custom", "forked_from_id": SESSION,
        "subagent_history_start_ordinal": 3,
        "source": {"subagent": {"thread_spawn": {"parent_thread_id": SESSION}}}
    }});
    let parent = json!({"type": "session_meta", "ordinal": 0,
        "payload": {"session_id": SESSION, "model_provider": "custom"}});
    let mut parent_context = fixture.context(500, "model-a");
    parent_context["ordinal"] = json!(1);
    let mut parent_usage = fixture.usage(1_000, 1);
    parent_usage["ordinal"] = json!(2);
    let mut child_context = fixture.context(1_500, "model-child");
    child_context["ordinal"] = json!(3);
    let mut child_usage = fixture.usage(2_000, 2);
    child_usage["ordinal"] = json!(4);
    let text = [
        head,
        parent,
        parent_context,
        parent_usage,
        child_context,
        child_usage,
    ]
    .into_iter()
    .map(|row| format!("{row}\n"))
    .collect::<String>();
    fs::write(fixture.home.join("sessions/rollout-child.jsonl"), text).unwrap();
    let before = fixture.snapshot();
    let mut parent_proof = request();
    parent_proof.timestamp_ms = fixture.start + 990;
    parent_proof.started_at_ms = fixture.start + 900;
    fs::write(
        &fixture.log,
        format!("{}\n", serde_json::to_string(&parent_proof).unwrap()),
    )
    .unwrap();
    let parent_only = fixture.snapshot();
    let response = snapshot_response(&parent_only);
    assert_eq!(
        response["sessionInfo"][SESSION]["verifiedUsage"][0]["provider"],
        "supplier-a"
    );
    assert_eq!(response["sessionInfo"][CHILD]["parentSessionId"], SESSION);
    assert!(response["sessionInfo"][CHILD]["verifiedUsage"].is_null());
    let child = parent_only
        .events
        .iter()
        .find(|event| event.session == CHILD)
        .unwrap();
    assert!(unknown_provider(&child.provider));
    assert_eq!(child.model, "model-child");

    let mut child_proof = parent_proof.clone();
    child_proof.id = "request-child".into();
    child_proof.thread_id = format!("local:{CHILD}");
    child_proof.model = "model-child".into();
    child_proof.upstream_model = "model-child".into();
    child_proof.relay_id = "supplier-b".into();
    child_proof.relay_name = "Supplier B".into();
    child_proof.timestamp_ms = fixture.start + 1_990;
    child_proof.started_at_ms = fixture.start + 1_900;
    fs::write(
        &fixture.log,
        format!(
            "{}\n{}\n",
            serde_json::to_string(&parent_proof).unwrap(),
            serde_json::to_string(&child_proof).unwrap()
        ),
    )
    .unwrap();
    let after = fixture.snapshot();
    let response = snapshot_response(&after);
    for (session, provider, role) in [
        (SESSION, "supplier-a", "main"),
        (CHILD, "supplier-b", "subagent"),
    ] {
        let event = after
            .events
            .iter()
            .find(|event| event.session == session)
            .unwrap();
        let verified = response["sessionInfo"][session]["verifiedUsage"]
            .as_array()
            .unwrap();
        assert_eq!(verified.len(), 1);
        assert_eq!(verified[0]["provider"], provider);
        assert_eq!(verified[0]["role"], role);
        assert_eq!(
            verified[0]["sourceEventId"],
            event
                .id
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
    }
    assert_eq!(
        response["days"][0]["usage"],
        snapshot_response(&before)["days"][0]["usage"]
    );
    assert_eq!(
        response["days"][0]["accountingFingerprint"],
        snapshot_response(&before)["days"][0]["accountingFingerprint"]
    );
}

fn receipt(id: &str) -> Value {
    json!({
        "id": id, "sessionId": SESSION, "timestamp": 100_010,
        "before": {"input": 0, "output": 0, "cached": 0},
        "after": {"input": 100, "output": 10, "cached": 20}
    })
}

fn match_receipts(records: &[RequestEvidence], receipts: Value) -> Value {
    live_receipts::match_receipts(records, &BTreeMap::new(), &receipts)
}

fn assert_no_attribution(result: &Value) {
    assert!(
        result
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["provider"].is_null()),
        "{result}"
    );
}

#[test]
fn duplicate_file_baselines_must_not_choose_an_arbitrary_delta() {
    let mut responses = Vec::new();
    for baseline_first in [false, true] {
        let mut fixture = Fixture::new();
        let (with_baseline, without_baseline) = if baseline_first {
            ("rollout-a.jsonl", "rollout-b.jsonl")
        } else {
            ("rollout-b.jsonl", "rollout-a.jsonl")
        };
        let mut previous = fixture.usage(0, 1);
        previous["timestamp"] = json!((Local::now() - Days::days(RETENTION_DAYS + 2)).to_rfc3339());
        previous["payload"]["info"]["total_token_usage"] = json!({
            "input_tokens": 80, "output_tokens": 5,
            "cached_input_tokens": 10, "reasoning_output_tokens": 1
        });
        let event = fixture.usage(1_000, 1);
        fixture.write_segment(with_baseline, vec![previous, event.clone()]);
        fixture.write_segment(without_baseline, vec![event]);
        let snapshot = fixture.snapshot();
        let query = Query::parse(&json!({}), Local::now().date_naive()).unwrap();
        let response = snapshot.response_page(&query, false, 1, None, None);
        eprintln!("baseline_first={baseline_first}, response={response}");
        responses.push(response);
    }
    for response in responses {
        assert_eq!(response["complete"], false);
        assert_eq!(response["scan"]["uncertainCountEvents"], 1);
        assert_eq!(response["days"], json!([]));
    }
}

fn snapshot_response(snapshot: &Snapshot) -> Value {
    let query = Query::parse(&json!({}), Local::now().date_naive()).unwrap();
    snapshot.response_page(&query, false, 1, None, None)
}

fn reopen_index(fixture: &mut Fixture) {
    fixture.index = Index {
        settings_path: Some(fixture.settings.clone()),
        storage_dir: Some(fixture._root.path().join("cache")),
        ..Index::default()
    };
}

#[test]
fn count_conflict_survives_changed_files_restart_and_missing_event() {
    let mut fixture = Fixture::new();
    let event = fixture.usage(1_000, 1);
    let mut previous = fixture.usage(0, 1);
    previous["timestamp"] = json!((Local::now() - Days::days(RETENTION_DAYS + 2)).to_rfc3339());
    previous["payload"]["info"]["total_token_usage"] = json!({
        "input_tokens": 80, "output_tokens": 5, "cached_input_tokens": 10,
        "reasoning_output_tokens": 1
    });
    fixture.write_segment("rollout-a.jsonl", vec![previous, event.clone()]);
    fixture.write_segment("rollout-b.jsonl", vec![event.clone()]);
    let first = fixture.snapshot();
    assert_eq!(snapshot_response(&first)["scan"]["uncertainCountEvents"], 1);

    let db = rusqlite::Connection::open(
        fixture
            ._root
            .path()
            .join("cache/usage-provider-cache.sqlite"),
    )
    .unwrap();
    let data: String = db
        .query_row("SELECT data FROM native_attributions", [], |row| row.get(0))
        .unwrap();
    let stored: Value = serde_json::from_str(&data).unwrap();
    assert_eq!(stored["version"], 3);
    assert_eq!(stored["count_conflict"], 2);
    assert!(!data.contains(SESSION));
    assert_eq!(stored["key"]["session"], "");
    assert_eq!(stored["key"]["model"], "");
    assert_eq!(stored["key"]["input"], 0);
    assert_eq!(stored["provider"], "");
    drop(db);

    let later = fixture.usage(2_000, 2);
    for name in ["rollout-a.jsonl", "rollout-b.jsonl"] {
        fixture.write_segment(name, vec![event.clone(), later.clone()]);
    }
    let repaired = fixture.snapshot();
    let response = snapshot_response(&repaired);
    assert_eq!(response["complete"], false);
    assert_eq!(response["scan"]["uncertainCountEvents"], 1);
    assert_eq!(response["days"][0]["usage"]["totalTokens"], 110);
    assert!(repaired.events[0].count_uncertain);
    assert!(!repaired.events[1].count_uncertain);

    reopen_index(&mut fixture);
    let restarted = fixture.snapshot();
    assert_eq!(
        snapshot_response(&restarted)["scan"]["uncertainCountEvents"],
        1
    );
    assert_eq!(
        snapshot_response(&restarted)["days"][0]["usage"]["totalTokens"],
        110
    );

    for name in ["rollout-a.jsonl", "rollout-b.jsonl"] {
        fixture.write_segment(name, vec![]);
    }
    let absent = fixture.snapshot();
    let response = snapshot_response(&absent);
    assert_eq!(response["complete"], false);
    assert_eq!(response["scan"]["uncertainCountEvents"], 1);
    assert_eq!(response["scan"]["unobservedCountConflicts"], 1);
}

#[test]
fn independent_stream_does_not_change_an_agreed_duplicate_delta() {
    for baseline_first in [false, true] {
        let mut fixture = Fixture::new();
        let mut event = fixture.usage(1_000, 1);
        event["payload"]["info"]["last_token_usage"] = json!({
            "input_tokens": 20, "output_tokens": 2, "cached_input_tokens": 0,
            "reasoning_output_tokens": 0
        });
        let mut old = fixture.usage(0, 1);
        old["timestamp"] = json!((Local::now() - Days::days(RETENTION_DAYS + 2)).to_rfc3339());
        old["payload"]["info"]["total_token_usage"] = json!({
            "input_tokens": 80, "output_tokens": 8, "cached_input_tokens": 20,
            "reasoning_output_tokens": 2
        });
        let mut in_window = fixture.usage(500, 1);
        in_window["payload"]["info"]["total_token_usage"] = json!({
            "input_tokens": 10, "output_tokens": 1, "cached_input_tokens": 0,
            "reasoning_output_tokens": 0
        });
        in_window["payload"]["info"]["last_token_usage"] =
            in_window["payload"]["info"]["total_token_usage"].clone();
        let (with_baseline, without_baseline) = if baseline_first {
            ("rollout-a.jsonl", "rollout-b.jsonl")
        } else {
            ("rollout-b.jsonl", "rollout-a.jsonl")
        };
        fixture.write_segment(with_baseline, vec![old, event.clone()]);
        fixture.write_segment(without_baseline, vec![event]);
        fixture.write_segment("rollout-c.jsonl", vec![in_window]);
        let snapshot = fixture.snapshot();
        let response = snapshot_response(&snapshot);
        assert_eq!(response["complete"], true);
        assert_eq!(response["scan"]["uncertainCountEvents"], 0);
        assert_eq!(response["days"][0]["usage"]["totalTokens"], 33);
    }
}

#[test]
fn shared_session_baseline_can_make_different_file_deltas_agree() {
    for baseline_first in [false, true] {
        let mut fixture = Fixture::new();
        let event = fixture.usage(1_000, 1);
        let mut previous = fixture.usage(500, 1);
        previous["payload"]["info"]["total_token_usage"] = json!({
            "input_tokens": 80, "output_tokens": 5, "cached_input_tokens": 10,
            "reasoning_output_tokens": 1
        });
        previous["payload"]["info"]["last_token_usage"] =
            previous["payload"]["info"]["total_token_usage"].clone();
        let (with_baseline, without_baseline) = if baseline_first {
            ("rollout-a.jsonl", "rollout-b.jsonl")
        } else {
            ("rollout-b.jsonl", "rollout-a.jsonl")
        };
        fixture.write_segment(with_baseline, vec![previous, event.clone()]);
        fixture.write_segment(without_baseline, vec![event]);
        let response = snapshot_response(&fixture.snapshot());
        assert_eq!(response["complete"], true);
        assert_eq!(response["scan"]["uncertainCountEvents"], 0);
        assert_eq!(response["days"][0]["usage"]["totalTokens"], 110);
    }
}

#[test]
fn zero_or_missing_reasoning_delta_cannot_hide_a_duplicate_conflict() {
    for zero_delta in [false, true] {
        for baseline_first in [false, true] {
            let mut fixture = Fixture::new();
            let event = fixture.usage(1_000, 1);
            let mut previous = fixture.usage(0, 1);
            previous["timestamp"] =
                json!((Local::now() - Days::days(RETENTION_DAYS + 2)).to_rfc3339());
            previous["payload"]["info"]["total_token_usage"] = if zero_delta {
                event["payload"]["info"]["total_token_usage"].clone()
            } else {
                json!({"input_tokens": 0, "output_tokens": 0, "cached_input_tokens": 0})
            };
            let (with_baseline, without_baseline) = if baseline_first {
                ("rollout-a.jsonl", "rollout-b.jsonl")
            } else {
                ("rollout-b.jsonl", "rollout-a.jsonl")
            };
            fixture.write_segment(with_baseline, vec![previous, event.clone()]);
            fixture.write_segment(without_baseline, vec![event]);
            let snapshot = fixture.snapshot();
            let response = snapshot_response(&snapshot);
            assert_eq!(snapshot.events.len(), 1);
            assert_eq!(response["complete"], false);
            assert_eq!(response["scan"]["uncertainCountEvents"], 1);
            assert_eq!(response["days"], json!([]));
        }
    }
}

#[test]
fn count_cache_write_failure_fails_closed_and_retries() {
    let mut fixture = Fixture::new();
    let event = fixture.usage(1_000, 1);
    fixture.write_segment("rollout-a.jsonl", vec![event.clone()]);
    fixture.snapshot();
    let db = rusqlite::Connection::open(
        fixture
            ._root
            .path()
            .join("cache/usage-provider-cache.sqlite"),
    )
    .unwrap();
    db.execute_batch("BEGIN IMMEDIATE").unwrap();
    let mut previous = fixture.usage(0, 1);
    previous["timestamp"] = json!((Local::now() - Days::days(RETENTION_DAYS + 2)).to_rfc3339());
    previous["payload"]["info"]["total_token_usage"] = json!({
        "input_tokens": 80, "output_tokens": 5, "cached_input_tokens": 10,
        "reasoning_output_tokens": 1
    });
    fixture.write_segment("rollout-b.jsonl", vec![previous, event]);
    let failed = fixture.snapshot();
    let response = snapshot_response(&failed);
    assert_eq!(response["complete"], false);
    assert_eq!(response["scan"]["countCacheAvailable"], false);
    assert_eq!(response["days"], json!([]));
    db.execute_batch("ROLLBACK").unwrap();
    drop(db);
    let retried = fixture.snapshot();
    assert_eq!(
        snapshot_response(&retried)["scan"]["countCacheAvailable"],
        true
    );
    reopen_index(&mut fixture);
    assert_eq!(
        snapshot_response(&fixture.snapshot())["scan"]["uncertainCountEvents"],
        1
    );
}

#[test]
fn conflict_survives_first_cache_open_failure_and_retry() {
    let mut fixture = Fixture::new();
    let cache = fixture._root.path().join("cache");
    fs::write(&cache, "not a directory").unwrap();
    let event = fixture.usage(1_000, 1);
    let mut previous = fixture.usage(0, 1);
    previous["timestamp"] = json!((Local::now() - Days::days(RETENTION_DAYS + 2)).to_rfc3339());
    previous["payload"]["info"]["total_token_usage"] = json!({
        "input_tokens": 80, "output_tokens": 5, "cached_input_tokens": 10,
        "reasoning_output_tokens": 1
    });
    fixture.write_segment("rollout-a.jsonl", vec![previous, event.clone()]);
    fixture.write_segment("rollout-b.jsonl", vec![event.clone()]);
    let failed = fixture.snapshot();
    assert_eq!(
        snapshot_response(&failed)["scan"]["countCacheAvailable"],
        false
    );
    assert!(failed.events[0].count_uncertain);

    fs::remove_file(&cache).unwrap();
    fixture.write_segment("rollout-a.jsonl", vec![event.clone()]);
    fixture.write_segment("rollout-b.jsonl", vec![event]);
    let retried = fixture.snapshot();
    assert_eq!(snapshot_response(&retried)["complete"], false);
    assert_eq!(
        snapshot_response(&retried)["scan"]["uncertainCountEvents"],
        1
    );
    reopen_index(&mut fixture);
    assert_eq!(
        snapshot_response(&fixture.snapshot())["scan"]["uncertainCountEvents"],
        1
    );
}

#[test]
fn unsupported_cache_version_is_not_modified() {
    let mut fixture = Fixture::new();
    let cache = fixture._root.path().join("cache");
    fs::create_dir_all(&cache).unwrap();
    let db = rusqlite::Connection::open(cache.join("usage-provider-cache.sqlite")).unwrap();
    db.execute_batch("PRAGMA user_version=99").unwrap();
    drop(db);
    fixture.write_segment("rollout-a.jsonl", vec![fixture.usage(1_000, 1)]);
    let response = snapshot_response(&fixture.snapshot());
    assert_eq!(response["scan"]["countCacheAvailable"], false);
    assert_eq!(response["days"], json!([]));
    let db = rusqlite::Connection::open(cache.join("usage-provider-cache.sqlite")).unwrap();
    let tables: usize = db
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(tables, 0);
    let version: i64 = db
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 99);
}

#[test]
fn native_cache_capacity_keeps_unpersisted_conflicts_closed_after_restart() {
    let mut fixture = Fixture::new();
    fixture.snapshot();
    let db = rusqlite::Connection::open(
        fixture
            ._root
            .path()
            .join("cache/usage-provider-cache.sqlite"),
    )
    .unwrap();
    let at = Local::now().timestamp_millis();
    let invalid = json!({"key":{"session":"","model":"","input":0,"output":0,"cached":0},
        "at":at,"turn":"","provider":""})
    .to_string();
    db.execute(
        "WITH RECURSIVE n(value) AS (VALUES(1) UNION ALL SELECT value+1 FROM n WHERE value<?1)
         INSERT INTO native_attributions(id,at,data) SELECT CAST(printf('%032d',value) AS BLOB),?2,?3 FROM n",
        rusqlite::params![(MAX_EVENTS - 1) as i64, at, invalid],
    ).unwrap();
    drop(db);

    let event = fixture.usage(1_000, 1);
    let mut previous = fixture.usage(0, 1);
    previous["timestamp"] = json!((Local::now() - Days::days(RETENTION_DAYS + 2)).to_rfc3339());
    previous["payload"]["info"]["total_token_usage"] = json!({
        "input_tokens": 80, "output_tokens": 5, "cached_input_tokens": 10,
        "reasoning_output_tokens": 1
    });
    for (prefix, session) in [
        ("one", SESSION),
        ("two", "00000000-0000-4000-8000-000000000012"),
    ] {
        let head = json!({"type":"session_meta","timestamp":fixture.at(0),
            "payload":{"id":session,"model_provider":"custom"}});
        for (suffix, with_baseline) in [("a", true), ("b", false)] {
            let mut rows = vec![head.clone()];
            if with_baseline {
                rows.push(previous.clone());
            }
            rows.push(event.clone());
            let text = rows
                .into_iter()
                .map(|row| format!("{row}\n"))
                .collect::<String>();
            fs::write(
                fixture
                    .home
                    .join("sessions")
                    .join(format!("rollout-{prefix}-{suffix}.jsonl")),
                text,
            )
            .unwrap();
        }
    }
    let first = fixture.snapshot();
    assert_eq!(
        snapshot_response(&first)["scan"]["countCacheAvailable"],
        false
    );
    let db = rusqlite::Connection::open(
        fixture
            ._root
            .path()
            .join("cache/usage-provider-cache.sqlite"),
    )
    .unwrap();
    let count: usize = db
        .query_row("SELECT count(*) FROM native_attributions", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert!(
        count <= MAX_EVENTS,
        "native attribution rows exceed budget: {count}"
    );
    let guard_at: i64 = db
        .query_row(
            "SELECT at FROM structured_count_guard WHERE singleton=1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(guard_at >= at);
    db.execute(
        "DELETE FROM native_attributions WHERE id=CAST(printf('%032d',1) AS BLOB)",
        [],
    )
    .unwrap();
    drop(db);

    for name in ["rollout-one-a.jsonl", "rollout-two-a.jsonl"] {
        fs::write(fixture.home.join("sessions").join(name), "").unwrap();
    }
    reopen_index(&mut fixture);
    let restarted = fixture.snapshot();
    assert_eq!(snapshot_response(&restarted)["complete"], false);
    assert_eq!(
        snapshot_response(&restarted)["scan"]["countCacheAvailable"],
        false
    );
    assert_eq!(snapshot_response(&restarted)["days"], json!([]));
}

#[test]
fn original_replaces_inherited_count_conflict_only_after_complete_scan() {
    use std::io::Write;

    let mut fixture = Fixture::new();
    let child = json!({"type": "session_meta", "payload": {
        "id": "00000000-0000-4000-8000-000000000011", "forked_from_id": SESSION,
        "subagent_history_start_ordinal": 4, "source": {"subagent": {}}
    }});
    let parent = json!({"type": "session_meta", "ordinal": 0,
        "payload": {"id": SESSION, "model_provider": "supplier-a"}});
    let mut context = fixture.context(500, "model-a");
    context["ordinal"] = json!(1);
    context["payload"]["model_provider"] = json!("supplier-a");
    let mut previous = fixture.usage(0, 1);
    previous["ordinal"] = json!(2);
    previous["timestamp"] = json!((Local::now() - Days::days(RETENTION_DAYS + 2)).to_rfc3339());
    previous["payload"]["info"]["total_token_usage"] = json!({
        "input_tokens": 80, "output_tokens": 5, "cached_input_tokens": 10,
        "reasoning_output_tokens": 1
    });
    let mut event = fixture.usage(1_000, 1);
    event["ordinal"] = json!(3);
    for (name, with_baseline) in [("rollout-a.jsonl", true), ("rollout-b.jsonl", false)] {
        let mut rows = vec![child.clone(), parent.clone(), context.clone()];
        if with_baseline {
            rows.push(previous.clone());
        }
        rows.push(event.clone());
        let text = rows
            .into_iter()
            .map(|row| format!("{row}\n"))
            .collect::<String>();
        fs::write(fixture.home.join("sessions").join(name), text).unwrap();
    }
    let inherited = fixture.snapshot();
    assert!(inherited.events[0].inherited && inherited.events[0].count_uncertain);
    assert_eq!(
        snapshot_response(&inherited)["scan"]["uncertainCountEvents"],
        1
    );

    fixture.write_classified_segment("rollout-c.jsonl", "model-a", "supplier-a", "main");
    let pending = fixture.home.join("sessions/rollout-pending.jsonl");
    fs::write(&pending, "{").unwrap();
    let partial = fixture
        .index
        .update_with_request_log(&fixture.home, &fixture.log);
    assert!(partial.errors > 0);
    assert!(!partial.events[0].inherited && partial.events[0].count_uncertain);
    assert_eq!(snapshot_response(&partial)["complete"], false);

    let mut file = fs::OpenOptions::new().append(true).open(pending).unwrap();
    writeln!(file, "\"type\":\"noop\",\"payload\":{{}}}}").unwrap();
    drop(file);
    let complete = fixture.snapshot();
    assert!(!complete.events[0].inherited && !complete.events[0].count_uncertain);
    assert_accounting(&complete, 1);
    assert_eq!(snapshot_response(&complete)["complete"], true);
    reopen_index(&mut fixture);
    let restarted = fixture.snapshot();
    assert_accounting(&restarted, 1);
    assert_eq!(
        snapshot_response(&restarted)["scan"]["uncertainCountEvents"],
        0
    );
}

#[test]
fn future_context_does_not_relabel_existing_usage() {
    use std::io::Write;

    let mut fixture = Fixture::new();
    fixture.write_segment("rollout-sequence.jsonl", vec![fixture.usage(1_000, 1)]);
    let before = fixture.snapshot();
    assert_models(&before, &[""]);

    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(fixture.home.join("sessions/rollout-sequence.jsonl"))
        .unwrap();
    writeln!(file, "{}", fixture.context(2_000, "model-b")).unwrap();
    writeln!(file, "{}", fixture.usage(3_000, 2)).unwrap();
    drop(file);

    let after = fixture.snapshot();
    assert_models(&after, &["", "model-b"]);
    assert!(before.events[0] == after.events[0]);
    assert_accounting(&after, 2);
}

#[test]
fn exact_evidence_fills_only_its_event_without_changing_usage() {
    let mut fixture = Fixture::new();
    fixture.write_segment(
        "rollout-sequence.jsonl",
        vec![
            fixture.usage(1_000, 1),
            fixture.usage(2_000, 2),
            fixture.context(3_000, "model-b"),
            fixture.usage(4_000, 3),
        ],
    );
    let before = fixture.snapshot();
    fixture.write_evidence(vec![evidence(&before.events[0], "model-a")]);
    let after = fixture.snapshot();
    assert_models(&after, &["model-a", "", "model-b"]);
    assert_eq!(after.events[0].provider, "supplier-a");
    assert_eq!(after.provider_evidence["appliedEvents"], 1);
    assert_eq!(before.cursors, after.cursors);
    for (old, new) in before.events.iter().zip(&after.events) {
        assert_eq!(old.id, new.id);
        assert_eq!(old.delta, new.delta);
        assert_eq!(old.total, new.total);
    }
    assert_accounting(&before, 3);
    assert_accounting(&after, 3);
}

#[test]
fn conflicting_or_duplicate_evidence_is_rejected() {
    for case in [
        "input",
        "timestamp",
        "session",
        "source",
        "model",
        "provider",
        "duplicate",
    ] {
        let mut fixture = Fixture::new();
        let mut rows = Vec::new();
        if case == "model" {
            rows.push(fixture.context(500, "model-b"));
        }
        if case == "provider" {
            let mut context = fixture.context(500, "");
            context["payload"]["model_provider"] = json!("supplier-b");
            rows.push(context);
        }
        rows.push(fixture.usage(1_000, 1));
        fixture.write_segment("rollout-sequence.jsonl", rows);
        let before = fixture.snapshot();
        let mut record = evidence(&before.events[0], "model-a");
        match case {
            "input" => record["input"] = json!(101),
            "timestamp" => record["timestamp"] = json!(fixture.at(2_000)),
            "session" => record["session"] = json!("00000000-0000-4000-8000-000000000011"),
            "source" => record["source"] = json!("user-classified"),
            _ => {}
        }
        let records = if case == "duplicate" {
            vec![record.clone(), record]
        } else {
            vec![record]
        };
        fixture.write_evidence(records);
        let after = fixture.snapshot();
        assert_models(&after, if case == "model" { &["model-b"] } else { &[""] });
        assert_eq!(after.provider_evidence["appliedEvents"], 0, "case={case}");
        assert_eq!(
            after.provider_evidence["invalid"],
            usize::from(case == "duplicate")
        );
        assert_eq!(
            after.provider_evidence["signatureMismatches"],
            usize::from(matches!(
                case,
                "input" | "timestamp" | "session" | "source" | "model"
            ))
        );
        assert_eq!(
            after.provider_evidence["conflictedEvents"],
            usize::from(case == "provider")
        );
        if case == "provider" {
            assert_eq!(after.events[0].provider, "supplier-b");
        }
        assert_eq!(before.events[0].delta, after.events[0].delta);
        assert_accounting(&after, 1);
    }
}

#[test]
fn duplicate_rollout_events_are_counted_once() {
    let mut fixture = Fixture::new();
    let rows = vec![fixture.usage(1_000, 1)];
    fixture.write_segment("rollout-a.jsonl", rows.clone());
    fixture.write_segment("rollout-b.jsonl", rows);
    let before = fixture.snapshot();
    assert_accounting(&before, 1);
    fixture.write_evidence(vec![evidence(&before.events[0], "model-a")]);
    let after = fixture.snapshot();
    assert_models(&after, &["model-a"]);
    assert_accounting(&after, 1);
}

#[test]
fn conflicting_duplicate_context_is_order_independent_and_cannot_be_supplemented() {
    for (a, b, expected_model, expected_provider, expected_role, provider_conflict) in [
        (
            ("model-a", "supplier-a", "main"),
            ("model-b", "supplier-a", "main"),
            "",
            "supplier-a",
            "main",
            false,
        ),
        (
            ("model-a", "supplier-a", "main"),
            ("model-a", "supplier-b", "main"),
            "model-a",
            "",
            "main",
            true,
        ),
        (
            ("model-a", "supplier-a", "main"),
            ("model-a", "supplier-a", "subagent"),
            "model-a",
            "supplier-a",
            "unknown",
            false,
        ),
    ] {
        for reverse in [false, true] {
            let mut fixture = Fixture::new();
            let (first, second) = if reverse { (b, a) } else { (a, b) };
            fixture.write_classified_segment("rollout-a.jsonl", first.0, first.1, first.2);
            fixture.write_classified_segment("rollout-b.jsonl", second.0, second.1, second.2);
            // A third matching copy must not erase a previously observed conflict.
            fixture.write_classified_segment("rollout-c.jsonl", first.0, first.1, first.2);
            let before = fixture.snapshot();
            assert_accounting(&before, 1);
            assert!(before.events[0].context_conflicts.any());
            assert_eq!(before.events[0].model, expected_model);
            assert_eq!(before.events[0].provider, expected_provider);
            let query = Query::parse(&json!({}), Local::now().date_naive()).unwrap();
            let response = before.response_page(&query, false, 1, None, None);
            assert_eq!(response["scan"]["contextConflicts"], 1);
            let row = &response["days"][0]["breakdowns"][0];
            assert_eq!(row["role"], expected_role);
            assert_eq!(row["providerConflict"] == true, provider_conflict);
            assert!(response["sessionInfo"][SESSION]["verifiedUsage"].is_null());

            let mut proof = evidence(&before.events[0], "model-a");
            proof["role"] = json!("main");
            fixture.write_evidence(vec![proof]);
            let after = fixture.snapshot();
            assert_eq!(after.provider_evidence["appliedEvents"], 0);
            assert_eq!(after.provider_evidence["conflictedEvents"], 1);
            assert_eq!(after.events[0].model, expected_model);
            assert_eq!(after.events[0].provider, expected_provider);
            assert_accounting(&after, 1);
        }
    }
}

#[test]
fn compatible_duplicate_context_preserves_known_fields_in_both_orders() {
    for reverse in [false, true] {
        let mut fixture = Fixture::new();
        let unknown = ("", "custom", "main");
        let known = ("model-a", "supplier-b", "main");
        let (first, second) = if reverse {
            (known, unknown)
        } else {
            (unknown, known)
        };
        fixture.write_classified_segment("rollout-a.jsonl", first.0, first.1, first.2);
        fixture.write_classified_segment("rollout-b.jsonl", second.0, second.1, second.2);
        let before = fixture.snapshot();
        assert!(!before.events[0].context_conflicts.any());
        assert_eq!(before.events[0].model, "model-a");
        assert_eq!(before.events[0].provider, "supplier-b");
        fixture.write_evidence(vec![evidence(&before.events[0], "model-a")]);
        let after = fixture.snapshot();
        assert_eq!(after.events[0].provider, "supplier-b");
        assert_eq!(after.provider_evidence["conflictedEvents"], 1);
        assert_accounting(&after, 1);
    }
}

#[test]
fn original_context_wins_over_conflicting_inherited_copy_in_both_orders() {
    for inherited_first in [false, true] {
        let mut fixture = Fixture::new();
        let (original, inherited) = if inherited_first {
            ("rollout-b.jsonl", "rollout-a.jsonl")
        } else {
            ("rollout-a.jsonl", "rollout-b.jsonl")
        };
        fixture.write_classified_segment(original, "model-a", "supplier-a", "main");
        fixture.write_inherited_segment(inherited, "model-b", "supplier-b");
        let snapshot = fixture.snapshot();
        assert_accounting(&snapshot, 1);
        assert!(!snapshot.events[0].inherited);
        assert!(!snapshot.events[0].context_conflicts.any());
        assert_eq!(snapshot.events[0].model, "model-a");
        assert_eq!(snapshot.events[0].provider, "supplier-a");
    }
}

#[test]
fn native_conflict_rejection_persists_across_partial_scan_and_index_restart() {
    use std::io::Write;

    let mut fixture = Fixture::new();
    let mut context = fixture.context(500, "model-a");
    context["payload"]["turn_id"] = json!("turn-a");
    let event = fixture.usage(1_000, 1);
    fixture.write_segment("rollout-a.jsonl", vec![context.clone(), event.clone()]);
    let initial = fixture.snapshot();
    let original = &initial.events[0];
    let at = DateTime::parse_from_rfc3339(&original.timestamp)
        .unwrap()
        .timestamp_millis();
    let saved: native_endpoints::Attribution = serde_json::from_value(json!({
        "key": {"session": SESSION, "model": "model-a", "input": 100, "output": 10, "cached": 20},
        "at": at, "turn": "turn-a", "provider": "supplier-a", "version": 2,
        "binding_since": at - 1, "rejected": false
    }))
    .unwrap();
    fixture
        .index
        .native_evidence
        .verified
        .insert(original.id, saved);
    fixture
        .index
        .provider_cache
        .save_native(&fixture.index.native_evidence.verified)
        .unwrap();

    for (name, provider) in [
        ("rollout-b.jsonl", "supplier-a"),
        ("rollout-c.jsonl", "supplier-b"),
    ] {
        let mut candidate = context.clone();
        candidate["payload"]["model_provider"] = json!(provider);
        fixture.write_segment(name, vec![candidate, event.clone()]);
    }
    let conflicted = fixture.snapshot();
    assert!(conflicted.events[0].context_conflicts.provider);
    assert_eq!(
        serde_json::to_value(&fixture.index.native_evidence.verified[&original.id]).unwrap()["rejected"],
        true
    );

    fixture.write_segment("rollout-b.jsonl", vec![]);
    let pending = fixture.home.join("sessions/rollout-c.jsonl");
    fs::write(&pending, "{").unwrap();
    reopen_index(&mut fixture);
    let partial = fixture
        .index
        .update_with_request_log(&fixture.home, &fixture.log);
    assert!(partial.errors > 0);
    assert!(!partial.events[0].context_conflicts.any());
    assert!(unknown_provider(&partial.events[0].provider));
    assert_eq!(
        serde_json::to_value(&fixture.index.native_evidence.verified[&original.id]).unwrap()["rejected"],
        true
    );

    let mut file = fs::OpenOptions::new().append(true).open(pending).unwrap();
    writeln!(file, "\"type\":\"noop\",\"payload\":{{}}}}").unwrap();
    drop(file);
    let complete = fixture.snapshot();
    assert!(unknown_provider(&complete.events[0].provider));
}

#[test]
fn damaged_history_preserves_native_proofs_without_unsafe_reclassification() {
    const OTHER: &str = "00000000-0000-4000-8000-000000000011";
    for kind in [
        "complete",
        "unrelated",
        "same",
        "unidentified",
        "pending",
        "identity-conflict",
        "inherited",
    ] {
        let mut fixture = Fixture::new();
        let mut context = fixture.context(500, "model-a");
        context["payload"]["turn_id"] = json!("turn-a");
        fixture.write_segment("rollout-a.jsonl", vec![context, fixture.usage(1_000, 1)]);
        let initial = fixture.snapshot();
        let event = &initial.events[0];
        let at = DateTime::parse_from_rfc3339(&event.timestamp)
            .unwrap()
            .timestamp_millis();
        let saved = serde_json::from_value(json!({
            "key": {"session": SESSION, "model": "model-a", "input": 100, "output": 10, "cached": 20},
            "at": at, "turn": "turn-a", "provider": "supplier-a", "version": 2,
            "binding_since": at - 1, "rejected": false
        })).unwrap();
        fixture
            .index
            .native_evidence
            .verified
            .insert(event.id, saved);
        fixture
            .index
            .provider_cache
            .save_native(&fixture.index.native_evidence.verified)
            .unwrap();

        let header = |id| json!({"type": "session_meta", "payload": {"id": id}});
        let damaged = match kind {
            "complete" => format!("{}\n", header(OTHER)),
            "unrelated" => format!("{}\n\0\0\n", header(OTHER)),
            "same" => format!("{}\n\0\0\n", header(SESSION)),
            "unidentified" => "\0\0\n".into(),
            "pending" => "{".into(),
            "identity-conflict" => format!("{}\n{}\n\0\0\n", header(OTHER), header(SESSION)),
            "inherited" => format!(
                "{}\n{}\n\0\0\n",
                json!({"type": "session_meta", "payload": {
                    "id": OTHER, "forked_from_id": SESSION, "subagent_history_start_ordinal": 4}}),
                json!({"type": "session_meta", "ordinal": 0, "payload": {"id": SESSION}})
            ),
            _ => unreachable!(),
        };
        fs::write(fixture.home.join("sessions/rollout-damaged.jsonl"), damaged).unwrap();
        for restart in [false, true] {
            if restart {
                reopen_index(&mut fixture);
            }
            let snapshot = fixture
                .index
                .update_with_request_log(&fixture.home, &fixture.log);
            assert_eq!(
                snapshot_response(&snapshot)["complete"],
                kind == "complete",
                "{kind}"
            );
            assert_eq!(snapshot.events.len(), 1, "{kind}");
            assert_eq!(snapshot.events[0].delta, event.delta, "{kind}");
            assert_eq!(snapshot.events[0].id, event.id, "{kind}");
            if kind == "complete" {
                assert_eq!(
                    snapshot.events[0].provider, "supplier-a",
                    "restart={restart}"
                );
            } else {
                assert!(
                    unknown_provider(&snapshot.events[0].provider),
                    "{kind}, restart={restart}"
                );
            }
            assert_eq!(
                serde_json::to_value(&fixture.index.native_evidence.verified[&event.id]).unwrap()["rejected"],
                false,
                "{kind}"
            );
        }
    }
}

#[test]
fn conflicted_duplicate_blocks_request_log_attribution() {
    let mut fixture = Fixture::new();
    fixture.write_classified_segment("rollout-a.jsonl", "model-a", "supplier-a", "main");
    fixture.write_classified_segment("rollout-b.jsonl", "model-a", "supplier-b", "main");
    let snapshot = fixture.snapshot();
    let event = snapshot.events[0].clone();
    let at = DateTime::parse_from_rfc3339(&event.timestamp)
        .unwrap()
        .timestamp_millis();
    let mut proof = request();
    proof.started_at_ms = at - 100;
    proof.timestamp_ms = at - 10;

    for conflicted in [false, true] {
        let mut test_event = event.clone();
        test_event.context_conflicts.provider = conflicted;
        let mut cache = RequestEvidenceCache::default();
        cache.records.push(proof.clone());
        let mut events = vec![test_event];
        let result = cache.attribute(&mut events, &mut BTreeMap::new(), &BTreeMap::new());
        assert_eq!(
            events[0].provider,
            if conflicted { "" } else { "supplier-a" }
        );
        assert_eq!(result["recoveredEvents"], usize::from(!conflicted));
    }
}

#[test]
fn uncertain_counts_do_not_make_another_event_match_a_request_uniquely() {
    let mut fixture = Fixture::new();
    fixture.write_classified_segment("rollout-a.jsonl", "model-a", "custom", "main");
    let mut event = fixture.snapshot().events.remove(0);
    let at = DateTime::parse_from_rfc3339(&event.timestamp)
        .unwrap()
        .timestamp_millis();
    let mut proof = request();
    proof.started_at_ms = at - 100;
    proof.timestamp_ms = at - 10;
    let mut uncertain = event.clone();
    uncertain.id = [33; 32];
    uncertain.count_uncertain = true;
    uncertain.delta = Tokens::new(20, 0, 2);
    event.provider.clear();
    let mut events = vec![event, uncertain];
    let mut cache = RequestEvidenceCache::default();
    cache.records.push(proof);
    let result = cache.attribute(&mut events, &mut BTreeMap::new(), &BTreeMap::new());
    assert_eq!(result["recoveredEvents"], 0);
    assert!(unknown_provider(&events[0].provider));
}

#[test]
fn repeated_requests_and_competing_receipts_cannot_reuse_proof() {
    let record = request();
    let row = receipt("receipt-1");
    let positive = match_receipts(&[record.clone()], json!([row.clone()]));
    assert_eq!(positive[0]["provider"], "supplier-a");
    assert_eq!(positive[0]["model"], "model-a");
    assert_eq!(positive[0]["requestIds"], json!(["request-1"]));

    let mut saved = row.clone();
    for key in ["model", "provider", "requestIds", "requestSignatures"] {
        saved[key] = positive[0][key].clone();
    }
    let duplicate = match_receipts(&[record.clone(), record.clone()], json!([row.clone()]));
    assert_no_attribution(&duplicate);
    let mut conflicting = record.clone();
    conflicting.relay_id = "supplier-b".into();
    assert_no_attribution(&match_receipts(
        &[record.clone(), conflicting],
        json!([row.clone()]),
    ));
    let mut same_counts = record.clone();
    same_counts.id = "request-2".into();
    assert_no_attribution(&match_receipts(
        &[record.clone(), same_counts],
        json!([row.clone()]),
    ));
    let revoked = match_receipts(&[record.clone(), record.clone()], json!([saved]));
    assert_eq!(revoked[0]["rejected"], true);

    let competing = match_receipts(
        &[record.clone()],
        json!([row.clone(), receipt("receipt-2")]),
    );
    assert_no_attribution(&competing);
    assert_eq!(competing.as_array().unwrap().len(), 2);
    assert!(
        competing
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["rejected"] == true)
    );

    let repeated = match_receipts(&[record], json!([row.clone(), row]));
    assert_no_attribution(&repeated);
    assert_eq!(repeated.as_array().unwrap().len(), 2);
    assert!(
        repeated
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["rejected"] == true)
    );
}

#[test]
fn routed_live_receipt_uses_the_successful_upstream_model() {
    let mut record = request();
    record.model = "group/pool".into();
    record.upstream_model = "model-b".into();
    record.relay_id = "supplier-b".into();
    record.relay_name = "Supplier B".into();
    let mut row = receipt("routed");
    row["model"] = json!("auto");
    let result = match_receipts(&[record.clone()], json!([row.clone()]));
    assert_eq!(result[0]["provider"], "supplier-b");
    assert_eq!(result[0]["model"], "model-b");
    assert_eq!(result[0]["input"], 100);
    assert_eq!(result[0]["output"], 10);

    for key in ["model", "provider", "requestIds", "requestSignatures"] {
        row[key] = result[0][key].clone();
    }
    row["model"] = json!("supplier-b/model-b");
    let validated = match_receipts(&[record], json!([row]));
    assert_eq!(validated[0]["provider"], "supplier-b");
    assert_ne!(validated[0]["rejected"], true);
}

#[test]
fn fallback_live_receipt_requires_the_same_original_request() {
    let mut record = request();
    record.model = "supplier-a/model-a".into();
    record.relay_id = "supplier-b".into();
    record.relay_name = "Supplier B".into();
    let mut row = receipt("fallback");
    row["model"] = json!("supplier-a/model-a");

    let proof = match_receipts(&[record.clone()], json!([row.clone()]));
    assert_eq!(proof[0]["provider"], "supplier-b");
    assert_eq!(proof[0]["model"], "model-a");
    assert_eq!(proof[0]["requestIds"], json!(["request-1"]));
    assert_eq!(proof[0]["input"], 100);
    assert_eq!(proof[0]["output"], 10);
    assert_eq!(proof[0]["cached"], 20);

    let mut retried = record.clone();
    retried.attempts = Some(2);
    retried.final_response_evidence = Some("final-response-v1".into());
    assert_eq!(
        match_receipts(&[retried.clone()], json!([row.clone()]))[0]["provider"],
        "supplier-b"
    );
    retried.final_response_evidence = None;
    assert_no_attribution(&match_receipts(&[retried], json!([row.clone()])));

    for original in ["", "supplier-c/model-a"] {
        let mut changed = record.clone();
        changed.model = original.into();
        let result = match_receipts(&[changed], json!([row.clone()]));
        assert_no_attribution(&result);
        assert_eq!(result[0]["rejected"], true, "original={original}");
    }
    let mut conflicting_provider = row.clone();
    conflicting_provider["provider"] = json!("supplier-a");
    let result = match_receipts(&[record.clone()], json!([conflicting_provider]));
    assert_no_attribution(&result);
    assert_eq!(result[0]["rejected"], true);

    for key in ["provider", "requestIds", "requestSignatures"] {
        row[key] = proof[0][key].clone();
    }
    assert_eq!(
        match_receipts(&[record.clone()], json!([row.clone()]))[0]["provider"],
        "supplier-b"
    );
    assert_eq!(match_receipts(&[], json!([row.clone()])), json!([]));
    let mut changed = record.clone();
    changed.model = "supplier-c/model-a".into();
    assert_eq!(
        match_receipts(&[changed], json!([row.clone()]))[0]["rejected"],
        true
    );
    let mut invalid_signature = row.clone();
    invalid_signature["requestSignatures"]["request-1"] = json!("b".repeat(64));
    assert_eq!(
        match_receipts(&[record.clone()], json!([invalid_signature]))[0]["rejected"],
        true
    );

    let mut competing = record.clone();
    competing.id = "request-2".into();
    assert_no_attribution(&match_receipts(&[record, competing], json!([row])));
}

#[test]
fn fallback_live_receipt_preserves_namespaced_upstream_model() {
    let mut record = request();
    record.model = "supplier-a/namespace/model-a".into();
    record.upstream_model = "namespace/model-a".into();
    record.relay_id = "supplier-b".into();
    record.relay_name = "Supplier B".into();
    let mut row = receipt("namespaced-fallback");
    row["model"] = json!("supplier-a/namespace/model-a");

    let proof = match_receipts(&[record.clone()], json!([row.clone()]));
    assert_eq!(proof[0]["provider"], "supplier-b");
    assert_eq!(proof[0]["model"], "namespace/model-a");

    record.upstream_model = "model-a".into();
    let result = match_receipts(&[record], json!([row]));
    assert_no_attribution(&result);
    assert_eq!(result[0]["rejected"], true);
}

#[test]
fn fallback_merged_receipt_requires_each_original_request_to_match() {
    let mut first = request();
    first.model = "supplier-a/model-a".into();
    first.relay_id = "supplier-b".into();
    first.relay_name = "Supplier B".into();
    first.started_at_ms = 98_000;
    first.timestamp_ms = 99_000;
    first.usage = Some(RequestEvidenceUsage {
        input_tokens: Some(50),
        output_tokens: Some(5),
        cached_tokens: Some(10),
    });
    let mut second = first.clone();
    second.id = "request-2".into();
    second.started_at_ms = 99_500;
    second.timestamp_ms = 100_000;
    let mut row = receipt("merged-fallback");
    row["model"] = json!("supplier-a/model-a");

    let proof = match_receipts(&[first.clone(), second.clone()], json!([row.clone()]));
    assert_eq!(proof[0]["provider"], "supplier-b");
    assert_eq!(proof[0]["model"], "model-a");
    assert_eq!(proof[0]["requestIds"].as_array().unwrap().len(), 2);

    for changed_index in 0..2 {
        let mut records = [first.clone(), second.clone()];
        records[changed_index].model = "supplier-c/model-a".into();
        let result = match_receipts(&records, json!([row.clone()]));
        assert_no_attribution(&result);
        assert_eq!(result[0]["rejected"], true, "changed_index={changed_index}");
    }
}

#[test]
fn unknown_successful_request_blocks_routed_live_receipt() {
    let mut unknown = request();
    unknown.id = "unknown-request".into();
    unknown.model = "unknown".into();
    unknown.upstream_model = "unknown".into();
    let mut row = receipt("ambiguous");
    row["model"] = json!("auto");
    let result = match_receipts(&[request(), unknown], json!([row]));
    assert_no_attribution(&result);
    assert_eq!(result[0]["rejected"], true);
}

#[test]
fn incomplete_successful_request_blocks_ambiguous_live_receipt() {
    for missing in [
        "cached",
        "input",
        "usage",
        "partial-merge",
        "attempts",
        "retries",
    ] {
        let mut incomplete = request();
        incomplete.id = format!("incomplete-{missing}");
        match missing {
            "cached" => incomplete.usage.as_mut().unwrap().cached_tokens = None,
            "input" => incomplete.usage.as_mut().unwrap().input_tokens = None,
            "attempts" => incomplete.attempts = None,
            "retries" => incomplete.auto_retries = None,
            "partial-merge" => {
                let usage = incomplete.usage.as_mut().unwrap();
                usage.input_tokens = Some(40);
                usage.output_tokens = Some(3);
                usage.cached_tokens = None;
            }
            _ => incomplete.usage = None,
        }
        let mut row = receipt("ambiguous-partial");
        row["model"] = json!("auto");
        let result = match_receipts(&[request(), incomplete], json!([row]));
        assert_no_attribution(&result);
        assert_eq!(result[0]["rejected"], true, "{missing}");
    }

    let mut too_large = request();
    too_large.id = "larger-request".into();
    too_large.usage.as_mut().unwrap().input_tokens = Some(101);
    too_large.usage.as_mut().unwrap().cached_tokens = None;
    let mut row = receipt("unrelated-partial");
    row["model"] = json!("auto");
    let result = match_receipts(&[request(), too_large], json!([row]));
    assert_eq!(result[0]["provider"], "supplier-a");
}

#[test]
fn conflicting_duplicate_request_cannot_disappear_from_live_competition() {
    let mut competitor = request();
    competitor.id = "competing-request".into();
    let mut incomplete = competitor.clone();
    incomplete.attempts = None;
    for records in [
        vec![request(), competitor.clone(), incomplete.clone()],
        vec![request(), incomplete.clone(), competitor.clone()],
    ] {
        let result = match_receipts(&records, json!([receipt("duplicate-competitor")]));
        assert_no_attribution(&result);
        assert_eq!(result[0]["rejected"], true);
    }
}

#[test]
fn rotated_live_proof_is_revoked_when_new_competitors_appear() {
    let row = receipt("rotated-proof");
    let proof = match_receipts(&[request()], json!([row.clone()]));
    let mut saved = row;
    for key in ["model", "provider", "requestIds", "requestSignatures"] {
        saved[key] = proof[0][key].clone();
    }
    assert_eq!(match_receipts(&[], json!([saved.clone()])), json!([]));

    let mut competitor = request();
    competitor.id = "later-competitor".into();
    for incomplete in [false, true] {
        let mut candidate = competitor.clone();
        if incomplete {
            candidate.attempts = None;
        }
        let result = match_receipts(&[candidate], json!([saved.clone()]));
        assert_no_attribution(&result);
        assert_eq!(result[0]["rejected"], true, "incomplete={incomplete}");
    }

    let mut unrelated = competitor;
    unrelated.thread_id = "another-session".into();
    assert_eq!(match_receipts(&[unrelated], json!([saved])), json!([]));
}

#[test]
fn earlier_incomplete_request_blocks_merged_live_receipt() {
    let mut competitor = request();
    competitor.id = "earlier-competitor".into();
    competitor.timestamp_ms -= 60_000;
    competitor.started_at_ms -= 60_000;
    competitor.attempts = None;
    competitor.usage.as_mut().unwrap().input_tokens = Some(40);
    let result = match_receipts(&[request(), competitor], json!([receipt("earlier-part")]));
    assert_no_attribution(&result);
    assert_eq!(result[0]["rejected"], true);
}

#[test]
fn incomplete_successful_request_blocks_historical_attribution() {
    let mut fixture = Fixture::new();
    fixture.write_classified_segment("rollout-a.jsonl", "model-a", "custom", "main");
    let event = fixture.snapshot().events.remove(0);
    let at = DateTime::parse_from_rfc3339(&event.timestamp)
        .unwrap()
        .timestamp_millis();
    let mut proof = request();
    proof.started_at_ms = at - 100;
    proof.timestamp_ms = at - 10;

    for missing in ["attempts", "retries", "model", "provider", "cached"] {
        let mut competitor = proof.clone();
        competitor.id = format!("incomplete-{missing}");
        match missing {
            "attempts" => competitor.attempts = None,
            "retries" => competitor.auto_retries = None,
            "model" => competitor.upstream_model.clear(),
            "provider" => competitor.relay_id.clear(),
            "cached" => competitor.usage.as_mut().unwrap().cached_tokens = None,
            _ => unreachable!(),
        }
        let mut cache = RequestEvidenceCache::default();
        cache.records = vec![proof.clone(), competitor];
        let mut events = vec![event.clone()];
        let result = cache.attribute(&mut events, &mut BTreeMap::new(), &BTreeMap::new());
        assert_eq!(result["recoveredEvents"], 0, "{missing}");
        assert!(unknown_provider(&events[0].provider), "{missing}");
    }
}

#[test]
fn unrelated_incomplete_request_does_not_block_historical_attribution() {
    let mut fixture = Fixture::new();
    fixture.write_classified_segment("rollout-a.jsonl", "model-a", "custom", "main");
    let event = fixture.snapshot().events.remove(0);
    let at = DateTime::parse_from_rfc3339(&event.timestamp)
        .unwrap()
        .timestamp_millis();
    let mut proof = request();
    proof.started_at_ms = at - 100;
    proof.timestamp_ms = at - 10;

    for unrelated in ["session", "time", "counts", "agent", "failed"] {
        let mut other = proof.clone();
        other.id = format!("unrelated-{unrelated}");
        other.attempts = None;
        match unrelated {
            "session" => other.thread_id = "another-session".into(),
            "time" => other.timestamp_ms = at - DELAYED_REQUEST_MATCH_WINDOW_MS - 1,
            "counts" => other.usage.as_mut().unwrap().input_tokens = Some(101),
            "agent" => other.agent = "another-client".into(),
            "failed" => other.success = false,
            _ => unreachable!(),
        }
        let mut cache = RequestEvidenceCache::default();
        cache.records = vec![proof.clone(), other];
        let mut events = vec![event.clone()];
        let result = cache.attribute(&mut events, &mut BTreeMap::new(), &BTreeMap::new());
        assert_eq!(result["recoveredEvents"], 1, "{unrelated}");
        assert_eq!(events[0].provider, "supplier-a", "{unrelated}");
    }
}

#[test]
fn new_incomplete_competitor_revokes_saved_historical_proof() {
    let mut fixture = Fixture::new();
    fixture.write_classified_segment("rollout-a.jsonl", "model-a", "custom", "main");
    let event = fixture.snapshot().events.remove(0);
    let at = DateTime::parse_from_rfc3339(&event.timestamp)
        .unwrap()
        .timestamp_millis();
    let mut proof = request();
    proof.started_at_ms = at - 100;
    proof.timestamp_ms = at - 10;
    let mut cache = RequestEvidenceCache::default();
    cache.records.push(proof.clone());
    let mut events = vec![event.clone()];
    cache.attribute(&mut events, &mut BTreeMap::new(), &BTreeMap::new());
    assert_eq!(events[0].provider, "supplier-a");
    assert!(!cache.verified[&event.id].rejected);

    let mut competitor = proof;
    competitor.id = "later-incomplete-competitor".into();
    competitor.attempts = None;
    cache.records.push(competitor);
    let mut events = vec![event.clone()];
    cache.attribute(&mut events, &mut BTreeMap::new(), &BTreeMap::new());
    assert!(unknown_provider(&events[0].provider));
    assert!(cache.verified[&event.id].rejected);
    assert_eq!(events[0].delta.total_tokens, event.delta.total_tokens);

    cache.records.pop();
    let mut events = vec![event.clone()];
    cache.attribute(&mut events, &mut BTreeMap::new(), &BTreeMap::new());
    assert!(unknown_provider(&events[0].provider));
    assert!(cache.verified[&event.id].rejected);
}

#[test]
fn non_codex_request_cannot_supply_live_receipt_evidence() {
    let mut encoded = serde_json::to_value(request()).unwrap();
    encoded["agent"] = json!("claude");
    let other: RequestEvidence = serde_json::from_value(encoded).unwrap();
    assert_no_attribution(&match_receipts(&[other], json!([receipt("other-agent")])));
}

#[test]
fn missing_codex_agent_preserves_request_signature() {
    let original = serde_json::to_value(request()).unwrap();
    let mut named = original.clone();
    named["agent"] = json!("codex");
    let normalized: RequestEvidence = serde_json::from_value(named).unwrap();
    assert_eq!(serde_json::to_value(normalized).unwrap(), original);
}

const LEGACY_CHILD: &str = "00000000-0000-4000-8000-000000000011";

fn legacy_child_segment(fixture: &Fixture, child: &str, copied_header: bool) {
    let mut rows = vec![json!({"type":"session_meta", "payload":{
        "id":child, "parent_thread_id":SESSION, "subagent_history_start_ordinal":5,
        "source":{"subagent":{}}, "model_provider":"custom"
    }})];
    if copied_header {
        rows.push(json!({"type":"session_meta", "ordinal":1,
            "payload":{"id":SESSION, "model_provider":"parent-supplier"}}));
    }
    rows.push(fixture.context(500, "gpt-test"));
    let mut usage = fixture.usage(1_000, 1);
    usage["ordinal"] = json!(6);
    rows.push(usage);
    fs::write(
        fixture
            .home
            .join("sessions")
            .join(format!("rollout-{child}.jsonl")),
        rows.into_iter()
            .map(|row| format!("{row}\n"))
            .collect::<String>(),
    )
    .unwrap();
}

fn legacy_child_record(event: &UsageEvent) -> Value {
    let mut digest = Sha256::new();
    digest.update(SESSION.as_bytes());
    digest.update(event.timestamp.as_bytes());
    digest.update(serde_json::to_vec(&json!([event.total, event.last])).unwrap());
    let mut record = evidence(event, &event.model);
    record["eventId"] = json!(format!("{:x}", digest.finalize()));
    record["session"] = json!(SESSION);
    record["role"] = json!("main");
    record
}

#[test]
fn legacy_child_evidence_migration_is_exact_unique_and_does_not_change_usage() {
    let mut fixture = Fixture::new();
    legacy_child_segment(&fixture, LEGACY_CHILD, true);
    let original = fixture.snapshot().events.remove(0);
    assert_eq!(original.session, LEGACY_CHILD);
    assert_eq!(original.role, "subagent");
    assert!(unknown_provider(&original.provider));
    fixture.write_evidence(vec![legacy_child_record(&original)]);
    let snapshot = fixture.snapshot();
    let restored = &snapshot.events[0];
    assert_eq!(restored.provider, "supplier-a");
    assert_eq!(snapshot.provider_evidence["migratedEvents"], 1);
    assert_eq!(restored.id, original.id);
    assert_eq!(restored.session, original.session);
    assert_eq!(restored.role, original.role);
    assert_eq!(restored.total, original.total);
    assert_eq!(restored.delta, original.delta);
    assert_eq!(restored.model, original.model);
    assert_eq!(
        restored.provider_source.as_deref(),
        Some("upstream-exact-bidirectional")
    );
    fs::remove_file(
        fixture
            .settings
            .with_file_name("usage-provider-evidence.json"),
    )
    .unwrap();
    assert!(unknown_provider(&fixture.snapshot().events[0].provider));
}

#[test]
fn legacy_child_evidence_rejects_every_signature_mismatch() {
    for field in [
        "eventId",
        "session",
        "timestamp",
        "model",
        "role",
        "input",
        "output",
        "cached",
    ] {
        let mut fixture = Fixture::new();
        legacy_child_segment(&fixture, LEGACY_CHILD, true);
        let original = fixture.snapshot().events.remove(0);
        let mut record = legacy_child_record(&original);
        record[field] = match field {
            "eventId" => json!("a".repeat(64)),
            "session" => json!(LEGACY_CHILD),
            "timestamp" => json!(fixture.at(2_000)),
            "model" => json!("different-model"),
            "role" => json!("subagent"),
            _ => json!(record[field].as_u64().unwrap() + 1),
        };
        fixture.write_evidence(vec![record]);
        let snapshot = fixture.snapshot();
        assert!(unknown_provider(&snapshot.events[0].provider), "{field}");
        assert_eq!(snapshot.events[0].id, original.id);
        assert_eq!(snapshot.events[0].delta, original.delta);
    }
}

#[test]
fn legacy_child_evidence_requires_explicit_copied_header_and_unique_owners() {
    for competitor in [
        "missing-header",
        "another-child",
        "real-parent",
        "duplicate-current",
    ] {
        let mut fixture = Fixture::new();
        legacy_child_segment(&fixture, LEGACY_CHILD, competitor != "missing-header");
        let original = fixture.snapshot().events.remove(0);
        if competitor == "another-child" {
            legacy_child_segment(&fixture, "00000000-0000-4000-8000-000000000012", true);
        }
        if competitor == "real-parent" {
            fixture.write_segment(
                "rollout-parent.jsonl",
                vec![fixture.context(500, "gpt-test"), fixture.usage(1_000, 1)],
            );
        }
        fixture.write_evidence(vec![legacy_child_record(&original)]);
        let mut snapshot = fixture.snapshot();
        if competitor == "duplicate-current" {
            snapshot.events = vec![original.clone(), original.clone()];
            fixture
                .index
                .supplement_legacy_evidence(&fixture.settings, &mut snapshot);
        }
        for event in snapshot
            .events
            .iter()
            .filter(|event| event.session != SESSION)
        {
            assert!(unknown_provider(&event.provider), "{competitor}");
            assert_eq!(event.delta, original.delta);
        }
    }
}

#[test]
fn legacy_child_evidence_rejects_direct_conflicts_and_bad_direct_signatures() {
    for conflict in ["provider", "signature", "known-provider"] {
        let mut fixture = Fixture::new();
        legacy_child_segment(&fixture, LEGACY_CHILD, true);
        let original = fixture.snapshot().events.remove(0);
        let legacy = legacy_child_record(&original);
        let mut direct = evidence(&original, &original.model);
        if conflict == "provider" {
            direct["provider"] = json!("supplier-b");
        } else {
            direct["role"] = json!("main");
        }
        fixture.write_evidence(if conflict == "known-provider" {
            vec![legacy]
        } else {
            vec![legacy, direct]
        });
        let mut snapshot = fixture.snapshot();
        if conflict == "known-provider" {
            snapshot.events[0].provider = "supplier-b".into();
            fixture
                .index
                .supplement_legacy_evidence(&fixture.settings, &mut snapshot);
            assert_eq!(snapshot.events[0].provider, "supplier-b");
        } else {
            assert!(unknown_provider(&snapshot.events[0].provider), "{conflict}");
        }
        assert_eq!(snapshot.events[0].id, original.id);
        assert_eq!(snapshot.events[0].delta, original.delta);
    }
}
