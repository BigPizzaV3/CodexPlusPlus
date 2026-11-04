//! 供应商证据按查询生命周期采集，缓存与采集锁保存在管理器状态目录。
use super::native_endpoints::timestamp_ms;
use super::*;
use rusqlite::{Connection, OptionalExtension, params};
use std::sync::mpsc;

const INTERVAL: Duration = Duration::from_secs(10);
const ID_WINDOW: i64 = 16_384;
const ROW_LIMIT: usize = 512;
const LOCK_DIAGNOSTIC_INTERVAL: Duration = Duration::from_secs(60);

pub struct ProviderCollector {
    stop: mpsc::Sender<()>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for ProviderCollector {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub fn start_provider_collector(
    home: PathBuf,
    sqlite_home: PathBuf,
    settings_path: PathBuf,
) -> io::Result<ProviderCollector> {
    start_provider_collector_with_log(
        home,
        sqlite_home,
        settings_path,
        codex_plus_core::request_log::request_log_path(),
    )
}

fn start_provider_collector_with_log(
    home: PathBuf,
    sqlite_home: PathBuf,
    settings_path: PathBuf,
    request_log: PathBuf,
) -> io::Result<ProviderCollector> {
    let (stop, receiver) = mpsc::channel();
    let thread = std::thread::Builder::new().name("supplier-evidence".into()).spawn(move || {
        let mut collector = Collector::default();
        let mut owner = None;
        let mut failed = false;
        let mut last_lock_diagnostic = None;
        loop {
            if owner.is_none() && home.is_dir() {
                match acquire(&home, &settings_path) {
                    Ok(lock) => owner = Some(lock),
                    Err(error) => {
                        if should_log_lock_error(&error, &mut last_lock_diagnostic, Instant::now()) {
                            let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                                "usage.provider_capture.lock_failed",
                                json!({"kind":format!("{:?}", error.kind()),"message":error.to_string()}),
                            );
                        }
                    }
                }
            }
            let mut pending = false;
            if owner.is_some() {
                match collector.tick(&home, &sqlite_home, &settings_path, &request_log) {
                    Ok(backlog) => { pending = backlog; failed = false; },
                    Err(_) => {
                        // 短暂锁库或半写配置下次重试；不影响请求转发或页面。
                        collector.settings_stamp = None;
                        if !failed {
                            let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                                "usage.provider_capture.retry",
                                json!({"reason":"local evidence temporarily unavailable"}),
                            );
                        }
                        failed = true;
                    }
                }
            }
            let delay = if pending { Duration::from_millis(250) } else { INTERVAL };
            if receiver.recv_timeout(delay) != Err(mpsc::RecvTimeoutError::Timeout) {
                // 只做最后一个有界批次，避免退出后还留有后台写入。
                if owner.is_some() { let _ = collector.tick(&home, &sqlite_home, &settings_path, &request_log); }
                return;
            }
        }
    })?;
    Ok(ProviderCollector {
        stop,
        thread: Some(thread),
    })
}

fn acquire(home: &Path, settings_path: &Path) -> io::Result<File> {
    use fs2::FileExt;
    let storage = provider_cache::storage_dir(home, settings_path);
    fs::create_dir_all(&storage)?;
    let path = storage.join("usage-provider-collector.lock");
    if fs::symlink_metadata(&path).is_ok_and(|m| !m.is_file()) {
        return Err(io::Error::other(
            "supplier collector lock is not a regular file",
        ));
    }
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    file.try_lock_exclusive().map_err(|error| {
        if error.raw_os_error().is_some()
            && error.raw_os_error() == fs2::lock_contended_error().raw_os_error()
        {
            io::Error::new(
                io::ErrorKind::WouldBlock,
                "supplier collector is already owned",
            )
        } else {
            error
        }
    })?;
    Ok(file)
}

fn should_log_lock_error(error: &io::Error, last: &mut Option<Instant>, now: Instant) -> bool {
    if error.kind() == io::ErrorKind::WouldBlock {
        return false;
    }
    if last.is_some_and(|last| now.duration_since(last) < LOCK_DIAGNOSTIC_INTERVAL) {
        return false;
    }
    *last = Some(now);
    true
}

#[derive(Default)]
struct Collector {
    cache: provider_cache::ProviderCache,
    settings_stamp: Option<native_endpoints::ConfigStamp>,
    endpoints: BTreeMap<String, Option<String>>,
    since: i64,
    requests: RequestEvidenceCache,
    sources: BTreeMap<PathBuf, Source>,
}

#[derive(Default)]
struct Source {
    identity: String,
    position: i64,
    anchor: String,
    floor: i64,
    stamp: Vec<Option<(u64, Option<SystemTime>, Option<(u64, u64)>)>>,
    pending: bool,
}

impl Source {
    fn seal(
        &self,
        cache: &mut provider_cache::ProviderCache,
        observed_at: i64,
    ) -> anyhow::Result<()> {
        if self.identity.is_empty() {
            return Ok(());
        }
        cache.capture_native(
            &self.identity,
            self.position,
            &[],
            true,
            if self.pending {
                self.floor.max(observed_at)
            } else {
                self.floor
            },
            &self.anchor,
        )
    }
}

fn stamp(path: &Path) -> Option<(u64, Option<SystemTime>, Option<(u64, u64)>)> {
    fs::symlink_metadata(path)
        .ok()
        .filter(|m| m.is_file())
        .map(|m| (m.len(), m.modified().ok(), file_identity(&m)))
}

fn cursor_anchor(db: &Connection, position: i64) -> anyhow::Result<String> {
    if position == 0 {
        return Ok("start".into());
    }
    let row: Option<(i64, i64, Option<String>)> = db
        .query_row(
            "SELECT ts,ts_nanos,target FROM logs WHERE id=?1",
            [position],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    Ok(match row {
        Some(row) => serde_json::to_string(&row)?,
        None => String::new(),
    })
}

fn verify_source(path: &Path, opened: &File, identity: &str) -> anyhow::Result<()> {
    use codex_plus_core::codex_sqlite::opened_file_identity;
    anyhow::ensure!(
        opened_file_identity(opened)? == opened_file_identity(&File::open(path)?)?
            && native_endpoints::source_identity(path)? == identity,
        "native log source changed during capture",
    );
    Ok(())
}

fn gap_floor(at: Option<i64>, observed_at: i64) -> i64 {
    at.filter(|at| *at > 0)
        .map(|at| at - 1)
        .unwrap_or(observed_at)
}

impl Collector {
    fn tick(
        &mut self,
        home: &Path,
        sqlite_home: &Path,
        settings: &Path,
        request_log: &Path,
    ) -> anyhow::Result<bool> {
        let cutoff = Local::now().date_naive() - Days::days(RETENTION_DAYS - 1);
        self.cache
            .open(&provider_cache::storage_dir(home, settings), cutoff)?;
        // 代理完成日志已实时落盘；在轮转前留存其最小匹配元数据。
        let before = self.requests.reads;
        self.requests.refresh(request_log);
        if self.requests.reads != before && self.requests.stamp.is_some() {
            if let Err(error) = self.cache.capture_proxy(&self.requests.records) {
                self.requests.stamp = None;
                return Err(error);
            }
        }
        let current = native_endpoints::config_stamp(settings);
        if self.settings_stamp.as_ref() != Some(&current) || self.since == 0 {
            // 配置读取失败不能继续沿用旧映射。
            self.endpoints.clear();
            self.settings_stamp = None;
            self.since = 0;
            if current.iter().any(Option::is_some) {
                let profiles = native_endpoints::profiles(settings)?;
                if native_endpoints::config_stamp(settings) != current {
                    anyhow::bail!("supplier settings changed during read");
                }
                self.endpoints = native_endpoints::endpoints(&profiles);
                self.since = self.cache.endpoint_binding_since(&self.endpoints)?;
                self.settings_stamp = Some(current);
            }
        }
        if self.since == 0 {
            return Ok(false);
        }
        let paths = native_endpoints::log_paths(sqlite_home)?;
        let observed_at = Local::now().timestamp_millis();
        let disappeared: Vec<_> = self
            .sources
            .keys()
            .filter(|path| !paths.contains(path))
            .cloned()
            .collect();
        for path in disappeared {
            self.sources[&path].seal(&mut self.cache, observed_at)?;
            self.sources.remove(&path);
        }
        let identities: anyhow::Result<HashSet<_>> = paths
            .iter()
            .map(|path| native_endpoints::source_identity(path))
            .collect();
        // 启动时也封存进程中断前遗留的孤儿 cursor；读取身份失败不能当作源已消失。
        let mut pending = if let Ok(identities) = identities {
            self.cache.retire_native_sources(&identities, observed_at)?
        } else {
            false
        };
        // 一个损坏的数据库不能阻止其他版本的日志继续采集。
        let mut failed = false;
        for path in paths {
            match self.read_source(&path) {
                Ok(backlog) => pending |= backlog,
                Err(_) => {
                    if let Some(source) = self.sources.get_mut(&path) {
                        source.stamp.clear();
                        source.pending = true;
                    }
                    failed = true;
                }
            }
        }
        if failed {
            anyhow::bail!("native supplier evidence temporarily unavailable");
        }
        Ok(pending)
    }

    fn read_source(&mut self, path: &Path) -> anyhow::Result<bool> {
        let observed_at = Local::now().timestamp_millis();
        let current: Vec<_> = codex_plus_core::codex_sqlite::codex_sqlite_sidecar_paths(path)
            .iter()
            .map(|p| stamp(p))
            .collect();
        if current[0].is_none() {
            return Ok(false);
        }
        // 保持文件句柄存活，避免路径替换后旧文件 ID 被立即复用。
        let opened = File::open(path)?;
        let source_id = native_endpoints::source_identity(path)?;
        let source = self.sources.entry(path.to_owned()).or_default();
        let db = native_endpoints::open_log(path)?;
        // high、旧锚点和本批数据必须来自同一个 SQLite 读快照。
        let snapshot = db.unchecked_transaction()?;
        let high: i64 =
            snapshot.query_row("SELECT coalesce(max(id),0) FROM logs", [], |row| row.get(0))?;
        verify_source(path, &opened, &source_id)?;
        let mut discontinuity = false;
        if source.identity != source_id {
            // 完整来源轮转不构成已知缺口；未读尾部丢失才抬高覆盖边界。
            discontinuity = !source.identity.is_empty() && source.pending;
            source.seal(&mut self.cache, observed_at)?;
            source.identity = source_id;
            let (position, anchor, floor) = self
                .cache
                .capture_cursor(&source.identity)?
                .unwrap_or((0, "start".into(), 0));
            source.position = position;
            source.anchor = anchor;
            source.floor = floor;
            source.stamp.clear();
        }
        let anchor = cursor_anchor(&snapshot, source.position)?;
        if source.position < 0
            || high < source.position
            || source.floor < 0
            || anchor.is_empty()
            || anchor != source.anchor
        {
            // 原文件 DELETE/重建后 high 可能更大；冲突必须重读旧 cursor 前的全部 ID。
            source.position = 0;
            source.anchor = "start".into();
            source.floor = source.floor.max(0);
            source.stamp.clear();
            discontinuity = true;
        }
        if source.stamp == current && !source.pending && source.position == high {
            return Ok(false);
        }
        let mut position = source.position;
        let mut anchor = source.anchor.clone();
        // 连续的 1..high（含空前缀）没有未扫描 ID；不能把配置 epoch 当覆盖边界。
        // 查询侧会再叠加缓存保留期限，旧映射请求仍以 provider=None 参与歧义判断。
        let mut floor = source.floor;
        let mut requests = Vec::new();
        if high > position {
            let first: i64 = snapshot.query_row(
                "SELECT id FROM logs WHERE id>?1 ORDER BY id LIMIT 1",
                [position],
                |row| row.get(0),
            )?;
            // 数字 ID 大缺口直接跳到首个真实行；每轮仍至多覆盖 16384 个主键。
            let upper = high.min(first.saturating_add(ID_WINDOW - 1));
            let mut checkpoint = upper;
            let mut last_target = position;
            let mut statement = snapshot.prepare(
                "SELECT id,ts,ts_nanos,
                substr(feedback_log_body,1,16385),thread_id
                FROM logs WHERE id>?1 AND id<=?2 AND target='codex_http_client::client'
                ORDER BY id LIMIT ?3",
            )?;
            let rows =
                statement.query_map(params![position, upper, (ROW_LIMIT + 1) as i64], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                })?;
            for (offset, row) in rows.enumerate() {
                let (id, seconds, nanos, body, session) = row?;
                if offset == ROW_LIMIT {
                    checkpoint = last_target;
                    break;
                }
                last_target = id;
                let (Some(body), Some(session)) = (body, session) else {
                    continue;
                };
                let Some(row) = native_endpoints::request(
                    &body,
                    &session,
                    (seconds, nanos),
                    &self.endpoints,
                    self.since,
                    &source.identity,
                    id,
                ) else {
                    continue;
                };
                let key =
                    Sha256::digest(serde_json::to_vec(&(&source.identity, id, row.at))?).into();
                requests.push((key, row));
            }
            drop(statement);
            // 只扫本预算内的元数据，检测内部缺口并把锚点落在真实的 checkpoint 行。
            let mut statement = snapshot.prepare(
                "SELECT id,ts,ts_nanos,target FROM logs WHERE id>?1 AND id<=?2 ORDER BY id",
            )?;
            let rows = statement.query_map(params![position, checkpoint], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })?;
            for row in rows {
                let (id, seconds, nanos, target) = row?;
                if discontinuity || id > position.saturating_add(1) {
                    floor = floor.max(gap_floor(timestamp_ms(seconds, nanos), observed_at));
                }
                discontinuity = false;
                position = id;
                anchor = serde_json::to_string(&(seconds, nanos, target))?;
            }
        }
        // 重建后仍为空时还没有后区首条日志，先用观测时间保守界定真实缺口。
        if discontinuity {
            floor = floor.max(observed_at);
        }
        verify_source(path, &opened, &source.identity)?;
        let pending = position < high;
        self.cache.capture_native(
            &source.identity,
            position,
            &requests,
            !pending,
            floor,
            &anchor,
        )?;
        source.position = position;
        source.anchor = anchor;
        source.floor = floor;
        source.stamp = current;
        source.pending = pending;
        Ok(source.pending)
    }
}

#[cfg(test)]
mod backend_recovery_tests {
    use super::*;

    #[test]
    fn gateway_only_changes_refresh_collector_and_invalid_config_clears_binding() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let sqlite = root.path().join("sqlite");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&sqlite).unwrap();
        let settings = root.path().join("settings.json");
        fs::write(&settings, json!({"relayProfiles": []}).to_string()).unwrap();
        let request_log = root.path().join("requests.jsonl");
        let mut collector = Collector::default();
        collector
            .tick(&home, &sqlite, &settings, &request_log)
            .unwrap();
        assert!(collector.endpoints.is_empty());
        let gateway = root.path().join("gateway.json");
        fs::write(
            &gateway,
            json!({"providers": [{"relay": {
                "id": "gateway-a", "name": "Gateway A", "baseUrl": "https://gateway.example/v1"
            }}]})
            .to_string(),
        )
        .unwrap();
        collector
            .tick(&home, &sqlite, &settings, &request_log)
            .unwrap();
        assert_eq!(
            collector.endpoints["https://gateway.example/v1/responses"].as_deref(),
            Some("gateway-a")
        );
        fs::write(&gateway, "{").unwrap();
        assert!(
            collector
                .tick(&home, &sqlite, &settings, &request_log)
                .is_err()
        );
        assert!(collector.endpoints.is_empty());
        assert_eq!(collector.since, 0);
        assert!(collector.settings_stamp.is_none());
        fs::write(&gateway, json!({"providers": []}).to_string()).unwrap();
        collector
            .tick(&home, &sqlite, &settings, &request_log)
            .unwrap();
        assert!(collector.endpoints.is_empty());
        assert!(collector.settings_stamp.is_some());
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;

    #[test]
    fn drop_joins_collector_and_releases_single_writer_lock() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let sqlite = root.path().join("sqlite");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&sqlite).unwrap();
        let settings = root.path().join("settings.json");
        let request_log = root.path().join("requests.jsonl");
        fs::write(&settings, r#"{"relayProfiles":[]}"#).unwrap();
        fs::write(&request_log, "").unwrap();
        let collector =
            start_provider_collector_with_log(home.clone(), sqlite, settings.clone(), request_log)
                .unwrap();
        let cache =
            provider_cache::storage_dir(&home, &settings).join("usage-provider-cache.sqlite");
        let deadline = Instant::now() + Duration::from_secs(5);
        // 缓存只在采集器持锁后创建，避免测试先抢到锁而触发采集器退避。
        while !cache.is_file() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            cache.is_file(),
            "collector did not initialize its isolated cache"
        );
        assert!(
            matches!(acquire(&home, &settings), Err(error) if error.kind() == io::ErrorKind::WouldBlock),
            "collector did not acquire its isolated writer lock"
        );
        drop(collector);
        let lock = acquire(&home, &settings).expect("Drop must join before releasing ownership");
        drop(lock);
    }

    #[test]
    fn distinct_homes_have_independent_writer_locks() {
        let root = tempfile::tempdir().unwrap();
        let settings = root.path().join("settings.json");
        let home_a = root.path().join("home-a");
        let home_b = root.path().join("home-b");
        fs::create_dir_all(&home_a).unwrap();
        fs::create_dir_all(&home_b).unwrap();
        let first = acquire(&home_a, &settings).unwrap();
        let second = acquire(&home_b, &settings).unwrap();
        assert!(
            matches!(acquire(&home_a, &settings), Err(error) if error.kind() == io::ErrorKind::WouldBlock)
        );
        drop(first);
        drop(second);
        assert!(acquire(&home_a, &settings).is_ok());
    }

    #[test]
    fn lock_contention_is_silent_and_real_failures_are_rate_limited() {
        let mut last = None;
        let at = Instant::now();
        assert!(!should_log_lock_error(
            &io::ErrorKind::WouldBlock.into(),
            &mut last,
            at
        ));
        assert!(last.is_none());
        let error = io::ErrorKind::PermissionDenied.into();
        assert!(should_log_lock_error(&error, &mut last, at));
        assert!(!should_log_lock_error(
            &error,
            &mut last,
            at + Duration::from_secs(1)
        ));
        assert!(should_log_lock_error(
            &error,
            &mut last,
            at + LOCK_DIAGNOSTIC_INTERVAL
        ));
    }
}
