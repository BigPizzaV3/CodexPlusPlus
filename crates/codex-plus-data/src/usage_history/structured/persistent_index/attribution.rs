//! 复用宿主现有归类器，仅读取证据；用量计数不依赖归类结果。
use super::super::{
    MAX_REQUEST_LOG_BYTES, RequestEvidenceCache, RequestSessionAliases, UsageEvent,
    native_endpoints, provider_cache,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::Path;

pub(super) struct Inputs<'a> {
    pub(super) home: &'a Path,
    pub(super) request_log: &'a Path,
    pub(super) cache: &'a provider_cache::ProviderCache,
    pub(super) aliases: &'a BTreeMap<String, String>,
    pub(super) collision_families: &'a HashSet<[u8; 32]>,
    pub(super) inherited_uncertain: &'a HashMap<String, (Vec<i64>, bool)>,
    pub(super) endpoints: &'a BTreeMap<String, Option<String>>,
    pub(super) endpoint_since: i64,
    // 调用方须提供范围内所有竞争事件，仅标记有关来源已读完的规范会话。
    pub(super) ready_sessions: &'a HashSet<String>,
}

#[derive(Default)]
pub(super) struct Report {
    pub(super) proxy: Option<serde_json::Value>,
    pub(super) native: Option<serde_json::Value>,
    pub(super) proxy_error: Option<String>,
    pub(super) incomplete_sessions: usize,
}

#[derive(Default)]
pub(super) struct Classifier {
    requests: RequestEvidenceCache,
    native: native_endpoints::Cache,
}

impl Classifier {
    fn check_request_log(&mut self, path: &Path) -> anyhow::Result<()> {
        self.requests.refresh(path);
        let metadata = fs::symlink_metadata(path)?;
        anyhow::ensure!(
            metadata.is_file() && metadata.len() <= MAX_REQUEST_LOG_BYTES,
            "request log is not a bounded regular file"
        );
        anyhow::ensure!(
            metadata.len() != 0,
            "request log is empty and may be rotating"
        );
        let expected = (
            metadata.len(),
            metadata.modified().ok(),
            super::super::observed_file_identity(path, &metadata)?,
        );
        anyhow::ensure!(
            self.requests.stamp == Some(expected),
            "request log changed while reading"
        );
        anyhow::ensure!(
            self.requests.read_complete,
            "request log is incomplete or contains an unreadable record"
        );
        Ok(())
    }

    pub(super) fn attribute(
        &mut self,
        events: &mut [UsageEvent],
        names: &mut BTreeMap<String, String>,
        input: &Inputs<'_>,
    ) -> Report {
        let aliases = RequestSessionAliases::new(input.aliases);
        let mut incomplete = HashSet::new();
        let mut positions = Vec::new();
        let mut ready = Vec::new();
        for (position, event) in events.iter().enumerate() {
            match aliases.resolve(&event.session) {
                Some(session) if input.ready_sessions.contains(&session) => {
                    positions.push(position);
                    ready.push(event.clone());
                }
                Some(session) => {
                    incomplete.insert(session);
                }
                None => {
                    incomplete.insert(event.session.clone());
                }
            }
        }
        let mut report = Report {
            incomplete_sessions: incomplete.len(),
            ..Report::default()
        };
        if ready.is_empty() {
            return report;
        }
        self.requests.verified.clear();
        match self.check_request_log(input.request_log) {
            Ok(()) => {
                self.requests
                    .collision_families
                    .clone_from(input.collision_families);
                self.requests
                    .inherited_uncertain
                    .clone_from(input.inherited_uncertain);
                let (_, limited) =
                    self.requests
                        .supplement_captured(&ready, input.aliases, input.cache, true);
                match input.cache.rejected_proxy_ids_read_only(
                    self.requests
                        .records
                        .iter()
                        .chain(self.requests.verified.values().map(|saved| &saved.request)),
                ) {
                    Ok(rejected) => {
                        self.requests.captured_id_check_failed = false;
                        self.requests.captured_rejected = rejected;
                        let mut result = self.requests.attribute_with_actual_model(
                            &mut ready,
                            names,
                            input.aliases,
                            true,
                        );
                        result["capturedLimited"] = serde_json::json!(limited);
                        report.proxy = Some(result);
                    }
                    Err(error) => {
                        report.proxy_error =
                            Some(format!("request evidence unavailable: {error:#}"));
                    }
                }
            }
            Err(error) => report.proxy_error = Some(format!("{error:#}")),
        }
        if !input.endpoints.is_empty() && input.endpoint_since > 0 {
            self.native = native_endpoints::Cache::default();
            let prior: Vec<_> = ready.iter().map(|event| event.provider.clone()).collect();
            report.native = Some(self.native.attribute(
                input.home,
                &mut ready,
                input.endpoints,
                input.endpoint_since,
                input.aliases,
                input.cache,
                true,
            ));
            for (event, previous) in ready.iter_mut().zip(prior) {
                if super::super::unknown_provider(&previous)
                    && !super::super::unknown_provider(&event.provider)
                {
                    event.provider_source = Some("native-http-endpoint".into());
                }
            }
        }
        for (position, attributed) in positions.into_iter().zip(ready) {
            let original = &mut events[position];
            original.model = attributed.model;
            original.provider = attributed.provider;
            original.provider_source = attributed.provider_source;
            original.context_conflicts = attributed.context_conflicts;
        }
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage_history::structured::{
        ContextConflicts, MAX_REQUEST_EVIDENCE, MAX_REQUEST_LOG_LINE, RequestEvidence,
        RequestEvidenceUsage, Tokens,
    };
    use chrono::{DateTime, Days, Local};

    const SESSION: &str = "00000000-0000-4000-8000-000000000010";

    fn event(at: i64) -> UsageEvent {
        UsageEvent {
            id: [7; 32],
            session: SESSION.into(),
            turn: "turn-a".into(),
            model: "unknown".into(),
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
            relay_name: String::new(),
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

    #[test]
    fn native_capture_reuses_an_existing_read_only_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let cutoff = Local::now().date_naive() - Days::new(1);
        let mut writer = provider_cache::ProviderCache::default();
        writer.open(dir.path(), cutoff).unwrap();
        drop(writer);
        let cache = provider_cache::ProviderCache::read_snapshot(dir.path(), cutoff).unwrap();
        let result = cache.captured_native(SESSION, 0, i64::MAX).unwrap();
        assert!(result.rows.is_empty());
    }

    #[test]
    fn read_only_binding_requires_the_current_endpoint_map() {
        let dir = tempfile::tempdir().unwrap();
        let cutoff = Local::now().date_naive() - Days::new(1);
        let endpoints = BTreeMap::from([(
            "https://supplier.example/v1/responses".into(),
            Some("supplier-a".into()),
        )]);
        let mut writer = provider_cache::ProviderCache::default();
        writer.open(dir.path(), cutoff).unwrap();
        let since = writer.endpoint_binding_since(&endpoints).unwrap();
        drop(writer);
        let cache =
            provider_cache::ProviderCache::read_evidence_snapshot(dir.path(), cutoff).unwrap();
        assert!(cache.aliases.is_empty());
        assert_eq!(
            cache.endpoint_binding_since_read_only(&endpoints).unwrap(),
            Some(since)
        );
        assert_eq!(
            cache
                .endpoint_binding_since_read_only(&BTreeMap::new())
                .unwrap(),
            None
        );
        assert_eq!(
            cache.endpoint_binding_since_read_only(&endpoints).unwrap(),
            Some(since)
        );
    }

    #[test]
    fn read_only_proxy_classification_preserves_counts_and_stable_identity() {
        let dir = tempfile::tempdir().unwrap();
        let at = Local::now().timestamp_millis() - 1_000;
        let mut record = request(at);
        let log = dir.path().join("request-log.jsonl");
        let cutoff = Local::now().date_naive() - Days::new(1);
        let mut writer = provider_cache::ProviderCache::default();
        writer.open(dir.path(), cutoff).unwrap();
        writer.capture_proxy(&[record.clone()]).unwrap();
        record.relay_name = "Renamed supplier".into();
        fs::write(
            &log,
            format!("{}\n", serde_json::to_string(&record).unwrap()),
        )
        .unwrap();
        drop(writer);
        let cache = provider_cache::ProviderCache::read_snapshot(dir.path(), cutoff).unwrap();
        let aliases = BTreeMap::new();
        let collisions = HashSet::new();
        let uncertain = HashMap::new();
        let endpoints = BTreeMap::new();
        let ready_sessions = HashSet::from([SESSION.to_owned()]);
        let inputs = Inputs {
            home: dir.path(),
            request_log: &log,
            cache: &cache,
            aliases: &aliases,
            collision_families: &collisions,
            inherited_uncertain: &uncertain,
            endpoints: &endpoints,
            endpoint_since: 0,
            ready_sessions: &ready_sessions,
        };
        let mut events = vec![event(at)];
        let before = events[0].delta;
        let mut names = BTreeMap::new();
        let result = Classifier::default().attribute(&mut events, &mut names, &inputs);
        assert!(result.proxy_error.is_none(), "{:?}", result.proxy_error);
        assert_eq!(result.proxy.unwrap()["recoveredEvents"], 1);
        assert_eq!(events[0].provider, "supplier-a");
        assert_eq!(events[0].model, "model-a");
        assert_eq!(events[0].delta, before);
        assert_eq!(names["supplier-a"], "Renamed supplier");
    }

    #[test]
    fn known_requested_model_uses_actual_model_only_with_complete_capture() {
        let dir = tempfile::tempdir().unwrap();
        let at = Local::now().timestamp_millis() - 1_000;
        let mut record = request(at);
        record.model = "requested-model".into();
        record.upstream_model = "actual-model".into();
        let log = dir.path().join("request-log.jsonl");
        fs::write(
            &log,
            format!("{}\n", serde_json::to_string(&record).unwrap()),
        )
        .unwrap();
        let cutoff = Local::now().date_naive() - Days::new(1);
        let mut writer = provider_cache::ProviderCache::default();
        writer.open(dir.path(), cutoff).unwrap();
        writer.capture_proxy(&[record.clone()]).unwrap();
        drop(writer);
        let cache = provider_cache::ProviderCache::read_snapshot(dir.path(), cutoff).unwrap();
        let aliases = BTreeMap::new();
        let collisions = HashSet::new();
        let uncertain = HashMap::new();
        let endpoints = BTreeMap::new();
        let ready_sessions = HashSet::from([SESSION.to_owned()]);
        let inputs = Inputs {
            home: dir.path(),
            request_log: &log,
            cache: &cache,
            aliases: &aliases,
            collision_families: &collisions,
            inherited_uncertain: &uncertain,
            endpoints: &endpoints,
            endpoint_since: 0,
            ready_sessions: &ready_sessions,
        };
        let mut original = event(at);
        original.model = "requested-model".into();
        original.provider = "supplier-a".into();
        let mut events = vec![original.clone()];
        let result = Classifier::default().attribute(&mut events, &mut BTreeMap::new(), &inputs);
        assert!(result.proxy_error.is_none(), "{:?}", result.proxy_error);
        assert_eq!(events[0].model, "actual-model");
        assert_eq!(events[0].provider, original.provider);
        assert_eq!(events[0].delta, original.delta);

        let mut competing = record;
        competing.id = "competing-request".into();
        fs::write(
            &log,
            format!("{}\n", serde_json::to_string(&competing).unwrap()),
        )
        .unwrap();
        let mut events = vec![original.clone()];
        let result = Classifier::default().attribute(&mut events, &mut BTreeMap::new(), &inputs);
        assert!(result.proxy_error.is_none(), "{:?}", result.proxy_error);
        assert_eq!(events[0].model, original.model);
        assert_eq!(events[0].provider, original.provider);
        assert_eq!(events[0].delta, original.delta);
    }

    #[test]
    fn first_refresh_and_path_change_preserve_competing_event_evidence() {
        let dir = tempfile::tempdir().unwrap();
        let at = Local::now().timestamp_millis() - 1_000;
        let record = request(at);
        let logs = [
            dir.path().join("request-log-a.jsonl"),
            dir.path().join("request-log-b.jsonl"),
        ];
        for log in &logs {
            fs::write(
                log,
                format!("{}\n", serde_json::to_string(&record).unwrap()),
            )
            .unwrap();
        }
        let cutoff = Local::now().date_naive() - Days::new(1);
        let mut writer = provider_cache::ProviderCache::default();
        writer.open(dir.path(), cutoff).unwrap();
        writer.capture_proxy(&[record]).unwrap();
        drop(writer);
        let cache = provider_cache::ProviderCache::read_snapshot(dir.path(), cutoff).unwrap();
        let aliases = BTreeMap::new();
        let endpoints = BTreeMap::new();
        let ready_sessions = HashSet::from([SESSION.to_owned()]);
        let no_collisions = HashSet::new();
        let collisions = HashSet::from([event(at).original_id()]);
        let no_uncertainty = HashMap::new();
        let uncertain = HashMap::from([(SESSION.to_owned(), (vec![at], false))]);
        let mut classifier = Classifier::default();
        for (index, (collision, inherited)) in [
            (&collisions, &no_uncertainty),
            (&no_collisions, &uncertain),
            (&collisions, &no_uncertainty),
            (&no_collisions, &uncertain),
        ]
        .into_iter()
        .enumerate()
        {
            let inputs = Inputs {
                home: dir.path(),
                request_log: &logs[index % 2],
                cache: &cache,
                aliases: &aliases,
                collision_families: collision,
                inherited_uncertain: inherited,
                endpoints: &endpoints,
                endpoint_since: 0,
                ready_sessions: &ready_sessions,
            };
            let before = event(at);
            let mut events = vec![before.clone()];
            let report = classifier.attribute(&mut events, &mut BTreeMap::new(), &inputs);
            assert!(report.proxy_error.is_none(), "{:?}", report.proxy_error);
            assert_eq!(report.proxy.unwrap()["recoveredEvents"], 0);
            assert!(super::super::super::unknown_provider(&events[0].provider));
            assert!(super::super::super::unknown_model(&events[0].model));
            assert_eq!(events[0].total, before.total);
            assert_eq!(events[0].delta, before.delta);
        }
    }

    #[test]
    fn request_log_replacement_is_detected_when_size_and_timestamp_match() {
        let dir = tempfile::tempdir().unwrap();
        let at = Local::now().timestamp_millis() - 1_000;
        let first = request(at);
        let mut second = first.clone();
        second.relay_id = "supplier-b".into();
        let log = dir.path().join("request-log.jsonl");
        fs::write(
            &log,
            format!("{}\n", serde_json::to_string(&first).unwrap()),
        )
        .unwrap();
        let mut classifier = Classifier::default();
        classifier.check_request_log(&log).unwrap();
        let before = fs::metadata(&log).unwrap();
        fs::rename(&log, dir.path().join("request-log-old.jsonl")).unwrap();
        fs::write(
            &log,
            format!("{}\n", serde_json::to_string(&second).unwrap()),
        )
        .unwrap();
        fs::File::options()
            .write(true)
            .open(&log)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(before.modified().unwrap()))
            .unwrap();
        let after = fs::metadata(&log).unwrap();
        assert_eq!(before.len(), after.len());
        assert_eq!(before.modified().unwrap(), after.modified().unwrap());

        classifier.check_request_log(&log).unwrap();
        assert_eq!(classifier.requests.reads, 2);
        assert_eq!(classifier.requests.records[0].relay_id, "supplier-b");
    }

    #[test]
    fn aggregate_route_requires_matching_request_before_using_upstream_model() {
        let at = Local::now().timestamp_millis() - 1_000;
        let mut routed = event(at);
        routed.model = "cpx-supplier-a/requested".into();
        let before = routed.delta;

        let mut events = vec![routed.clone()];
        let mut cache = RequestEvidenceCache::default();
        cache.attribute_with_actual_model(&mut events, &mut BTreeMap::new(), &BTreeMap::new(), true);
        assert_eq!(events[0].model, routed.model);
        assert!(super::super::super::unknown_provider(&events[0].provider));
        assert_eq!(events[0].delta, before);

        let mut unresolved = request(at);
        unresolved.model = routed.model.clone();
        unresolved.upstream_model.clear();
        cache.records.push(unresolved);
        let mut events = vec![routed.clone()];
        cache.attribute_with_actual_model(&mut events, &mut BTreeMap::new(), &BTreeMap::new(), true);
        assert_eq!(events[0].model, routed.model);
        assert!(super::super::super::unknown_provider(&events[0].provider));
        assert_eq!(events[0].delta, before);

        let mut mismatch = request(at);
        mismatch.model = "cpx-supplier-b/requested".into();
        mismatch.upstream_model = routed.model.clone();
        cache.records = vec![mismatch];
        let mut events = vec![routed.clone()];
        cache.attribute_with_actual_model(&mut events, &mut BTreeMap::new(), &BTreeMap::new(), true);
        assert_eq!(events[0].model, routed.model);
        assert!(super::super::super::unknown_provider(&events[0].provider));
        assert_eq!(events[0].delta, before);

        let mut actual = request(at);
        actual.model = routed.model.clone();
        actual.upstream_model = "cpx-native/model-b".into();
        actual.relay_id = "supplier-b".into();
        cache.records = vec![actual];
        let mut events = vec![routed];
        cache.attribute_with_actual_model(&mut events, &mut BTreeMap::new(), &BTreeMap::new(), true);
        assert_eq!(events[0].model, "cpx-native/model-b");
        assert_eq!(events[0].provider, "supplier-b");
        assert_eq!(events[0].delta, before);
    }

    #[test]
    fn unavailable_or_incomplete_log_does_not_change_counts_or_guess() {
        let dir = tempfile::tempdir().unwrap();
        let cutoff = Local::now().date_naive() - Days::new(1);
        let mut writer = provider_cache::ProviderCache::default();
        writer.open(dir.path(), cutoff).unwrap();
        drop(writer);
        let cache = provider_cache::ProviderCache::read_snapshot(dir.path(), cutoff).unwrap();
        let log = dir.path().join("request-log.jsonl");
        fs::write(&log, b"{").unwrap();
        let aliases = BTreeMap::new();
        let collisions = HashSet::new();
        let uncertain = HashMap::new();
        let endpoints = BTreeMap::new();
        let ready_sessions = HashSet::from([SESSION.to_owned()]);
        let inputs = Inputs {
            home: dir.path(),
            request_log: &log,
            cache: &cache,
            aliases: &aliases,
            collision_families: &collisions,
            inherited_uncertain: &uncertain,
            endpoints: &endpoints,
            endpoint_since: 0,
            ready_sessions: &ready_sessions,
        };
        let mut events = vec![event(Local::now().timestamp_millis())];
        let before = events[0].clone();
        let report = Classifier::default().attribute(&mut events, &mut BTreeMap::new(), &inputs);
        assert!(report.proxy_error.is_some());
        assert!(events[0] == before);
    }

    #[test]
    fn truncated_or_damaged_request_log_cannot_validate_cached_evidence() {
        let at = Local::now().timestamp_millis() - 1_000;
        let record = request(at);
        let valid = format!("{}\n", serde_json::to_string(&record).unwrap());
        let mut too_many = String::new();
        for index in 0..=MAX_REQUEST_EVIDENCE {
            too_many.push_str(&format!(r#"{{"id":"filler-{index}","timestampMs":1}}"#));
            too_many.push('\n');
        }
        let oversized = format!(
            "{{\"id\":\"oversized\",\"padding\":\"{}\"}}\n{valid}",
            "x".repeat(MAX_REQUEST_LOG_LINE)
        );
        let malformed = format!("{{invalid json\n{valid}");
        let invalid_fields = format!("{{\"id\":\"damaged\",\"timestampMs\":\"bad\"}}\n{valid}");

        for (name, contents) in [
            ("empty", String::new()),
            ("too many", too_many),
            ("oversized line", oversized),
            ("malformed line", malformed),
            ("invalid fields", invalid_fields),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let log = dir.path().join("request-log.jsonl");
            fs::write(&log, contents).unwrap();
            let cutoff = Local::now().date_naive() - Days::new(1);
            let mut writer = provider_cache::ProviderCache::default();
            writer.open(dir.path(), cutoff).unwrap();
            writer.capture_proxy(&[record.clone()]).unwrap();
            drop(writer);
            let cache = provider_cache::ProviderCache::read_snapshot(dir.path(), cutoff).unwrap();
            let aliases = BTreeMap::new();
            let collisions = HashSet::new();
            let uncertain = HashMap::new();
            let endpoints = BTreeMap::new();
            let ready_sessions = HashSet::from([SESSION.to_owned()]);
            let inputs = Inputs {
                home: dir.path(),
                request_log: &log,
                cache: &cache,
                aliases: &aliases,
                collision_families: &collisions,
                inherited_uncertain: &uncertain,
                endpoints: &endpoints,
                endpoint_since: 0,
                ready_sessions: &ready_sessions,
            };
            let mut classifier = Classifier::default();
            let before = event(at);
            let mut events = vec![before.clone()];
            let report = classifier.attribute(&mut events, &mut BTreeMap::new(), &inputs);
            assert!(report.proxy_error.is_some(), "{name} must be incomplete");
            assert!(
                report.proxy.is_none(),
                "{name} must not use cached evidence"
            );
            assert!(events[0] == before, "{name} changed the usage event");

            let reads = classifier.requests.reads;
            assert_eq!(reads, 1, "{name} was not checked");
            for _ in 0..3 {
                let mut events = vec![before.clone()];
                let report = classifier.attribute(&mut events, &mut BTreeMap::new(), &inputs);
                assert!(
                    report.proxy_error.is_some(),
                    "{name} must remain incomplete"
                );
                assert!(events[0] == before);
            }
            assert_eq!(
                classifier.requests.reads, reads,
                "{name} was repeatedly parsed"
            );
            fs::write(&log, &valid).unwrap();
            let mut events = vec![before.clone()];
            let report = classifier.attribute(&mut events, &mut BTreeMap::new(), &inputs);
            assert!(
                report.proxy_error.is_none(),
                "{name} did not recover: {:?}",
                report.proxy_error
            );
            assert_eq!(classifier.requests.reads, reads + 1);
            assert_eq!(events[0].provider, "supplier-a");
            assert_eq!(events[0].model, "model-a");
            assert_eq!(events[0].total, before.total);
            assert_eq!(events[0].delta, before.delta);
        }
    }

    #[test]
    fn request_log_limits_are_inclusive() {
        let dir = tempfile::tempdir().unwrap();
        let at = Local::now().timestamp_millis() - 1_000;
        let record = request(at);
        let prefix = "{\"id\":\"filler-0\",\"timestampMs\":1,\"padding\":\"";
        let suffix = "\"}\n";
        let mut contents = format!(
            "{prefix}{}{suffix}",
            "x".repeat(MAX_REQUEST_LOG_LINE - prefix.len() - suffix.len())
        );
        for index in 1..MAX_REQUEST_EVIDENCE - 1 {
            contents.push_str(&format!(r#"{{"id":"filler-{index}","timestampMs":1}}"#));
            contents.push('\n');
        }
        contents.push_str(&format!("{}\n", serde_json::to_string(&record).unwrap()));
        let log = dir.path().join("request-log.jsonl");
        fs::write(&log, contents).unwrap();
        let cutoff = Local::now().date_naive() - Days::new(1);
        let mut writer = provider_cache::ProviderCache::default();
        writer.open(dir.path(), cutoff).unwrap();
        writer.capture_proxy(&[record]).unwrap();
        drop(writer);
        let cache = provider_cache::ProviderCache::read_snapshot(dir.path(), cutoff).unwrap();
        let aliases = BTreeMap::new();
        let collisions = HashSet::new();
        let uncertain = HashMap::new();
        let endpoints = BTreeMap::new();
        let ready_sessions = HashSet::from([SESSION.to_owned()]);
        let inputs = Inputs {
            home: dir.path(),
            request_log: &log,
            cache: &cache,
            aliases: &aliases,
            collision_families: &collisions,
            inherited_uncertain: &uncertain,
            endpoints: &endpoints,
            endpoint_since: 0,
            ready_sessions: &ready_sessions,
        };
        let mut classifier = Classifier::default();
        let before = event(at);
        let mut events = vec![before.clone()];
        let report = classifier.attribute(&mut events, &mut BTreeMap::new(), &inputs);
        assert!(report.proxy_error.is_none(), "{:?}", report.proxy_error);
        assert_eq!(events[0].provider, "supplier-a");
        assert_eq!(events[0].model, "model-a");
        assert_eq!(events[0].total, before.total);
        assert_eq!(events[0].delta, before.delta);
    }

    #[test]
    fn incomplete_other_session_does_not_block_ready_session() {
        let dir = tempfile::tempdir().unwrap();
        let at = Local::now().timestamp_millis() - 1_000;
        let mut first = request(at);
        first.relay_name = "First supplier".into();
        let mut second = request(at);
        second.id = "request-b".into();
        second.thread_id = "00000000-0000-4000-8000-000000000011".into();
        second.relay_id = "supplier-b".into();
        let log = dir.path().join("request-log.jsonl");
        fs::write(
            &log,
            format!(
                "{}\n{}\n",
                serde_json::to_string(&first).unwrap(),
                serde_json::to_string(&second).unwrap()
            ),
        )
        .unwrap();
        let cutoff = Local::now().date_naive() - Days::new(1);
        let mut writer = provider_cache::ProviderCache::default();
        writer.open(dir.path(), cutoff).unwrap();
        writer.capture_proxy(&[first, second.clone()]).unwrap();
        drop(writer);
        let cache = provider_cache::ProviderCache::read_snapshot(dir.path(), cutoff).unwrap();
        let aliases = BTreeMap::new();
        let collisions = HashSet::new();
        let uncertain = HashMap::new();
        let endpoints = BTreeMap::new();
        let ready_sessions = HashSet::from([SESSION.to_owned()]);
        let inputs = Inputs {
            home: dir.path(),
            request_log: &log,
            cache: &cache,
            aliases: &aliases,
            collision_families: &collisions,
            inherited_uncertain: &uncertain,
            endpoints: &endpoints,
            endpoint_since: 0,
            ready_sessions: &ready_sessions,
        };
        let mut other = event(at);
        other.id = [8; 32];
        other.session = second.thread_id;
        let mut events = vec![event(at), other];
        let before: Vec<_> = events
            .iter()
            .map(|event| (event.total, event.delta))
            .collect();
        let result = Classifier::default().attribute(&mut events, &mut BTreeMap::new(), &inputs);
        assert_eq!(result.incomplete_sessions, 1);
        assert_eq!(events[0].provider, "supplier-a");
        assert_eq!(events[0].model, "model-a");
        assert!(super::super::super::unknown_provider(&events[1].provider));
        assert!(super::super::super::unknown_model(&events[1].model));
        assert_eq!(
            events
                .iter()
                .map(|event| (event.total, event.delta))
                .collect::<Vec<_>>(),
            before
        );
    }

    #[test]
    fn matching_considers_competing_events_in_the_ready_window() {
        let dir = tempfile::tempdir().unwrap();
        let at = Local::now().timestamp_millis() - 1_000;
        let record = request(at);
        let log = dir.path().join("request-log.jsonl");
        fs::write(
            &log,
            format!("{}\n", serde_json::to_string(&record).unwrap()),
        )
        .unwrap();
        let cutoff = Local::now().date_naive() - Days::new(1);
        let mut writer = provider_cache::ProviderCache::default();
        writer.open(dir.path(), cutoff).unwrap();
        writer.capture_proxy(&[record]).unwrap();
        drop(writer);
        let cache = provider_cache::ProviderCache::read_snapshot(dir.path(), cutoff).unwrap();
        let aliases = BTreeMap::new();
        let collisions = HashSet::new();
        let uncertain = HashMap::new();
        let endpoints = BTreeMap::new();
        let ready_sessions = HashSet::from([SESSION.to_owned()]);
        let inputs = Inputs {
            home: dir.path(),
            request_log: &log,
            cache: &cache,
            aliases: &aliases,
            collision_families: &collisions,
            inherited_uncertain: &uncertain,
            endpoints: &endpoints,
            endpoint_since: 0,
            ready_sessions: &ready_sessions,
        };
        let mut other = event(at + 1);
        other.id = [8; 32];
        let mut events = vec![event(at), other];
        let before: Vec<_> = events
            .iter()
            .map(|event| (event.total, event.delta))
            .collect();
        let result = Classifier::default().attribute(&mut events, &mut BTreeMap::new(), &inputs);
        assert!(result.proxy_error.is_none());
        assert_eq!(result.proxy.unwrap()["recoveredEvents"], 0);
        assert!(
            events
                .iter()
                .all(|event| super::super::super::unknown_provider(&event.provider))
        );
        assert_eq!(
            events
                .iter()
                .map(|event| (event.total, event.delta))
                .collect::<Vec<_>>(),
            before
        );
    }
}
