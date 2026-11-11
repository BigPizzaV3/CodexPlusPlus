//! 按需维护持久索引；查询只读取已提交结果，不等待后台扫描。
use super::{
    Query, execution_api, historical_classification, history_api, history_enabled, native_endpoints,
    persistent_index, provider_cache, provider_capture,
};
use anyhow::{Context, ensure};
use chrono::{Days, Local, NaiveDate};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const TICK_INTERVAL: Duration = Duration::from_secs(5);
const MIGRATION_INTERVAL: Duration = Duration::from_secs(1);
const MAX_HOMES: usize = 2;
const RECENT_SOURCES: usize = 16;
const SOURCE: &str = "persistent-local-rollout";

struct State {
    touched: Instant,
    stop: bool,
    provider_names: BTreeMap<String, String>,
    scan: Value,
    error: Option<String>,
    preferred_session: Option<String>,
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
}

struct Worker {
    settings: PathBuf,
    shared: Arc<Shared>,
    thread: JoinHandle<()>,
}

fn workers() -> &'static Mutex<HashMap<PathBuf, Worker>> {
    static WORKERS: OnceLock<Mutex<HashMap<PathBuf, Worker>>> = OnceLock::new();
    WORKERS.get_or_init(Default::default)
}

fn stop(worker: Worker) {
    {
        let mut state = worker
            .shared
            .state
            .lock()
            .expect("committed usage state poisoned");
        state.stop = true;
        worker.shared.wake.notify_one();
    }
    worker
        .thread
        .join()
        .expect("committed usage worker panicked");
}

pub(super) fn clear(home: &Path) {
    let mut workers = workers().lock().expect("committed usage registry poisoned");
    if let Some(worker) = workers.remove(home) {
        // 持有注册表锁直到旧任务退出，避免同一 home 同时启动两个写者。
        stop(worker);
    }
}

fn ensure_worker(home: &Path, settings: &Path) -> anyhow::Result<Arc<Shared>> {
    let mut workers = workers().lock().expect("committed usage registry poisoned");
    ensure!(history_enabled(settings), "本机用量历史未开启或设置不可用");
    let finished: Vec<_> = workers
        .iter()
        .filter(|(_, worker)| worker.thread.is_finished())
        .map(|(path, _)| path.clone())
        .collect();
    for path in finished {
        stop(workers.remove(&path).expect("finished worker disappeared"));
    }
    if let Some(worker) = workers.get(home) {
        ensure!(worker.settings == settings, "用量索引设置路径发生变化");
        let mut state = worker
            .shared
            .state
            .lock()
            .expect("committed usage state poisoned");
        state.touched = Instant::now();
        return Ok(worker.shared.clone());
    }
    ensure!(workers.len() < MAX_HOMES, "本机用量索引已达到实例上限");
    let shared = Arc::new(Shared {
        state: Mutex::new(State {
            touched: Instant::now(),
            stop: false,
            provider_names: BTreeMap::new(),
            scan: json!({"processing":true,"ready":false}),
            error: None,
            preferred_session: None,
        }),
        wake: Condvar::new(),
    });
    let owned_home = home.to_owned();
    let owned_settings = settings.to_owned();
    let worker_shared = shared.clone();
    let thread = std::thread::Builder::new()
        .name("committed-usage-index".into())
        .spawn(move || run(owned_home, owned_settings, worker_shared))
        .context("无法启动已提交用量索引")?;
    workers.insert(
        home.to_owned(),
        Worker {
            settings: settings.to_owned(),
            shared: shared.clone(),
            thread,
        },
    );
    Ok(shared)
}

pub(super) fn touch_for_executions(
    home: &Path,
    settings: &Path,
    session: Option<&str>,
) -> anyhow::Result<Value> {
    let shared = ensure_worker(home, settings)?;
    let mut state = shared.state.lock().expect("committed usage state poisoned");
    if let Some(session) = session {
        state.preferred_session = super::super::normalize_session_id(session);
    }
    Ok(state.scan.clone())
}

fn priority_sources(sqlite_home: &Path, session: Option<&str>) -> anyhow::Result<Vec<PathBuf>> {
    #[cfg(not(test))]
    let paths = codex_plus_core::codex_sqlite::codex_session_db_paths_from_home(sqlite_home);
    #[cfg(test)]
    let paths = vec![sqlite_home.join("state_5.sqlite")];
    for path in paths {
        if !path.try_exists()? {
            continue;
        }
        let db = Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        db.busy_timeout(Duration::from_millis(50))?;
        let columns: Vec<String> = db
            .prepare("PRAGMA table_info(threads)")?
            .query_map([], |row| row.get(1))?
            .collect::<Result<_, _>>()?;
        if columns.is_empty() {
            continue;
        }
        ensure!(
            ["id", "rollout_path", "updated_at"]
                .iter()
                .all(|column| columns.iter().any(|name| name == *column)),
            "会话数据库缺少最近用量索引所需的字段"
        );
        let mut sources = Vec::new();
        if let Some(session) = session {
            let source: Option<String> = db
                .query_row(
                    "SELECT rollout_path FROM threads WHERE id = ?1 AND rollout_path <> ''",
                    [session],
                    |row| row.get(0),
                )
                .optional()?;
            sources.extend(source.map(PathBuf::from));
        }
        let mut statement = db.prepare(
            "SELECT rollout_path FROM threads WHERE rollout_path <> ''
             ORDER BY updated_at DESC, id DESC LIMIT ?1",
        )?;
        for source in statement.query_map([RECENT_SOURCES as i64], |row| row.get::<_, String>(0))? {
            let source = PathBuf::from(source?);
            if !sources.contains(&source) {
                sources.push(source);
            }
        }
        return Ok(sources);
    }
    Ok(Vec::new())
}

fn run(home: PathBuf, settings: PathBuf, shared: Arc<Shared>) {
    let mut controller = None;
    let mut provider_stamp = None;
    let mut provider_names = BTreeMap::new();
    let mut endpoints = BTreeMap::new();
    let mut settings_error = None;
    let mut collector = None;
    let mut reclassification = persistent_index::reclassification::Worker::default();
    let storage = provider_cache::storage_dir(&home, &settings);
    #[cfg(not(test))]
    let (request_log, sqlite_home) = (
        codex_plus_core::request_log::request_log_path(),
        std::env::var_os("CODEX_SQLITE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.clone()),
    );
    #[cfg(test)]
    let (request_log, sqlite_home) = (home.join("request-log.jsonl"), home.clone());
    loop {
        {
            let state = shared.state.lock().expect("committed usage state poisoned");
            if state.stop || state.touched.elapsed() >= IDLE_TIMEOUT {
                break;
            }
        }
        if !history_enabled(&settings) {
            break;
        }
        let stamp = native_endpoints::config_stamp(&settings);
        if provider_stamp.as_ref() != Some(&stamp) {
            match native_endpoints::profiles(&settings) {
                Ok(profiles) => {
                    endpoints = native_endpoints::endpoints(&profiles);
                    provider_names = profiles
                        .into_iter()
                        .map(|profile| (profile.id, profile.name))
                        .collect();
                    provider_stamp = Some(stamp);
                    settings_error = None;
                }
                Err(error) => {
                    provider_names.clear();
                    endpoints.clear();
                    provider_stamp = None;
                    settings_error = Some(format!("{error:#}"));
                }
            }
        }
        let mut classification_error = settings_error.clone();
        if collector.is_none() {
            match provider_capture::start_provider_collector_with_log(
                home.clone(),
                sqlite_home.clone(),
                settings.clone(),
                request_log.clone(),
            ) {
                Ok(started) => collector = Some(started),
                Err(error) => {
                    classification_error = Some(format!("供应商证据采集未启动: {error:#}"))
                }
            }
        }
        let preferred_session = {
            let mut state = shared.state.lock().expect("committed usage state poisoned");
            state.scan["processing"] = json!(true);
            state.provider_names.clone_from(&provider_names);
            state.preferred_session.clone()
        };
        let mut classification = None;
        let result = (|| -> anyhow::Result<_> {
            if controller.is_none() {
                fs::create_dir_all(&storage).context("无法创建用量索引目录")?;
                controller = Some(persistent_index::controller::Controller::open(
                    &home,
                    &storage.join("usage-index.sqlite"),
                )?);
            }
            let controller = controller.as_mut().expect("controller was initialized");
            // 优先活动会话，避免首次回填的旧大文件挡住今天的数据。
            let mut priority_failures = Vec::new();
            match priority_sources(&sqlite_home, preferred_session.as_deref()) {
                Ok(sources) => {
                    for source in sources {
                        if let Err(error) = controller.prioritize_source(&source) {
                            priority_failures.push(persistent_index::controller::ScanFailure {
                                source,
                                message: format!("{error:#}"),
                            });
                        }
                    }
                }
                Err(error) => priority_failures.push(persistent_index::controller::ScanFailure {
                    source: sqlite_home.clone(),
                    message: format!("无法读取最近活动会话: {error:#}"),
                }),
            }
            let mut report = controller.tick(
                Instant::now(),
                Local::now().timestamp_millis(),
                NaiveDate::MIN,
                &provider_names,
            )?;
            if let Some(report) = &mut report {
                report.failures.extend(priority_failures);
            }
            if settings_error.is_none() && !controller.migrating_sources() {
                let result = (|| -> anyhow::Result<_> {
                    let cutoff =
                        Local::now().date_naive() - Days::new((super::RETENTION_DAYS - 1) as u64);
                    // 只读快照限于本批次，避免长期持有证据库 WAL。
                    let cache =
                        provider_cache::ProviderCache::read_evidence_snapshot(&storage, cutoff)?;
                    let endpoint_since = cache
                        .endpoint_binding_since_read_only(&endpoints)?
                        .unwrap_or(0);
                    controller.reclassify(
                        &mut reclassification,
                        &persistent_index::reclassification::Inputs {
                            home: &home,
                            request_log: &request_log,
                            cache: &cache,
                            endpoints: &endpoints,
                            endpoint_since,
                            provider_names: &provider_names,
                            now_ms: Local::now().timestamp_millis(),
                        },
                    )
                })();
                match result {
                    Ok(value) => {
                        if let Some(report) = &mut report {
                            report.revision = value.revision;
                        }
                        if !value.errors.is_empty() {
                            classification_error = Some(value.errors.join("; "));
                        }
                        classification = Some(value);
                    }
                    Err(error) => classification_error = Some(format!("{error:#}")),
                }
            }
            Ok(report)
        })();
        let mut state = shared.state.lock().expect("committed usage state poisoned");
        match result {
            Ok(Some(report)) => {
                state.error = None;
                state.scan = json!({
                    "processing":false,
                    "ready":true,
                    "revision":report.revision,
                    "processedSources":report.processed_sources,
                    "deferredSources":report.deferred_sources,
                    "bytesRead":report.bytes_read,
                    "discoveryFinished":report.discovery_finished,
                    "coverageComplete":report.coverage_complete,
                    "failedSources":report.failures.len(),
                    "storageError":report.storage_error,
                    "storageNearLimit":report.storage_near_limit,
                    "migratingSources":report.migrating_sources,
                    "classificationError":classification_error,
                    "classification":classification,
                });
            }
            Ok(None) => state.scan["processing"] = json!(false),
            Err(error) => {
                state.error = Some(format!("{error:#}"));
                state.scan["processing"] = json!(false);
                state.scan["error"] = json!(state.error);
                state.scan["classificationError"] = json!(classification_error);
            }
        }
        let interval = if state.scan["migratingSources"] == true
            && state.scan["storageError"].is_null()
        {
            MIGRATION_INTERVAL
        } else {
            TICK_INTERVAL
        };
        let (state, _) = shared
            .wake
            .wait_timeout(state, interval)
            .expect("committed usage state poisoned");
        if state.stop {
            break;
        }
    }
}

pub(super) fn query(home: &Path, settings: &Path, options: &Query, request: &Value) -> Value {
    let unavailable = |message: String| json!({"status":"unavailable","complete":false,"source":SOURCE,"message":message});
    if options.turn_id.is_some() || options.include_activity || options.include_session_cursors {
        return unavailable("持久索引暂不支持此用量查询".into());
    }
    if request.get("scopeSessionId").is_some_and(|value| {
        value
            .as_str()
            .and_then(super::super::normalize_session_id)
            .is_none()
    }) {
        return unavailable("会话范围 ID 无效".into());
    }
    let start = request.get("startAt").filter(|value| !value.is_null());
    let end = request.get("endAt").filter(|value| !value.is_null());
    if !matches!((start, end), (None, None))
        && start
            .and_then(Value::as_i64)
            .zip(end.and_then(Value::as_i64))
            .is_none_or(|(start, end)| start >= end)
    {
        return unavailable("用量时间范围无效".into());
    }
    let shared = match ensure_worker(home, settings) {
        Ok(shared) => shared,
        Err(error) => return unavailable(format!("{error:#}")),
    };
    let (names, scan, worker_error) = {
        let mut state = shared.state.lock().expect("committed usage state poisoned");
        if let Some(session) = request.get("scopeSessionId").and_then(Value::as_str) {
            state.preferred_session = super::super::normalize_session_id(session);
        }
        (
            state.provider_names.clone(),
            state.scan.clone(),
            state.error.clone(),
        )
    };
    let policy_path =
        provider_cache::storage_dir(home, settings).join(historical_classification::FILE_NAME);
    let (policy, classification_revision, classification_error) =
        match historical_classification::load(&policy_path) {
            Ok(Some(loaded)) => (Some(loaded.policy), loaded.revision, None),
            Ok(None) => (None, "none".to_owned(), None),
            Err(error) => (None, "error".to_owned(), Some(format!("{error:#}"))),
        };
    let result = execution_api::open_index(home, settings).and_then(|mut db| {
        history_api::query_with_policy(&mut db, options, request, &names, policy.as_deref())
    });
    let mut response = match result {
        Ok(mut response) => {
            response["scan"] = scan;
            response
        }
        Err(error) => {
            let message =
                worker_error.unwrap_or_else(|| format!("已提交用量索引尚不可用: {error:#}"));
            let mut response = unavailable(message);
            response["scan"] = scan;
            response
        }
    };
    response["classificationRevision"] = json!(classification_revision);
    response["historicalClassificationError"] = json!(classification_error);
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::io::Write;

    const SESSION: &str = "00000000-0000-4000-8000-000000000010";

    struct Fixture {
        _root: tempfile::TempDir,
        home: PathBuf,
        settings: PathBuf,
        source: PathBuf,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl Fixture {
        fn new() -> Self {
            static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
            let guard = LOCK.get_or_init(Default::default).lock().unwrap();
            let root = tempfile::tempdir().unwrap();
            let home = root.path().join("home");
            fs::create_dir_all(home.join("sessions")).unwrap();
            let home = fs::canonicalize(home).unwrap();
            let settings = root.path().join("settings.json");
            fs::write(&settings, r#"{"usageHistoryEnabled":true}"#).unwrap();
            let source = home.join("sessions/rollout-committed.jsonl");
            Self {
                _root: root,
                home,
                settings,
                source,
                _guard: guard,
            }
        }

        fn request(&self, request: &Value) -> Value {
            let options = Query::parse(request, Local::now().date_naive()).unwrap();
            query(&self.home, &self.settings, &options, request)
        }

        fn append_usage(&self, total: u64) {
            let event = json!({"type":"event_msg","timestamp":Local::now().to_rfc3339(),"payload":{
                "type":"token_count","info":{
                    "total_token_usage":{"input_tokens":total,"cached_input_tokens":0,"output_tokens":total / 10},
                    "last_token_usage":{"input_tokens":10,"cached_input_tokens":0,"output_tokens":1}
                }
            }});
            let mut source = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.source)
                .unwrap();
            writeln!(source, "{event}").unwrap();
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            clear(&self.home);
        }
    }

    fn wait_for(fixture: &Fixture, expected_input: u64) -> Value {
        let deadline = Instant::now() + Duration::from_secs(12);
        let request = json!({"snapshot":"committed","period":"all"});
        loop {
            let response = fixture.request(&request);
            if response["scan"]["ready"] == true
                && response["days"][0]["usage"]["inputTokens"] == expected_input
            {
                return response;
            }
            assert!(
                Instant::now() < deadline,
                "committed snapshot did not update: {response}"
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    #[test]
    fn invalid_or_disabled_requests_do_not_start_a_worker() {
        let fixture = Fixture::new();
        for request in [
            json!({"snapshot":"committed","startAt":2,"endAt":1}),
            json!({"snapshot":"committed","includeActivity":true}),
            json!({"snapshot":"committed","scopeSessionId":"invalid"}),
        ] {
            assert_eq!(fixture.request(&request)["status"], "unavailable");
            assert!(!workers().lock().unwrap().contains_key(&fixture.home));
        }
        fs::write(&fixture.settings, r#"{"usageHistoryEnabled":false}"#).unwrap();
        let response = fixture.request(&json!({"snapshot":"committed"}));
        assert_eq!(response["status"], "unavailable");
        assert_eq!(response["source"], SOURCE);
        assert!(!workers().lock().unwrap().contains_key(&fixture.home));
        assert!(
            !super::super::provider_cache::storage_dir(&fixture.home, &fixture.settings).exists()
        );
    }

    #[test]
    fn execution_requests_validate_before_starting_a_worker() {
        let fixture = Fixture::new();
        for request in [
            json!({}),
            json!({"startAt":2,"endAt":1}),
            json!({"startAt":1,"endAt":2,"scopeSessionId":"invalid"}),
            json!({"startAt":1,"endAt":2,"limit":0}),
            json!({"startAt":1,"endAt":2,"kind":"other"}),
            json!({"startAt":1,"endAt":2,"cursor":"bad","revision":1}),
        ] {
            assert_eq!(
                execution_api::query_executions(&fixture.home, &fixture.settings, &request)["status"],
                "unavailable"
            );
            assert!(!workers().lock().unwrap().contains_key(&fixture.home));
        }
    }

    #[test]
    fn executions_alone_start_the_index_and_expose_scan_status() {
        let fixture = Fixture::new();
        let timestamp = Local::now().to_rfc3339();
        fs::write(
            &fixture.source,
            format!(
                "{}\n{}\n{}\n{}\n",
                json!({"type":"session_meta","payload":{"id":SESSION}}),
                json!({"type":"response_item","timestamp":timestamp,"payload":{
                    "type":"function_call","call_id":"call","name":"functions.exec_command",
                    "arguments":"{\"cmd\":\"echo hello\"}"
                }}),
                json!({"type":"response_item","timestamp":timestamp,"payload":{
                    "type":"local_shell_call","id":"local","status":"completed",
                    "action":{"type":"exec","command":["echo","hello"],"working_directory":"C:\\work"}
                }}),
                json!({"type":"response_item","timestamp":timestamp,"payload":{
                    "type":"function_call_output","call_id":"local",
                    "output":"Process exited with code 0"
                }})
            ),
        )
        .unwrap();
        let now = Local::now().timestamp_millis();
        let request = json!({
            "startAt":now - 60_000,"endAt":now + 60_000,
            "scopeSessionId":SESSION,"kind":"tool"
        });
        let first = execution_api::query_executions(&fixture.home, &fixture.settings, &request);
        assert!(first["scan"]["processing"].is_boolean());
        assert!(workers().lock().unwrap().contains_key(&fixture.home));
        let deadline = Instant::now() + Duration::from_secs(12);
        loop {
            let response =
                execution_api::query_executions(&fixture.home, &fixture.settings, &request);
            if response["scan"]["ready"] == true && response["summary"]["recentTool"] == 2 {
                let items = response["items"].as_array().unwrap();
                assert!(items.iter().any(|item| item["id"] == "call"));
                assert!(items.iter().any(|item| item["id"] == "local"));
                let detail = execution_api::query_execution_detail(
                    &fixture.home,
                    &fixture.settings,
                    &json!({"sessionId":SESSION,"id":"local","revision":response["revision"]}),
                );
                assert_eq!(detail["status"], "ok");
                assert_eq!(detail["detail"]["input"]["command"][1], "hello");
                assert_eq!(detail["detail"]["input"]["working_directory"], "C:\\work");
                assert_eq!(detail["detail"]["output"][0], "Process exited with code 0");
                assert_eq!(detail["detail"]["durationMs"], 0);
                break;
            }
            assert!(Instant::now() < deadline, "execution index did not update: {response}");
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    #[test]
    fn executions_report_index_open_failure_in_scan_status() {
        let fixture = Fixture::new();
        let storage = super::super::provider_cache::storage_dir(&fixture.home, &fixture.settings);
        fs::create_dir_all(storage.join("usage-index.sqlite")).unwrap();
        let now = Local::now().timestamp_millis();
        let request = json!({"startAt":now - 1,"endAt":now + 1});
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let response =
                execution_api::query_executions(&fixture.home, &fixture.settings, &request);
            assert_eq!(response["status"], "unavailable");
            if response["scan"]["error"].is_string() {
                assert_eq!(response["message"], response["scan"]["error"]);
                break;
            }
            assert!(Instant::now() < deadline, "index error was not reported: {response}");
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    #[test]
    fn background_worker_opens_then_refreshes_committed_data_and_stops() {
        let fixture = Fixture::new();
        fs::write(
            &fixture.source,
            format!(
                "{}\n",
                json!({"type":"session_meta","payload":{"id":SESSION}})
            ),
        )
        .unwrap();
        fixture.append_usage(10);
        let first = wait_for(&fixture, 10);
        assert_eq!(first["source"], SOURCE);
        assert_eq!(first["status"], "partial");
        assert!(!first["revision"].is_null());
        assert_eq!(first["scan"]["coverageComplete"], false);
        assert!(
            !super::super::homes()
                .lock()
                .unwrap()
                .contains_key(&fixture.home)
        );

        fixture.append_usage(20);
        let updated = wait_for(&fixture, 20);
        assert!(updated["revision"].as_i64().unwrap() > first["revision"].as_i64().unwrap());
        fs::write(&fixture.settings, r#"{"usageHistoryEnabled":false}"#).unwrap();
        clear(&fixture.home);
        assert!(!workers().lock().unwrap().contains_key(&fixture.home));
        assert_eq!(
            fixture.request(&json!({"snapshot":"committed"}))["status"],
            "unavailable"
        );
    }

    #[test]
    fn historical_policy_error_update_and_delete_keep_usage_readable() {
        let fixture = Fixture::new();
        let today = Local::now().date_naive();
        let yesterday = today.pred_opt().unwrap();
        let at = Local.from_local_datetime(&yesterday.and_hms_opt(12, 0, 0).unwrap())
            .single().unwrap();
        fs::write(
            &fixture.source,
            format!(
                "{}\n{}\n",
                json!({"type":"session_meta","payload":{"id":SESSION}}),
                json!({"type":"event_msg","timestamp":at.to_rfc3339(),"payload":{
                    "type":"token_count","info":{
                        "total_token_usage":{"input_tokens":10,"cached_input_tokens":0,"output_tokens":1},
                        "last_token_usage":{"input_tokens":10,"cached_input_tokens":0,"output_tokens":1}
                    }
                }})
            ),
        ).unwrap();
        let original = wait_for(&fixture, 10);
        assert_eq!(original["classificationRevision"], "none");
        assert!(original["historicalClassificationError"].is_null());
        let storage = provider_cache::storage_dir(&fixture.home, &fixture.settings);
        let db = Connection::open_with_flags(
            storage.join("usage-index.sqlite"), OpenFlags::SQLITE_OPEN_READ_ONLY,
        ).unwrap();
        let (model, provider, role, conflicts): (String, String, String, u8) = db.query_row(
            "SELECT model, provider, role, context_conflicts FROM usage_events WHERE session = ?1",
            [SESSION],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        ).unwrap();
        assert!(super::super::unknown_model(&model));
        assert!(super::super::unknown_provider(&provider));
        let path = storage.join(historical_classification::FILE_NAME);
        fs::write(&path, "{invalid").unwrap();
        let request = json!({"snapshot":"committed","period":"all"});
        let invalid = fixture.request(&request);
        assert_eq!(invalid["classificationRevision"], "error");
        assert!(invalid["historicalClassificationError"].is_string());
        assert_eq!(invalid["days"][0]["usage"], original["days"][0]["usage"]);

        let cutoff = Local.from_local_datetime(&today.and_hms_opt(0, 0, 0).unwrap())
            .single().unwrap().timestamp_millis();
        let mut document = json!({
            "version":1,"cutoffMs":cutoff,"cutoffDay":today.to_string(),
            "rules":[{"day":yesterday.to_string(),"session":SESSION,"model":model,
                "provider":provider,"role":role,"conflicts":conflicts,
                "estimatedModel":"model-a","estimatedProvider":"provider-a"}]
        });
        fs::write(&path, document.to_string()).unwrap();
        let first = fixture.request(&request);
        assert_eq!(first["days"][0]["usage"], original["days"][0]["usage"]);
        assert_eq!(first["days"][0]["breakdowns"][0]["model"], "model-a");
        assert_eq!(first["days"][0]["breakdowns"][0]["modelInferred"], true);
        assert!(first["historicalClassificationError"].is_null());
        document["rules"][0]["estimatedModel"] = json!("another-model");
        fs::write(&path, document.to_string()).unwrap();
        let second = fixture.request(&request);
        assert_ne!(first["classificationRevision"], second["classificationRevision"]);
        assert_eq!(second["days"][0]["breakdowns"][0]["model"], "another-model");
        fs::remove_file(&path).unwrap();
        let revoked = fixture.request(&request);
        assert_eq!(revoked["classificationRevision"], "none");
        assert_eq!(revoked["days"], original["days"]);
    }

    #[test]
    fn committed_worker_reclassifies_late_request_evidence_without_new_usage() {
        let fixture = Fixture::new();
        fs::write(
            &fixture.source,
            format!(
                "{}\n",
                json!({"type":"session_meta","payload":{"id":SESSION}})
            ),
        )
        .unwrap();
        let at = Local::now().timestamp_millis();
        fixture.append_usage(10);
        let first = wait_for(&fixture, 10);
        let log = fixture.home.join("request-log.jsonl");
        fs::write(&log, format!("{}\n", json!({
            "id":"committed-late-request",
            "threadId":SESSION,
            "model":"model-a",
            "upstreamModel":"model-a",
            "relayId":"supplier-a",
            "relayName":"Renamed supplier",
            "success":true,
            "state":"success",
            "statusCode":200,
            "timestampMs":at,
            "startedAtMs":at - 10,
            "attempts":1,
            "autoRetries":0,
            "usage":{"inputTokens":10,"outputTokens":1,"cachedTokens":0,"cachedTokensKnown":true}
        }))).unwrap();
        let storage = provider_cache::storage_dir(&fixture.home, &fixture.settings);
        let db = rusqlite::Connection::open_with_flags(
            storage.join("usage-index.sqlite"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        db.busy_timeout(Duration::from_millis(100)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(12);
        loop {
            let response = fixture.request(&json!({"snapshot":"committed","period":"all"}));
            let provider: String = db
                .query_row("SELECT provider FROM usage_events", [], |row| row.get(0))
                .unwrap();
            if provider == "supplier-a" {
                assert_eq!(response["days"][0]["usage"], first["days"][0]["usage"]);
                assert!(
                    response["revision"].as_i64().unwrap() > first["revision"].as_i64().unwrap()
                );
                break;
            }
            assert!(
                Instant::now() < deadline,
                "late request was not classified: {response}"
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    #[test]
    fn index_open_failure_is_reported_without_falling_back_to_old_scan() {
        let fixture = Fixture::new();
        let storage = super::super::provider_cache::storage_dir(&fixture.home, &fixture.settings);
        fs::create_dir_all(storage.join("usage-index.sqlite")).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let result = fixture.request(&json!({"snapshot":"committed"}));
            if result["scan"]["error"].is_string() {
                assert_eq!(result["status"], "unavailable");
                assert_eq!(result["source"], SOURCE);
                assert!(
                    !super::super::homes()
                        .lock()
                        .unwrap()
                        .contains_key(&fixture.home)
                );
                break;
            }
            assert!(
                Instant::now() < deadline,
                "index open failure was not reported"
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    #[test]
    fn recent_source_lookup_is_bounded_and_keeps_the_requested_session_first() {
        let root = tempfile::tempdir().unwrap();
        let db = Connection::open(root.path().join("state_5.sqlite")).unwrap();
        db.execute_batch(
            "CREATE TABLE threads(id TEXT PRIMARY KEY, rollout_path TEXT, updated_at INTEGER);",
        )
        .unwrap();
        for index in 0..30 {
            db.execute(
                "INSERT INTO threads VALUES (?1, ?2, ?3)",
                rusqlite::params![
                    index.to_string(),
                    format!("rollout-{index}.jsonl"),
                    index
                ],
            )
            .unwrap();
        }
        let sources = priority_sources(root.path(), Some("0")).unwrap();
        assert_eq!(sources.len(), RECENT_SOURCES + 1);
        assert_eq!(sources[0], PathBuf::from("rollout-0.jsonl"));
        assert_eq!(sources[1], PathBuf::from("rollout-29.jsonl"));
        let sources = priority_sources(root.path(), Some("29")).unwrap();
        assert_eq!(sources.len(), RECENT_SOURCES);
        assert_eq!(sources[0], PathBuf::from("rollout-29.jsonl"));
    }

    #[test]
    fn recent_rollout_becomes_queryable_before_large_old_backfill_finishes() {
        let fixture = Fixture::new();
        let old_source = fixture.home.join("sessions/rollout-old.jsonl.zst");
        let old_header = json!({"type":"session_meta","payload":{
            "id":"00000000-0000-4000-8000-000000000011"
        }});
        let filler = format!(
            "{}\n",
            json!({"type":"response_item","payload":{"type":"message","content":"x".repeat(1024)}})
        );
        let old_contents = format!("{old_header}\n{}", filler.repeat(12_000));
        fs::write(
            &old_source,
            zstd::stream::encode_all(old_contents.as_bytes(), 1).unwrap(),
        )
        .unwrap();
        let storage = provider_cache::storage_dir(&fixture.home, &fixture.settings);
        fs::create_dir_all(&storage).unwrap();
        let mut controller = persistent_index::controller::Controller::open(
            &fixture.home,
            &storage.join("usage-index.sqlite"),
        )
        .unwrap();
        let start = Instant::now();
        let now = Local::now();
        let first = controller
            .tick(start, now.timestamp_millis(), NaiveDate::MIN, &BTreeMap::new())
            .unwrap()
            .unwrap();
        assert_eq!(first.bytes_read, 4 * 1024 * 1024);
        assert!(!first.discovery_finished);

        fs::write(
            &fixture.source,
            format!("{}\n", json!({"type":"session_meta","payload":{"id":SESSION}})),
        )
        .unwrap();
        fixture.append_usage(10);
        let threads = Connection::open(fixture.home.join("state_5.sqlite")).unwrap();
        threads
            .execute_batch(
                "CREATE TABLE threads(id TEXT PRIMARY KEY, rollout_path TEXT, updated_at INTEGER);",
            )
            .unwrap();
        threads
            .execute(
                "INSERT INTO threads VALUES (?1, ?2, ?3)",
                rusqlite::params![SESSION, fixture.source.to_str().unwrap(), now.timestamp()],
            )
            .unwrap();
        for source in priority_sources(&fixture.home, None).unwrap() {
            controller.prioritize_source(&source).unwrap();
        }
        let second = controller
            .tick(
                start + TICK_INTERVAL,
                now.timestamp_millis(),
                NaiveDate::MIN,
                &BTreeMap::new(),
            )
            .unwrap()
            .unwrap();
        assert!(second.failures.is_empty());
        assert!(!second.discovery_finished);
        let mut db = execution_api::open_index(&fixture.home, &fixture.settings).unwrap();
        let request = json!({"period":"today"});
        let options = Query::parse(&request, now.date_naive()).unwrap();
        let response = history_api::query(&mut db, &options, &request, &BTreeMap::new()).unwrap();
        assert_eq!(response["days"][0]["usage"]["inputTokens"], 10);
    }
}
