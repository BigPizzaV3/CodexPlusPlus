use super::*;

#[test]
fn legacy_placeholder_keys_restore_without_unrejecting_real_conflicts() {
    let cutoff = Local::now().date_naive() - Days::days(1);
    for model in ["auto", "Unknown", "group/pool", " AUTO ", "Group/pool"] {
        for conflict in [
            "none",
            "rejected",
            "model",
            "session",
            "counts",
            "time",
            "signature",
        ] {
            let root = tempfile::tempdir().unwrap();
            let mut disk = provider_cache::ProviderCache::default();
            disk.open(root.path(), cutoff).unwrap();
            let mut state = FileState::default();
            accept(
                &mut state,
                json!({"type": "session_meta", "payload": {"id": SESSION, "model_provider": "custom"}}),
            );
            let mut row = context();
            row["payload"]["model"] = json!(model);
            row["payload"]["model_provider"] = json!("custom");
            accept(&mut state, row);
            accept(&mut state, usage());
            let original = state.events.remove(0);
            let at = DateTime::parse_from_rfc3339(&original.timestamp)
                .unwrap()
                .timestamp_millis();
            let request = RequestEvidence {
                id: "request-a".into(),
                thread_id: SESSION.into(),
                model: model.into(),
                upstream_model: "actual-model".into(),
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
                agent: String::new(),
                usage: Some(RequestEvidenceUsage {
                    input_tokens: Some(100),
                    output_tokens: Some(10),
                    cached_tokens: Some(20),
                }),
            };
            let mut saved = VerifiedAttribution {
                key: RequestMatchKey {
                    session: SESSION.into(),
                    model: model.into(),
                    input: 100,
                    output: 10,
                    cached: 20,
                },
                at,
                request: request.clone(),
                rejected: conflict == "rejected",
            };
            match conflict {
                "model" => saved.key.model = "different-known-model".into(),
                "session" => saved.key.session = CHILD.into(),
                "counts" => saved.key.input += 1,
                "time" => saved.at -= 1,
                _ => {}
            }
            disk.save(&HashMap::from([(original.id, saved)]), &BTreeMap::new())
                .unwrap();
            let mut cache = RequestEvidenceCache::default();
            cache.restore_verified(disk.load_verified(cutoff).unwrap());
            if conflict == "signature" {
                let mut changed = request;
                changed.upstream_model = "different-upstream-model".into();
                cache.records.push(changed);
            }
            let mut events = vec![original.clone()];
            let result = cache.attribute(&mut events, &mut BTreeMap::new(), &BTreeMap::new());
            assert_eq!(
                result["recoveredEvents"],
                usize::from(conflict == "none"),
                "{model}, conflict={conflict}"
            );
            assert_eq!(
                cache.verified[&original.id].rejected,
                conflict != "none",
                "{model}, {conflict}"
            );
            assert_eq!(events[0].delta, original.delta);
            assert_eq!(events[0].id, original.id);
            if conflict == "none" {
                assert_eq!(events[0].model, "actual-model");
                assert_eq!(events[0].provider, "supplier-a");
            } else {
                assert!(events == vec![original], "{model}, {conflict}");
            }
        }
    }
}

#[test]
fn session_id_alias_validates_uuid_before_accepting_context() {
    for head in [
        json!({"session_id": SESSION}),
        json!({"id": "invalid", "session_id": SESSION}),
    ] {
        let mut state = FileState::default();
        accept(&mut state, json!({"type": "session_meta", "payload": head}));
        accept(&mut state, context());
        accept(&mut state, usage());
        assert_eq!(state.events.len(), 1);
        assert_eq!(state.events[0].session, SESSION);
        assert_eq!(state.events[0].model, "model-a");
        assert_eq!(state.events[0].provider, "supplier-a");
    }
    for invalid in [
        "not-a-uuid",
        "00000000000040008000000000000010",
        "",
        "{00000000-0000-4000-8000-000000000010}",
    ] {
        let mut state = FileState::default();
        accept(
            &mut state,
            json!({"type": "session_meta", "payload": {"session_id": invalid}}),
        );
        accept(&mut state, context());
        accept(&mut state, usage());
        assert!(state.events.is_empty(), "{invalid}");
        assert!(state.invalid_counts);
    }
}

#[test]
fn two_own_heads_reject_later_usage_without_rebinding_aliases() {
    for field in ["id", "session_id"] {
        let mut state = FileState::default();
        accept(
            &mut state,
            json!({"type": "session_meta", "payload": {"id": SESSION}}),
        );
        accept(&mut state, context());
        accept(&mut state, usage());
        let original = state.events[0].clone();
        let mut payload = json!({"model_provider": "supplier-b"});
        payload[field] = json!(CHILD);
        accept(
            &mut state,
            json!({"type": "session_meta", "payload": payload}),
        );
        let mut later = usage();
        later["payload"]["info"]["total_token_usage"]["input_tokens"] = json!(200);
        accept(&mut state, later);
        assert_eq!(state.session, SESSION, "{field}");
        assert!(state.events == vec![original], "{field}");
        assert!(state.invalid_counts, "{field}");
        assert!(state.issues > 0, "{field}");
    }
}

#[test]
fn conflicting_file_cannot_override_valid_alias_or_parent_scope() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    fs::create_dir_all(home.join("sessions")).unwrap();
    let home = fs::canonicalize(home).unwrap();
    let alias = "00000000-0000-4000-8000-000000000012";
    let write = |id: &str, rows: Vec<Value>| {
        let rows: Vec<_> = rows.into_iter().map(|row| row.to_string()).collect();
        fs::write(
            home.join("sessions").join(format!("rollout-{id}.jsonl")),
            rows.join("\n") + "\n",
        )
        .unwrap();
    };
    write(
        SESSION,
        vec![
            json!({"type": "session_meta", "payload": {"id": CHILD}}),
            context(),
            usage(),
        ],
    );
    write(
        alias,
        vec![
            json!({"type": "session_meta", "payload": {"id": SESSION,
            "source": {"subagent": {"thread_spawn": {"parent_thread_id": CHILD}}}}}),
            context(),
            usage(),
            json!({"type": "session_meta", "payload": {"id": CHILD}}),
        ],
    );
    let mut index = Index {
        settings_path: Some(root.path().join("settings.json")),
        storage_dir: Some(root.path().join("cache")),
        ..Index::default()
    };
    let snapshot = index.update_with_request_log(&home, &root.path().join("requests.jsonl"));
    assert_eq!(
        snapshot.aliases.get(SESSION).map(String::as_str),
        Some(CHILD)
    );
    assert!(!snapshot.aliases.contains_key(alias));
    assert!(!snapshot.session_info.contains_key(SESSION));
    assert_eq!(snapshot.events.len(), 1);
    assert!(snapshot.scope_parents.is_empty());
}

#[test]
fn repeated_same_own_head_is_legal() {
    let mut state = FileState::default();
    for field in ["id", "session_id"] {
        let mut payload = json!({});
        payload[field] = json!(SESSION);
        accept(
            &mut state,
            json!({"type": "session_meta", "payload": payload}),
        );
    }
    accept(&mut state, context());
    accept(&mut state, usage());
    assert_eq!(state.events.len(), 1);
    assert!(!state.identity_conflict);
    assert!(!state.invalid_counts);
}

#[test]
fn inherited_parent_head_remains_legal_and_context_stays_with_owner() {
    let mut state = FileState::default();
    accept(
        &mut state,
        json!({"type": "session_meta", "payload": {
        "id": CHILD, "forked_from_id": SESSION, "subagent_history_start_ordinal": 3,
        "source": {"subagent": {"thread_spawn": {"parent_thread_id": SESSION}}}, "model_provider": "supplier-child"}}),
    );
    accept(
        &mut state,
        json!({"type": "session_meta", "ordinal": 0,
        "payload": {"session_id": SESSION, "model_provider": "supplier-a"}}),
    );
    accept(&mut state, context());
    accept(&mut state, usage());
    let mut own_context = context();
    own_context["ordinal"] = json!(3);
    own_context["payload"]["model"] = json!("model-child");
    own_context["payload"]["model_provider"] = json!("supplier-child");
    accept(&mut state, own_context);
    let mut own_usage = usage();
    own_usage["ordinal"] = json!(4);
    own_usage["payload"]["info"]["total_token_usage"]["input_tokens"] = json!(200);
    accept(&mut state, own_usage);
    assert_eq!(state.session, CHILD);
    assert_eq!(state.events.len(), 2);
    assert!(state.events[0].inherited);
    assert_eq!(state.events[0].session, SESSION);
    assert_eq!(state.events[0].model, "model-a");
    assert_eq!(state.events[0].provider, "supplier-a");
    assert!(!state.events[1].inherited);
    assert_eq!(state.events[1].session, CHILD);
    assert_eq!(state.events[1].model, "model-child");
    assert_eq!(state.events[1].provider, "supplier-child");
    assert!(!state.invalid_counts);
}

const SESSION: &str = "00000000-0000-4000-8000-000000000010";
const CHILD: &str = "00000000-0000-4000-8000-000000000011";

fn profile(id: &str, endpoint: &str) -> Value {
    json!({"id": id, "name": id, "baseUrl": endpoint})
}

fn gateway(path: &Path, profiles: Vec<Value>) {
    let providers: Vec<_> = profiles
        .into_iter()
        .map(|relay| json!({"relay": relay}))
        .collect();
    fs::write(path, json!({"providers": providers}).to_string()).unwrap();
}

fn accept(state: &mut FileState, row: Value) {
    state.accept(
        &serde_json::to_vec(&row).unwrap(),
        Local::now().date_naive() - Days::days(1),
    );
}

fn context() -> Value {
    json!({"type": "turn_context", "ordinal": 1,
        "timestamp": Local::now().to_rfc3339(),
        "payload": {"model": "model-a", "turn_id": "turn-a", "model_provider": "supplier-a"}})
}

fn usage() -> Value {
    json!({"type": "event_msg", "ordinal": 2,
        "timestamp": Local::now().to_rfc3339(), "payload": {"type": "token_count", "info": {
            "total_token_usage": {"input_tokens": 100, "output_tokens": 10, "cached_input_tokens": 20},
            "last_token_usage": {"input_tokens": 100, "output_tokens": 10, "cached_input_tokens": 20}}}})
}

#[test]
fn gateway_only_profiles_are_read_without_relay_settings() {
    let root = tempfile::tempdir().unwrap();
    let settings = root.path().join("settings.json");
    gateway(
        &root.path().join("gateway.json"),
        vec![profile("gateway-a", "https://gateway.example/v1")],
    );
    let stamp = native_endpoints::config_stamp(&settings);
    assert!(stamp[0].is_none());
    assert!(stamp[1].is_some());
    let profiles = native_endpoints::profiles(&settings).unwrap();
    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0].id, "gateway-a");
    assert_eq!(native_endpoints::config_stamp(&settings), stamp);
}

#[test]
fn gateway_changes_refresh_index_names_and_endpoints_and_fail_closed() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    fs::create_dir_all(home.join("sessions")).unwrap();
    let settings = root.path().join("settings.json");
    fs::write(
        &settings,
        json!({"relayProfiles": [profile("relay-a", "https://relay.example/v1")]}).to_string(),
    )
    .unwrap();
    let gateway_path = root.path().join("gateway.json");
    let request_log = root.path().join("requests.jsonl");
    let mut index = Index {
        settings_path: Some(settings),
        storage_dir: Some(root.path().join("cache")),
        ..Index::default()
    };
    let first = index.update_with_request_log(&home, &request_log);
    assert_eq!(first.provider_names.len(), 1);
    gateway(
        &gateway_path,
        vec![profile("gateway-a", "https://gateway.example/v1")],
    );
    let second = index.update_with_request_log(&home, &request_log);
    assert!(second.provider_names.contains_key("gateway-a"));
    assert_eq!(
        index.provider_endpoints["https://gateway.example/v1/responses"].as_deref(),
        Some("gateway-a")
    );
    let old_stamp = index.provider_stamp.clone();
    gateway(
        &gateway_path,
        vec![profile(
            "gateway-renamed",
            "https://updated-gateway.example/v1",
        )],
    );
    let updated = index.update_with_request_log(&home, &request_log);
    assert_ne!(index.provider_stamp, old_stamp);
    assert!(!updated.provider_names.contains_key("gateway-a"));
    assert!(updated.provider_names.contains_key("gateway-renamed"));
    assert!(
        !index
            .provider_endpoints
            .contains_key("https://gateway.example/v1/responses")
    );
    assert_eq!(
        index.provider_endpoints["https://updated-gateway.example/v1/responses"].as_deref(),
        Some("gateway-renamed")
    );
    fs::write(&gateway_path, "{").unwrap();
    let invalid = index.update_with_request_log(&home, &request_log);
    assert!(invalid.provider_names.is_empty());
    assert!(index.provider_endpoints.is_empty());
    assert!(index.provider_stamp.is_none());
    gateway(&gateway_path, Vec::new());
    let restored = index.update_with_request_log(&home, &request_log);
    assert_eq!(restored.provider_names.len(), 1);
    assert!(!restored.provider_names.contains_key("gateway-a"));
}

#[test]
fn duplicate_ids_and_shared_endpoints_are_ambiguous() {
    let profiles: Vec<_> = [
        profile("same", "https://one.example/v1"),
        profile("same", "https://two.example/v1"),
        profile("other", "https://three.example/v1"),
        profile("another", "https://three.example/v1"),
        profile("unique", "https://unique.example/v1"),
        profile("unique", "https://unique.example/v1"),
        profile("public-and-local", "https://public.example/v1"),
        profile("public-and-local", "http://127.0.0.1/v1"),
    ]
    .into_iter()
    .map(|value| serde_json::from_value(value).unwrap())
    .collect();
    let endpoints = native_endpoints::endpoints(&profiles);
    assert_eq!(endpoints["https://one.example/v1/responses"], None);
    assert_eq!(endpoints["https://two.example/v1/responses"], None);
    assert_eq!(endpoints["https://three.example/v1/responses"], None);
    assert_eq!(endpoints["https://public.example/v1/responses"], None);
    assert_eq!(
        endpoints["https://unique.example/v1/responses"].as_deref(),
        Some("unique")
    );
}
