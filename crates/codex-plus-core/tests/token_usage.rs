use std::fs;
use std::io::Write;

use codex_plus_core::token_usage::{
    CapturedResponseUsage, RolloutEvent, RolloutTailer, TokenUsageQuery, UsageCounts,
    append_proxy_retry_attempt_at, append_proxy_usage_record_at, deduplicate_rollout_events,
    read_proxy_usage_records_from_path, read_rollout_events_from_roots,
    read_token_usage_events_from_sources,
};
use serde_json::json;

fn event(id: &str, timestamp: &str, model: &str) -> RolloutEvent {
    RolloutEvent {
        id: id.to_string(),
        session_id: "session-1".to_string(),
        timestamp: timestamp.to_string(),
        model: model.to_string(),
        usage: UsageCounts {
            input: 179_540,
            cached_input: 179_200,
            cache_write: 0,
            output: 506,
            reasoning: 130,
            total: 180_046,
        },
        totals: UsageCounts {
            input: 72_898_219,
            cached_input: 67_888_640,
            cache_write: 0,
            output: 210_561,
            reasoning: 56_791,
            total: 73_108_780,
        },
        source: "rollout".to_string(),
        status: "completed".to_string(),
        usage_missing: false,
        timestamp_ms: 0,
        response_id: String::new(),
    }
}

#[test]
fn proxy_ledger_round_trips_and_deduplicates_response_id() {
    let temp = tempfile::tempdir().unwrap();
    let ledger = temp.path().join("responses-usage.jsonl");
    let captured = CapturedResponseUsage {
        response_id: "resp-1".to_string(),
        model: "gpt-5.6-terra".to_string(),
        status: "completed".to_string(),
        usage: UsageCounts {
            input: 300_000,
            cached_input: 280_000,
            cache_write: 0,
            output: 900,
            reasoning: 250,
            total: 300_900,
        },
        usage_missing: false,
    };

    append_proxy_usage_record_at(&ledger, &captured, 1_721_600_000_000).unwrap();
    append_proxy_usage_record_at(&ledger, &captured, 1_721_600_000_100).unwrap();

    let records = read_proxy_usage_records_from_path(&ledger, 0, 100).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].response_id, "resp-1");
    assert_eq!(records[0].timestamp_ms, 1_721_600_000_000);
    assert_eq!(records[0].usage.input, 300_000);
}

#[test]
fn proxy_retry_attempt_records_status_without_persisting_request_contents() {
    let temp = tempfile::tempdir().unwrap();
    let ledger = temp.path().join("responses-usage.jsonl");
    let request = json!({
        "model": "test-model",
        "input": "private prompt",
        "api_key": "must-not-be-written"
    });

    append_proxy_retry_attempt_at(&ledger, &request, 2, 1_721_600_000_000).unwrap();

    let records = read_proxy_usage_records_from_path(&ledger, 0, 100).unwrap();
    let raw = fs::read_to_string(&ledger).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].model, "test-model");
    assert_eq!(records[0].status, "retry");
    assert!(records[0].usage_missing);
    assert!(!raw.contains("private prompt"));
    assert!(!raw.contains("must-not-be-written"));
}

#[test]
fn proxy_ledger_rotates_at_twenty_megabytes_and_keeps_three_archives() {
    let temp = tempfile::tempdir().unwrap();
    let ledger = temp.path().join("responses-usage.jsonl");
    let captured = |index: u64| CapturedResponseUsage {
        response_id: format!("resp-{index}"),
        model: "test-model".to_string(),
        status: "completed".to_string(),
        usage: UsageCounts {
            input: index,
            total: index,
            ..UsageCounts::default()
        },
        usage_missing: false,
    };

    for index in 1..=4 {
        append_proxy_usage_record_at(&ledger, &captured(index), index).unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(&ledger)
            .unwrap()
            .set_len(20 * 1024 * 1024)
            .unwrap();
    }
    append_proxy_usage_record_at(&ledger, &captured(5), 5).unwrap();

    assert!(fs::metadata(&ledger).unwrap().len() < 1024 * 1024);
    for index in 1..=3 {
        assert!(fs::metadata(format!("{}.{}", ledger.display(), index)).is_ok());
    }
    assert!(fs::metadata(format!("{}.4", ledger.display())).is_err());
}

#[test]
fn combined_reader_pages_past_the_proxy_ledger_limit() {
    let temp = tempfile::tempdir().unwrap();
    let ledger = temp.path().join("responses-usage.jsonl");
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;

    let mut file = std::io::BufWriter::new(std::fs::File::create(&ledger).unwrap());
    for index in 1..=100_001_u64 {
        let record = RolloutEvent {
            id: format!("event-{index}"),
            session_id: String::new(),
            timestamp: (now_ms + index).to_string(),
            model: "test-model".to_string(),
            usage: UsageCounts {
                input: 1,
                total: 1,
                ..UsageCounts::default()
            },
            totals: UsageCounts::default(),
            source: "proxy".to_string(),
            status: "completed".to_string(),
            usage_missing: false,
            timestamp_ms: now_ms + index,
            response_id: format!("resp-{index}"),
        };
        serde_json::to_writer(&mut file, &record).unwrap();
        writeln!(file).unwrap();
    }
    file.flush().unwrap();

    let result = read_token_usage_events_from_sources(
        &[],
        &ledger,
        &TokenUsageQuery {
            days: 1,
            limit: 2,
            proxy_since_ms: now_ms + 100_000,
            ..TokenUsageQuery::default()
        },
    );

    assert_eq!(result.events.len(), 1);
    assert_eq!(result.events[0].response_id, "resp-100001");
    assert_eq!(result.proxy_next_since_ms, now_ms + 100_001);
}

#[test]
fn incremental_reader_skips_rollouts_and_resumes_from_proxy_offset() {
    let temp = tempfile::tempdir().unwrap();
    let rollout = temp.path().join("rollout-session.jsonl");
    let ledger = temp.path().join("responses-usage.jsonl");
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    fs::write(
        &rollout,
        [
            json!({"timestamp":"2026-07-28T00:00:00.000Z","payload":{"type":"session_meta","session_id":"session-1"}}),
            json!({"timestamp":"2026-07-28T00:00:00.000Z","payload":{"type":"turn_context","model":"rollout-model"}}),
            json!({"timestamp":"2026-07-28T00:00:01.000Z","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":100,"output_tokens":10},"total_token_usage":{"input_tokens":100,"output_tokens":10}}}}),
        ]
        .iter()
        .map(serde_json::Value::to_string)
        .collect::<Vec<_>>()
        .join("\n"),
    )
    .unwrap();
    append_proxy_usage_record_at(
        &ledger,
        &CapturedResponseUsage {
            response_id: "resp-first".to_string(),
            model: "proxy-model".to_string(),
            status: "completed".to_string(),
            usage: UsageCounts {
                input: 1,
                total: 1,
                ..UsageCounts::default()
            },
            usage_missing: false,
        },
        now_ms + 1,
    )
    .unwrap();
    let first_offset = fs::metadata(&ledger).unwrap().len();
    append_proxy_usage_record_at(
        &ledger,
        &CapturedResponseUsage {
            response_id: "resp-second".to_string(),
            model: "proxy-model".to_string(),
            status: "completed".to_string(),
            usage: UsageCounts {
                input: 2,
                total: 2,
                ..UsageCounts::default()
            },
            usage_missing: false,
        },
        now_ms + 2,
    )
    .unwrap();
    let query: TokenUsageQuery = serde_json::from_value(json!({
        "days": 1,
        "limit": 100,
        "includeRollout": false,
        "proxyOffset": first_offset
    }))
    .unwrap();

    let result =
        read_token_usage_events_from_sources(&[temp.path().to_path_buf()], &ledger, &query);
    let value = serde_json::to_value(&result).unwrap();

    assert_eq!(result.events.len(), 1);
    assert_eq!(result.events[0].response_id, "resp-second");
    assert_eq!(
        value["proxyNextOffset"],
        fs::metadata(&ledger).unwrap().len()
    );
    assert_eq!(value["proxyReset"], false);
}

#[test]
fn incremental_reader_does_not_advance_past_an_incomplete_jsonl_record() {
    let temp = tempfile::tempdir().unwrap();
    let ledger = temp.path().join("responses-usage.jsonl");
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    append_proxy_usage_record_at(
        &ledger,
        &CapturedResponseUsage {
            response_id: "resp-complete".to_string(),
            model: "test-model".to_string(),
            status: "completed".to_string(),
            usage: UsageCounts {
                input: 1,
                total: 1,
                ..UsageCounts::default()
            },
            usage_missing: false,
        },
        now_ms,
    )
    .unwrap();
    let complete_offset = fs::metadata(&ledger).unwrap().len();
    let pending = serde_json::to_string(&RolloutEvent {
        id: "pending-event".to_string(),
        session_id: String::new(),
        timestamp: (now_ms + 1).to_string(),
        model: "test-model".to_string(),
        usage: UsageCounts {
            input: 2,
            total: 2,
            ..UsageCounts::default()
        },
        totals: UsageCounts::default(),
        source: "proxy".to_string(),
        status: "completed".to_string(),
        usage_missing: false,
        timestamp_ms: now_ms + 1,
        response_id: "resp-pending".to_string(),
    })
    .unwrap();
    let split = pending.len() / 2;
    let mut file = fs::OpenOptions::new().append(true).open(&ledger).unwrap();
    file.write_all(&pending.as_bytes()[..split]).unwrap();
    file.flush().unwrap();

    let first = read_token_usage_events_from_sources(
        &[],
        &ledger,
        &serde_json::from_value(json!({
            "includeRollout": false,
            "proxyOffset": complete_offset,
            "limit": 100
        }))
        .unwrap(),
    );
    assert!(first.events.is_empty());
    assert_eq!(first.proxy_next_offset, complete_offset);

    file.write_all(&pending.as_bytes()[split..]).unwrap();
    file.write_all(b"\n").unwrap();
    file.flush().unwrap();
    let second = read_token_usage_events_from_sources(
        &[],
        &ledger,
        &serde_json::from_value(json!({
            "includeRollout": false,
            "proxyOffset": complete_offset,
            "limit": 100
        }))
        .unwrap(),
    );
    assert_eq!(second.events.len(), 1);
    assert_eq!(second.events[0].response_id, "resp-pending");
}

#[test]
fn incremental_reader_resets_when_ledger_generation_changes() {
    let temp = tempfile::tempdir().unwrap();
    let ledger = temp.path().join("responses-usage.jsonl");
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let captured = |response_id: &str, input: u64| CapturedResponseUsage {
        response_id: response_id.to_string(),
        model: "test-model".to_string(),
        status: "completed".to_string(),
        usage: UsageCounts {
            input,
            total: input,
            ..UsageCounts::default()
        },
        usage_missing: false,
    };
    append_proxy_usage_record_at(&ledger, &captured("old", 1), now_ms).unwrap();
    let original = read_token_usage_events_from_sources(
        &[],
        &ledger,
        &serde_json::from_value(json!({ "includeRollout": false, "limit": 100 })).unwrap(),
    );
    let original_generation = original.proxy_generation.clone();
    let original_offset = original.proxy_next_offset;

    fs::remove_file(&ledger).unwrap();
    append_proxy_usage_record_at(&ledger, &captured("new-1", 20), now_ms + 1).unwrap();
    append_proxy_usage_record_at(&ledger, &captured("new-2", 30), now_ms + 2).unwrap();
    assert!(fs::metadata(&ledger).unwrap().len() > original_offset);

    let replaced = read_token_usage_events_from_sources(
        &[],
        &ledger,
        &serde_json::from_value(json!({
            "includeRollout": false,
            "proxyOffset": original_offset,
            "proxyGeneration": original_generation,
            "limit": 100
        }))
        .unwrap(),
    );
    assert!(replaced.proxy_reset);
    assert_ne!(replaced.proxy_generation, original_generation);
    assert_eq!(
        replaced
            .events
            .iter()
            .map(|event| event.response_id.as_str())
            .collect::<Vec<_>>(),
        vec!["new-1", "new-2"]
    );
}

#[test]
fn combined_reader_uses_rollout_before_first_proxy_record_and_proxy_after_it() {
    let temp = tempfile::tempdir().unwrap();
    let rollout = temp.path().join("rollout-session.jsonl");
    let ledger = temp.path().join("responses-usage.jsonl");
    fs::write(
        &rollout,
        [
            json!({"timestamp":"2026-07-23T00:00:00.000Z","payload":{"type":"session_meta","session_id":"session-1"}}),
            json!({"timestamp":"2026-07-23T00:00:00.000Z","payload":{"type":"turn_context","model":"gpt-5.6-terra"}}),
            json!({"timestamp":"2026-07-23T00:00:01.000Z","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":100,"output_tokens":10},"total_token_usage":{"input_tokens":100,"output_tokens":10}}}}),
            json!({"timestamp":"2026-07-23T00:00:03.000Z","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":200,"output_tokens":20},"total_token_usage":{"input_tokens":300,"output_tokens":30}}}}),
        ]
        .iter()
        .map(serde_json::Value::to_string)
        .collect::<Vec<_>>()
        .join("\n"),
    )
    .unwrap();
    append_proxy_usage_record_at(
        &ledger,
        &CapturedResponseUsage {
            response_id: "resp-after-cutover".to_string(),
            model: "gpt-5.6-terra".to_string(),
            status: "completed".to_string(),
            usage: UsageCounts {
                input: 200,
                output: 20,
                total: 220,
                ..UsageCounts::default()
            },
            usage_missing: false,
        },
        1_784_764_802_000,
    )
    .unwrap();

    let result = read_token_usage_events_from_sources(
        &[temp.path().to_path_buf()],
        &ledger,
        &TokenUsageQuery {
            days: 31,
            limit: 100,
            ..TokenUsageQuery::default()
        },
    );

    assert_eq!(result.events.len(), 2);
    assert_eq!(result.events[0].source, "rollout");
    assert_eq!(result.events[0].usage.input, 100);
    assert_eq!(result.events[1].source, "proxy");
    assert_eq!(result.events[1].response_id, "resp-after-cutover");
    assert_eq!(result.proxy_enabled_at_ms, 1_784_764_802_000);
}

#[test]
fn combined_reader_keeps_unmatched_rollout_events_during_proxy_gaps() {
    let temp = tempfile::tempdir().unwrap();
    let rollout = temp.path().join("rollout-session.jsonl");
    let ledger = temp.path().join("responses-usage.jsonl");
    fs::write(
        &rollout,
        [
            json!({"timestamp":"2026-07-23T00:00:00.000Z","payload":{"type":"session_meta","session_id":"session-1"}}),
            json!({"timestamp":"2026-07-23T00:00:00.000Z","payload":{"type":"turn_context","model":"gpt-5.6-terra"}}),
            json!({"timestamp":"2026-07-23T00:00:01.000Z","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":100,"output_tokens":10},"total_token_usage":{"input_tokens":100,"output_tokens":10}}}}),
            json!({"timestamp":"2026-07-23T00:00:03.000Z","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":200,"output_tokens":20},"total_token_usage":{"input_tokens":300,"output_tokens":30}}}}),
            json!({"timestamp":"2026-07-23T00:00:05.000Z","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":300,"output_tokens":30},"total_token_usage":{"input_tokens":600,"output_tokens":60}}}}),
        ]
        .iter()
        .map(serde_json::Value::to_string)
        .collect::<Vec<_>>()
        .join("\n"),
    )
    .unwrap();
    append_proxy_usage_record_at(
        &ledger,
        &CapturedResponseUsage {
            response_id: "resp-middle".to_string(),
            model: "gpt-5.6-terra".to_string(),
            status: "completed".to_string(),
            usage: UsageCounts {
                input: 200,
                output: 20,
                total: 220,
                ..UsageCounts::default()
            },
            usage_missing: false,
        },
        1_784_764_803_000,
    )
    .unwrap();

    let result = read_token_usage_events_from_sources(
        &[temp.path().to_path_buf()],
        &ledger,
        &TokenUsageQuery {
            days: 31,
            limit: 100,
            ..TokenUsageQuery::default()
        },
    );

    assert_eq!(result.events.len(), 3);
    assert_eq!(result.events[0].usage.input, 100);
    assert_eq!(result.events[1].source, "proxy");
    assert_eq!(result.events[1].usage.input, 200);
    assert_eq!(result.events[2].source, "rollout");
    assert_eq!(result.events[2].usage.input, 300);
}

#[test]
fn combined_reader_matches_a_uniquely_renamed_model_without_model_aliases() {
    let temp = tempfile::tempdir().unwrap();
    let rollout = temp.path().join("rollout-session.jsonl");
    let ledger = temp.path().join("responses-usage.jsonl");
    fs::write(
        &rollout,
        [
            json!({"timestamp":"2026-07-23T00:00:00.000Z","payload":{"type":"session_meta","session_id":"session-1"}}),
            json!({"timestamp":"2026-07-23T00:00:00.000Z","payload":{"type":"turn_context","model":"local-model-name"}}),
            json!({"timestamp":"2026-07-23T00:00:01.000Z","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":200,"cached_input_tokens":100,"output_tokens":20},"total_token_usage":{"input_tokens":200,"cached_input_tokens":100,"output_tokens":20}}}}),
        ]
        .iter()
        .map(serde_json::Value::to_string)
        .collect::<Vec<_>>()
        .join("\n"),
    )
    .unwrap();
    append_proxy_usage_record_at(
        &ledger,
        &CapturedResponseUsage {
            response_id: "resp-renamed".to_string(),
            model: "provider-model-name".to_string(),
            status: "completed".to_string(),
            usage: UsageCounts {
                input: 200,
                cached_input: 100,
                output: 20,
                total: 220,
                ..UsageCounts::default()
            },
            usage_missing: false,
        },
        1_784_764_801_500,
    )
    .unwrap();

    let result = read_token_usage_events_from_sources(
        &[temp.path().to_path_buf()],
        &ledger,
        &TokenUsageQuery {
            days: 31,
            limit: 100,
            ..TokenUsageQuery::default()
        },
    );

    assert_eq!(result.events.len(), 1);
    assert_eq!(result.events[0].source, "proxy");
    assert_eq!(result.events[0].response_id, "resp-renamed");
    assert!(!include_str!("../src/token_usage.rs").contains("gpt-5.6"));
}

#[test]
fn same_session_watermark_ignores_model_and_prefers_known_model() {
    let original = event("original", "2026-07-22T00:23:39.106Z", "gpt-5.6-terra");
    let replay = event("replay", "2026-07-22T05:13:32.738Z", "Unknown");

    let deduplicated = deduplicate_rollout_events(vec![original.clone(), replay]);

    assert_eq!(deduplicated, vec![original]);
}

#[test]
fn rollout_reader_deduplicates_child_history_across_models() {
    let temp = tempfile::tempdir().unwrap();
    let original = temp.path().join("rollout-original.jsonl");
    let replay = temp.path().join("rollout-child.jsonl");
    let usage = json!({
        "input_tokens": 179540,
        "cached_input_tokens": 179200,
        "cache_creation_input_tokens": 128,
        "output_tokens": 506,
        "reasoning_output_tokens": 130,
        "total_tokens": 180046
    });
    let totals = json!({
        "input_tokens": 72898219,
        "cached_input_tokens": 67888640,
        "cache_creation_input_tokens": 128,
        "output_tokens": 210561,
        "reasoning_output_tokens": 56791,
        "total_tokens": 73108780
    });
    let token_count = |timestamp: &str| {
        json!({
            "timestamp": timestamp,
            "payload": {
                "type": "token_count",
                "info": {
                    "last_token_usage": usage,
                    "total_token_usage": totals
                }
            }
        })
    };
    fs::write(
        &original,
        [
            json!({"timestamp":"2026-07-22T00:23:24.431Z","payload":{"type":"session_meta","session_id":"session-1","id":"session-1"}}),
            json!({"timestamp":"2026-07-22T00:23:24.431Z","payload":{"type":"turn_context","turn_id":"turn-1","model":"gpt-5.6-terra"}}),
            token_count("2026-07-22T00:23:39.106Z"),
        ]
        .iter()
        .map(serde_json::Value::to_string)
        .collect::<Vec<_>>()
        .join("\n"),
    )
    .unwrap();
    fs::write(
        &replay,
        [
            json!({"timestamp":"2026-07-22T05:13:32.738Z","payload":{"type":"session_meta","session_id":"session-1","id":"child-1","source":{"subagent":{}}}}),
            token_count("2026-07-22T05:13:32.738Z"),
        ]
        .iter()
        .map(serde_json::Value::to_string)
        .collect::<Vec<_>>()
        .join("\n"),
    )
    .unwrap();

    let result = read_rollout_events_from_roots(
        &[temp.path().to_path_buf()],
        &TokenUsageQuery {
            days: 7,
            limit: 100,
            ..TokenUsageQuery::default()
        },
    );

    assert!(result.warnings.is_empty());
    assert_eq!(result.events.len(), 1);
    assert_eq!(result.events[0].model, "gpt-5.6-terra");
    assert_eq!(result.events[0].usage.cache_write, 128);
    assert!(
        serde_json::to_value(&result.events[0])
            .unwrap()
            .get("sourcePath")
            .is_none()
    );
}

#[test]
fn rollout_tailer_reads_only_events_appended_since_the_previous_scan() {
    let temp = tempfile::tempdir().unwrap();
    let rollout = temp.path().join("rollout-session.jsonl");
    let initial = [
        json!({"timestamp":"2026-07-28T00:00:00.000Z","payload":{"type":"session_meta","session_id":"session-1"}}),
        json!({"timestamp":"2026-07-28T00:00:00.000Z","payload":{"type":"turn_context","model":"test-model"}}),
        json!({"timestamp":"2026-07-28T00:00:01.000Z","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":100,"output_tokens":10},"total_token_usage":{"input_tokens":100,"output_tokens":10}}}}),
    ]
    .iter()
    .map(serde_json::Value::to_string)
    .collect::<Vec<_>>()
    .join("\n");
    fs::write(&rollout, format!("{initial}\n")).unwrap();
    let query = TokenUsageQuery {
        days: 31,
        limit: 100,
        ..TokenUsageQuery::default()
    };
    let mut tailer = RolloutTailer::default();

    let first = tailer.read_from_roots(&[temp.path().to_path_buf()], &query);
    assert_eq!(first.events.len(), 1);
    assert_eq!(first.events[0].usage.input, 100);

    let mut file = fs::OpenOptions::new().append(true).open(&rollout).unwrap();
    writeln!(
        file,
        "{}",
        json!({"timestamp":"2026-07-28T00:00:02.000Z","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":200,"output_tokens":20},"total_token_usage":{"input_tokens":300,"output_tokens":30}}}})
    )
    .unwrap();
    file.flush().unwrap();

    let second = tailer.read_from_roots(&[temp.path().to_path_buf()], &query);
    assert_eq!(second.events.len(), 1);
    assert_eq!(second.events[0].usage.input, 200);
    assert!(
        tailer
            .read_from_roots(&[temp.path().to_path_buf()], &query)
            .events
            .is_empty()
    );
}
