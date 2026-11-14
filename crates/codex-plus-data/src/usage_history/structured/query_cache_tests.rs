use super::*;

fn event(id: u8, session: &str, timestamp: &str, model: &str, input: u64) -> UsageEvent {
    let time = DateTime::parse_from_rfc3339(timestamp).unwrap();
    let tokens = Tokens::new(input, 0, 0);
    UsageEvent {
        id: [id; 32],
        session: session.into(),
        turn: String::new(),
        model: model.into(),
        timestamp: timestamp.into(),
        date: time.with_timezone(&Local).date_naive(),
        total: tokens,
        delta: tokens,
        has_file_baseline: true,
        last: Some(tokens),
        count_uncertain: false,
        count_incomplete: false,
        provider: "provider-a".into(),
        provider_source: None,
        role: "main".into(),
        inherited: false,
        legacy_identity: None,
        context_conflicts: ContextConflicts::default(),
    }
}

#[test]
fn cached_queries_keep_bounds_scope_and_filters_separate() {
    let first = event(
        2,
        "session-a",
        "2026-11-10T12:00:00Z",
        "model-a",
        10,
    );
    let second = event(
        1,
        "session-b",
        "2026-11-10T12:01:00Z",
        "model-b",
        20,
    );
    let start = DateTime::parse_from_rfc3339(&first.timestamp)
        .unwrap()
        .timestamp_millis();
    let end = DateTime::parse_from_rfc3339(&second.timestamp)
        .unwrap()
        .timestamp_millis();
    let snapshot = Snapshot {
        events: vec![first, second],
        count_cache_available: true,
        ..Snapshot::default()
    };
    let today = Local::now().date_naive();
    let all = Query::parse(&json!({"period":"all"}), today).unwrap();
    let model = Query::parse(&json!({"period":"all","model":"model-a"}), today).unwrap();
    let scoped = HashSet::from(["session-b".to_owned()]);

    let rows = snapshot.usage_days(&all, None, None);
    assert_eq!(rows[0]["usage"]["inputTokens"], 30);
    assert_eq!(snapshot.usage_days(&model, None, None)[0]["usage"]["inputTokens"], 10);
    assert_eq!(
        snapshot.usage_days(&all, Some((start, end)), None)[0]["usage"]["inputTokens"],
        10
    );
    assert_eq!(
        snapshot.usage_days(&all, None, Some(&scoped))[0]["usage"]["inputTokens"],
        20
    );
    assert_eq!(snapshot.usage_days(&all, None, None), rows);
    assert_eq!(snapshot.usage_cache.lock().unwrap().entries.len(), 4);
    assert_eq!(snapshot.ordered_events.get().unwrap(), &[1, 0]);
}

#[test]
fn query_cache_is_bounded_and_belongs_to_one_snapshot() {
    let snapshot = Snapshot {
        events: vec![event(
            1,
            "session-a",
            "2026-11-10T12:00:00Z",
            "model-a",
            10,
        )],
        count_cache_available: true,
        ..Snapshot::default()
    };
    for index in 0..MAX_CACHED_QUERIES + 3 {
        let query = Query::parse(
            &json!({"period":"all","search":format!("query-{index}")}),
            Local::now().date_naive(),
        )
        .unwrap();
        snapshot.usage_days(&query, None, None);
    }
    let cache = snapshot.usage_cache.lock().unwrap();
    assert_eq!(cache.entries.len(), MAX_CACHED_QUERIES);
    assert!(cache.bytes <= MAX_CACHED_QUERY_BYTES);
    assert!(!cache.entries.iter().any(|entry| entry.0.search == "query-0"));
    drop(cache);
    assert!(Snapshot::default().usage_cache.lock().unwrap().entries.is_empty());
}

#[test]
fn unavailable_evidence_cache_keeps_observed_counts_partial() {
    let observed = event(
        1,
        "session-a",
        "2026-11-10T12:00:00Z",
        "model-a",
        10,
    );
    let mut conflicted = event(
        2,
        "session-a",
        "2026-11-10T12:01:00Z",
        "model-b",
        20,
    );
    conflicted.count_uncertain = true;
    let snapshot = Snapshot {
        events: vec![observed, conflicted],
        ready: true,
        count_cache_available: false,
        ..Snapshot::default()
    };
    let query = Query::parse(&json!({"period":"all"}), Local::now().date_naive()).unwrap();
    let response = snapshot.response_page(&query, false, 1, None, None);
    assert_eq!(response["status"], "partial");
    assert_eq!(response["countStatus"]["complete"], false);
    assert_eq!(response["countStatus"]["uncertainEvents"], 1);
    assert_eq!(response["coverage"]["daily"], "unknown");
    assert_eq!(response["days"][0]["usage"]["inputTokens"], 10);
}
