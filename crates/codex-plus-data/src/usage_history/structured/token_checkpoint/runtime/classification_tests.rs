use super::*;
use chrono::TimeDelta;
use rusqlite::params;
use std::fmt::Write;

#[test]
fn recent_sessions_cover_the_day_then_their_own_history_and_revisit_late_proofs() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    fs::create_dir_all(home.join("sessions")).unwrap();
    let home = fs::canonicalize(home).unwrap();
    let settings = root.path().join("settings.json");
    fs::write(&settings, r#"{"usageHistoryEnabled":true}"#).unwrap();
    let log = root.path().join("requests.jsonl");
    let now = Local::now();
    let yesterday = now - TimeDelta::hours(26);
    let earlier = now - TimeDelta::hours(6);
    let usage = json!({"input_tokens":100,"cached_input_tokens":20,"output_tokens":10});
    let mut proofs = String::new();
    let mut late_proof = String::new();
    for n in 1..=14 {
        let session = format!("00000000-0000-4000-8000-{n:012}");
        let model = format!("gpt-fixture-{n}");
        let meta = json!({"type":"session_meta","timestamp":yesterday.to_rfc3339(),
            "payload":{"id":session,"model_provider":"custom","source":"vscode"}});
        let turn = json!({"type":"turn_context","timestamp":yesterday.to_rfc3339(),
            "payload":{"turn_id":"test-turn","model":model}});
        let mut rows = format!("{meta}\n{turn}\n");
        let mut times = vec![yesterday, earlier];
        if n == 14 {
            times.push(earlier + TimeDelta::seconds(230));
        }
        times.push(now);
        for (i, at) in times.into_iter().enumerate() {
            let k = i + 1;
            let count = json!({"type":"event_msg","timestamp":at.to_rfc3339(),"payload":{
                "type":"token_count","info":{"total_token_usage":{"input_tokens":100*k,
                "cached_input_tokens":20*k,"output_tokens":10*k},"last_token_usage":usage}}});
            writeln!(rows, "{count}").unwrap();
            if n == 14 && i == 2 {
                continue;
            }
            let request_at = if n == 14 && i == 1 {
                at + TimeDelta::seconds(115)
            } else {
                at
            };
            let proof = json!({"id":format!("{n}-{i}"),"threadId":session,"model":model,
                "upstreamModel":model,"relayId":"fixture-relay","success":true,"state":"success",
                "statusCode":200,"timestampMs":request_at.timestamp_millis(),
                "startedAtMs":request_at.timestamp_millis()-100,"attempts":1,"autoRetries":0,
                "usage":{"inputTokens":100,"cachedTokens":20,"outputTokens":10}});
            writeln!(
                if n == 1 && i == 1 {
                    &mut late_proof
                } else {
                    &mut proofs
                },
                "{proof}"
            )
            .unwrap();
        }
        fs::write(home.join(format!("sessions/rollout-{n}.jsonl")), rows).unwrap();
    }
    fs::write(&log, &proofs).unwrap();
    let mut worker = Runtime::new(root.path().join("history"), settings);
    let pass = |worker: &mut Runtime| {
        let state = worker.update_with_request_log(&home, &log);
        assert!(state.storage_error.is_none(), "{:?}", state.storage_error);
        assert!(
            state.classification_error.is_none(),
            "{:?}",
            state.classification_error
        );
    };
    let exact = |worker: &Runtime, at: i64| {
        worker
            .store
            .as_ref()
            .unwrap()
            .db
            .query_row(
                "SELECT count(*) FROM token_resolved WHERE timestamp_ms = ?1
             AND json_extract(CAST(payload AS TEXT), '$.provider') = 'fixture-relay'",
                [at],
                |row| row.get::<_, usize>(0),
            )
            .unwrap()
    };
    pass(&mut worker);
    assert_eq!(exact(&worker, now.timestamp_millis()), 14);
    assert_eq!(exact(&worker, earlier.timestamp_millis()), 12);
    assert_eq!(exact(&worker, yesterday.timestamp_millis()), 0);
    pass(&mut worker);
    assert_eq!(exact(&worker, yesterday.timestamp_millis()), 14);

    proofs.push_str(&late_proof);
    fs::write(&log, proofs).unwrap();
    for _ in 0..3 {
        pass(&mut worker);
    }
    assert_eq!(exact(&worker, earlier.timestamp_millis()), 13);
    assert_eq!(
        exact(
            &worker,
            (earlier + TimeDelta::seconds(230)).timestamp_millis()
        ),
        0
    );
    let totals = worker.store.as_ref().unwrap().db.query_row(
        "SELECT count(*), sum(json_extract(CAST(payload AS TEXT), '$.delta.t')) FROM token_resolved",
        [], |row| Ok((row.get::<_, usize>(0)?, row.get::<_, usize>(1)?)),
    ).unwrap();
    assert_eq!(totals, (43, 43 * 110));

    let byte_budget = worker
        .store
        .as_ref()
        .unwrap()
        .db
        .query_row(
            "SELECT max(length(payload)) FROM token_resolved WHERE timestamp_ms = ?1",
            [now.timestamp_millis()],
            |row| row.get::<_, usize>(0),
        )
        .unwrap();
    let mut state = worker.initial_state();
    let cutoff = yesterday.date_naive();
    worker.evidence_before.clear();
    worker.evidence_cursor = 0;
    let blocked = worker
        .classify_retained(&home, cutoff, 1, &mut state)
        .unwrap_err();
    assert!(
        blocked
            .to_string()
            .contains("competition window exceeds input budget")
    );
    assert!(worker.evidence_before.is_empty());
    // 一轮只够一个完整窗口时，轮转使每个会话都能先拿到全部预算。
    for expected in 1..=14 {
        worker
            .classify_retained(&home, cutoff, byte_budget, &mut state)
            .unwrap();
        assert_eq!(worker.evidence_before.len(), expected);
    }
}

#[test]
fn byte_pages_reserve_whole_timestamp_groups_and_both_competition_margins() {
    let store = Store::open(Connection::open_in_memory().unwrap()).unwrap();
    store
        .db
        .execute(
            "INSERT INTO token_sessions(id,session) VALUES(1,'dense')",
            [],
        )
        .unwrap();
    let mut at = Vec::new();
    // 每组两行，以小预算强制分段；前后竞争范围各有跨页记录。
    for i in 0..120 {
        let stamp = i * 60_000;
        for _ in 0..2 {
            at.push(stamp);
            store
                .db
                .execute(
                    "INSERT INTO token_resolved VALUES(?1,1,?2,'2026-11-01',?3)",
                    params![at.len().to_be_bytes().as_slice(), stamp, b"{}".as_slice()],
                )
                .unwrap();
        }
    }
    let mut latest = at[at.len() - 1];
    let mut seen = vec![false; at.len()];
    let mut pages = 0;
    loop {
        let (window, bytes) = store
            .classification_page("dense", 0, latest, 100, 100)
            .unwrap()
            .unwrap();
        assert!(bytes <= 100);
        let actual = at
            .iter()
            .filter(|&&t| {
                (window.start - classification_lookback_ms()..window.end + 240_000).contains(&t)
            })
            .count()
            * 2;
        assert_eq!(bytes, actual);
        for (i, &t) in at.iter().enumerate().filter(|(_, t)| window.contains(t)) {
            assert!(!seen[i], "duplicate target at {t}");
            seen[i] = true;
        }
        pages += 1;
        let Some(previous) = store.classification_before("dense", window.start).unwrap() else {
            break;
        };
        latest = previous;
    }
    assert!(pages > 1);
    assert!(seen.into_iter().all(|target| target));
    assert!(
        store
            .classification_page("dense", 0, 60 * 60_000, 4, 4)
            .unwrap()
            .is_none()
    );
    let (window, bytes) = store
        .classification_page("dense", 0, 60 * 60_000, 4, 100)
        .unwrap()
        .unwrap();
    assert_eq!(
        at.iter().filter(|t| window.contains(t)).collect::<Vec<_>>(),
        [&3_600_000, &3_600_000]
    );
    assert_eq!(bytes, 15 * 4);
}
