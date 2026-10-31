use codex_plus_data::usage_history::{clear_history, query_history};
use serde_json::{Value, json};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use tempfile::TempDir;
use uuid::Uuid;

static SERIAL: Mutex<()> = Mutex::new(());

struct Fixture {
    home: TempDir,
    session: String,
    turn: String,
    model: String,
    unit: u64,
    at: String,
    _serial: MutexGuard<'static, ()>,
}

impl Fixture {
    fn new() -> Self {
        let serial = SERIAL.lock().unwrap_or_else(|error| error.into_inner());
        let home = tempfile::tempdir().unwrap();
        fs::create_dir(home.path().join("sessions")).unwrap();
        Self {
            home,
            session: Uuid::new_v4().to_string(),
            turn: Uuid::new_v4().to_string(),
            model: format!("synthetic-{}", Uuid::new_v4()),
            unit: u64::from(Uuid::new_v4().as_bytes()[0]) + 11,
            at: chrono::Local::now().to_rfc3339(),
            _serial: serial,
        }
    }

    fn usage(&self, units: u64) -> Value {
        let amount = self.unit * units;
        json!({"input_tokens":7*amount,"cached_input_tokens":2*amount,
            "output_tokens":3*amount,"total_tokens":10*amount,"reasoning_output_tokens":amount})
    }

    fn expected(&self, units: u64) -> Value {
        let amount = self.unit * units;
        json!({"inputTokens":7*amount,"cachedInputTokens":2*amount,
            "outputTokens":3*amount,"totalTokens":10*amount,"reasoningOutputTokens":amount})
    }

    fn meta(&self) -> Value {
        json!({"type":"session_meta","timestamp":self.at,"payload":{"id":self.session}})
    }

    fn context(&self, model: &str) -> Value {
        json!({"type":"turn_context","timestamp":self.at,
            "payload":{"turn_id":self.turn,"model":model}})
    }

    fn count(&self, ordinal: u64, total: u64, last: Option<u64>) -> Value {
        let mut row = json!({"type":"event_msg","timestamp":self.at,"ordinal":ordinal,
            "payload":{"type":"token_count","turn_id":self.turn,
                "info":{"total_token_usage":self.usage(total)}}});
        if let Some(last) = last {
            row["payload"]["info"]["last_token_usage"] = self.usage(last);
        }
        row
    }

    fn segment(&self, events: &[Value]) -> Vec<u8> {
        let mut rows = vec![self.meta(), self.context(&self.model)];
        rows.extend_from_slice(events);
        jsonl(&rows)
    }

    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self
            .home
            .path()
            .join("sessions")
            .join(format!("rollout-{name}-{}.jsonl", self.session));
        fs::write(&path, bytes).unwrap();
        path
    }

    fn read(&self) -> Value {
        for _ in 0..64 {
            let response = query_history(
                self.home.path(),
                &json!({
                    "period":"all","includeSessionCursors":true
                }),
            );
            assert_ne!(response["status"], "unavailable", "{response}");
            if response["scan"]["discovering"] == false && response["scan"]["pendingFiles"] == 0 {
                return response;
            }
        }
        panic!("synthetic fixture did not finish within its read budget");
    }

    fn assert_usage(&self, response: &Value, units: u64) {
        assert_eq!(response["periods"]["all"]["usage"], self.expected(units));
        assert_eq!(
            response["machineSummary"]["totalTokens"],
            self.unit * units * 10
        );
        assert_eq!(response["codex"]["totalTokens"], self.unit * units * 10);
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        clear_history(self.home.path());
    }
}

fn jsonl(rows: &[Value]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for row in rows {
        serde_json::to_writer(&mut bytes, row).unwrap();
        bytes.push(b'\n');
    }
    bytes
}

#[test]
fn appended_half_line_is_not_counted_until_completed() {
    let fixture = Fixture::new();
    let path = fixture.write("append", &fixture.segment(&[fixture.count(1, 1, None)]));
    let before = fixture.read();
    assert_eq!(before["complete"], true);
    fixture.assert_usage(&before, 1);

    let next = jsonl(&[fixture.count(2, 3, None)]);
    let split = next.len() / 2;
    let mut file = OpenOptions::new().append(true).open(path).unwrap();
    file.write_all(&next[..split]).unwrap();
    file.flush().unwrap();
    let partial = fixture.read();
    assert_eq!(partial["complete"], false);
    fixture.assert_usage(&partial, 1);
    assert_eq!(partial["records"], before["records"]);

    file.write_all(&next[split..]).unwrap();
    file.flush().unwrap();
    let after = fixture.read();
    assert_eq!(after["complete"], true);
    fixture.assert_usage(&after, 3);
    assert_eq!(after["records"].as_array().unwrap().len(), 1);
    assert_eq!(after["records"][0]["usage"], fixture.expected(3));
    assert_eq!(fixture.read()["records"], after["records"]);
}

#[test]
fn truncation_discards_the_old_usage_and_partial_buffer() {
    let fixture = Fixture::new();
    let mut old = fixture.segment(&[fixture.count(1, 9, None)]);
    old.extend_from_slice(b"{\"type\":\"event_msg\",\"padding\":\"");
    old.extend(std::iter::repeat_n(b'x', 4096));
    let path = fixture.write("truncate", &old);
    let before = fixture.read();
    assert_eq!(before["complete"], false);
    fixture.assert_usage(&before, 9);

    let replacement = fixture.segment(&[fixture.count(2, 2, None)]);
    assert!(replacement.len() < old.len());
    fs::write(path, replacement).unwrap();
    let after = fixture.read();
    assert_eq!(after["complete"], true);
    fixture.assert_usage(&after, 2);
    assert_eq!(after["records"][0]["usage"], fixture.expected(2));
}

#[test]
fn same_length_rotation_uses_new_file_identity() {
    let fixture = Fixture::new();
    let mut old = fixture.segment(&[fixture.count(1, 1, None)]);
    let mut replacement = fixture.segment(&[fixture.count(2, 2, None)]);
    let size = old.len().max(replacement.len()) + 64;
    for bytes in [&mut old, &mut replacement] {
        bytes.resize(size - 1, b' ');
        bytes.push(b'\n');
    }
    let path = fixture.write("rotate", &old);
    fixture.assert_usage(&fixture.read(), 1);
    fs::rename(&path, fixture.home.path().join("retired.jsonl")).unwrap();
    fs::write(path, replacement).unwrap();
    let after = fixture.read();
    assert_eq!(after["complete"], true);
    fixture.assert_usage(&after, 2);
}

#[test]
fn filename_id_mismatch_uses_verified_session_metadata() {
    let fixture = Fixture::new();
    let filename_session = Uuid::new_v4().to_string();
    assert_ne!(filename_session, fixture.session);
    let path = fixture
        .home
        .path()
        .join("sessions")
        .join(format!("rollout-mismatch-{filename_session}.jsonl"));
    fs::write(path, fixture.segment(&[fixture.count(1, 3, None)])).unwrap();

    let result = fixture.read();
    assert_eq!(result["complete"], true);
    assert_eq!(result["scan"]["identityMismatchFiles"], 1);
    assert_eq!(result["records"][0]["sessionId"], fixture.session);
    fixture.assert_usage(&result, 3);

    let actual = query_history(
        fixture.home.path(),
        &json!({"period": "all", "sessionId": fixture.session}),
    );
    assert_eq!(actual["records"].as_array().unwrap().len(), 1);
    let filename = query_history(
        fixture.home.path(),
        &json!({"period": "all", "sessionId": filename_session}),
    );
    assert!(filename["records"].as_array().unwrap().is_empty());
}

#[test]
fn overlapping_segments_and_copies_count_each_observation_once() {
    let fixture = Fixture::new();
    let first = fixture.count(1, 1, None);
    let shared = fixture.count(2, 3, None);
    let third = fixture.count(3, 6, None);
    fixture.write("a", &fixture.segment(&[first, shared.clone()]));
    let second = fixture.segment(&[shared, third]);
    fixture.write("b", &second);
    fixture.write("copy", &second);
    let result = fixture.read();
    assert_eq!(result["complete"], true);
    assert_eq!(result["scan"]["discoveredFiles"], 3);
    assert_eq!(result["records"].as_array().unwrap().len(), 1);
    assert_eq!(result["periods"]["all"]["recordCount"], 1);
    assert_eq!(result["records"][0]["usage"], fixture.expected(6));
    fixture.assert_usage(&result, 6);
}

#[test]
fn cumulative_rollback_uses_last_usage_and_never_replays_missing_last() {
    let fixture = Fixture::new();
    fixture.write(
        "reset",
        &fixture.segment(&[
            fixture.count(1, 5, None),
            fixture.count(2, 2, Some(1)),
            fixture.count(3, 1, None),
        ]),
    );
    let result = fixture.read();
    assert_eq!(result["complete"], true);
    fixture.assert_usage(&result, 6);
    assert_eq!(result["records"][0]["usage"], fixture.expected(6));
    assert_eq!(result["sessionCursors"][0]["total"], fixture.expected(1));
}

#[test]
fn conflicting_models_for_one_observation_stay_unknown_without_double_counting() {
    let fixture = Fixture::new();
    let observation = fixture.count(1, 2, None);
    fixture.write("model-a", &fixture.segment(&[observation.clone()]));
    let other = format!("synthetic-{}", Uuid::new_v4());
    fixture.write(
        "model-b",
        &jsonl(&[fixture.meta(), fixture.context(&other), observation]),
    );
    let result = fixture.read();
    assert_eq!(result["complete"], false);
    assert_eq!(result["scan"]["truncated"], true);
    assert_eq!(result["records"].as_array().unwrap().len(), 1);
    assert_eq!(result["records"][0]["model"], Value::Null);
    assert_eq!(result["periods"]["all"]["unknownModelRecords"], 1);
    fixture.assert_usage(&result, 2);
}

#[test]
fn inherited_parent_usage_is_only_a_baseline_for_the_child() {
    let fixture = Fixture::new();
    let mut meta = fixture.meta();
    meta["payload"]["forked_from_id"] = json!(Uuid::new_v4().to_string());
    meta["payload"]["subagent_history_start_ordinal"] = json!(3);
    fixture.write(
        "child",
        &jsonl(&[
            meta,
            fixture.count(2, 3, None),
            fixture.context(&fixture.model),
            fixture.count(4, 5, None),
        ]),
    );
    let result = fixture.read();
    assert_eq!(result["complete"], true);
    fixture.assert_usage(&result, 2);
    assert_eq!(result["records"][0]["sessionId"], fixture.session);
    assert_eq!(result["sessionCursors"][0]["total"], fixture.expected(5));
}

#[test]
fn compressed_suffix_is_skipped_by_discovery_not_treated_as_plain_jsonl() {
    let fixture = Fixture::new();
    fixture.write("plain", &fixture.segment(&[fixture.count(1, 1, None)]));
    let skipped = fixture
        .home
        .path()
        .join("sessions")
        .join(format!("rollout-skipped-{}.jsonl.gz", Uuid::new_v4()));
    fs::write(skipped, fixture.segment(&[fixture.count(2, 8, None)])).unwrap();
    let result = fixture.read();
    assert_eq!(result["complete"], true);
    assert_eq!(result["scan"]["discoveredFiles"], 1);
    fixture.assert_usage(&result, 1);
}

#[test]
fn oversized_unrelated_line_is_skipped_and_reading_resumes() {
    let fixture = Fixture::new();
    let noise = json!({"type":"event_msg","payload":{
        "type":"synthetic_noise",
        "padding":format!("token_count {}", "x".repeat(300 * 1024))
    }});
    assert!(jsonl(&[noise.clone()]).len() > 256 * 1024);
    fixture.write(
        "long-noise",
        &fixture.segment(&[fixture.count(1, 1, None), noise, fixture.count(2, 3, None)]),
    );
    let result = fixture.read();
    assert_eq!(result["complete"], true);
    assert_eq!(result["scan"]["truncated"], false);
    assert_eq!(result["records"].as_array().unwrap().len(), 1);
    fixture.assert_usage(&result, 3);
    assert_eq!(result["records"][0]["usage"], fixture.expected(3));
}

#[test]
fn oversized_usage_line_cannot_claim_complete_or_add_unparsed_usage() {
    let fixture = Fixture::new();
    let mut oversized = fixture.count(2, 3, None);
    oversized["payload"]["padding"] = json!("x".repeat(300 * 1024));
    assert!(jsonl(&[oversized.clone()]).len() > 256 * 1024);
    let finished = json!({"type":"event_msg","timestamp":fixture.at,
        "payload":{"type":"task_complete","turn_id":fixture.turn}});
    fixture.write(
        "long-usage",
        &fixture.segment(&[fixture.count(1, 1, None), oversized, finished]),
    );
    let result = fixture.read();
    assert_eq!(result["complete"], false);
    assert_eq!(result["scan"]["truncated"], true);
    fixture.assert_usage(&result, 1);
    assert_eq!(result["records"][0]["status"], "completed");
    assert_eq!(result["sessionCursors"][0]["total"], fixture.expected(1));
    assert_eq!(fixture.read()["complete"], false);
}
