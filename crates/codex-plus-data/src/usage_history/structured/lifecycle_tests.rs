use super::*;
use chrono::TimeZone;

static SERIAL: Mutex<()> = Mutex::new(());

fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf, Query) {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    fs::create_dir_all(home.join("sessions")).unwrap();
    let home = fs::canonicalize(home).unwrap();
    let settings = root.path().join("settings.json");
    fs::write(
        &settings,
        r#"{"usageHistoryEnabled":true,"relayProfiles":[]}"#,
    )
    .unwrap();
    let options = Query::parse(&json!({}), Local::now().date_naive()).unwrap();
    (root, home, settings, options)
}

fn settled(home: &Path, settings: &Path, options: &Query) -> Value {
    for _ in 0..1000 {
        let response = query_with_settings(home, options, &json!({}), settings.to_owned());
        if response["scan"]["processing"] == false {
            if response["complete"] == true {
                return response;
            }
            // 采集器并发写缓存时允许本轮 partial，等待重试后的完整快照。
            if let Some(cache) = homes().lock().unwrap().get(home) {
                cache.lock().unwrap().refreshed = None;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("isolated index did not settle");
}

#[test]
fn disabled_query_creates_no_index_or_cache() {
    let _serial = SERIAL.lock().unwrap();
    let (root, home, settings, options) = fixture();
    fs::write(&settings, r#"{"usageHistoryEnabled":false}"#).unwrap();
    let response = query_with_settings(&home, &options, &json!({}), settings);
    assert_eq!(response["enabled"], false);
    assert!(!homes().lock().unwrap().contains_key(&home));
    assert!(!root.path().join("usage-history").exists());
}

#[test]
fn idle_expiry_releases_index_and_clear_joins_inflight_worker() {
    let _serial = SERIAL.lock().unwrap();
    let (_root, home, settings, options) = fixture();
    assert_eq!(settled(&home, &settings, &options)["complete"], true);
    let cache = homes().lock().unwrap()[&home].clone();
    let weak = Arc::downgrade(&cache);
    cache.lock().unwrap().touched = Instant::now() - IDLE_TIMEOUT - Duration::from_secs(1);
    assert!(evict_idle(&home, &cache));
    drop(cache);
    assert!(weak.upgrade().is_none());
    let _ = query_with_settings(&home, &options, &json!({}), settings);
    clear(&home);
    assert!(!homes().lock().unwrap().contains_key(&home));
}

#[test]
fn cursor_and_activity_payloads_require_explicit_opt_in() {
    let _serial = SERIAL.lock().unwrap();
    let (_root, home, settings, options) = fixture();
    let response = settled(&home, &settings, &options);
    assert_eq!(response["accountingVersion"], 2);
    assert!(response.get("sessionCursors").is_none());
    assert!(response.get("activityRecords").is_none());
    let request = json!({"includeActivity":true,"includeSessionCursors":true});
    let options = Query::parse(&request, Local::now().date_naive()).unwrap();
    let response = query_with_settings(&home, &options, &request, settings);
    assert!(response["activityRecords"].is_array());
    assert!(response["sessionCursors"].is_array());
    clear(&home);
}

#[test]
fn invalid_scope_and_half_open_time_bounds_are_rejected() {
    let _serial = SERIAL.lock().unwrap();
    let (_root, home, settings, options) = fixture();
    for request in [
        json!({"scopeSessionId":"not-a-session"}),
        json!({"startAt":20}),
        json!({"startAt":20,"endAt":10}),
    ] {
        assert_eq!(
            query_with_settings(&home, &options, &request, settings.clone())["status"],
            "unavailable"
        );
    }
    assert!(!homes().lock().unwrap().contains_key(&home));
}

#[test]
fn history_route_keeps_committed_usage_readable_while_the_writer_retries() {
    use std::io::Write;

    let _serial = SERIAL.lock().unwrap();
    let (_root, home, settings, options) = fixture();
    let source = home.join("sessions").join("rollout-01.jsonl");
    let session = "00000000-0000-4000-8000-000000000001";
    let now = Local::now();
    let head = json!({
        "type":"session_meta", "timestamp":now.to_rfc3339(),
        "payload":{"id":session,"model_provider":"custom"}
    });
    let count = |step: u64| {
        json!({
            "type":"event_msg", "timestamp":(now + Days::milliseconds(step as i64)).to_rfc3339(),
            "payload":{"type":"token_count","info":{
                "total_token_usage":{"input_tokens":100*step,"cached_input_tokens":20*step,"output_tokens":10*step},
                "last_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":10}
            }}
        })
    };
    fs::write(&source, format!("{head}\n{}\n", count(1))).unwrap();
    let first = settled(&home, &settings, &options);
    assert_eq!(first["source"], "persistent-local-rollout");
    assert_eq!(first["days"][0]["usage"]["totalTokens"], 110);
    let db = rusqlite::Connection::open(
        provider_cache::storage_dir(&home, &settings).join(super::super::TOKEN_INDEX_DB),
    )
    .unwrap();
    db.execute_batch(
        "CREATE TRIGGER refuse_usage BEFORE INSERT ON token_days
         BEGIN SELECT RAISE(ABORT, 'simulated storage write failure'); END;",
    )
    .unwrap();
    let mut tail = fs::OpenOptions::new().append(true).open(&source).unwrap();
    writeln!(tail, "{}", count(2)).unwrap();
    drop(tail);
    let cache = homes().lock().unwrap()[&home].clone();
    cache.lock().unwrap().refreshed = None;
    let request =
        json!({"scopeSessionId":session,"includeSessionCursors":true,"includeActivity":true});
    let query = Query::parse(&request, now.date_naive()).unwrap();
    let mut blocked = None;
    for _ in 0..500 {
        let response = query_with_settings(&home, &query, &request, settings.clone());
        assert_eq!(response["revision"], first["revision"]);
        assert_eq!(response["days"][0]["usage"]["totalTokens"], 110);
        if response["scan"]["processing"] == false && response["scan"]["failedSources"] == 1 {
            blocked = Some(response);
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let blocked = blocked.expect("failed write was visible through the history route");
    assert_eq!(blocked["complete"], false);
    assert_eq!(blocked["sessionCursors"][0]["total"]["totalTokens"], 110);
    assert_eq!(blocked["scopedUsage"]["days"], first["days"]);
    db.execute_batch("DROP TRIGGER refuse_usage").unwrap();
    drop(db);
    cache.lock().unwrap().refreshed = None;
    let complete = settled(&home, &settings, &options);
    assert_eq!(complete["days"][0]["usage"]["totalTokens"], 220);
    drop(cache);
    clear(&home);
    let cold = settled(&home, &settings, &options);
    assert_eq!(cold["revision"], complete["revision"]);
    assert_eq!(cold["days"], complete["days"]);
    clear(&home);
}

#[test]
fn history_route_recovers_cold_archive_and_rejects_a_cached_result_after_source_removal() {
    let _serial = SERIAL.lock().unwrap();
    let (_root, home, settings, options) = fixture();
    let session = "00000000-0000-4000-8000-000000000011";
    let old_day = Local::now()
        .date_naive()
        .checked_sub_days(chrono::Days::new(65))
        .unwrap();
    let at = Local
        .from_local_datetime(&old_day.and_hms_opt(12, 0, 0).unwrap())
        .single()
        .unwrap();
    let timestamp = at.to_rfc3339();
    let count = json!({"input_tokens":100,"cached_input_tokens":20,"output_tokens":10});
    let archive = home.join("archived_sessions");
    fs::create_dir(&archive).unwrap();
    let original = archive.join("rollout-cold.jsonl");
    let rows = [
        json!({"type":"session_meta","timestamp":timestamp,"payload":{"id":session}}),
        json!({"type":"turn_context","timestamp":timestamp,"payload":{"model":"gpt-fixture","turn_id":"fixture"}}),
        json!({"type":"event_msg","timestamp":timestamp,"payload":{"type":"token_count","info":{
            "total_token_usage":count,"last_token_usage":count
        }}}),
    ];
    fs::write(
        &original,
        rows.iter()
            .map(|row| format!("{row}\n"))
            .collect::<String>(),
    )
    .unwrap();
    let revision = settled(&home, &settings, &options)["revision"].clone();
    let request = json!({
        "scopeSessionId":session, "includeTrend":true,
        "startAt":at.timestamp_millis(), "endAt":at.timestamp_millis()+60_000
    });
    let query = Query::parse(&request, Local::now().date_naive()).unwrap();
    let read = || query_with_settings(&home, &query, &request, settings.clone());
    let pending = read();
    assert_eq!(pending["archiveRecovery"]["status"], "pending");
    assert_eq!(pending["countStatus"]["complete"], false);
    assert!(pending["trend"].is_null());
    let mut recovered = read();
    for _ in 0..500 {
        if recovered["archiveRecovery"]["status"] != "pending" {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
        recovered = read();
    }
    assert_eq!(recovered["archiveRecovery"]["status"], "complete");
    assert_eq!(recovered["revision"], revision);
    assert_eq!(
        recovered["scopedUsage"]["days"][0]["usage"]["totalTokens"],
        110
    );
    assert_eq!(
        recovered["trend"]["buckets"][0]["usage"]["totalTokens"],
        110
    );
    assert_eq!(
        recovered["coverage"]["days"][old_day.to_string()],
        "complete"
    );
    fs::remove_file(&original).unwrap();
    let stale = read();
    assert_eq!(stale["revision"], revision);
    assert_eq!(stale["countStatus"]["complete"], false);
    assert!(stale["trend"].is_null());
    let cache = homes().lock().unwrap()[&home].clone();
    assert!(cache.lock().unwrap().archive.worker.is_some());
    clear(&home);
    let state = cache.lock().unwrap();
    assert!(state.archive.cancelled.load(Ordering::Relaxed));
    assert!(state.archive.worker.is_none());
}
