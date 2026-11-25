use super::{
    historical::{EvidenceOverlay, EvidenceVerifier, Historical},
    scan::{RECLAIM_BATCH, Scan},
    store::{
        Store,
        archive::{ArchiveKey, Recovered},
        executions::{DetailQuery, ExecutionQuery},
        projection::ArchiveAnswer,
    },
};
use crate::usage_history::{
    Query, TOKEN_INDEX_DB,
    structured::{Index, RequestEvidence, live_receipts, read_history_settings},
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

const RECONCILIATION_BYTES: usize = 32 * 1024 * 1024;
const CLASSIFICATION_SESSIONS_PER_ROUND: usize = 6;
const CLASSIFICATION_SLICE_MS: i64 = 300_000;

pub(in super::super) struct Runtime {
    storage: PathBuf,
    settings: PathBuf,
    store: Option<Store>,
    scan: Scan,
    retry_full: bool,
    evidence: Index,
    evidence_cursor: usize,
    evidence_before: BTreeMap<String, i64>,
    historical: Option<Arc<Historical>>,
    historical_verifier: Arc<Mutex<EvidenceVerifier>>,
}

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
    db.busy_timeout(Duration::from_millis(50))?;
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
            historical: None,
            historical_verifier: Arc::default(),
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
        match self.refresh(
            home,
            cutoff,
            u64::from(limits.storage_mib) * 1024 * 1024,
            &mut state,
        ) {
            Ok(true) if evidence_available => {
                if let Err(error) = self.classify_recent(home, proof_cutoff, &mut state) {
                    state.classification_error = Some(format!("{error:#}"));
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
            if evidence.captured_id_check_failed || evidence.captured_rejected.contains(&request.id)
            {
                request.success = false;
            }
        }
        state
    }

    fn classify_recent(
        &mut self,
        home: &Path,
        cutoff: NaiveDate,
        state: &mut ReadState,
    ) -> Result<()> {
        let store = self
            .store
            .as_mut()
            .expect("successful scan owns a usage index");
        let now = Local::now().timestamp_millis();
        let sessions = store.recent_sessions(now - 86_400_000)?;
        let active: std::collections::BTreeSet<_> = sessions.iter().map(|(id, _)| id).collect();
        self.evidence_before.retain(|id, _| active.contains(id));
        if sessions.is_empty() {
            self.evidence_cursor = 0;
            return Ok(());
        }
        let offset = self.evidence_cursor % sessions.len();
        let count = CLASSIFICATION_SESSIONS_PER_ROUND.min(sessions.len());
        self.evidence_cursor = offset + count;
        let index = &mut self.evidence;
        let mut failure = None;
        for (session, latest) in sessions.iter().cycle().skip(offset).take(count) {
            let tail = latest.saturating_sub(CLASSIFICATION_SLICE_MS - 1)..latest.saturating_add(1);
            let before = self
                .evidence_before
                .entry(session.clone())
                .or_insert(tail.start);
            let older = store
                .classification_before(session, (*before).min(tail.start))?
                .map(|at| at.saturating_sub(CLASSIFICATION_SLICE_MS - 1)..at.saturating_add(1));
            *before = older.as_ref().map_or(tail.start, |window| window.start);
            for window in older.into_iter().chain(std::iter::once(tail)) {
                let result = store.reclassify_window(
                    session,
                    Some(window),
                    RECONCILIATION_BYTES,
                    self.scan.scoped_count_gaps(),
                    |events, aliases, scope| {
                        ensure!(
                            index.native_evidence.observe_count_conflicts(events, true),
                            "native count evidence exceeds capacity"
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
                        ensure!(
                            !index.request_evidence.captured_id_check_failed,
                            "captured request validation failed"
                        );
                        index.native_evidence.attribute(
                            home,
                            events,
                            &index.provider_endpoints,
                            index.provider_endpoint_since,
                            aliases,
                            &index.provider_cache,
                            true,
                            Some(scope),
                        );
                        index.native_evidence.apply_count_conflicts(events);
                        Ok(())
                    },
                );
                if let Err(error) = result {
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
        ensure!(available, "native evidence cache exceeds capacity");
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
            RECONCILIATION_BYTES,
        );
        state.failed_sources = report.errors.len();
        let can_classify = report.complete
            || (report.pending_sources == 0
                && report.issues != 0
                && report.issues == self.scan.scoped_count_gaps().len());
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
        Ok(can_classify)
    }
}

fn disk_full(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<rusqlite::Error>()
        .is_some_and(|error| error.sqlite_error_code() == Some(rusqlite::ErrorCode::DiskFull))
}

fn allocated_pages(db: &Connection) -> Result<(u64, u64)> {
    Ok((
        db.pragma_query_value(None, "page_count", |row| row.get(0))?,
        db.pragma_query_value(None, "freelist_count", |row| row.get(0))?,
    ))
}

// 长读事务会阻止 WAL 回收；先暂停摄取，避免每一轮继续累加日志。
fn checkpoint_if_over_budget(db: &Connection, storage: &Path, budget: u64) -> Result<bool> {
    let bytes = || -> Result<u64> {
        let mut used = 0;
        for suffix in ["", "-wal", "-shm", "-journal"] {
            match fs::metadata(storage.join(format!("{TOKEN_INDEX_DB}{suffix}"))) {
                Ok(metadata) => used += metadata.len(),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(used)
    };
    if bytes()? >= budget {
        let (busy, _, _): (i64, i64, i64) =
            db.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?;
        ensure!(
            busy == 0,
            "usage index working budget exhausted (WAL reader active)"
        );
        return Ok(bytes()? < budget);
    }
    Ok(true)
}

impl ReadState {
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
        self.read_with_archive(storage, options, request, scope_session, processing, None)
            .0
    }

    pub(in super::super) fn read_with_archive(
        &self,
        storage: &Path,
        options: &Query,
        request: &Value,
        scope_session: Option<&str>,
        processing: bool,
        archive: Option<&ArchiveAnswer>,
    ) -> (Value, Option<ArchiveKey>) {
        let (mut response, schedule) = match open_database(storage, false).and_then(|mut db| {
            Store::read_response_with_archive(
                &mut db,
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
) -> ArchiveAnswer {
    let answer = (|| -> Result<Recovered> {
        let db = open_database(storage, false)?;
        let mut replay = key.replay(&db, home, snapshot.provider_names.clone())?;
        loop {
            ensure!(
                !cancelled.load(Ordering::Relaxed),
                "archive replay cancelled"
            );
            if replay.step()? {
                let mut recovered = replay.finish(&db)?;
                recovered.evidence = Some(
                    EvidenceOverlay::for_archive(
                        &snapshot.historical_verifier,
                        settings,
                        &recovered.events,
                        key.session.is_some(),
                    )
                    .map_err(|error| format!("{error:#}")),
                );
                return Ok(recovered);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    })()
    .map_err(|error| format!("{error:#}"));
    (key, answer)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
