use super::*;
use chrono::{DateTime, TimeDelta};
use rusqlite::params;
use std::fmt::Write;

#[test]
fn namespace_inference_reopens_retracts_and_upgrades_to_delivered_supplier() -> Result<()> {
    let root = tempfile::tempdir()?;
    let home = root.path().join("home");
    fs::create_dir_all(home.join("sessions"))?;
    let home = fs::canonicalize(home)?;
    let settings = root.path().join("settings.json");
    let configured = json!({"usageHistoryEnabled":true, "relayProfiles":[{
        "id":"supplier-a", "name":"Supplier A", "baseUrl":"https://supplier-a.example/v1"
    }]});
    fs::write(&settings, configured.to_string())?;
    let log = root.path().join("requests.jsonl");
    fs::write(&log, "")?;
    let session = "00000000-0000-4000-8000-000000000082";
    let at = Local::now() - TimeDelta::minutes(5);
    let model = "supplier-a/model-a";
    let usage = json!({"input_tokens":100,"cached_input_tokens":20,"output_tokens":10});
    let rows = [
        json!({"type":"session_meta","timestamp":at.to_rfc3339(),
            "payload":{"id":session,"model_provider":"custom"}}),
        json!({"type":"turn_context","timestamp":at.to_rfc3339(),
            "payload":{"turn_id":"turn","model":model}}),
        json!({"type":"token_usage_record","timestamp":at.to_rfc3339(),
            "payload":{"thread_id":session,"turn_id":"turn","response_id":"response-a",
                "usage":usage,"thread_token_usage":usage}}),
        json!({"type":"event_msg","timestamp":at.to_rfc3339(),
            "payload":{"type":"token_count","info":{
                "last_token_usage":usage,"total_token_usage":usage}}}),
    ];
    fs::write(home.join("sessions/rollout-namespace.jsonl"),
        rows.iter().map(|row| format!("{row}\n")).collect::<String>())?;
    let storage = root.path().join("history");
    let mut worker = Runtime::new(storage.clone(), settings.clone());
    let inspect = |worker: &mut Runtime| -> Result<Value> {
        for _ in 0..3 {
            let state = worker.update_with_request_log(&home, &log);
            assert!(state.storage_error.is_none(), "{:?}", state.storage_error);
            assert!(state.classification_error.is_none(), "{:?}", state.classification_error);
        }
        let db = &worker.store.as_ref().unwrap().db;
        assert_eq!(db.query_row("SELECT count(*) FROM token_resolved", [],
            |row| row.get::<_, usize>(0))?, 1);
        let payload: Vec<u8> = db.query_row("SELECT payload FROM token_resolved", [],
            |row| row.get(0))?;
        Ok(serde_json::from_slice(&payload)?)
    };
    let inferred = inspect(&mut worker)?;
    assert_eq!(inferred["provider"], "supplier-a");
    assert_eq!(inferred["provider_source"], "model-namespace");
    drop(worker);
    let mut worker = Runtime::new(storage, settings.clone());
    assert_eq!(inspect(&mut worker)?, inferred);
    fs::write(&settings, r#"{"usageHistoryEnabled":true}"#)?;
    let withdrawn = inspect(&mut worker)?;
    assert_eq!(withdrawn["provider"], "custom");
    assert!(withdrawn["provider_source"].is_null());
    fs::write(&settings, configured.to_string())?;
    assert_eq!(inspect(&mut worker)?, inferred);

    let proof = json!({"id":"request-a","threadId":session,
        "model":model,"upstreamModel":model,"relayId":"supplier-b",
        "success":true,"state":"success","statusCode":200,"timestampMs":at.timestamp_millis(),
        "startedAtMs":at.timestamp_millis()-100,"attempts":1,"autoRetries":0,
        "finalResponseEvidence":"final-response-supplier-v1","finalResponseId":"response-a",
        "usage":{"inputTokens":100,"outputTokens":10,"cachedTokensKnown":false}});
    fs::write(&log, format!("{proof}\n"))?;
    let direct = inspect(&mut worker)?;
    assert_eq!(direct["provider"], "supplier-b");
    assert_eq!(direct["provider_source"], "request-log-recovered-supplier-only");
    let mut conflicting = proof.clone();
    conflicting["id"] = json!("conflicting-request");
    conflicting["relayId"] = json!("supplier-c");
    fs::write(&log, format!("{proof}\n{conflicting}\n"))?;
    let conflict = inspect(&mut worker)?;
    assert_eq!(conflict["provider"], "custom");
    assert!(conflict["provider_source"].is_null());
    for row in [&withdrawn, &direct, &conflict] {
        for key in ["model", "timestamp", "delta", "total"] {
            assert_eq!(row[key], inferred[key], "{key}");
        }
    }
    Ok(())
}

#[test]
fn delayed_adjacent_receipts_confirm_suppliers_without_a_time_limit() -> Result<()> {
    for case in ["ordered", "midnight", "separate", "intervening", "conflicting"] {
        let root = tempfile::tempdir()?;
        let home = root.path().join("home");
        fs::create_dir_all(home.join("sessions"))?;
        let home = fs::canonicalize(home)?;
        let settings = root.path().join("settings.json");
        let log = root.path().join("requests.jsonl");
        fs::write(&settings, r#"{"usageHistoryEnabled":true}"#)?;
        let session = "00000000-0000-4000-8000-000000000081";
        let now = Local::now();
        let at = if case == "midnight" {
            now.date_naive().and_time(chrono::NaiveTime::MIN)
                .and_local_timezone(Local).earliest().unwrap() - TimeDelta::minutes(1)
        } else {
            now - TimeDelta::minutes(10)
        };
        let usage = |n| json!({"input_tokens":100*n,"cached_input_tokens":20*n,"output_tokens":10*n});
        let header = format!("{}\n{}\n",
            json!({"type":"session_meta","timestamp":at.to_rfc3339(),
                "payload":{"id":session,"model_provider":"custom"}}),
            json!({"type":"turn_context","timestamp":at.to_rfc3339(),
                "payload":{"turn_id":"turn","model":"gpt-fixture"}}));
        let mut rows = header.clone();
        let mut proofs = String::new();
        for n in 1..=2 {
            let receipt_at = at + TimeDelta::seconds((n - 1) * 240);
            let counter_at = receipt_at + TimeDelta::seconds(180);
            let receipt = json!({"type":"token_usage_record","timestamp":receipt_at.to_rfc3339(),
                "payload":{"thread_id":session,"turn_id":"turn","response_id":format!("response-{n}"),
                    "usage":usage(1),"thread_token_usage":usage(n)}});
            if n == 1 && case == "separate" {
                fs::write(home.join("sessions/rollout-peer.jsonl"), format!("{header}{receipt}\n"))?;
            } else {
                writeln!(rows, "{receipt}")?;
            }
            if n == 1 && case == "intervening" {
                writeln!(rows, "{}", json!({"type":"event_msg",
                    "timestamp":(receipt_at + TimeDelta::seconds(1)).to_rfc3339(),
                    "payload":{"type":"token_count","info":{
                        "last_token_usage":usage(0),"total_token_usage":usage(0)}}}))?;
            }
            if n == 1 && case == "conflicting" {
                let mut competing = receipt.clone();
                competing["payload"]["response_id"] = json!("other-response");
                writeln!(rows, "{competing}")?;
            }
            writeln!(rows, "{}", json!({"type":"event_msg","timestamp":counter_at.to_rfc3339(),
                "payload":{"type":"token_count","info":{
                    "last_token_usage":usage(1),"total_token_usage":usage(n)}}}))?;
            writeln!(proofs, "{}", json!({"id":format!("request-{n}"),"threadId":session,
                "model":"gpt-fixture","upstreamModel":"gpt-fixture","relayId":format!("supplier-{n}"),
                "success":true,"state":"success","statusCode":200,"timestampMs":receipt_at.timestamp_millis(),
                "startedAtMs":receipt_at.timestamp_millis()-100,"attempts":1,"autoRetries":0,
                "finalResponseEvidence":"final-response-v1","finalResponseId":format!("response-{n}"),
                "usage":{"inputTokens":100,"cachedTokens":20,"outputTokens":10}}))?;
        }
        fs::write(home.join("sessions/rollout-main.jsonl"), rows)?;
        fs::write(&log, &proofs)?;
        let storage = root.path().join("history");
        let mut worker = Runtime::new(storage.clone(), settings.clone());
        let inspect = |worker: &mut Runtime| -> Result<Vec<Value>> {
            for _ in 0..3 {
                let state = worker.update_with_request_log(&home, &log);
                assert!(state.storage_error.is_none(), "{case}: {:?}", state.storage_error);
                assert!(state.classification_error.is_none(), "{case}: {:?}", state.classification_error);
            }
            let db = &worker.store.as_ref().unwrap().db;
            let mut query = db.prepare("SELECT payload FROM token_resolved ORDER BY timestamp_ms")?;
            let rows = query.query_map([], |row| row.get::<_, Vec<u8>>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows.iter().map(|row| serde_json::from_slice(row)).collect::<serde_json::Result<_>>()?)
        };
        let before = inspect(&mut worker)?;
        assert_eq!(before.len(), 2, "{case}");
        for (i, event) in before.iter().enumerate() {
            assert_eq!(event["delta"]["i"], 100, "{case}");
            assert_eq!(event["delta"]["c"], 20, "{case}");
            assert_eq!(event["delta"]["o"], 10, "{case}");
            let direct = i == 1 || matches!(case, "ordered" | "midnight");
            assert_eq!(event["provider_source"] == "request-log-recovered-supplier-only", direct, "{case}: {event}");
            if direct {
                assert_eq!(event["provider"], format!("supplier-{}", i + 1), "{case}");
            }
        }
        let captured = proofs.lines().map(serde_json::from_str).collect::<serde_json::Result<Vec<_>>>()?;
        worker.evidence.provider_cache.capture_proxy(&captured)?;
        drop(worker);
        fs::write(&log, "")?;
        let mut worker = Runtime::new(storage, settings);
        assert_eq!(inspect(&mut worker)?, before, "{case}: persisted proofs changed");
    }
    Ok(())
}

#[test]
fn archive_backlog_keeps_paired_supplier_proofs_revocable() -> Result<()> {
    let root = tempfile::tempdir()?;
    let home = root.path().join("home");
    fs::create_dir_all(home.join("sessions"))?;
    fs::create_dir(home.join("archived_sessions"))?;
    let home = fs::canonicalize(home)?;
    let settings = root.path().join("settings.json");
    let log = root.path().join("requests.jsonl");
    fs::write(&settings, r#"{"usageHistoryEnabled":true}"#)?;
    let at = (Local::now() - TimeDelta::minutes(5)).to_rfc3339();
    let usage = json!({"input_tokens":100,"cached_input_tokens":20,"output_tokens":10});
    let mut proofs = String::new();
    for n in 1..=5 {
        let session = format!("00000000-0000-4000-8000-{n:012}");
        let id = format!("response-{n}");
        let meta = json!({"type":"session_meta","timestamp":at,
            "payload":{"id":session,"model_provider":"custom","source":"vscode"}});
        let turn = json!({"type":"turn_context","timestamp":at,
            "payload":{"turn_id":"turn","model":"gpt-fixture"}});
        let mut native_usage = usage.clone();
        if n == 3 {
            native_usage.as_object_mut().unwrap().remove("cached_input_tokens");
        }
        let mut receipt_usage = native_usage.clone();
        if n == 4 {
            receipt_usage.as_object_mut().unwrap().remove("cached_input_tokens");
        } else if n == 5 {
            receipt_usage["cached_input_tokens"] = json!(21);
        }
        let receipt = json!({"type":"token_usage_record","timestamp":at,"payload":{
            "thread_id":session,"turn_id":"turn","response_id":id,
            "usage":receipt_usage,"thread_token_usage":receipt_usage}});
        let count = json!({"type":"event_msg","timestamp":at,"payload":{"type":"token_count",
            "info":{"total_token_usage":native_usage,"last_token_usage":native_usage}}});
        let mut rows = format!("{meta}\n{turn}\n");
        if n != 2 {
            writeln!(rows, "{receipt}")?;
        }
        writeln!(rows, "{count}")?;
        fs::write(home.join(format!("sessions/rollout-{session}.jsonl")), rows)?;
        let at = DateTime::parse_from_rfc3339(&at)?.timestamp_millis();
        let mut proof = json!({"id":format!("request-{n}"),"threadId":session,"model":"gpt-fixture",
            "upstreamModel":"gpt-fixture","relayId":"fixture-relay","success":true,"state":"success",
            "statusCode":200,"timestampMs":at,"startedAtMs":at-100,
            "finalResponseEvidence":"final-response-v1","finalResponseId":id,
            "attempts":1,"autoRetries":0,"usage":{"inputTokens":100,"cachedTokens":20,"outputTokens":10}});
        if n >= 3 {
            proof["finalResponseEvidence"] = json!("final-response-supplier-v1");
            proof["usage"]["cachedTokens"] = Value::Null;
            proof["usage"]["cachedTokensKnown"] = json!(false);
        }
        writeln!(proofs, "{proof}")?;
    }
    let padding = format!("{{\"type\":\"ignored\",\"payload\":\"{}\"}}\n", "x".repeat(65536));
    fs::write(&log, "")?;
    let storage = root.path().join("history");
    let mut worker = Runtime::new(storage.clone(), settings.clone());
    assert!(worker.update_with_request_log(&home, &log).storage_error.is_none());
    fs::write(home.join("archived_sessions/rollout-pending.jsonl"), padding.repeat(1200))?;
    fs::write(&log, &proofs)?;
    let pass = |worker: &mut Runtime, exact: usize| -> Result<()> {
        let state = worker.update_with_request_log(&home, &log);
        assert!(state.storage_error.is_none(), "{:?}", state.storage_error);
        assert!(state.classification_error.is_none(), "{:?}", state.classification_error);
        let db = &worker.store.as_ref().unwrap().db;
        assert!(db.query_row("SELECT pending_sources FROM token_coverage", [], |row| row.get::<_, usize>(0))? > 0);
        assert_eq!(db.query_row(
            "SELECT count(*),sum(json_extract(CAST(payload AS TEXT),'$.delta.t')),
             sum(coalesce(json_extract(CAST(payload AS TEXT),'$.provider_source')='request-log-exact',0)),
             sum(coalesce(json_extract(CAST(payload AS TEXT),'$.provider_source')='session-context-inferred',0)),
             sum(coalesce(json_extract(CAST(payload AS TEXT),'$.provider_source')='request-log-recovered-supplier-only',0))
             FROM token_resolved",
            [], |row| Ok((row.get::<_, usize>(0)?,row.get::<_, usize>(1)?,row.get::<_, usize>(2)?,row.get::<_, usize>(3)?,row.get::<_, usize>(4)?)),
        )?, (5, 550, exact, 2, 2));
        for n in [4, 5] {
            let session = format!("00000000-0000-4000-8000-{n:012}");
            let (source, input, cached, known, output): (String, u64, u64, bool, u64) = db.query_row(
                "SELECT json_extract(CAST(payload AS TEXT),'$.provider_source'),
                 json_extract(CAST(payload AS TEXT),'$.delta.i'),
                 json_extract(CAST(payload AS TEXT),'$.delta.c'),
                 json_extract(CAST(payload AS TEXT),'$.delta.k'),
                 json_extract(CAST(payload AS TEXT),'$.delta.o')
                 FROM token_resolved WHERE session_id=(SELECT id FROM token_sessions WHERE session=?1)",
                [&session],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )?;
            assert_eq!((input, cached, known, output), (100, 20, true, 10));
            assert_eq!(source, if n == 4 {
                "request-log-recovered-supplier-only"
            } else {
                "session-context-inferred"
            });
        }
        Ok(())
    };
    pass(&mut worker, 1)?;
    pass(&mut worker, 1)?;
    // 正式网关在日志轮转前同步保留完成请求；夹具复用同一写入入口。
    let captured = proofs.lines().map(serde_json::from_str).collect::<serde_json::Result<Vec<_>>>()?;
    worker.evidence.provider_cache.capture_proxy(&captured)?;
    drop(worker);
    fs::write(&log, "")?;
    let mut worker = Runtime::new(storage, settings);
    pass(&mut worker, 1)?;
    let mut competing: Value = serde_json::from_str(proofs.lines().next().unwrap())?;
    competing["id"] = json!("competing-request");
    competing["relayId"] = json!("competing-relay");
    writeln!(proofs, "{competing}")?;
    fs::write(&log, proofs)?;
    pass(&mut worker, 0)?;
    Ok(())
}

#[test]
fn distinct_terminal_ids_keep_equal_count_supplier_proofs() -> Result<()> {
    let root = tempfile::tempdir()?;
    let home = root.path().join("home");
    fs::create_dir_all(home.join("sessions"))?;
    let home = fs::canonicalize(home)?;
    let settings = root.path().join("settings.json");
    fs::write(&settings, r#"{"usageHistoryEnabled":true}"#)?;
    let log = root.path().join("requests.jsonl");
    let source = home.join("sessions/rollout-equal-counts.jsonl");
    let session = "00000000-0000-4000-8000-000000000098";
    let now = Local::now() - TimeDelta::minutes(5);
    let mut rows = format!("{}\n", json!({"type":"session_meta","timestamp":now.to_rfc3339(),
        "payload":{"id":session,"model_provider":"custom"}}));
    let mut proofs = Vec::new();
    let mut worker = Runtime::new(root.path().join("history"), settings);
    let read = |worker: &mut Runtime| -> Result<Vec<(String, Option<String>, u64)>> {
        let state = worker.update_with_request_log(&home, &log);
        assert!(state.storage_error.is_none(), "{:?}", state.storage_error);
        assert!(state.classification_error.is_none(), "{:?}", state.classification_error);
        let mut query = worker.store.as_ref().unwrap().db.prepare(
            "SELECT json_extract(CAST(payload AS TEXT),'$.provider'),
                    json_extract(CAST(payload AS TEXT),'$.provider_source'),
                    json_extract(CAST(payload AS TEXT),'$.delta.t')
             FROM token_resolved ORDER BY timestamp_ms",
        )?;
        let rows = query.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    };
    let counts = |n| json!({"input_tokens":100*n, "cached_input_tokens":20*n, "output_tokens":10*n});
    for n in 1..=2 {
        let at = now + TimeDelta::seconds(n);
        let stamp = at.to_rfc3339();
        let turn = format!("turn-{n}");
        let response = format!("resp_equal_{n}");
        writeln!(rows, "{}", json!({"type":"turn_context","timestamp":stamp,
            "payload":{"turn_id":turn,"model":"gpt-fixture"}}))?;
        writeln!(rows, "{}", json!({"type":"token_usage_record","timestamp":stamp,
            "payload":{"thread_id":session,"turn_id":turn,"response_id":response,
                "usage":counts(1),"thread_token_usage":counts(n)}}))?;
        writeln!(rows, "{}", json!({"type":"event_msg","timestamp":stamp,"payload":{
            "type":"token_count","info":{"last_token_usage":counts(1),"total_token_usage":counts(n)}}}))?;
        proofs.push(json!({"id":format!("request-{n}"),"threadId":session,"model":"gpt-fixture",
            "upstreamModel":"gpt-fixture","relayId":format!("supplier-{n}"),"success":true,
            "state":"success","statusCode":200,"timestampMs":at.timestamp_millis()-10,
            "startedAtMs":at.timestamp_millis()-100,"attempts":1,"autoRetries":0,
            "finalResponseEvidence":"final-response-v1","finalResponseId":response,
            "usage":{"inputTokens":100,"cachedTokens":20,"outputTokens":10}}));
        fs::write(&source, &rows)?;
        fs::write(&log, proofs.iter().map(|proof| format!("{proof}\n")).collect::<String>())?;
        let actual = read(&mut worker)?;
        assert_eq!(actual.len(), n as usize);
        for (index, (supplier, proof, total)) in actual.into_iter().enumerate() {
            assert_eq!(supplier, format!("supplier-{}", index + 1));
            assert!(matches!(proof.as_deref(), Some("request-log-exact" | "request-log-recovered-supplier-only")));
            assert_eq!(total, 110);
        }
    }
    let mut conflict = proofs[0].clone();
    conflict["id"] = json!("competing-request");
    conflict["relayId"] = json!("competing-supplier");
    proofs.push(conflict);
    fs::write(&log, proofs.iter().map(|proof| format!("{proof}\n")).collect::<String>())?;
    let actual = read(&mut worker)?;
    assert_eq!(actual[0], ("custom".into(), None, 110));
    assert_eq!(actual[1].0, "supplier-2");
    assert_eq!(actual[1].2, 110);
    Ok(())
}

#[test]
fn completion_during_ingest_is_direct_on_first_classification() -> Result<()> {
    let root = tempfile::tempdir()?;
    let home = root.path().join("home");
    fs::create_dir_all(home.join("sessions"))?;
    let home = fs::canonicalize(home)?;
    let settings = root.path().join("settings.json");
    fs::write(&settings, r#"{"usageHistoryEnabled":true}"#)?;
    let log = root.path().join("requests.jsonl");
    let source = home.join("sessions/rollout-completion-during-ingest.jsonl");
    let session = "00000000-0000-4000-8000-000000000097";
    let at = Local::now() - TimeDelta::minutes(1);
    let usage = json!({"input_tokens":100,"cached_input_tokens":20,"output_tokens":10});
    let rows = [
        json!({"type":"session_meta","timestamp":at.to_rfc3339(),
            "payload":{"id":session,"model_provider":"custom"}}),
        json!({"type":"turn_context","timestamp":at.to_rfc3339(),
            "payload":{"turn_id":"turn","model":"gpt-fixture"}}),
        json!({"type":"token_usage_record","timestamp":at.to_rfc3339(),
            "payload":{"thread_id":session,"turn_id":"turn","response_id":"resp_new",
                "usage":usage,"thread_token_usage":usage}}),
        json!({"type":"event_msg","timestamp":at.to_rfc3339(),
            "payload":{"type":"token_count","info":{
                "last_token_usage":usage,"total_token_usage":usage}}}),
    ];
    let request = json!({"id":"new-completion","threadId":session,"model":"gpt-fixture",
        "upstreamModel":"gpt-fixture","relayId":"supplier-new","success":true,
        "state":"success","statusCode":200,"timestampMs":at.timestamp_millis(),
        "startedAtMs":at.timestamp_millis()-100,"attempts":1,"autoRetries":0,
        "finalResponseEvidence":"final-response-v1","finalResponseId":"resp_new",
        "usage":{"inputTokens":100,"cachedTokens":20,"outputTokens":10}});
    fs::write(&log, "")?;
    let mut worker = Runtime::new(root.path().join("history"), settings);
    let cutoff = at.date_naive() - Days::new(3);
    let (_, available) = worker.evidence.load_evidence(&home, cutoff, &log);
    assert!(available);
    worker.evidence.request_evidence.refresh(&log);
    fs::write(&source, rows.iter().map(|row| format!("{row}\n")).collect::<String>())?;
    let mut state = worker.initial_state();
    assert!(worker.refresh(&home, cutoff, 64 << 20, &mut state)?);
    // Native usage arrived during ingestion; gateway completion is durable before classification.
    fs::write(&log, format!("{request}\n"))?;
    worker.classify_retained(&home, cutoff, MAX_RETAINED_BYTES, &log, &mut state)?;
    let (supplier, evidence, total): (String, Option<String>, u64) = worker.store.as_ref().unwrap().db
        .query_row("SELECT json_extract(CAST(payload AS TEXT),'$.provider'),
            json_extract(CAST(payload AS TEXT),'$.provider_source'),
            json_extract(CAST(payload AS TEXT),'$.delta.t') FROM token_resolved", [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
    assert_eq!(supplier, "supplier-new");
    assert!(matches!(evidence.as_deref(), Some("request-log-exact" | "request-log-recovered-supplier-only")));
    assert_eq!(total, 110);
    Ok(())
}

#[test]
fn new_tail_preempts_one_old_page_without_starving_history() -> Result<()> {
    use std::io::Write as _;
    let root = tempfile::tempdir()?;
    let home = root.path().join("home");
    fs::create_dir_all(home.join("sessions"))?;
    let home = fs::canonicalize(home)?;
    let settings = root.path().join("settings.json");
    fs::write(&settings, r#"{"usageHistoryEnabled":true}"#)?;
    let log = root.path().join("requests.jsonl");
    let session = "00000000-0000-4000-8000-000000000083";
    let now = Local::now();
    let times = [
        now - TimeDelta::hours(50),
        now - TimeDelta::hours(26),
        now - TimeDelta::minutes(16),
        now - TimeDelta::minutes(13),
        now - TimeDelta::minutes(10),
        now - TimeDelta::minutes(7),
        now - TimeDelta::minutes(4),
    ];
    let usage = json!({"input_tokens":100,"cached_input_tokens":20,"output_tokens":10});
    let source = home.join("sessions/rollout-priority.jsonl");
    fs::write(&source, format!("{}\n{}\n",
        json!({"type":"session_meta","timestamp":times[0].to_rfc3339(),
            "payload":{"id":session,"model_provider":"custom"}}),
        json!({"type":"turn_context","timestamp":times[0].to_rfc3339(),
            "payload":{"turn_id":"turn","model":"gpt-fixture"}})))?;
    let append = |i: usize| -> Result<()> {
        let at = times[i];
        let n = i + 1;
        let response = format!("resp_priority_{i}");
        let total = json!({"input_tokens":100*n,"cached_input_tokens":20*n,"output_tokens":10*n});
        let mut out = fs::OpenOptions::new().append(true).open(&source)?;
        writeln!(out, "{}",
            json!({"type":"token_usage_record","timestamp":at.to_rfc3339(),"payload":{
                "thread_id":session,"turn_id":"turn","response_id":response,
                "usage":usage,"thread_token_usage":total}}))?;
        writeln!(out, "{}",
            json!({"type":"event_msg","timestamp":at.to_rfc3339(),"payload":{
                "type":"token_count","info":{"last_token_usage":usage,"total_token_usage":total}}}))?;
        writeln!(fs::OpenOptions::new().append(true).create(true).open(&log)?, "{}",
            json!({"id":format!("proof-{i}"),"threadId":session,"model":"gpt-fixture",
                "upstreamModel":"gpt-fixture","relayId":format!("fixture-relay-{}", i % 2),"success":true,
                "state":"success","statusCode":200,"timestampMs":at.timestamp_millis(),
                "startedAtMs":at.timestamp_millis()-100,"attempts":1,"autoRetries":0,
                "finalResponseEvidence":"final-response-v1","finalResponseId":response,
                "usage":{"inputTokens":100,"cachedTokens":20,"outputTokens":10}}))?;
        Ok(())
    };
    for i in 0..3 {
        append(i)?;
    }
    let mut worker = Runtime::new(root.path().join("history"), settings);
    let state = worker.update_with_request_log(&home, &log);
    assert!(state.storage_error.is_none(), "{:?}", state.storage_error);
    assert!(state.classification_error.is_none(), "{:?}", state.classification_error);
    let cursor = &worker.evidence_before[session];
    assert!(cursor.older_next);
    let before = cursor.before;
    let exact = |worker: &Runtime, i: usize| -> Result<bool> {
        Ok(worker.store.as_ref().unwrap().db.query_row(
            "SELECT coalesce(json_extract(CAST(payload AS TEXT),'$.provider_source')='request-log-exact'
               AND json_extract(CAST(payload AS TEXT),'$.provider')=?,0)
             FROM token_resolved WHERE timestamp_ms=?",
            params![format!("fixture-relay-{}", i % 2), times[i].timestamp_millis()], |row| row.get(0))?)
    };
    assert!(exact(&worker, 2)?);
    assert!(!exact(&worker, 1)?);
    let pass = |worker: &mut Runtime, budget| -> Result<()> {
        worker.evidence.request_evidence.refresh(&log);
        let cutoff = now.date_naive() - Days::new(3);
        let mut state = worker.initial_state();
        assert!(worker.refresh(&home, cutoff, 64 << 20, &mut state)?);
        // 活跃尾部和历史各自推进，不减少完整竞争窗口。
        worker.classify_retained(&home, cutoff, budget, &log, &mut state)?;
        Ok(())
    };
    append(3)?;
    pass(&mut worker, 1)?;
    assert!(exact(&worker, 3)?);
    assert!(!exact(&worker, 1)?);
    assert!(worker.evidence_before[session].tail_preempted);
    assert_eq!(worker.evidence_before[session].before, before);
    append(4)?;
    pass(&mut worker, 1)?;
    assert!(exact(&worker, 1)?);
    assert!(exact(&worker, 4)?, "an old page must not defer the new terminal proof");
    assert!(!worker.evidence_before[session].tail_preempted);
    assert!(worker.evidence_before[session].before < before);
    append(5)?;
    pass(&mut worker, 1)?;
    assert!(exact(&worker, 5)?);
    assert!(!exact(&worker, 0)?);
    append(6)?;
    pass(&mut worker, MAX_RETAINED_BYTES)?;
    assert!(exact(&worker, 0)?);
    assert!(exact(&worker, 6)?, "remaining budget should classify the new tail");
    let totals: (usize, usize) = worker.store.as_ref().unwrap().db.query_row(
        "SELECT count(*),sum(json_extract(CAST(payload AS TEXT),'$.delta.t')) FROM token_resolved",
        [], |row| Ok((row.get(0)?, row.get(1)?)))?;
    assert_eq!(totals, (7, 770));
    Ok(())
}

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
             AND json_extract(CAST(payload AS TEXT), '$.provider') = 'fixture-relay'
             AND json_extract(CAST(payload AS TEXT), '$.provider_source') = 'request-log-exact'",
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
    fs::write(&log, &proofs).unwrap();
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
    let totals = |worker: &Runtime| worker.store.as_ref().unwrap().db.query_row(
        "SELECT count(*), sum(json_extract(CAST(payload AS TEXT), '$.delta.t')) FROM token_resolved",
        [], |row| Ok((row.get::<_, usize>(0)?, row.get::<_, usize>(1)?)),
    ).unwrap();
    assert_eq!(totals(&worker), (43, 43 * 110));

    let mut competitor = proofs.lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .find(|proof| proof["id"] == "1-2")
        .unwrap();
    competitor["id"] = json!("late-competitor");
    writeln!(proofs, "{competitor}").unwrap();
    fs::write(&log, proofs).unwrap();
    worker.evidence.request_evidence.refresh(&log);
    let mut state = worker.initial_state();
    let cutoff = yesterday.date_naive();
    worker.evidence_before.clear();
    worker.evidence_cursor = 0;
    // 即使预算放不下一条记录，每轮仍核对一个完整窗口，再轮到下一会话。
    for expected in 1..=14 {
        worker
            .classify_retained(&home, cutoff, 1, &log, &mut state)
            .unwrap();
        assert_eq!(worker.evidence_before.len(), expected);
        assert_eq!(exact(&worker, now.timestamp_millis()), 13);
    }
    assert_eq!(totals(&worker), (43, 43 * 110));
}

#[test]
fn unrelated_legacy_prefix_allows_first_request_log_proof_during_normal_update() -> Result<()> {
    let root = tempfile::tempdir()?;
    let home = root.path().join("home");
    fs::create_dir_all(home.join("sessions"))?;
    let home = fs::canonicalize(home)?;
    let settings = root.path().join("settings.json");
    let storage = root.path().join("history");
    let log = root.path().join("requests.jsonl");
    fs::write(&settings, r#"{"usageHistoryEnabled":true}"#)?;
    fs::write(&log, "")?;
    let at = Local::now() - TimeDelta::minutes(5);
    let sessions = ["00000000-0000-4000-8000-000000000031", "00000000-0000-4000-8000-000000000032"];
    let counts = json!({"input_tokens":100,"cached_input_tokens":20,"output_tokens":10});
    let mut proofs = String::new();
    let large = home.join(format!("sessions/rollout-{}.jsonl", sessions[0]));
    for session in sessions {
        let meta = json!({"type":"session_meta","timestamp":at.to_rfc3339(),
            "payload":{"id":session,"model_provider":"custom","source":"vscode"}});
        let turn = json!({"type":"turn_context","timestamp":at.to_rfc3339(),
            "payload":{"turn_id":"turn","model":"gpt-fixture"}});
        let count = json!({"type":"event_msg","timestamp":at.to_rfc3339(),"payload":{
            "type":"token_count","info":{"total_token_usage":counts,"last_token_usage":counts}}});
        let mut rows = format!("{meta}\n{turn}\n{count}\n");
        if session == sessions[0] {
            let padding = format!("{{\"type\":\"ignored\",\"payload\":\"{}\"}}\n", "x".repeat(65_536));
            rows.push_str(&padding.repeat(145));
        }
        fs::write(home.join(format!("sessions/rollout-{session}.jsonl")), rows)?;
        let proof = json!({"id":format!("proof-{session}"),"threadId":session,"model":"gpt-fixture",
            "upstreamModel":"gpt-fixture-final","relayId":"fixture-relay","success":true,"state":"success",
            "statusCode":200,"timestampMs":at.timestamp_millis(),"startedAtMs":at.timestamp_millis()-100,
            "finalResponseEvidence":"final-response-v1","finalResponseId":format!("resp-{session}"),
            "attempts":1,"autoRetries":0,"usage":{"inputTokens":100,"cachedTokens":20,"outputTokens":10}});
        writeln!(proofs, "{proof}")?;
    }
    let pass = |worker: &mut Runtime| {
        let state = worker.update_with_request_log(&home, &log);
        assert!(state.storage_error.is_none(), "{:?}", state.storage_error);
        assert!(state.classification_error.is_none(), "{:?}", state.classification_error);
    };
    let pending = |worker: &Runtime| {
        worker.store.as_ref().unwrap().db.query_row(
            "SELECT pending_sources FROM token_coverage WHERE singleton=1", [], |row| row.get::<_, usize>(0),
        ).unwrap()
    };
    let classifications = |worker: &mut Runtime| -> Result<[bool; 2]> {
        let mut actual = BTreeMap::new();
        Store::read_committed(&mut worker.store.as_mut().unwrap().db, i64::MIN, i64::MAX, |event| {
            let delta = event.delta;
            assert_eq!((delta.input_tokens, delta.cached_input_tokens, delta.output_tokens, delta.total_tokens,
                delta.cached_known), (100, 20, 10, 110, true));
            let exact = event.provider_source.as_deref() == Some("request-log-exact");
            if exact {
                assert_eq!((event.provider.as_str(), event.model.as_str()), ("fixture-relay", "gpt-fixture-final"));
            }
            assert!(actual.insert(event.session, exact).is_none());
            Ok(())
        })?;
        assert_eq!(actual.len(), sessions.len());
        Ok(sessions.map(|session| actual[session]))
    };

    let mut worker = Runtime::new(storage.clone(), settings.clone());
    pass(&mut worker);
    for _ in 0..16 {
        if pending(&worker) == 0 { break; }
        pass(&mut worker);
    }
    assert_eq!(pending(&worker), 0);
    assert_eq!(classifications(&mut worker)?, [false, false]);
    let store = worker.store.as_ref().unwrap();
    let (revision, _) = store.checkpoint(&large)?.unwrap();
    store.db.execute(
        "UPDATE token_sources SET checkpoint=CAST(json_remove(checkpoint,'$.legacy_identity_prefix') AS BLOB)
         WHERE revision=?", [revision],
    )?;
    drop(worker);

    fs::write(&log, proofs)?;
    let mut worker = Runtime::new(storage, settings);
    pass(&mut worker);
    assert!(pending(&worker) > 0);
    assert!(worker.store.as_ref().unwrap().checkpoint(&large)?.unwrap().1.legacy_identity_prefix > 0);
    let during_recheck = classifications(&mut worker)?;
    for _ in 0..16 {
        pass(&mut worker);
        if pending(&worker) == 0 { break; }
    }
    assert_eq!(pending(&worker), 0);
    assert_eq!(classifications(&mut worker)?, [true, true]);
    assert_eq!(during_recheck, [false, true]);
    Ok(())
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
