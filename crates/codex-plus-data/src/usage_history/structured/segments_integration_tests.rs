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
