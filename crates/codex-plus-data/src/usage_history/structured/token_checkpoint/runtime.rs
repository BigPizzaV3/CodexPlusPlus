use super::{
    disk_full,
    historical::{EvidenceOverlay, EvidenceVerifier, Historical},
    scan::{RECLAIM_BATCH, Scan},
    store::{
        Store,
        archive::{ArchiveKey, Recovered},
        executions::{DetailQuery, ExecutionQuery},
        projection::{ArchiveAnswer, classification_lookback_ms},
    },
};
use crate::usage_history::{
    Query, TOKEN_INDEX_DB,
    structured::{Index, RequestEvidence, index_budget::MAX_RETAINED_BYTES, live_receipts, read_history_settings},
};
use anyhow::{Context, Result, ensure};
use chrono::{Days, Local, NaiveDate};
use codex_plus_core::settings::USAGE_HISTORY_MAX_PRECISE_DAYS;
use rusqlite::{Connection, OpenFlags};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

const HISTORICAL_CLASSIFICATION_WINDOWS_PER_ROUND: usize = 2;

struct RecentClassification {
    latest: i64,
    before: i64,
    older_next: bool,
    tail_preempted: bool,
}

pub(in super::super) struct Runtime {
    storage: PathBuf,
    settings: PathBuf,
    store: Option<Store>,
    scan: Scan,
    retry_full: bool,
    evidence: Index,
    evidence_cursor: usize,
    evidence_before: BTreeMap<String, RecentClassification>,
    historical_cursor: Option<(i64, i64)>,
    historical: Option<Arc<Historical>>,
    historical_verifier: Arc<Mutex<EvidenceVerifier>>,
    published_days: Option<(published_days::Publication, Option<Arc<Historical>>)>,
}

mod published_days;

#[derive(Default)]
pub(in super::super) struct ReadState {
    provider_names: BTreeMap<String, String>,
    requests: Vec<RequestEvidence>,
    classification_error: Option<String>,
    storage_error: Option<String>,
    failed_sources: usize,
    retention_days: Option<u64>,
    historical: Option<Arc<Historical>>,
    historical_error: Option<String>,
    historical_verifier: Arc<Mutex<EvidenceVerifier>>,
}

pub(in super::super) type HistoryReader = Arc<Mutex<Connection>>;

pub(in super::super) struct RefreshReaders {
    summary: HistoryReader,
    detail: HistoryReader,
}

impl RefreshReaders {
    pub(in super::super) fn reader(&self, detailed: bool) -> HistoryReader {
        Arc::clone(if detailed { &self.detail } else { &self.summary })
    }
}

fn pin_reader(db: &Connection) -> Result<i64> {
    db.execute_batch("BEGIN")?;
    Ok(db.query_row("SELECT revision FROM token_revision", [], |row| row.get(0))?)
}

fn open_database(storage: &Path, write: bool) -> Result<Connection> {
    if write {
        fs::create_dir_all(storage)?;
    }
    let storage = fs::canonicalize(storage)?;
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let file = storage.join(format!("{TOKEN_INDEX_DB}{suffix}"));
        match fs::symlink_metadata(&file) {
            Ok(metadata) => ensure!(metadata.is_file(), "Token index path is not a regular file"),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    let flags = if write {
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
    } else {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    };
    let db = Connection::open_with_flags(
        storage.join(TOKEN_INDEX_DB),
        flags | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )?;
    // 冷备份切换 WAL 需要等前台读事务结束；后台写者不沿用前台的短等待。
    db.busy_timeout(if write {
        Duration::from_secs(5)
    } else {
        Duration::from_millis(50)
    })?;
    Ok(db)
}

impl Runtime {
    pub(in super::super) fn new(storage: PathBuf, settings: PathBuf) -> Self {
        Self {
            evidence: Index {
                storage_dir: Some(storage.clone()),
                ..Index::default()
            },
            storage,
            settings,
            store: None,
            scan: Scan::default(),
            retry_full: false,
            evidence_cursor: 0,
            evidence_before: BTreeMap::new(),
            historical_cursor: Some((0, i64::MAX)),
            historical: None,
            historical_verifier: Arc::default(),
            published_days: None,
        }
    }

    pub(in super::super) fn update(&mut self, home: &Path) -> ReadState {
        self.update_with_request_log(home, &codex_plus_core::request_log::request_log_path())
    }

    pub(in super::super) fn initial_state(&self) -> ReadState {
        ReadState {
            historical_verifier: Arc::clone(&self.historical_verifier),
            ..ReadState::default()
        }
    }

    pub(in super::super) fn pin_refresh_readers(&self) -> Result<Option<RefreshReaders>> {
        // 旧进程可能在入库与分类之间退出，冷启动先完成本轮刷新。
        if self.store.is_none() {
            return Ok(None);
        }
        let budget = u64::from(read_history_settings(&self.settings)?.storage_mib) * 1024 * 1024;
        let used = working_index_bytes(&self.storage)?;
        if used == 0 || used >= budget {
            return Ok(None);
        }
        let summary = open_database(&self.storage, false)?;
        // A read transaction must not block the first transition to WAL.
        if summary.pragma_query_value(None, "journal_mode", |row| row.get::<_, String>(0))? != "wal" {
            return Ok(None);
        }
        // 在写者启动前固定同一版本，趋势读取不会让前台摘要排队。
        let revision = pin_reader(&summary)?;
        let detail = open_database(&self.storage, false)?;
        ensure!(pin_reader(&detail)? == revision, "usage snapshot changed while opening refresh readers");
        Ok(Some(RefreshReaders {
            summary: Arc::new(Mutex::new(summary)),
            detail: Arc::new(Mutex::new(detail)),
        }))
    }

    fn update_with_request_log(&mut self, home: &Path, request_log: &Path) -> ReadState {
        let mut state = self.initial_state();
        let limits = match read_history_settings(&self.settings).and_then(|limits| {
            ensure!(limits.storage_mib > 0, "用量索引容量必须大于零");
            ensure!(
                (1..=USAGE_HISTORY_MAX_PRECISE_DAYS).contains(&limits.precise_days),
                "用量明细保留天数超出范围"
            );
            Ok(limits)
        }) {
            Ok(limits) => limits,
            Err(error) => {
                state.storage_error = Some(format!("{error:#}"));
                return state;
            }
        };
        let today = Local::now().date_naive();
        state.retention_days = Some(limits.precise_days);
        let cutoff = today - Days::new(limits.precise_days - 1);
        let proof_cutoff = today - chrono::Duration::days(super::super::RETENTION_DAYS - 1);
        // 明细保留期与证明缓存分开；缩短明细不迁移或裁剪已有供应商证明。
        let (_, evidence_available) = self.evidence.load_evidence(home, proof_cutoff, request_log);
        self.evidence.refresh_provider_profiles(&self.settings);
        self.evidence.request_evidence.refresh(request_log);
        state
            .provider_names
            .clone_from(&self.evidence.provider_names);
        let mut attribution_checked = false;
        match self.refresh(
            home,
            cutoff,
            u64::from(limits.storage_mib) * 1024 * 1024,
            &mut state,
        ) {
            Ok(true) if evidence_available => {
                if let Err(error) =
                    self.classify_retained(home, proof_cutoff, MAX_RETAINED_BYTES, request_log, &mut state)
                {
                    state.classification_error = Some(format!("{error:#}"));
                } else {
                    attribution_checked = true;
                }
            }
            Ok(_) if !evidence_available => {
                state.classification_error = Some("usage evidence cache unavailable".into());
            }
            Ok(_) => {}
            Err(error) => {
                state.storage_error = Some(format!("{error:#}"));
            }
        }
        if let Some(store) = &mut self.store {
            match Historical::refresh(
                self.historical.as_ref(),
                store,
                &self.historical_verifier,
                &self.settings,
            ) {
                Ok(evidence) => {
                    self.historical = evidence;
                    state.historical.clone_from(&self.historical);
                }
                Err(error) => state.historical_error = Some(format!("{error:#}")),
            }
        }
        state
            .requests
            .clone_from(&self.evidence.request_evidence.records);
        for request in &mut state.requests {
            let evidence = &self.evidence.request_evidence;
            if evidence.captured_id_check_error.is_some() || evidence.captured_rejected.contains(&request.id)
            {
                request.success = false;
            }
        }
        if attribution_checked
            && state.historical_error.is_none()
            && let Some(store) = &self.store
            && let Err(error) = published_days::save(
                store, &self.storage, &self.settings, &state, &mut self.published_days,
            )
        {
            eprintln!("saving published usage days: {error:#}");
        }
        state
    }

    fn classify_retained(
        &mut self,
        home: &Path,
        cutoff: NaiveDate,
        byte_budget: usize,
        request_log: &Path,
        state: &mut ReadState,
    ) -> Result<()> {
        // 扫描期间完成的网关请求也应参与本轮归属，避免已到达的回执要等下一轮。
        self.evidence.request_evidence.refresh(request_log);
        let store = self
            .store
            .as_mut()
            .expect("successful scan owns a usage index");
        let recent_since = Local::now().timestamp_millis() - super::RECENT_CLASSIFICATION_MS;
        let sessions = store.recent_sessions(recent_since)?;
        let active: std::collections::BTreeSet<_> = sessions.iter().map(|(id, _)| id).collect();
        self.evidence_before.retain(|id, _| active.contains(id));
        let slots = sessions.len() + usize::from(self.historical_cursor.is_some());
        if slots == 0 {
            return Ok(());
        }
        let offset = self.evidence_cursor % slots;
        self.evidence_cursor = offset + 1;
        let index = &mut self.evidence;
        let mut classify_page = |store: &mut Store, session: &str, from, to, shares: usize, remaining: &mut usize| {
            if *remaining == 0 {
                return Ok(None);
            }
            // 每轮首个窗口可完整读取竞争记录，后续窗口共享剩余预算。
            let available = if *remaining == byte_budget {
                usize::MAX
            } else {
                *remaining
            };
            let Some((window, bytes)) =
                store.classification_page(session, from, to, *remaining / shares, available)?
            else {
                return Ok(None);
            };
            *remaining = remaining.saturating_sub(bytes);
            let start = window.start;
            store.reclassify_window(
                session,
                Some(window),
                bytes,
                self.scan.scoped_count_gaps(),
                self.scan.sealed_pending(),
                self.scan.paired_only,
                |events, aliases, scope| {
                    ensure!(
                        index.native_evidence.observe_count_conflicts(events, true),
                        "native count conflict has an invalid timestamp"
                    );
                    index.request_evidence.supplement_captured_scoped(
                        events,
                        aliases,
                        &index.provider_cache,
                        Some(scope),
                    );
                    index.request_evidence.attribute_with_cache_scoped(
                        events,
                        &mut state.provider_names,
                        aliases,
                        &index.provider_cache,
                        Some(scope),
                    );
                    if let Some(error) = &index.request_evidence.captured_id_check_error {
                        anyhow::bail!("captured request validation failed: {error}");
                    }
                    index.native_evidence.attribute(
                        || super::super::native_endpoints::log_paths_for_home(home),
                        events,
                        &index.provider_endpoints,
                        index.provider_endpoint_since,
                        aliases,
                        &index.provider_cache,
                        true,
                        Some(scope),
                    );
                    index.native_evidence.apply_count_conflicts(events);
                    index.request_evidence.infer_scoped(events, &mut state.provider_names, aliases, scope)?;
                    Ok(())
                },
            )?;
            Ok::<_, anyhow::Error>(Some(start))
        };
        let mut failure = None;
        let mut remaining = byte_budget;
        for step in 0..slots {
            let slot = (offset + step) % slots;
            let shares = slots - step;
            if let Some((session, latest)) = sessions.get(slot) {
                let cursor = self.evidence_before.get(session);
                let preempt_old = cursor.is_some_and(|cursor| {
                    cursor.older_next && !cursor.tail_preempted && *latest > cursor.latest
                });
                let older = match cursor.filter(|cursor| cursor.older_next && !preempt_old) {
                    Some(cursor) => store.classification_before(session, cursor.before)?,
                    None => None,
                };
                // 新尾部可抢占一次旧页；下次兑现旧页，持续追加也不会饿死历史校验。
                let (from, to) = match (cursor, older) {
                    (_, Some(at)) => (i64::MIN, at),
                    (Some(cursor), None) => (
                        cursor
                            .latest
                            .min(*latest)
                            .saturating_sub(classification_lookback_ms()),
                        *latest,
                    ),
                    (None, None) => (recent_since, *latest),
                };
                match classify_page(store, session, from, to, shares, &mut remaining) {
                    Ok(Some(start)) => {
                        let before = match cursor {
                            Some(cursor) if !cursor.older_next || preempt_old => cursor.before,
                            _ => start,
                        };
                        self.evidence_before.insert(
                            session.clone(),
                            RecentClassification {
                                latest: match (cursor, older) {
                                    (Some(cursor), Some(_)) => cursor.latest,
                                    _ => *latest,
                                },
                                before,
                                older_next: older.is_none(),
                                tail_preempted: preempt_old,
                            },
                        );
                    }
                    Ok(None) => {}
                    Err(error) => {
                        failure.get_or_insert(error);
                    }
                }
            } else {
                for left in (1..=HISTORICAL_CLASSIFICATION_WINDOWS_PER_ROUND).rev() {
                    let (session_id, before) = self.historical_cursor.unwrap();
                    let Some((id, session, at)) =
                        store.historical_classification_before(session_id, before, recent_since)?
                    else {
                        self.historical_cursor = None;
                        break;
                    };
                    match classify_page(store, &session, i64::MIN, at, shares * left, &mut remaining) {
                        Ok(Some(start)) => self.historical_cursor = Some((id, start)),
                        Ok(None) => break,
                        Err(error) => {
                            failure.get_or_insert(error);
                            break;
                        }
                    }
                }
            }
        }
        // 旧页可以用完本轮份额，但已入库的新尾不能因此等待下一轮。
        // 独立份额仍逐页读写，不同时保留新旧窗口。
        let mut live_remaining = byte_budget;
        for (step, (session, latest)) in sessions
            .iter()
            .cycle()
            .skip(offset)
            .take(sessions.len())
            .enumerate()
        {
            let Some(cursor) = self
                .evidence_before
                .get_mut(session)
                .filter(|cursor| cursor.latest < *latest)
            else {
                continue;
            };
            let from = cursor.latest.saturating_sub(classification_lookback_ms());
            match classify_page(
                store, session, from, *latest, sessions.len() - step, &mut live_remaining,
            ) {
                Ok(Some(_)) => {
                    cursor.latest = *latest;
                    cursor.tail_preempted = cursor.older_next;
                    cursor.older_next = true;
                }
                Ok(None) => {}
                Err(error) => {
                    failure.get_or_insert(error);
                }
            }
        }
        index
            .provider_cache
            .save(&index.request_evidence.verified, &BTreeMap::new())?;
        let (_, available) = index
            .native_evidence
            .save_bounded(&self.storage, cutoff, &[])?;
        ensure!(available, "native count evidence has an unresolved global conflict");
        if let Some(error) = failure {
            return Err(error);
        }
        Ok(())
    }

    fn refresh(
        &mut self,
        home: &Path,
        cutoff: NaiveDate,
        budget: u64,
        state: &mut ReadState,
    ) -> Result<bool> {
        if self.store.is_none() {
            let db = open_database(&self.storage, true).context("opening usage index")?;
            let pages = db.pragma_query_value(None, "page_count", |row| row.get::<_, u64>(0))?;
            if pages == 0 {
                db.pragma_update(None, "auto_vacuum", "INCREMENTAL")?;
            }
            db.pragma_update(None, "synchronous", "FULL")?;
            self.store = Some(Store::open(db).context("initializing usage index")?);
        }
        let store = self.store.as_mut().expect("usage index is open");
        let precise_from: String = store.db.query_row(
            "SELECT precise_from FROM token_revision WHERE singleton=1",
            [],
            |row| row.get(0),
        )?;
        let cutoff = if precise_from.is_empty() {
            cutoff
        } else {
            cutoff.max(NaiveDate::parse_from_str(&precise_from, "%Y-%m-%d")?)
        };
        let page_size: u64 = store
            .db
            .pragma_query_value(None, "page_size", |row| row.get(0))?;
        // 给 WAL 留出余量；降低上限不能瞬间收缩已分配的主库。
        let page_limit = budget * 3 / 4 / page_size;
        let within_budget = checkpoint_if_over_budget(&store.db, &self.storage, budget)?;
        let (allocated, free) = allocated_pages(&store.db)?;
        if !within_budget || allocated - free >= page_limit || self.retry_full {
            match self.scan.maintain(store, cutoff) {
                Err(error) if !disk_full(&error) => {
                    return Err(error.context("maintaining usage index"));
                }
                _ => {}
            }
            let (allocated, free) = allocated_pages(&store.db)?;
            if allocated - free >= page_limit || (self.retry_full && free == 0) {
                store.evict_execution_days(cutoff, RECLAIM_BATCH)?;
            }
            if !within_budget || allocated > page_limit {
                store.db.execute_batch("PRAGMA incremental_vacuum(256)")?;
            }
            let (allocated, free) = allocated_pages(&store.db)?;
            ensure!(
                checkpoint_if_over_budget(&store.db, &self.storage, budget)?
                    && allocated - free < page_limit,
                "usage index working budget exhausted"
            );
        }
        store.db.pragma_update(None, "max_page_count", page_limit)?;
        store
            .db
            .pragma_update(None, "journal_size_limit", budget / 8)?;
        self.retry_full = false;
        let report = self.scan.round(
            store,
            home,
            cutoff,
            &state.provider_names,
            MAX_RETAINED_BYTES,
        );
        state.failed_sources = report.errors.len();
        if let Some(error) = report.maintenance_error {
            self.retry_full = disk_full(&error);
            return Err(error.context("maintaining usage index"));
        }
        if let Some((_, error)) = report
            .errors
            .into_iter()
            .find(|(_, error)| disk_full(error))
        {
            self.retry_full = true;
            return Err(error.context("writing usage index"));
        }
        Ok(report.can_classify)
    }
}

fn allocated_pages(db: &Connection) -> Result<(u64, u64)> {
    Ok((
        db.pragma_query_value(None, "page_count", |row| row.get(0))?,
        db.pragma_query_value(None, "freelist_count", |row| row.get(0))?,
    ))
}

// 长读事务会阻止 WAL 回收；先暂停摄取，避免每一轮继续累加日志。
fn working_index_bytes(storage: &Path) -> Result<u64> {
    let mut used = 0;
    for suffix in ["", "-wal", "-shm", "-journal"] {
        match fs::metadata(storage.join(format!("{TOKEN_INDEX_DB}{suffix}"))) {
            Ok(metadata) => used += metadata.len(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(used)
}

fn checkpoint_if_over_budget(db: &Connection, storage: &Path, budget: u64) -> Result<bool> {
    if working_index_bytes(storage)? >= budget {
        let (busy, _, _): (i64, i64, i64) =
            db.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?;
        ensure!(
            busy == 0,
            "usage index working budget exhausted (WAL reader active)"
        );
        return Ok(working_index_bytes(storage)? < budget);
    }
    Ok(true)
}

impl ReadState {
    pub(in super::super) fn open_reader(storage: &Path) -> Result<HistoryReader> {
        let db = open_database(storage, false)?;
        pin_reader(&db)?;
        Ok(Arc::new(Mutex::new(db)))
    }

    pub(in super::super) fn read_executions(
        &self,
        storage: &Path,
        request: &ExecutionQuery,
        processing: bool,
    ) -> Value {
        match open_database(storage, false).and_then(|mut db| {
            Store::read_execution_page(
                &mut db,
                request,
                Local::now().timestamp_millis(),
                processing,
                self.storage_error.as_deref(),
            )
        }) {
            Ok(response) => response,
            Err(error) => json!({
                "status":if processing {"partial"} else {"unavailable"},
                "message":error.to_string(), "complete":false,
                "items":[], "nextCursor":null, "coverage":{"complete":false},
                "scan":{"processing":processing,"storageError":self.storage_error}
            }),
        }
    }

    #[cfg(test)]
    pub(in super::super) fn read(
        &self,
        storage: &Path,
        options: &Query,
        request: &Value,
        scope_session: Option<&str>,
        processing: bool,
    ) -> Value {
        self.read_with_archive(
            storage, Self::open_reader(storage), options, request, scope_session, processing, &[],
        ).0
    }

    pub(in super::super) fn read_with_archive(
        &self,
        storage: &Path,
        reader: Result<HistoryReader>,
        options: &Query,
        request: &Value,
        scope_session: Option<&str>,
        processing: bool,
        archive: &[&ArchiveAnswer],
    ) -> (Value, Option<ArchiveKey>) {
        let (mut response, schedule) = match reader.and_then(|reader| {
            let db = reader.lock().map_err(|_| anyhow::anyhow!("usage snapshot reader panicked"))?;
            Store::read_snapshot(
                &db,
                options,
                request,
                scope_session,
                &self.provider_names,
                archive,
                self.historical.as_deref(),
            )
        }) {
            Ok(answer) => answer,
            Err(error) => (
                json!({
                    "status":if processing {"partial"} else {"unavailable"},
                    "complete":false, "message":error.to_string(),
                    "scan":{"ready":false,"discoveryFinished":false},
                }),
                None,
            ),
        };
        response["scan"]["processing"] = json!(processing);
        response["scan"]["failedSources"] = json!(self.failed_sources);
        if let Some(days) = self.retention_days {
            response["scan"]["retentionDays"] = json!(days);
        }
        if let Some(error) = &self.historical_error {
            response["historicalClassificationError"] = json!(error);
        }
        if let Some(error) = &self.classification_error {
            response["scan"]["classificationError"] = json!(error);
        }
        if self.classification_error.is_some() || self.storage_error.is_some() {
            if let Some(status) = response.get_mut("classificationStatus") {
                status["complete"] = json!(false);
            }
            if let Some(scope) = response.get_mut("scopedUsage") {
                scope["classificationStatus"]["complete"] = json!(false);
            }
        }
        if let Some(error) = &self.storage_error {
            response["scan"]["storageError"] = json!(error);
            response["scan"]["storageBlocked"] = json!(true);
            response["status"] = json!("partial");
            response["complete"] = json!(false);
            if let Some(status) = response.get_mut("countStatus") {
                status["complete"] = json!(false);
            }
            if let Some(scope) = response.get_mut("scopedUsage") {
                scope["complete"] = json!(false);
                scope["countStatus"]["complete"] = json!(false);
            }
        }
        response["liveAttributions"] = if request["liveReceipts"].is_array()
            && let Some(aliases) = response.get("sessionAliases")
        {
            let aliases =
                BTreeMap::deserialize(aliases).expect("snapshot emits a session alias map");
            live_receipts::match_captured_receipts(
                storage,
                &self.requests,
                &aliases,
                &request["liveReceipts"],
            )
        } else {
            json!([])
        };
        (response, schedule)
    }
}

pub(in super::super) fn read_execution_detail(
    storage: &Path,
    home: &Path,
    request: &DetailQuery,
) -> Value {
    match open_database(storage, false).and_then(|mut db| {
        Store::read_execution_detail(&mut db, home, request, || Local::now().timestamp_millis())
    }) {
        Ok(response) => response,
        Err(error) => json!({"status":"unavailable","message":error.to_string()}),
    }
}

pub(in super::super) fn recover_archive(
    key: ArchiveKey,
    home: &Path,
    storage: &Path,
    settings: &Path,
    snapshot: &ReadState,
    cancelled: &AtomicBool,
    previous: Option<&ArchiveAnswer>,
) -> ArchiveAnswer {
    let answer = (|| -> Result<(ArchiveKey, Recovered)> {
        let db = open_database(storage, false)?;
        let reused = match previous {
            Some((cached, Ok(recovered))) if cached == &key => recovered.reuse(&db, &key)?,
            _ => None,
        };
        let reused = match reused {
            Some(recovered) => Some((key.clone(), recovered)),
            None => Recovered::load_for(&db, storage, &key)?,
        };
        let needs_save = reused.is_none();
        let (covered, mut recovered) = if let Some(recovered) = reused {
            recovered
        } else {
            let mut replay = key.replay(&db, home, snapshot.provider_names.clone())?;
            loop {
                ensure!(
                    !cancelled.load(Ordering::Relaxed),
                    "archive replay cancelled"
                );
                if replay.step()? {
                    break (key.clone(), replay.finish(&db)?);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        ensure!(
            !cancelled.load(Ordering::Relaxed),
            "archive recovery cancelled"
        );
        recovered.evidence = Some(
            EvidenceOverlay::for_archive(
                &snapshot.historical_verifier,
                settings,
                &recovered.events,
                covered.session.is_some(),
            )
            .map_err(|error| format!("{error:#}")),
        );
        if needs_save && let Err(error) = recovered.save(&key, storage) {
            eprintln!("Usage archive cache write failed: {error:#}");
        }
        Ok((covered, recovered))
    })()
    .map_err(|error| format!("{error:#}"));
    match answer {
        Ok((covered, recovered)) => (covered, Ok(recovered)),
        Err(error) => (key, Err(error)),
    }
}

#[cfg(test)]
mod classification_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cold_seed_writer_waits_for_a_foreground_reader_before_enabling_wal() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(open_database(root.path(), true).unwrap()).unwrap();
        store
            .db
            .execute("UPDATE token_revision SET revision=6245", [])
            .unwrap();
        store.db.pragma_update(None, "journal_mode", "DELETE").unwrap();
        drop(store);

        let storage = root.path().to_owned();
        let settings = root.path().join("settings.json");
        fs::write(&settings, r#"{"usageHistoryEnabled":true}"#).unwrap();
        assert!(Runtime::new(storage.clone(), settings).pin_refresh_readers().unwrap().is_none());
        let (pinned, ready) = std::sync::mpsc::channel();
        let reader = std::thread::spawn(move || {
            let mut db = open_database(&storage, false).unwrap();
            let read = db.transaction().unwrap();
            let revision: i64 = read
                .query_row("SELECT revision FROM token_revision", [], |row| row.get(0))
                .unwrap();
            pinned.send(revision).unwrap();
            std::thread::sleep(Duration::from_millis(250));
            read.commit().unwrap();
        });
        assert_eq!(ready.recv().unwrap(), 6245);
        let opened = Store::open(open_database(root.path(), true).unwrap());
        reader.join().unwrap();
        let store = opened.expect("writer must wait for the foreground read to finish");
        assert_eq!(
            store
                .db
                .pragma_query_value(None, "journal_mode", |row| row.get::<_, String>(0))
                .unwrap(),
            "wal"
        );
        assert_eq!(read_revision(root.path()), 6245);
        store
            .db
            .execute("UPDATE token_revision SET revision=revision+1", [])
            .unwrap();
        assert_eq!(read_revision(root.path()), 6246);
    }

    #[test]
    fn storage_policy_applies_to_the_live_writer_without_rewinding_retained_history() {
        let root = tempfile::tempdir().unwrap();
        let settings = root.path().join("settings.json");
        let storage = root.path().join("index");
        let log = root.path().join("requests.jsonl");
        let home = root.path().join("home");
        fs::create_dir_all(home.join("sessions")).unwrap();
        let today = Local::now().date_naive();
        let mut runtime = Runtime::new(storage.clone(), settings.clone());
        let update = |runtime: &mut Runtime, mib: u64, days: u64| {
            fs::write(
                &settings,
                json!({
                    "usageHistoryEnabled": true,
                    "usageHistoryStorageMiB": mib,
                    "usageHistoryPreciseDays": days,
                })
                .to_string(),
            )
            .unwrap();
            let state = runtime.update_with_request_log(&home, &log);
            let request = json!({"period":"today","includeFingerprints":false});
            let query = Query::parse(&request, today).unwrap();
            assert!(state.storage_error.is_none(), "{:?}", state.storage_error);
            let read = state.read(&storage, &query, &request, None, false);
            assert_eq!(read["scan"]["retentionDays"], days);
            let db = &runtime.store.as_ref().unwrap().db;
            let page_size: u64 = db
                .pragma_query_value(None, "page_size", |row| row.get(0))
                .unwrap();
            let pages: u64 = db
                .pragma_query_value(None, "max_page_count", |row| row.get(0))
                .unwrap();
            let mode: u64 = db
                .pragma_query_value(None, "auto_vacuum", |row| row.get(0))
                .unwrap();
            assert_eq!(pages, mib * 1024 * 1024 * 3 / 4 / page_size);
            assert_eq!(mode, 2);
            read["scan"]["preciseFrom"].clone()
        };
        assert_eq!(
            update(&mut runtime, 2, 60),
            (today - Days::new(59)).to_string()
        );
        assert_eq!(
            update(&mut runtime, 3, 7),
            (today - Days::new(6)).to_string()
        );
        assert_eq!(
            update(&mut runtime, 3, 60),
            (today - Days::new(6)).to_string()
        );
    }

    #[test]
    fn page_pressure_reclaims_old_tool_rows_without_changing_token_or_source_state() {
        let root = tempfile::tempdir().unwrap();
        let settings = root.path().join("settings.json");
        let storage = root.path().join("index");
        let log = root.path().join("requests.jsonl");
        let home = root.path().join("home");
        fs::create_dir_all(home.join("sessions")).unwrap();
        fs::write(&settings, r#"{"usageHistoryEnabled":true}"#).unwrap();
        let mut runtime = Runtime::new(storage, settings);
        assert!(
            runtime
                .update_with_request_log(&home, &log)
                .storage_error
                .is_none()
        );
        let store = runtime.store.as_mut().unwrap();
        let today = Local::now().date_naive();
        let cutoff = today - Days::new(59);
        let oldest = cutoff.pred_opt().unwrap().to_string();
        store
            .db
            .execute(
                "WITH RECURSIVE n(id) AS (VALUES(1) UNION ALL SELECT id+1 FROM n WHERE id<2048)
             INSERT INTO token_sessions SELECT id, printf('fixture-%i',id) FROM n",
                [],
            )
            .unwrap();
        store
            .db
            .execute(
                "INSERT INTO execution_daily SELECT ?1,id,2,0,0,2,0 FROM token_sessions",
                [oldest.as_str()],
            )
            .unwrap();
        store
            .db
            .execute(
                "INSERT INTO execution_daily VALUES(?1,1,3,0,0,3,0)",
                [cutoff.to_string()],
            )
            .unwrap();
        let (allocated, free) = allocated_pages(&store.db).unwrap();
        let page_size: u64 = store
            .db
            .pragma_query_value(None, "page_size", |r| r.get(0))
            .unwrap();
        let budget = (allocated - free) * page_size * 4 / 3;
        // The first below-cap maintenance round must actually delete at most one batch.
        let mut state = runtime.initial_state();
        let _first = runtime.refresh(&home, cutoff, budget, &mut state);
        let store = runtime.store.as_mut().unwrap();
        let remaining: u32 = store
            .db
            .query_row(
                "SELECT count(*) FROM execution_daily WHERE day=?1",
                [oldest.as_str()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 2048 - RECLAIM_BATCH);
        assert_eq!(
            store
                .db
                .query_row("SELECT through_day FROM execution_evicted", [], |r| r
                    .get::<_, String>(0))
                .unwrap(),
            oldest
        );
        assert_eq!(
            store
                .db
                .query_row(
                    "SELECT tool FROM execution_daily WHERE day=?1",
                    [cutoff.to_string()],
                    |r| r.get::<_, u64>(0)
                )
                .unwrap(),
            3
        );
        assert_eq!(
            store
                .db
                .query_row("SELECT count(*) FROM token_sessions", [], |r| r
                    .get::<_, u64>(0))
                .unwrap(),
            2048
        );
    }

    #[test]
    fn working_budget_waits_for_pinned_wal_without_invalidating_reads() {
        let root = tempfile::tempdir().unwrap();
        let db = open_database(root.path(), true).unwrap();
        let store = Store::open(db).unwrap();
        store
            .db
            .pragma_update(None, "wal_autocheckpoint", 0)
            .unwrap();
        store
            .db
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .unwrap();
        let mut read = open_database(root.path(), false).unwrap();
        let pin = read.transaction().unwrap();
        let snapshot: i64 = pin
            .query_row("SELECT revision FROM token_revision", [], |row| row.get(0))
            .unwrap();
        let budget = fs::metadata(root.path().join(TOKEN_INDEX_DB))
            .unwrap()
            .len()
            + 64 * 1024;
        for _ in 0..64 {
            store
                .db
                .execute("UPDATE token_revision SET revision = revision + 1", [])
                .unwrap();
        }
        let error = checkpoint_if_over_budget(&store.db, root.path(), budget).unwrap_err();
        assert!(error.to_string().contains("working budget exhausted"));
        assert_eq!(
            pin.query_row("SELECT revision FROM token_revision", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            snapshot,
        );
        assert_eq!(read_revision(root.path()), snapshot + 64,);
        pin.commit().unwrap();
        assert!(checkpoint_if_over_budget(&store.db, root.path(), budget).unwrap());
        assert_eq!(read_revision(root.path()), snapshot + 64);
    }

    fn read_revision(storage: &Path) -> i64 {
        open_database(storage, false)
            .unwrap()
            .query_row("SELECT revision FROM token_revision", [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn cold_historical_attribution_is_batched_and_preserves_counts_and_conflicts() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        fs::create_dir_all(home.join("sessions")).unwrap();
        let home = fs::canonicalize(home).unwrap();
        let settings = root.path().join("settings.json");
        fs::write(&settings, r#"{"usageHistoryEnabled":true}"#).unwrap();
        let request_log = root.path().join("requests.jsonl");
        let now = Local::now();
        let at = now - chrono::Duration::days(3);
        let stamp = at.to_rfc3339();
        let usage = json!({"input_tokens":100,"cached_input_tokens":20,"output_tokens":10});
        let mut log = String::new();
        for i in 1..=4 {
            let session = format!("00000000-0000-4000-8000-{i:012}");
            let model = format!("gpt-fixture-{i}");
            let meta = json!({"type":"session_meta","timestamp":stamp,
                "payload":{"id":session,"model_provider":"custom","source":"vscode"}});
            let turn = json!({"type":"turn_context","timestamp":stamp,
                "payload":{"turn_id":"test-turn","model":model}});
            let count = json!({"type":"event_msg","timestamp":stamp,"payload":{
                "type":"token_count","info":{"total_token_usage":usage,"last_token_usage":usage}}});
            fs::write(
                home.join(format!("sessions/rollout-{i}.jsonl")),
                format!("{meta}\n{turn}\n{count}\n"),
            )
            .unwrap();
            let mut proof = json!({"id":format!("proof-{i}"),"threadId":session,"model":model,
                "upstreamModel":model,"relayId":"fixture-relay","relayName":"Fixture",
                "success":true,"state":"success","statusCode":200,"timestampMs":at.timestamp_millis(),
                "startedAtMs":at.timestamp_millis()-100,"attempts":1,"autoRetries":0,
                "usage":{"inputTokens":100,"cachedTokens":20,"outputTokens":10}});
            log.push_str(&format!("{proof}\n"));
            if i == 4 {
                proof["id"] = json!("competing-proof");
                proof["relayId"] = json!("competing-relay");
                log.push_str(&format!("{proof}\n"));
            }
        }
        fs::write(&request_log, log).unwrap();
        let day = at.date_naive().to_string();
        let request = json!({"from":day,"to":day});
        let query = Query::parse(&request, now.date_naive()).unwrap();
        let exact_models = |result: &Value| {
            let mut models: Vec<_> = result["days"][0]["breakdowns"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|row| row["classificationEvidence"] == "request-log-exact")
                .map(|row| row["model"].as_str().unwrap().to_owned())
                .collect();
            models.sort_unstable();
            models
        };
        // 冷启动按文件修改时间摄取，使有竞争证明的会话分别落在第二批、第一批。
        for (order, expected_first) in [
            ([1, 2, 3, 4], &["gpt-fixture-1", "gpt-fixture-2"][..]),
            ([3, 4, 1, 2], &["gpt-fixture-3"][..]),
        ] {
            for (rank, i) in order.iter().enumerate() {
                let path = home.join(format!("sessions/rollout-{i}.jsonl"));
                let modified =
                    std::time::UNIX_EPOCH + Duration::from_secs(1_700_000_000 - rank as u64 * 60);
                fs::File::options()
                    .write(true)
                    .open(path)
                    .unwrap()
                    .set_modified(modified)
                    .unwrap();
            }
            let storage = root.path().join(format!("history-{}", order[0]));
            let mut worker = Runtime::new(storage.clone(), settings.clone());
            let pass = |worker: &mut Runtime| {
                let state = worker.update_with_request_log(&home, &request_log);
                assert!(
                    state.classification_error.is_none(),
                    "{:?}",
                    state.classification_error
                );
                state.read(&storage, &query, &request, None, false)
            };
            let first = pass(&mut worker);
            assert_eq!(exact_models(&first), expected_first, "{first}");
            assert_eq!(worker.historical_cursor.unwrap().0, 2);
            let second = pass(&mut worker);
            assert_eq!(
                exact_models(&second),
                ["gpt-fixture-1", "gpt-fixture-2", "gpt-fixture-3"],
                "{second}"
            );
            assert_eq!(second["days"][0]["usage"], first["days"][0]["usage"]);
            let usage = &second["days"][0]["usage"];
            assert_eq!(usage["inputTokens"], 400);
            assert_eq!(usage["cachedInputTokens"], 80);
            assert_eq!(usage["outputTokens"], 40);
            assert_eq!(usage["totalTokens"], 440);
            assert_eq!(second["days"][0]["recordCount"], 4);
            assert_eq!(second["classificationStatus"]["uncertainEvents"], 1);
            let complete = pass(&mut worker);
            assert!(worker.historical_cursor.is_none());
            assert_eq!(complete["revision"], second["revision"]);
            drop(worker);
            let reopened = pass(&mut Runtime::new(storage.clone(), settings.clone()));
            assert_eq!(reopened["days"], second["days"]);
            assert_eq!(reopened["revision"], second["revision"]);
        }
    }

    #[test]
    fn late_gateway_attribution_with_an_unrelated_count_gap_survives_restart() {
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
        let request_log = root.path().join("requests.jsonl");
        let storage = root.path().join("history");
        let session = "00000000-0000-4000-8000-000000000001";
        let now = Local::now();
        let head = json!({"type":"session_meta","timestamp":now.to_rfc3339(),
            "payload":{"id":session,"model_provider":"custom"}});
        let context = json!({"type":"turn_context","timestamp":now.to_rfc3339(),
            "payload":{"turn_id":"test-turn","model":"gpt-fixture"}});
        let usage = json!({"input_tokens":100,"cached_input_tokens":20,"output_tokens":10});
        let event = json!({"type":"event_msg","timestamp":now.to_rfc3339(),
            "payload":{"type":"token_count","info":{"total_token_usage":usage,"last_token_usage":usage}}});
        fs::write(
            home.join("sessions/rollout-01.jsonl"),
            format!("{head}\n{context}\n{event}\n"),
        )
        .unwrap();
        let mut unrelated = head;
        unrelated["payload"]["id"] = json!("00000000-0000-4000-8000-000000000002");
        let mut malformed = event;
        malformed["payload"]["info"]["total_token_usage"]["input_tokens"] = json!("invalid");
        fs::write(
            home.join("sessions/rollout-02.jsonl"),
            format!("{unrelated}\n{malformed}\n"),
        )
        .unwrap();
        let request = json!({"period":"today","includeSessionCursors":true});
        let query = Query::parse(&request, now.date_naive()).unwrap();
        let mut worker = Runtime::new(storage.clone(), settings.clone());
        let before = worker
            .update_with_request_log(&home, &request_log)
            .read(&storage, &query, &request, None, false);
        assert_eq!(before["days"][0]["usage"]["totalTokens"], 110);
        assert_eq!(before["scan"]["sourceIssues"], 1);
        assert!(before["scan"].get("malformedRecords").is_none());
        assert_eq!(before["countStatus"]["complete"], false);
        let row = json!({"id":"test-proof","threadId":session,"model":"gpt-fixture","upstreamModel":"gpt-fixture",
            "relayId":"fixture-relay","relayName":"Fixture Relay","success":true,"state":"success",
            "statusCode":200,"timestampMs":now.timestamp_millis(),"startedAtMs":now.timestamp_millis()-100,
            "attempts":1,"autoRetries":0,"usage":{"inputTokens":100,"cachedTokens":20,"outputTokens":10}});
        fs::write(&request_log, format!("{row}\n")).unwrap();
        let after = worker
            .update_with_request_log(&home, &request_log)
            .read(&storage, &query, &request, None, false);
        assert_eq!(
            after["days"][0]["breakdowns"][0]["provider"], "fixture-relay",
            "{after}"
        );
        assert_eq!(
            after["days"][0]["breakdowns"][0]["classificationEvidence"],
            "request-log-exact"
        );
        assert_eq!(after["days"][0]["usage"], before["days"][0]["usage"]);
        assert_eq!(after["sessionCursors"], before["sessionCursors"]);
        drop(worker);
        fs::write(&request_log, "").unwrap();
        let mut worker = Runtime::new(storage.clone(), settings);
        let cold = worker
            .update_with_request_log(&home, &request_log)
            .read(&storage, &query, &request, None, false);
        assert_eq!(cold["days"], after["days"]);
        assert_eq!(cold["revision"], after["revision"]);
    }

    #[test]
    fn sealed_queued_sources_allow_gateway_evidence_and_a_new_or_replaced_archive_withdraws_it() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let sources = home.join("sessions");
        fs::create_dir_all(&sources).unwrap();
        let home = fs::canonicalize(home).unwrap();
        let settings = root.path().join("settings.json");
        fs::write(&settings, r#"{"usageHistoryEnabled":true}"#).unwrap();
        let log = root.path().join("requests.jsonl");
        let storage = root.path().join("history");
        let now = Local::now();
        let session = "00000000-0000-4000-8000-000000000001";
        let other = "00000000-0000-4000-8000-000000000002";
        let head = |id: &str| {
            json!({"type":"session_meta","timestamp":now.to_rfc3339(),
                                     "payload":{"id":id,"model_provider":"custom"}})
        };
        let turn = json!({"type":"turn_context","timestamp":now.to_rfc3339(),
                          "payload":{"turn_id":"turn","model":"gpt-fixture"}});
        let usage = json!({"input_tokens":100,"cached_input_tokens":20,"output_tokens":10});
        let mut count = json!({"type":"event_msg","timestamp":now.to_rfc3339(),"payload":{
            "type":"token_count","info":{"total_token_usage":usage,"last_token_usage":usage}}});
        fs::write(
            sources.join("rollout-000.jsonl"),
            format!("{}\n{turn}\n{count}\n", head(session)),
        )
        .unwrap();
        for i in 1..74 {
            fs::write(
                sources.join(format!("rollout-{i:03}.jsonl")),
                format!("{}\n", head(other)),
            )
            .unwrap();
        }
        count["payload"]["info"]["total_token_usage"]["input_tokens"] = json!("invalid");
        let archived = sources.join("rollout-last.jsonl.zst");
        let data = format!("{}\n{count}\n", head(other));
        fs::write(
            &archived,
            zstd::stream::encode_all(data.as_bytes(), 1).unwrap(),
        )
        .unwrap();
        let oldest = std::time::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        fs::File::options()
            .write(true)
            .open(&archived)
            .unwrap()
            .set_modified(oldest)
            .unwrap();
        let request = json!({"period":"today","includeSessionCursors":true});
        let query = Query::parse(&request, now.date_naive()).unwrap();
        let pass = |worker: &mut Runtime| {
            let state = worker.update_with_request_log(&home, &log);
            assert!(state.storage_error.is_none(), "{:?}", state.storage_error);
            assert!(
                state.classification_error.is_none(),
                "{:?}",
                state.classification_error
            );
            state.read(&storage, &query, &request, None, false)
        };
        let mut worker = Runtime::new(storage.clone(), settings.clone());
        pass(&mut worker);
        let native = pass(&mut worker);
        assert_eq!(native["days"][0]["recordCount"], 1);
        assert_eq!(native["scan"]["sourceIssues"], 1);
        let proof = json!({"id":"test-proof","threadId":session,"model":"gpt-fixture",
            "upstreamModel":"gpt-fixture","relayId":"fixture-relay","relayName":"Fixture",
            "success":true,"state":"success","statusCode":200,"timestampMs":now.timestamp_millis(),
            "startedAtMs":now.timestamp_millis()-100,"attempts":1,"autoRetries":0,
            "usage":{"inputTokens":100,"cachedTokens":20,"outputTokens":10}});
        fs::write(&log, format!("{proof}\n")).unwrap();
        drop(worker);
        let exact = |view: &Value| {
            view["days"][0]["breakdowns"][0]["classificationEvidence"] == "request-log-exact"
        };
        for case in ["new", "replaced"] {
            let mut worker = Runtime::new(storage.clone(), settings.clone());
            let known = pass(&mut worker);
            assert!(known["scan"]["pendingFiles"].as_u64().unwrap() > 0);
            assert_eq!(known["countStatus"]["complete"], false);
            assert!(
                exact(&known),
                "{case}: sealed backlog must allow exact evidence"
            );
            drop(worker);
            let disturb = if case == "new" {
                sources.join("rollout-new-last.jsonl.zst")
            } else {
                archived.clone()
            };
            let fresh = format!("{}\n", head(other));
            fs::write(
                &disturb,
                zstd::stream::encode_all(fresh.as_bytes(), 1).unwrap(),
            )
            .unwrap();
            fs::File::options()
                .write(true)
                .open(&disturb)
                .unwrap()
                .set_modified(oldest)
                .unwrap();
            let mut worker = Runtime::new(storage.clone(), settings.clone());
            let unverified = pass(&mut worker);
            assert!(unverified["scan"]["pendingFiles"].as_u64().unwrap() > 0);
            assert!(
                exact(&unverified),
                "{case}: unrelated queued work must retain a previously verified proof"
            );
            assert_eq!(unverified["days"][0]["usage"], native["days"][0]["usage"]);
            assert_eq!(unverified["sessionCursors"], native["sessionCursors"]);
            assert!(
                exact(&pass(&mut worker)),
                "{case}: resolved backlog must recover"
            );
        }
        use std::io::Write;
        let mut active = fs::OpenOptions::new()
            .append(true)
            .open(sources.join("rollout-000.jsonl"))
            .unwrap();
        active.write_all(b"{").unwrap();
        let mut worker = Runtime::new(storage.clone(), settings.clone());
        let partial = pass(&mut worker);
        assert!(partial["scan"]["pendingFiles"].as_u64().unwrap() > 0);
        assert!(
            exact(&partial),
            "an unpublished same-session tail must not withdraw an existing proof"
        );
        assert_eq!(partial["days"][0]["usage"], native["days"][0]["usage"]);
        active
            .write_all(b"\"type\":\"fixture-ignored\",\"payload\":{}}\n")
            .unwrap();
        drop(active);
        drop(worker);
        let mut reopened = Runtime::new(storage.clone(), settings);
        assert!(exact(&pass(&mut reopened)));
    }
}
