use super::*;

const SEGMENT_SESSION: &str = "00000000-0000-4000-8000-000000000071";
const OTHER_SEGMENT_SESSION: &str = "00000000-0000-4000-8000-000000000072";

struct SegmentFixture {
    _root: tempfile::TempDir,
    home: PathBuf,
    log: PathBuf,
    index: Index,
    start: i64,
}

impl SegmentFixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        fs::create_dir_all(home.join("sessions")).unwrap();
        let home = fs::canonicalize(home).unwrap();
        let settings = root.path().join("settings.json");
        let log = root.path().join("requests.jsonl");
        fs::write(
            &settings,
            br#"{"usageHistoryEnabled":true,"relayProfiles":[]}"#,
        )
        .unwrap();
        fs::write(&log, b"").unwrap();
        let index = Index {
            settings_path: Some(settings),
            storage_dir: Some(root.path().join("cache")),
            ..Index::default()
        };
        Self {
            _root: root,
            home,
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

    fn head(&self) -> Value {
        json!({"type":"session_meta","payload":{"id":SEGMENT_SESSION}})
    }

    fn context(&self, turn: &str) -> Value {
        json!({"type":"turn_context","timestamp":self.at(0),
            "payload":{"model":"gpt-6.1-sol","turn_id":turn}})
    }

    fn usage(&self, offset: i64, total: u64, last: u64) -> Value {
        let tokens = |input: u64| {
            json!({
                "input_tokens":input,"output_tokens":input / 10,
                "cached_input_tokens":input / 5,"reasoning_output_tokens":0
            })
        };
        json!({"type":"event_msg","timestamp":self.at(offset),
        "payload":{"type":"token_count","info":{
            "total_token_usage":tokens(total),"last_token_usage":tokens(last)
        }}})
    }

    fn write_segment(&self, name: &str, rows: Vec<Value>) -> PathBuf {
        let text = std::iter::once(self.head())
            .chain(rows)
            .map(|row| format!("{row}\n"))
            .collect::<String>();
        let file = self.home.join("sessions").join(name);
        fs::write(&file, text).unwrap();
        fs::canonicalize(file).unwrap()
    }

    fn snapshot(&mut self) -> Snapshot {
        self.index.update_with_request_log(&self.home, &self.log)
    }

    fn read_large_line(&mut self, body: &str) -> (PathBuf, FileState) {
        let file = self.home.join("sessions/rollout-large.jsonl");
        fs::write(
            &file,
            format!(
                "{}\n{}\n{body}\n{}\n",
                self.head(),
                self.context("large"),
                self.usage(1_000, 100, 100)
            ),
        )
        .unwrap();
        let file = fs::canonicalize(file).unwrap();
        let cutoff = Local::now().date_naive() - Days::days(RETENTION_DAYS - 1);
        let mut state = FileState::default();
        let mut budget = index_budget::Budget::new(std::iter::empty());
        assert!(state.read_with_budget(&file, cutoff, &mut budget).unwrap());
        (file, state)
    }
}

fn segment_response(snapshot: &Snapshot) -> Value {
    let query = Query::parse(&json!({}), Local::now().date_naive()).unwrap();
    snapshot.response_page(&query, false, 1, None, None)
}

fn assert_segment_totals(snapshot: &Snapshot, events: usize, input: u64) {
    assert_eq!(snapshot.errors, 0);
    assert_eq!(snapshot.issues, 0);
    assert_eq!(snapshot.events.len(), events);
    assert!(
        snapshot
            .events
            .iter()
            .all(|event| !event.count_uncertain && !event.count_incomplete)
    );
    assert_eq!(
        snapshot
            .events
            .iter()
            .map(|event| event.delta.input_tokens)
            .sum::<u64>(),
        input
    );
    assert_eq!(
        snapshot
            .events
            .iter()
            .map(|event| event.delta.output_tokens)
            .sum::<u64>(),
        input / 10
    );
    assert_eq!(segment_response(snapshot)["complete"], true);
}

#[test]
fn local_segments_without_shared_nodes_keep_independent_baselines() {
    let mut fixture = SegmentFixture::new();
    fixture.write_segment(
        "rollout-first.jsonl",
        vec![
            fixture.context("first"),
            fixture.usage(1_000, 100, 100),
            fixture.usage(2_000, 200, 100),
        ],
    );
    fixture.write_segment(
        "rollout-second.jsonl",
        vec![
            fixture.context("second"),
            fixture.usage(3_000, 300, 300),
            fixture.usage(4_000, 500, 200),
        ],
    );
    let snapshot = fixture.snapshot();
    assert_segment_totals(&snapshot, 4, 700);
    let mut events = snapshot.events.clone();
    let (components, cyclic) = segments::causal_order(&mut events, &fixture.index.files);
    assert!(!cyclic);
    assert_eq!(components.into_iter().collect::<HashSet<_>>().len(), 2);
}

#[test]
fn local_segments_identical_time_and_counts_keep_distinct_turns() {
    let mut fixture = SegmentFixture::new();
    for (name, turn) in [
        ("rollout-turn-a.jsonl", "turn-a"),
        ("rollout-turn-b.jsonl", "turn-b"),
    ] {
        fixture.write_segment(
            name,
            vec![fixture.context(turn), fixture.usage(1_000, 100, 100)],
        );
    }
    let snapshot = fixture.snapshot();
    assert_segment_totals(&snapshot, 2, 200);
    assert_ne!(snapshot.events[0].id, snapshot.events[1].id);
    assert_ne!(snapshot.events[0].turn, snapshot.events[1].turn);
    assert_eq!(snapshot.events[0].timestamp, snapshot.events[1].timestamp);
    assert!(snapshot.events[0].total == snapshot.events[1].total);
}

#[test]
fn local_segments_clock_rollback_keeps_shared_successor_counts() {
    let mut fixture = SegmentFixture::new();
    fixture.write_segment(
        "rollout-before.jsonl",
        vec![
            fixture.context("shared"),
            fixture.usage(3_000, 100, 100),
            fixture.usage(1_000, 200, 100),
        ],
    );
    fixture.write_segment(
        "rollout-after.jsonl",
        vec![
            fixture.context("shared"),
            fixture.usage(1_000, 200, 100),
            fixture.usage(2_000, 300, 100),
        ],
    );
    let snapshot = fixture.snapshot();
    assert_segment_totals(&snapshot, 3, 300);
    assert_eq!(
        snapshot
            .events
            .iter()
            .map(|event| event.total.input_tokens)
            .collect::<Vec<_>>(),
        vec![100, 200, 300]
    );
    let mut events = snapshot.events.clone();
    let (components, cyclic) = segments::causal_order(&mut events, &fixture.index.files);
    assert!(!cyclic);
    assert_eq!(components.into_iter().collect::<HashSet<_>>().len(), 1);
}

#[test]
fn local_segments_session_id_header_fallback_accepts_usage() {
    let fixture = SegmentFixture::new();
    let cutoff = Local::now().date_naive() - Days::days(RETENTION_DAYS - 1);
    let mut state = FileState::default();
    for row in [
        json!({"type":"session_meta","payload":{"session_id":SEGMENT_SESSION}}),
        fixture.context("fallback"),
        fixture.usage(1_000, 100, 100),
    ] {
        state.accept(&serde_json::to_vec(&row).unwrap(), cutoff);
    }
    assert_eq!(state.session, SEGMENT_SESSION);
    assert_eq!(state.issues, 0);
    assert!(!state.identity_conflict);
    assert_eq!(state.events.len(), 1);
    assert_eq!(state.events[0].session, SEGMENT_SESSION);
    assert_eq!(state.events[0].delta.input_tokens, 100);
}

#[test]
fn local_segments_conflicting_own_headers_exclude_entire_source() {
    let mut fixture = SegmentFixture::new();
    let conflict = fixture.write_segment(
        "rollout-conflict.jsonl",
        vec![
            fixture.context("conflict"),
            fixture.usage(1_000, 100, 100),
            json!({"type":"session_meta","payload":{"id":OTHER_SEGMENT_SESSION}}),
            fixture.usage(2_000, 200, 100),
        ],
    );
    fixture.write_segment(
        "rollout-valid.jsonl",
        vec![fixture.context("valid"), fixture.usage(3_000, 70, 70)],
    );
    let snapshot = fixture.snapshot();
    assert!(fixture.index.files[&conflict].identity_conflict);
    assert!(!fixture.index.files[&conflict].events.is_empty());
    assert_eq!(snapshot.errors, 0);
    assert!(snapshot.issues > 0);
    assert_eq!(snapshot.ignored, 1);
    assert_eq!(snapshot.events.len(), 1);
    assert_eq!(snapshot.events[0].delta.input_tokens, 70);
    assert_eq!(snapshot.events[0].turn, "valid");
    assert_eq!(segment_response(&snapshot)["status"], "partial");
}

#[test]
fn local_segments_irrelevant_huge_body_does_not_damage_usage() {
    let mut fixture = SegmentFixture::new();
    let body = format!(
        r#"{{"type":"response_item","payload":{{"type":"message","content":[{{"type":"output_text","text":"{}"}}]}}}}"#,
        "token_count ".repeat(MAX_LINE / 12 + 128),
    );
    assert!(body.len() > MAX_LINE);
    let (file, state) = fixture.read_large_line(&body);
    assert_eq!(state.issues, 0);
    assert!(!state.invalid_counts);
    assert!(state.activity_truncated);
    assert_eq!(state.events.len(), 1);
    fixture.index.files.insert(file, state);
    let snapshot = fixture.snapshot();
    assert_segment_totals(&snapshot, 1, 100);
    assert_eq!(segment_response(&snapshot)["activityComplete"], false);
}

#[test]
fn local_segments_relevant_huge_line_reports_partial_and_resumes_usage() {
    let mut fixture = SegmentFixture::new();
    let body = format!(
        r#"{{"type":"event_msg","timestamp":"{}","payload":{{"type":"token_count","info":{},"padding":"{}"}}}}"#,
        fixture.at(500),
        fixture.usage(500, 50, 50)["payload"]["info"],
        "x".repeat(MAX_LINE + 64),
    );
    assert!(body.len() > MAX_LINE);
    let (file, state) = fixture.read_large_line(&body);
    assert!(state.issues > 0);
    assert_eq!(state.events.len(), 1);
    assert_eq!(state.events[0].total.input_tokens, 100);
    fixture.index.files.insert(file, state);
    let snapshot = fixture.snapshot();
    let response = segment_response(&snapshot);
    assert_eq!(snapshot.errors, 0);
    assert!(snapshot.issues > 0);
    assert_eq!(response["complete"], false);
    assert_eq!(response["status"], "partial");
}

#[test]
fn local_component_correction_stays_inside_its_reset_segment() {
    let mut fixture = SegmentFixture::new();
    let first = fixture.usage(1_000, 100, 100);
    let second = fixture.usage(2_000, 200, 100);
    let reset = fixture.usage(3_000, 50, 50);
    let mut correction = reset.clone();
    correction["timestamp"] = json!(fixture.at(4_000));
    correction["payload"]["info"]["total_token_usage"]["cached_input_tokens"] = json!(30);
    correction["payload"]["info"]["total_token_usage"]["reasoning_output_tokens"] = json!(2);
    let mut later = fixture.usage(5_000, 120, 70);
    later["payload"]["info"]["total_token_usage"]["cached_input_tokens"] = json!(44);
    later["payload"]["info"]["total_token_usage"]["reasoning_output_tokens"] = json!(3);
    later["payload"]["info"]["last_token_usage"]["reasoning_output_tokens"] = json!(1);
    fixture.write_segment(
        "rollout-a.jsonl",
        vec![
            fixture.context("reset"),
            first,
            second,
            reset.clone(),
            correction.clone(),
            later.clone(),
        ],
    );
    fixture.write_segment(
        "rollout-b.jsonl",
        vec![fixture.context("reset"), reset, correction, later],
    );
    let snapshot = fixture.snapshot();
    assert_eq!(snapshot.events.len(), 4);
    assert_eq!(
        snapshot
            .events
            .iter()
            .map(|event| event.delta.input_tokens)
            .sum::<u64>(),
        320
    );
    assert_eq!(
        snapshot
            .events
            .iter()
            .map(|event| event.delta.output_tokens)
            .sum::<u64>(),
        32
    );
    assert!(snapshot.events[0].delta.cached_known);
    assert!(snapshot.events[1].delta.cached_known);
    assert!(!snapshot.events[2].delta.cached_known);
    assert_eq!(snapshot.events[2].delta.reasoning_output_tokens, None);
    assert!(snapshot.events[3].delta.cached_known);
    assert!(snapshot.events[3].delta.reasoning_output_tokens.is_some());
}

#[test]
fn local_component_correction_follows_continuation_baseline() {
    let mut fixture = SegmentFixture::new();
    let first = fixture.usage(1_000, 100, 100);
    let second = fixture.usage(2_000, 200, 100);
    let mut correction = second.clone();
    correction["timestamp"] = json!(fixture.at(3_000));
    correction["payload"]["info"]["total_token_usage"]["cached_input_tokens"] = json!(80);
    let mut later = fixture.usage(4_000, 300, 100);
    later["payload"]["info"]["total_token_usage"]["cached_input_tokens"] = json!(100);
    fixture.write_segment(
        "rollout-a.jsonl",
        vec![fixture.context("continuation"), first, second],
    );
    fixture.write_segment(
        "rollout-b.jsonl",
        vec![
            json!({"type":"session_meta","payload":{
                "id":SEGMENT_SESSION,"history_base":{"end_ordinal_exclusive":1}
            }}),
            fixture.context("continuation"),
            correction,
            later,
        ],
    );
    let snapshot = fixture.snapshot();
    assert_eq!(snapshot.events.len(), 3);
    assert_eq!(
        snapshot
            .events
            .iter()
            .map(|event| event.delta.input_tokens)
            .sum::<u64>(),
        300
    );
    assert!(!snapshot.events[0].delta.cached_known);
    assert!(!snapshot.events[1].delta.cached_known);
    assert!(snapshot.events[2].delta.cached_known);
    assert!(
        snapshot
            .events
            .iter()
            .all(|event| event.delta.reasoning_output_tokens.is_some())
    );
}

#[test]
fn local_component_correction_does_not_affect_an_independent_stream() {
    let mut fixture = SegmentFixture::new();
    let first = fixture.usage(1_000, 100, 100);
    let mut correction = first.clone();
    correction["timestamp"] = json!(fixture.at(2_000));
    correction["payload"]["info"]["total_token_usage"]["cached_input_tokens"] = json!(40);
    fixture.write_segment(
        "rollout-a.jsonl",
        vec![fixture.context("a"), first, correction],
    );
    fixture.write_segment(
        "rollout-b.jsonl",
        vec![
            fixture.context("b"),
            fixture.usage(3_000, 300, 300),
            fixture.usage(4_000, 400, 100),
        ],
    );
    let snapshot = fixture.snapshot();
    assert_eq!(snapshot.events.len(), 3);
    assert_eq!(
        snapshot
            .events
            .iter()
            .map(|event| event.delta.input_tokens)
            .sum::<u64>(),
        500
    );
    assert!(!snapshot.events[0].delta.cached_known);
    assert!(snapshot.events[1].delta.cached_known);
    assert!(snapshot.events[2].delta.cached_known);
}

#[test]
fn local_component_correction_is_detected_on_interleaved_source_branches() {
    let mut fixture = SegmentFixture::new();
    let shared = fixture.usage(1_000, 100, 100);
    let second = fixture.usage(2_000, 200, 100);
    let mut correction = second.clone();
    correction["timestamp"] = json!(fixture.at(4_000));
    correction["payload"]["info"]["total_token_usage"]["cached_input_tokens"] = json!(80);
    let branch = fixture.usage(3_000, 150, 50);
    fixture.write_segment(
        "rollout-a.jsonl",
        vec![
            fixture.context("branch"),
            shared.clone(),
            second,
            correction,
        ],
    );
    fixture.write_segment(
        "rollout-b.jsonl",
        vec![fixture.context("branch"), shared, branch],
    );
    let snapshot = fixture.snapshot();
    let second = snapshot
        .events
        .iter()
        .find(|event| event.total.input_tokens == 200 && event.delta.input_tokens > 0)
        .unwrap();
    let shared = snapshot
        .events
        .iter()
        .find(|event| event.total.input_tokens == 100)
        .unwrap();
    assert!(!second.delta.cached_known);
    assert!(!shared.delta.cached_known);
    assert_eq!(
        snapshot
            .events
            .iter()
            .map(|event| event.delta.input_tokens)
            .sum::<u64>(),
        250
    );
}

#[test]
fn local_invalid_count_only_blocks_the_next_identifiable_cumulative_delta() {
    let mut fixture = SegmentFixture::new();
    let first = fixture.usage(1_000, 100, 100);
    let mut invalid = fixture.usage(2_000, 200, 100);
    invalid["payload"]["info"]["total_token_usage"]
        .as_object_mut()
        .unwrap()
        .remove("input_tokens");
    let next = fixture.usage(3_000, 300, 100);
    let later = fixture.usage(4_000, 400, 100);
    fixture.write_segment(
        "rollout-a.jsonl",
        vec![
            fixture.context("gap"),
            first,
            invalid,
            next.clone(),
            later.clone(),
        ],
    );
    fixture.write_segment("rollout-b.jsonl", vec![fixture.context("gap"), next, later]);
    let snapshot = fixture.snapshot();
    assert!(snapshot.issues > 0);
    assert_eq!(snapshot.events.len(), 3);
    assert_eq!(
        snapshot
            .events
            .iter()
            .map(|event| event.count_incomplete)
            .collect::<Vec<_>>(),
        vec![false, true, false]
    );
    assert_eq!(
        snapshot
            .events
            .iter()
            .map(|event| event.delta.input_tokens)
            .sum::<u64>(),
        400
    );
    let response = segment_response(&snapshot);
    assert_eq!(response["complete"], false);
    assert_eq!(response["days"][0]["usage"]["totalTokens"], 220);
}

#[test]
fn local_invalid_count_followed_by_zero_delta_retains_gap_marker() {
    let mut fixture = SegmentFixture::new();
    let first = fixture.usage(1_000, 100, 100);
    let mut invalid = fixture.usage(2_000, 200, 100);
    invalid["payload"]["info"]["total_token_usage"]
        .as_object_mut()
        .unwrap()
        .remove("output_tokens");
    let mut unchanged = first.clone();
    unchanged["timestamp"] = json!(fixture.at(3_000));
    fixture.write_segment(
        "rollout-a.jsonl",
        vec![
            fixture.context("gap"),
            first,
            invalid,
            unchanged,
            fixture.usage(4_000, 200, 100),
        ],
    );
    let snapshot = fixture.snapshot();
    assert_eq!(snapshot.events.len(), 3);
    assert_eq!(snapshot.events[1].delta.total_tokens, 0);
    assert!(snapshot.events[1].count_incomplete);
    assert!(!snapshot.events[0].count_incomplete);
    assert!(!snapshot.events[2].count_incomplete);
    assert_eq!(segment_response(&snapshot)["complete"], false);
}

#[test]
fn local_unlocated_and_unclosed_count_gaps_remain_conservative() {
    for missing_time in [false, true] {
        let mut fixture = SegmentFixture::new();
        let first = fixture.usage(1_000, 100, 100);
        let mut invalid = fixture.usage(2_000, 200, 100);
        if missing_time {
            invalid["timestamp"] = Value::Null;
        } else {
            invalid["payload"]["info"]["total_token_usage"]
                .as_object_mut()
                .unwrap()
                .remove("input_tokens");
        }
        fixture.write_segment(
            "rollout-a.jsonl",
            vec![fixture.context("gap"), first, invalid],
        );
        let snapshot = fixture.snapshot();
        assert!(snapshot.issues > 0, "missing_time={missing_time}");
        assert_eq!(snapshot.events.len(), 1);
        assert!(snapshot.events[0].count_incomplete);
        assert_eq!(segment_response(&snapshot)["complete"], false);
    }
}

#[test]
fn local_missing_session_identity_keeps_later_counts_conservative() {
    let mut fixture = SegmentFixture::new();
    let text = [
        fixture.usage(1_000, 100, 100),
        fixture.head(),
        fixture.context("gap"),
        fixture.usage(2_000, 200, 100),
    ]
    .into_iter()
    .map(|row| format!("{row}\n"))
    .collect::<String>();
    fs::write(fixture.home.join("sessions/rollout-a.jsonl"), text).unwrap();
    let snapshot = fixture.snapshot();
    assert!(snapshot.issues > 0);
    assert_eq!(snapshot.events.len(), 1);
    assert!(snapshot.events[0].count_incomplete);
    assert_eq!(segment_response(&snapshot)["complete"], false);
}

#[test]
fn local_child_count_gap_requires_a_proven_inheritance_boundary() {
    for (start, ordinal, ambiguous_child) in [
        (Some(3), Some(5), false),
        (Some(3), None, false),
        (None, Some(5), true),
        (Some(3), Some(1), false),
    ] {
        let mut fixture = SegmentFixture::new();
        let parent_rows = [
            json!({"type":"session_meta","payload":{"id":OTHER_SEGMENT_SESSION}}),
            fixture.usage(1_000, 75, 75),
        ];
        let mut head = fixture.head();
        head["payload"]["forked_from_id"] = json!(OTHER_SEGMENT_SESSION);
        if let Some(start) = start {
            head["payload"]["subagent_history_start_ordinal"] = json!(start);
        }
        let mut first = fixture.usage(2_000, 100, 100);
        first["ordinal"] = json!(4);
        let mut invalid = fixture.usage(3_000, 200, 100);
        invalid["payload"]["info"]["total_token_usage"]
            .as_object_mut()
            .unwrap()
            .remove("input_tokens");
        if let Some(ordinal) = ordinal {
            invalid["ordinal"] = json!(ordinal);
        }
        let mut next = fixture.usage(4_000, 300, 100);
        next["ordinal"] = json!(6);
        for (name, rows) in [
            ("rollout-parent.jsonl", parent_rows.to_vec()),
            ("rollout-child.jsonl", vec![head, first, invalid, next]),
        ] {
            let text = rows
                .into_iter()
                .map(|row| format!("{row}\n"))
                .collect::<String>();
            fs::write(fixture.home.join("sessions").join(name), text).unwrap();
        }
        let snapshot = fixture.snapshot();
        let parent = snapshot
            .events
            .iter()
            .find(|event| event.session == OTHER_SEGMENT_SESSION)
            .unwrap();
        let child: Vec<_> = snapshot
            .events
            .iter()
            .filter(|event| event.session == SEGMENT_SESSION)
            .collect();
        assert_eq!(child.len(), 2, "start={start:?}, ordinal={ordinal:?}");
        assert!(!parent.count_incomplete);
        assert_eq!(
            child[0].count_incomplete, ambiguous_child,
            "start={start:?}, ordinal={ordinal:?}"
        );
        assert!(child[1].count_incomplete);
        assert!(snapshot.issues > 0);
        assert!(segment_response(&snapshot)["days"][0]["usage"]["inputTokens"]
            .as_u64()
            .unwrap()
            >= 75);
    }
}

#[test]
fn local_child_usage_requires_a_proven_inheritance_boundary() {
    for (start, ordinal, ambiguous) in [
        (Some(3), Some(4), false),
        (Some(3), None, true),
        (None, Some(4), true),
    ] {
        let mut fixture = SegmentFixture::new();
        let parent_rows = [
            json!({"type":"session_meta","payload":{"id":OTHER_SEGMENT_SESSION}}),
            fixture.usage(1_000, 75, 75),
        ];
        let mut head = fixture.head();
        head["payload"]["forked_from_id"] = json!(OTHER_SEGMENT_SESSION);
        if let Some(start) = start {
            head["payload"]["subagent_history_start_ordinal"] = json!(start);
        }
        let mut child_usage = fixture.usage(2_000, 100, 100);
        if let Some(ordinal) = ordinal {
            child_usage["ordinal"] = json!(ordinal);
        }
        for (name, rows) in [
            ("rollout-parent.jsonl", parent_rows.to_vec()),
            ("rollout-child.jsonl", vec![head, child_usage]),
        ] {
            let text = rows
                .into_iter()
                .map(|row| format!("{row}\n"))
                .collect::<String>();
            fs::write(fixture.home.join("sessions").join(name), text).unwrap();
        }
        let snapshot = fixture.snapshot();
        assert_eq!(snapshot.events.len(), 2);
        assert_eq!(snapshot.issues > 0, ambiguous);
        let parent = snapshot.events.iter().find(|event| event.session == OTHER_SEGMENT_SESSION).unwrap();
        let child = snapshot.events.iter().find(|event| event.session == SEGMENT_SESSION).unwrap();
        assert!(!parent.count_incomplete);
        assert_eq!(child.count_incomplete, ambiguous, "start={start:?}, ordinal={ordinal:?}");
        assert_eq!(
            segment_response(&snapshot)["days"][0]["usage"]["inputTokens"],
            if ambiguous { 75 } else { 175 }
        );
    }
}

#[test]
fn local_child_unknown_context_does_not_relabel_known_counts() {
    let mut fixture = SegmentFixture::new();
    let mut head = fixture.head();
    head["payload"]["forked_from_id"] = json!(OTHER_SEGMENT_SESSION);
    head["payload"]["subagent_history_start_ordinal"] = json!(3);
    let mut known = fixture.context("known");
    known["ordinal"] = json!(4);
    let mut ambiguous = fixture.context("ambiguous");
    ambiguous["payload"]["model"] = json!("unverified-model");
    let mut usage = fixture.usage(2_000, 100, 100);
    usage["ordinal"] = json!(6);
    let text = [head, known, ambiguous, usage]
        .into_iter()
        .map(|row| format!("{row}\n"))
        .collect::<String>();
    fs::write(fixture.home.join("sessions/rollout-child.jsonl"), text).unwrap();

    let snapshot = fixture.snapshot();
    assert!(snapshot.issues > 0);
    assert_eq!(snapshot.events.len(), 1);
    assert!(!snapshot.events[0].count_incomplete);
    assert!(snapshot.events[0].model.is_empty());
    assert!(snapshot.events[0].turn.is_empty());
    assert_eq!(segment_response(&snapshot)["days"][0]["usage"]["inputTokens"], 100);
}

#[test]
fn local_subagent_parent_relationship_alone_does_not_imply_copied_history() {
    let mut fixture = SegmentFixture::new();
    let mut head = fixture.head();
    head["payload"]["source"] =
        json!({"subagent":{"thread_spawn":{"parent_thread_id":OTHER_SEGMENT_SESSION}}});
    let text = [head, fixture.context("own"), fixture.usage(1_000, 100, 100)]
        .into_iter()
        .map(|row| format!("{row}\n"))
        .collect::<String>();
    fs::write(fixture.home.join("sessions/rollout-child.jsonl"), text).unwrap();

    let snapshot = fixture.snapshot();
    assert_segment_totals(&snapshot, 1, 100);
    assert_eq!(snapshot.events[0].model, "gpt-6.1-sol");
    assert_eq!(snapshot.events[0].role, "subagent");
}

#[test]
fn local_fork_reference_and_fresh_spawn_keep_own_usage() {
    for mode in ["reference", "reference-without-last", "mismatched-reference", "spawn", "conflicting-spawn"] {
        let mut fixture = SegmentFixture::new();
        let mut head = fixture.head();
        let mut context = fixture.context("own");
        let mut usage = fixture.usage(1_000, 500, 100);
        if mode.ends_with("spawn") {
            head["ordinal"] = json!(0);
            head["payload"]["parent_thread_id"] = json!(OTHER_SEGMENT_SESSION);
            head["payload"]["source"] = json!({"subagent":{"thread_spawn":{
                "parent_thread_id": if mode == "spawn" { OTHER_SEGMENT_SESSION } else { SEGMENT_SESSION }
            }}});
            context["ordinal"] = json!(1);
            usage["ordinal"] = json!(2);
        } else {
            head["ordinal"] = json!(100);
            head["payload"]["forked_from_id"] = json!(OTHER_SEGMENT_SESSION);
            head["payload"]["history_base"] = json!({
                "thread_id":OTHER_SEGMENT_SESSION,
                "end_ordinal_exclusive":if mode == "mismatched-reference" { 99 } else { 100 },
                "end_byte_offset":4096
            });
            context["ordinal"] = json!(101);
            usage["ordinal"] = json!(102);
            if mode == "reference-without-last" {
                usage["payload"]["info"]["last_token_usage"] = Value::Null;
            }
        }
        let text = [head, context, usage].into_iter()
            .map(|row| format!("{row}\n")).collect::<String>();
        fs::write(fixture.home.join("sessions/rollout-child.jsonl"), text).unwrap();
        let snapshot = fixture.snapshot();
        let event = &snapshot.events[0];
        assert_eq!(event.session, SEGMENT_SESSION, "{mode}");
        match mode {
            "mismatched-reference" | "conflicting-spawn" => assert!(event.count_incomplete, "{mode}"),
            "reference-without-last" => {
                assert!(event.count_incomplete);
                assert_eq!(segment_response(&snapshot)["days"], json!([]));
            }
            _ => {
                assert!(!event.count_incomplete && !event.count_uncertain, "{mode}");
                assert_eq!(event.model, "gpt-6.1-sol");
                assert_eq!(event.delta.input_tokens, 100);
            }
        }
    }
}

#[test]
fn local_unknown_inherited_owner_temporarily_blocks_parent_request_evidence() {
    let mut fixture = SegmentFixture::new();
    let mut parent = fixture.head();
    parent["payload"]["id"] = json!(OTHER_SEGMENT_SESSION);
    let parent_rows = [parent, fixture.context("parent"), fixture.usage(1_000, 100, 100)];
    let text = parent_rows
        .into_iter()
        .map(|row| format!("{row}\n"))
        .collect::<String>();
    fs::write(fixture.home.join("sessions/rollout-parent.jsonl"), text).unwrap();
    let at = fixture.start + 1_000;
    let request = RequestEvidence {
        id: "request-parent".into(),
        thread_id: OTHER_SEGMENT_SESSION.into(),
        model: "gpt-6.1-sol".into(),
        upstream_model: "gpt-6.1-sol".into(),
        relay_id: "supplier-a".into(),
        relay_name: "Supplier A".into(),
        success: true,
        state: "success".into(),
        status_code: 200,
        timestamp_ms: at - 10,
        started_at_ms: at - 100,
        attempts: Some(1),
        auto_retries: Some(0),
        usage: Some(RequestEvidenceUsage {
            input_tokens: Some(100),
            output_tokens: Some(10),
            cached_tokens: Some(20),
            cached_tokens_known: None,
        }),
        ..RequestEvidence::default()
    };
    fs::write(&fixture.log, format!("{}\n", serde_json::to_string(&request).unwrap())).unwrap();
    let initial = fixture.snapshot();
    assert_eq!(initial.events[0].provider, "supplier-a");

    let mut child = fixture.head();
    child["payload"]["forked_from_id"] = json!(OTHER_SEGMENT_SESSION);
    let child_path = fixture.home.join("sessions/rollout-child.jsonl");
    fs::write(
        &child_path,
        format!("{}\n{}\n", child, fixture.usage(2_000, 100, 100)),
    )
    .unwrap();
    let partial = fixture.snapshot();
    let parent = partial.events.iter().find(|row| row.session == OTHER_SEGMENT_SESSION).unwrap();
    assert!(unknown_provider(&parent.provider));
    assert!(!parent.count_incomplete);
    assert_eq!(segment_response(&partial)["days"][0]["usage"]["inputTokens"], 100);

    let mut invalid = fixture.usage(2_000, 100, 100);
    invalid["payload"]["info"]["total_token_usage"]
        .as_object_mut()
        .unwrap()
        .remove("input_tokens");
    fs::write(&child_path, format!("{child}\n{invalid}\n")).unwrap();
    let partial = fixture.snapshot();
    let parent = partial.events.iter().find(|row| row.session == OTHER_SEGMENT_SESSION).unwrap();
    assert!(unknown_provider(&parent.provider));
    assert!(!parent.count_incomplete);

    let distant = fixture.usage(2_000 + 2 * DELAYED_REQUEST_MATCH_WINDOW_MS, 100, 100);
    fs::write(&child_path, format!("{child}\n{distant}\n")).unwrap();
    let distant = fixture.snapshot();
    let parent = distant.events.iter().find(|row| row.session == OTHER_SEGMENT_SESSION).unwrap();
    assert_eq!(parent.provider, "supplier-a");

    fs::write(&child_path, format!("{child}\n")).unwrap();
    let repaired = fixture.snapshot();
    let parent = repaired.events.iter().find(|row| row.session == OTHER_SEGMENT_SESSION).unwrap();
    assert_eq!(parent.provider, "supplier-a");
}
