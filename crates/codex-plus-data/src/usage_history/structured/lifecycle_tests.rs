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
    assert!(collectors().lock().unwrap().contains_key(&home));
    let _ = query_with_settings(&home, &options, &json!({}), settings);
    clear(&home);
    assert!(!homes().lock().unwrap().contains_key(&home));
    assert!(!collectors().lock().unwrap().contains_key(&home));
}

#[test]
fn enabled_recording_captures_without_a_history_query_and_stops_when_disabled() {
    use fs2::FileExt;
    use std::io::Write;

    let _serial = SERIAL.lock().unwrap();
    let _request_log = codex_plus_core::request_log::request_log_test_guard();
    let (root, home, settings, options) = fixture();
    let request_log = root.path().join("requests.jsonl");
    let request = |id: &str| {
        let at = Local::now().timestamp_millis();
        json!({"id":id,"threadId":"00000000-0000-4000-8000-000000000001",
            "model":"provider/model","upstreamModel":"model","relayId":"provider","relayName":"Provider",
            "success":true,"state":"success","statusCode":200,"timestampMs":at,"startedAtMs":at-100,
            "attempts":1,"autoRetries":0,"finalResponseEvidence":"final-response-v1","finalResponseId":id,
            "usage":{"inputTokens":100,"outputTokens":10,"cachedTokens":20,"cachedTokensKnown":true}})
    };
    fs::write(&request_log, format!("{}\n", request("before-query"))).unwrap();
    codex_plus_core::request_log::set_request_log_path_for_tests(Some(request_log.clone()));
    let started = sync_provider_capture(&home, &settings);
    codex_plus_core::request_log::set_request_log_path_for_tests(None);
    started.unwrap();
    assert!(!homes().lock().unwrap().contains_key(&home));
    let storage = provider_cache::storage_dir(&home, &settings);
    let captured = |id: &str| {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if storage.join("usage-provider-cache.sqlite").exists() {
                let db = rusqlite::Connection::open_with_flags(
                    storage.join("usage-provider-cache.sqlite"),
                    rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
                )
                .unwrap();
                let found = db.query_row(
                    "SELECT count(*) FROM completed_proxy_requests r
                     JOIN completed_proxy_request_extra e USING(id)
                     WHERE r.id=?1 AND json_extract(r.data,'$.relayId')='provider'
                     AND json_extract(e.data,'$.final_response_id')=?1
                     AND json_extract(e.data,'$.cached_tokens_known')=1",
                    [id],
                    |row| row.get::<_, i64>(0),
                );
                if matches!(found, Ok(1)) {
                    break;
                }
            }
            assert!(Instant::now() < deadline, "request {id} was not captured");
            std::thread::sleep(Duration::from_millis(10));
        }
    };
    captured("before-query");
    let lock = File::open(storage.join("usage-provider-collector.lock")).unwrap();
    assert!(lock.try_lock_exclusive().is_err());
    sync_provider_capture(&home, &settings).unwrap();
    assert!(lock.try_lock_exclusive().is_err());
    assert!(!storage.join(super::super::TOKEN_INDEX_DB).exists());
    assert_eq!(settled(&home, &settings, &options)["complete"], true);
    let cache = homes().lock().unwrap()[&home].clone();
    cache.lock().unwrap().touched = Instant::now() - IDLE_TIMEOUT - Duration::from_secs(1);
    assert!(evict_idle(&home, &cache));
    let mut log = fs::OpenOptions::new()
        .append(true)
        .open(&request_log)
        .unwrap();
    writeln!(log, "{}", request("after-idle")).unwrap();
    drop(log);
    captured("after-idle");
    fs::write(&settings, r#"{"usageHistoryEnabled":false}"#).unwrap();
    sync_provider_capture(&home, &settings).unwrap();
    assert!(!collectors().lock().unwrap().contains_key(&home));
    lock.try_lock_exclusive().unwrap();
}

#[test]
fn third_home_keeps_recent_and_scanning_indexes_available() {
    let _serial = SERIAL.lock().unwrap();
    for scanning in [false, true] {
        let fixtures = [fixture(), fixture(), fixture()];
        let caches: Vec<_> = fixtures[..2]
            .iter()
            .map(|(_, home, settings, query)| {
                settled(home, settings, query);
                let cache = homes().lock().unwrap()[home].clone();
                cache.lock().unwrap().active = scanning;
                cache
            })
            .collect();
        let (_, home, settings, query) = &fixtures[2];
        let response = query_with_settings(home, query, &json!({}), settings.clone());
        let retained = {
            let registry = homes().lock().unwrap();
            fixtures
                .iter()
                .zip(&caches)
                .all(|((_, path, _, _), cache)| {
                    registry
                        .get(path)
                        .is_some_and(|current| Arc::ptr_eq(current, cache))
                })
        };
        for (_, home, _, _) in &fixtures {
            clear(home);
        }
        assert_ne!(response["status"], "unavailable", "scanning={scanning}");
        assert!(retained, "scanning={scanning}");
    }
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
fn history_route_reuses_published_days_without_exposing_interrupted_refresh() {
    let _serial = SERIAL.lock().unwrap();
    let (_root, home, settings, options) = fixture();
    let today = Local::now().date_naive();
    let midnight = |date: NaiveDate| {
        date.and_hms_opt(0, 0, 0)
            .unwrap()
            .and_local_timezone(Local)
            .single()
            .unwrap()
    };
    let at = midnight(today) + chrono::Duration::hours(1);
    let session = "00000000-0000-4000-8000-000000000039";
    let header = json!({"type":"session_meta","timestamp":at.to_rfc3339(),
        "payload":{"id":session,"model_provider":"fixture-supplier"}});
    let turn = json!({"type":"turn_context","timestamp":at.to_rfc3339(),
        "payload":{"turn_id":"fixture","model":"gpt-fixture"}});
    let usage = json!({"input_tokens":100,"cached_input_tokens":20,"output_tokens":10});
    let count = json!({"type":"event_msg","timestamp":at.to_rfc3339(),"payload":{
        "type":"token_count","info":{"total_token_usage":usage,"last_token_usage":usage}}});
    fs::write(
        home.join("sessions")
            .join(format!("rollout-{session}.jsonl")),
        format!("{header}\n{turn}\n{count}\n"),
    )
    .unwrap();
    let storage = provider_cache::storage_dir(&home, &settings);
    let deadline = Instant::now() + Duration::from_secs(5);
    let first = loop {
        let response = settled(&home, &settings, &options);
        if storage.join("published-usage-days-v1.json").is_file() {
            break response;
        }
        // A settled accounting response may precede the collector's first evidence publication.
        assert!(Instant::now() < deadline, "{response}");
        homes().lock().unwrap()[&home].lock().unwrap().refreshed = None;
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(first["days"][0]["usage"]["totalTokens"], 110);
    let request = json!({
        "from":today.to_string(),"to":today.to_string(),
        "startAt":midnight(today).timestamp_millis(),
        "endAt":midnight(today.succ_opt().unwrap()).timestamp_millis(),
        "includeFingerprints":false,"includeSessionMetadata":false,
    });
    let options = Query::parse(&request, today).unwrap();
    let first = query_with_settings(&home, &options, &request, settings.clone());
    let mut scoped_request = request.clone();
    scoped_request["scopeSessionId"] = json!(session);
    scoped_request["scopeOnly"] = json!(true);
    let scoped_committed = query_with_settings(&home, &options, &scoped_request, settings.clone());
    assert_eq!(scoped_committed["scopedUsage"]["days"], first["days"]);
    let mut clipped = request.clone();
    clipped["startAt"] = json!(at.timestamp_millis());
    clipped["endAt"] = json!(at.timestamp_millis() + 60_000);
    let clipped_committed = query_with_settings(&home, &options, &clipped, settings.clone());
    let mut clipped_scope = scoped_request.clone();
    clipped_scope["startAt"] = clipped["startAt"].clone();
    clipped_scope["endAt"] = clipped["endAt"].clone();
    let cache = homes().lock().unwrap()[&home].clone();
    let warm_readers = cache
        .lock()
        .unwrap()
        .runtime
        .as_ref()
        .unwrap()
        .pin_refresh_readers()
        .unwrap()
        .unwrap();
    let writer = rusqlite::Connection::open(storage.join(super::super::TOKEN_INDEX_DB)).unwrap();
    writer
        .execute_batch(
            "BEGIN; DELETE FROM token_days; DELETE FROM token_range_projection;
                        UPDATE token_sources SET checkpoint=CAST('{}' AS BLOB);
                        UPDATE token_revision SET revision=revision+1; COMMIT;",
        )
        .unwrap();
    let cold = token_checkpoint::Runtime::new(storage.clone(), settings.clone());
    {
        let mut state = cache.lock().unwrap();
        state.refresh_readers = cold.pin_refresh_readers().unwrap();
        state.snapshot = Arc::new(cold.initial_state());
        state.refreshed = None;
        state.active = true;
    }

    let read = |request: &Value| query_with_settings(&home, &options, request, settings.clone());
    let pending = read(&request);
    assert_eq!(pending["status"], "partial");
    assert_eq!(pending["scan"]["processing"], true);
    assert_eq!(pending["scan"]["lastPublished"], true, "{pending}");
    assert_eq!(pending["revision"], first["revision"]);
    assert_eq!(pending["days"], first["days"]);
    let scoped_pending = read(&scoped_request);
    assert_eq!(scoped_pending["scan"]["lastPublished"], true);
    assert_eq!(scoped_pending["revision"], scoped_committed["revision"]);
    assert_eq!(
        scoped_pending["sessionAliases"],
        scoped_committed["sessionAliases"]
    );
    assert_eq!(
        scoped_pending["scopedUsage"]["sessionIds"],
        json!([session])
    );
    assert_eq!(
        scoped_pending["scopedUsage"]["days"],
        scoped_committed["scopedUsage"]["days"]
    );
    assert_eq!(scoped_pending["scopedUsage"]["complete"], false);
    let clipped_cold = read(&clipped);
    assert_eq!(clipped_cold["scan"]["lastPublished"], true);
    assert_eq!(clipped_cold["revision"], first["revision"]);
    assert_eq!(clipped_cold["days"], clipped_committed["days"]);
    assert_eq!(clipped_cold["days"][0]["usage"]["totalTokens"], 110);
    let scoped_clip = read(&clipped_scope);
    assert_eq!(scoped_clip["scan"]["lastPublished"], true);
    assert_eq!(scoped_clip["revision"], clipped_cold["revision"]);
    assert_eq!(scoped_clip["days"], json!([]));
    assert_eq!(scoped_clip["scopedUsage"]["days"], clipped_cold["days"]);
    let mut excluded = clipped.clone();
    excluded["startAt"] = clipped["endAt"].clone();
    excluded["endAt"] = json!(at.timestamp_millis() + 120_000);
    assert_eq!(read(&excluded)["days"], json!([]));
    let mut excluded_scope = clipped_scope.clone();
    excluded_scope["startAt"] = excluded["startAt"].clone();
    excluded_scope["endAt"] = excluded["endAt"].clone();
    assert_eq!(read(&excluded_scope)["scopedUsage"]["days"], json!([]));
    let mut sub_minute = clipped.clone();
    sub_minute["startAt"] = json!(at.timestamp_millis() + 1);
    assert!(read(&sub_minute).get("revision").is_none());
    let mut sub_minute_scope = clipped_scope.clone();
    sub_minute_scope["startAt"] = sub_minute["startAt"].clone();
    assert!(read(&sub_minute_scope).get("scopedUsage").is_none());

    fs::write(
        settings.with_file_name("usage-provider-evidence.json"),
        "{}",
    )
    .unwrap();
    let rechecking = read(&request);
    assert!(rechecking.get("revision").is_none());
    assert!(rechecking.get("days").is_none());
    assert!(read(&clipped).get("revision").is_none());
    assert!(read(&scoped_request).get("scopedUsage").is_none());
    assert!(read(&clipped_scope).get("scopedUsage").is_none());

    let background = warm_readers.reader(true);
    cache.lock().unwrap().refresh_readers = Some(warm_readers);
    let held = background.lock().unwrap();
    let pinned = std::thread::scope(|scope| {
        let (send, receive) = std::sync::mpsc::channel();
        let read = &read;
        let clipped = &clipped;
        scope.spawn(move || send.send(read(clipped)).unwrap());
        let result = receive.recv_timeout(Duration::from_secs(2));
        drop(held);
        result.expect("clipped primary blocked behind the trend reader")
    });
    assert_eq!(pinned["scan"]["processing"], true);
    assert_eq!(pinned["revision"], first["revision"]);
    assert_eq!(pinned["days"], first["days"]);
    let mut trend_request = clipped;
    trend_request["includeTrend"] = json!(true);
    let trend = read(&trend_request);
    assert_eq!(trend["revision"], pinned["revision"]);
    assert_eq!(trend["days"], pinned["days"]);
    assert!(!trend["trend"]["buckets"].as_array().unwrap().is_empty());

    {
        let mut state = cache.lock().unwrap();
        state.refresh_readers = None;
        state.refreshed = Some(Instant::now());
        state.active = false;
    }
    let published = query_with_settings(&home, &options, &request, settings);
    assert_eq!(
        published["revision"],
        first["revision"].as_i64().unwrap() + 1
    );
    assert_eq!(published["days"], json!([]));
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
    let earlier = at - chrono::Duration::days(2);
    let other_session = "00000000-0000-4000-8000-000000000012";
    fs::write(
        archive.join("rollout-earlier.jsonl"),
        rows.iter()
            .map(|row| format!("{row}\n"))
            .collect::<String>()
            .replace(session, other_session)
            .replace(&timestamp, &earlier.to_rfc3339()),
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
    let cache = homes().lock().unwrap()[&home].clone();
    let raw_cache = fs::read_dir(
        cache
            .lock()
            .unwrap()
            .storage
            .join("usage-token-archives-v1"),
    )
    .unwrap()
    .next()
    .unwrap()
    .unwrap()
    .path();
    let raw_modified = fs::metadata(&raw_cache).unwrap().modified().unwrap();
    cache.lock().unwrap().archive.results[0].0 = Instant::now() - Duration::from_secs(301);
    let older = read();
    assert_eq!(older["archiveRecovery"]["status"], "complete");
    assert_eq!(
        older["scopedUsage"]["days"],
        recovered["scopedUsage"]["days"]
    );

    let evidence = settings.with_file_name("usage-provider-evidence.json");
    {
        let state = cache.lock().unwrap();
        let answer = &state.archive.results[0].1;
        let event = &answer.1.as_ref().unwrap().events[0];
        fs::write(
            &evidence,
            json!({"version":1,"records":[{
                "eventId":event.id.iter().map(|byte| format!("{byte:02x}")).collect::<String>(),
                "session":event.session,"timestamp":event.timestamp,"role":event.role,"model":event.model,
                "input":100,"cached":20,"output":10,"provider":"archive-fixture","providerName":"Archive Fixture",
                "source":"upstream-exact-bidirectional",
            }]}).to_string(),
        )
        .unwrap();
    }
    let supplier =
        |value: &Value| value["scopedUsage"]["days"][0]["breakdowns"][0]["provider"].clone();
    let mut confirmed = read();
    for _ in 0..500 {
        if supplier(&confirmed) == "archive-fixture" {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
        confirmed = read();
    }
    assert_eq!(supplier(&confirmed), "archive-fixture");
    assert_eq!(
        fs::metadata(&raw_cache).unwrap().modified().unwrap(),
        raw_modified
    );
    assert_eq!(
        confirmed["scopedUsage"]["days"][0]["usage"],
        recovered["scopedUsage"]["days"][0]["usage"]
    );
    let other_request = json!({
        "scopeSessionId":other_session, "includeTrend":true,
        "startAt":earlier.timestamp_millis(), "endAt":earlier.timestamp_millis()+60_000
    });
    let other_query = Query::parse(&other_request, Local::now().date_naive()).unwrap();
    let read_other = || query_with_settings(&home, &other_query, &other_request, settings.clone());
    let mut other = read_other();
    for _ in 0..500 {
        if other["archiveRecovery"]["status"] == "complete" {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
        other = read_other();
    }
    assert_eq!(other["archiveRecovery"]["status"], "complete");
    assert_eq!(other["scopedUsage"]["days"][0]["usage"]["totalTokens"], 110);
    assert_eq!(supplier(&read()), "archive-fixture");
    assert_eq!(cache.lock().unwrap().archive.results.len(), 2);

    clear(&home);
    settled(&home, &settings, &options);
    let cache = homes().lock().unwrap()[&home].clone();
    assert!(cache.lock().unwrap().archive.results.is_empty());
    let await_archive = |read: &dyn Fn() -> Value| {
        let mut response = read();
        for _ in 0..500 {
            if response["archiveRecovery"]["status"] == "complete" {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
            response = read();
        }
        response
    };
    let reopened = await_archive(&read);
    assert_eq!(reopened["archiveRecovery"]["status"], "complete");
    assert_eq!(
        reopened["scopedUsage"]["days"],
        confirmed["scopedUsage"]["days"]
    );
    assert_eq!(reopened["trend"], confirmed["trend"]);
    assert_eq!(cache.lock().unwrap().archive.results.len(), 1);
    assert_eq!(
        await_archive(&read_other)["scopedUsage"]["days"],
        other["scopedUsage"]["days"]
    );
    assert_eq!(
        fs::metadata(&raw_cache).unwrap().modified().unwrap(),
        raw_modified
    );

    fs::remove_file(&evidence).unwrap();
    let mut withdrawn = read();
    for _ in 0..500 {
        if withdrawn["archiveRecovery"]["status"] == "complete"
            && supplier(&withdrawn) != "archive-fixture"
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
        withdrawn = read();
    }
    assert_ne!(supplier(&withdrawn), "archive-fixture");
    assert_eq!(
        withdrawn["scopedUsage"]["days"][0]["usage"],
        confirmed["scopedUsage"]["days"][0]["usage"]
    );
    assert_eq!(
        fs::metadata(&raw_cache).unwrap().modified().unwrap(),
        raw_modified
    );
    clear(&home);
    settled(&home, &settings, &options);
    let reopened = await_archive(&read);
    assert_eq!(reopened["archiveRecovery"]["status"], "complete");
    assert_ne!(supplier(&reopened), "archive-fixture");
    let cache = homes().lock().unwrap()[&home].clone();
    fs::remove_file(&original).unwrap();
    let stale = read();
    assert_eq!(stale["revision"], revision);
    assert_eq!(stale["countStatus"]["complete"], false);
    assert!(stale["trend"].is_null());
    assert!(cache.lock().unwrap().archive.worker.is_some());
    clear(&home);
    let state = cache.lock().unwrap();
    assert!(state.archive.cancelled.load(Ordering::Relaxed));
    assert!(state.archive.worker.is_none());
}
