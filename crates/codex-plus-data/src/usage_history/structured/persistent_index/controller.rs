//! 独立的有界扫描入口；调用方完成查询接入后再决定是否启动。
use super::{BUDGET_BLOCKED, PersistentIndex, append::Appender, checkpoint, reclassification, retention, scan, sealed_retention};
use crate::usage_history::structured::{FileState, execution_index, index_budget, log_file, observed_file_identity};
use anyhow::{Context, ensure};
use chrono::NaiveDate;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::{self, ReadDir};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const MIGRATION_INTERVAL: Duration = Duration::from_secs(1);
const PURGE_INTERVAL: Duration = Duration::from_secs(60);
const METADATA_REPAIR_INTERVAL: Duration = Duration::from_secs(300);
const STORAGE_RETRY_INTERVAL: Duration = Duration::from_secs(60);
const MIN_RETRY_BYTES: u64 = 16 * 1024;
const MAX_STEPS: usize = 128;
const MAX_PENDING_SOURCES: usize = 64;

pub(in crate::usage_history::structured) struct ScanFailure {
    pub(in crate::usage_history::structured) source: PathBuf,
    pub(in crate::usage_history::structured) message: String,
}

pub(in crate::usage_history::structured) struct ScanReport {
    pub(in crate::usage_history::structured) revision: i64,
    pub(in crate::usage_history::structured) processed_sources: usize,
    pub(in crate::usage_history::structured) deferred_sources: usize,
    pub(in crate::usage_history::structured) bytes_read: u64,
    pub(in crate::usage_history::structured) discovery_finished: bool,
    pub(in crate::usage_history::structured) coverage_complete: bool,
    pub(in crate::usage_history::structured) storage_error: Option<String>,
    pub(in crate::usage_history::structured) storage_near_limit: bool,
    pub(in crate::usage_history::structured) migrating_sources: bool,
    pub(in crate::usage_history::structured) failures: Vec<ScanFailure>,
}

pub(in crate::usage_history::structured) struct Controller {
    home: PathBuf,
    index: PersistentIndex,
    appender: Appender,
    discovery: Option<Discovery>,
    active: Option<(PathBuf, FileState)>,
    priority: VecDeque<PathBuf>,
    deferred: VecDeque<PathBuf>,
    suspended: Option<(PathBuf, FileState)>,
    last_tick: Option<Instant>,
    last_purge: Option<Instant>,
    last_compact: Option<Instant>,
    compaction_cursor: Option<execution_index::CompactionCursor>,
    compaction_reduced: bool,
    compaction_pressure: bool,
    last_usage_compact: Option<Instant>,
    usage_compaction_cursor: Option<retention::Cursor>,
    last_observation_compact: Option<Instant>,
    observation_compaction_cursor: Option<sealed_retention::Cursor>,
    payload_compaction_cursor: Option<checkpoint::PayloadCursor>,
    payload_compaction_done: bool,
    payload_compaction_changed: bool,
    payload_compaction_retry_at: Option<Instant>,
    last_metadata_repair: Option<Instant>,
    metadata_repair_cursor: Option<execution_index::MetadataCursor>,
    paused_sources: BTreeSet<PathBuf>,
    storage_error: Option<String>,
    storage_retry_at: Option<Instant>,
    detail_reclaim_cursor: Option<execution_index::DetailReclaimCursor>,
    detail_reclaim_retry_at: Option<Instant>,
    retry_read_bytes: Option<u64>,
}

impl Controller {
    pub(in crate::usage_history::structured) fn open(
        home: &Path,
        database: &Path,
    ) -> anyhow::Result<Self> {
        let home = fs::canonicalize(home).context("cannot locate usage home")?;
        ensure!(home.is_dir(), "usage home is not a directory");
        Ok(Self {
            home,
            index: PersistentIndex::open(database)?,
            appender: Appender::default(),
            discovery: None,
            active: None,
            priority: VecDeque::new(),
            deferred: VecDeque::new(),
            suspended: None,
            last_tick: None,
            last_purge: None,
            last_compact: None,
            compaction_cursor: None,
            compaction_reduced: false,
            compaction_pressure: false,
            last_usage_compact: None,
            usage_compaction_cursor: None,
            last_observation_compact: None,
            observation_compaction_cursor: None,
            payload_compaction_cursor: None,
            payload_compaction_done: false,
            payload_compaction_changed: false,
            payload_compaction_retry_at: None,
            last_metadata_repair: None,
            metadata_repair_cursor: None,
            paused_sources: BTreeSet::new(),
            storage_error: None,
            storage_retry_at: None,
            detail_reclaim_cursor: None,
            detail_reclaim_retry_at: None,
            retry_read_bytes: None,
        })
    }

    pub(in crate::usage_history::structured) fn reclassify(
        &mut self,
        worker: &mut reclassification::Worker,
        input: &reclassification::Inputs<'_>,
    ) -> anyhow::Result<reclassification::Report> {
        worker.tick(&mut self.index, input)
    }

    pub(in crate::usage_history::structured) fn migrating_sources(&self) -> bool {
        !self.index.source_migration_complete
    }

    pub(in crate::usage_history::structured) fn prioritize_source(&mut self, source: &Path) -> anyhow::Result<()> {
        let source = fs::canonicalize(source).context("cannot locate priority usage source")?;
        let metadata = fs::metadata(&source)?;
        ensure!(
            (source.starts_with(self.home.join("sessions"))
                || source.starts_with(self.home.join("archived_sessions")))
                && metadata.is_file()
                && log_file(&source),
            "priority usage source is outside the session directories"
        );
        if self
            .active
            .as_ref()
            .is_some_and(|(active, _)| active == &source)
            || self
                .suspended
                .as_ref()
                .is_some_and(|(suspended, _)| suspended == &source)
            || self.priority.contains(&source)
        {
            return Ok(());
        }
        if let Some(position) = self.deferred.iter().position(|deferred| deferred == &source) {
            self.priority.push_back(self.deferred.remove(position).unwrap());
            return Ok(());
        }
        if let Some(saved) = self.index.source_state(&scan::encode_source(&source))? {
            if saved.complete && saved.scope_indexed && saved.execution_indexed {
                let state = checkpoint::restore(&saved.parser_state)?;
                let stamp = Some((
                    metadata.len(),
                    metadata.modified().ok(),
                    observed_file_identity(&source, &metadata)?,
                ));
                if state.stamp == stamp {
                    return Ok(());
                }
            }
        }
        ensure!(
            self.priority.len() + self.deferred.len() + usize::from(self.suspended.is_some())
                < MAX_PENDING_SOURCES,
            "priority usage source queue is full"
        );
        self.priority.push_back(source);
        Ok(())
    }

    /// 返回 None 表示尚未到下一轮；调用方应在后台周期调用。
    pub(in crate::usage_history::structured) fn tick(
        &mut self,
        now: Instant,
        now_ms: i64,
        cutoff: NaiveDate,
        provider_names: &BTreeMap<String, String>,
    ) -> anyhow::Result<Option<ScanReport>> {
        let interval = if self.migrating_sources() && self.storage_retry_at.is_none() {
            MIGRATION_INTERVAL
        } else {
            REFRESH_INTERVAL
        };
        if self
            .last_tick
            .is_some_and(|last| now.saturating_duration_since(last) < interval)
        {
            return Ok(None);
        }
        self.last_tick = Some(now);
        let page_size: u64 = self.index.db.pragma_query_value(None, "page_size", |row| row.get(0))?;
        let pages: u64 = self.index.db.pragma_query_value(None, "page_count", |row| row.get(0))?;
        let free: u64 = self.index.db.pragma_query_value(None, "freelist_count", |row| row.get(0))?;
        let max_pages = self.index.budget_bytes / page_size / 3;
        let mut report = ScanReport {
            revision: self.index.revision()?,
            processed_sources: 0,
            deferred_sources: 0,
            bytes_read: 0,
            discovery_finished: false,
            // 扫描结束不能证明来源、归类证据和历史覆盖均完整。
            coverage_complete: false,
            storage_error: self.storage_error.clone(),
            storage_near_limit: pages.saturating_sub(free).saturating_mul(5) >= max_pages.saturating_mul(4),
            migrating_sources: self.migrating_sources(),
            failures: Vec::new(),
        };
        if self.index.budget_blocked && self.storage_retry_at.is_some_and(|retry| now < retry) {
            return Ok(Some(report));
        }
        if self.index.budget_blocked {
            if self.storage_error.is_some() {
                match PersistentIndex::open_config(
                    &self.index.path,
                    self.index.budget_bytes,
                    false,
                ) {
                    Ok(index) => self.index = index,
                    Err(error) => {
                        let message = format!("{error:#}");
                        self.storage_error = Some(message.clone());
                        report.storage_error = Some(message);
                        self.storage_retry_at = Some(now + STORAGE_RETRY_INTERVAL);
                        return Ok(Some(report));
                    }
                }
            }
            if self.index.budget_blocked {
                self.storage_error = Some(BUDGET_BLOCKED.into());
                report.storage_error = self.storage_error.clone();
                self.storage_retry_at = Some(now + STORAGE_RETRY_INTERVAL);
                return Ok(Some(report));
            }
            self.storage_error = None;
            report.storage_error = None;
        }
        let mut detail_reclaim_failed = false;
        if self.storage_error.is_some()
            && self.detail_reclaim_retry_at.is_none_or(|retry| now >= retry)
        {
            match self.index.reclaim_execution_details(now_ms, self.detail_reclaim_cursor.as_ref()) {
                Ok(batch) => {
                    self.detail_reclaim_cursor = batch.next;
                    self.detail_reclaim_retry_at = None;
                    if batch.changed {
                        self.storage_retry_at = None;
                    }
                }
                Err(error) => {
                    if !storage_full(&error) {
                        return Err(error);
                    }
                    self.storage_error = Some(format!("{error:#}"));
                    report.storage_error = self.storage_error.clone();
                    self.storage_retry_at = Some(now + STORAGE_RETRY_INTERVAL);
                    self.detail_reclaim_retry_at = Some(now + STORAGE_RETRY_INTERVAL);
                    detail_reclaim_failed = true;
                }
            }
        } else if self.storage_error.is_none() {
            self.detail_reclaim_cursor = None;
            self.detail_reclaim_retry_at = None;
        }
        // 缩短既有记录可释放摄取空间，不能被摄取失败的退避计时器一并暂停。
        if !self.payload_compaction_done
            && self.payload_compaction_retry_at.is_none_or(|retry| now >= retry)
        {
            let result = self.index.compact_usage_payloads(self.payload_compaction_cursor.as_ref())
                .and_then(|(next, rewritten)| {
                    let changed = self.payload_compaction_changed || rewritten != 0;
                    if next.is_none() && (changed || self.storage_error.is_some()) {
                        self.index.repack_usage_payloads()?;
                        self.storage_retry_at = None;
                    }
                    Ok((next, rewritten, changed))
                });
            match result {
                Ok((next, rewritten, changed)) => {
                    self.payload_compaction_changed = changed;
                    self.payload_compaction_done = next.is_none();
                    self.payload_compaction_cursor = next;
                    self.payload_compaction_retry_at = None;
                    if rewritten != 0 {
                        self.storage_retry_at = None;
                    }
                }
                Err(error) => {
                    if !storage_full(&error) {
                        return Err(error);
                    }
                    self.storage_error = Some(format!("{error:#}"));
                    report.storage_error = self.storage_error.clone();
                    self.storage_retry_at = Some(now + STORAGE_RETRY_INTERVAL);
                    self.payload_compaction_retry_at = Some(now + STORAGE_RETRY_INTERVAL);
                    return Ok(Some(report));
                }
            }
        }
        if detail_reclaim_failed {
            self.storage_retry_at = Some(now + STORAGE_RETRY_INTERVAL);
        }
        if self
            .last_purge
            .is_none_or(|last| now.saturating_duration_since(last) >= PURGE_INTERVAL)
        {
            match self.index.purge_execution_details(now_ms) {
                Ok(changed) => {
                    self.last_purge = (!changed).then_some(now);
                }
                Err(error) => {
                    if !storage_full(&error) {
                        return Err(error);
                    }
                    self.storage_error = Some(format!("{error:#}"));
                    report.storage_error = self.storage_error.clone();
                    self.storage_retry_at = Some(now + STORAGE_RETRY_INTERVAL);
                    return Ok(Some(report));
                }
            }
        }
        if self.migrating_sources() {
            // 迁移会改变来源键；先完成压缩以保持游标稳定并为迁移腾出空间。
            if !self.payload_compaction_done
                || self.storage_retry_at.is_some_and(|retry| now < retry)
            {
                report.revision = self.index.revision()?;
                return Ok(Some(report));
            }
            match self.index.migrate_sources() {
                Ok(complete) => {
                    report.migrating_sources = !complete;
                    self.storage_retry_at = None;
                    if !complete {
                        report.revision = self.index.revision()?;
                        return Ok(Some(report));
                    }
                }
                Err(error) => {
                    if !storage_full(&error) {
                        return Err(error);
                    }
                    self.storage_error = Some(format!("{error:#}"));
                    report.storage_error = self.storage_error.clone();
                    self.storage_retry_at = Some(now + STORAGE_RETRY_INTERVAL);
                    report.revision = self.index.revision()?;
                    return Ok(Some(report));
                }
            }
        }
        let used_pages = pages.saturating_sub(free);
        let pressure = self.storage_error.is_some()
            || if self.compaction_pressure {
                used_pages.saturating_mul(4) >= max_pages.saturating_mul(3)
            } else {
                used_pages.saturating_mul(10) >= max_pages.saturating_mul(9)
            };
        if pressure != self.compaction_pressure {
            self.compaction_cursor = None;
            self.compaction_reduced = false;
            self.last_compact = None;
            self.compaction_pressure = pressure;
        }
        if self
            .last_compact
            .is_none_or(|last| now.saturating_duration_since(last) >= PURGE_INTERVAL)
        {
            match self
                .index
                .compact_execution_times(now_ms, self.compaction_cursor.as_ref(), pressure)
            {
                Ok(batch) => {
                    self.compaction_reduced |= batch.reduced;
                    self.compaction_cursor = batch.next;
                    self.last_compact = if self.compaction_cursor.is_none() {
                        let reduced = std::mem::take(&mut self.compaction_reduced);
                        (!reduced).then_some(now)
                    } else {
                        None
                    };
                }
                Err(error) => {
                    if !storage_full(&error) {
                        return Err(error);
                    }
                    self.storage_error = Some(format!("{error:#}"));
                    report.storage_error = self.storage_error.clone();
                    self.storage_retry_at = Some(now + STORAGE_RETRY_INTERVAL);
                    return Ok(Some(report));
                }
            }
        }
        if self
            .last_observation_compact
            .is_none_or(|last| now.saturating_duration_since(last) >= PURGE_INTERVAL)
        {
            match self.index.compact_usage_observations(
                now_ms,
                self.observation_compaction_cursor.as_ref(),
            ) {
                Ok(batch) => {
                    self.observation_compaction_cursor = batch.next;
                    self.last_observation_compact =
                        self.observation_compaction_cursor.is_none().then_some(now);
                }
                Err(error) => {
                    if !storage_full(&error) {
                        return Err(error);
                    }
                    self.storage_error = Some(format!("{error:#}"));
                    report.storage_error = self.storage_error.clone();
                    self.storage_retry_at = Some(now + STORAGE_RETRY_INTERVAL);
                    return Ok(Some(report));
                }
            }
        }
        if self
            .last_usage_compact
            .is_none_or(|last| now.saturating_duration_since(last) >= PURGE_INTERVAL)
        {
            match self
                .index
                .compact_usage_events(now_ms, self.usage_compaction_cursor.as_ref())
            {
                Ok(batch) => {
                    let finished = batch.deleted == 0;
                    self.usage_compaction_cursor = batch.next;
                    self.last_usage_compact = finished.then_some(now);
                }
                Err(error) => {
                    if !storage_full(&error) {
                        return Err(error);
                    }
                    self.storage_error = Some(format!("{error:#}"));
                    report.storage_error = self.storage_error.clone();
                    self.storage_retry_at = Some(now + STORAGE_RETRY_INTERVAL);
                    return Ok(Some(report));
                }
            }
        }
        // 摄取退避只限制读取新来源；到期记录仍须持续清理以释放空间。
        if self.storage_retry_at.is_some_and(|retry| now < retry) {
            report.revision = self.index.revision()?;
            return Ok(Some(report));
        }
        self.storage_retry_at = None;
        let scan_revision = self.index.revision()?;
        if self.discovery.is_none() {
            self.discovery = Some(Discovery::new(&self.home));
        }
        let mut budget =
            index_budget::Budget::new(self.suspended.iter().map(|(_, state)| state));
        let normal_read_bytes = budget.read_left;
        if let Some(limit) = self.retry_read_bytes {
            budget.read_left = budget.read_left.min(limit);
            budget.source_left = budget.source_left.min(limit);
        }
        if self.suspended.is_some() {
            budget.allow_second_reader();
        }
        for _ in 0..MAX_STEPS {
            if budget.read_left == 0 || budget.source_left == 0 {
                break;
            }
            if !self.priority.is_empty() && (self.active.is_none() || self.suspended.is_none()) {
                let path = self.priority.pop_front().unwrap();
                if let Some(suspended) = self.active.take() {
                    budget.replace(
                        index_budget::Usage::default(),
                        index_budget::Usage::of(&suspended.1),
                    );
                    budget.allow_second_reader();
                    self.suspended = Some(suspended);
                }
                match scan::restore_source(&self.index, &path) {
                    Ok(state) => self.active = Some((path, state)),
                    Err(error) => report.failures.push(ScanFailure {
                        source: path,
                        message: format!("{error:#}"),
                    }),
                }
            }
            if let Some((path, mut state)) = self.active.take() {
                let before = state.bytes_read;
                let read_before = budget.read_left;
                let result = scan::scan_source_resolved(
                    &mut self.index,
                    &mut state,
                    &path,
                    cutoff,
                    &mut budget,
                    |tx, sessions, batch| {
                        self.appender.resolve(tx, sessions, batch, provider_names)
                    },
                );
                report.bytes_read += state.bytes_read.saturating_sub(before);
                match result {
                    Ok(true) => report.processed_sources += 1,
                    Ok(false) if state.reader.is_some() => {
                        let progressed = state.bytes_read > before;
                        self.active = Some((path, state));
                        if !progressed {
                            report.deferred_sources += 1;
                            break;
                        }
                    }
                    Ok(false) => {
                        // 未结束的最后一行等待追加，不占用唯一 reader。
                        report.deferred_sources += 1;
                    }
                    Err(error) => {
                        if error.downcast_ref::<super::SealedReviewRequired>().is_some()
                            && self.paused_sources.len() < MAX_PENDING_SOURCES
                        {
                            self.paused_sources.insert(path.clone());
                        }
                        let full = storage_full(&error);
                        if full {
                            self.storage_error = Some(format!("{error:#}"));
                            report.storage_error = self.storage_error.clone();
                            let next_limit = (read_before / 2).max(MIN_RETRY_BYTES);
                            let at_minimum = self.retry_read_bytes == Some(MIN_RETRY_BYTES);
                            self.retry_read_bytes = Some(next_limit);
                            self.storage_retry_at = Some(
                                now + if at_minimum {
                                    STORAGE_RETRY_INTERVAL
                                } else {
                                    REFRESH_INTERVAL
                                },
                            );
                        }
                        report.failures.push(ScanFailure {
                            source: path.clone(),
                            message: format!("{error:#}"),
                        });
                        if full {
                            if let Some((suspended, _)) = self.suspended.take() {
                                self.deferred.push_back(suspended);
                            }
                            self.active = Some((path, state));
                            break;
                        }
                    }
                }
                continue;
            }
            if let Some(suspended) = self.suspended.take() {
                budget.replace(
                    index_budget::Usage::of(&suspended.1),
                    index_budget::Usage::default(),
                );
                self.active = Some(suspended);
                continue;
            }
            if let Some(path) = self.deferred.pop_front() {
                match scan::restore_source(&self.index, &path) {
                    Ok(state) => self.active = Some((path, state)),
                    Err(error) => report.failures.push(ScanFailure {
                        source: path,
                        message: format!("{error:#}"),
                    }),
                }
                continue;
            }
            match self.discovery.as_mut().unwrap().step(&self.home) {
                Ok(DiscoveryStep::Source(path)) if self.paused_sources.contains(&path) => {
                    report.deferred_sources += 1;
                }
                Ok(DiscoveryStep::Source(path)) => match scan::restore_source(&self.index, &path) {
                    Ok(state) => self.active = Some((path, state)),
                    Err(error) => report.failures.push(ScanFailure {
                        source: path,
                        message: format!("{error:#}"),
                    }),
                },
                Ok(DiscoveryStep::Progress) => {}
                Ok(DiscoveryStep::Finished) => {
                    self.discovery = None;
                    report.discovery_finished = true;
                    break;
                }
                Err(failure) => report.failures.push(failure),
            }
        }
        if report.storage_error.is_none()
            && self.last_metadata_repair.is_none_or(|last| {
                now.saturating_duration_since(last) >= METADATA_REPAIR_INTERVAL
            })
        {
            // 旧版本清掉的工具名称只做定点恢复，正常用量摄取先完成本轮预算。
            match self.index.repair_execution_metadata(
                &self.home,
                self.metadata_repair_cursor.as_ref(),
            ) {
                Ok(repair) => {
                    self.metadata_repair_cursor = repair.next;
                    self.last_metadata_repair =
                        self.metadata_repair_cursor.is_none().then_some(now);
                    report.failures.extend(
                        repair.failures.into_iter().map(|(source, message)| ScanFailure {
                            source,
                            message,
                        }),
                    );
                }
                Err(error) => {
                    if !storage_full(&error) {
                        return Err(error);
                    }
                    self.storage_error = Some(format!("{error:#}"));
                    report.storage_error = self.storage_error.clone();
                    self.storage_retry_at = Some(now + STORAGE_RETRY_INTERVAL);
                    report.revision = self.index.revision()?;
                    return Ok(Some(report));
                }
            }
        }
        report.revision = self.index.revision()?;
        if report.storage_error.is_some()
            && report.failures.is_empty()
            && (report.revision > scan_revision
                || self.active.is_none() && self.suspended.is_none())
        {
            self.storage_error = None;
            report.storage_error = None;
        }
        if report.storage_error.is_none() {
            if let Some(limit) = self.retry_read_bytes {
                let next = limit.saturating_mul(2).min(normal_read_bytes);
                self.retry_read_bytes = (next < normal_read_bytes).then_some(next);
            }
        }
        Ok(Some(report))
    }
}

fn storage_full(error: &anyhow::Error) -> bool {
    matches!(
        error.downcast_ref::<rusqlite::Error>(),
        Some(rusqlite::Error::SqliteFailure(error, _))
            if error.code == rusqlite::ErrorCode::DiskFull
    )
}

struct Discovery {
    roots: std::vec::IntoIter<PathBuf>,
    stack: Vec<(PathBuf, ReadDir)>,
}

enum DiscoveryStep {
    Source(PathBuf),
    Progress,
    Finished,
}

impl Discovery {
    fn new(home: &Path) -> Self {
        Self {
            roots: vec![home.join("sessions"), home.join("archived_sessions")].into_iter(),
            stack: Vec::new(),
        }
    }

    fn step(&mut self, home: &Path) -> Result<DiscoveryStep, ScanFailure> {
        if self.stack.is_empty() {
            let Some(root) = self.roots.next() else {
                return Ok(DiscoveryStep::Finished);
            };
            match fs::read_dir(&root) {
                Ok(entries) => self.stack.push((root, entries)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(ScanFailure {
                        source: root,
                        message: error.to_string(),
                    });
                }
            }
            return Ok(DiscoveryStep::Progress);
        }
        let (directory, entries) = self.stack.last_mut().unwrap();
        let Some(entry) = entries.next() else {
            self.stack.pop();
            return Ok(DiscoveryStep::Progress);
        };
        let entry = entry.map_err(|error| ScanFailure {
            source: directory.clone(),
            message: error.to_string(),
        })?;
        let path = entry.path();
        let kind = entry.file_type().map_err(|error| ScanFailure {
            source: path.clone(),
            message: error.to_string(),
        })?;
        if kind.is_dir()
            && path
                .strip_prefix(home)
                .is_ok_and(|relative| relative.components().count() <= 4)
        {
            let entries = fs::read_dir(&path).map_err(|error| ScanFailure {
                source: path.clone(),
                message: error.to_string(),
            })?;
            self.stack.push((path, entries));
        } else if kind.is_file() && log_file(&path) {
            let canonical = fs::canonicalize(&path).map_err(|error| ScanFailure {
                source: path.clone(),
                message: error.to_string(),
            })?;
            if !canonical.starts_with(home) {
                return Err(ScanFailure {
                    source: path,
                    message: "usage source is outside its home".into(),
                });
            }
            return Ok(DiscoveryStep::Source(canonical));
        }
        Ok(DiscoveryStep::Progress)
    }
}

#[cfg(test)]
mod tests;
