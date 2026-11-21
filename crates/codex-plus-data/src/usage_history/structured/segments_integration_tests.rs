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
    let (components, _, cyclic) = segments::causal_order(&mut events, &fixture.index.files);
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
    let (components, _, cyclic) = segments::causal_order(&mut events, &fixture.index.files);
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
fn local_branch_events_use_their_own_source_baseline() {
    let mut fixture = SegmentFixture::new();
    let shared = fixture.usage(1_000, 100, 100);
    fixture.write_segment(
        "rollout-a.jsonl",
        vec![
            fixture.context("shared"),
            shared.clone(),
            fixture.context("branch-a"),
            fixture.usage(2_000, 200, 100),
        ],
    );
    fixture.write_segment(
        "rollout-b.jsonl",
        vec![
            fixture.context("shared"),
            shared,
            fixture.context("branch-b"),
            fixture.usage(3_000, 250, 150),
        ],
    );
    let snapshot = fixture.snapshot();
    assert_segment_totals(&snapshot, 3, 350);
    let branch = snapshot
        .events
        .iter()
        .find(|event| event.turn == "branch-b")
        .unwrap();
    assert_eq!(branch.delta.input_tokens, 150);
    assert_eq!(branch.delta.cached_input_tokens, 30);
    assert!(branch.delta.cached_known);
}

#[test]
fn local_component_correction_follows_only_its_source_ancestors() {
    let mut fixture = SegmentFixture::new();
    let shared = fixture.usage(1_000, 100, 100);
    let mut corrected = fixture.usage(4_000, 200, 100);
    corrected["payload"]["info"]["total_token_usage"]["cached_input_tokens"] = json!(80);
    corrected["payload"]["info"]["total_token_usage"]["reasoning_output_tokens"] = json!(8);
    let mut later = fixture.usage(5_000, 300, 100);
    later["payload"]["info"]["total_token_usage"]["cached_input_tokens"] = json!(100);
    later["payload"]["info"]["total_token_usage"]["reasoning_output_tokens"] = json!(9);
    later["payload"]["info"]["last_token_usage"]["reasoning_output_tokens"] = json!(1);
    let mut first = fixture.usage(2_000, 200, 100);
    first["payload"]["info"]["total_token_usage"]["reasoning_output_tokens"] = json!(4);
    first["payload"]["info"]["last_token_usage"]["reasoning_output_tokens"] = json!(4);
    let mut shared = shared;
    shared["payload"]["info"]["total_token_usage"]["reasoning_output_tokens"] = json!(2);
    shared["payload"]["info"]["last_token_usage"]["reasoning_output_tokens"] = json!(2);
    let mut branch = fixture.usage(3_000, 250, 150);
    branch["payload"]["info"]["total_token_usage"]["reasoning_output_tokens"] = json!(5);
    branch["payload"]["info"]["last_token_usage"]["reasoning_output_tokens"] = json!(3);
    fixture.write_segment(
        "rollout-a.jsonl",
        vec![
            fixture.context("shared"),
            shared.clone(),
            fixture.context("branch-a"),
            first,
            corrected,
            later,
        ],
    );
    fixture.write_segment(
        "rollout-b.jsonl",
        vec![
            fixture.context("shared"),
            shared,
            fixture.context("branch-b"),
            branch,
        ],
    );
    let snapshot = fixture.snapshot();
    assert_eq!(
        snapshot
            .events
            .iter()
            .map(|event| event.delta.input_tokens)
            .sum::<u64>(),
        450
    );
    let event = |turn: &str, total: u64| {
        snapshot
            .events
            .iter()
            .find(|event| event.turn == turn && event.total.input_tokens == total)
            .unwrap()
    };
    for affected in [event("shared", 100), event("branch-a", 200)] {
        assert!(!affected.delta.cached_known);
        assert_eq!(affected.delta.reasoning_output_tokens, None);
    }
    let independent = event("branch-b", 250);
    assert!(independent.delta.cached_known);
    assert_eq!(independent.delta.cached_input_tokens, 30);
    assert_eq!(independent.delta.reasoning_output_tokens, Some(3));
    let later = event("branch-a", 300);
    assert!(later.delta.cached_known);
    assert_eq!(later.delta.cached_input_tokens, 20);
    assert_eq!(later.delta.reasoning_output_tokens, Some(1));
}

#[test]
fn local_component_correction_does_not_cross_a_cumulative_reset() {
    let mut fixture = SegmentFixture::new();
    let mut corrected = fixture.usage(4_000, 50, 50);
    corrected["payload"]["info"]["total_token_usage"]["cached_input_tokens"] = json!(30);
    let mut later = fixture.usage(5_000, 120, 70);
    later["payload"]["info"]["total_token_usage"]["cached_input_tokens"] = json!(44);
    fixture.write_segment(
        "rollout-reset.jsonl",
        vec![
            fixture.context("reset"),
            fixture.usage(1_000, 100, 100),
            fixture.usage(2_000, 200, 100),
            fixture.usage(3_000, 50, 50),
            corrected,
            later,
        ],
    );
    let snapshot = fixture.snapshot();
    assert_eq!(snapshot.events.len(), 4);
    assert!(snapshot.events[0].delta.cached_known);
    assert!(snapshot.events[1].delta.cached_known);
    assert!(!snapshot.events[2].delta.cached_known);
    assert!(snapshot.events[3].delta.cached_known);
    assert_eq!(
        snapshot
            .events
            .iter()
            .map(|event| event.delta.input_tokens)
            .sum::<u64>(),
        320
    );
}

#[test]
fn local_sparse_source_copy_keeps_the_complete_chain() {
    for sparse_first in [false, true] {
        let mut fixture = SegmentFixture::new();
        let first = fixture.usage(1_000, 100, 100);
        let middle = fixture.usage(2_000, 200, 100);
        let last = fixture.usage(3_000, 300, 100);
        let (full_name, sparse_name) = if sparse_first {
            ("rollout-z.jsonl", "rollout-a.jsonl")
        } else {
            ("rollout-a.jsonl", "rollout-z.jsonl")
        };
        fixture.write_segment(
            full_name,
            vec![
                fixture.context("chain"),
                first.clone(),
                middle,
                last.clone(),
            ],
        );
        fixture.write_segment(sparse_name, vec![fixture.context("chain"), first, last]);
        let snapshot = fixture.snapshot();
        assert_segment_totals(&snapshot, 3, 300);
        assert_eq!(snapshot.events[2].delta.input_tokens, 100);
    }
}

#[test]
fn local_unrelated_predecessors_recover_when_a_shared_chain_appears() {
    for sparse_first in [false, true] {
        let mut fixture = SegmentFixture::new();
        let first = fixture.usage(1_000, 100, 100);
        let middle = fixture.usage(2_000, 200, 100);
        let last = fixture.usage(3_000, 300, 100);
        let (sparse, other) = if sparse_first {
            ("rollout-a.jsonl", "rollout-z.jsonl")
        } else {
            ("rollout-z.jsonl", "rollout-a.jsonl")
        };
        fixture.write_segment(
            sparse,
            vec![
                fixture.context("first"),
                first.clone(),
                fixture.context("shared"),
                last.clone(),
            ],
        );
        fixture.write_segment(
            other,
            vec![
                fixture.context("middle"),
                middle.clone(),
                fixture.context("shared"),
                last.clone(),
            ],
        );
        let partial = fixture.snapshot();
        let shared = partial
            .events
            .iter()
            .find(|event| event.turn == "shared")
            .unwrap();
        assert!(shared.count_incomplete);
        assert!(!shared.count_uncertain);
        assert_eq!(segment_response(&partial)["complete"], false);

        fixture.write_segment(
            "rollout-middle.jsonl",
            vec![
                fixture.context("first"),
                first,
                fixture.context("middle"),
                middle,
                fixture.context("shared"),
                last,
            ],
        );
        let recovered = fixture.snapshot();
        assert_segment_totals(&recovered, 3, 300);
        fixture.index = Index {
            settings_path: Some(fixture._root.path().join("settings.json")),
            storage_dir: Some(fixture._root.path().join("cache")),
            ..Index::default()
        };
        assert_segment_totals(&fixture.snapshot(), 3, 300);
    }
}

#[test]
fn local_transitive_predecessor_work_is_bounded() {
    let mut predecessors = vec![Vec::new(); 2_001];
    for (position, before) in predecessors.iter_mut().enumerate().skip(1) {
        before.push(position - 1);
        if position > 2 {
            before.push(1);
        }
    }
    let unresolved = segments::prune_transitive_predecessors(&mut predecessors, 100);
    assert!(unresolved.iter().any(|&value| value));
    assert!(
        unresolved
            .iter()
            .enumerate()
            .filter(|(_, value)| **value)
            .all(|(position, _)| predecessors[position].len() == 2)
    );
}

#[test]
fn local_transitive_predecessors_match_all_small_dags() {
    const NODES: usize = 6;
    const EDGES: usize = NODES * (NODES - 1) / 2;
    for mask in 0..(1_u32 << EDGES) {
        let mut predecessors = vec![Vec::new(); NODES];
        let mut bit = 0;
        for (after, incoming) in predecessors.iter_mut().enumerate().skip(1) {
            for before in 0..after {
                if mask & (1 << bit) != 0 {
                    incoming.push(before);
                }
                bit += 1;
            }
        }
        let mut ancestors = [0_u32; NODES];
        for after in 0..NODES {
            for &before in &predecessors[after] {
                ancestors[after] |= ancestors[before] | (1 << before);
            }
        }
        let expected: Vec<Vec<usize>> = predecessors
            .iter()
            .map(|incoming| {
                incoming
                    .iter()
                    .copied()
                    .filter(|&before| {
                        !incoming
                            .iter()
                            .any(|&other| ancestors[other] & (1 << before) != 0)
                    })
                    .collect()
            })
            .collect();
        let unresolved = segments::prune_transitive_predecessors(&mut predecessors, usize::MAX);
        assert!(unresolved.iter().all(|&value| !value), "{mask}");
        assert_eq!(predecessors, expected, "{mask}");
    }
}

#[test]
fn local_sparse_shared_prefix_is_resolved_with_a_tight_work_budget() {
    let mut predecessors = vec![Vec::new(); 2_001];
    for (position, before) in predecessors.iter_mut().enumerate().skip(1) {
        before.push(position - 1);
        if position > 1 {
            before.push(0);
        }
    }
    let unresolved = segments::prune_transitive_predecessors(&mut predecessors, 0);
    assert!(unresolved.iter().all(|&value| !value));
    assert!(
        predecessors
            .iter()
            .enumerate()
            .skip(1)
            .all(|(position, before)| *before == [position - 1])
    );
}

#[test]
fn local_continuation_with_multiple_possible_sources_is_partial() {
    let mut fixture = SegmentFixture::new();
    fixture.write_segment(
        "rollout-a.jsonl",
        vec![fixture.context("a"), fixture.usage(1_000, 100, 100)],
    );
    fixture.write_segment(
        "rollout-c.jsonl",
        vec![fixture.context("c"), fixture.usage(2_000, 150, 150)],
    );
    fixture.write_segment(
        "rollout-continuation.jsonl",
        vec![
            json!({"type":"session_meta","payload":{
                "id":SEGMENT_SESSION,"history_base":{"end_ordinal_exclusive":1}
            }}),
            fixture.context("continuation"),
            fixture.usage(3_000, 200, 100),
            fixture.usage(4_000, 300, 100),
        ],
    );
    let snapshot = fixture.snapshot();
    let first = snapshot
        .events
        .iter()
        .find(|event| event.turn == "continuation" && event.total.input_tokens == 200)
        .unwrap();
    assert_eq!(first.delta.input_tokens, 100);
    assert!(first.count_incomplete);
    assert!(!first.count_uncertain);
    assert_eq!(segment_response(&snapshot)["complete"], false);
    assert_eq!(
        segment_response(&snapshot)["days"][0]["usage"]["inputTokens"],
        350
    );
    let later = snapshot
        .events
        .iter()
        .find(|event| event.turn == "continuation" && event.total.input_tokens == 300)
        .unwrap();
    assert_eq!(later.delta.input_tokens, 100);
}

#[test]
fn local_unanchored_continuation_does_not_replay_a_stale_last_request() {
    let mut fixture = SegmentFixture::new();
    let first = fixture.usage(1_000, 100, 100);
    let mut repeated = first.clone();
    repeated["timestamp"] = json!(fixture.at(2_000));
    fixture.write_segment(
        "rollout-a.jsonl",
        vec![fixture.context("first"), first.clone()],
    );
    fixture.write_segment(
        "rollout-b.jsonl",
        vec![
            json!({"type":"session_meta","payload":{
                "id":SEGMENT_SESSION,"history_base":{"end_ordinal_exclusive":1}
            }}),
            fixture.context("continued"),
            repeated.clone(),
        ],
    );
    let partial = fixture.snapshot();
    assert_eq!(segment_response(&partial)["complete"], false);
    assert_eq!(
        segment_response(&partial)["days"][0]["usage"]["inputTokens"],
        100
    );
    assert!(partial.events.iter().any(|event| event.count_incomplete));

    fixture.write_segment(
        "rollout-c.jsonl",
        vec![
            fixture.context("first"),
            first,
            fixture.context("continued"),
            repeated,
        ],
    );
    let recovered = fixture.snapshot();
    assert_segment_totals(&recovered, 1, 100);
}

#[test]
fn local_missing_cache_on_one_source_preserves_known_input_and_output() {
    let mut fixture = SegmentFixture::new();
    let mut missing = fixture.usage(1_000, 100, 100);
    missing["payload"]["info"]["total_token_usage"]
        .as_object_mut()
        .unwrap()
        .remove("cached_input_tokens");
    let known = fixture.usage(2_000, 100, 100);
    let mut shared = fixture.usage(3_000, 200, 100);
    shared["payload"]["info"]["last_token_usage"]
        .as_object_mut()
        .unwrap()
        .remove("cached_input_tokens");
    fixture.write_segment(
        "rollout-a.jsonl",
        vec![
            fixture.context("missing"),
            missing,
            fixture.context("shared"),
            shared.clone(),
        ],
    );
    fixture.write_segment(
        "rollout-b.jsonl",
        vec![
            fixture.context("known"),
            known,
            fixture.context("shared"),
            shared,
        ],
    );
    let snapshot = fixture.snapshot();
    let shared = snapshot
        .events
        .iter()
        .find(|event| event.turn == "shared")
        .unwrap();
    assert_eq!(shared.delta.input_tokens, 100);
    assert_eq!(shared.delta.output_tokens, 10);
    assert!(!shared.delta.cached_known);
    assert!(!shared.count_uncertain);
}

#[test]
fn local_missing_cache_on_a_zero_consumption_snapshot_is_not_a_correction() {
    let mut fixture = SegmentFixture::new();
    let first = fixture.usage(1_000, 100, 100);
    let mut missing = fixture.usage(2_000, 100, 100);
    missing["payload"]["info"]["total_token_usage"]
        .as_object_mut()
        .unwrap()
        .remove("cached_input_tokens");
    fixture.write_segment(
        "rollout-missing.jsonl",
        vec![fixture.context("chain"), first, missing],
    );
    let snapshot = fixture.snapshot();
    assert_eq!(snapshot.events.len(), 1);
    assert!(snapshot.events[0].delta.cached_known);
    assert_eq!(snapshot.events[0].delta.cached_input_tokens, 20);
}
