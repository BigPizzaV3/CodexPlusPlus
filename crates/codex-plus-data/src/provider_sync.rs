use fs2::FileExt;
use rusqlite::{Connection, OptionalExtension, params_from_iter, types::Value as SqlValue};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_PROVIDER: &str = "openai";
const SESSION_DIRS: [&str; 2] = ["sessions", "archived_sessions"];
const BACKUP_KEEP_COUNT: usize = 5;
const REMOTE_CONTROL_CREATION_WINDOW_SECS: i64 = 15 * 60;
const SESSION_TRANSACTION_FILE: &str = "session-transaction.json";
const SESSION_TRANSACTION_NAMESPACE: &str = "provider-sync-rollout-transaction";
const SESSION_TRANSACTION_IN_PROGRESS: &str = "in_progress";
const SESSION_TRANSACTION_COMMITTED: &str = "committed";
const SESSION_TRANSACTION_ROLLED_BACK: &str = "rolled_back";
const PROVIDER_SYNC_SCAN_STATE_FILE: &str = "rollout-scan-state.json";
const PROVIDER_SYNC_SCAN_STATE_NAMESPACE: &str = "provider-sync-rollout-scan-state";
const PROVIDER_SYNC_SCAN_STATE_VERSION: u32 = 1;
const PROVIDER_SYNC_SCAN_STATE_MAX_BYTES: u64 = 32 * 1024 * 1024;
const PROVIDER_SYNC_SCAN_STATE_MAX_ENTRIES: usize = 100_000;
const PROVIDER_SYNC_SCAN_RULES_V1: &str = concat!(
    "provider-sync-rollout-scan/v1;",
    "utf8-jsonl;session_meta-payload-id-cwd-model_provider;",
    "user_message-or-user_input;encrypted_content-byte-marker;",
    "non-root-agent-source;missing-provider-sentinel"
);

/// `create_lock` 先建目录再写 `owner.json`，两步之间被强杀会留下没有 owner 的锁目录。
/// 该窗口只有几毫秒，因此超过这个时长仍缺 owner 的锁一定是中断残留，可以安全回收；
/// 反过来说，宽限期内的无主锁必须保留，否则会把正在建锁的同伴进程挤掉。
const LOCK_INTERRUPTED_GRACE_SECS: u64 = 60;
/// Legacy owner files do not record the OS process creation time. A live PID whose process began
/// well after the lock was created is a reused PID, not the original lock owner.
const LEGACY_PID_REUSE_TOLERANCE_SECS: u64 = 5 * 60;
const LEGACY_PID_REUSE_MIN_LOCK_AGE_SECS: u64 = 24 * 60 * 60;
const PROCESS_START_MATCH_TOLERANCE_SECS: u64 = 5;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProviderSyncLockOwner {
    pid: u32,
    started_at: u64,
    #[serde(default)]
    process_started_at: Option<u64>,
    #[serde(default)]
    process_birth_id: Option<String>,
    #[serde(default)]
    lock_id: Option<String>,
}

/// provider sync 锁的可观测状态。管理器在强杀 launcher 前用它判断
/// 「现在是不是有人正在同步」，避免把同步中的进程打断（issue #1901）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum ProviderSyncLockState {
    /// 没有锁，可以安全重启。
    Free,
    /// 锁被一个仍在运行的进程持有，同步很可能正在进行中。
    Held { pid: u32, started_at: u64 },
    /// 锁存在但持有者已经退出（或 owner 信息缺失且已过宽限期），
    /// 下一次 `acquire_lock` 会自动回收它。
    Stale { pid: Option<u32> },
    /// 锁存在、owner 信息不可读，但仍在宽限期内——无法判断是否有人正在建锁。
    Indeterminate,
}

#[derive(Debug)]
pub struct ProviderSyncLifecycleGuard {
    lock_dir: PathBuf,
    lock_file: File,
    lock_id: String,
    directory_released: bool,
    file_unlocked: bool,
}

impl ProviderSyncLifecycleGuard {
    /// Releases both compatibility and OS ownership before a caller starts a successor process.
    /// A mismatched owner is an ABA conflict and must block the successor instead of deleting it.
    pub fn release(mut self) -> std::io::Result<()> {
        if !release_owned_lock(&self.lock_dir, &self.lock_id)? {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "provider-sync lock ownership changed before release",
            ));
        }
        self.directory_released = true;
        FileExt::unlock(&self.lock_file)?;
        self.file_unlocked = true;
        Ok(())
    }
}

impl Drop for ProviderSyncLifecycleGuard {
    fn drop(&mut self) {
        if !self.directory_released {
            let _ = release_owned_lock(&self.lock_dir, &self.lock_id);
        }
        if !self.file_unlocked {
            let _ = FileExt::unlock(&self.lock_file);
        }
    }
}

/// Atomically reserves provider-sync lifecycle ownership for a restart or a real sync.
/// The OS file lock is released automatically if the process exits; the legacy directory remains
/// present while held so older launchers also stay out of the critical section.
pub fn try_acquire_provider_sync_lifecycle_guard(
    codex_home: Option<&Path>,
) -> std::io::Result<ProviderSyncLifecycleGuard> {
    let home = codex_home
        .map(Path::to_path_buf)
        .unwrap_or_else(default_codex_home_dir);
    acquire_lock_inner(&home.join("tmp/provider-sync.lock"), false)
}

/// 读取 provider sync 锁的当前状态，不获取也不修改它。
pub fn inspect_provider_sync_lock(codex_home: Option<&Path>) -> ProviderSyncLockState {
    let home = codex_home
        .map(Path::to_path_buf)
        .unwrap_or_else(default_codex_home_dir);
    inspect_lock(&home.join("tmp/provider-sync.lock"))
}

fn inspect_lock(path: &Path) -> ProviderSyncLockState {
    if !path.exists() {
        return ProviderSyncLockState::Free;
    }
    classify_lock(
        read_lock_owner(path).as_ref(),
        lock_dir_age_secs(path),
        codex_plus_core::watcher::inspect_process_instance,
    )
}

/// 根据 owner 信息和锁目录年龄判定锁的状态。与文件系统解耦，便于穷举测试。
fn classify_lock(
    owner: Option<&ProviderSyncLockOwner>,
    age_secs: Option<u64>,
    inspect_process: impl Fn(u32) -> codex_plus_core::watcher::ProcessInstanceState,
) -> ProviderSyncLockState {
    let Some(owner) = owner else {
        // owner.json 缺失或损坏。持有者只在建锁的几毫秒内处于这个状态，
        // 所以超过宽限期就说明它是被强杀留下的残骸。
        return if age_secs.is_some_and(|age| age >= LOCK_INTERRUPTED_GRACE_SECS) {
            ProviderSyncLockState::Stale { pid: None }
        } else {
            ProviderSyncLockState::Indeterminate
        };
    };
    use codex_plus_core::watcher::ProcessInstanceState;
    match inspect_process(owner.pid) {
        ProcessInstanceState::NotRunning => ProviderSyncLockState::Stale {
            pid: Some(owner.pid),
        },
        ProcessInstanceState::Running {
            started_at_secs,
            birth_id: current_birth_id,
        } => {
            let birth_mismatch = owner
                .process_birth_id
                .as_deref()
                .zip(current_birth_id.as_deref())
                .is_some_and(|(expected, current)| expected != current);
            let recorded_start_mismatch = owner.process_birth_id.is_none()
                && owner.process_started_at.zip(started_at_secs).is_some_and(
                    |(expected, current)| {
                        expected.abs_diff(current) > PROCESS_START_MATCH_TOLERANCE_SECS
                    },
                );
            let legacy_pid_reuse = owner.process_birth_id.is_none()
                && owner.process_started_at.is_none()
                && age_secs.is_some_and(|age| age >= LEGACY_PID_REUSE_MIN_LOCK_AGE_SECS)
                && started_at_secs.is_some_and(|current| {
                    current
                        > owner
                            .started_at
                            .saturating_add(LEGACY_PID_REUSE_TOLERANCE_SECS)
                });
            if birth_mismatch || recorded_start_mismatch || legacy_pid_reuse {
                ProviderSyncLockState::Stale {
                    pid: Some(owner.pid),
                }
            } else {
                ProviderSyncLockState::Held {
                    pid: owner.pid,
                    started_at: owner.started_at,
                }
            }
        }
        // Unknown process identity cannot prove that the owner is gone. Preserve the lock.
        ProcessInstanceState::Unknown => ProviderSyncLockState::Held {
            pid: owner.pid,
            started_at: owner.started_at,
        },
    }
}

fn current_process_identity() -> (Option<u64>, Option<String>) {
    match codex_plus_core::watcher::inspect_process_instance(std::process::id()) {
        codex_plus_core::watcher::ProcessInstanceState::Running {
            started_at_secs,
            birth_id,
        } => (started_at_secs, birth_id),
        _ => (None, None),
    }
}

fn read_lock_owner(path: &Path) -> Option<ProviderSyncLockOwner> {
    serde_json::from_slice::<ProviderSyncLockOwner>(&fs::read(path.join("owner.json")).ok()?).ok()
}

fn lock_dir_age_secs(path: &Path) -> Option<u64> {
    let created = fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()?;
    SystemTime::now()
        .duration_since(created)
        .ok()
        .map(|elapsed| elapsed.as_secs())
}

fn default_codex_home_dir() -> PathBuf {
    codex_plus_core::codex_home::default_codex_home_dir()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderSyncStatus {
    Disabled,
    Skipped,
    Synced,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderSyncResult {
    pub status: ProviderSyncStatus,
    pub message: String,
    pub target_provider: String,
    pub backup_dir: Option<PathBuf>,
    pub changed_session_files: usize,
    pub skipped_locked_rollout_files: Vec<PathBuf>,
    pub sqlite_rows_updated: usize,
    pub sqlite_provider_rows_updated: usize,
    pub sqlite_user_event_rows_updated: usize,
    pub sqlite_cwd_rows_updated: usize,
    pub sqlite_catalog_rows_inserted: usize,
    #[serde(default)]
    pub sqlite_catalog_rows_removed: usize,
    pub updated_workspace_roots: usize,
    pub encrypted_content_warning: Option<String>,
    #[serde(default)]
    pub repair_audit: ProviderSyncAudit,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSyncAudit {
    pub catalog_only_sessions: usize,
    pub catalog_only_with_current_rollout: usize,
    pub catalog_only_with_backup_database: usize,
    pub catalog_only_without_recovery_source: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionIndexCleanupCandidate {
    pub id: String,
    pub thread_name: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionIndexCleanupPreview {
    pub snapshot_sha256: String,
    pub candidates: Vec<SessionIndexCleanupCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionIndexCleanupResult {
    pub pruned_entries: usize,
    pub backup_dir: Option<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct SessionIndexCleanupApplyError {
    pub message: String,
    pub backup_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderSyncTargetSource {
    Config,
    Rollout,
    Sqlite,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSyncTargetOption {
    pub id: String,
    pub sources: Vec<ProviderSyncTargetSource>,
    pub is_current_provider: bool,
    pub is_manual: bool,
    pub is_saved: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSyncTargetList {
    pub current_provider: String,
    pub targets: Vec<ProviderSyncTargetOption>,
}

#[derive(Debug, Clone)]
struct SessionChange {
    path: PathBuf,
    original_sha256: String,
    original_size: u64,
    thread_id: Option<String>,
    cwd: Option<String>,
    has_user_event: bool,
    rewrite_needed: bool,
    original_mtime: Option<SystemTime>,
    rewrite_mode: SessionRewriteMode,
}

#[derive(Debug, Clone)]
enum SessionRewriteMode {
    AllProviders,
    SourceProvider { source_provider: String },
}

#[derive(Debug, Default)]
struct RolloutRewrite {
    rewrite_needed: bool,
    thread_id: Option<String>,
    cwd: Option<String>,
    providers: HashSet<String>,
    session_meta_count: usize,
    has_user_event: bool,
    has_encrypted_content: bool,
    marks_non_root_agent: bool,
    original_sha256: String,
    original_size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProviderSyncRolloutScanState {
    relative_path: String,
    size: u64,
    modified_secs: Option<u64>,
    modified_nanos: Option<u32>,
    file_identity: String,
    sha256: String,
    thread_id: Option<String>,
    cwd: Option<String>,
    has_user_event: bool,
    has_encrypted_content: bool,
    marks_non_root_agent: bool,
    session_meta_count: usize,
    providers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProviderSyncRolloutScanStateManifest {
    version: u32,
    namespace: String,
    rules_sha256: String,
    rollout_roots: HashMap<String, SessionTransactionRootEvidence>,
    entries: Vec<ProviderSyncRolloutScanState>,
}

#[derive(Debug, Default)]
struct SessionChanges {
    changes: Vec<SessionChange>,
    skipped_locked_rollout_files: Vec<PathBuf>,
    encrypted_content_counts: HashMap<String, usize>,
    subagent_thread_ids: HashSet<String>,
    scan_state_entries: Vec<ProviderSyncRolloutScanState>,
}

#[derive(Debug, Default)]
struct ProviderSyncThreadKinds {
    subagent_thread_ids: HashSet<String>,
    explicit_user_thread_ids: HashSet<String>,
}

#[derive(Debug, Default)]
struct AppliedSessionChanges {
    changed_files: usize,
    skipped_locked_rollout_files: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionTransactionEntry {
    relative_path: String,
    original_sha256: String,
    next_sha256: String,
    original_size: u64,
    next_size: u64,
    session_meta_backup_sha256: String,
    original_mtime_secs: Option<u64>,
    original_mtime_nanos: Option<u32>,
    #[serde(default)]
    external_sha256: Option<String>,
    #[serde(default)]
    external_size: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SessionTransactionMode {
    Full,
    Remote,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SessionTransactionPhase {
    RolloutsApplying,
    RolloutsApplied,
    DownstreamStarted,
    CommitDecided,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionTransactionRootEvidence {
    canonical_path: String,
    identity: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProviderSyncBackupFileEvidence {
    size: u64,
    sha256: String,
}

struct PreparedProviderSyncBackupFile {
    file: File,
    modified: Option<SystemTime>,
    evidence: ProviderSyncBackupFileEvidence,
}

struct PreparedProviderSyncBackupSet {
    files: HashMap<String, PreparedProviderSyncBackupFile>,
    snapshot_dir: PathBuf,
}

impl Drop for PreparedProviderSyncBackupSet {
    fn drop(&mut self) {
        self.files.clear();
        let _ = fs::remove_dir_all(&self.snapshot_dir);
    }
}

struct ProviderSyncDirectoryEvidence {
    canonical_path: PathBuf,
    identity: String,
    guard: File,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionTransactionManifest {
    version: u32,
    namespace: String,
    status: String,
    transaction_id: String,
    mode: SessionTransactionMode,
    phase: SessionTransactionPhase,
    rollout_roots: HashMap<String, SessionTransactionRootEvidence>,
    entries: Vec<SessionTransactionEntry>,
}

#[derive(Debug, Clone)]
struct SessionIndexPlan {
    path: PathBuf,
    original_bytes: Vec<u8>,
    original_text: String,
    snapshot_sha256: String,
    candidates: Vec<SessionIndexCleanupCandidate>,
}

#[derive(Debug, Default)]
struct SqliteUpdateCounts {
    provider_rows: usize,
    user_event_rows: usize,
    cwd_rows: usize,
    catalog_insert_rows: usize,
    catalog_remove_rows: usize,
}

#[derive(Debug, Clone)]
struct CatalogRepairThread {
    id: String,
    display_title: String,
    source_created_at: f64,
    source_updated_at: f64,
    cwd: String,
    source_kind: String,
    source_detail: String,
    model_provider: String,
    git_branch: Option<String>,
    thread_source: Option<String>,
}

#[derive(Debug)]
struct CatalogRepairObservedThread {
    thread: CatalogRepairThread,
    eligible: bool,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct CatalogRepairCounts {
    inserted_rows: usize,
    removed_rows: usize,
}

impl CatalogRepairCounts {
    fn total(&self) -> usize {
        self.inserted_rows + self.removed_rows
    }

    fn add(&mut self, other: Self) {
        self.inserted_rows += other.inserted_rows;
        self.removed_rows += other.removed_rows;
    }
}

#[derive(Debug, Default)]
struct CatalogRepairPlan {
    threads: HashMap<String, CatalogRepairThread>,
    non_root_thread_ids: HashSet<String>,
    ineligible_thread_ids: HashSet<String>,
    catalog_non_root_thread_ids: HashMap<PathBuf, HashSet<String>>,
}

impl CatalogRepairPlan {
    fn has_cleanup_candidates(&self) -> bool {
        !self.non_root_thread_ids.is_empty()
            || !self.ineligible_thread_ids.is_empty()
            || self
                .catalog_non_root_thread_ids
                .values()
                .any(|thread_ids| !thread_ids.is_empty())
    }

    fn cleanup_thread_ids_for_path(&self, path: &Path) -> HashSet<String> {
        let mut thread_ids = self.non_root_thread_ids.clone();
        thread_ids.extend(self.ineligible_thread_ids.iter().cloned());
        if let Some(catalog_thread_ids) = self.catalog_non_root_thread_ids.get(path) {
            thread_ids.extend(catalog_thread_ids.iter().cloned());
        }
        thread_ids
    }
}

enum RemoteControlRolloutLookup {
    Ready(PathBuf),
    Archived,
    UnsupportedProvider,
    Missing,
}

impl SqliteUpdateCounts {
    fn total(&self) -> usize {
        self.provider_rows
            + self.user_event_rows
            + self.cwd_rows
            + self.catalog_insert_rows
            + self.catalog_remove_rows
    }

    fn add(&mut self, other: Self) {
        self.provider_rows += other.provider_rows;
        self.user_event_rows += other.user_event_rows;
        self.cwd_rows += other.cwd_rows;
        self.catalog_insert_rows += other.catalog_insert_rows;
        self.catalog_remove_rows += other.catalog_remove_rows;
    }
}

pub fn run_provider_sync(codex_home: Option<&Path>) -> ProviderSyncResult {
    run_provider_sync_with_target(codex_home, None)
}

pub fn remote_control_session_recovery_candidate_exists(
    codex_home: Option<&Path>,
    thread_id: &str,
) -> anyhow::Result<bool> {
    let thread_id = thread_id.trim();
    if thread_id.is_empty() || thread_id.len() > 128 {
        return Ok(false);
    }
    let home = codex_home
        .map(Path::to_path_buf)
        .unwrap_or_else(default_codex_home_dir);
    let minimum_created_at = now_secs() as i64 - REMOTE_CONTROL_CREATION_WINDOW_SECS;
    for path in provider_sync_db_paths(&home) {
        if !path.exists() {
            continue;
        }
        let db = Connection::open(path)?;
        let columns = table_columns(&db, "threads")?;
        if !columns.contains("id") || !columns.contains("model_provider") {
            continue;
        }
        let archived_expr = if columns.contains("archived") {
            "COALESCE(archived, 0)"
        } else {
            "0"
        };
        let created_expr = if columns.contains("created_at_ms") {
            "CAST(COALESCE(created_at_ms, 0) / 1000 AS INTEGER)"
        } else if columns.contains("created_at") {
            "CAST(COALESCE(created_at, 0) AS INTEGER)"
        } else {
            continue;
        };
        let sql = format!(
            "SELECT 1 FROM threads WHERE id = ?1 AND model_provider = ?2 AND {archived_expr} = 0 AND {created_expr} >= ?3 LIMIT 1"
        );
        if db
            .query_row(
                &sql,
                (thread_id, DEFAULT_PROVIDER, minimum_created_at),
                |_| Ok(()),
            )
            .optional()?
            .is_some()
        {
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn run_remote_control_session_catalog_recovery_for_thread_with_target(
    codex_home: Option<&Path>,
    thread_id: &str,
    target_provider: &str,
) -> ProviderSyncResult {
    let require_stopped_app = codex_home.is_none();
    let thread_id = thread_id.trim();
    if thread_id.is_empty() || thread_id.len() > 128 {
        return result(
            ProviderSyncStatus::Skipped,
            "Remote Control session recovery requires a valid thread id",
            DEFAULT_PROVIDER,
            None,
            0,
            0,
        );
    }
    let target_provider = target_provider.trim();
    if target_provider.is_empty() || target_provider == DEFAULT_PROVIDER {
        return result(
            ProviderSyncStatus::Skipped,
            "Remote Control session recovery requires a non-openai target provider",
            target_provider,
            None,
            0,
            0,
        );
    }
    let home = codex_home
        .map(Path::to_path_buf)
        .unwrap_or_else(default_codex_home_dir);
    let lock_dir = home.join("tmp/provider-sync.lock");
    let _lock_guard = match acquire_lock(&lock_dir) {
        Ok(guard) => guard,
        Err(_) => {
            return result(
                ProviderSyncStatus::Skipped,
                format!("Provider sync lock exists: {}", lock_dir.to_string_lossy()),
                target_provider,
                None,
                0,
                0,
            );
        }
    };
    if require_stopped_app {
        let running_processes =
            codex_plus_core::watcher::find_session_index_cleanup_blocking_processes();
        if !running_processes.is_empty() {
            return result(
                ProviderSyncStatus::Skipped,
                "Remote Control session catalog recovery requires Codex App / ChatGPT to be stopped",
                target_provider,
                None,
                0,
                0,
            );
        }
    }
    if let Err(error) = recover_interrupted_session_transactions(&home) {
        return result(
            ProviderSyncStatus::Skipped,
            format!("Remote Control session catalog recovery skipped: {error}"),
            target_provider,
            None,
            0,
            0,
        );
    }
    let thread_ids = HashSet::from([thread_id.to_string()]);
    let recovery = run_remote_control_catalog_recovery_for_threads(
        &home,
        &provider_sync_db_paths(&home),
        target_provider,
        &thread_ids,
    );
    recovery.unwrap_or_else(|error| {
        result(
            ProviderSyncStatus::Skipped,
            format!("Remote Control session catalog recovery skipped: {error}"),
            target_provider,
            None,
            0,
            0,
        )
    })
}

pub fn run_remote_control_session_finalization_for_thread_with_target(
    codex_home: Option<&Path>,
    thread_id: &str,
    target_provider: &str,
) -> ProviderSyncResult {
    let require_stopped_app = codex_home.is_none();
    let thread_id = thread_id.trim();
    let target_provider = target_provider.trim();
    if thread_id.is_empty()
        || thread_id.len() > 128
        || target_provider.is_empty()
        || target_provider == DEFAULT_PROVIDER
    {
        return result(
            ProviderSyncStatus::Skipped,
            "Remote Control session finalization requires a thread id and target provider",
            target_provider,
            None,
            0,
            0,
        );
    }
    let home = codex_home
        .map(Path::to_path_buf)
        .unwrap_or_else(default_codex_home_dir);
    let lock_dir = home.join("tmp/provider-sync.lock");
    let _lock_guard = match acquire_lock(&lock_dir) {
        Ok(guard) => guard,
        Err(_) => {
            return result(
                ProviderSyncStatus::Skipped,
                format!("Provider sync lock exists: {}", lock_dir.to_string_lossy()),
                target_provider,
                None,
                0,
                0,
            );
        }
    };
    if require_stopped_app {
        let running_processes =
            codex_plus_core::watcher::find_session_index_cleanup_blocking_processes();
        if !running_processes.is_empty() {
            return result(
                ProviderSyncStatus::Skipped,
                "Remote Control session finalization requires Codex App / ChatGPT to be stopped",
                target_provider,
                None,
                0,
                0,
            );
        }
    }
    let recovery = (|| -> anyhow::Result<ProviderSyncResult> {
        recover_interrupted_session_transactions(&home)?;
        let sqlite_paths = provider_sync_db_paths(&home);
        let rollout_path = match remote_control_rollout_for_thread(
            &home,
            &sqlite_paths,
            thread_id,
            target_provider,
        )? {
            RemoteControlRolloutLookup::Ready(path) => path,
            RemoteControlRolloutLookup::Archived => {
                return Ok(result(
                    ProviderSyncStatus::Synced,
                    "Remote Control session finalization ignored an archived thread",
                    target_provider,
                    None,
                    0,
                    0,
                ));
            }
            RemoteControlRolloutLookup::UnsupportedProvider => {
                return Ok(result(
                    ProviderSyncStatus::Synced,
                    "Remote Control session finalization ignored a thread owned by another provider",
                    target_provider,
                    None,
                    0,
                    0,
                ));
            }
            RemoteControlRolloutLookup::Missing => {
                return Ok(result(
                    ProviderSyncStatus::Skipped,
                    "Remote Control session finalization deferred until the thread rollout is available",
                    target_provider,
                    None,
                    0,
                    0,
                ));
            }
        };
        let collected = collect_session_change_for_path(
            &rollout_path,
            target_provider,
            DEFAULT_PROVIDER,
            thread_id,
        )?;
        let rewrite_changes = collected
            .changes
            .iter()
            .filter(|change| change.rewrite_needed)
            .cloned()
            .collect::<Vec<_>>();
        let backup_dir = create_backup(
            &home,
            target_provider,
            SessionTransactionMode::Remote,
            &rewrite_changes,
        )?;
        let applied = apply_session_changes(&home, &backup_dir, target_provider, &rewrite_changes)?;
        set_session_transaction_phase(&backup_dir, SessionTransactionPhase::RolloutsApplied)?;
        if !rollout_file_matches_provider(&rollout_path, thread_id, target_provider)? {
            rollback_session_transaction(&home, &backup_dir)?;
            let mut deferred = result(
                ProviderSyncStatus::Skipped,
                "Remote Control session finalization deferred for a changed or locked rollout",
                target_provider,
                Some(backup_dir),
                applied.changed_files,
                0,
            );
            deferred.skipped_locked_rollout_files = applied.skipped_locked_rollout_files;
            return Ok(deferred);
        }
        set_session_transaction_phase(&backup_dir, SessionTransactionPhase::CommitDecided)?;
        commit_session_transaction(&backup_dir)?;
        let thread_ids = HashSet::from([thread_id.to_string()]);
        let catalog_repairs = repair_missing_local_thread_catalog_rows_for_threads(
            &home,
            &sqlite_paths,
            target_provider,
            &thread_ids,
        )?;
        let mut sqlite_updates = apply_remote_control_recovery_sqlite_updates(
            &sqlite_paths,
            target_provider,
            &thread_ids,
        )?;
        sqlite_updates.catalog_insert_rows = catalog_repairs.inserted_rows;
        sqlite_updates.catalog_remove_rows = catalog_repairs.removed_rows;
        prune_backups(&home)?;
        let mut synced = result(
            ProviderSyncStatus::Synced,
            "Remote Control session finalization complete",
            target_provider,
            Some(backup_dir),
            applied.changed_files,
            sqlite_updates.total(),
        );
        synced.sqlite_provider_rows_updated = sqlite_updates.provider_rows;
        synced.sqlite_catalog_rows_inserted = sqlite_updates.catalog_insert_rows;
        synced.sqlite_catalog_rows_removed = sqlite_updates.catalog_remove_rows;
        Ok(synced)
    })();
    recovery.unwrap_or_else(|error| {
        result(
            ProviderSyncStatus::Skipped,
            format!("Remote Control session finalization skipped: {error}"),
            target_provider,
            None,
            0,
            0,
        )
    })
}

fn run_remote_control_catalog_recovery_for_threads(
    home: &Path,
    sqlite_paths: &[PathBuf],
    target_provider: &str,
    requested_thread_ids: &HashSet<String>,
) -> anyhow::Result<ProviderSyncResult> {
    let thread_ids = remote_control_catalog_recovery_thread_ids(
        sqlite_paths,
        target_provider,
        requested_thread_ids,
    )?;
    if thread_ids.is_empty() {
        return Ok(result(
            ProviderSyncStatus::Synced,
            "Remote Control session catalog already up to date",
            target_provider,
            None,
            0,
            0,
        ));
    }

    let catalog_repairs = repair_missing_local_thread_catalog_rows_for_threads(
        home,
        sqlite_paths,
        target_provider,
        &thread_ids,
    )?;
    let provider_rows =
        apply_remote_control_catalog_updates(sqlite_paths, target_provider, &thread_ids)?;
    let mut synced = result(
        ProviderSyncStatus::Synced,
        "Remote Control session catalog recovery complete",
        target_provider,
        None,
        0,
        provider_rows + catalog_repairs.total(),
    );
    synced.sqlite_provider_rows_updated = provider_rows;
    synced.sqlite_catalog_rows_inserted = catalog_repairs.inserted_rows;
    synced.sqlite_catalog_rows_removed = catalog_repairs.removed_rows;
    Ok(synced)
}

pub fn run_provider_sync_with_target(
    codex_home: Option<&Path>,
    explicit_target_provider: Option<&str>,
) -> ProviderSyncResult {
    let require_stopped_app = codex_home.is_none();
    let home = codex_home
        .map(Path::to_path_buf)
        .unwrap_or_else(default_codex_home_dir);
    if !home.exists() {
        return result(
            ProviderSyncStatus::Skipped,
            format!("Codex home not found: {}", home.to_string_lossy()),
            DEFAULT_PROVIDER,
            None,
            0,
            0,
        );
    }
    let target_provider =
        match resolve_target_provider(&home.join("config.toml"), explicit_target_provider) {
            Ok(provider) => provider,
            Err(message) => {
                return result(
                    ProviderSyncStatus::Skipped,
                    message,
                    DEFAULT_PROVIDER,
                    None,
                    0,
                    0,
                );
            }
        };
    if require_stopped_app {
        let running_processes =
            codex_plus_core::watcher::find_session_index_cleanup_blocking_processes();
        if !running_processes.is_empty() {
            return result(
                ProviderSyncStatus::Skipped,
                format!(
                    "Codex App / ChatGPT 仍在运行（进程：{}）；请完全退出 App 后再修复历史会话",
                    running_processes
                        .iter()
                        .map(u32::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                &target_provider,
                None,
                0,
                0,
            );
        }
    }
    let lock_dir = home.join("tmp/provider-sync.lock");
    let _lock_guard = match acquire_lock(&lock_dir) {
        Ok(guard) => guard,
        Err(_) => {
            return result(
                ProviderSyncStatus::Skipped,
                format!("Provider sync lock exists: {}", lock_dir.to_string_lossy()),
                &target_provider,
                None,
                0,
                0,
            );
        }
    };
    if require_stopped_app {
        let running_processes =
            codex_plus_core::watcher::find_session_index_cleanup_blocking_processes();
        if !running_processes.is_empty() {
            return result(
                ProviderSyncStatus::Skipped,
                "Codex App / ChatGPT started before provider-sync recovery",
                &target_provider,
                None,
                0,
                0,
            );
        }
    }
    let sync_result = (|| -> anyhow::Result<ProviderSyncResult> {
        recover_interrupted_session_transactions(&home)?;
        let sqlite_paths = provider_sync_db_paths(&home);
        let thread_kinds = sqlite_provider_sync_thread_kinds(&sqlite_paths)?;
        let repair_audit = match audit_provider_sync_state(&home, &sqlite_paths) {
            Ok(audit) => audit,
            Err(error) => {
                let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                    "provider_sync.repair_audit_failed",
                    json!({
                        "error": error.to_string(),
                        "backup_root": home
                            .join("backups_state/provider-sync")
                            .to_string_lossy(),
                    }),
                );
                ProviderSyncAudit::default()
            }
        };
        let collected = collect_session_changes(
            &home,
            &target_provider,
            &thread_kinds.subagent_thread_ids,
            &thread_kinds.explicit_user_thread_ids,
        )?;
        let mut subagent_thread_ids = thread_kinds.subagent_thread_ids;
        subagent_thread_ids.extend(collected.subagent_thread_ids.iter().cloned());
        let encrypted_content_warning =
            build_encrypted_content_warning(&collected.encrypted_content_counts, &target_provider);
        let rewrite_changes = collected
            .changes
            .iter()
            .filter(|change| change.rewrite_needed)
            .cloned()
            .collect::<Vec<_>>();
        let thread_ids_with_user_events = collected
            .changes
            .iter()
            .filter(|change| change.has_user_event)
            .filter_map(|change| change.thread_id.clone())
            .collect::<HashSet<_>>();
        let projectless_thread_ids =
            load_projectless_thread_ids(&home.join(".codex-global-state.json"))?;
        let cwd_by_thread_id = collected
            .changes
            .iter()
            .filter_map(|change| Some((change.thread_id.clone()?, change.cwd.clone()?)))
            .filter(|(thread_id, _)| !projectless_thread_ids.contains(thread_id))
            .collect::<HashMap<_, _>>();
        let sqlite_update_count = count_sqlite_updates_for_paths(
            &sqlite_paths,
            &target_provider,
            &thread_ids_with_user_events,
            &cwd_by_thread_id,
            &subagent_thread_ids,
        )?;
        let catalog_repair_count =
            count_local_thread_catalog_repairs(&home, &sqlite_paths, &target_provider)?;
        let global_state_update_count =
            count_global_state_updates(&home.join(".codex-global-state.json"))?;
        if rewrite_changes.is_empty()
            && sqlite_update_count == 0
            && catalog_repair_count == 0
            && global_state_update_count == 0
        {
            if require_stopped_app {
                let running_processes =
                    codex_plus_core::watcher::find_session_index_cleanup_blocking_processes();
                if !running_processes.is_empty() {
                    anyhow::bail!(
                        "Codex App / ChatGPT started while provider sync was scanning rollouts"
                    );
                }
            }
            persist_provider_sync_scan_state_best_effort(&home, &collected.scan_state_entries);
            let mut synced = result(
                ProviderSyncStatus::Synced,
                "Provider sync already up to date",
                &target_provider,
                None,
                0,
                0,
            );
            synced.skipped_locked_rollout_files = collected.skipped_locked_rollout_files;
            synced.encrypted_content_warning = encrypted_content_warning;
            synced.repair_audit = repair_audit;
            synced.message =
                provider_sync_message_with_audit(&synced.message, &synced.repair_audit);
            return Ok(synced);
        }
        if require_stopped_app {
            let running_processes =
                codex_plus_core::watcher::find_session_index_cleanup_blocking_processes();
            if !running_processes.is_empty() {
                anyhow::bail!(
                    "Codex App / ChatGPT started while provider sync was scanning rollouts"
                );
            }
        }
        let backup_dir = create_backup(
            &home,
            &target_provider,
            SessionTransactionMode::Full,
            &rewrite_changes,
        )?;
        let applied =
            apply_session_changes(&home, &backup_dir, &target_provider, &rewrite_changes)?;
        set_session_transaction_phase(&backup_dir, SessionTransactionPhase::RolloutsApplied)?;
        set_session_transaction_phase(&backup_dir, SessionTransactionPhase::DownstreamStarted)?;
        let apply_result = (|| -> anyhow::Result<(SqliteUpdateCounts, usize)> {
            let sqlite_updates = apply_sqlite_update_for_paths(
                &sqlite_paths,
                &target_provider,
                &thread_ids_with_user_events,
                &cwd_by_thread_id,
                &subagent_thread_ids,
            )?;
            let mut sqlite_updates = sqlite_updates;
            let catalog_repairs =
                repair_missing_local_thread_catalog_rows(&home, &sqlite_paths, &target_provider)?;
            sqlite_updates.catalog_insert_rows = catalog_repairs.inserted_rows;
            sqlite_updates.catalog_remove_rows = catalog_repairs.removed_rows;
            let updated_workspace_roots =
                apply_global_state_update(&home.join(".codex-global-state.json"))?;
            Ok((sqlite_updates, updated_workspace_roots))
        })();
        let (sqlite_updates, updated_workspace_roots) = match apply_result {
            Ok(counts) => counts,
            Err(err) => {
                restore_provider_sync_downstream_backup(&home, &backup_dir).map_err(
                    |restore_error| {
                        anyhow::anyhow!(
                            "provider-sync downstream update failed ({err}); backup restore also failed: {restore_error}"
                        )
                    },
                )?;
                rollback_session_transaction(&home, &backup_dir)?;
                return Err(err);
            }
        };
        set_session_transaction_phase(&backup_dir, SessionTransactionPhase::CommitDecided)?;
        commit_session_transaction(&backup_dir)?;
        persist_committed_provider_sync_scan_state_best_effort(
            &home,
            &backup_dir,
            &target_provider,
            collected.scan_state_entries.clone(),
        );
        prune_backups(&home)?;
        let mut synced = result(
            ProviderSyncStatus::Synced,
            "Provider sync complete",
            &target_provider,
            Some(backup_dir),
            applied.changed_files,
            sqlite_updates.total(),
        );
        synced.skipped_locked_rollout_files = collected.skipped_locked_rollout_files;
        synced
            .skipped_locked_rollout_files
            .extend(applied.skipped_locked_rollout_files);
        synced.skipped_locked_rollout_files.sort();
        synced.skipped_locked_rollout_files.dedup();
        synced.sqlite_provider_rows_updated = sqlite_updates.provider_rows;
        synced.sqlite_user_event_rows_updated = sqlite_updates.user_event_rows;
        synced.sqlite_cwd_rows_updated = sqlite_updates.cwd_rows;
        synced.sqlite_catalog_rows_inserted = sqlite_updates.catalog_insert_rows;
        synced.sqlite_catalog_rows_removed = sqlite_updates.catalog_remove_rows;
        synced.updated_workspace_roots = updated_workspace_roots;
        synced.encrypted_content_warning = encrypted_content_warning;
        synced.repair_audit = repair_audit;
        synced.message = provider_sync_message_with_audit(&synced.message, &synced.repair_audit);
        Ok(synced)
    })();
    sync_result.unwrap_or_else(|err| {
        result(
            ProviderSyncStatus::Skipped,
            format!("Provider sync skipped: {err}"),
            &target_provider,
            None,
            0,
            0,
        )
    })
}

fn result(
    status: ProviderSyncStatus,
    message: impl Into<String>,
    target_provider: &str,
    backup_dir: Option<PathBuf>,
    changed_session_files: usize,
    sqlite_rows_updated: usize,
) -> ProviderSyncResult {
    ProviderSyncResult {
        status,
        message: message.into(),
        target_provider: target_provider.to_string(),
        backup_dir,
        changed_session_files,
        skipped_locked_rollout_files: Vec::new(),
        sqlite_rows_updated,
        sqlite_provider_rows_updated: 0,
        sqlite_user_event_rows_updated: 0,
        sqlite_cwd_rows_updated: 0,
        sqlite_catalog_rows_inserted: 0,
        sqlite_catalog_rows_removed: 0,
        updated_workspace_roots: 0,
        encrypted_content_warning: None,
        repair_audit: ProviderSyncAudit::default(),
    }
}

fn provider_sync_message_with_audit(message: &str, audit: &ProviderSyncAudit) -> String {
    if audit.catalog_only_sessions == 0 {
        return message.to_string();
    }
    format!(
        "{message}；审计发现 {} 条仅存在于本地会话目录的记录，其中 {} 条仍有当前 rollout、{} 条只能在历史数据库备份中找到，{} 条没有可用恢复来源；未自动重建缺失的 canonical 会话。",
        audit.catalog_only_sessions,
        audit.catalog_only_with_current_rollout,
        audit.catalog_only_with_backup_database,
        audit.catalog_only_without_recovery_source,
    )
}

fn provider_sync_db_paths(home: &Path) -> Vec<PathBuf> {
    let mut paths = codex_plus_core::codex_sqlite::codex_session_db_paths_from_home(home);
    for path in codex_plus_core::codex_sqlite::codex_thread_reference_db_paths_from_home(home) {
        if !paths.iter().any(|candidate| candidate == &path) {
            paths.push(path);
        }
    }
    paths
}

fn audit_provider_sync_state(
    home: &Path,
    sqlite_paths: &[PathBuf],
) -> anyhow::Result<ProviderSyncAudit> {
    let mut canonical_thread_ids = HashSet::new();
    let mut catalog_thread_ids = HashSet::new();
    for path in sqlite_paths {
        canonical_thread_ids.extend(sqlite_table_ids(path, "threads", "id")?);
        catalog_thread_ids.extend(sqlite_user_thread_ids(path)?);
    }

    let catalog_only = catalog_thread_ids
        .difference(&canonical_thread_ids)
        .cloned()
        .collect::<HashSet<_>>();
    if catalog_only.is_empty() {
        return Ok(ProviderSyncAudit::default());
    }

    let current_rollout_ids = rollout_files(home)?
        .into_iter()
        .filter_map(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .and_then(rollout_thread_id_from_filename)
        })
        .collect::<HashSet<_>>();
    let backup_database_ids = backup_database_thread_ids(home)?;
    let with_current_rollout = catalog_only
        .iter()
        .filter(|thread_id| current_rollout_ids.contains(*thread_id))
        .count();
    let with_backup_database = catalog_only
        .iter()
        .filter(|thread_id| {
            !current_rollout_ids.contains(*thread_id) && backup_database_ids.contains(*thread_id)
        })
        .count();

    Ok(ProviderSyncAudit {
        catalog_only_sessions: catalog_only.len(),
        catalog_only_with_current_rollout: with_current_rollout,
        catalog_only_with_backup_database: with_backup_database,
        catalog_only_without_recovery_source: catalog_only
            .iter()
            .filter(|thread_id| {
                !current_rollout_ids.contains(*thread_id)
                    && !backup_database_ids.contains(*thread_id)
            })
            .count(),
    })
}

fn backup_database_thread_ids(home: &Path) -> anyhow::Result<HashSet<String>> {
    let root = home.join("backups_state/provider-sync");
    let mut ids = HashSet::new();
    if !root.exists() {
        return Ok(ids);
    }
    let mut files = Vec::new();
    collect_files_recursive(&root, &mut files)?;
    for path in files {
        if !matches!(
            path.extension().and_then(|value| value.to_str()),
            Some("sqlite" | "db")
        ) {
            continue;
        }
        if let Ok(thread_ids) = sqlite_table_ids(&path, "threads", "id") {
            ids.extend(thread_ids);
        }
    }
    Ok(ids)
}

fn collect_files_recursive(root: &Path, files: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        if file_type.is_dir() {
            collect_files_recursive(&path, files)?;
        } else if file_type.is_file() {
            files.push(path);
        }
    }
    Ok(())
}

pub fn load_provider_sync_targets(codex_home: Option<&Path>) -> ProviderSyncTargetList {
    let home = codex_home
        .map(Path::to_path_buf)
        .unwrap_or_else(default_codex_home_dir);
    let current_provider = read_current_provider(&home.join("config.toml"));
    let mut sources: HashMap<String, HashSet<ProviderSyncTargetSource>> = HashMap::new();

    fn add_sources(
        sources: &mut HashMap<String, HashSet<ProviderSyncTargetSource>>,
        ids: impl IntoIterator<Item = String>,
        source: ProviderSyncTargetSource,
    ) {
        for id in ids {
            if !is_valid_provider_id_for_discovery(&id) {
                continue;
            }
            sources.entry(id).or_default().insert(source);
        }
    }

    add_sources(
        &mut sources,
        list_configured_provider_ids(&home.join("config.toml")),
        ProviderSyncTargetSource::Config,
    );
    add_sources(
        &mut sources,
        [current_provider.clone()],
        ProviderSyncTargetSource::Config,
    );
    if let Ok(ids) = rollout_provider_ids(&home) {
        add_sources(&mut sources, ids, ProviderSyncTargetSource::Rollout);
    }
    for db_path in provider_sync_db_paths(&home) {
        if let Ok(ids) = sqlite_provider_ids(&db_path) {
            add_sources(&mut sources, ids, ProviderSyncTargetSource::Sqlite);
        }
    }

    let mut targets = sources
        .into_iter()
        .map(|(id, source_set)| {
            let mut source_list = source_set.into_iter().collect::<Vec<_>>();
            source_list.sort();
            ProviderSyncTargetOption {
                is_current_provider: id == current_provider,
                is_manual: source_list.contains(&ProviderSyncTargetSource::Manual),
                is_saved: false,
                id,
                sources: source_list,
            }
        })
        .collect::<Vec<_>>();
    targets.sort_by(|left, right| {
        right
            .is_current_provider
            .cmp(&left.is_current_provider)
            .then_with(|| left.id.cmp(&right.id))
    });

    ProviderSyncTargetList {
        current_provider,
        targets,
    }
}

fn read_current_provider(path: &Path) -> String {
    let Ok(text) = fs::read_to_string(path) else {
        return DEFAULT_PROVIDER.to_string();
    };
    let provider = root_toml_string_value(&text, "model_provider").unwrap_or_default();
    if provider.trim().is_empty() {
        DEFAULT_PROVIDER.to_string()
    } else {
        provider
    }
}

fn resolve_target_provider(
    config_path: &Path,
    explicit_target_provider: Option<&str>,
) -> Result<String, String> {
    if let Some(raw) = explicit_target_provider {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Ok(read_current_provider(config_path));
        }
        if !is_valid_explicit_provider_id(trimmed) {
            return Err(format!("Invalid provider sync target: {trimmed:?}"));
        }
        return Ok(trimmed.to_string());
    }
    Ok(read_current_provider(config_path))
}

fn is_valid_explicit_provider_id(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
}

fn list_configured_provider_ids(path: &Path) -> Vec<String> {
    let mut ids = HashSet::new();
    ids.insert(DEFAULT_PROVIDER.to_string());
    let Ok(text) = fs::read_to_string(path) else {
        return sorted_provider_ids(ids);
    };
    for line in text.lines() {
        let stripped = line.trim();
        let Some(section) = stripped
            .strip_prefix("[model_providers.")
            .and_then(|rest| rest.strip_suffix(']'))
        else {
            continue;
        };
        let id = section.trim();
        if is_valid_provider_id_for_discovery(id) {
            ids.insert(id.to_string());
        }
    }
    sorted_provider_ids(ids)
}

fn sorted_provider_ids(ids: HashSet<String>) -> Vec<String> {
    let mut ids = ids
        .into_iter()
        .filter(|id| !id.trim().is_empty())
        .collect::<Vec<_>>();
    ids.sort();
    ids
}

fn is_valid_provider_id_for_discovery(value: &str) -> bool {
    !value.trim().is_empty() && !value.chars().any(char::is_control)
}

fn root_toml_string_value(text: &str, key: &str) -> Option<String> {
    for line in text.lines() {
        let stripped = line.trim();
        if stripped.starts_with('[') {
            break;
        }
        let Some(raw) = toml_key_raw_value(stripped, key) else {
            continue;
        };
        return toml_string_value(raw);
    }
    None
}

fn toml_key_raw_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(key)?.trim_start();
    rest.strip_prefix('=').map(str::trim_start)
}

fn toml_string_value(raw: &str) -> Option<String> {
    let quote = raw.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let mut value = String::new();
    let mut escaping = false;
    for ch in raw[quote.len_utf8()..].chars() {
        if quote == '"' && escaping {
            value.push(ch);
            escaping = false;
        } else if quote == '"' && ch == '\\' {
            escaping = true;
        } else if ch == quote {
            return Some(value);
        } else {
            value.push(ch);
        }
    }
    None
}

fn acquire_lock(path: &Path) -> std::io::Result<ProviderSyncLifecycleGuard> {
    acquire_lock_inner(path, true)
}

fn acquire_lock_inner(path: &Path, log_busy: bool) -> std::io::Result<ProviderSyncLifecycleGuard> {
    fs::create_dir_all(path.parent().unwrap_or_else(|| Path::new(".")))?;
    let lifecycle_path = path.with_file_name("provider-sync.lifecycle.lock");
    let lock_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(lifecycle_path)?;
    if let Err(error) = lock_file.try_lock_exclusive() {
        let error = normalize_lock_contention_error(error);
        if log_busy {
            log_lock_busy(path);
        }
        return Err(error);
    }
    let lock_id = uuid::Uuid::new_v4().to_string();
    match create_lock(path, &lock_id) {
        Ok(()) => Ok(ProviderSyncLifecycleGuard {
            lock_dir: path.to_path_buf(),
            lock_file,
            lock_id,
            directory_released: false,
            file_unlocked: false,
        }),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let Some((owner, isolated_path)) = isolate_stale_lock(path) else {
                if log_busy {
                    log_lock_busy(path);
                }
                return Err(error);
            };
            match create_lock(path, &lock_id) {
                Ok(()) => {
                    let quarantine_cleanup_failed = fs::remove_dir_all(&isolated_path).is_err();
                    let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                        "provider_sync.stale_lock_recovered",
                        json!({
                            "owner_pid": owner.as_ref().map(|owner| owner.pid),
                            "owner_started_at": owner.as_ref().map(|owner| owner.started_at),
                            "owner_process_started_at": owner
                                .as_ref()
                                .and_then(|owner| owner.process_started_at),
                            // owner 缺失说明持有者是在建锁中途被强杀的（issue #1901）
                            "interrupted": owner.is_none(),
                            "quarantine_cleanup_failed": quarantine_cleanup_failed,
                        }),
                    );
                    Ok(ProviderSyncLifecycleGuard {
                        lock_dir: path.to_path_buf(),
                        lock_file,
                        lock_id,
                        directory_released: false,
                        file_unlocked: false,
                    })
                }
                Err(retry_error) => {
                    let _ = fs::remove_dir_all(isolated_path);
                    Err(retry_error)
                }
            }
        }
        Err(error) => Err(error),
    }
}

fn normalize_lock_contention_error(error: std::io::Error) -> std::io::Error {
    #[cfg(windows)]
    if error.raw_os_error() == Some(33) {
        return std::io::Error::new(std::io::ErrorKind::WouldBlock, error);
    }
    error
}

/// 锁没能拿到时留下现场，用于区分「另一个同步真的在跑」和「残留锁把同步永久卡死」。
fn log_lock_busy(path: &Path) {
    let state = inspect_lock(path);
    let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
        "provider_sync.lock_busy",
        json!({
            "lock_dir": path.to_string_lossy(),
            "state": state,
            "age_secs": lock_dir_age_secs(path),
        }),
    );
}

fn create_lock(path: &Path, lock_id: &str) -> std::io::Result<()> {
    fs::create_dir(path)?;
    let (process_started_at, process_birth_id) = current_process_identity();
    let write_result = fs::write(
        path.join("owner.json"),
        json!({
            "pid": std::process::id(),
            "startedAt": now_secs(),
            "processStartedAt": process_started_at,
            "processBirthId": process_birth_id,
            "lockId": lock_id,
        })
        .to_string(),
    );
    if let Err(error) = write_result {
        let _ = fs::remove_dir_all(path);
        return Err(error);
    }
    Ok(())
}

/// 在已经持有 OS 生命周期锁时，把可回收的兼容目录挪到隔离路径，让调用方重新建锁。
///
/// 三种可回收的形态：
/// - owner.json 带 `lockId`，证明目录来自新版协议；OS 锁既然已取得，该目录必为孤儿；
/// - owner.json 可读且持有进程已退出（正常的崩溃残留）；
/// - owner.json 缺失/损坏，且锁目录存在时间已超过 [`LOCK_INTERRUPTED_GRACE_SECS`]
///   ——持有者在 `create_lock` 中途被强杀，不会再有人来补写 owner（issue #1901）。
///
/// 其余情况一律保留锁：宁可跳过一次同步，也不能抢走仍在写入的进程的锁。
fn isolate_stale_lock(path: &Path) -> Option<(Option<ProviderSyncLockOwner>, PathBuf)> {
    let parsed_owner = read_lock_owner(path);
    let owner = if parsed_owner
        .as_ref()
        .and_then(|owner| owner.lock_id.as_ref())
        .is_some()
    {
        parsed_owner
    } else {
        match inspect_lock(path) {
            ProviderSyncLockState::Stale { .. } => parsed_owner,
            _ => return None,
        }
    };
    let file_name = path.file_name()?.to_string_lossy();
    let owner_tag = owner
        .as_ref()
        .map_or_else(|| "interrupted".to_string(), |owner| owner.pid.to_string());
    let isolated_path = path.with_file_name(format!(
        "{file_name}.stale-{owner_tag}-{}",
        uuid::Uuid::new_v4()
    ));
    fs::rename(path, &isolated_path).ok()?;
    Some((owner, isolated_path))
}

fn release_owned_lock(path: &Path, lock_id: &str) -> std::io::Result<bool> {
    if !path.exists() {
        return Ok(true);
    }
    if read_lock_owner(path)
        .and_then(|owner| owner.lock_id)
        .is_some_and(|owner_lock_id| owner_lock_id == lock_id)
    {
        fs::remove_dir_all(path)?;
        return Ok(true);
    }
    Ok(false)
}

fn provider_sync_scan_rules_sha256() -> String {
    let mut hasher = Sha256::new();
    hasher.update(PROVIDER_SYNC_SCAN_RULES_V1.as_bytes());
    hasher.update([0]);
    hasher.update(env!("CARGO_PKG_VERSION").as_bytes());
    format!("{:x}", hasher.finalize())
}

fn provider_sync_scan_state_path(home: &Path) -> PathBuf {
    home.join("backups_state/provider-sync")
        .join(PROVIDER_SYNC_SCAN_STATE_FILE)
}

fn load_provider_sync_scan_state(
    home: &Path,
) -> anyhow::Result<HashMap<String, ProviderSyncRolloutScanState>> {
    let path = provider_sync_scan_state_path(home);
    let path_metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(HashMap::new()),
        Err(error) => return Err(error.into()),
    };
    if !path_metadata.is_file() {
        anyhow::bail!("provider-sync rollout scan state is not a bounded regular file");
    }
    if let Some(parent) = path.parent() {
        ensure_path_components_not_reparse(home, parent)?;
    }
    ensure_not_reparse_or_symlink(&path)?;
    let mut file = File::open(&path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > PROVIDER_SYNC_SCAN_STATE_MAX_BYTES {
        anyhow::bail!("provider-sync rollout scan state is not a bounded regular file");
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    std::io::Read::take(&mut file, PROVIDER_SYNC_SCAN_STATE_MAX_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > PROVIDER_SYNC_SCAN_STATE_MAX_BYTES {
        anyhow::bail!("provider-sync rollout scan state exceeded the read limit");
    }
    let manifest: ProviderSyncRolloutScanStateManifest = serde_json::from_slice(&bytes)?;
    if manifest.version != PROVIDER_SYNC_SCAN_STATE_VERSION
        || manifest.namespace != PROVIDER_SYNC_SCAN_STATE_NAMESPACE
        || manifest.rules_sha256 != provider_sync_scan_rules_sha256()
        || manifest.rollout_roots != session_transaction_rollout_roots(home)?
        || manifest.entries.len() > PROVIDER_SYNC_SCAN_STATE_MAX_ENTRIES
    {
        anyhow::bail!("unsupported provider-sync rollout scan state");
    }
    let mut entries = HashMap::new();
    for entry in manifest.entries {
        validated_rollout_relative_path(&entry.relative_path)?;
        if entry.relative_path.len() > 32 * 1024
            || entry.file_identity.is_empty()
            || entry.file_identity.len() > 512
            || entry.sha256.len() != 64
            || !entry.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
            || entry.modified_secs.is_some() != entry.modified_nanos.is_some()
            || entry
                .thread_id
                .as_ref()
                .is_some_and(|value| value.len() > 512)
            || entry
                .cwd
                .as_ref()
                .is_some_and(|value| value.len() > 32 * 1024)
            || entry.providers.len() > 1024
            || entry.providers.iter().any(|value| value.len() > 512)
            || !entry.providers.windows(2).all(|pair| pair[0] < pair[1])
            || entries.insert(entry.relative_path.clone(), entry).is_some()
        {
            anyhow::bail!("invalid provider-sync rollout scan state entry");
        }
    }
    Ok(entries)
}

fn load_provider_sync_scan_state_best_effort(
    home: &Path,
) -> HashMap<String, ProviderSyncRolloutScanState> {
    match load_provider_sync_scan_state(home) {
        Ok(entries) => entries,
        Err(error) => {
            let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                "provider_sync.scan_state_ignored",
                json!({
                    "error": error.to_string(),
                    "path": provider_sync_scan_state_path(home).to_string_lossy(),
                }),
            );
            HashMap::new()
        }
    }
}

fn persist_provider_sync_scan_state(
    home: &Path,
    entries: &[ProviderSyncRolloutScanState],
) -> anyhow::Result<()> {
    if entries.len() > PROVIDER_SYNC_SCAN_STATE_MAX_ENTRIES {
        anyhow::bail!("provider-sync rollout scan state has too many entries");
    }
    let mut entries = entries.to_vec();
    entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    let path = provider_sync_scan_state_path(home);
    if let Some(parent) = path.parent() {
        create_validated_directory_path(home, parent)?;
    }
    match fs::symlink_metadata(&path) {
        Ok(_) => ensure_not_reparse_or_symlink(&path)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let bytes = serde_json::to_vec_pretty(&ProviderSyncRolloutScanStateManifest {
        version: PROVIDER_SYNC_SCAN_STATE_VERSION,
        namespace: PROVIDER_SYNC_SCAN_STATE_NAMESPACE.to_string(),
        rules_sha256: provider_sync_scan_rules_sha256(),
        rollout_roots: session_transaction_rollout_roots(home)?,
        entries,
    })?;
    if bytes.len() as u64 > PROVIDER_SYNC_SCAN_STATE_MAX_BYTES {
        anyhow::bail!("provider-sync rollout scan state is too large");
    }
    codex_plus_core::settings::atomic_write(&path, &bytes)
}

fn persist_provider_sync_scan_state_best_effort(
    home: &Path,
    entries: &[ProviderSyncRolloutScanState],
) {
    if let Err(error) = persist_provider_sync_scan_state(home, entries) {
        let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
            "provider_sync.scan_state_write_failed",
            json!({
                "error": error.to_string(),
                "path": provider_sync_scan_state_path(home).to_string_lossy(),
            }),
        );
    }
}

fn invalidate_provider_sync_scan_state(home: &Path) -> anyhow::Result<()> {
    let path = provider_sync_scan_state_path(home);
    match fs::symlink_metadata(&path) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    }
    if let Some(parent) = path.parent() {
        ensure_path_components_not_reparse(home, parent)?;
    }
    ensure_not_reparse_or_symlink(&path)?;
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn committed_provider_sync_scan_state(
    home: &Path,
    backup_dir: &Path,
    target_provider: &str,
    mut entries: Vec<ProviderSyncRolloutScanState>,
) -> anyhow::Result<Vec<ProviderSyncRolloutScanState>> {
    let transaction = read_session_transaction(backup_dir)?;
    if transaction.status != SESSION_TRANSACTION_COMMITTED
        || transaction.mode != SessionTransactionMode::Full
    {
        anyhow::bail!("provider-sync rollout scan state requires a committed full transaction");
    }
    let committed = transaction
        .entries
        .iter()
        .map(|entry| (entry.relative_path.as_str(), entry))
        .collect::<HashMap<_, _>>();
    let mut next_entries = Vec::with_capacity(entries.len());
    for mut state in entries.drain(..) {
        let relative = validated_rollout_relative_path(&state.relative_path)?;
        let path = home.join(relative);
        if let Some(entry) = committed.get(state.relative_path.as_str()) {
            state.size = entry.next_size;
            state.sha256 = entry.next_sha256.clone();
            if state.session_meta_count > 0 {
                state.providers = vec![target_provider.to_string()];
            }
            let Ok(file) = File::open(&path) else {
                continue;
            };
            let Ok(metadata) = file.metadata() else {
                continue;
            };
            let Ok(file_identity) = codex_plus_core::settings::file_instance_identity(&file) else {
                continue;
            };
            let modified = metadata.modified().ok();
            if !metadata.is_file() || metadata.len() != entry.next_size || modified.is_none() {
                continue;
            }
            state.file_identity = file_identity;
            (state.modified_secs, state.modified_nanos) = system_time_parts(modified);
            next_entries.push(state);
        } else {
            let providers = state.providers.iter().cloned().collect::<HashSet<_>>();
            let still_needs_rewrite = state.session_meta_count > 0
                && !state.marks_non_root_agent
                && rewrite_needed_for_providers(
                    &providers,
                    target_provider,
                    &SessionRewriteMode::AllProviders,
                );
            if !still_needs_rewrite && provider_sync_scan_state_matches(&path, &state) {
                next_entries.push(state);
            }
        }
    }
    Ok(next_entries)
}

fn persist_committed_provider_sync_scan_state_best_effort(
    home: &Path,
    backup_dir: &Path,
    target_provider: &str,
    entries: Vec<ProviderSyncRolloutScanState>,
) {
    match committed_provider_sync_scan_state(home, backup_dir, target_provider, entries) {
        Ok(entries) => persist_provider_sync_scan_state_best_effort(home, &entries),
        Err(error) => {
            let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                "provider_sync.scan_state_commit_failed",
                json!({
                    "error": error.to_string(),
                    "backup_dir": backup_dir.to_string_lossy(),
                }),
            );
        }
    }
}

fn provider_sync_scan_state_matches(path: &Path, state: &ProviderSyncRolloutScanState) -> bool {
    let Ok(file) = File::open(path) else {
        return false;
    };
    let Ok(metadata) = file.metadata() else {
        return false;
    };
    let Ok(file_identity) = codex_plus_core::settings::file_instance_identity(&file) else {
        return false;
    };
    let (modified_secs, modified_nanos) = system_time_parts(metadata.modified().ok());
    state.modified_secs.is_some()
        && metadata.is_file()
        && metadata.len() == state.size
        && file_identity == state.file_identity
        && modified_secs == state.modified_secs
        && modified_nanos == state.modified_nanos
}

fn rewrite_needed_for_providers(
    providers: &HashSet<String>,
    target_provider: &str,
    rewrite_mode: &SessionRewriteMode,
) -> bool {
    match rewrite_mode {
        SessionRewriteMode::AllProviders => {
            providers.iter().any(|provider| provider != target_provider)
        }
        SessionRewriteMode::SourceProvider { source_provider } => providers
            .iter()
            .any(|provider| provider == "(missing)" || provider == source_provider),
    }
}

fn rollout_rewrite_from_scan_state(
    state: &ProviderSyncRolloutScanState,
    target_provider: &str,
    rewrite_mode: &SessionRewriteMode,
) -> RolloutRewrite {
    let providers = state.providers.iter().cloned().collect::<HashSet<_>>();
    RolloutRewrite {
        rewrite_needed: rewrite_needed_for_providers(&providers, target_provider, rewrite_mode),
        thread_id: state.thread_id.clone(),
        cwd: state.cwd.clone(),
        providers,
        session_meta_count: state.session_meta_count,
        has_user_event: state.has_user_event,
        has_encrypted_content: state.has_encrypted_content,
        marks_non_root_agent: state.marks_non_root_agent,
        original_sha256: state.sha256.clone(),
        original_size: state.size,
    }
}

fn provider_sync_cached_state_can_skip_body(
    state: &ProviderSyncRolloutScanState,
    target_provider: &str,
    excluded_thread_ids: &HashSet<String>,
    explicit_user_thread_ids: &HashSet<String>,
) -> bool {
    if state.session_meta_count == 0 || state.marks_non_root_agent {
        return true;
    }
    let is_explicit_user = state
        .thread_id
        .as_ref()
        .is_some_and(|thread_id| explicit_user_thread_ids.contains(thread_id));
    if !is_explicit_user
        && state
            .thread_id
            .as_ref()
            .is_some_and(|thread_id| excluded_thread_ids.contains(thread_id))
    {
        return true;
    }
    let providers = state.providers.iter().cloned().collect::<HashSet<_>>();
    !rewrite_needed_for_providers(
        &providers,
        target_provider,
        &SessionRewriteMode::AllProviders,
    )
}

fn scan_rollout_for_provider_sync_state(
    home: &Path,
    path: &Path,
    target_provider: &str,
    rewrite_mode: &SessionRewriteMode,
) -> std::io::Result<(
    RolloutRewrite,
    Option<SystemTime>,
    Option<ProviderSyncRolloutScanState>,
)> {
    let mut file = File::open(path)?;
    let before = file.metadata()?;
    let before_modified = before.modified().ok();
    let before_identity = codex_plus_core::settings::file_instance_identity(&file).ok();
    let rewrite =
        scan_rollout_session_meta_providers_from_file(&mut file, target_provider, rewrite_mode)?;
    let after = file.metadata()?;
    let after_modified = after.modified().ok();
    let after_identity = codex_plus_core::settings::file_instance_identity(&file).ok();
    let current = File::open(path)?;
    let current_metadata = current.metadata()?;
    let current_modified = current_metadata.modified().ok();
    let current_identity = codex_plus_core::settings::file_instance_identity(&current).ok();
    let stable = before.len() == after.len()
        && after.len() == rewrite.original_size
        && after.len() == current_metadata.len()
        && before_modified.is_some()
        && before_modified == after_modified
        && after_modified == current_modified
        && before_identity.is_some()
        && before_identity == after_identity
        && after_identity == current_identity;
    let mut providers = rewrite.providers.iter().cloned().collect::<Vec<_>>();
    providers.sort();
    let relative_path = rollout_relative_path(home, path)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    let (modified_secs, modified_nanos) = system_time_parts(after_modified);
    let state = stable.then(|| ProviderSyncRolloutScanState {
        relative_path,
        size: rewrite.original_size,
        modified_secs,
        modified_nanos,
        file_identity: after_identity.expect("stable rollout scan has file identity"),
        sha256: rewrite.original_sha256.clone(),
        thread_id: rewrite.thread_id.clone(),
        cwd: rewrite.cwd.clone(),
        has_user_event: rewrite.has_user_event,
        has_encrypted_content: rewrite.has_encrypted_content,
        marks_non_root_agent: rewrite.marks_non_root_agent,
        session_meta_count: rewrite.session_meta_count,
        providers,
    });
    Ok((rewrite, after_modified, state))
}

fn collect_session_changes(
    home: &Path,
    target_provider: &str,
    excluded_thread_ids: &HashSet<String>,
    explicit_user_thread_ids: &HashSet<String>,
) -> anyhow::Result<SessionChanges> {
    collect_session_changes_with_scanner(
        home,
        target_provider,
        excluded_thread_ids,
        explicit_user_thread_ids,
        load_provider_sync_scan_state_best_effort(home),
        scan_rollout_for_provider_sync_state,
    )
}

fn collect_session_changes_with_scanner<F>(
    home: &Path,
    target_provider: &str,
    excluded_thread_ids: &HashSet<String>,
    explicit_user_thread_ids: &HashSet<String>,
    mut scan_state: HashMap<String, ProviderSyncRolloutScanState>,
    mut scanner: F,
) -> anyhow::Result<SessionChanges>
where
    F: FnMut(
        &Path,
        &Path,
        &str,
        &SessionRewriteMode,
    ) -> std::io::Result<(
        RolloutRewrite,
        Option<SystemTime>,
        Option<ProviderSyncRolloutScanState>,
    )>,
{
    let mut collected = SessionChanges::default();
    for path in rollout_files(home)? {
        let relative_path = rollout_relative_path(home, &path)?;
        let cached = scan_state
            .remove(&relative_path)
            .filter(|state| provider_sync_scan_state_matches(&path, state))
            .filter(|state| {
                provider_sync_cached_state_can_skip_body(
                    state,
                    target_provider,
                    excluded_thread_ids,
                    explicit_user_thread_ids,
                )
            });
        let (rewrite, original_mtime, scan_state_entry) = match cached {
            Some(state) => {
                let original_mtime = fs::metadata(&path).and_then(|value| value.modified()).ok();
                (
                    rollout_rewrite_from_scan_state(
                        &state,
                        target_provider,
                        &SessionRewriteMode::AllProviders,
                    ),
                    original_mtime,
                    Some(state),
                )
            }
            None => match scanner(
                home,
                &path,
                target_provider,
                &SessionRewriteMode::AllProviders,
            ) {
                Ok(scanned) => scanned,
                Err(error) if is_locked_io_error(&error) => {
                    collected.skipped_locked_rollout_files.push(path);
                    continue;
                }
                Err(error) => return Err(error.into()),
            },
        };
        if let Some(scan_state_entry) = scan_state_entry {
            collected.scan_state_entries.push(scan_state_entry);
        }
        if rewrite.session_meta_count == 0 {
            continue;
        }
        let is_explicit_user = rewrite
            .thread_id
            .as_ref()
            .is_some_and(|thread_id| explicit_user_thread_ids.contains(thread_id));
        if rewrite.marks_non_root_agent {
            if let Some(thread_id) = &rewrite.thread_id {
                collected.subagent_thread_ids.insert(thread_id.clone());
            }
            continue;
        }
        if !is_explicit_user
            && rewrite
                .thread_id
                .as_ref()
                .is_some_and(|thread_id| excluded_thread_ids.contains(thread_id))
        {
            continue;
        }
        if rewrite.has_encrypted_content {
            for provider in &rewrite.providers {
                *collected
                    .encrypted_content_counts
                    .entry(provider.clone())
                    .or_insert(0) += 1;
            }
        }
        collected.changes.push(SessionChange {
            path,
            original_sha256: rewrite.original_sha256,
            original_size: rewrite.original_size,
            thread_id: rewrite.thread_id,
            cwd: rewrite.cwd,
            has_user_event: rewrite.has_user_event,
            rewrite_needed: rewrite.rewrite_needed,
            original_mtime,
            rewrite_mode: SessionRewriteMode::AllProviders,
        });
    }
    Ok(collected)
}

fn remote_control_rollout_for_thread(
    home: &Path,
    paths: &[PathBuf],
    thread_id: &str,
    target_provider: &str,
) -> anyhow::Result<RemoteControlRolloutLookup> {
    let mut archived_seen = false;
    let mut unsupported_seen = false;
    let mut candidate_seen = false;

    for path in paths {
        if !path.exists() {
            continue;
        }
        let db = Connection::open(path)?;
        let columns = table_columns(&db, "threads")?;
        if !columns.contains("id") {
            continue;
        }
        let provider_expr = if columns.contains("model_provider") {
            "COALESCE(model_provider, '')"
        } else {
            "''"
        };
        let archived_expr = if columns.contains("archived") {
            "COALESCE(archived, 0)"
        } else {
            "0"
        };
        let rollout_expr = if columns.contains("rollout_path") {
            "COALESCE(rollout_path, '')"
        } else {
            "''"
        };
        let sql = format!(
            "SELECT {provider_expr}, {archived_expr}, {rollout_expr} FROM threads WHERE id = ?1"
        );
        let mut stmt = db.prepare(&sql)?;
        let rows = stmt.query_map([thread_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        for row in rows {
            let (provider, archived, rollout_path) = row?;
            candidate_seen = true;
            if archived != 0 {
                archived_seen = true;
                continue;
            }
            if !provider.is_empty() && provider != DEFAULT_PROVIDER && provider != target_provider {
                unsupported_seen = true;
                continue;
            }
            let Some(rollout_path) = resolve_active_rollout_path(home, &rollout_path) else {
                continue;
            };
            let Some((rollout_thread_id, providers)) =
                rollout_provider_state_for_path(&rollout_path)?
            else {
                continue;
            };
            if rollout_thread_id != thread_id {
                continue;
            }
            if providers.is_empty()
                || providers
                    .iter()
                    .any(|provider| provider != DEFAULT_PROVIDER && provider != target_provider)
            {
                unsupported_seen = true;
                continue;
            }
            return Ok(RemoteControlRolloutLookup::Ready(rollout_path));
        }
    }

    if archived_seen && !unsupported_seen {
        Ok(RemoteControlRolloutLookup::Archived)
    } else if unsupported_seen {
        Ok(RemoteControlRolloutLookup::UnsupportedProvider)
    } else if candidate_seen {
        Ok(RemoteControlRolloutLookup::Missing)
    } else {
        Ok(RemoteControlRolloutLookup::Missing)
    }
}

fn resolve_active_rollout_path(home: &Path, value: &str) -> Option<PathBuf> {
    let raw = value.trim();
    if raw.is_empty() {
        return None;
    }
    let path = PathBuf::from(raw);
    let path = if path.is_absolute() {
        path
    } else {
        home.join(path)
    };
    let canonical = fs::canonicalize(path).ok()?;
    let sessions_root = fs::canonicalize(home.join("sessions")).ok()?;
    if !canonical.starts_with(sessions_root) {
        return None;
    }
    Some(canonical)
}

fn rollout_provider_state_for_path(
    path: &Path,
) -> anyhow::Result<Option<(String, HashSet<String>)>> {
    let rewrite = match scan_rollout_session_meta_providers(
        path,
        DEFAULT_PROVIDER,
        &SessionRewriteMode::AllProviders,
    ) {
        Ok(rewrite) => rewrite,
        Err(error) if is_locked_io_error(&error) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    Ok(rewrite
        .thread_id
        .map(|thread_id| (thread_id, rewrite.providers.into_iter().collect())))
}

fn collect_session_change_for_path(
    path: &Path,
    target_provider: &str,
    source_provider: &str,
    thread_id: &str,
) -> anyhow::Result<SessionChanges> {
    let mut collected = SessionChanges::default();
    let rewrite_mode = SessionRewriteMode::SourceProvider {
        source_provider: source_provider.to_string(),
    };
    let rewrite = match scan_rollout_session_meta_providers(path, target_provider, &rewrite_mode) {
        Ok(rewrite) => rewrite,
        Err(error) if is_locked_io_error(&error) => {
            collected
                .skipped_locked_rollout_files
                .push(path.to_path_buf());
            return Ok(collected);
        }
        Err(error) => return Err(error.into()),
    };
    if rewrite.session_meta_count == 0 || rewrite.thread_id.as_deref() != Some(thread_id) {
        return Ok(collected);
    }
    if rewrite.has_encrypted_content {
        for provider in &rewrite.providers {
            *collected
                .encrypted_content_counts
                .entry(provider.clone())
                .or_insert(0) += 1;
        }
    }
    let original_mtime = fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok();
    collected.changes.push(SessionChange {
        path: path.to_path_buf(),
        original_sha256: rewrite.original_sha256,
        original_size: rewrite.original_size,
        thread_id: rewrite.thread_id,
        cwd: rewrite.cwd,
        has_user_event: rewrite.has_user_event,
        rewrite_needed: rewrite.rewrite_needed,
        original_mtime,
        rewrite_mode,
    });
    Ok(collected)
}

fn rollout_file_matches_provider(
    path: &Path,
    thread_id: &str,
    target_provider: &str,
) -> anyhow::Result<bool> {
    let Some((rollout_thread_id, providers)) = rollout_provider_state_for_path(path)? else {
        return Ok(false);
    };
    Ok(rollout_thread_id == thread_id
        && !providers.is_empty()
        && providers.iter().all(|provider| provider == target_provider))
}

fn scan_rollout_session_meta_providers(
    path: &Path,
    target_provider: &str,
    rewrite_mode: &SessionRewriteMode,
) -> std::io::Result<RolloutRewrite> {
    let mut file = File::open(path)?;
    scan_rollout_session_meta_providers_from_file(&mut file, target_provider, rewrite_mode)
}

fn scan_rollout_session_meta_providers_from_file(
    file: &mut File,
    target_provider: &str,
    rewrite_mode: &SessionRewriteMode,
) -> std::io::Result<RolloutRewrite> {
    let mut rewrite = RolloutRewrite::default();
    file.seek(SeekFrom::Start(0))?;
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    let mut hasher = Sha256::new();
    loop {
        line.clear();
        let read = reader.read_until(b'\n', &mut line)?;
        if read == 0 {
            break;
        }
        rewrite.original_size += read as u64;
        hasher.update(&line);
        rewrite.has_user_event |=
            contains_bytes(&line, b"\"user_message\"") || contains_bytes(&line, b"\"user_input\"");
        rewrite.has_encrypted_content |= contains_bytes(&line, b"encrypted_content");
        let (line_bytes, _) = split_line_ending_bytes(&line);
        let line_text = std::str::from_utf8(line_bytes)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        if !contains_bytes(line_bytes, b"\"session_meta\"") {
            continue;
        }
        let Ok(mut record) = serde_json::from_str::<Value>(line_text) else {
            continue;
        };
        if record.get("type").and_then(Value::as_str) != Some("session_meta") {
            continue;
        }
        rewrite.marks_non_root_agent |= record
            .get("payload")
            .and_then(|payload| payload.get("source"))
            .is_some_and(source_value_marks_non_root_agent);
        let Some(payload) = record.get_mut("payload").and_then(Value::as_object_mut) else {
            continue;
        };
        rewrite.session_meta_count += 1;
        if rewrite.thread_id.is_none() {
            rewrite.thread_id = payload
                .get("id")
                .and_then(Value::as_str)
                .map(ToString::to_string);
        }
        if rewrite.cwd.is_none() {
            rewrite.cwd = payload
                .get("cwd")
                .and_then(Value::as_str)
                .and_then(to_desktop_workspace_path);
        }
        let provider = payload
            .get("model_provider")
            .and_then(Value::as_str)
            .map(ToString::to_string);
        rewrite
            .providers
            .insert(provider.clone().unwrap_or_else(|| "(missing)".to_string()));
        rewrite.rewrite_needed |= match rewrite_mode {
            SessionRewriteMode::AllProviders => provider.as_deref() != Some(target_provider),
            SessionRewriteMode::SourceProvider { source_provider } => provider
                .as_deref()
                .is_none_or(|provider| provider == source_provider),
        };
    }
    rewrite.original_sha256 = format!("{:x}", hasher.finalize());
    Ok(rewrite)
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

fn split_line_ending_bytes(line: &[u8]) -> (&[u8], &[u8]) {
    if line.ends_with(b"\r\n") {
        (&line[..line.len() - 2], &line[line.len() - 2..])
    } else if line.ends_with(b"\n") {
        (&line[..line.len() - 1], &line[line.len() - 1..])
    } else {
        (line, &[])
    }
}

fn rollout_files(home: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let canonical_home = fs::canonicalize(home)?;
    for dirname in SESSION_DIRS {
        let root = home.join(dirname);
        if root.exists() {
            ensure_not_reparse_or_symlink(&root)?;
            let canonical_root = fs::canonicalize(&root)?;
            if !canonical_root.starts_with(&canonical_home) {
                anyhow::bail!("provider-sync rollout root resolves outside Codex home");
            }
            collect_rollout_files(&root, &mut files)?;
        }
    }
    files.sort();
    Ok(files)
}

fn collect_live_thread_ids(
    home: &Path,
    sqlite_paths: &[PathBuf],
) -> anyhow::Result<HashSet<String>> {
    let mut ids = HashSet::new();
    for path in rollout_files(home)? {
        if let Some(id) = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(rollout_thread_id_from_filename)
        {
            ids.insert(id);
        }
        let rewrite = match scan_rollout_session_meta_providers(
            &path,
            DEFAULT_PROVIDER,
            &SessionRewriteMode::AllProviders,
        ) {
            Ok(rewrite) => rewrite,
            Err(error) if is_locked_io_error(&error) => continue,
            Err(error) => return Err(error.into()),
        };
        if let Some(id) = rewrite.thread_id.filter(|id| !id.trim().is_empty()) {
            ids.insert(id);
        }
    }
    for path in sqlite_paths {
        ids.extend(sqlite_thread_ids(path)?);
    }
    Ok(ids)
}

fn rollout_thread_id_from_filename(name: &str) -> Option<String> {
    let stem = name.strip_prefix("rollout-")?.strip_suffix(".jsonl")?;
    let bytes = stem.as_bytes();
    if bytes.len() < 36 {
        return None;
    }
    let candidate = &stem[stem.len() - 36..];
    let valid = candidate
        .chars()
        .enumerate()
        .all(|(index, ch)| match index {
            8 | 13 | 18 | 23 => ch == '-',
            _ => ch.is_ascii_hexdigit(),
        });
    valid.then(|| candidate.to_string())
}

fn sqlite_thread_ids(path: &Path) -> anyhow::Result<HashSet<String>> {
    if !path.exists() {
        return Ok(HashSet::new());
    }
    let db = Connection::open(path)?;
    let mut ids = HashSet::new();
    for (table, column) in [
        ("threads", "id"),
        ("local_thread_catalog", "thread_id"),
        ("automation_runs", "thread_id"),
        ("inbox_items", "thread_id"),
        ("sessions", "id"),
        ("messages", "session_id"),
        ("thread_dynamic_tools", "thread_id"),
        ("thread_goals", "thread_id"),
        ("thread_spawn_edges", "parent_thread_id"),
        ("thread_spawn_edges", "child_thread_id"),
        ("stage1_outputs", "thread_id"),
        ("agent_job_items", "assigned_thread_id"),
    ] {
        if !table_columns(&db, table)?.contains(column) {
            continue;
        }
        let mut stmt = db.prepare(&format!(
            "SELECT DISTINCT {column} FROM {table} WHERE COALESCE({column}, '') <> ''"
        ))?;
        ids.extend(
            stmt.query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<HashSet<_>>>()?,
        );
    }
    Ok(ids)
}

fn sqlite_table_ids(path: &Path, table: &str, column: &str) -> anyhow::Result<HashSet<String>> {
    if !path.exists() {
        return Ok(HashSet::new());
    }
    let db = Connection::open(path)?;
    if !table_columns(&db, table)?.contains(column) {
        return Ok(HashSet::new());
    }
    let sql = format!("SELECT DISTINCT {column} FROM {table} WHERE COALESCE({column}, '') <> ''");
    Ok(db
        .prepare(&sql)?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<HashSet<_>>>()?)
}

fn sqlite_user_thread_ids(path: &Path) -> anyhow::Result<HashSet<String>> {
    if !path.exists() {
        return Ok(HashSet::new());
    }
    let db = Connection::open(path)?;
    let columns = table_columns(&db, "local_thread_catalog")?;
    if !columns.contains("thread_id") {
        return Ok(HashSet::new());
    }
    let source_kind = text_expr(&columns, "source_kind", "''");
    let thread_source = text_expr(&columns, "thread_source", "NULL");
    let sql = format!(
        "SELECT thread_id, {source_kind}, {thread_source} FROM local_thread_catalog WHERE COALESCE(thread_id, '') <> ''"
    );
    let mut ids = HashSet::new();
    for row in db.prepare(&sql)?.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1).unwrap_or_default(),
            row.get::<_, Option<String>>(2).unwrap_or(None),
        ))
    })? {
        let (thread_id, source_kind, thread_source) = row?;
        if !thread_source_marks_non_root(thread_source.as_deref())
            && !source_marks_non_root_agent(&source_kind)
        {
            ids.insert(thread_id);
        }
    }
    Ok(ids)
}

fn plan_session_index_cleanup(
    path: &Path,
    live_thread_ids: &HashSet<String>,
) -> anyhow::Result<Option<SessionIndexPlan>> {
    if !path.exists() {
        return Ok(None);
    }
    let original_bytes = fs::read(path)?;
    let original_text = String::from_utf8(original_bytes.clone())?;
    let mut candidates = Vec::new();
    for segment in original_text.split_inclusive('\n') {
        let (line, _) = split_line_ending(segment);
        if let Some(candidate) = known_session_index_candidate(line)
            && !live_thread_ids.contains(&candidate.id)
        {
            candidates.push(candidate);
        }
    }
    Ok(Some(SessionIndexPlan {
        path: path.to_path_buf(),
        snapshot_sha256: sha256_hex(&original_bytes),
        original_bytes,
        original_text,
        candidates,
    }))
}

fn known_session_index_candidate(line: &str) -> Option<SessionIndexCleanupCandidate> {
    let record = serde_json::from_str::<Value>(line).ok()?;
    let object = record.as_object()?;
    if object.len() != 3
        || !["id", "thread_name", "updated_at"]
            .iter()
            .all(|key| object.contains_key(*key))
    {
        return None;
    }
    let id = object.get("id")?.as_str()?.trim();
    let thread_name = object.get("thread_name")?.as_str()?;
    let updated_at = object.get("updated_at")?.as_str()?;
    if id.is_empty() || updated_at.trim().is_empty() {
        return None;
    }
    Some(SessionIndexCleanupCandidate {
        id: id.to_string(),
        thread_name: thread_name.to_string(),
        updated_at: updated_at.to_string(),
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn filtered_session_index_text(
    plan: &SessionIndexPlan,
    selected_ids: &HashSet<String>,
) -> (String, usize) {
    let mut next_text = String::with_capacity(plan.original_text.len());
    let mut removed_entries = 0;
    for segment in plan.original_text.split_inclusive('\n') {
        let (line, line_ending) = split_line_ending(segment);
        let remove = known_session_index_candidate(line)
            .is_some_and(|candidate| selected_ids.contains(&candidate.id));
        if remove {
            removed_entries += 1;
        } else {
            next_text.push_str(line);
            next_text.push_str(line_ending);
        }
    }
    (next_text, removed_entries)
}

pub fn preview_session_index_cleanup(
    codex_home: Option<&Path>,
) -> anyhow::Result<SessionIndexCleanupPreview> {
    let home = codex_home
        .map(Path::to_path_buf)
        .unwrap_or_else(default_codex_home_dir);
    let sqlite_paths =
        codex_plus_core::codex_sqlite::codex_thread_reference_db_paths_from_home(&home);
    let live_thread_ids = collect_live_thread_ids(&home, &sqlite_paths)?;
    let plan = plan_session_index_cleanup(&home.join("session_index.jsonl"), &live_thread_ids)?;
    Ok(match plan {
        Some(plan) => SessionIndexCleanupPreview {
            snapshot_sha256: plan.snapshot_sha256,
            candidates: plan.candidates,
        },
        None => SessionIndexCleanupPreview {
            snapshot_sha256: sha256_hex(&[]),
            candidates: Vec::new(),
        },
    })
}

pub fn apply_session_index_cleanup(
    codex_home: Option<&Path>,
    expected_snapshot_sha256: &str,
    confirmed_thread_ids: &[String],
) -> Result<SessionIndexCleanupResult, SessionIndexCleanupApplyError> {
    let require_stopped_app = codex_home.is_none();
    if require_stopped_app {
        ensure_codex_app_stopped(None)?;
    }
    let home = codex_home
        .map(Path::to_path_buf)
        .unwrap_or_else(default_codex_home_dir);
    let lock_dir = home.join("tmp/provider-sync.lock");
    let _lock_guard = acquire_lock(&lock_dir).map_err(|error| cleanup_apply_error(error, None))?;
    let result = (|| {
        let sqlite_paths =
            codex_plus_core::codex_sqlite::codex_thread_reference_db_paths_from_home(&home);
        let live_thread_ids = collect_live_thread_ids(&home, &sqlite_paths)
            .map_err(|error| cleanup_apply_error(error, None))?;
        let plan = plan_session_index_cleanup(&home.join("session_index.jsonl"), &live_thread_ids)
            .map_err(|error| cleanup_apply_error(error, None))?
            .ok_or_else(|| cleanup_apply_error("session_index.jsonl 不存在，无法清理", None))?;
        if plan.snapshot_sha256 != expected_snapshot_sha256 {
            return Err(cleanup_apply_error(
                "session_index.jsonl 已在预览后发生变化；为避免覆盖 Codex 新内容，本次清理已中止，请重新预览",
                None,
            ));
        }
        let candidate_ids = plan
            .candidates
            .iter()
            .map(|candidate| candidate.id.as_str())
            .collect::<HashSet<_>>();
        let selected_ids = confirmed_thread_ids
            .iter()
            .map(|id| id.trim())
            .filter(|id| !id.is_empty())
            .map(ToString::to_string)
            .collect::<HashSet<_>>();
        if selected_ids
            .iter()
            .any(|id| !candidate_ids.contains(id.as_str()))
        {
            return Err(cleanup_apply_error(
                "确认列表已过期或包含非候选任务；本次清理未执行，请重新预览",
                None,
            ));
        }
        let (next_text, removed_entries) = filtered_session_index_text(&plan, &selected_ids);
        if removed_entries == 0 {
            return Ok(SessionIndexCleanupResult {
                pruned_entries: 0,
                backup_dir: None,
            });
        }
        let backup_dir = create_session_index_cleanup_backup(&home, &plan, removed_entries)?;
        let current_bytes = fs::read(&plan.path)
            .map_err(|error| cleanup_apply_error(error, Some(backup_dir.clone())))?;
        if current_bytes != plan.original_bytes {
            return Err(cleanup_apply_error(
                "session_index.jsonl 在写入前再次发生变化；未覆盖 Codex 新内容，请重新预览",
                Some(backup_dir),
            ));
        }
        if require_stopped_app {
            ensure_codex_app_stopped(Some(backup_dir.clone()))?;
        }
        codex_plus_core::settings::atomic_write(&plan.path, next_text.as_bytes()).map_err(
            |error| {
                cleanup_apply_error(
                    format!(
                        "原子写入 session_index.jsonl 失败；原文件未被主动覆盖，可从备份目录手动恢复：{error}"
                    ),
                    Some(backup_dir.clone()),
                )
            },
        )?;
        let _ = prune_backups(&home);
        Ok(SessionIndexCleanupResult {
            pruned_entries: removed_entries,
            backup_dir: Some(backup_dir),
        })
    })();
    result
}

/// Return the `session_index.jsonl` lines (without trailing newline) that
/// reference `thread_id`. Used by the delete flow to keep a backup of the
/// entries it is about to remove.
pub fn session_index_lines_for_thread(
    codex_home: &Path,
    thread_id: &str,
) -> anyhow::Result<Vec<String>> {
    let path = codex_home.join("session_index.jsonl");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(&path)?;
    let mut lines = Vec::new();
    for segment in text.split_inclusive('\n') {
        let (line, _) = split_line_ending(segment);
        if known_session_index_candidate(line).is_some_and(|candidate| candidate.id == thread_id) {
            lines.push(line.to_string());
        }
    }
    Ok(lines)
}

/// Remove every `session_index.jsonl` entry for `thread_id` and write the
/// result back atomically. Returns the number of removed entries.
///
/// Best-effort: returns `Ok(0)` without writing when the file is missing or
/// changed since it was read, so a delete flow never clobbers fresh entries.
pub fn remove_session_index_entry(codex_home: &Path, thread_id: &str) -> anyhow::Result<usize> {
    let path = codex_home.join("session_index.jsonl");
    if !path.exists() {
        return Ok(0);
    }
    let original_bytes = fs::read(&path)?;
    let original_text = String::from_utf8(original_bytes.clone())?;
    let plan = SessionIndexPlan {
        path,
        snapshot_sha256: sha256_hex(&original_bytes),
        original_bytes,
        original_text,
        candidates: Vec::new(),
    };
    let selected_ids = HashSet::from([thread_id.to_string()]);
    let (next_text, removed_entries) = filtered_session_index_text(&plan, &selected_ids);
    if removed_entries == 0 {
        return Ok(0);
    }
    if fs::read(&plan.path)? != plan.original_bytes {
        return Ok(0);
    }
    codex_plus_core::settings::atomic_write(&plan.path, next_text.as_bytes())?;
    Ok(removed_entries)
}

/// Append previously removed `session_index.jsonl` lines back (undo flow).
/// Lines whose `id` already exists are skipped. Returns the number of
/// appended lines. Best-effort: returns `Ok(0)` without writing when the
/// file changed since it was read.
pub fn restore_session_index_entries(codex_home: &Path, lines: &[String]) -> anyhow::Result<usize> {
    if lines.is_empty() {
        return Ok(0);
    }
    let path = codex_home.join("session_index.jsonl");
    let original_bytes = if path.exists() {
        fs::read(&path)?
    } else {
        Vec::new()
    };
    let original_text = String::from_utf8(original_bytes.clone())?;
    let mut existing_ids = HashSet::new();
    for segment in original_text.split_inclusive('\n') {
        let (line, _) = split_line_ending(segment);
        if let Some(candidate) = known_session_index_candidate(line) {
            existing_ids.insert(candidate.id);
        }
    }
    let mut next_text = original_text;
    if !next_text.is_empty() && !next_text.ends_with('\n') {
        next_text.push('\n');
    }
    let mut appended = 0usize;
    for line in lines {
        if let Some(candidate) = known_session_index_candidate(line) {
            if existing_ids.contains(&candidate.id) {
                continue;
            }
            existing_ids.insert(candidate.id);
        }
        next_text.push_str(line);
        next_text.push('\n');
        appended += 1;
    }
    if appended == 0 {
        return Ok(0);
    }
    if fs::read(&path)? != original_bytes {
        return Ok(0);
    }
    codex_plus_core::settings::atomic_write(&path, next_text.as_bytes())?;
    Ok(appended)
}

fn ensure_codex_app_stopped(
    backup_dir: Option<PathBuf>,
) -> Result<(), SessionIndexCleanupApplyError> {
    let running_processes =
        codex_plus_core::watcher::find_session_index_cleanup_blocking_processes();
    if running_processes.is_empty() {
        return Ok(());
    }
    Err(cleanup_apply_error(
        format!(
            "Codex App / ChatGPT 仍在运行（进程：{}）；请完全退出 App 后重新预览并确认清理",
            running_processes
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        backup_dir,
    ))
}

fn cleanup_apply_error(
    message: impl std::fmt::Display,
    backup_dir: Option<PathBuf>,
) -> SessionIndexCleanupApplyError {
    SessionIndexCleanupApplyError {
        message: message.to_string(),
        backup_dir,
    }
}

fn rollout_provider_ids(home: &Path) -> anyhow::Result<Vec<String>> {
    let mut ids = HashSet::new();
    for path in rollout_files(home)? {
        let rewrite = match scan_rollout_session_meta_providers(
            &path,
            DEFAULT_PROVIDER,
            &SessionRewriteMode::AllProviders,
        ) {
            Ok(rewrite) => rewrite,
            Err(error) if is_locked_io_error(&error) => continue,
            Err(error) => return Err(error.into()),
        };
        for provider in rewrite.providers {
            if is_valid_provider_id_for_discovery(&provider) {
                ids.insert(provider);
            }
        }
    }
    Ok(sorted_provider_ids(ids))
}

fn collect_rollout_files(root: &Path, files: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        ensure_not_reparse_or_symlink(&path)?;
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_rollout_files(&path, files)?;
        } else if file_type.is_file()
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("rollout-") && name.ends_with(".jsonl"))
        {
            files.push(path);
        }
    }
    Ok(())
}

fn split_line_ending(segment: &str) -> (&str, &str) {
    if let Some(line) = segment.strip_suffix("\r\n") {
        (line, "\r\n")
    } else if let Some(line) = segment.strip_suffix('\n') {
        (line, "\n")
    } else {
        (segment, "")
    }
}

fn to_desktop_workspace_path(value: &str) -> Option<String> {
    let stripped = value.trim();
    if stripped.is_empty() {
        return None;
    }
    let lower = stripped.to_ascii_lowercase();
    if lower.starts_with(r"\\?\unc\") {
        return Some(format!(r"\\{}", stripped[8..].replace('/', r"\")));
    }
    if stripped.starts_with(r"\\?\") {
        return Some(stripped[4..].replace('\\', "/"));
    }
    Some(stripped.to_string())
}

fn is_locked_io_error(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::WouldBlock
    ) || matches!(error.raw_os_error(), Some(32 | 33))
}

fn build_encrypted_content_warning(
    encrypted_content_counts: &HashMap<String, usize>,
    target_provider: &str,
) -> Option<String> {
    let risky_providers = encrypted_content_counts
        .iter()
        .filter(|(provider, count)| provider.as_str() != target_provider && **count > 0)
        .map(|(provider, _)| provider.as_str())
        .collect::<Vec<_>>();
    if risky_providers.is_empty() {
        return None;
    }
    let total = encrypted_content_counts.values().sum::<usize>();
    Some(format!(
        "检测到 {total} 个会话文件包含来自 {} 的 encrypted_content。可见会话元数据已同步到 {target_provider}，但继续或压缩这些历史可能出现 invalid_encrypted_content；需要可靠续聊时请切回原供应商/账号或开启新会话。",
        risky_providers.join(", ")
    ))
}

fn create_backup(
    home: &Path,
    target_provider: &str,
    mode: SessionTransactionMode,
    changes: &[SessionChange],
) -> anyhow::Result<PathBuf> {
    let backup_root = home.join("backups_state/provider-sync");
    let mut backup_dir = backup_root.join(timestamp_name());
    let mut suffix = 0;
    while backup_dir.exists() {
        suffix += 1;
        backup_dir = backup_root.join(format!("{}-{suffix}", timestamp_name()));
    }
    fs::create_dir_all(&backup_dir)?;
    let mut global_state_files = Vec::new();
    let mut backup_files = HashMap::new();
    for name in [
        "config.toml",
        ".codex-global-state.json",
        ".codex-global-state.json.bak",
    ] {
        let source = home.join(name);
        if source.exists() {
            let target = backup_dir.join(name);
            fs::copy(&source, &target)?;
            if name != "config.toml" {
                global_state_files.push(name);
                let (sha256, size) = file_sha256_and_size(&target)?;
                backup_files.insert(
                    name.to_string(),
                    ProviderSyncBackupFileEvidence { size, sha256 },
                );
            }
        }
    }
    let db_dir = backup_dir.join("db");
    let mut db_files = Vec::new();
    for db_path in provider_sync_db_paths(home) {
        for source in codex_plus_core::codex_sqlite::codex_sqlite_sidecar_paths(&db_path) {
            if !source.exists() {
                continue;
            }
            let relative = source.strip_prefix(home).map_err(|_| {
                anyhow::anyhow!(
                    "provider-sync database is outside Codex home: {}",
                    source.display()
                )
            })?;
            validated_backup_relative_path(relative)?;
            let target = db_dir.join(&relative);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&source, &target)?;
            let relative = relative.to_string_lossy().replace('\\', "/");
            let (sha256, size) = file_sha256_and_size(&target)?;
            backup_files.insert(
                format!("db/{relative}"),
                ProviderSyncBackupFileEvidence { size, sha256 },
            );
            db_files.push(relative);
        }
    }
    fs::write(
        backup_dir.join("session-meta-backup.json"),
        serde_json::to_string_pretty(&Vec::<Value>::new())?,
    )?;
    fs::write(
        backup_dir.join("metadata.json"),
        serde_json::to_string_pretty(&json!({
            "version": 1,
            "namespace": "provider-sync",
            "codexHome": home.to_string_lossy(),
            "targetProvider": target_provider,
            "createdAt": chrono::Utc::now().to_rfc3339(),
            "dbFiles": db_files,
            "globalStateFiles": global_state_files,
            "backupFiles": backup_files,
            "changedSessionFiles": changes.len(),
            "managedBy": "Codex++ provider sync"
        }))?,
    )?;
    write_session_transaction(
        &backup_dir,
        &SessionTransactionManifest {
            version: 1,
            namespace: SESSION_TRANSACTION_NAMESPACE.to_string(),
            status: SESSION_TRANSACTION_IN_PROGRESS.to_string(),
            transaction_id: uuid::Uuid::new_v4().simple().to_string(),
            mode,
            phase: SessionTransactionPhase::RolloutsApplying,
            rollout_roots: session_transaction_rollout_roots(home)?,
            entries: Vec::new(),
        },
    )?;
    Ok(backup_dir)
}

fn create_session_index_cleanup_backup(
    home: &Path,
    plan: &SessionIndexPlan,
    removed_entries: usize,
) -> Result<PathBuf, SessionIndexCleanupApplyError> {
    let backup_root = home.join("backups_state/provider-sync");
    let mut backup_dir = backup_root.join(timestamp_name());
    let mut suffix = 0;
    while backup_dir.exists() {
        suffix += 1;
        backup_dir = backup_root.join(format!("{}-{suffix}", timestamp_name()));
    }
    fs::create_dir_all(&backup_dir).map_err(|error| cleanup_apply_error(error, None))?;
    fs::write(backup_dir.join("session_index.jsonl"), &plan.original_bytes)
        .map_err(|error| cleanup_apply_error(error, Some(backup_dir.clone())))?;
    let metadata = serde_json::to_string_pretty(&json!({
        "version": 1,
        "namespace": "provider-sync-session-index-cleanup",
        "codexHome": home.to_string_lossy(),
        "createdAt": chrono::Utc::now().to_rfc3339(),
        "snapshotSha256": plan.snapshot_sha256,
        "prunedSessionIndexEntries": removed_entries,
        "managedBy": "Codex++ provider sync"
    }))
    .map_err(|error| cleanup_apply_error(error, Some(backup_dir.clone())))?;
    fs::write(backup_dir.join("metadata.json"), metadata)
        .map_err(|error| cleanup_apply_error(error, Some(backup_dir.clone())))?;
    Ok(backup_dir)
}

fn apply_session_changes(
    home: &Path,
    backup_dir: &Path,
    target_provider: &str,
    changes: &[SessionChange],
) -> anyhow::Result<AppliedSessionChanges> {
    let apply_result = (|| -> anyhow::Result<AppliedSessionChanges> {
        if !changes.is_empty() {
            invalidate_provider_sync_scan_state(home)?;
        }
        let mut applied = AppliedSessionChanges::default();
        let mut transaction = read_session_transaction(backup_dir)?;
        for change in changes {
            let entry_index = transaction.entries.len();
            let relative_path = rollout_relative_path(home, &change.path)?;
            let target_path = validated_rollout_transaction_path(
                home,
                &relative_path,
                &transaction.rollout_roots,
            )?;
            let staged_file_name = format!(
                ".{}.provider-sync-{}-{}.tmp",
                target_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("rollout"),
                transaction.transaction_id,
                entry_index
            );
            let staged_path = target_path
                .parent()
                .ok_or_else(|| anyhow::anyhow!("rollout has no parent: {}", target_path.display()))?
                .join(&staged_file_name);
            let session_meta_backup_path =
                backup_dir.join(format!("session-meta/{entry_index}.jsonl"));
            let mut source = match open_session_file_for_update(&target_path) {
                Ok(source) => source,
                Err(error) if is_locked_io_error(&error) => {
                    applied
                        .skipped_locked_rollout_files
                        .push(change.path.clone());
                    continue;
                }
                Err(error) => return Err(error.into()),
            };
            if source.try_lock().is_err() {
                applied
                    .skipped_locked_rollout_files
                    .push(change.path.clone());
                continue;
            }
            let staged = match stage_next_session_file(
                &mut source,
                &staged_path,
                &session_meta_backup_path,
                target_provider,
                &change.rewrite_mode,
            ) {
                Ok(staged) => staged,
                Err(error) => {
                    let _ = fs::remove_file(&staged_path);
                    let _ = fs::remove_file(&session_meta_backup_path);
                    return Err(error.into());
                }
            };
            if staged.original_sha256 != change.original_sha256
                || staged.original_size != change.original_size
            {
                drop(source);
                let _ = fs::remove_file(&staged_path);
                let _ = fs::remove_file(&session_meta_backup_path);
                applied
                    .skipped_locked_rollout_files
                    .push(change.path.clone());
                continue;
            }
            let (original_mtime_secs, original_mtime_nanos) =
                system_time_parts(change.original_mtime);
            transaction.entries.push(SessionTransactionEntry {
                relative_path: relative_path.clone(),
                original_sha256: staged.original_sha256,
                next_sha256: staged.next_sha256.clone(),
                original_size: staged.original_size,
                next_size: staged.next_size,
                session_meta_backup_sha256: staged.session_meta_backup_sha256,
                original_mtime_secs,
                original_mtime_nanos,
                external_sha256: None,
                external_size: None,
            });
            if let Err(error) = write_session_transaction(backup_dir, &transaction) {
                transaction.entries.pop();
                drop(source);
                let _ = fs::remove_file(&staged_path);
                let _ = fs::remove_file(&session_meta_backup_path);
                let _ = rollback_session_transaction(home, backup_dir);
                return Err(error);
            }
            if let Err(error) = write_session_meta_backup_manifest(backup_dir, &transaction) {
                drop(source);
                let _ = fs::remove_file(&staged_path);
                let _ = rollback_session_transaction(home, backup_dir);
                return Err(error);
            }
            drop(source);
            let replacement_target = validated_rollout_transaction_path(
                home,
                &relative_path,
                &transaction.rollout_roots,
            )?;
            if replacement_target != target_path {
                anyhow::bail!("provider-sync rollout path changed before replacement");
            }
            let displaced_path =
                transaction_displaced_path(&target_path, &transaction.transaction_id, entry_index)?;
            if let Err(error) = codex_plus_core::settings::atomic_replace_file_with_backup(
                &staged_path,
                &target_path,
                &displaced_path,
            ) {
                let _ = fs::remove_file(&staged_path);
                if is_locked_io_error_from_anyhow(&error) {
                    transaction.entries.pop();
                    write_session_transaction(backup_dir, &transaction)?;
                    write_session_meta_backup_manifest(backup_dir, &transaction)?;
                    let _ = fs::remove_file(&session_meta_backup_path);
                    applied
                        .skipped_locked_rollout_files
                        .push(change.path.clone());
                    continue;
                }
                let _ = rollback_session_transaction(home, backup_dir);
                return Err(error);
            }
            let (displaced_sha256, displaced_size) = file_sha256_and_size(&displaced_path)?;
            if displaced_sha256 != change.original_sha256 || displaced_size != change.original_size
            {
                let entry = transaction
                    .entries
                    .last_mut()
                    .ok_or_else(|| anyhow::anyhow!("provider-sync transaction entry missing"))?;
                entry.external_sha256 = Some(displaced_sha256);
                entry.external_size = Some(displaced_size);
                write_session_transaction(backup_dir, &transaction)?;
                restore_displaced_session_file(
                    &target_path,
                    &displaced_path,
                    &transaction.transaction_id,
                    entry_index,
                )?;
                anyhow::bail!(
                    "rollout changed between provider-sync staging and replacement: {}",
                    target_path.display()
                );
            }
            restore_file_mtime(&target_path, change.original_mtime);
            let (persisted_sha256, persisted_size) = file_sha256_and_size(&target_path)?;
            if persisted_sha256 != staged.next_sha256 || persisted_size != staged.next_size {
                let _ = rollback_session_transaction(home, backup_dir);
                anyhow::bail!(
                    "rollout changed while provider metadata was being replaced: {}",
                    target_path.display()
                );
            }
            fs::remove_file(displaced_path)?;
            applied.changed_files += 1;
        }
        Ok(applied)
    })();
    if apply_result.is_err() {
        let _ = rollback_session_transaction(home, backup_dir);
    }
    apply_result
}

#[derive(Debug)]
struct StagedSessionFile {
    original_sha256: String,
    next_sha256: String,
    original_size: u64,
    next_size: u64,
    session_meta_backup_sha256: String,
}

fn stage_next_session_file(
    source: &mut File,
    staged_path: &Path,
    session_meta_backup_path: &Path,
    target_provider: &str,
    rewrite_mode: &SessionRewriteMode,
) -> std::io::Result<StagedSessionFile> {
    let staged_file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(staged_path)?;
    if let Some(parent) = session_meta_backup_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let session_meta_backup_file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(session_meta_backup_path)?;
    let mut reader = BufReader::new(source);
    let mut writer = BufWriter::new(staged_file);
    let mut session_meta_writer = BufWriter::new(session_meta_backup_file);
    let mut line = Vec::new();
    let mut original_hasher = Sha256::new();
    let mut next_hasher = Sha256::new();
    let mut session_meta_hasher = Sha256::new();
    let mut original_size = 0_u64;
    let mut next_size = 0_u64;
    loop {
        line.clear();
        let read = reader.read_until(b'\n', &mut line)?;
        if read == 0 {
            break;
        }
        original_size += read as u64;
        original_hasher.update(&line);
        let (line_bytes, line_ending) = split_line_ending_bytes(&line);
        let line_text = std::str::from_utf8(line_bytes)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        if session_meta_line_has_payload_object(line_text) {
            let backup_line = serde_json::to_vec(line_text)
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
            session_meta_writer.write_all(&backup_line)?;
            session_meta_writer.write_all(b"\n")?;
            session_meta_hasher.update(&backup_line);
            session_meta_hasher.update(b"\n");
        }
        if let Some(next_line) =
            rewritten_session_meta_line(line_text, target_provider, rewrite_mode)?
        {
            writer.write_all(&next_line)?;
            writer.write_all(line_ending)?;
            next_hasher.update(&next_line);
            next_hasher.update(line_ending);
            next_size += (next_line.len() + line_ending.len()) as u64;
        } else {
            writer.write_all(&line)?;
            next_hasher.update(&line);
            next_size += line.len() as u64;
        }
    }
    writer.flush()?;
    writer.get_ref().sync_all()?;
    session_meta_writer.flush()?;
    session_meta_writer.get_ref().sync_all()?;
    Ok(StagedSessionFile {
        original_sha256: format!("{:x}", original_hasher.finalize()),
        next_sha256: format!("{:x}", next_hasher.finalize()),
        original_size,
        next_size,
        session_meta_backup_sha256: format!("{:x}", session_meta_hasher.finalize()),
    })
}

fn write_session_meta_backup_manifest(
    backup_dir: &Path,
    transaction: &SessionTransactionManifest,
) -> anyhow::Result<()> {
    let manifest = transaction
        .entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            json!({
                "path": entry.relative_path,
                "originalSha256": entry.original_sha256,
                "originalSize": entry.original_size,
                "sessionMetaBackup": format!("session-meta/{index}.jsonl"),
            })
        })
        .collect::<Vec<_>>();
    codex_plus_core::settings::atomic_write(
        &backup_dir.join("session-meta-backup.json"),
        &serde_json::to_vec_pretty(&manifest)?,
    )
}

fn session_meta_line_has_payload_object(line: &str) -> bool {
    line.contains("\"session_meta\"")
        && serde_json::from_str::<Value>(line).is_ok_and(|record| {
            record.get("type").and_then(Value::as_str) == Some("session_meta")
                && record.get("payload").and_then(Value::as_object).is_some()
        })
}

fn rewritten_session_meta_line(
    line: &str,
    target_provider: &str,
    rewrite_mode: &SessionRewriteMode,
) -> std::io::Result<Option<Vec<u8>>> {
    if !line.contains("\"session_meta\"") {
        return Ok(None);
    }
    let Ok(mut record) = serde_json::from_str::<Value>(line) else {
        return Ok(None);
    };
    if record.get("type").and_then(Value::as_str) != Some("session_meta") {
        return Ok(None);
    }
    let Some(payload) = record.get_mut("payload").and_then(Value::as_object_mut) else {
        return Ok(None);
    };
    let provider = payload.get("model_provider").and_then(Value::as_str);
    let rewrite = match rewrite_mode {
        SessionRewriteMode::AllProviders => provider != Some(target_provider),
        SessionRewriteMode::SourceProvider { source_provider } => {
            provider.is_none_or(|provider| provider == source_provider)
        }
    };
    if !rewrite {
        return Ok(None);
    }
    payload.insert("model_provider".to_string(), json!(target_provider));
    serde_json::to_vec(&record)
        .map(Some)
        .map_err(std::io::Error::other)
}

fn transaction_displaced_path(
    path: &Path,
    transaction_id: &str,
    entry_index: usize,
) -> anyhow::Result<PathBuf> {
    Ok(path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("rollout has no parent: {}", path.display()))?
        .join(format!(
            ".{}.provider-sync-displaced-{}-{}.tmp",
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("rollout"),
            transaction_id,
            entry_index
        )))
}

fn restore_displaced_session_file(
    path: &Path,
    displaced_path: &Path,
    transaction_id: &str,
    entry_index: usize,
) -> anyhow::Result<()> {
    if !path.exists() {
        return codex_plus_core::settings::atomic_replace_file(displaced_path, path);
    }
    let rejected_path = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("rollout has no parent: {}", path.display()))?
        .join(format!(
            ".{}.provider-sync-rejected-{}-{}.tmp",
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("rollout"),
            transaction_id,
            entry_index
        ));
    codex_plus_core::settings::atomic_replace_file_with_backup(
        displaced_path,
        path,
        &rejected_path,
    )?;
    fs::remove_file(rejected_path)?;
    Ok(())
}

fn rollback_session_transaction(home: &Path, backup_dir: &Path) -> anyhow::Result<()> {
    let mut transaction = read_session_transaction(backup_dir)?;
    if transaction.status == SESSION_TRANSACTION_COMMITTED
        || transaction.status == SESSION_TRANSACTION_ROLLED_BACK
    {
        return Ok(());
    }
    if transaction.status != SESSION_TRANSACTION_IN_PROGRESS {
        anyhow::bail!("unknown provider-sync rollout transaction status");
    }
    for entry_index in (0..transaction.entries.len()).rev() {
        let entry = transaction.entries[entry_index].clone();
        let path = validated_rollout_transaction_path(
            home,
            &entry.relative_path,
            &transaction.rollout_roots,
        )?;
        let displaced_path =
            transaction_displaced_path(&path, &transaction.transaction_id, entry_index)?;
        if let (Some(external_sha256), Some(external_size)) =
            (entry.external_sha256.as_deref(), entry.external_size)
        {
            if path.exists() {
                let (current_sha256, current_size) = file_sha256_and_size(&path)?;
                if current_sha256 == external_sha256 && current_size == external_size {
                    if displaced_path.exists() {
                        let (displaced_sha256, displaced_size) =
                            file_sha256_and_size(&displaced_path)?;
                        let displaced_is_original = displaced_sha256 == entry.original_sha256
                            && displaced_size == entry.original_size;
                        let displaced_is_external =
                            displaced_sha256 == external_sha256 && displaced_size == external_size;
                        if !displaced_is_original && !displaced_is_external {
                            anyhow::bail!(
                                "provider-sync displaced file changed after external restore: {}",
                                displaced_path.display()
                            );
                        }
                        fs::remove_file(&displaced_path)?;
                    }
                    continue;
                }
                if current_sha256 != entry.next_sha256 || current_size != entry.next_size {
                    anyhow::bail!(
                        "rollout changed after external restore decision: {}",
                        path.display()
                    );
                }
            }
            if displaced_path.exists() {
                let (displaced_sha256, displaced_size) = file_sha256_and_size(&displaced_path)?;
                if displaced_sha256 != external_sha256 || displaced_size != external_size {
                    anyhow::bail!(
                        "provider-sync displaced external version changed: {}",
                        displaced_path.display()
                    );
                }
                restore_displaced_session_file(
                    &path,
                    &displaced_path,
                    &transaction.transaction_id,
                    entry_index,
                )?;
                continue;
            }
            anyhow::bail!(
                "provider-sync external version is missing after restore decision: {}",
                path.display()
            );
        }
        if displaced_path.exists() {
            let (displaced_sha256, displaced_size) = file_sha256_and_size(&displaced_path)?;
            if !path.exists() {
                restore_displaced_session_file(
                    &path,
                    &displaced_path,
                    &transaction.transaction_id,
                    entry_index,
                )?;
                if displaced_sha256 == entry.original_sha256
                    && displaced_size == entry.original_size
                {
                    restore_file_mtime_parts(
                        &path,
                        entry.original_mtime_secs,
                        entry.original_mtime_nanos,
                    );
                }
                continue;
            }
            let (current_sha256, current_size) = file_sha256_and_size(&path)?;
            if displaced_sha256 == entry.original_sha256 && displaced_size == entry.original_size {
                if current_sha256 == entry.original_sha256 && current_size == entry.original_size {
                    fs::remove_file(&displaced_path)?;
                    continue;
                }
                if current_sha256 != entry.next_sha256 || current_size != entry.next_size {
                    transaction.entries[entry_index].external_sha256 = Some(current_sha256);
                    transaction.entries[entry_index].external_size = Some(current_size);
                    write_session_transaction(backup_dir, &transaction)?;
                    fs::remove_file(&displaced_path)?;
                    continue;
                }
                restore_displaced_session_file(
                    &path,
                    &displaced_path,
                    &transaction.transaction_id,
                    entry_index,
                )?;
                restore_file_mtime_parts(
                    &path,
                    entry.original_mtime_secs,
                    entry.original_mtime_nanos,
                );
                continue;
            }
            if current_sha256 != entry.next_sha256 || current_size != entry.next_size {
                anyhow::bail!(
                    "rollout and displaced file both changed; refusing recovery: {}",
                    path.display()
                );
            }
            transaction.entries[entry_index].external_sha256 = Some(displaced_sha256);
            transaction.entries[entry_index].external_size = Some(displaced_size);
            write_session_transaction(backup_dir, &transaction)?;
            restore_displaced_session_file(
                &path,
                &displaced_path,
                &transaction.transaction_id,
                entry_index,
            )?;
            continue;
        }
        let (current_sha256, current_size) = file_sha256_and_size(&path)?;
        if current_sha256 == entry.original_sha256 && current_size == entry.original_size {
            continue;
        }
        if current_sha256 != entry.next_sha256 || current_size != entry.next_size {
            anyhow::bail!(
                "rollout changed after provider-sync replacement; refusing rollback: {}",
                path.display()
            );
        }
        let restore_name = format!(
            ".{}.provider-sync-restore-{}.tmp",
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("rollout"),
            transaction.transaction_id
        );
        let restore_path = path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("rollout has no parent: {}", path.display()))?
            .join(restore_name);
        let restored = match stage_original_session_file(
            &path,
            &restore_path,
            backup_dir,
            entry_index,
            &entry,
        ) {
            Ok(restored) => restored,
            Err(error) => {
                let _ = fs::remove_file(&restore_path);
                return Err(error);
            }
        };
        if restored.0 != entry.original_sha256 || restored.1 != entry.original_size {
            let _ = fs::remove_file(&restore_path);
            anyhow::bail!("provider-sync rollback hash mismatch: {}", path.display());
        }
        codex_plus_core::settings::atomic_replace_file(&restore_path, &path)?;
        restore_file_mtime_parts(&path, entry.original_mtime_secs, entry.original_mtime_nanos);
    }
    cleanup_transaction_staged_files(home, &transaction.transaction_id)?;
    transaction.status = SESSION_TRANSACTION_ROLLED_BACK.to_string();
    write_session_transaction(backup_dir, &transaction)
}

fn stage_original_session_file(
    path: &Path,
    staged_path: &Path,
    backup_dir: &Path,
    entry_index: usize,
    entry: &SessionTransactionEntry,
) -> anyhow::Result<(String, u64)> {
    let mut source = open_session_file_for_update(path)?;
    source.try_lock()?;
    let staged_file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(staged_path)?;
    let mut reader = BufReader::new(&mut source);
    let mut writer = BufWriter::new(staged_file);
    let canonical_backup_dir = fs::canonicalize(backup_dir)?;
    let session_meta_backup_path = backup_dir.join(format!("session-meta/{entry_index}.jsonl"));
    let canonical_session_meta_backup_path = fs::canonicalize(&session_meta_backup_path)?;
    if !canonical_session_meta_backup_path.starts_with(&canonical_backup_dir) {
        anyhow::bail!("provider-sync session-meta backup resolves outside its backup directory");
    }
    if !fs::symlink_metadata(&session_meta_backup_path)?
        .file_type()
        .is_file()
    {
        anyhow::bail!("provider-sync session-meta backup is not a regular file");
    }
    let mut session_meta_reader = BufReader::new(File::open(&session_meta_backup_path)?);
    let mut line = Vec::new();
    let mut session_meta_line = Vec::new();
    let mut hasher = Sha256::new();
    let mut session_meta_hasher = Sha256::new();
    let mut size = 0_u64;
    loop {
        line.clear();
        let read = reader.read_until(b'\n', &mut line)?;
        if read == 0 {
            break;
        }
        let (line_bytes, line_ending) = split_line_ending_bytes(&line);
        let line_text = std::str::from_utf8(line_bytes)?;
        let is_session_meta = if line_text.contains("\"session_meta\"") {
            serde_json::from_str::<Value>(line_text).is_ok_and(|record| {
                record.get("type").and_then(Value::as_str) == Some("session_meta")
                    && record.get("payload").and_then(Value::as_object).is_some()
            })
        } else {
            false
        };
        if is_session_meta {
            session_meta_line.clear();
            if session_meta_reader.read_until(b'\n', &mut session_meta_line)? == 0 {
                anyhow::bail!("provider-sync rollback metadata is incomplete");
            }
            session_meta_hasher.update(&session_meta_line);
            let backup_line = session_meta_line
                .strip_suffix(b"\n")
                .unwrap_or(&session_meta_line);
            let original_line: String = serde_json::from_slice(backup_line)?;
            writer.write_all(original_line.as_bytes())?;
            writer.write_all(line_ending)?;
            hasher.update(original_line.as_bytes());
            hasher.update(line_ending);
            size += (original_line.len() + line_ending.len()) as u64;
        } else {
            writer.write_all(&line)?;
            hasher.update(&line);
            size += line.len() as u64;
        }
    }
    session_meta_line.clear();
    if session_meta_reader.read_until(b'\n', &mut session_meta_line)? != 0 {
        anyhow::bail!("provider-sync rollback metadata count mismatch");
    }
    let session_meta_backup_sha256 = format!("{:x}", session_meta_hasher.finalize());
    if session_meta_backup_sha256 != entry.session_meta_backup_sha256 {
        anyhow::bail!("provider-sync session-meta backup hash mismatch");
    }
    writer.flush()?;
    writer.get_ref().sync_all()?;
    Ok((format!("{:x}", hasher.finalize()), size))
}

fn commit_session_transaction(backup_dir: &Path) -> anyhow::Result<()> {
    let mut transaction = read_session_transaction(backup_dir)?;
    if transaction.phase != SessionTransactionPhase::CommitDecided {
        anyhow::bail!("provider-sync rollout transaction has no commit decision");
    }
    transaction.status = SESSION_TRANSACTION_COMMITTED.to_string();
    write_session_transaction(backup_dir, &transaction)
}

fn set_session_transaction_phase(
    backup_dir: &Path,
    phase: SessionTransactionPhase,
) -> anyhow::Result<()> {
    let mut transaction = read_session_transaction(backup_dir)?;
    if transaction.status != SESSION_TRANSACTION_IN_PROGRESS {
        anyhow::bail!("provider-sync rollout transaction is no longer active");
    }
    let valid = matches!(
        (transaction.phase, phase),
        (
            SessionTransactionPhase::RolloutsApplying,
            SessionTransactionPhase::RolloutsApplied
        ) | (
            SessionTransactionPhase::RolloutsApplied,
            SessionTransactionPhase::DownstreamStarted
        ) | (
            SessionTransactionPhase::RolloutsApplied,
            SessionTransactionPhase::CommitDecided
        ) | (
            SessionTransactionPhase::DownstreamStarted,
            SessionTransactionPhase::CommitDecided
        )
    );
    if !valid {
        anyhow::bail!("invalid provider-sync rollout transaction phase transition");
    }
    transaction.phase = phase;
    write_session_transaction(backup_dir, &transaction)
}

fn recover_interrupted_session_transactions(home: &Path) -> anyhow::Result<()> {
    let backup_root = home.join("backups_state/provider-sync");
    let entries = match fs::read_dir(&backup_root) {
        Ok(entries) => entries,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
            ) =>
        {
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    };
    let mut backup_dirs = Vec::new();
    for entry in entries {
        let entry = entry?;
        if entry.file_type()?.is_dir() && entry.path().join(SESSION_TRANSACTION_FILE).is_file() {
            backup_dirs.push(entry.path());
        }
    }
    backup_dirs.sort();
    for backup_dir in backup_dirs {
        let transaction = read_session_transaction(&backup_dir)?;
        if transaction.status != SESSION_TRANSACTION_IN_PROGRESS {
            continue;
        }
        match (transaction.mode, transaction.phase) {
            (_, SessionTransactionPhase::CommitDecided) => {
                commit_session_transaction(&backup_dir)?;
            }
            (SessionTransactionMode::Full, SessionTransactionPhase::DownstreamStarted) => {
                restore_provider_sync_downstream_backup(home, &backup_dir)?;
                rollback_session_transaction(home, &backup_dir)?;
            }
            _ => rollback_session_transaction(home, &backup_dir)?,
        }
    }
    Ok(())
}

fn restore_provider_sync_downstream_backup(home: &Path, backup_dir: &Path) -> anyhow::Result<()> {
    let transaction = read_session_transaction(backup_dir)?;
    if transaction.mode != SessionTransactionMode::Full {
        anyhow::bail!("remote provider-sync transactions do not own downstream state");
    }
    let metadata: Value = serde_json::from_slice(&fs::read(backup_dir.join("metadata.json"))?)?;
    let db_files = metadata
        .get("dbFiles")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("provider-sync backup is missing dbFiles"))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("provider-sync backup has an invalid dbFiles entry"))
                .and_then(|value| validated_backup_relative_path(Path::new(value)))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let allowed_db_files = provider_sync_db_paths(home)
        .into_iter()
        .flat_map(|db_path| codex_plus_core::codex_sqlite::codex_sqlite_sidecar_paths(&db_path))
        .map(|path| {
            let relative = path.strip_prefix(home).map_err(|_| {
                anyhow::anyhow!(
                    "provider-sync database is outside Codex home: {}",
                    path.display()
                )
            })?;
            Ok(validated_backup_relative_path(relative)?
                .to_string_lossy()
                .replace('\\', "/"))
        })
        .collect::<anyhow::Result<HashSet<_>>>()?;
    let db_file_set = db_files
        .iter()
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .collect::<HashSet<_>>();
    if !db_file_set.is_subset(&allowed_db_files) {
        anyhow::bail!("provider-sync backup dbFiles contains an unexpected path");
    }
    let actual_backup_db_files = collect_backup_relative_files(&backup_dir.join("db"))?;
    if db_file_set != actual_backup_db_files {
        anyhow::bail!("provider-sync backup dbFiles does not match the backup directory");
    }

    let global_state_files = metadata
        .get("globalStateFiles")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("provider-sync backup is missing globalStateFiles"))?
        .iter()
        .map(|value| {
            value.as_str().ok_or_else(|| {
                anyhow::anyhow!("provider-sync backup has an invalid globalStateFiles entry")
            })
        })
        .collect::<anyhow::Result<HashSet<_>>>()?;
    let allowed_global_state_files =
        HashSet::from([".codex-global-state.json", ".codex-global-state.json.bak"]);
    if !global_state_files.is_subset(&allowed_global_state_files) {
        anyhow::bail!("provider-sync backup globalStateFiles contains an unexpected path");
    }
    let actual_global_state_files = allowed_global_state_files
        .iter()
        .copied()
        .filter(|name| {
            fs::symlink_metadata(backup_dir.join(name))
                .is_ok_and(|metadata| metadata.file_type().is_file())
        })
        .collect::<HashSet<_>>();
    if global_state_files != actual_global_state_files {
        anyhow::bail!("provider-sync backup globalStateFiles does not match the backup directory");
    }

    let backup_files: HashMap<String, ProviderSyncBackupFileEvidence> = serde_json::from_value(
        metadata
            .get("backupFiles")
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("provider-sync backup is missing backupFiles"))?,
    )?;
    let expected_backup_files = db_file_set
        .iter()
        .map(|relative| format!("db/{relative}"))
        .chain(global_state_files.iter().map(|name| (*name).to_string()))
        .collect::<HashSet<_>>();
    let recorded_backup_files = backup_files.keys().cloned().collect::<HashSet<_>>();
    if recorded_backup_files != expected_backup_files {
        anyhow::bail!("provider-sync backupFiles does not match the recorded backup files");
    }
    let mut prepared_backup_files =
        prepare_provider_sync_backup_files(backup_dir, &backup_files, &transaction.transaction_id)?;
    let target_parents = prepare_provider_sync_target_parents(home, &db_files)?;

    for db_path in provider_sync_db_paths(home) {
        for (index, current) in codex_plus_core::codex_sqlite::codex_sqlite_sidecar_paths(&db_path)
            .into_iter()
            .enumerate()
        {
            let Ok(relative) = current.strip_prefix(home) else {
                anyhow::bail!(
                    "provider-sync database is outside Codex home: {}",
                    current.display()
                );
            };
            let relative = validated_backup_relative_path(relative)?;
            let key = relative.to_string_lossy().replace('\\', "/");
            if index > 0 && current.exists() && !db_file_set.contains(&key) {
                let parent_evidence =
                    validate_provider_sync_target_parent(home, &current, &target_parents)?;
                ensure_not_reparse_or_symlink(&current)?;
                remove_provider_sync_target_file(&current, parent_evidence)?;
            }
        }
    }
    for relative in db_files {
        let key = format!("db/{}", relative.to_string_lossy().replace('\\', "/"));
        let prepared = prepared_backup_files
            .files
            .get_mut(&key)
            .ok_or_else(|| anyhow::anyhow!("provider-sync backup file evidence is missing"))?;
        let target = home.join(&relative);
        restore_file_from_provider_sync_backup(
            home,
            &target,
            &transaction.transaction_id,
            prepared,
            &target_parents,
        )?;
    }
    for name in [".codex-global-state.json", ".codex-global-state.json.bak"] {
        let target = home.join(name);
        if global_state_files.contains(name) {
            let prepared = prepared_backup_files
                .files
                .get_mut(name)
                .ok_or_else(|| anyhow::anyhow!("provider-sync backup file evidence is missing"))?;
            restore_file_from_provider_sync_backup(
                home,
                &target,
                &transaction.transaction_id,
                prepared,
                &target_parents,
            )?;
        } else if target.exists() {
            let parent_evidence =
                validate_provider_sync_target_parent(home, &target, &target_parents)?;
            ensure_not_reparse_or_symlink(&target)?;
            remove_provider_sync_target_file(&target, parent_evidence)?;
        }
    }
    Ok(())
}

fn prepare_provider_sync_backup_files(
    backup_dir: &Path,
    backup_files: &HashMap<String, ProviderSyncBackupFileEvidence>,
    transaction_id: &str,
) -> anyhow::Result<PreparedProviderSyncBackupSet> {
    let canonical_backup_dir = fs::canonicalize(backup_dir)?;
    let snapshot_dir = std::env::temp_dir().join(format!(
        "codex-plus-provider-sync-{transaction_id}-{}",
        uuid::Uuid::new_v4().simple()
    ));
    fs::create_dir(&snapshot_dir)?;
    ensure_not_reparse_or_symlink(&snapshot_dir)?;
    let mut prepared = PreparedProviderSyncBackupSet {
        files: HashMap::new(),
        snapshot_dir,
    };
    for (index, (relative, evidence)) in backup_files.iter().enumerate() {
        if evidence.sha256.len() != 64
            || !evidence.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            anyhow::bail!("provider-sync backupFiles contains an invalid hash");
        }
        let relative_path = validated_backup_relative_path(Path::new(relative))?;
        let source = backup_dir.join(relative_path);
        ensure_not_reparse_or_symlink(&source)?;
        if !fs::symlink_metadata(&source)?.file_type().is_file() {
            anyhow::bail!("provider-sync backupFiles contains a non-file entry");
        }
        let canonical_source = fs::canonicalize(&source)?;
        if !canonical_source.starts_with(&canonical_backup_dir) {
            anyhow::bail!("provider-sync backup file resolves outside its backup directory");
        }
        let mut source_file = File::open(&source)?;
        let modified = source_file.metadata()?.modified().ok();
        let snapshot_path = prepared.snapshot_dir.join(format!("{index}.snapshot"));
        let mut snapshot = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&snapshot_path)?;
        let mut hasher = Sha256::new();
        let mut size = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = source_file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            snapshot.write_all(&buffer[..read])?;
            hasher.update(&buffer[..read]);
            size += read as u64;
        }
        snapshot.sync_all()?;
        let sha256 = format!("{:x}", hasher.finalize());
        if sha256 != evidence.sha256 || size != evidence.size {
            anyhow::bail!("provider-sync backup file hash or size mismatch");
        }
        snapshot.seek(SeekFrom::Start(0))?;
        snapshot.lock_exclusive()?;
        #[cfg(unix)]
        fs::remove_file(&snapshot_path)?;
        prepared.files.insert(
            relative.clone(),
            PreparedProviderSyncBackupFile {
                file: snapshot,
                modified,
                evidence: evidence.clone(),
            },
        );
    }
    Ok(prepared)
}

fn file_handle_sha256_and_size(file: &mut File) -> std::io::Result<(String, u64)> {
    let mut hasher = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        size += read as u64;
    }
    Ok((format!("{:x}", hasher.finalize()), size))
}

fn prepare_provider_sync_target_parents(
    home: &Path,
    db_files: &[PathBuf],
) -> anyhow::Result<HashMap<PathBuf, ProviderSyncDirectoryEvidence>> {
    let mut parents = HashSet::from([home.to_path_buf()]);
    for relative in db_files {
        let target = home.join(relative);
        let parent = target
            .parent()
            .ok_or_else(|| anyhow::anyhow!("backup restore target has no parent"))?;
        parents.insert(parent.to_path_buf());
    }
    let canonical_home = fs::canonicalize(home)?;
    let mut evidence = HashMap::new();
    for parent in parents {
        create_validated_directory_path(home, &parent)?;
        let canonical_path = fs::canonicalize(&parent)?;
        if !canonical_path.starts_with(&canonical_home) {
            anyhow::bail!("provider-sync backup target resolves outside Codex home");
        }
        let guard = open_directory_mutation_guard(&parent)?;
        let guarded_identity = codex_plus_core::settings::file_instance_identity(&guard)?;
        if fs::canonicalize(&parent)? != canonical_path
            || codex_plus_core::settings::directory_instance_identity(&parent)? != guarded_identity
        {
            anyhow::bail!("provider-sync backup target parent changed while being guarded");
        }
        evidence.insert(
            parent.clone(),
            ProviderSyncDirectoryEvidence {
                canonical_path,
                identity: guarded_identity,
                guard,
            },
        );
    }
    Ok(evidence)
}

fn create_validated_directory_path(home: &Path, target: &Path) -> anyhow::Result<()> {
    let relative = target
        .strip_prefix(home)
        .map_err(|_| anyhow::anyhow!("provider-sync target parent is outside Codex home"))?;
    let mut current = home.to_path_buf();
    for component in relative.components() {
        let std::path::Component::Normal(name) = component else {
            anyhow::bail!("provider-sync target parent contains an invalid component");
        };
        current.push(name);
        if current.exists() {
            ensure_not_reparse_or_symlink(&current)?;
        } else {
            fs::create_dir(&current)?;
            ensure_not_reparse_or_symlink(&current)?;
        }
    }
    Ok(())
}

fn validate_provider_sync_target_parent<'a>(
    home: &Path,
    target: &Path,
    target_parents: &'a HashMap<PathBuf, ProviderSyncDirectoryEvidence>,
) -> anyhow::Result<&'a ProviderSyncDirectoryEvidence> {
    let parent = target
        .parent()
        .ok_or_else(|| anyhow::anyhow!("backup restore target has no parent"))?;
    let expected = target_parents
        .get(parent)
        .ok_or_else(|| anyhow::anyhow!("provider-sync backup target parent was not recorded"))?;
    ensure_path_components_not_reparse(home, parent)?;
    let canonical_parent = fs::canonicalize(parent)?;
    if canonical_parent != expected.canonical_path
        || codex_plus_core::settings::directory_instance_identity(parent)? != expected.identity
    {
        anyhow::bail!("provider-sync backup target parent changed during restore");
    }
    Ok(expected)
}

#[cfg(windows)]
fn open_directory_mutation_guard(path: &Path) -> std::io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_SHARE_WRITE: u32 = 0x0000_0002;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
}

#[cfg(not(windows))]
fn open_directory_mutation_guard(path: &Path) -> std::io::Result<File> {
    File::open(path)
}

fn restore_file_from_provider_sync_backup(
    home: &Path,
    target: &Path,
    transaction_id: &str,
    prepared: &mut PreparedProviderSyncBackupFile,
    target_parents: &HashMap<PathBuf, ProviderSyncDirectoryEvidence>,
) -> anyhow::Result<()> {
    let parent_evidence = validate_provider_sync_target_parent(home, target, target_parents)?;
    if target.exists() {
        ensure_not_reparse_or_symlink(target)?;
    }
    let parent = target
        .parent()
        .ok_or_else(|| anyhow::anyhow!("backup restore target has no parent"))?;
    let temp = parent.join(format!(
        ".{}.provider-sync-downstream-{}.tmp",
        target
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("state"),
        transaction_id
    ));
    let restore_result = (|| -> anyhow::Result<()> {
        if let Err(error) = remove_provider_sync_target_file(&temp, parent_evidence)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            return Err(error.into());
        }
        let mut temp_file = create_provider_sync_target_file(&temp, parent_evidence)?;
        prepared.file.seek(SeekFrom::Start(0))?;
        std::io::copy(&mut prepared.file, &mut temp_file)?;
        temp_file.sync_all()?;
        temp_file.seek(SeekFrom::Start(0))?;
        let (sha256, size) = file_handle_sha256_and_size(&mut temp_file)?;
        if sha256 != prepared.evidence.sha256 || size != prepared.evidence.size {
            anyhow::bail!("provider-sync backup changed during restore");
        }
        drop(temp_file);
        let parent_evidence = validate_provider_sync_target_parent(home, target, target_parents)?;
        if target.exists() {
            ensure_not_reparse_or_symlink(target)?;
        }
        atomic_replace_provider_sync_target(&temp, target, parent_evidence)?;
        restore_file_mtime(target, prepared.modified);
        Ok(())
    })();
    if restore_result.is_err() {
        let _ = remove_provider_sync_target_file(&temp, parent_evidence);
    }
    restore_result
}

#[cfg(unix)]
fn create_provider_sync_target_file(
    path: &Path,
    parent: &ProviderSyncDirectoryEvidence,
) -> std::io::Result<File> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;

    let name = path.file_name().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "missing file name")
    })?;
    let name = CString::new(name.as_bytes())
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid file name"))?;
    let fd = unsafe {
        libc::openat(
            parent.guard.as_raw_fd(),
            name.as_ptr(),
            libc::O_CREAT | libc::O_EXCL | libc::O_RDWR | libc::O_CLOEXEC,
            0o600,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}

#[cfg(not(unix))]
fn create_provider_sync_target_file(
    path: &Path,
    parent: &ProviderSyncDirectoryEvidence,
) -> std::io::Result<File> {
    let _directory_guard = &parent.guard;
    OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(path)
}

#[cfg(unix)]
fn remove_provider_sync_target_file(
    path: &Path,
    parent: &ProviderSyncDirectoryEvidence,
) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;

    let name = path.file_name().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "missing file name")
    })?;
    let name = CString::new(name.as_bytes())
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid file name"))?;
    if unsafe { libc::unlinkat(parent.guard.as_raw_fd(), name.as_ptr(), 0) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(unix))]
fn remove_provider_sync_target_file(
    path: &Path,
    parent: &ProviderSyncDirectoryEvidence,
) -> std::io::Result<()> {
    let _directory_guard = &parent.guard;
    fs::remove_file(path)
}

#[cfg(unix)]
fn atomic_replace_provider_sync_target(
    replacement: &Path,
    target: &Path,
    parent: &ProviderSyncDirectoryEvidence,
) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;

    let replacement = replacement.file_name().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "missing replacement name")
    })?;
    let target = target.file_name().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "missing target name")
    })?;
    let replacement = CString::new(replacement.as_bytes()).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid replacement name")
    })?;
    let target = CString::new(target.as_bytes()).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid target name")
    })?;
    if unsafe {
        libc::renameat(
            parent.guard.as_raw_fd(),
            replacement.as_ptr(),
            parent.guard.as_raw_fd(),
            target.as_ptr(),
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(unix))]
fn atomic_replace_provider_sync_target(
    replacement: &Path,
    target: &Path,
    parent: &ProviderSyncDirectoryEvidence,
) -> anyhow::Result<()> {
    let _directory_guard = &parent.guard;
    codex_plus_core::settings::atomic_replace_file(replacement, target)
}

fn validated_backup_relative_path(path: &Path) -> anyhow::Result<PathBuf> {
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        anyhow::bail!("invalid provider-sync backup path");
    }
    Ok(path.to_path_buf())
}

fn collect_backup_relative_files(root: &Path) -> anyhow::Result<HashSet<String>> {
    if !root.exists() {
        return Ok(HashSet::new());
    }
    let canonical_root = fs::canonicalize(root)?;
    let mut files = HashSet::new();
    collect_backup_relative_files_in(root, &canonical_root, &mut files)?;
    Ok(files)
}

fn collect_backup_relative_files_in(
    root: &Path,
    canonical_root: &Path,
    files: &mut HashSet<String>,
) -> anyhow::Result<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_backup_relative_files_in(&path, canonical_root, files)?;
            continue;
        }
        if !file_type.is_file() {
            anyhow::bail!("provider-sync backup db directory contains a non-file entry");
        }
        let canonical_path = fs::canonicalize(&path)?;
        let relative = canonical_path.strip_prefix(canonical_root).map_err(|_| {
            anyhow::anyhow!("provider-sync backup db file resolves outside its directory")
        })?;
        files.insert(
            validated_backup_relative_path(relative)?
                .to_string_lossy()
                .replace('\\', "/"),
        );
    }
    Ok(())
}

fn read_session_transaction(backup_dir: &Path) -> anyhow::Result<SessionTransactionManifest> {
    let transaction: SessionTransactionManifest =
        serde_json::from_slice(&fs::read(backup_dir.join(SESSION_TRANSACTION_FILE))?)?;
    if transaction.version != 1 || transaction.namespace != SESSION_TRANSACTION_NAMESPACE {
        anyhow::bail!("unsupported provider-sync rollout transaction manifest");
    }
    if transaction.transaction_id.len() != 32
        || !transaction
            .transaction_id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        anyhow::bail!("invalid provider-sync rollout transaction id");
    }
    for (name, root) in &transaction.rollout_roots {
        if !SESSION_DIRS.contains(&name.as_str())
            || !Path::new(&root.canonical_path).is_absolute()
            || root.identity.trim().is_empty()
        {
            anyhow::bail!("invalid provider-sync rollout root evidence");
        }
    }
    for entry in &transaction.entries {
        for hash in [
            Some(entry.original_sha256.as_str()),
            Some(entry.next_sha256.as_str()),
            Some(entry.session_meta_backup_sha256.as_str()),
            entry.external_sha256.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                anyhow::bail!("invalid provider-sync rollout transaction hash");
            }
        }
        if entry.external_sha256.is_some() != entry.external_size.is_some() {
            anyhow::bail!("incomplete provider-sync external restore decision");
        }
    }
    Ok(transaction)
}

fn write_session_transaction(
    backup_dir: &Path,
    transaction: &SessionTransactionManifest,
) -> anyhow::Result<()> {
    codex_plus_core::settings::atomic_write(
        &backup_dir.join(SESSION_TRANSACTION_FILE),
        &serde_json::to_vec_pretty(transaction)?,
    )
}

fn rollout_relative_path(home: &Path, path: &Path) -> anyhow::Result<String> {
    let canonical_home = fs::canonicalize(home)?;
    let relative = path
        .strip_prefix(home)
        .or_else(|_| path.strip_prefix(&canonical_home))
        .map_err(|_| anyhow::anyhow!("rollout is outside Codex home: {}", path.display()))?;
    validated_rollout_relative_path(&relative.to_string_lossy().replace('\\', "/"))?;
    Ok(relative.to_string_lossy().replace('\\', "/"))
}

fn validated_rollout_relative_path(value: &str) -> anyhow::Result<PathBuf> {
    let path = PathBuf::from(value);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        anyhow::bail!("invalid provider-sync rollout transaction path");
    }
    let Some(first) = path.components().next() else {
        anyhow::bail!("empty provider-sync rollout transaction path");
    };
    let std::path::Component::Normal(first) = first else {
        anyhow::bail!("invalid provider-sync rollout transaction root");
    };
    if !SESSION_DIRS
        .iter()
        .any(|root| first == std::ffi::OsStr::new(root))
    {
        anyhow::bail!("provider-sync transaction path is outside rollout roots");
    }
    Ok(path)
}

fn session_transaction_rollout_roots(
    home: &Path,
) -> anyhow::Result<HashMap<String, SessionTransactionRootEvidence>> {
    let canonical_home = fs::canonicalize(home)?;
    let mut roots = HashMap::new();
    for dirname in SESSION_DIRS {
        let root = home.join(dirname);
        if !root.exists() {
            continue;
        }
        let canonical_root = fs::canonicalize(&root)?;
        if !canonical_root.starts_with(&canonical_home) {
            anyhow::bail!("provider-sync rollout root resolves outside Codex home");
        }
        roots.insert(
            dirname.to_string(),
            SessionTransactionRootEvidence {
                canonical_path: canonical_root.to_string_lossy().to_string(),
                identity: rollout_root_identity(&root)?,
            },
        );
    }
    Ok(roots)
}

fn rollout_root_identity(path: &Path) -> anyhow::Result<String> {
    ensure_not_reparse_or_symlink(path)?;
    codex_plus_core::settings::directory_instance_identity(path)
}

fn ensure_not_reparse_or_symlink(path: &Path) -> anyhow::Result<()> {
    let metadata = fs::symlink_metadata(path)?;

    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;

        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            anyhow::bail!(
                "provider-sync path cannot be a reparse point: {}",
                path.display()
            );
        }
    }
    #[cfg(not(windows))]
    if metadata.file_type().is_symlink() {
        anyhow::bail!("provider-sync path cannot be a symlink: {}", path.display());
    }
    Ok(())
}

fn ensure_path_components_not_reparse(root: &Path, target: &Path) -> anyhow::Result<()> {
    let relative = target
        .strip_prefix(root)
        .map_err(|_| anyhow::anyhow!("provider-sync path is outside its validated root"))?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let std::path::Component::Normal(name) = component else {
            anyhow::bail!("provider-sync path contains an invalid component");
        };
        current.push(name);
        ensure_not_reparse_or_symlink(&current)?;
    }
    Ok(())
}

fn validated_rollout_transaction_path(
    home: &Path,
    value: &str,
    rollout_roots: &HashMap<String, SessionTransactionRootEvidence>,
) -> anyhow::Result<PathBuf> {
    let relative = validated_rollout_relative_path(value)?;
    let root_name = relative
        .components()
        .next()
        .and_then(|component| match component {
            std::path::Component::Normal(name) => Some(name.to_os_string()),
            _ => None,
        })
        .ok_or_else(|| anyhow::anyhow!("provider-sync rollout path has no root"))?;
    let root_name_text = root_name.to_string_lossy();
    let expected_root = rollout_roots
        .get(root_name_text.as_ref())
        .ok_or_else(|| anyhow::anyhow!("provider-sync rollout root was not recorded"))?;
    let path = home.join(&relative);
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("provider-sync rollout path has no parent"))?;
    let canonical_home = fs::canonicalize(home)?;
    let root = home.join(&root_name);
    let canonical_root = fs::canonicalize(&root)?;
    if !canonical_root.starts_with(&canonical_home) {
        anyhow::bail!("provider-sync rollout root resolves outside Codex home");
    }
    if canonical_root != PathBuf::from(&expected_root.canonical_path)
        || rollout_root_identity(&root)? != expected_root.identity
    {
        anyhow::bail!("provider-sync rollout root identity changed after backup");
    }
    let canonical_parent = fs::canonicalize(parent)?;
    if !canonical_parent.starts_with(&canonical_root) {
        anyhow::bail!("provider-sync rollout path resolves outside rollout roots");
    }
    ensure_path_components_not_reparse(&root, parent)?;
    if path.exists() && !fs::canonicalize(&path)?.starts_with(&canonical_root) {
        anyhow::bail!("provider-sync rollout path resolves outside Codex home");
    }
    if path.exists() {
        ensure_path_components_not_reparse(&root, &path)?;
    }
    Ok(path)
}

fn cleanup_transaction_staged_files(home: &Path, transaction_id: &str) -> std::io::Result<()> {
    for dirname in SESSION_DIRS {
        cleanup_transaction_staged_files_in(&home.join(dirname), transaction_id)?;
    }
    Ok(())
}

fn cleanup_transaction_staged_files_in(root: &Path, transaction_id: &str) -> std::io::Result<()> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            cleanup_transaction_staged_files_in(&path, transaction_id)?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        let should_remove = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| provider_sync_temp_name_belongs(name, transaction_id));
        if should_remove {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

fn provider_sync_temp_name_belongs(name: &str, transaction_id: &str) -> bool {
    let Some(body) = name
        .strip_prefix('.')
        .and_then(|name| name.strip_suffix(".tmp"))
    else {
        return false;
    };
    for kind in ["", "displaced-", "rejected-"] {
        let marker = format!(".provider-sync-{kind}{transaction_id}-");
        if let Some((rollout_name, index)) = body.rsplit_once(&marker) {
            return !rollout_name.is_empty()
                && !index.is_empty()
                && index.bytes().all(|byte| byte.is_ascii_digit());
        }
    }
    let marker = format!(".provider-sync-restore-{transaction_id}");
    body.strip_suffix(&marker)
        .is_some_and(|rollout_name| !rollout_name.is_empty())
}

fn file_sha256_and_size(path: &Path) -> std::io::Result<(String, u64)> {
    let mut file = File::open(path)?;
    let mut buffer = [0_u8; 64 * 1024];
    let mut hasher = Sha256::new();
    let mut size = 0_u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        size += read as u64;
    }
    Ok((format!("{:x}", hasher.finalize()), size))
}

fn system_time_parts(time: Option<SystemTime>) -> (Option<u64>, Option<u32>) {
    let Some(time) = time else {
        return (None, None);
    };
    let Ok(duration) = time.duration_since(UNIX_EPOCH) else {
        return (None, None);
    };
    (Some(duration.as_secs()), Some(duration.subsec_nanos()))
}

fn restore_file_mtime_parts(path: &Path, secs: Option<u64>, nanos: Option<u32>) {
    let Some(secs) = secs else { return };
    let time = UNIX_EPOCH + std::time::Duration::new(secs, nanos.unwrap_or_default());
    restore_file_mtime(path, Some(time));
}

fn is_locked_io_error_from_anyhow(error: &anyhow::Error) -> bool {
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<std::io::Error>())
        .is_some_and(is_locked_io_error)
}

fn open_session_file_for_update(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0);
    }
    options.open(path)
}

fn restore_file_mtime(path: &Path, mtime: Option<SystemTime>) {
    let Some(mtime) = mtime else { return };
    let Ok(file) = fs::File::options().write(true).open(path) else {
        return;
    };
    let times = std::fs::FileTimes::new().set_modified(mtime);
    let _ = file.set_times(times);
}

fn table_columns(db: &Connection, table: &str) -> anyhow::Result<HashSet<String>> {
    let mut stmt = db.prepare(&format!(
        "PRAGMA table_info(\"{}\")",
        table.replace('"', "\"\"")
    ))?;
    Ok(stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<HashSet<_>>>()?)
}

fn sqlite_provider_ids(path: &Path) -> anyhow::Result<Vec<String>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let db = Connection::open(path)?;
    let mut ids = HashSet::new();
    for table in ["threads", "local_thread_catalog"] {
        let columns = table_columns(&db, table)?;
        if !columns.contains("model_provider") {
            continue;
        }
        let subagent_filter = if table == "threads" {
            subagent_filter(&db, "threads.id")?
        } else if columns.contains("thread_id") {
            subagent_filter(&db, "local_thread_catalog.thread_id")?
        } else {
            String::new()
        };
        let mut stmt = db.prepare(&format!(
            "SELECT DISTINCT COALESCE(model_provider, '') FROM {table} WHERE COALESCE(model_provider, '') <> ''{subagent_filter}"
        ))?;
        for item in stmt.query_map([], |row| row.get::<_, String>(0))? {
            let id = item?;
            if is_valid_provider_id_for_discovery(&id) {
                ids.insert(id);
            }
        }
    }
    Ok(sorted_provider_ids(ids))
}

fn sqlite_provider_sync_thread_kinds(paths: &[PathBuf]) -> anyhow::Result<ProviderSyncThreadKinds> {
    let mut kinds = ProviderSyncThreadKinds::default();
    for path in paths {
        if !path.exists() {
            continue;
        }
        let db = Connection::open(path)?;
        for (table, column) in [
            ("thread_spawn_edges", "child_thread_id"),
            ("agent_job_items", "assigned_thread_id"),
        ] {
            if !table_columns(&db, table)?.contains(column) {
                continue;
            }
            let sql =
                format!("SELECT DISTINCT {column} FROM {table} WHERE COALESCE({column}, '') <> ''");
            kinds.subagent_thread_ids.extend(
                db.prepare(&sql)?
                    .query_map([], |row| row.get::<_, String>(0))?
                    .collect::<rusqlite::Result<HashSet<_>>>()?,
            );
        }

        for (table, id_column, source_column) in [
            ("threads", "id", "source"),
            ("local_thread_catalog", "thread_id", "source_kind"),
        ] {
            let columns = table_columns(&db, table)?;
            if !columns.contains(id_column) {
                continue;
            }
            let source = text_expr(&columns, source_column, "''");
            let thread_source = text_expr(&columns, "thread_source", "NULL");
            let sql = format!(
                "SELECT {id_column}, {source}, {thread_source} FROM {table} WHERE COALESCE({id_column}, '') <> ''"
            );
            let mut stmt = db.prepare(&sql)?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1).unwrap_or_default(),
                    row.get::<_, Option<String>>(2).unwrap_or(None),
                ))
            })?;
            for row in rows {
                let (thread_id, source, thread_source) = row?;
                if source_structured_marks_non_root_agent(&source)
                    || thread_source_marks_non_root(thread_source.as_deref())
                {
                    kinds.subagent_thread_ids.insert(thread_id);
                } else if thread_source_is_user(thread_source.as_deref()) {
                    kinds.explicit_user_thread_ids.insert(thread_id);
                } else if source_marks_non_root_agent(&source) {
                    kinds.subagent_thread_ids.insert(thread_id);
                }
            }
        }
    }
    kinds
        .subagent_thread_ids
        .retain(|thread_id| !kinds.explicit_user_thread_ids.contains(thread_id));
    Ok(kinds)
}

fn subagent_filter(db: &Connection, id_expr: &str) -> anyhow::Result<String> {
    let mut filters = Vec::new();
    if table_columns(db, "thread_spawn_edges")?
        .iter()
        .any(|column| column == "child_thread_id")
    {
        filters.push(format!(
            "NOT EXISTS (SELECT 1 FROM thread_spawn_edges e WHERE e.child_thread_id = {id_expr})"
        ));
    }
    if table_columns(db, "agent_job_items")?
        .iter()
        .any(|column| column == "assigned_thread_id")
    {
        filters.push(format!(
            "NOT EXISTS (SELECT 1 FROM agent_job_items j WHERE j.assigned_thread_id = {id_expr})"
        ));
    }
    if filters.is_empty() {
        Ok(String::new())
    } else {
        Ok(format!(" AND {}", filters.join(" AND ")))
    }
}

fn remote_control_catalog_recovery_thread_ids(
    paths: &[PathBuf],
    target_provider: &str,
    requested_thread_ids: &HashSet<String>,
) -> anyhow::Result<HashSet<String>> {
    let mut known_thread_ids = HashSet::new();
    let mut ready_thread_ids = HashSet::new();
    let mut has_local_catalog = false;
    for path in paths {
        if !path.exists() {
            continue;
        }
        let db = Connection::open(path)?;
        let thread_columns = table_columns(&db, "threads")?;
        if thread_columns.contains("id") {
            let mut stmt = db.prepare("SELECT id FROM threads WHERE COALESCE(id, '') <> ''")?;
            for item in stmt.query_map([], |row| row.get::<_, String>(0))? {
                let thread_id = item?;
                if requested_thread_ids.contains(&thread_id) {
                    known_thread_ids.insert(thread_id);
                }
            }
        }

        let catalog_columns = table_columns(&db, "local_thread_catalog")?;
        if !catalog_columns.contains("thread_id") {
            continue;
        }
        let Some(host_id) = local_catalog_host_id(&db)? else {
            continue;
        };
        has_local_catalog = true;
        let provider_expr = if catalog_columns.contains("model_provider") {
            "COALESCE(model_provider, '')"
        } else {
            "''"
        };
        let missing_expr = if catalog_columns.contains("missing_candidate") {
            "COALESCE(missing_candidate, 0)"
        } else {
            "0"
        };
        let host_filter = if catalog_columns.contains("host_id") {
            " AND host_id = ?1"
        } else {
            " AND ?1 = ?1"
        };
        let sql = format!(
            "SELECT thread_id, {provider_expr}, {missing_expr} FROM local_thread_catalog WHERE COALESCE(thread_id, '') <> ''{host_filter}"
        );
        let mut stmt = db.prepare(&sql)?;
        for item in stmt.query_map([host_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })? {
            let (thread_id, provider, missing_candidate) = item?;
            if requested_thread_ids.contains(&thread_id)
                && provider == target_provider
                && missing_candidate == 0
            {
                ready_thread_ids.insert(thread_id);
            }
        }
    }
    if !has_local_catalog {
        return Ok(HashSet::new());
    }
    known_thread_ids.retain(|thread_id| !ready_thread_ids.contains(thread_id));
    Ok(known_thread_ids)
}

fn provider_update_thread_ids(
    db: &Connection,
    table: &str,
    id_column: &str,
    target_provider: &str,
    excluded_thread_ids: &HashSet<String>,
) -> anyhow::Result<Vec<String>> {
    let sql = format!(
        "SELECT {id_column} FROM {table} WHERE COALESCE({id_column}, '') <> '' AND COALESCE(model_provider, '') <> ?1"
    );
    let mut stmt = db.prepare(&sql)?;
    let mut thread_ids = Vec::new();
    for item in stmt.query_map([target_provider], |row| row.get::<_, String>(0))? {
        let thread_id = item?;
        if !excluded_thread_ids.contains(&thread_id) {
            thread_ids.push(thread_id);
        }
    }
    Ok(thread_ids)
}

fn count_sqlite_updates(
    path: &Path,
    target_provider: &str,
    user_event_thread_ids: &HashSet<String>,
    cwd_by_thread_id: &HashMap<String, String>,
    excluded_thread_ids: &HashSet<String>,
) -> anyhow::Result<usize> {
    if !path.exists() {
        return Ok(0);
    }
    let db = Connection::open(path)?;
    let columns = table_columns(&db, "threads")?;
    let catalog_columns = table_columns(&db, "local_thread_catalog")?;
    let mut total = 0;
    if columns.contains("id") && columns.contains("model_provider") {
        total +=
            provider_update_thread_ids(&db, "threads", "id", target_provider, excluded_thread_ids)?
                .len();
    }
    if catalog_columns.contains("thread_id") && catalog_columns.contains("model_provider") {
        total += provider_update_thread_ids(
            &db,
            "local_thread_catalog",
            "thread_id",
            target_provider,
            excluded_thread_ids,
        )?
        .len();
    }
    if columns.contains("has_user_event") {
        for thread_id in user_event_thread_ids {
            if excluded_thread_ids.contains(thread_id) {
                continue;
            }
            total += db.query_row(
                "SELECT COUNT(*) FROM threads WHERE id = ?1 AND COALESCE(has_user_event, 0) <> 1",
                [thread_id],
                |row| row.get::<_, i64>(0),
            )? as usize;
        }
    }
    if columns.contains("cwd") {
        for (thread_id, cwd) in cwd_by_thread_id {
            if excluded_thread_ids.contains(thread_id) {
                continue;
            }
            total += db.query_row(
                "SELECT COUNT(*) FROM threads WHERE id = ?1 AND COALESCE(cwd, '') <> ?2",
                (thread_id, cwd),
                |row| row.get::<_, i64>(0),
            )? as usize;
        }
    }
    Ok(total)
}

fn count_sqlite_updates_for_paths(
    paths: &[PathBuf],
    target_provider: &str,
    user_event_thread_ids: &HashSet<String>,
    cwd_by_thread_id: &HashMap<String, String>,
    excluded_thread_ids: &HashSet<String>,
) -> anyhow::Result<usize> {
    let mut total = 0;
    for path in paths {
        total += count_sqlite_updates(
            path,
            target_provider,
            user_event_thread_ids,
            cwd_by_thread_id,
            excluded_thread_ids,
        )?;
    }
    Ok(total)
}

fn apply_sqlite_update(
    path: &Path,
    target_provider: &str,
    user_event_thread_ids: &HashSet<String>,
    cwd_by_thread_id: &HashMap<String, String>,
    excluded_thread_ids: &HashSet<String>,
) -> anyhow::Result<SqliteUpdateCounts> {
    if !path.exists() {
        return Ok(SqliteUpdateCounts::default());
    }
    let mut db = Connection::open(path)?;
    let columns = table_columns(&db, "threads")?;
    let catalog_columns = table_columns(&db, "local_thread_catalog")?;
    if !columns.contains("model_provider") && !catalog_columns.contains("model_provider") {
        return Ok(SqliteUpdateCounts::default());
    }
    let tx = db.transaction()?;
    let mut counts = SqliteUpdateCounts::default();
    if columns.contains("id") && columns.contains("model_provider") {
        for thread_id in
            provider_update_thread_ids(&tx, "threads", "id", target_provider, excluded_thread_ids)?
        {
            counts.provider_rows += tx.execute(
                "UPDATE threads SET model_provider = ?1 WHERE id = ?2 AND COALESCE(model_provider, '') <> ?1",
                (target_provider, thread_id),
            )?;
        }
    }
    if catalog_columns.contains("thread_id") && catalog_columns.contains("model_provider") {
        for thread_id in provider_update_thread_ids(
            &tx,
            "local_thread_catalog",
            "thread_id",
            target_provider,
            excluded_thread_ids,
        )? {
            counts.provider_rows += tx.execute(
                "UPDATE local_thread_catalog SET model_provider = ?1 WHERE thread_id = ?2 AND COALESCE(model_provider, '') <> ?1",
                (target_provider, thread_id),
            )?;
        }
    }
    if columns.contains("has_user_event") {
        for thread_id in user_event_thread_ids {
            if excluded_thread_ids.contains(thread_id) {
                continue;
            }
            counts.user_event_rows += tx.execute(
                "UPDATE threads SET has_user_event = 1 WHERE id = ?1 AND COALESCE(has_user_event, 0) <> 1",
                [thread_id],
            )?;
        }
    }
    if columns.contains("cwd") {
        for (thread_id, cwd) in cwd_by_thread_id {
            if excluded_thread_ids.contains(thread_id) {
                continue;
            }
            counts.cwd_rows += tx.execute(
                "UPDATE threads SET cwd = ?1 WHERE id = ?2 AND COALESCE(cwd, '') <> ?1",
                (cwd, thread_id),
            )?;
        }
    }
    tx.commit()?;
    Ok(counts)
}

fn apply_sqlite_update_for_paths(
    paths: &[PathBuf],
    target_provider: &str,
    user_event_thread_ids: &HashSet<String>,
    cwd_by_thread_id: &HashMap<String, String>,
    excluded_thread_ids: &HashSet<String>,
) -> anyhow::Result<SqliteUpdateCounts> {
    let mut total = SqliteUpdateCounts::default();
    for path in paths {
        total.add(apply_sqlite_update(
            path,
            target_provider,
            user_event_thread_ids,
            cwd_by_thread_id,
            excluded_thread_ids,
        )?);
    }
    Ok(total)
}

fn apply_remote_control_recovery_sqlite_updates(
    paths: &[PathBuf],
    target_provider: &str,
    thread_ids: &HashSet<String>,
) -> anyhow::Result<SqliteUpdateCounts> {
    let mut counts = SqliteUpdateCounts::default();
    for path in paths {
        if !path.exists() {
            continue;
        }
        let mut db = Connection::open(path)?;
        let thread_columns = table_columns(&db, "threads")?;
        let catalog_columns = table_columns(&db, "local_thread_catalog")?;
        let local_host_id = if catalog_columns.contains("thread_id") {
            local_catalog_host_id(&db)?
        } else {
            None
        };
        let tx = db.transaction()?;
        if thread_columns.contains("id") && thread_columns.contains("model_provider") {
            for thread_id in thread_ids {
                counts.provider_rows += tx.execute(
                    "UPDATE threads SET model_provider = ?1 WHERE id = ?2 AND model_provider = ?3",
                    (target_provider, thread_id, DEFAULT_PROVIDER),
                )?;
            }
        }
        if catalog_columns.contains("thread_id")
            && catalog_columns.contains("model_provider")
            && local_host_id.is_some()
        {
            let host_id = local_host_id.as_deref().unwrap_or("local");
            let host_filter = if catalog_columns.contains("host_id") {
                " AND host_id = ?3"
            } else {
                " AND ?3 = ?3"
            };
            for thread_id in thread_ids {
                let sql = format!(
                    "UPDATE local_thread_catalog SET model_provider = ?1 WHERE thread_id = ?2{host_filter} AND model_provider = ?4"
                );
                counts.provider_rows += tx.execute(
                    &sql,
                    (target_provider, thread_id, host_id, DEFAULT_PROVIDER),
                )?;
                if catalog_columns.contains("missing_candidate") {
                    let sql = format!(
                        "UPDATE local_thread_catalog SET missing_candidate = 0 WHERE thread_id = ?1{} AND COALESCE(missing_candidate, 0) <> 0",
                        if catalog_columns.contains("host_id") {
                            " AND host_id = ?2"
                        } else {
                            " AND ?2 = ?2"
                        }
                    );
                    tx.execute(&sql, (thread_id, host_id))?;
                }
            }
        }
        tx.commit()?;
    }
    Ok(counts)
}

fn apply_remote_control_catalog_updates(
    paths: &[PathBuf],
    target_provider: &str,
    thread_ids: &HashSet<String>,
) -> anyhow::Result<usize> {
    let mut total = 0;
    for path in paths {
        if !path.exists() {
            continue;
        }
        let mut db = Connection::open(path)?;
        let columns = table_columns(&db, "local_thread_catalog")?;
        if !columns.contains("thread_id") || !columns.contains("model_provider") {
            continue;
        }
        let Some(host_id) = local_catalog_host_id(&db)? else {
            continue;
        };
        let host_filter = if columns.contains("host_id") {
            " AND host_id = ?3"
        } else {
            " AND ?3 = ?3"
        };
        let tx = db.transaction()?;
        for thread_id in thread_ids {
            let sql = format!(
                "UPDATE local_thread_catalog SET model_provider = ?1{} WHERE thread_id = ?2{} AND COALESCE(model_provider, '') <> ?1",
                if columns.contains("missing_candidate") {
                    ", missing_candidate = 0"
                } else {
                    ""
                },
                host_filter
            );
            total += tx.execute(&sql, (target_provider, thread_id, &host_id))?;
            if columns.contains("missing_candidate") {
                let sql = format!(
                    "UPDATE local_thread_catalog SET missing_candidate = 0 WHERE thread_id = ?1{} AND COALESCE(missing_candidate, 0) <> 0",
                    if columns.contains("host_id") {
                        " AND host_id = ?2"
                    } else {
                        " AND ?2 = ?2"
                    }
                );
                tx.execute(&sql, (thread_id, &host_id))?;
            }
        }
        tx.commit()?;
    }
    Ok(total)
}

fn count_local_thread_catalog_repairs(
    home: &Path,
    paths: &[PathBuf],
    target_provider: &str,
) -> anyhow::Result<usize> {
    let plan = collect_catalog_repair_plan(home, paths, target_provider, None)?;
    if plan.threads.is_empty() && !plan.has_cleanup_candidates() {
        return Ok(0);
    }
    let mut total = 0;
    for path in paths {
        if !path.exists() {
            continue;
        }
        let db = Connection::open(path)?;
        let columns = table_columns(&db, "local_thread_catalog")?;
        if !catalog_supports_repair(&columns) {
            continue;
        }
        let Some(host_id) = local_catalog_host_id(&db)? else {
            continue;
        };
        for thread in plan.threads.values() {
            if !local_catalog_contains_thread(&db, &host_id, &thread.id)? {
                total += 1;
            }
        }
        for thread_id in plan.cleanup_thread_ids_for_path(path) {
            if local_catalog_contains_thread(&db, &host_id, &thread_id)? {
                total += 1;
            }
        }
    }
    Ok(total)
}

fn repair_missing_local_thread_catalog_rows(
    home: &Path,
    paths: &[PathBuf],
    target_provider: &str,
) -> anyhow::Result<CatalogRepairCounts> {
    repair_missing_local_thread_catalog_rows_filtered(home, paths, target_provider, None, true)
}

fn repair_missing_local_thread_catalog_rows_for_threads(
    home: &Path,
    paths: &[PathBuf],
    target_provider: &str,
    thread_ids: &HashSet<String>,
) -> anyhow::Result<CatalogRepairCounts> {
    repair_missing_local_thread_catalog_rows_filtered(
        home,
        paths,
        target_provider,
        Some(thread_ids),
        false,
    )
}

fn repair_missing_local_thread_catalog_rows_filtered(
    home: &Path,
    paths: &[PathBuf],
    target_provider: &str,
    thread_ids: Option<&HashSet<String>>,
    update_full_sync_state: bool,
) -> anyhow::Result<CatalogRepairCounts> {
    let plan = collect_catalog_repair_plan(home, paths, target_provider, thread_ids)?;
    if plan.threads.is_empty() && (!update_full_sync_state || !plan.has_cleanup_candidates()) {
        return Ok(CatalogRepairCounts::default());
    }
    let mut total = CatalogRepairCounts::default();
    for path in paths {
        if !path.exists() {
            continue;
        }
        let mut db = Connection::open(path)?;
        let columns = table_columns(&db, "local_thread_catalog")?;
        if !catalog_supports_repair(&columns) {
            continue;
        }
        let sync_columns = table_columns(&db, "local_thread_catalog_sync_state")?;
        let metadata_columns = table_columns(&db, "local_thread_catalog_metadata")?;
        let Some(host_id) = local_catalog_host_id(&db)? else {
            continue;
        };
        let mut observation_sequence = local_catalog_max_observation_sequence(&db, &host_id)?;
        let insert_columns = local_catalog_insert_columns(&columns);
        let placeholders = std::iter::repeat_n("?", insert_columns.len())
            .collect::<Vec<_>>()
            .join(", ");
        let insert_sql = format!(
            "INSERT OR IGNORE INTO local_thread_catalog ({}) VALUES ({})",
            insert_columns.join(", "),
            placeholders
        );
        let tx = db.transaction()?;
        let mut removed = 0;
        if update_full_sync_state {
            let cleanup_thread_ids = plan.cleanup_thread_ids_for_path(path);
            let mut non_root_thread_ids = cleanup_thread_ids.iter().collect::<Vec<_>>();
            non_root_thread_ids.sort();
            let mut delete = tx.prepare(
                "DELETE FROM local_thread_catalog WHERE host_id = ?1 AND thread_id = ?2",
            )?;
            for thread_id in non_root_thread_ids {
                removed += delete.execute((&host_id, thread_id))?;
            }
            drop(delete);
        }
        let mut inserted = 0;
        let mut max_source_updated_at = 0.0_f64;
        let mut threads = plan.threads.values().collect::<Vec<_>>();
        threads.sort_by(|left, right| left.id.cmp(&right.id));
        for thread in threads {
            let next_observation_sequence = observation_sequence + 1;
            let values = local_catalog_insert_values(
                &insert_columns,
                &host_id,
                thread,
                next_observation_sequence,
            );
            let affected = tx.execute(&insert_sql, params_from_iter(values))?;
            if affected > 0 {
                observation_sequence = next_observation_sequence;
                inserted += affected;
                max_source_updated_at = max_source_updated_at.max(thread.source_updated_at);
            }
        }
        let changed = inserted + removed;
        if changed > 0 {
            update_local_catalog_metadata(&tx, &metadata_columns, changed)?;
            if update_full_sync_state {
                update_local_catalog_sync_state(
                    &tx,
                    &sync_columns,
                    &host_id,
                    observation_sequence,
                    max_source_updated_at,
                )?;
            }
        }
        tx.commit()?;
        total.add(CatalogRepairCounts {
            inserted_rows: inserted,
            removed_rows: removed,
        });
    }
    Ok(total)
}

fn collect_catalog_repair_plan(
    home: &Path,
    paths: &[PathBuf],
    target_provider: &str,
    thread_ids: Option<&HashSet<String>>,
) -> anyhow::Result<CatalogRepairPlan> {
    let spawned_child_ids = collect_spawned_child_thread_ids(paths)?;
    let mut catalog_non_root_thread_ids =
        collect_catalog_marked_non_root_thread_ids(paths, &spawned_child_ids)?;
    let mut observed_threads = HashMap::new();
    for path in paths {
        if !path.exists() {
            continue;
        }
        let db = Connection::open(path)?;
        let columns = table_columns(&db, "threads")?;
        if !columns.contains("id") {
            continue;
        }
        let display_title = coalesce_text_expr(
            &columns,
            &["name", "title", "preview", "first_user_message"],
            "id",
        );
        let source_created_at = timestamp_expr(&columns, "created_at_ms", "created_at");
        let source_updated_at = timestamp_expr(&columns, "updated_at_ms", "updated_at");
        let cwd = text_expr(&columns, "cwd", "''");
        let source_kind = coalesce_text_expr(&columns, &["source"], "'cli'");
        let source_detail = text_expr(&columns, "rollout_path", "''");
        let git_branch = text_expr(&columns, "git_branch", "NULL");
        let thread_source = text_expr(&columns, "thread_source", "NULL");
        let archived = text_expr(&columns, "archived", "0");
        let has_user_event = text_expr(&columns, "has_user_event", "1");
        let agent_role = text_expr(&columns, "agent_role", "''");
        let subagent_filter = subagent_filter(&db, "threads.id")?;
        let sql = format!(
            "SELECT id, {display_title}, {source_created_at}, {source_updated_at}, {cwd}, {source_kind}, {source_detail}, {git_branch}, {thread_source}, {archived}, {has_user_event}, {agent_role} FROM threads WHERE COALESCE(id, '') <> ''{subagent_filter}"
        );
        let mut stmt = db.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            Ok((
                CatalogRepairThread {
                    id: row.get(0)?,
                    display_title: row.get::<_, String>(1).unwrap_or_default(),
                    source_created_at: row.get::<_, f64>(2).unwrap_or_default(),
                    source_updated_at: row.get::<_, f64>(3).unwrap_or_default(),
                    cwd: row.get::<_, String>(4).unwrap_or_default(),
                    source_kind: row
                        .get::<_, String>(5)
                        .unwrap_or_else(|_| "cli".to_string()),
                    source_detail: row.get::<_, String>(6).unwrap_or_default(),
                    model_provider: target_provider.to_string(),
                    git_branch: row.get::<_, Option<String>>(7).unwrap_or(None),
                    thread_source: row.get::<_, Option<String>>(8).unwrap_or(None),
                },
                row.get::<_, i64>(9).unwrap_or_default(),
                row.get::<_, i64>(10).unwrap_or(1),
                row.get::<_, String>(11).unwrap_or_default(),
            ))
        })?;
        for item in rows {
            let (thread, archived, has_user_event, agent_role) = item?;
            let marked_non_user = columns.contains("thread_source")
                && thread.thread_source.as_deref().is_some_and(|value| {
                    let value = value.trim();
                    !value.is_empty() && !value.eq_ignore_ascii_case("user")
                });
            let non_root = is_catalog_non_root_agent(&thread, &spawned_child_ids);
            let source_is_exec = thread.source_kind.trim().eq_ignore_ascii_case("exec");
            let rollout_exists = catalog_rollout_path_exists(home, &thread.source_detail);
            let eligible = archived == 0
                && has_user_event == 1
                && agent_role.trim().is_empty()
                && !marked_non_user
                && !source_is_exec
                && !non_root
                && rollout_exists;
            let replace = observed_threads
                .get(&thread.id)
                .map(|current: &CatalogRepairObservedThread| {
                    // Copies can share a timestamp; an ineligible observation wins the tie so
                    // an archived or agent-owned thread cannot be resurrected by a stale copy.
                    thread.source_updated_at > current.thread.source_updated_at
                        || (thread.source_updated_at == current.thread.source_updated_at
                            && !eligible
                            && current.eligible)
                })
                .unwrap_or(true);
            if replace {
                observed_threads.insert(
                    thread.id.clone(),
                    CatalogRepairObservedThread { thread, eligible },
                );
            }
        }
    }
    if let Some(thread_ids) = thread_ids {
        observed_threads.retain(|thread_id, _| thread_ids.contains(thread_id));
    }
    let explicit_user_thread_ids = observed_threads
        .values()
        .filter(|observed| thread_source_is_user(observed.thread.thread_source.as_deref()))
        .map(|observed| observed.thread.id.clone())
        .collect::<HashSet<_>>();
    let non_root_thread_ids = observed_threads
        .values()
        .filter(|observed| is_catalog_non_root_agent(&observed.thread, &spawned_child_ids))
        .map(|observed| observed.thread.id.clone())
        .collect::<HashSet<_>>();
    let ineligible_thread_ids = observed_threads
        .values()
        .filter(|observed| !observed.eligible)
        .map(|observed| observed.thread.id.clone())
        .collect::<HashSet<_>>();
    let threads = observed_threads
        .into_iter()
        .filter_map(|(thread_id, observed)| {
            observed.eligible.then_some((thread_id, observed.thread))
        })
        .collect::<HashMap<_, _>>();
    // Catalog-only evidence stays path-scoped so one stale database cannot remove another's row.
    for catalog_thread_ids in catalog_non_root_thread_ids.values_mut() {
        catalog_thread_ids.retain(|thread_id| {
            thread_ids
                .map(|requested| requested.contains(thread_id))
                .unwrap_or(true)
                && !explicit_user_thread_ids.contains(thread_id)
        });
    }
    catalog_non_root_thread_ids.retain(|_, thread_ids| !thread_ids.is_empty());
    Ok(CatalogRepairPlan {
        threads,
        non_root_thread_ids,
        ineligible_thread_ids,
        catalog_non_root_thread_ids,
    })
}

fn catalog_rollout_path_exists(home: &Path, rollout_path: &str) -> bool {
    let rollout_path = rollout_path.trim();
    if rollout_path.is_empty() {
        return true;
    }
    let rollout_path = Path::new(rollout_path);
    if rollout_path.is_absolute() {
        rollout_path.is_file()
    } else {
        home.join(rollout_path).is_file()
    }
}

fn collect_spawned_child_thread_ids(paths: &[PathBuf]) -> anyhow::Result<HashSet<String>> {
    let mut thread_ids = HashSet::new();
    for path in paths {
        if !path.exists() {
            continue;
        }
        let db = Connection::open(path)?;
        let columns = table_columns(&db, "thread_spawn_edges")?;
        if !columns.contains("child_thread_id") {
            continue;
        }
        let mut stmt = db.prepare(
            "SELECT child_thread_id FROM thread_spawn_edges WHERE COALESCE(child_thread_id, '') <> ''",
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        for thread_id in rows {
            thread_ids.insert(thread_id?);
        }
    }
    Ok(thread_ids)
}

fn collect_catalog_marked_non_root_thread_ids(
    paths: &[PathBuf],
    spawned_child_ids: &HashSet<String>,
) -> anyhow::Result<HashMap<PathBuf, HashSet<String>>> {
    let mut thread_ids_by_path: HashMap<PathBuf, HashSet<String>> = HashMap::new();
    for path in paths {
        if !path.exists() {
            continue;
        }
        let db = Connection::open(path)?;
        let columns = table_columns(&db, "local_thread_catalog")?;
        if !columns.contains("host_id") || !columns.contains("thread_id") {
            continue;
        }
        let Some(host_id) = local_catalog_host_id(&db)? else {
            continue;
        };
        let source_kind = text_expr(&columns, "source_kind", "''");
        let thread_source = text_expr(&columns, "thread_source", "NULL");
        let sql = format!(
            "SELECT thread_id, {source_kind}, {thread_source} FROM local_thread_catalog WHERE host_id = ?1 AND COALESCE(thread_id, '') <> ''"
        );
        let mut stmt = db.prepare(&sql)?;
        let rows = stmt.query_map([host_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1).unwrap_or_default(),
                row.get::<_, Option<String>>(2).unwrap_or(None),
            ))
        })?;
        for row in rows {
            let (thread_id, source_kind, thread_source) = row?;
            if source_structured_marks_non_root_agent(&source_kind)
                || thread_source_marks_non_root(thread_source.as_deref())
            {
                thread_ids_by_path
                    .entry(path.clone())
                    .or_default()
                    .insert(thread_id);
                continue;
            }
            if thread_source_is_user(thread_source.as_deref()) {
                continue;
            }
            if source_marks_non_root_agent(&source_kind) || spawned_child_ids.contains(&thread_id) {
                thread_ids_by_path
                    .entry(path.clone())
                    .or_default()
                    .insert(thread_id);
            }
        }
    }
    Ok(thread_ids_by_path)
}

fn is_catalog_non_root_agent(
    thread: &CatalogRepairThread,
    spawned_child_ids: &HashSet<String>,
) -> bool {
    if source_structured_marks_non_root_agent(&thread.source_kind)
        || thread_source_marks_non_root(thread.thread_source.as_deref())
    {
        return true;
    }
    // The explicit user marker is authoritative over legacy text and spawn-edge fallbacks.
    if thread_source_is_user(thread.thread_source.as_deref()) {
        return false;
    }
    source_marks_non_root_agent(&thread.source_kind) || spawned_child_ids.contains(&thread.id)
}

fn thread_source_is_user(thread_source: Option<&str>) -> bool {
    thread_source
        .map(str::trim)
        .is_some_and(|value| value.eq_ignore_ascii_case("user"))
}

fn thread_source_marks_non_root(thread_source: Option<&str>) -> bool {
    thread_source.map(str::trim).is_some_and(|value| {
        value.eq_ignore_ascii_case("subagent") || value.eq_ignore_ascii_case("memory_consolidation")
    })
}

fn source_marks_non_root_agent(source: &str) -> bool {
    let source = source.trim();
    if source_text_marks_non_root_agent(source) {
        return true;
    }
    source_structured_marks_non_root_agent(source)
}

fn source_structured_marks_non_root_agent(source: &str) -> bool {
    serde_json::from_str::<Value>(source.trim())
        .is_ok_and(|source| source_value_marks_non_root_agent(&source))
}

fn source_value_marks_non_root_agent(source: &Value) -> bool {
    match source {
        // 只看 key 在不在会把 `{"internal": false}`、`{"sub_agent": null}` 这种
        // 明确表示「不是子代理」的记录判成子代理，而这个判定的下游是 DELETE，
        // 误判等于真实会话被删。所以要求 value 本身也表示「是」。
        Value::Object(object) => ["sub_agent", "subagent", "internal"]
            .iter()
            .any(|key| object.get(*key).is_some_and(value_asserts_non_root_agent)),
        Value::String(value) => source_text_marks_non_root_agent(value),
        _ => false,
    }
}

/// 判断标记字段的取值是否真的在声明「这是子代理线程」。
/// 空对象/空数组同样按「没声明」处理，避免占位字段引发误删。
fn value_asserts_non_root_agent(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Object(object) => !object.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::String(text) => !text.trim().is_empty(),
        Value::Number(_) => true,
    }
}

fn source_text_marks_non_root_agent(source: &str) -> bool {
    let source = source.trim().to_ascii_lowercase();
    source == "subagent"
        || source == "internal"
        || source.starts_with("subagent_")
        || source.starts_with("internal_")
}

fn catalog_supports_repair(columns: &HashSet<String>) -> bool {
    [
        "host_id",
        "thread_id",
        "display_title",
        "source_created_at",
        "source_updated_at",
        "cwd",
        "source_kind",
        "model_provider",
        "observation_sequence",
    ]
    .iter()
    .all(|column| columns.contains(*column))
}

fn local_catalog_host_id(db: &Connection) -> anyhow::Result<Option<String>> {
    let columns = table_columns(db, "local_thread_catalog_hosts")?;
    if !columns.contains("host_id") {
        return Ok(Some("local".to_string()));
    }
    let query = if columns.contains("host_kind") {
        "SELECT host_id FROM local_thread_catalog_hosts WHERE LOWER(COALESCE(host_kind, '')) = 'local' ORDER BY host_id LIMIT 1"
    } else {
        "SELECT host_id FROM local_thread_catalog_hosts WHERE host_id = 'local' LIMIT 1"
    };
    match db.query_row(query, [], |row| row.get::<_, String>(0)) {
        Ok(host_id) if !host_id.trim().is_empty() => Ok(Some(host_id)),
        Ok(_) | Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn local_catalog_max_observation_sequence(db: &Connection, host_id: &str) -> anyhow::Result<i64> {
    let columns = table_columns(db, "local_thread_catalog")?;
    if !columns.contains("observation_sequence") {
        return Ok(0);
    }
    if columns.contains("host_id") {
        Ok(db.query_row(
            "SELECT COALESCE(MAX(observation_sequence), 0) FROM local_thread_catalog WHERE host_id = ?1",
            [host_id],
            |row| row.get::<_, i64>(0),
        )?)
    } else {
        Ok(db.query_row(
            "SELECT COALESCE(MAX(observation_sequence), 0) FROM local_thread_catalog",
            [],
            |row| row.get::<_, i64>(0),
        )?)
    }
}

fn local_catalog_contains_thread(
    db: &Connection,
    host_id: &str,
    thread_id: &str,
) -> anyhow::Result<bool> {
    Ok(db
        .query_row(
            "SELECT 1 FROM local_thread_catalog WHERE host_id = ?1 AND thread_id = ?2 LIMIT 1",
            (host_id, thread_id),
            |_| Ok(()),
        )
        .is_ok())
}

fn local_catalog_insert_columns(columns: &HashSet<String>) -> Vec<&'static str> {
    let mut names = vec![
        "host_id",
        "thread_id",
        "display_title",
        "source_created_at",
        "source_updated_at",
        "cwd",
        "source_kind",
        "model_provider",
        "observation_sequence",
    ];
    for optional in [
        "source_detail",
        "missing_candidate",
        "git_branch",
        "thread_source",
    ] {
        if columns.contains(optional) {
            names.push(optional);
        }
    }
    names
}

fn local_catalog_insert_values(
    columns: &[&str],
    host_id: &str,
    thread: &CatalogRepairThread,
    observation_sequence: i64,
) -> Vec<SqlValue> {
    columns
        .iter()
        .map(|column| match *column {
            "host_id" => SqlValue::Text(host_id.to_string()),
            "thread_id" => SqlValue::Text(thread.id.clone()),
            "display_title" => SqlValue::Text(thread.display_title.clone()),
            "source_created_at" => SqlValue::Real(thread.source_created_at),
            "source_updated_at" => SqlValue::Real(thread.source_updated_at),
            "cwd" => SqlValue::Text(thread.cwd.clone()),
            "source_kind" => SqlValue::Text(thread.source_kind.clone()),
            "source_detail" => SqlValue::Text(thread.source_detail.clone()),
            "model_provider" => SqlValue::Text(thread.model_provider.clone()),
            "git_branch" => thread
                .git_branch
                .clone()
                .map(SqlValue::Text)
                .unwrap_or(SqlValue::Null),
            "thread_source" => thread
                .thread_source
                .clone()
                .map(SqlValue::Text)
                .unwrap_or(SqlValue::Null),
            "observation_sequence" => SqlValue::Integer(observation_sequence),
            "missing_candidate" => SqlValue::Integer(0),
            _ => SqlValue::Null,
        })
        .collect()
}

fn update_local_catalog_metadata(
    tx: &rusqlite::Transaction<'_>,
    columns: &HashSet<String>,
    inserted: usize,
) -> anyhow::Result<()> {
    if !columns.contains("catalog_revision") {
        return Ok(());
    }
    let affected = tx.execute(
        "UPDATE local_thread_catalog_metadata SET catalog_revision = catalog_revision + ?1",
        [inserted as i64],
    )?;
    if affected == 0 && columns.contains("id") {
        tx.execute(
            "INSERT INTO local_thread_catalog_metadata (id, catalog_revision) VALUES (1, ?1)",
            [inserted as i64],
        )?;
    }
    Ok(())
}

fn update_local_catalog_sync_state(
    tx: &rusqlite::Transaction<'_>,
    columns: &HashSet<String>,
    host_id: &str,
    observation_sequence: i64,
    max_source_updated_at: f64,
) -> anyhow::Result<()> {
    if !columns.contains("host_id") {
        return Ok(());
    }
    let now = now_secs() as i64;
    let mut assignments = Vec::new();
    let mut values = Vec::new();
    if columns.contains("initial_build_complete") {
        assignments.push("initial_build_complete = 1");
    }
    if columns.contains("observation_sequence") {
        assignments.push("observation_sequence = MAX(COALESCE(observation_sequence, 0), ?)");
        values.push(SqlValue::Integer(observation_sequence));
    }
    if columns.contains("watermark_updated_at") {
        assignments.push("watermark_updated_at = MAX(COALESCE(watermark_updated_at, 0), ?)");
        values.push(SqlValue::Real(max_source_updated_at));
    }
    if columns.contains("last_full_reconciled_at") {
        assignments.push("last_full_reconciled_at = MAX(COALESCE(last_full_reconciled_at, 0), ?)");
        values.push(SqlValue::Integer(now));
    }
    if assignments.is_empty() {
        return Ok(());
    }
    let update_sql = format!(
        "UPDATE local_thread_catalog_sync_state SET {} WHERE host_id = ?",
        assignments.join(", ")
    );
    let mut update_values = values.clone();
    update_values.push(SqlValue::Text(host_id.to_string()));
    let affected = tx.execute(&update_sql, params_from_iter(update_values))?;
    if affected == 0 {
        let mut insert_columns = vec!["host_id"];
        let mut insert_values = vec![SqlValue::Text(host_id.to_string())];
        if columns.contains("watermark_updated_at") {
            insert_columns.push("watermark_updated_at");
            insert_values.push(SqlValue::Real(max_source_updated_at));
        }
        if columns.contains("initial_build_complete") {
            insert_columns.push("initial_build_complete");
            insert_values.push(SqlValue::Integer(1));
        }
        if columns.contains("observation_sequence") {
            insert_columns.push("observation_sequence");
            insert_values.push(SqlValue::Integer(observation_sequence));
        }
        if columns.contains("last_full_reconciled_at") {
            insert_columns.push("last_full_reconciled_at");
            insert_values.push(SqlValue::Integer(now));
        }
        let placeholders = std::iter::repeat_n("?", insert_columns.len())
            .collect::<Vec<_>>()
            .join(", ");
        let insert_sql = format!(
            "INSERT INTO local_thread_catalog_sync_state ({}) VALUES ({})",
            insert_columns.join(", "),
            placeholders
        );
        tx.execute(&insert_sql, params_from_iter(insert_values))?;
    }
    Ok(())
}

fn text_expr(columns: &HashSet<String>, column: &str, fallback: &str) -> String {
    if columns.contains(column) {
        format!("COALESCE({column}, {fallback})")
    } else {
        fallback.to_string()
    }
}

fn coalesce_text_expr(columns: &HashSet<String>, candidates: &[&str], fallback: &str) -> String {
    let mut parts = candidates
        .iter()
        .filter(|column| columns.contains(**column))
        .map(|column| format!("NULLIF({column}, '')"))
        .collect::<Vec<_>>();
    parts.push(fallback.to_string());
    if parts.len() == 1 {
        parts.remove(0)
    } else {
        format!("COALESCE({})", parts.join(", "))
    }
}

fn timestamp_expr(columns: &HashSet<String>, ms_column: &str, seconds_column: &str) -> String {
    if columns.contains(ms_column) {
        format!("COALESCE({ms_column} / 1000.0, 0)")
    } else if columns.contains(seconds_column) {
        format!(
            "CASE WHEN COALESCE({seconds_column}, 0) > 9999999999 THEN {seconds_column} / 1000.0 ELSE COALESCE({seconds_column}, 0) END"
        )
    } else {
        "0".to_string()
    }
}

fn load_global_state(path: &Path) -> anyhow::Result<Map<String, Value>> {
    if !path.exists() {
        return Ok(Map::new());
    }
    Ok(serde_json::from_str::<Value>(&fs::read_to_string(path)?)?
        .as_object()
        .cloned()
        .unwrap_or_default())
}

fn load_projectless_thread_ids(path: &Path) -> anyhow::Result<HashSet<String>> {
    let state = load_global_state(path)?;
    let mut ids = HashSet::new();
    if let Some(items) = state
        .get("projectless-thread-ids")
        .and_then(Value::as_array)
    {
        for item in items {
            if let Some(id) = item.as_str().filter(|id| !id.trim().is_empty()) {
                ids.insert(id.to_string());
            }
        }
    }
    Ok(ids)
}

fn normalized_global_state(state: &Map<String, Value>) -> Map<String, Value> {
    let mut next = Map::new();
    if let Some(value) = state.get("electron-saved-workspace-roots") {
        next.insert(
            "electron-saved-workspace-roots".to_string(),
            json!(dedupe_paths(path_array(value))),
        );
    }
    if let Some(value) = state.get("project-order") {
        next.insert(
            "project-order".to_string(),
            json!(dedupe_paths(path_array(value))),
        );
    }
    if let Some(value) = state.get("active-workspace-roots") {
        let normalized = dedupe_paths(path_array(value));
        let next_value = if value.is_array() {
            json!(normalized)
        } else if let Some(first) = normalized.first() {
            json!(first)
        } else {
            value.clone()
        };
        next.insert("active-workspace-roots".to_string(), next_value);
    }
    if let Some(value) = state
        .get("electron-workspace-root-labels")
        .and_then(Value::as_object)
    {
        let mut labels = Map::new();
        for (key, item) in value {
            labels.insert(
                to_desktop_workspace_path(key).unwrap_or_else(|| key.clone()),
                item.clone(),
            );
        }
        next.insert(
            "electron-workspace-root-labels".to_string(),
            Value::Object(labels),
        );
    }
    if let Some(open_targets) = state
        .get("open-in-target-preferences")
        .and_then(Value::as_object)
    {
        let mut next_open_targets = open_targets.clone();
        if let Some(per_path) =
            copy_resolved_object_keys(open_targets.get("perPath").and_then(Value::as_object))
        {
            next_open_targets.insert("perPath".to_string(), Value::Object(per_path));
        }
        next.insert(
            "open-in-target-preferences".to_string(),
            Value::Object(next_open_targets),
        );
    }
    next
}

fn copy_resolved_object_keys(value: Option<&Map<String, Value>>) -> Option<Map<String, Value>> {
    let value = value?;
    let mut next = Map::new();
    for (key, item) in value {
        next.insert(
            to_desktop_workspace_path(key).unwrap_or_else(|| key.clone()),
            item.clone(),
        );
    }
    Some(next)
}

fn count_global_state_updates(path: &Path) -> anyhow::Result<usize> {
    let state = load_global_state(path)?;
    let next = normalized_global_state(&state);
    Ok(next
        .iter()
        .filter(|(key, value)| state.get(*key) != Some(*value))
        .count())
}

fn apply_global_state_update(path: &Path) -> anyhow::Result<usize> {
    let mut state = load_global_state(path)?;
    let next = normalized_global_state(&state);
    let count = next
        .iter()
        .filter(|(key, value)| state.get(*key) != Some(*value))
        .count();
    if count > 0 {
        for (key, value) in next {
            state.insert(key, value);
        }
        let text = serde_json::to_string_pretty(&Value::Object(state))?;
        fs::write(path, &text)?;
        if let Some(parent) = path.parent() {
            fs::write(parent.join(".codex-global-state.json.bak"), text)?;
        }
    }
    Ok(count)
}

fn path_array(value: &Value) -> Vec<String> {
    if let Some(items) = value.as_array() {
        items
            .iter()
            .filter_map(Value::as_str)
            .filter(|item| !item.trim().is_empty())
            .map(ToString::to_string)
            .collect()
    } else if let Some(value) = value.as_str().filter(|item| !item.trim().is_empty()) {
        vec![value.to_string()]
    } else {
        Vec::new()
    }
}

fn dedupe_paths(paths: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for path in paths {
        let Some(desktop) = to_desktop_workspace_path(&path) else {
            continue;
        };
        let comparable = desktop
            .replace('/', r"\")
            .trim_end_matches('\\')
            .to_ascii_lowercase();
        if seen.insert(comparable) {
            result.push(desktop);
        }
    }
    result
}

fn prune_backups(home: &Path) -> anyhow::Result<()> {
    let root = home.join("backups_state/provider-sync");
    if !root.exists() {
        return Ok(());
    }
    let mut managed = Vec::new();
    for entry in fs::read_dir(&root)? {
        let path = entry?.path();
        if !path.is_dir() {
            continue;
        }
        let Ok(text) = fs::read_to_string(path.join("metadata.json")) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        if value.get("managedBy").and_then(Value::as_str) == Some("Codex++ provider sync") {
            managed.push(path);
        }
    }
    managed.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
    for path in managed.into_iter().skip(BACKUP_KEEP_COUNT) {
        let _ = fs::remove_dir_all(path);
    }
    Ok(())
}

fn timestamp_name() -> String {
    chrono::Local::now().format("%Y%m%d%H%M%S").to_string()
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod bounded_memory_tests {
    use super::*;
    use std::cell::Cell;
    use std::io::{BufWriter, Write};
    use tempfile::tempdir;

    #[test]
    fn collection_retains_metadata_instead_of_rollout_payloads() {
        let tmp = tempdir().unwrap();
        let home = tmp.path().join(".codex");
        fs::create_dir_all(home.join("sessions")).unwrap();
        let chunk = vec![b'A'; 64 * 1024];
        for index in 0..8 {
            let path = home.join(format!("sessions/rollout-{index}.jsonl"));
            let mut writer = BufWriter::new(File::create(path).unwrap());
            writeln!(
                writer,
                "{}",
                json!({
                    "type": "session_meta",
                    "payload": {
                        "id": format!("thread-{index}"),
                        "model_provider": "openai",
                        "cwd": "C:/workspace"
                    }
                })
            )
            .unwrap();
            writer
                .write_all(b"{\"type\":\"event_msg\",\"payload\":{\"blob\":\"")
                .unwrap();
            for _ in 0..16 {
                writer.write_all(&chunk).unwrap();
            }
            writer.write_all(b"\"}}\n").unwrap();
            writer.flush().unwrap();
        }

        let collected =
            collect_session_changes(&home, "custom", &HashSet::new(), &HashSet::new()).unwrap();

        assert_eq!(collected.changes.len(), 8);
        let retained_bytes = collected
            .changes
            .iter()
            .map(|change| {
                change.original_sha256.len()
                    + change.thread_id.as_ref().map_or(0, String::len)
                    + change.cwd.as_ref().map_or(0, String::len)
            })
            .sum::<usize>();
        assert!(
            retained_bytes < 64 * 1024,
            "retained {retained_bytes} bytes"
        );
    }

    #[test]
    fn repeated_session_meta_providers_are_retained_once() {
        let tmp = tempdir().unwrap();
        let rollout = tmp.path().join("rollout-many-meta.jsonl");
        let mut writer = BufWriter::new(File::create(&rollout).unwrap());
        for index in 0..10_000 {
            writeln!(
                writer,
                "{}",
                json!({
                    "type": "session_meta",
                    "payload": {
                        "id": format!("thread-{index}"),
                        "model_provider": "openai"
                    }
                })
            )
            .unwrap();
        }
        writer.flush().unwrap();

        let rewrite = scan_rollout_session_meta_providers(
            &rollout,
            "custom",
            &SessionRewriteMode::AllProviders,
        )
        .unwrap();

        assert_eq!(rewrite.session_meta_count, 10_000);
        assert_eq!(rewrite.providers, HashSet::from(["openai".to_string()]));
    }

    #[test]
    fn unchanged_rollout_reuses_committed_scan_state_without_reading_the_body() {
        let tmp = tempdir().unwrap();
        let home = tmp.path().join(".codex");
        let rollout = home.join("sessions/rollout-cached.jsonl");
        fs::create_dir_all(rollout.parent().unwrap()).unwrap();
        fs::write(
            &rollout,
            format!(
                "{}\n{}\n",
                json!({
                    "type": "session_meta",
                    "payload": {
                        "id": "thread-cached",
                        "model_provider": "openai",
                        "cwd": "C:/cached"
                    }
                }),
                json!({"type": "event_msg", "payload": {"type": "user_message"}})
            ),
        )
        .unwrap();
        let (_, _, state) = scan_rollout_for_provider_sync_state(
            &home,
            &rollout,
            "openai",
            &SessionRewriteMode::AllProviders,
        )
        .unwrap();
        let state = state.unwrap();
        persist_provider_sync_scan_state(&home, std::slice::from_ref(&state)).unwrap();

        let collected = collect_session_changes_with_scanner(
            &home,
            "openai",
            &HashSet::new(),
            &HashSet::new(),
            load_provider_sync_scan_state(&home).unwrap(),
            |_, _, _, _| panic!("unchanged rollout body should not be opened"),
        )
        .unwrap();

        assert_eq!(collected.changes.len(), 1);
        assert!(!collected.changes[0].rewrite_needed);
        assert!(collected.changes[0].has_user_event);
        assert_eq!(collected.changes[0].cwd.as_deref(), Some("C:/cached"));
        assert_eq!(collected.scan_state_entries, vec![state]);
    }

    #[test]
    fn changed_target_or_missing_mtime_forces_a_full_rollout_scan() {
        let tmp = tempdir().unwrap();
        let home = tmp.path().join(".codex");
        let rollout = home.join("sessions/rollout-rescan.jsonl");
        fs::create_dir_all(rollout.parent().unwrap()).unwrap();
        fs::write(
            &rollout,
            format!(
                "{}\n",
                json!({
                    "type": "session_meta",
                    "payload": {
                        "id": "thread-rescan",
                        "model_provider": "openai"
                    }
                })
            ),
        )
        .unwrap();
        let (_, _, state) = scan_rollout_for_provider_sync_state(
            &home,
            &rollout,
            "openai",
            &SessionRewriteMode::AllProviders,
        )
        .unwrap();
        let state = state.unwrap();
        let scans = Cell::new(0);
        let collected = collect_session_changes_with_scanner(
            &home,
            "custom",
            &HashSet::new(),
            &HashSet::new(),
            HashMap::from([(state.relative_path.clone(), state.clone())]),
            |home, path, target, mode| {
                scans.set(scans.get() + 1);
                scan_rollout_for_provider_sync_state(home, path, target, mode)
            },
        )
        .unwrap();
        assert_eq!(scans.get(), 1);
        assert!(collected.changes[0].rewrite_needed);

        let mut legacy = state;
        legacy.modified_secs = None;
        legacy.modified_nanos = None;
        let scans = Cell::new(0);
        collect_session_changes_with_scanner(
            &home,
            "openai",
            &HashSet::new(),
            &HashSet::new(),
            HashMap::from([(legacy.relative_path.clone(), legacy)]),
            |home, path, target, mode| {
                scans.set(scans.get() + 1);
                scan_rollout_for_provider_sync_state(home, path, target, mode)
            },
        )
        .unwrap();
        assert_eq!(scans.get(), 1);
    }

    #[test]
    fn same_path_size_and_mtime_still_reject_a_replaced_file_instance() {
        let tmp = tempdir().unwrap();
        let home = tmp.path().join(".codex");
        let rollout = home.join("sessions/rollout-replaced.jsonl");
        fs::create_dir_all(rollout.parent().unwrap()).unwrap();
        let bytes = format!(
            "{}\n",
            json!({
                "type": "session_meta",
                "payload": {"id": "thread-replaced", "model_provider": "openai"}
            })
        );
        fs::write(&rollout, &bytes).unwrap();
        let (_, _, state) = scan_rollout_for_provider_sync_state(
            &home,
            &rollout,
            "openai",
            &SessionRewriteMode::AllProviders,
        )
        .unwrap();
        let state = state.unwrap();
        let original_mtime = fs::metadata(&rollout).unwrap().modified().unwrap();
        let original_identity = state.file_identity.clone();
        let replacement = rollout.with_extension("replacement");
        fs::write(&replacement, &bytes).unwrap();
        fs::File::options()
            .write(true)
            .open(&replacement)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(original_mtime))
            .unwrap();
        fs::remove_file(&rollout).unwrap();
        fs::rename(&replacement, &rollout).unwrap();
        let replacement_file = File::open(&rollout).unwrap();
        assert_ne!(
            codex_plus_core::settings::file_instance_identity(&replacement_file).unwrap(),
            original_identity
        );
        assert_eq!(fs::metadata(&rollout).unwrap().len(), state.size);
        assert_eq!(
            system_time_parts(fs::metadata(&rollout).unwrap().modified().ok()),
            (state.modified_secs, state.modified_nanos)
        );
        assert!(!provider_sync_scan_state_matches(&rollout, &state));
    }

    #[test]
    fn scan_rule_digest_or_corrupt_state_invalidates_the_entire_cache() {
        let tmp = tempdir().unwrap();
        let home = tmp.path().join(".codex");
        let path = provider_sync_scan_state_path(&home);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            serde_json::to_vec_pretty(&ProviderSyncRolloutScanStateManifest {
                version: PROVIDER_SYNC_SCAN_STATE_VERSION,
                namespace: PROVIDER_SYNC_SCAN_STATE_NAMESPACE.to_string(),
                rules_sha256: "0".repeat(64),
                rollout_roots: session_transaction_rollout_roots(&home).unwrap(),
                entries: Vec::new(),
            })
            .unwrap(),
        )
        .unwrap();
        assert!(load_provider_sync_scan_state(&home).is_err());

        fs::write(&path, b"not-json").unwrap();
        assert!(load_provider_sync_scan_state(&home).is_err());
        assert!(load_provider_sync_scan_state_best_effort(&home).is_empty());

        let oversized = File::create(&path).unwrap();
        oversized
            .set_len(PROVIDER_SYNC_SCAN_STATE_MAX_BYTES + 1)
            .unwrap();
        drop(oversized);
        assert!(load_provider_sync_scan_state(&home).is_err());
    }

    #[test]
    fn replacing_a_rollout_root_invalidates_the_entire_cache() {
        let tmp = tempdir().unwrap();
        let home = tmp.path().join(".codex");
        let sessions = home.join("sessions");
        let rollout = sessions.join("rollout-root.jsonl");
        fs::create_dir_all(&sessions).unwrap();
        fs::write(
            &rollout,
            format!(
                "{}\n",
                json!({
                    "type": "session_meta",
                    "payload": {"id": "thread-root", "model_provider": "openai"}
                })
            ),
        )
        .unwrap();
        let (_, _, state) = scan_rollout_for_provider_sync_state(
            &home,
            &rollout,
            "openai",
            &SessionRewriteMode::AllProviders,
        )
        .unwrap();
        persist_provider_sync_scan_state(&home, &[state.unwrap()]).unwrap();

        fs::rename(&sessions, home.join("sessions-old")).unwrap();
        fs::create_dir(&sessions).unwrap();

        assert!(load_provider_sync_scan_state(&home).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn scan_state_parent_creation_rejects_a_symlink_before_touching_its_target() {
        use std::os::unix::fs::symlink;

        let tmp = tempdir().unwrap();
        let home = tmp.path().join(".codex");
        let outside = tmp.path().join("outside");
        fs::create_dir(&home).unwrap();
        fs::create_dir(&outside).unwrap();
        symlink(&outside, home.join("backups_state")).unwrap();

        assert!(persist_provider_sync_scan_state(&home, &[]).is_err());
        assert!(!outside.join("provider-sync").exists());
    }

    #[cfg(windows)]
    #[test]
    fn scan_state_parent_creation_rejects_a_junction_before_touching_its_target() {
        use std::process::Command;

        let tmp = tempdir().unwrap();
        let home = tmp.path().join(".codex");
        let outside = tmp.path().join("outside");
        fs::create_dir(&home).unwrap();
        fs::create_dir(&outside).unwrap();
        let link = home.join("backups_state");
        let output = Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "New-Item -ItemType Junction -Path $env:CODEXPP_TEST_LINK -Target $env:CODEXPP_TEST_TARGET | Out-Null",
            ])
            .env("CODEXPP_TEST_LINK", &link)
            .env("CODEXPP_TEST_TARGET", &outside)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "junction creation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        assert!(persist_provider_sync_scan_state(&home, &[]).is_err());
        assert!(!outside.join("provider-sync").exists());
        fs::remove_dir(link).unwrap();
    }

    #[test]
    fn prepared_backup_handle_keeps_the_verified_file_instance() {
        let tmp = tempdir().unwrap();
        let backup_dir = tmp.path().join("backup");
        let source = backup_dir.join("db/state_5.sqlite");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(&source, b"verified-backup").unwrap();
        let (sha256, size) = file_sha256_and_size(&source).unwrap();
        let evidence = HashMap::from([(
            "db/state_5.sqlite".to_string(),
            ProviderSyncBackupFileEvidence { size, sha256 },
        )]);
        let mut prepared = prepare_provider_sync_backup_files(
            &backup_dir,
            &evidence,
            "0123456789abcdef0123456789abcdef",
        )
        .unwrap();

        fs::write(&source, b"replacement-path-content").unwrap();
        let prepared = prepared.files.get_mut("db/state_5.sqlite").unwrap();
        prepared.file.seek(SeekFrom::Start(0)).unwrap();
        let mut bytes = Vec::new();
        prepared.file.read_to_end(&mut bytes).unwrap();

        assert_eq!(bytes, b"verified-backup");
        assert_eq!(fs::read(source).unwrap(), b"replacement-path-content");
    }

    #[cfg(unix)]
    #[test]
    fn rollout_discovery_rejects_nested_symlinks() {
        use std::os::unix::fs::symlink;

        let tmp = tempdir().unwrap();
        let home = tmp.path().join(".codex");
        let outside = tmp.path().join("outside");
        fs::create_dir_all(home.join("sessions")).unwrap();
        fs::create_dir(&outside).unwrap();
        symlink(&outside, home.join("sessions/link")).unwrap();

        let error = rollout_files(&home).unwrap_err();

        assert!(error.to_string().contains("symlink"));
    }

    #[cfg(windows)]
    #[test]
    fn rollout_discovery_rejects_nested_junctions() {
        use std::process::Command;

        let tmp = tempdir().unwrap();
        let home = tmp.path().join(".codex");
        let outside = tmp.path().join("outside");
        fs::create_dir_all(home.join("sessions")).unwrap();
        fs::create_dir(&outside).unwrap();
        let link = home.join("sessions/link");
        let output = Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "New-Item -ItemType Junction -Path $env:CODEXPP_TEST_LINK -Target $env:CODEXPP_TEST_TARGET | Out-Null",
            ])
            .env("CODEXPP_TEST_LINK", &link)
            .env("CODEXPP_TEST_TARGET", &outside)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "junction creation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let error = rollout_files(&home).unwrap_err();

        assert!(error.to_string().contains("reparse point"));
        fs::remove_dir(link).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn downstream_directory_guard_blocks_parent_replacement() {
        let tmp = tempdir().unwrap();
        let home = tmp.path().join(".codex");
        let moved = tmp.path().join("moved-home");
        fs::create_dir(&home).unwrap();
        let guards = prepare_provider_sync_target_parents(&home, &[]).unwrap();

        assert!(fs::rename(&home, &moved).is_err());
        drop(guards);
        fs::rename(&home, &moved).unwrap();
    }

    #[test]
    fn orphan_cleanup_requires_an_exact_transaction_temp_name() {
        let transaction_id = "0123456789abcdef0123456789abcdef";
        assert!(provider_sync_temp_name_belongs(
            ".rollout-a.jsonl.provider-sync-0123456789abcdef0123456789abcdef-4.tmp",
            transaction_id
        ));
        assert!(provider_sync_temp_name_belongs(
            ".rollout-a.jsonl.provider-sync-displaced-0123456789abcdef0123456789abcdef-4.tmp",
            transaction_id
        ));
        assert!(provider_sync_temp_name_belongs(
            ".rollout-a.jsonl.provider-sync-restore-0123456789abcdef0123456789abcdef.tmp",
            transaction_id
        ));
        assert!(!provider_sync_temp_name_belongs(
            ".user.provider-sync-note-0123456789abcdef0123456789abcdef.tmp",
            transaction_id
        ));
        assert!(!provider_sync_temp_name_belongs(
            ".rollout-a.jsonl.provider-sync-0123456789abcdef0123456789abcdef-x.tmp",
            transaction_id
        ));
    }
}

#[cfg(test)]
mod non_root_agent_tests {
    use super::*;

    fn marks_non_root(source: &str) -> bool {
        source_structured_marks_non_root_agent(source)
    }

    #[test]
    fn structured_subagent_markers_still_identify_child_threads() {
        assert!(marks_non_root(
            r#"{"subagent":{"thread_spawn":{"depth":1}}}"#
        ));
        assert!(marks_non_root(r#"{"sub_agent":{"other":"review"}}"#));
        assert!(marks_non_root(r#"{"internal":true}"#));
    }

    /// 这些取值明确表示「不是子代理」。判定的下游是 DELETE，
    /// 按 key 存在就算数会把真实会话删掉（issue #1948）。
    #[test]
    fn markers_that_explicitly_deny_being_a_subagent_do_not_count() {
        assert!(!marks_non_root(r#"{"internal":false}"#));
        assert!(!marks_non_root(r#"{"sub_agent":null}"#));
        assert!(!marks_non_root(r#"{"subagent":false}"#));
    }

    /// 占位字段（空对象/空串）同样不构成声明。
    #[test]
    fn empty_placeholder_markers_do_not_count() {
        assert!(!marks_non_root(r#"{"subagent":{}}"#));
        assert!(!marks_non_root(r#"{"sub_agent":[]}"#));
        assert!(!marks_non_root(r#"{"internal":"  "}"#));
    }

    #[test]
    fn unrelated_or_malformed_sources_are_left_alone() {
        assert!(!marks_non_root(r#"{"origin":"subagent"}"#));
        assert!(!marks_non_root(r#"{"sub_agent":"#));
        assert!(!marks_non_root("cli"));
    }
}

#[cfg(test)]
mod lock_state_tests {
    use super::*;
    use codex_plus_core::watcher::ProcessInstanceState;

    fn owner(pid: u32) -> ProviderSyncLockOwner {
        ProviderSyncLockOwner {
            pid,
            started_at: 1234,
            process_started_at: Some(1200),
            process_birth_id: Some("birth-1200".to_string()),
            lock_id: Some("lock-1".to_string()),
        }
    }

    fn running(started_at_secs: Option<u64>) -> ProcessInstanceState {
        ProcessInstanceState::Running {
            started_at_secs,
            birth_id: started_at_secs.map(|started_at| format!("birth-{started_at}")),
        }
    }

    #[test]
    fn live_owner_counts_as_held() {
        let state = classify_lock(Some(&owner(42)), Some(0), |_| running(Some(1200)));

        assert_eq!(
            state,
            ProviderSyncLockState::Held {
                pid: 42,
                started_at: 1234
            }
        );
    }

    #[test]
    fn dead_owner_counts_as_stale() {
        let state = classify_lock(Some(&owner(42)), Some(0), |_| {
            ProcessInstanceState::NotRunning
        });

        assert_eq!(state, ProviderSyncLockState::Stale { pid: Some(42) });
    }

    #[test]
    fn reused_pid_with_a_different_process_start_is_stale() {
        let state = classify_lock(Some(&owner(42)), Some(9_999), |_| running(Some(5000)));

        assert_eq!(state, ProviderSyncLockState::Stale { pid: Some(42) });
    }

    #[test]
    fn matching_birth_id_tolerates_approximate_unix_start_time_drift() {
        let state = classify_lock(Some(&owner(42)), Some(0), |_| {
            ProcessInstanceState::Running {
                started_at_secs: Some(1201),
                birth_id: Some("birth-1200".to_string()),
            }
        });

        assert_eq!(
            state,
            ProviderSyncLockState::Held {
                pid: 42,
                started_at: 1234
            }
        );
    }

    #[test]
    fn legacy_owner_with_a_much_newer_process_is_stale() {
        let legacy_owner = ProviderSyncLockOwner {
            process_started_at: None,
            process_birth_id: None,
            lock_id: None,
            ..owner(42)
        };
        let state = classify_lock(
            Some(&legacy_owner),
            Some(LEGACY_PID_REUSE_MIN_LOCK_AGE_SECS),
            |_| {
                running(Some(
                    legacy_owner.started_at + LEGACY_PID_REUSE_TOLERANCE_SECS + 1,
                ))
            },
        );

        assert_eq!(state, ProviderSyncLockState::Stale { pid: Some(42) });
    }

    #[test]
    fn legacy_owner_keeps_a_process_started_before_the_lock() {
        let legacy_owner = ProviderSyncLockOwner {
            process_started_at: None,
            process_birth_id: None,
            lock_id: None,
            ..owner(42)
        };
        let state = classify_lock(Some(&legacy_owner), Some(9_999), |_| {
            running(Some(legacy_owner.started_at - 1))
        });

        assert_eq!(
            state,
            ProviderSyncLockState::Held {
                pid: 42,
                started_at: 1234
            }
        );
    }

    #[test]
    fn recent_legacy_lock_remains_held_even_if_wall_clock_evidence_looks_newer() {
        let legacy_owner = ProviderSyncLockOwner {
            process_started_at: None,
            process_birth_id: None,
            lock_id: None,
            ..owner(42)
        };
        let state = classify_lock(
            Some(&legacy_owner),
            Some(LEGACY_PID_REUSE_MIN_LOCK_AGE_SECS - 1),
            |_| {
                running(Some(
                    legacy_owner.started_at + LEGACY_PID_REUSE_TOLERANCE_SECS + 1,
                ))
            },
        );

        assert_eq!(
            state,
            ProviderSyncLockState::Held {
                pid: 42,
                started_at: 1234
            }
        );
    }

    #[test]
    fn unknown_process_identity_is_treated_as_held_rather_than_stolen() {
        let state = classify_lock(Some(&owner(42)), Some(9_999), |_| {
            ProcessInstanceState::Unknown
        });

        assert_eq!(
            state,
            ProviderSyncLockState::Held {
                pid: 42,
                started_at: 1234
            }
        );
    }

    #[test]
    fn aged_lock_without_owner_is_recoverable_interrupted_leftover() {
        let state = classify_lock(None, Some(LOCK_INTERRUPTED_GRACE_SECS), |_| {
            running(Some(1200))
        });

        assert_eq!(state, ProviderSyncLockState::Stale { pid: None });
    }

    #[test]
    fn fresh_lock_without_owner_is_left_alone_for_the_process_still_creating_it() {
        let state = classify_lock(None, Some(LOCK_INTERRUPTED_GRACE_SECS - 1), |_| {
            running(Some(1200))
        });

        assert_eq!(state, ProviderSyncLockState::Indeterminate);
    }

    #[test]
    fn unreadable_lock_age_is_left_alone() {
        let state = classify_lock(None, None, |_| running(Some(1200)));

        assert_eq!(state, ProviderSyncLockState::Indeterminate);
    }

    #[test]
    fn legacy_owner_json_remains_compatible() {
        let owner: ProviderSyncLockOwner =
            serde_json::from_str(r#"{"pid":42,"startedAt":1234}"#).unwrap();

        assert_eq!(owner.pid, 42);
        assert_eq!(owner.started_at, 1234);
        assert_eq!(owner.process_started_at, None);
        assert_eq!(owner.process_birth_id, None);
        assert_eq!(owner.lock_id, None);
    }

    #[test]
    fn lifecycle_guard_serializes_and_releases_the_legacy_directory() {
        let temp = tempfile::tempdir().unwrap();
        let lock_dir = temp.path().join("tmp/provider-sync.lock");
        let first = acquire_lock_inner(&lock_dir, false).unwrap();

        assert!(lock_dir.join("owner.json").is_file());
        let error = acquire_lock_inner(&lock_dir, false).unwrap_err();
        assert!(
            matches!(
                error.kind(),
                std::io::ErrorKind::AlreadyExists | std::io::ErrorKind::WouldBlock
            ),
            "unexpected lock contention error: {error:?}; raw={:?}",
            error.raw_os_error()
        );

        drop(first);
        assert!(!lock_dir.exists());
        let second = acquire_lock_inner(&lock_dir, false).unwrap();
        drop(second);
        assert!(!lock_dir.exists());
    }

    #[test]
    fn a_guard_cannot_remove_a_directory_owned_by_another_lock_id() {
        let temp = tempfile::tempdir().unwrap();
        let lock_dir = temp.path().join("tmp/provider-sync.lock");
        let guard = acquire_lock_inner(&lock_dir, false).unwrap();

        assert!(!release_owned_lock(&lock_dir, "not-the-owner").unwrap());
        assert!(lock_dir.join("owner.json").is_file());

        drop(guard);
        assert!(!lock_dir.exists());
    }

    #[test]
    fn explicit_release_rejects_changed_directory_ownership() {
        let temp = tempfile::tempdir().unwrap();
        let lock_dir = temp.path().join("tmp/provider-sync.lock");
        let guard = acquire_lock_inner(&lock_dir, false).unwrap();
        fs::write(
            lock_dir.join("owner.json"),
            json!({
                "pid": std::process::id(),
                "startedAt": now_secs(),
                "lockId": "replacement-owner",
            })
            .to_string(),
        )
        .unwrap();

        let error = guard.release().unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::Other);
        assert!(lock_dir.exists());
    }

    #[test]
    fn os_lock_authoritatively_recovers_an_orphaned_new_protocol_directory() {
        let temp = tempfile::tempdir().unwrap();
        let lock_dir = temp.path().join("tmp/provider-sync.lock");
        let guard = acquire_lock_inner(&lock_dir, false).unwrap();
        fs::write(
            lock_dir.join("owner.json"),
            json!({
                "pid": std::process::id(),
                "startedAt": now_secs(),
                "lockId": "orphaned-owner",
            })
            .to_string(),
        )
        .unwrap();
        drop(guard);
        assert!(lock_dir.exists());

        let recovered = acquire_lock_inner(&lock_dir, false).unwrap();
        recovered.release().unwrap();

        assert!(!lock_dir.exists());
    }
}
