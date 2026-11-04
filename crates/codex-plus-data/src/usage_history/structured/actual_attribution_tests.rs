use super::*;
mod actual_attribution {
    use super::*;

    fn event(model: &str, provider: &str) -> UsageEvent {
        let mut fixture = Fixture::new();
        fixture.write_classified_segment("rollout-actual.jsonl", model, provider, "main");
        fixture.snapshot().events.remove(0)
    }

    fn proof(event: &UsageEvent) -> RequestEvidence {
        let mut record = request();
        let at = DateTime::parse_from_rfc3339(&event.timestamp)
            .unwrap()
            .timestamp_millis();
        record.started_at_ms = at - 100;
        record.timestamp_ms = at - 10;
        record.model = event.model.clone();
        record.upstream_model = "actual-b".into();
        record.relay_id = "supplier-b".into();
        record.relay_name = "Supplier B".into();
        record
    }

    fn apply(
        events: &mut [UsageEvent],
        records: Vec<RequestEvidence>,
    ) -> (RequestEvidenceCache, Value) {
        let mut cache = RequestEvidenceCache {
            records,
            ..Default::default()
        };
        let result = cache.attribute(events, &mut BTreeMap::new(), &BTreeMap::new());
        (cache, result)
    }

    #[test]
    fn actual_unknown_group_auto_recover_each_event_without_changing_identity_or_counts() {
        for model in [
            "",
            "Unknown",
            "group/pool",
            "auto",
            "cpx-supplier-a/model-a",
        ] {
            let original = event(model, "custom");
            let mut events = vec![original.clone()];
            let record = proof(&original);
            let (cache, result) = apply(&mut events, vec![record]);
            assert_eq!(result["recoveredEvents"], 1, "model={model}");
            assert_eq!(events[0].provider, "supplier-b", "model={model}");
            assert_eq!(events[0].model, "actual-b", "model={model}");
            assert_eq!(events[0].id, original.id);
            assert_eq!(events[0].original_id(), original.original_id());
            assert_eq!(events[0].total, original.total);
            assert_eq!(events[0].delta, original.delta);
            assert_eq!(
                cache.verified[&original.id].key.model,
                if unknown_model(&original.model) {
                    ""
                } else {
                    request_base_model(&original.model)
                }
            );
        }
    }

    #[test]
    fn actual_cpx_routing_label_cannot_preempt_successful_fallback() {
        let names = BTreeMap::from([("supplier-a".into(), "A".into())]);
        assert!(unknown_provider(usage_provider(
            "cpx-supplier-a/model-a",
            "custom",
            &names
        )));
        let mut fixture = Fixture::new();
        fixture.write_classified_segment(
            "rollout-cpx.jsonl",
            "cpx-supplier-a/model-a",
            "custom",
            "main",
        );
        let original = fixture.snapshot().events.remove(0);
        fs::write(
            &fixture.log,
            format!("{}\n", serde_json::to_string(&proof(&original)).unwrap()),
        )
        .unwrap();
        let matched = fixture.snapshot();
        assert_eq!(matched.events[0].provider, "supplier-b");
        assert_eq!(matched.events[0].model, "actual-b");
        assert_eq!(matched.events[0].id, original.id);
        assert_eq!(matched.events[0].delta, original.delta);
    }

    #[test]
    fn actual_explicit_direct_provider_conflict_is_not_overwritten_or_counted_as_a() {
        for model in ["cpx-supplier-a/model-a", "group/pool", "auto", "model-a"] {
            let original = event(model, "supplier-a");
            let mut events = vec![original.clone()];
            let (cache, _) = apply(&mut events, vec![proof(&original)]);
            assert_ne!(events[0].provider, "supplier-b");
            assert!(unknown_provider(&events[0].provider));
            assert!(events[0].context_conflicts.provider);
            assert!(cache.verified[&original.id].rejected);
            assert_eq!(events[0].id, original.id);
            assert_eq!(events[0].delta, original.delta);
        }
    }

    #[test]
    fn actual_all_known_unknown_and_subagent_events_compete_both_directions() {
        for competitor_model in ["auto", "group/other", "known-other"] {
            for role in ["main", "subagent"] {
                let original = event("Unknown", "custom");
                let mut competitor = original.clone();
                competitor.id = [42; 32];
                competitor.model = competitor_model.into();
                competitor.provider = "known-provider".into();
                competitor.role = role.into();
                let mut events = vec![original.clone(), competitor];
                let (_, result) = apply(&mut events, vec![proof(&original)]);
                assert_eq!(result["recoveredEvents"], 0);
                assert!(unknown_provider(&events[0].provider));
                assert_eq!(events[1].provider, "known-provider");
            }
        }
    }

    #[test]
    fn actual_all_successful_candidates_compete_even_when_models_differ_or_are_unknown() {
        for competing_model in ["model-a", "other", "Unknown", "auto"] {
            let original = event("Unknown", "custom");
            let first = proof(&original);
            let mut second = first.clone();
            second.id = "request-2".into();
            second.model = competing_model.into();
            second.upstream_model = competing_model.into();
            second.relay_id = "supplier-c".into();
            let mut events = vec![original];
            let (_, result) = apply(&mut events, vec![first, second]);
            assert_eq!(result["recoveredEvents"], 0);
            assert!(unknown_provider(&events[0].provider));
        }
    }

    #[test]
    fn actual_full_counts_time_and_session_are_required() {
        for mutation in 0..7 {
            let original = event("auto", "custom");
            let mut record = proof(&original);
            match mutation {
                0 => record.thread_id = "different-session".into(),
                1 => record.usage.as_mut().unwrap().input_tokens = Some(101),
                2 => record.usage.as_mut().unwrap().output_tokens = Some(11),
                3 => record.usage.as_mut().unwrap().cached_tokens = None,
                4 => record.timestamp_ms += DELAYED_REQUEST_MATCH_WINDOW_MS + 1_000,
                5 => record.success = false,
                _ => record.status_code = 500,
            }
            let mut events = vec![original];
            let (_, result) = apply(&mut events, vec![record]);
            assert_eq!(result["recoveredEvents"], 0);
            assert!(unknown_provider(&events[0].provider));
        }
    }

    #[test]
    fn actual_source_classification_survives_requery_retention_and_revocation() {
        let original = event("group/pool", "custom");
        let record = proof(&original);
        let (mut cache, _) = apply(&mut [original.clone()], vec![record.clone()]);
        cache.records.clear();
        let mut restored = vec![original.clone()];
        cache.attribute(&mut restored, &mut BTreeMap::new(), &BTreeMap::new());
        assert_eq!(restored[0].provider, "supplier-b");
        assert_eq!(restored[0].id, original.id);
        let mut changed = record;
        changed.relay_id = "supplier-c".into();
        cache.records.push(changed);
        let mut revoked = vec![original.clone()];
        cache.attribute(&mut revoked, &mut BTreeMap::new(), &BTreeMap::new());
        assert_eq!(revoked[0].model, original.model);
        assert!(unknown_provider(&revoked[0].provider));
        assert!(cache.verified[&original.id].rejected);
    }

    fn relay(id: &str, endpoint: &str) -> codex_plus_core::settings::RelayProfile {
        codex_plus_core::settings::RelayProfile {
            id: id.into(),
            name: format!("Name {id}"),
            relay_mode: codex_plus_core::settings::RelayMode::PureApi,
            base_url: endpoint.into(),
            upstream_base_url: endpoint.into(),
            config_contents: format!(
                "model_provider = \"custom\"\n[model_providers.custom]\nname = \"Custom\"\nbase_url = \"{endpoint}\"\nwire_api = \"responses\"\n"
            ),
            ..Default::default()
        }
    }

    #[test]
    fn actual_gateway_profiles_merge_without_collapsing_distinct_endpoint_identities() {
        let fixture = Fixture::new();
        let legacy = relay("old-id", "https://one.example/v1");
        let gateway = relay("new-id", "https://one.example/v1");
        fs::write(
            &fixture.settings,
            json!({"relayProfiles":[legacy]}).to_string(),
        )
        .unwrap();
        fs::write(
            fixture._root.path().join("gateway.json"),
            json!({"providers":[
                {"source":"codex:old-id", "relay":gateway},
                {"relay":relay("only-gateway", "https://two.example/v1")}
            ]})
            .to_string(),
        )
        .unwrap();
        let profiles = native_endpoints::profiles(&fixture.settings).unwrap();
        assert_eq!(profiles.len(), 3);
        let endpoints = native_endpoints::endpoints(&profiles);
        assert_eq!(endpoints["https://one.example/v1/responses"], None);
        assert_eq!(
            endpoints["https://two.example/v1/responses"].as_deref(),
            Some("only-gateway")
        );
    }

    #[test]
    fn actual_same_id_alias_deduplicates_but_changed_endpoint_identity_is_ambiguous() {
        let one = relay("supplier", "https://one.example/v1");
        let mut alias = one.clone();
        alias.name = "Renamed".into();
        let same = native_endpoints::endpoints(&[one.clone(), alias]);
        assert_eq!(
            same["https://one.example/v1/responses"].as_deref(),
            Some("supplier")
        );
        let changed =
            native_endpoints::endpoints(&[one, relay("supplier", "https://two.example/v1")]);
        assert!(changed.values().all(Option::is_none));
    }

    #[test]
    fn actual_gateway_change_invalidates_index_and_never_relabels_older_http_rows() {
        let mut fixture = Fixture::new();
        fixture.snapshot();
        let before = fixture.index.provider_stamp.clone();
        let gateway = fixture._root.path().join("gateway.json");
        fs::write(
            &gateway,
            json!({"providers":[{"relay":relay("only-gateway", "https://two.example/v1")}]})
                .to_string(),
        )
        .unwrap();
        fixture.snapshot();
        assert_ne!(fixture.index.provider_stamp, before);
        assert_eq!(
            fixture.index.provider_endpoints["https://two.example/v1/responses"].as_deref(),
            Some("only-gateway")
        );
        let at = fixture.index.provider_endpoint_since - 1;
        let body = format!(
            "{{thread.id={SESSION} turn.id=turn-1 model=model-a}}: Request completed method=POST url=https://two.example/v1/responses status=200 headers={{}}"
        );
        let request = native_endpoints::request(
            &body,
            SESSION,
            (at.div_euclid(1000), at.rem_euclid(1000) * 1_000_000),
            &fixture.index.provider_endpoints,
            fixture.index.provider_endpoint_since,
            "fixture",
            1,
        )
        .unwrap();
        assert!(serde_json::to_value(&request).unwrap()["provider"].is_null());
    }

    #[test]
    fn actual_request_agent_filter_preserves_codex_legacy_signatures() {
        let old = serde_json::to_value(request()).unwrap();
        assert!(old.get("agent").is_none());
        for agent in [json!(null), json!("codex"), json!(" Codex ")] {
            let mut with_agent = old.clone();
            with_agent["agent"] = agent;
            let record: RequestEvidence = serde_json::from_value(with_agent).unwrap();
            assert!(!record.keys().is_empty());
            assert_eq!(serde_json::to_value(record).unwrap(), old);
        }
        let original = event("auto", "custom");
        let mut wrong_agent = serde_json::to_value(proof(&original)).unwrap();
        wrong_agent["agent"] = json!("claude");
        let mut events = vec![original];
        let (_, result) = apply(
            &mut events,
            vec![serde_json::from_value(wrong_agent).unwrap()],
        );
        assert_eq!(result["recoveredEvents"], 0);
    }

    #[test]
    fn actual_live_receipts_report_upstream_model_and_preserve_competing_claims() {
        let mut record = request();
        record.model = "group/pool".into();
        record.upstream_model = "actual-b".into();
        let mut row = receipt("live");
        row["model"] = json!("auto");
        let matched = match_receipts(&[record.clone()], json!([row.clone()]));
        assert_eq!(matched[0]["model"], "actual-b");
        let mut competitor = record.clone();
        competitor.id = "request-2".into();
        competitor.upstream_model = "other".into();
        assert_no_attribution(&match_receipts(
            &[record.clone(), competitor],
            json!([row.clone()]),
        ));
        let mut competing_row = row.clone();
        competing_row["id"] = json!("known-other");
        competing_row["model"] = json!("different");
        competing_row["provider"] = json!("known-provider");
        assert_no_attribution(&match_receipts(
            &[record.clone()],
            json!([row.clone(), competing_row]),
        ));
        let mut incomplete = record.clone();
        incomplete.id = "request-incomplete".into();
        incomplete.model = "Unknown".into();
        incomplete.upstream_model = "Unknown".into();
        assert_no_attribution(&match_receipts(&[record, incomplete], json!([row])));
    }

    #[test]
    fn actual_delayed_and_strict_candidates_cannot_preempt_each_other() {
        let original = event("auto", "custom");
        let first = proof(&original);
        let mut delayed = first.clone();
        delayed.id = "delayed".into();
        delayed.started_at_ms -= 60_000;
        delayed.timestamp_ms -= 60_000;
        let (_, result) = apply(&mut [original.clone()], vec![first, delayed.clone()]);
        assert_eq!(result["recoveredEvents"], 0);
        let mut events = vec![original.clone()];
        let (_, result) = apply(&mut events, vec![delayed.clone()]);
        assert_eq!(result["delayedEvents"], 1);
        delayed.attempts = Some(2);
        let (_, result) = apply(&mut [original], vec![delayed]);
        assert_eq!(result["recoveredEvents"], 0);
    }

    #[test]
    fn actual_bad_gateway_configuration_clears_mapping_and_retries() {
        let mut fixture = Fixture::new();
        let gateway = fixture._root.path().join("gateway.json");
        fs::write(
            &gateway,
            json!({"providers":[{"relay":relay("provider", "https://one.example/v1")}]})
                .to_string(),
        )
        .unwrap();
        fixture.snapshot();
        assert!(!fixture.index.provider_endpoints.is_empty());
        fs::write(&gateway, "{invalid").unwrap();
        fixture.snapshot();
        assert!(fixture.index.provider_endpoints.is_empty());
        assert!(fixture.index.provider_stamp.is_none());
        fs::write(&gateway, "{\"providers\":[]}").unwrap();
        fixture.snapshot();
        assert!(fixture.index.provider_stamp.is_some());
    }
}
