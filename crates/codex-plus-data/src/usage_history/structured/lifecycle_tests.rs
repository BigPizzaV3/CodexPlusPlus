use super::*;

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
