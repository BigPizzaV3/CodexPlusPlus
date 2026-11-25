use super::super::{FileState, index_budget::Budget};
use chrono::NaiveDate;
use serde_json::json;

#[test]
fn long_context_rows_keep_session_branch_model_provider_and_turn() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let root = dir.path().canonicalize()?;
    let session = "00000000-0000-4000-8000-000000000001";
    let parent = "00000000-0000-4000-8000-000000000002";
    let at = "2026-01-02T12:00:00Z";
    let model = "m".repeat(600);
    let provider = "p".repeat(303);
    let counter = json!({"input_tokens":10,"cached_input_tokens":0,"output_tokens":2});
    let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let padding = "x".repeat(1100 << 10);
    for compressed in [false, true] {
        let path = root.join(if compressed {
            "context.jsonl.zst"
        } else {
            "context.jsonl"
        });
        for pad in ["", padding.as_str()] {
            let mut text = String::new();
            for (ordinal, kind, payload) in [
                (
                    2,
                    "session_meta",
                    json!({"session_id":session,"parent_thread_id":parent,"subagent_history_start_ordinal":1,
                    "source":{"subagent":{"thread_spawn":{"parent_thread_id":parent,"prompt":pad}}},
                    "history_base":{"thread_id":parent,"end_ordinal_exclusive":2,"end_byte_offset":17},"model_provider":"base"}),
                ),
                (
                    3,
                    "turn_context",
                    json!({"modelProvider":"ctx-provider","model":"ctx-model","turn_id":"ctx-turn","body":pad}),
                ),
                (
                    4,
                    "event_msg",
                    json!({"type":"thread_settings_changed","thread_settings":{"model":model,"model_provider":provider,"body":pad}}),
                ),
                (
                    5,
                    "event_msg",
                    json!({"type":"task_started","turn_id":"first","body":pad}),
                ),
                (
                    6,
                    "event_msg",
                    json!({"type":"turn_started","turn_id":"second","body":pad}),
                ),
                (
                    7,
                    "event_msg",
                    json!({"type":"token_count","info":{"total_token_usage":counter}}),
                ),
            ] {
                text.push_str(&format!(r#"{{"type":"{kind}","timestamp":"{at}","ordinal":{ordinal},"payload":{payload}}}"#));
                text.push('\n');
            }
            if compressed {
                std::fs::write(&path, zstd::stream::encode_all(text.as_bytes(), 1)?)?;
            } else {
                std::fs::write(&path, text.as_bytes())?;
            }
            let mut state = FileState::default();
            let mut done = false;
            for _ in 0..8 {
                done =
                    state.read_with_budget(&path, cutoff, &mut Budget::new(std::iter::empty()))?;
                if done {
                    break;
                }
            }
            assert!(done);
            assert_eq!(state.issues, 0);
            assert_eq!(
                state.scope_parent_candidates[session].iter().next(),
                Some(&Some(parent.to_owned()))
            );
            assert_eq!(state.history_boundary.parent.as_deref(), Some(parent));
            assert_eq!(state.history_boundary.start, Some(1));
            let base = state.history_base.unwrap();
            assert_eq!(base.rollout.to_string(), parent);
            assert_eq!((base.end_ordinal, base.end_offset), (2, 17));
            assert_eq!(state.events.len(), 1);
            let event = &state.events[0];
            assert_eq!(event.session, session);
            assert_eq!(event.role, "subagent");
            assert_eq!(event.turn, "second");
            assert_eq!((&event.model, &event.provider), (&model, &provider));
            let (ordinal, start, end) = event.rollout_position.unwrap();
            assert_eq!((ordinal, end), (7, text.len() as u64));
            assert!(text[start as usize..].contains("token_count"));
        }
    }
    Ok(())
}

#[test]
fn projection_preserves_large_identity_and_original_duplicate_semantics() -> anyhow::Result<()> {
    let big = "m".repeat(1100 << 10);
    for (kind, payload, expected) in [
        (
            "turn_context",
            format!(r#"{{"model":"{big}","unknown":"{big}"}}"#),
            json!({"model":big}),
        ),
        (
            "session_meta",
            r#"{"id":"id","id":"id"}"#.into(),
            json!(false),
        ),
        (
            "event_msg",
            r#"{"type":"task_started","type":"task_started","turn_id":"id"}"#.into(),
            json!(false),
        ),
        (
            "event_msg",
            r#"{"type":"turn_started","turn_id":"old","turn_id":"new"}"#.into(),
            json!({"type":"turn_started","turn_id":"new","duplicate_context":true,"duplicate_terminal":true}),
        ),
    ] {
        let prefix = format!(
            r#"{{"type":"{kind}","ordinal":5,"timestamp":"2026-01-02T12:00:00Z","payload":"#
        );
        let text = format!("{prefix}{payload}}}\n");
        assert!(super::inspect(&text.as_bytes()[..text.len().min(1024)]).2);
        let super::Projected::Line(line) = super::Spool::new(text.as_bytes(), true)?.finish(
            std::path::Path::new("unused"),
            text.len() as u64,
            None,
            "session",
            "prior",
            |_| false,
        )?
        else {
            panic!("context should reach the record parser")
        };
        let projected: serde_json::Value = serde_json::from_slice(&line)?;
        assert_eq!(projected["payload"], expected);
        assert_eq!(projected["ordinal"], 5);
    }
    Ok(())
}
