use super::{
    historical::{EvidenceOverlay, EvidenceVerifier, Historical},
    scan::Scan,
    store::{
        Store,
        archive::{ArchiveKey, Recovered},
        projection::ArchiveAnswer,
    },
};
use crate::usage_history::{
    Query, TOKEN_INDEX_DB,
    structured::{Index, RequestEvidence, live_receipts},
};
use anyhow::{Context, Result, ensure};
use chrono::{Days, Local, NaiveDate};
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

const RETENTION_DAYS: u64 = 60;
const RECONCILIATION_BYTES: usize = 32 * 1024 * 1024;
const WORKING_BUDGET: u64 = 2 * 1024 * 1024 * 1024;
const CLASSIFICATION_SESSIONS_PER_ROUND: usize = 6;

pub(in super::super) struct Runtime {
    storage: PathBuf,
    settings: PathBuf,
    store: Option<Store>,
    scan: Scan,
    evidence: Index,
    evidence_cursor: usize,
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
            evidence_cursor: 0,
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
        let today = Local::now().date_naive();
        let cutoff = today - Days::new(RETENTION_DAYS - 1);
        let proof_cutoff = today - chrono::Duration::days(super::super::RETENTION_DAYS - 1);
        // 明细保留期与证明缓存分开；缩短明细不迁移或裁剪已有供应商证明。
        let (_, evidence_available) = self.evidence.load_evidence(home, proof_cutoff, request_log);
        self.evidence.refresh_provider_profiles(&self.settings);
        self.evidence.request_evidence.refresh(request_log);
        state
            .provider_names
            .clone_from(&self.evidence.provider_names);
        match self.refresh(home, cutoff, &mut state) {
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
        if sessions.is_empty() {
            self.evidence_cursor = 0;
            return Ok(());
        }
        let offset = self.evidence_cursor % sessions.len();
        let count = CLASSIFICATION_SESSIONS_PER_ROUND.min(sessions.len());
        self.evidence_cursor = offset + count;
        let index = &mut self.evidence;
        let mut failure = None;
        for session in sessions.iter().cycle().skip(offset).take(count) {
            let result = store.reclassify_session(
                session,
                RECONCILIATION_BYTES,
                self.scan.scoped_count_gaps(),
                |events, aliases| {
                    ensure!(
                        index.native_evidence.observe_count_conflicts(events, true),
                        "native count evidence exceeds capacity"
                    );
                    index.request_evidence.supplement_captured(
                        events,
                        aliases,
                        &index.provider_cache,
                    );
                    index.request_evidence.attribute_with_cache(
                        events,
                        &mut state.provider_names,
                        aliases,
                        &index.provider_cache,
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
                    );
                    index.native_evidence.apply_count_conflicts(events);
                    Ok(())
                },
            );
            if let Err(error) = result {
                failure.get_or_insert(error);
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

    fn refresh(&mut self, home: &Path, cutoff: NaiveDate, state: &mut ReadState) -> Result<bool> {
        if self.store.is_none() {
            let db = open_database(&self.storage, true).context("opening usage index")?;
            let page_size: u64 = db.pragma_query_value(None, "page_size", |row| row.get(0))?;
            // 给 WAL、共享页和在读快照预留空间，不把用户预算全部交给主库。
            db.pragma_update(None, "max_page_count", WORKING_BUDGET / page_size * 3 / 4)?;
            db.pragma_update(None, "journal_size_limit", WORKING_BUDGET / 8)?;
            db.pragma_update(None, "synchronous", "FULL")?;
            self.store = Some(Store::open(db).context("initializing usage index")?);
        }
        let store = self.store.as_mut().expect("usage index is open");
        enforce_working_budget(&store.db, &self.storage, WORKING_BUDGET)?;
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
            return Err(error.context("maintaining usage index"));
        }
        if let Some((_, error)) = report.errors.into_iter().find(|(_, error)| {
            error
                .downcast_ref::<rusqlite::Error>()
                .is_some_and(|error| {
                    error.sqlite_error_code() == Some(rusqlite::ErrorCode::DiskFull)
                })
        }) {
            return Err(error.context("writing usage index"));
        }
        Ok(can_classify)
    }
}

// 长读事务会阻止 WAL 回收；先暂停摄取，避免每一轮继续累加日志。
fn enforce_working_budget(db: &Connection, storage: &Path, budget: u64) -> Result<()> {
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
            busy == 0 && bytes()? < budget,
            "usage index working budget exhausted"
        );
    }
    Ok(())
}

impl ReadState {
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
        let error = enforce_working_budget(&store.db, root.path(), budget).unwrap_err();
        assert!(error.to_string().contains("working budget exhausted"));
        assert_eq!(
            pin.query_row("SELECT revision FROM token_revision", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            snapshot,
        );
        assert_eq!(read_revision(root.path()), snapshot + 64,);
        pin.commit().unwrap();
        enforce_working_budget(&store.db, root.path(), budget).unwrap();
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
        assert_eq!(before["scan"]["malformedRecords"], 1);
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
