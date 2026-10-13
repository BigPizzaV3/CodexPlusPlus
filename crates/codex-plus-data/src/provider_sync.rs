use fs2::FileExt;
use crate::storage::{has_table, json_to_sql_value, select_dicts};
use rusqlite::{Connection, OptionalExtension, ToSql, params_from_iter, types::Value as SqlValue};
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
const PROVIDER_SYNC_PROGRESS_INTERVAL: usize = 32;
/// 全局状态的旁路备份名；解析失败时指向它作为恢复来源（issue #2160）。
const GLOBAL_STATE_BACKUP_FILE_NAME: &str = ".codex-global-state.json.bak";
/// 删除墓碑（issue #2199）。放在 `tmp/` 下，和同步锁同级：
/// 既不属于会话数据，也不在 `rollout_files` 的扫描范围（只扫 sessions/archived_sessions）里，
/// 因此不会被同步逻辑当成会话读取。
const THREAD_TOMBSTONE_RELATIVE_PATH: &str = "tmp/provider-sync-tombstones.json";
/// 墓碑只增不删，长期使用会攒条目；超过这个数量就丢掉最旧的一批。
/// 丢墓碑最坏的结果是「被删的会话可能被重建回来」，与不记墓碑时一致，
/// 不会损坏任何数据，所以这里的上限不必设得很大。
const MAX_THREAD_TOMBSTONES: usize = 4096;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderSyncProgressPhase {
    Scanning,
    Planning,
    BackingUp,
    Rewriting,
    UpdatingIndexes,
    RollingBack,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSyncProgress {
    pub phase: ProviderSyncProgressPhase,
    pub total_rollout_files: usize,
    pub scanned_rollout_files: usize,
    pub planned_rewrite_files: usize,
    pub applied_rewrite_files: usize,
    pub skipped_locked_rollout_files: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSyncAudit {
    pub catalog_only_sessions: usize,
    pub catalog_only_with_current_rollout: usize,
    pub catalog_only_with_backup_database: usize,
    pub catalog_only_without_recovery_source: usize,
    // issue #982：以下为「可诊断性」字段。用户报「同步一次：0 个会话文件，0 行索引」时，
    // 光看两个 0 分不清是「目录里本来就没有会话文件」还是「筛选条件把它们全漏掉了」。
    // 把实际扫描到的范围一并带出来，下一次报障就能自助定位，不必再索要目录结构。
    /// 本次实际扫描到的 rollout 会话文件数。
    #[serde(default)]
    pub scanned_rollout_files: usize,
    /// 其中 session_meta 的 model_provider 已等于目标、本来就不需要改写的文件数。
    #[serde(default)]
    pub rollout_files_already_on_target: usize,
    /// canonical `threads` 表行数（跨所有候选库去重）。
    #[serde(default)]
    pub canonical_thread_rows: usize,
    /// catalog（用户可见侧边栏索引）行数（跨所有候选库去重）。
    #[serde(default)]
    pub catalog_thread_rows: usize,
    /// 实际存在并被扫描的会话目录名（sessions / archived_sessions）。
    #[serde(default)]
    pub scanned_session_dirs: Vec<String>,
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
    #[serde(default)]
    pub is_resolvable: bool,
    #[serde(default)]
    pub unavailable_reason: Option<String>,
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
    original_text: String,
    next_text: String,
    original_session_meta_lines: Vec<String>,
    rewrite_needed: bool,
    original_mtime: Option<SystemTime>,
}

#[derive(Debug, Default)]
struct RolloutRewrite {
    next_text: String,
    rewrite_needed: bool,
    thread_id: Option<String>,
    providers: Vec<String>,
    original_session_meta_lines: Vec<String>,
    session_meta_count: usize,
}

#[derive(Debug, Default)]
struct SessionChanges {
    changes: Vec<SessionChange>,
    skipped_locked_rollout_files: Vec<PathBuf>,
    encrypted_content_counts: HashMap<String, usize>,
}

#[derive(Debug, Default)]
struct ProviderSyncThreadKinds {
    subagent_thread_ids: HashSet<String>,
    explicit_user_thread_ids: HashSet<String>,
}

#[derive(Debug, Default)]
struct AppliedSessionChanges {
    changes: Vec<SessionChange>,
    skipped_locked_rollout_files: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BulkSessionRewritePlan {
    path: PathBuf,
    original_sha256: String,
    original_mtime: Option<SystemTime>,
    original_session_meta_lines: Vec<String>,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct BulkSessionScan {
    rewrite_plans: Vec<BulkSessionRewritePlan>,
    skipped_locked_rollout_files: Vec<PathBuf>,
    encrypted_content_counts: HashMap<String, usize>,
    subagent_thread_ids: HashSet<String>,
    thread_ids_with_user_events: HashSet<String>,
    cwd_by_thread_id: HashMap<String, String>,
    total_rollout_files: usize,
    /// issue #982：扫描中顺手统计的「已带 session_meta 且 model_provider 已是目标、
    /// 本来就不需要改写」的文件数。用来把「没有会话文件」和「会话文件都已在目标供应商上」
    /// 这两种同样表现为 0 改动的情形区分开。
    rollout_files_already_on_target: usize,
}

#[derive(Debug, Clone)]
struct AppliedBulkSessionRewrite {
    plan: BulkSessionRewritePlan,
    rewritten_sha256: String,
}

#[derive(Debug, Default)]
struct AppliedBulkSessionRewrites {
    changes: Vec<AppliedBulkSessionRewrite>,
    skipped_locked_rollout_files: Vec<PathBuf>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionMetaBackupEntry<'a> {
    path: String,
    original_session_meta_lines: &'a [String],
}

#[derive(Debug, Clone)]
struct SessionIndexPlan {
    path: PathBuf,
    original_bytes: Vec<u8>,
    original_text: String,
}

#[derive(Debug, Clone)]
struct SessionIndexCleanupPlan {
    path: PathBuf,
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
    /// issue #240：补建的 canonical `threads` 行数（只增不删）。
    thread_rows_rebuilt: usize,
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
            + self.thread_rows_rebuilt
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

/// issue #240：只审计、不写盘的预览。
///
/// 用户报「修复历史会话后会话从列表消失」时，需要先看到「这次会动到多少条记录、
/// 备份在哪」，再决定要不要真的执行。这里刻意不做任何写入：不开锁、不改
/// rollout、不碰 sqlite，只复用审计函数统计受影响范围。
///
/// 返回值里的 `backup_root` 是执行同步时备份会落到的目录，供 UI 提示用户保留。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSyncDryRunPreview {
    pub target_provider: String,
    pub audit: ProviderSyncAudit,
    pub backup_root: PathBuf,
}

pub fn preview_provider_sync(codex_home: Option<&Path>) -> anyhow::Result<ProviderSyncDryRunPreview> {
    let home = codex_home
        .map(Path::to_path_buf)
        .unwrap_or_else(default_codex_home_dir);
    let config_path = home.join("config.toml");
    let target_provider =
        match resolve_provider_sync_target_snapshot(&config_path, None) {
            Ok(snapshot) => snapshot.target_provider,
            // 预览不该因为目标解析失败就报错——那会让用户在真正执行前拿不到审计信息。
            // 退回默认 provider，只影响展示。
            Err(_) => DEFAULT_PROVIDER.to_string(),
        };
    let sqlite_paths = provider_sync_db_paths(&home);
    let audit = match audit_provider_sync_state(&home, &sqlite_paths) {
        Ok(audit) => audit,
        Err(error) => {
            let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                "provider_sync.preview_audit_failed",
                json!({ "error": error.to_string() }),
            );
            ProviderSyncAudit::default()
        }
    };
    Ok(ProviderSyncDryRunPreview {
        target_provider,
        audit,
        backup_root: home.join("backups_state/provider-sync"),
    })
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
    let recovery = (|| -> anyhow::Result<ProviderSyncResult> {
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
        let backup_dir = create_backup(&home, target_provider, &rewrite_changes)?;
        let applied = apply_session_changes(&rewrite_changes)?;
        if !rollout_file_matches_provider(&rollout_path, thread_id, target_provider)? {
            let mut deferred = result(
                ProviderSyncStatus::Skipped,
                "Remote Control session finalization deferred for a changed or locked rollout",
                target_provider,
                Some(backup_dir),
                applied.changes.len(),
                0,
            );
            deferred.skipped_locked_rollout_files = applied.skipped_locked_rollout_files;
            return Ok(deferred);
        }
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
            applied.changes.len(),
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
    run_provider_sync_with_target_and_progress(codex_home, explicit_target_provider, |_| {})
}

pub fn run_provider_sync_with_target_and_progress(
    codex_home: Option<&Path>,
    explicit_target_provider: Option<&str>,
    report_progress: impl FnMut(ProviderSyncProgress),
) -> ProviderSyncResult {
    let require_stopped_app = codex_home.is_none();
    let home = codex_home
        .map(Path::to_path_buf)
        .unwrap_or_else(default_codex_home_dir);
    run_provider_sync_with_target_in_home(
        home,
        explicit_target_provider,
        require_stopped_app,
        || {},
        report_progress,
    )
}

fn run_provider_sync_with_target_in_home<BeforeFirstWrite, ReportProgress>(
    home: PathBuf,
    explicit_target_provider: Option<&str>,
    require_stopped_app: bool,
    before_first_write: BeforeFirstWrite,
    mut report_progress: ReportProgress,
) -> ProviderSyncResult
where
    BeforeFirstWrite: FnOnce(),
    ReportProgress: FnMut(ProviderSyncProgress),
{
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
    let config_path = home.join("config.toml");
    let target_snapshot =
        match resolve_provider_sync_target_snapshot(&config_path, explicit_target_provider) {
            Ok(snapshot) => snapshot,
            Err(message) => {
                return result(
                    ProviderSyncStatus::Skipped,
                    message,
                    &safe_explicit_target_provider(explicit_target_provider),
                    None,
                    0,
                    0,
                );
            }
        };
    let target_provider = target_snapshot.target_provider.clone();
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
    let sync_result = (|| -> anyhow::Result<ProviderSyncResult> {
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
        let scan = scan_bulk_session_rewrites(
            &home,
            &target_provider,
            &thread_kinds.subagent_thread_ids,
            &thread_kinds.explicit_user_thread_ids,
            &mut report_progress,
        )?;
        let BulkSessionScan {
            rewrite_plans,
            skipped_locked_rollout_files: scanned_skipped_rollout_files,
            encrypted_content_counts,
            subagent_thread_ids: scanned_subagent_thread_ids,
            thread_ids_with_user_events,
            cwd_by_thread_id: scanned_cwd_by_thread_id,
            total_rollout_files,
            rollout_files_already_on_target: scanned_already_on_target,
        } = scan;
        // issue #982：扫描已经逐个读过这些文件，这里直接复用其结果，不再二次读盘。
        let mut repair_audit = repair_audit;
        repair_audit.scanned_rollout_files = total_rollout_files;
        repair_audit.rollout_files_already_on_target = scanned_already_on_target;
        let mut subagent_thread_ids = thread_kinds.subagent_thread_ids;
        subagent_thread_ids.extend(scanned_subagent_thread_ids);
        let encrypted_content_warning =
            build_encrypted_content_warning(&encrypted_content_counts, &target_provider);
        report_provider_sync_progress(
            &mut report_progress,
            ProviderSyncProgressPhase::Planning,
            total_rollout_files,
            total_rollout_files,
            rewrite_plans.len(),
            0,
            scanned_skipped_rollout_files.len(),
        );
        let projectless_thread_ids =
            load_projectless_thread_ids(&home.join(".codex-global-state.json"))?;
        let mut cwd_by_thread_id = scanned_cwd_by_thread_id;
        cwd_by_thread_id.retain(|thread_id, _| !projectless_thread_ids.contains(thread_id));
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
        // issue #240：待补建的 canonical 行也算「有活要干」，否则会被下面这条
        // 「already up to date」短路掉，明明缺行却不修。
        let canonical_rebuild_count =
            collect_rebuildable_canonical_threads(&home, &sqlite_paths, &target_provider)?.len();
        if rewrite_plans.is_empty()
            && sqlite_update_count == 0
            && catalog_repair_count == 0
            && global_state_update_count == 0
            && canonical_rebuild_count == 0
        {
            revalidate_provider_sync_target_snapshot(
                &config_path,
                explicit_target_provider,
                &target_snapshot,
            )
            .map_err(anyhow::Error::msg)?;
            let mut synced = result(
                ProviderSyncStatus::Synced,
                "Provider sync already up to date",
                &target_provider,
                None,
                0,
                0,
            );
            synced.skipped_locked_rollout_files = scanned_skipped_rollout_files;
            synced.encrypted_content_warning = encrypted_content_warning;
            synced.repair_audit = repair_audit;
            synced.message =
                provider_sync_message_with_audit(&synced.message, &synced.repair_audit);
            report_provider_sync_progress(
                &mut report_progress,
                ProviderSyncProgressPhase::Complete,
                total_rollout_files,
                total_rollout_files,
                0,
                0,
                synced.skipped_locked_rollout_files.len(),
            );
            return Ok(synced);
        }
        before_first_write();
        revalidate_provider_sync_target_snapshot(
            &config_path,
            explicit_target_provider,
            &target_snapshot,
        )
        .map_err(anyhow::Error::msg)?;
        report_provider_sync_progress(
            &mut report_progress,
            ProviderSyncProgressPhase::BackingUp,
            total_rollout_files,
            total_rollout_files,
            rewrite_plans.len(),
            0,
            scanned_skipped_rollout_files.len(),
        );
        let backup_dir = create_bulk_backup(&home, &target_provider, &rewrite_plans)?;
        let applied = apply_bulk_session_rewrite_plans(
            &rewrite_plans,
            &target_provider,
            total_rollout_files,
            scanned_skipped_rollout_files.len(),
            &mut report_progress,
        )?;
        report_provider_sync_progress(
            &mut report_progress,
            ProviderSyncProgressPhase::UpdatingIndexes,
            total_rollout_files,
            total_rollout_files,
            rewrite_plans.len(),
            applied.changes.len(),
            scanned_skipped_rollout_files.len() + applied.skipped_locked_rollout_files.len(),
        );
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
            // issue #240：canonical `threads` 行缺失时补建。
            //
            // 只 INSERT，不 UPDATE、不 DELETE。用户报「修复后会话从列表消失」，
            // 而列表读的是 canonical `threads`；这里补上「rollout 还在、但
            // threads 行没了」的会话，属纯增量加固。
            sqlite_updates.thread_rows_rebuilt =
                rebuild_missing_canonical_thread_rows(&home, &sqlite_paths, &target_provider)?;
            let updated_workspace_roots =
                apply_global_state_update(&home.join(".codex-global-state.json"))?;
            prune_backups(&home)?;
            Ok((sqlite_updates, updated_workspace_roots))
        })();
        let (sqlite_updates, updated_workspace_roots) = match apply_result {
            Ok(counts) => counts,
            Err(err) => {
                report_provider_sync_progress(
                    &mut report_progress,
                    ProviderSyncProgressPhase::RollingBack,
                    total_rollout_files,
                    total_rollout_files,
                    rewrite_plans.len(),
                    applied.changes.len(),
                    scanned_skipped_rollout_files.len()
                        + applied.skipped_locked_rollout_files.len(),
                );
                if let Err(restore_error) = restore_bulk_session_rewrites(&applied.changes) {
                    return Err(anyhow::anyhow!(
                        "{err}; rollout rollback failed: {restore_error}"
                    ));
                }
                return Err(err);
            }
        };
        let mut synced = result(
            ProviderSyncStatus::Synced,
            "Provider sync complete",
            &target_provider,
            Some(backup_dir),
            applied.changes.len(),
            sqlite_updates.total(),
        );
        synced.skipped_locked_rollout_files = scanned_skipped_rollout_files;
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
        report_provider_sync_progress(
            &mut report_progress,
            ProviderSyncProgressPhase::Complete,
            total_rollout_files,
            total_rollout_files,
            rewrite_plans.len(),
            applied.changes.len(),
            synced.skipped_locked_rollout_files.len(),
        );
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

fn report_provider_sync_progress(
    report_progress: &mut dyn FnMut(ProviderSyncProgress),
    phase: ProviderSyncProgressPhase,
    total_rollout_files: usize,
    scanned_rollout_files: usize,
    planned_rewrite_files: usize,
    applied_rewrite_files: usize,
    skipped_locked_rollout_files: usize,
) {
    report_progress(ProviderSyncProgress {
        phase,
        total_rollout_files,
        scanned_rollout_files,
        planned_rewrite_files,
        applied_rewrite_files,
        skipped_locked_rollout_files,
    });
}

/// issue #240：补建缺失的 canonical `threads` 行（只 INSERT）。
///
/// 背景：列表读的是 `threads` 表，而同步此前只会删 `local_thread_catalog` 的投影行，
/// 从不重建 `threads`。一旦某会话只剩投影行，同步后它就彻底从列表消失。
///
/// 这里只处理「rollout 文件仍在」的记录——文件不在的会话即使补行也打不开，
/// 反而制造点开就报错的假条目；那部分留给审计报告，由用户决定怎么处理。
///
/// 安全性：按目标库实际存在的列构造 INSERT，且用 `INSERT OR IGNORE`，
/// 已存在的行不会被覆盖；任何一条失败都不影响其余会话。
fn rebuild_missing_canonical_thread_rows(
    home: &Path,
    sqlite_paths: &[PathBuf],
    target_provider: &str,
) -> anyhow::Result<usize> {
    let mut rebuilt = 0usize;
    for thread in collect_rebuildable_canonical_threads(home, sqlite_paths, target_provider)? {
        for path in sqlite_paths {
            if !path.exists() {
                continue;
            }
            let Ok(mut db) = Connection::open(path) else {
                continue;
            };
            let Ok(columns) = table_columns(&db, "threads") else {
                continue;
            };
            // 只往「本来就该有这些列」的库补行；列不全说明不是目标 schema，跳过。
            if !columns.contains("id") {
                continue;
            }
            let insert_columns = canonical_thread_insert_columns(&columns);
            if !insert_columns.contains(&"id") {
                continue;
            }
            let placeholders = std::iter::repeat_n("?", insert_columns.len())
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!(
                "INSERT OR IGNORE INTO threads ({}) VALUES ({})",
                insert_columns.join(", "),
                placeholders
            );
            let values = canonical_thread_insert_values(&insert_columns, &thread);
            let Ok(tx) = db.transaction() else {
                continue;
            };
            match tx.execute(&sql, params_from_iter(values)) {
                Ok(affected) => {
                    if tx.commit().is_ok() {
                        rebuilt += affected;
                    }
                }
                // 单条失败不影响其余会话；缺列/约束不符是预期内情况。
                Err(_) => continue,
            }
        }
    }
    Ok(rebuilt)
}

/// 找出「catalog 里有、canonical `threads` 里没有、但 rollout 还在」的用户会话。
///
/// 字段直接取自 `local_thread_catalog` 那一行：catalog-only 的会话在 `threads` 里
/// 本来就没有行，不能指望从 `threads` 扫描结果里拿到 title/cwd。
fn collect_rebuildable_canonical_threads(
    home: &Path,
    sqlite_paths: &[PathBuf],
    target_provider: &str,
) -> anyhow::Result<Vec<CatalogRepairThread>> {
    let mut canonical_thread_ids = HashSet::new();
    for path in sqlite_paths {
        canonical_thread_ids.extend(sqlite_table_ids(path, "threads", "id")?);
    }
    let current_rollout_ids = rollout_files(home)?
        .into_iter()
        .filter_map(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .and_then(rollout_thread_id_from_filename)
        })
        .collect::<HashSet<_>>();

    // issue #2199：这是「删了又回来」的另一条重建入口——catalog 里还有行、canonical
    // `threads` 里没有、rollout 文件还在，就会被补回列表。已被删除的会话要排除在外。
    let thread_tombstones = load_thread_tombstones(home);
    let mut rebuildable = Vec::new();
    for path in sqlite_paths {
        for thread in catalog_only_threads_for_path(path, &canonical_thread_ids)? {
            if thread_tombstones.contains(&thread.id) {
                continue;
            }
            if current_rollout_ids.contains(&thread.id) {
                rebuildable.push(CatalogRepairThread {
                    model_provider: target_provider.to_string(),
                    ..thread
                });
            }
        }
    }
    Ok(rebuildable)
}

/// 读出某个库里「canonical `threads` 表没有对应行」的用户会话。
fn catalog_only_threads_for_path(
    path: &Path,
    canonical_thread_ids: &HashSet<String>,
) -> anyhow::Result<Vec<CatalogRepairThread>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let db = Connection::open(path)?;
    let columns = table_columns(&db, "local_thread_catalog")?;
    if !columns.contains("thread_id") {
        return Ok(Vec::new());
    }
    let source_kind = text_expr(&columns, "source_kind", "'cli'");
    let source_detail = text_expr(&columns, "source_detail", "''");
    let thread_source = text_expr(&columns, "thread_source", "NULL");
    let display_title = text_expr(&columns, "display_title", "''");
    let cwd = text_expr(&columns, "cwd", "''");
    let git_branch = text_expr(&columns, "git_branch", "NULL");
    let created_at = text_expr(&columns, "source_created_at", "0");
    let updated_at = text_expr(&columns, "source_updated_at", "0");
    let sql = format!(
        "SELECT thread_id, {display_title}, {created_at}, {updated_at}, {cwd}, \
         {source_kind}, {source_detail}, {git_branch}, {thread_source} \
         FROM local_thread_catalog WHERE COALESCE(thread_id, '') <> ''"
    );
    let mut stmt = db.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1).unwrap_or_default(),
            row.get::<_, f64>(2).unwrap_or_default(),
            row.get::<_, f64>(3).unwrap_or_default(),
            row.get::<_, String>(4).unwrap_or_default(),
            row.get::<_, String>(5).unwrap_or_default(),
            row.get::<_, String>(6).unwrap_or_default(),
            row.get::<_, Option<String>>(7).unwrap_or(None),
            row.get::<_, Option<String>>(8).unwrap_or(None),
        ))
    })?;
    let mut threads = Vec::new();
    for row in rows {
        let (
            thread_id,
            display_title,
            source_created_at,
            source_updated_at,
            cwd,
            source_kind,
            source_detail,
            git_branch,
            thread_source,
        ) = row?;
        if canonical_thread_ids.contains(&thread_id) {
            continue;
        }
        // 子代理/非根线程不补：它们本来就不该出现在用户会话列表里。
        if thread_source_marks_non_root(thread_source.as_deref())
            || source_marks_non_root_agent(&source_kind)
        {
            continue;
        }
        threads.push(CatalogRepairThread {
            id: thread_id,
            display_title,
            source_created_at,
            source_updated_at,
            cwd,
            source_kind,
            source_detail,
            model_provider: String::new(),
            git_branch,
            thread_source,
        });
    }
    Ok(threads)
}

/// 只挑选 `threads` 表真实拥有的列，缺列不补默认值（交给库自身约束）。
fn canonical_thread_insert_columns(columns: &HashSet<String>) -> Vec<&'static str> {
    [
        "id",
        "model_provider",
        "archived",
        "has_user_event",
        "cwd",
        "title",
        "rollout_path",
        "source",
        "created_at_ms",
        "updated_at_ms",
        "git_branch",
        "thread_source",
    ]
    .into_iter()
    .filter(|column| columns.contains(*column))
    .collect()
}

fn canonical_thread_insert_values(
    columns: &[&str],
    thread: &CatalogRepairThread,
) -> Vec<SqlValue> {
    columns
        .iter()
        .map(|column| match *column {
            "id" => SqlValue::Text(thread.id.clone()),
            "model_provider" => SqlValue::Text(thread.model_provider.clone()),
            // archived 一定是 0：只补「能出现在列表里」的会话。
            "archived" => SqlValue::Integer(0),
            "has_user_event" => SqlValue::Integer(1),
            "cwd" => SqlValue::Text(thread.cwd.clone()),
            "title" => SqlValue::Text(thread.display_title.clone()),
            "rollout_path" => SqlValue::Text(thread.source_detail.clone()),
            "source" => SqlValue::Text(thread.source_kind.clone()),
            "created_at_ms" => SqlValue::Real(thread.source_created_at * 1000.0),
            "updated_at_ms" => SqlValue::Real(thread.source_updated_at * 1000.0),
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
            _ => SqlValue::Null,
        })
        .collect()
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
    // issue #982：把本次实际扫描范围附在结果后面。用户看到「0 个会话文件，0 行索引」
    // 时，这条诊断能直接说明是「目录里没有会话文件」还是「有文件但都已就绪」，
    // 以及 canonical/catalog 两侧各有多少行可对齐。
    let scanned_dirs = if audit.scanned_session_dirs.is_empty() {
        "无（sessions 与 archived_sessions 均不存在）".to_string()
    } else {
        audit.scanned_session_dirs.join(", ")
    };
    let diagnostics = format!(
        "本次扫描：会话目录 [{}]，rollout 文件 {} 个（其中 {} 个已指向目标供应商无需改写），canonical 行 {} 条，catalog 行 {} 条。",
        scanned_dirs,
        audit.scanned_rollout_files,
        audit.rollout_files_already_on_target,
        audit.canonical_thread_rows,
        audit.catalog_thread_rows,
    );
    if audit.catalog_only_sessions == 0 {
        return format!("{message}；{diagnostics}");
    }
    format!(
        "{message}；审计发现 {} 条仅存在于本地会话目录的记录，其中 {} 条仍有当前 rollout、{} 条只能在历史数据库备份中找到，{} 条没有可用恢复来源；未自动重建缺失的 canonical 会话（rollout 文件仍在、可安全补建的那些本次已补建）。{diagnostics}",
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

    // issue #982：可诊断字段无论有没有 catalog-only 记录都要填。
    // 原来的实现一进 catalog_only.is_empty() 就返回 default()，于是
    // 「0 个会话文件，0 行索引」背后到底是「目录里真没文件」还是「库是空的」
    // 全都看不出来，正是报障时卡住的那一步。
    let scanned_session_dirs = SESSION_DIRS
        .iter()
        .filter(|dirname| home.join(dirname).exists())
        .map(|dirname| (*dirname).to_string())
        .collect::<Vec<_>>();
    let rollout_paths = rollout_files(home)?;
    let scanned_rollout_files = rollout_paths.len();
    let current_rollout_ids = rollout_paths
        .iter()
        .filter_map(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .and_then(rollout_thread_id_from_filename)
        })
        .collect::<HashSet<_>>();
    let mut audit = ProviderSyncAudit {
        scanned_rollout_files,
        rollout_files_already_on_target: 0,
        canonical_thread_rows: canonical_thread_ids.len(),
        catalog_thread_rows: catalog_thread_ids.len(),
        scanned_session_dirs,
        ..ProviderSyncAudit::default()
    };

    let catalog_only = catalog_thread_ids
        .difference(&canonical_thread_ids)
        .cloned()
        .collect::<HashSet<_>>();
    if catalog_only.is_empty() {
        return Ok(audit);
    }

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

    audit.catalog_only_sessions = catalog_only.len();
    audit.catalog_only_with_current_rollout = with_current_rollout;
    audit.catalog_only_with_backup_database = with_backup_database;
    audit.catalog_only_without_recovery_source = catalog_only
        .iter()
        .filter(|thread_id| {
            !current_rollout_ids.contains(*thread_id) && !backup_database_ids.contains(*thread_id)
        })
        .count();
    Ok(audit)
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

#[derive(Debug, Clone)]
struct LiveProviderConfig {
    current_provider: Option<String>,
    configured_provider_ids: HashSet<String>,
    provider_table_ids: HashSet<String>,
}

impl LiveProviderConfig {
    fn is_provider_resolvable(&self, provider: &str) -> bool {
        is_valid_explicit_provider_id(provider)
            && (provider == DEFAULT_PROVIDER || self.provider_table_ids.contains(provider))
    }

    fn validate_current_provider(&self) -> Result<(), String> {
        let Some(current_provider) = self.current_provider.as_deref() else {
            return Err(
                "Current provider identity is missing or invalid in live config.toml".to_string(),
            );
        };
        if !self.is_provider_resolvable(current_provider) {
            return Err(
                "Current provider is not resolvable from an exact live model provider table"
                    .to_string(),
            );
        }
        Ok(())
    }
}

/// Secret-free provider identity inputs used to detect a config switch while history is scanned.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ProviderSyncTargetSnapshot {
    target_provider: String,
    current_provider: Option<String>,
}

/// Resolves a repair target from live config without modifying configuration or history.
pub fn validate_provider_sync_target(
    codex_home: Option<&Path>,
    explicit_target_provider: Option<&str>,
) -> Result<String, String> {
    let home = codex_home
        .map(Path::to_path_buf)
        .unwrap_or_else(default_codex_home_dir);
    if !home.exists() {
        return Err(format!("Codex home not found: {}", home.to_string_lossy()));
    }
    resolve_provider_sync_target_snapshot(&home.join("config.toml"), explicit_target_provider)
        .map(|snapshot| snapshot.target_provider)
}

pub fn load_provider_sync_targets(codex_home: Option<&Path>) -> ProviderSyncTargetList {
    let home = codex_home
        .map(Path::to_path_buf)
        .unwrap_or_else(default_codex_home_dir);
    let config_path = home.join("config.toml");
    let live_config = load_live_provider_config(&config_path);
    let current_provider = match &live_config {
        Ok(config) => config.current_provider.clone().unwrap_or_default(),
        Err(_) => read_current_provider(&config_path),
    };
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
        live_config
            .as_ref()
            .map(|config| sorted_provider_ids(config.configured_provider_ids.clone()))
            .unwrap_or_else(|_| list_configured_provider_ids(&config_path)),
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
            let (is_resolvable, unavailable_reason) =
                provider_resolution_for_discovery(&live_config, &id);
            ProviderSyncTargetOption {
                is_current_provider: id == current_provider,
                is_manual: source_list.contains(&ProviderSyncTargetSource::Manual),
                is_saved: false,
                is_resolvable,
                unavailable_reason,
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
        return String::new();
    };
    let provider = root_toml_string_value(&text, "model_provider").unwrap_or_default();
    let provider = provider.trim();
    is_valid_explicit_provider_id(provider)
        .then(|| provider.to_string())
        .unwrap_or_default()
}

fn resolve_provider_sync_target_snapshot(
    config_path: &Path,
    explicit_target_provider: Option<&str>,
) -> Result<ProviderSyncTargetSnapshot, String> {
    let explicit_target_provider = explicit_target_provider
        .map(str::trim)
        .filter(|provider| !provider.is_empty());
    if let Some(trimmed) = explicit_target_provider {
        if !is_valid_explicit_provider_id(trimmed) {
            return Err("Invalid provider sync target identity".to_string());
        }
    }

    let live_config = load_live_provider_config(config_path)?;
    let target_provider = match explicit_target_provider {
        Some(provider) => provider,
        None => {
            live_config.validate_current_provider()?;
            live_config.current_provider.as_deref().ok_or_else(|| {
                "Current provider identity is missing or invalid in live config.toml".to_string()
            })?
        }
    };
    if !live_config.is_provider_resolvable(target_provider) {
        return Err(format!(
            "Provider sync target {target_provider:?} is not resolvable from an exact live model provider table"
        ));
    }

    Ok(ProviderSyncTargetSnapshot {
        target_provider: target_provider.to_string(),
        current_provider: live_config.current_provider,
    })
}

fn safe_explicit_target_provider(explicit_target_provider: Option<&str>) -> String {
    explicit_target_provider
        .map(str::trim)
        .filter(|provider| is_valid_explicit_provider_id(provider))
        .unwrap_or_default()
        .to_string()
}

fn revalidate_provider_sync_target_snapshot(
    config_path: &Path,
    explicit_target_provider: Option<&str>,
    expected: &ProviderSyncTargetSnapshot,
) -> Result<(), String> {
    let current = resolve_provider_sync_target_snapshot(config_path, explicit_target_provider)
        .map_err(|_| {
            "Live provider configuration changed or became unavailable during provider sync; retry"
                .to_string()
        })?;
    if &current != expected {
        return Err(
            "Live provider configuration changed or became unavailable during provider sync; retry"
                .to_string(),
        );
    }
    Ok(())
}

fn is_valid_explicit_provider_id(value: &str) -> bool {
    !value.is_empty() && value.trim() == value && !value.chars().any(char::is_control)
}

fn load_live_provider_config(path: &Path) -> Result<LiveProviderConfig, String> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err("Live config.toml was not found".to_string());
        }
        Err(_) => return Err("Live config.toml could not be read safely".to_string()),
    };
    let root = toml::from_str::<toml::Table>(&text)
        .map_err(|_| "Live config.toml could not be parsed safely".to_string())?;
    let current_provider = match root.get("model_provider") {
        None => Some(DEFAULT_PROVIDER.to_string()),
        Some(value) => value
            .as_str()
            .filter(|provider| is_valid_explicit_provider_id(provider))
            .map(ToString::to_string),
    };

    let mut configured_provider_ids = HashSet::from([DEFAULT_PROVIDER.to_string()]);
    let mut provider_table_ids = HashSet::new();
    if let Some(value) = root.get("model_providers") {
        let providers = value
            .as_table()
            .ok_or_else(|| "Live model provider mappings are invalid".to_string())?;
        for (provider, mapping) in providers {
            configured_provider_ids.insert(provider.clone());
            if mapping.is_table() {
                provider_table_ids.insert(provider.clone());
            }
        }
    }

    Ok(LiveProviderConfig {
        current_provider,
        configured_provider_ids,
        provider_table_ids,
    })
}

fn provider_resolution_for_discovery(
    live_config: &Result<LiveProviderConfig, String>,
    provider: &str,
) -> (bool, Option<String>) {
    let config = match live_config {
        Ok(config) => config,
        Err(message) => return (false, Some(message.clone())),
    };
    if config.is_provider_resolvable(provider) {
        (true, None)
    } else {
        (
            false,
            Some(
                "Provider is visible in history but has no exact live model provider table"
                    .to_string(),
            ),
        )
    }
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

fn scan_bulk_session_rewrites(
    home: &Path,
    target_provider: &str,
    excluded_thread_ids: &HashSet<String>,
    explicit_user_thread_ids: &HashSet<String>,
    report_progress: &mut dyn FnMut(ProviderSyncProgress),
) -> anyhow::Result<BulkSessionScan> {
    scan_bulk_session_rewrites_with_parser(
        home,
        target_provider,
        excluded_thread_ids,
        explicit_user_thread_ids,
        report_progress,
        parse_session_meta_record,
        false,
    )
}

#[derive(Deserialize)]
struct BulkRolloutEnvelope<'a> {
    #[serde(rename = "type", borrow)]
    kind: Option<&'a serde_json::value::RawValue>,
    #[serde(rename = "payload", borrow)]
    _payload: Option<&'a serde_json::value::RawValue>,
}

fn parse_session_meta_record(line: &str) -> Option<Value> {
    // Validate JSON without materializing large response/tool payloads. Escaped keys,
    // arbitrary field order and duplicate-key last-value semantics remain supported.
    let envelope = match serde_json::from_str::<BulkRolloutEnvelope<'_>>(line) {
        Ok(envelope) => envelope,
        // Derived structs reject duplicate fields, whereas Value keeps the last.
        Err(_) => return serde_json::from_str::<Value>(line).ok()
            .filter(|record| record["type"].as_str() == Some("session_meta")),
    };
    let kind = envelope.kind?.get().trim();
    let metadata = kind == "\"session_meta\""
        || (kind.contains('\\')
            && serde_json::from_str::<String>(kind).ok().as_deref() == Some("session_meta"));
    if !metadata {
        return None;
    }
    serde_json::from_str(line).ok()
}

#[derive(Debug, Default, PartialEq, Eq)]
struct BulkRolloutFacts {
    session_meta_count: usize,
    thread_id: Option<String>,
    cwd: Option<String>,
    providers: Vec<String>,
    original_session_meta_lines: Vec<String>,
    rewrite_needed: bool,
    rollout_marks_non_root_agent: bool,
    has_user_event: bool,
    has_encrypted_content: bool,
}

fn scan_bulk_rollout(
    reader: &mut impl BufRead,
    target_provider: &str,
    parse_record: &impl Fn(&str) -> Option<Value>,
    hash_source: bool,
) -> anyhow::Result<(BulkRolloutFacts, Option<String>)> {
    let mut facts = BulkRolloutFacts::default();
    let mut line = String::new();
    let mut hasher = hash_source.then(Sha256::new);
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        if let Some(hasher) = &mut hasher {
            hasher.update(line.as_bytes());
        }
        facts.has_user_event = facts.has_user_event
            || line.contains("\"user_message\"") || line.contains("\"user_input\"");
        facts.has_encrypted_content = facts.has_encrypted_content || line.contains("encrypted_content");
        let (record_line, _) = split_line_ending(&line);
        if record_line.trim().is_empty() {
            continue;
        }
        let Some(record) = parse_record(record_line) else {
            continue;
        };
        if record.get("type").and_then(Value::as_str) != Some("session_meta") {
            continue;
        }
        let Some(payload) = record.get("payload").and_then(Value::as_object) else {
            continue;
        };
        facts.session_meta_count += 1;
        facts.original_session_meta_lines.push(record_line.to_string());
        if facts.thread_id.is_none() {
            facts.thread_id = payload.get("id").and_then(Value::as_str).map(ToString::to_string);
        }
        if facts.cwd.is_none() {
            facts.cwd = payload.get("cwd").and_then(Value::as_str).and_then(to_desktop_workspace_path);
        }
        facts.providers.push(
            payload.get("model_provider").and_then(Value::as_str).unwrap_or("(missing)").to_string(),
        );
        facts.rewrite_needed |=
            payload.get("model_provider").and_then(Value::as_str) != Some(target_provider);
        facts.rollout_marks_non_root_agent |=
            payload.get("source").is_some_and(source_value_marks_non_root_agent);
    }
    Ok((facts, hasher.map(|hasher| format!("{:x}", hasher.finalize()))))
}

fn hash_bulk_rollout_plan(
    reader: &mut (impl BufRead + Seek),
    target_provider: &str,
    parse_record: &impl Fn(&str) -> Option<Value>,
    expected: &BulkRolloutFacts,
) -> anyhow::Result<String> {
    // A write plan still needs an exact preimage. Rewind the same handle and
    // revalidate all scan facts while hashing, including metadata late in a file.
    reader.seek(SeekFrom::Start(0))?;
    let (actual, digest) = scan_bulk_rollout(reader, target_provider, parse_record, true)?;
    anyhow::ensure!(actual == *expected, "Rollout changed while planning provider sync");
    digest.ok_or_else(|| anyhow::anyhow!("Missing rollout source hash"))
}

fn scan_bulk_session_rewrites_with_parser(
    home: &Path,
    target_provider: &str,
    excluded_thread_ids: &HashSet<String>,
    explicit_user_thread_ids: &HashSet<String>,
    report_progress: &mut dyn FnMut(ProviderSyncProgress),
    parse_record: impl Fn(&str) -> Option<Value>,
    hash_all_sources: bool,
) -> anyhow::Result<BulkSessionScan> {
    let paths = rollout_files(home)?;
    let mut scan = BulkSessionScan {
        total_rollout_files: paths.len(),
        ..Default::default()
    };
    report_provider_sync_progress(
        report_progress,
        ProviderSyncProgressPhase::Scanning,
        scan.total_rollout_files,
        0,
        0,
        0,
        0,
    );
    for (index, path) in paths.iter().enumerate() {
        let scanned_rollout_files = index + 1;
        let file = match File::open(path) {
            Ok(file) => file,
            Err(error) if is_locked_io_error(&error) => {
                scan.skipped_locked_rollout_files.push(path.clone());
                report_scan_progress(
                    report_progress,
                    scan.total_rollout_files,
                    scanned_rollout_files,
                    scan.skipped_locked_rollout_files.len(),
                );
                continue;
            }
            Err(error) => return Err(error.into()),
        };

        let mut reader = BufReader::with_capacity(64 * 1024, file);
        let (facts, mut original_sha256) =
            scan_bulk_rollout(&mut reader, target_provider, &parse_record, hash_all_sources)?;
        let is_explicit_user = facts.thread_id
            .as_ref()
            .is_some_and(|id| explicit_user_thread_ids.contains(id));
        let excluded = facts.rollout_marks_non_root_agent
            || (!is_explicit_user && facts.thread_id
                .as_ref()
                .is_some_and(|id| excluded_thread_ids.contains(id)));
        if facts.rewrite_needed && !excluded && original_sha256.is_none() {
            original_sha256 = Some(hash_bulk_rollout_plan(
                &mut reader, target_provider, &parse_record, &facts,
            )?);
        }
        let BulkRolloutFacts {
            session_meta_count, thread_id, cwd, providers, original_session_meta_lines,
            rewrite_needed, rollout_marks_non_root_agent, has_user_event, has_encrypted_content,
        } = facts;

        if session_meta_count > 0 {
            // issue #982：带 session_meta 且无需改写的文件计入「已就绪」，仅用于诊断。
            // 放在子任务/显式用户筛选之前，表示「这个文件本身已经是目标供应商」，
            // 与它是否参与本次改写无关。
            if !rewrite_needed {
                scan.rollout_files_already_on_target += 1;
            }
            if rollout_marks_non_root_agent {
                if let Some(thread_id) = thread_id {
                    scan.subagent_thread_ids.insert(thread_id);
                }
            } else if !is_explicit_user
                && thread_id
                    .as_ref()
                    .is_some_and(|id| excluded_thread_ids.contains(id))
            {
                // Keep the existing SQLite subagent exclusion behavior.
            } else {
                if has_user_event {
                    if let Some(thread_id) = &thread_id {
                        scan.thread_ids_with_user_events.insert(thread_id.clone());
                    }
                }
                if let (Some(thread_id), Some(cwd)) = (thread_id.as_ref(), cwd) {
                    scan.cwd_by_thread_id.insert(thread_id.clone(), cwd);
                }
                if has_encrypted_content {
                    for provider in providers {
                        *scan.encrypted_content_counts.entry(provider).or_insert(0) += 1;
                    }
                }
                if rewrite_needed {
                    scan.rewrite_plans.push(BulkSessionRewritePlan {
                        path: path.clone(),
                        original_sha256: original_sha256
                            .ok_or_else(|| anyhow::anyhow!("Missing rollout source hash"))?,
                        original_mtime: fs::metadata(path)
                            .and_then(|metadata| metadata.modified())
                            .ok(),
                        original_session_meta_lines,
                    });
                }
            }
        }
        report_scan_progress(
            report_progress,
            scan.total_rollout_files,
            scanned_rollout_files,
            scan.skipped_locked_rollout_files.len(),
        );
    }
    Ok(scan)
}

fn report_scan_progress(
    report_progress: &mut dyn FnMut(ProviderSyncProgress),
    total_rollout_files: usize,
    scanned_rollout_files: usize,
    skipped_locked_rollout_files: usize,
) {
    if scanned_rollout_files == 1
        || scanned_rollout_files == total_rollout_files
        || scanned_rollout_files % PROVIDER_SYNC_PROGRESS_INTERVAL == 0
    {
        report_provider_sync_progress(
            report_progress,
            ProviderSyncProgressPhase::Scanning,
            total_rollout_files,
            scanned_rollout_files,
            0,
            0,
            skipped_locked_rollout_files,
        );
    }
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
    let sessions_root = fs::canonicalize(home.join("sessions")).ok()?;
    // issue #1424：WSL 智能体环境下 codex 写进 threads.rollout_path 的是 WSL 视角
    // 路径（/mnt/c/...），Windows 侧进程按原样读不到；删除链路早已做视角互转（#162），
    // 会话修复这里也要回退一次，否则 WSL 下找不到 rollout 直接判定会话不可修复。
    for candidate in rollout_path_candidates(home, raw) {
        let Some(canonical) = fs::canonicalize(&candidate).ok() else {
            continue;
        };
        if canonical.starts_with(&sessions_root) {
            return Some(canonical);
        }
    }
    None
}

/// 按「原样」再「另一种视角」的顺序给出 rollout 路径候选（issue #1424）。
fn rollout_path_candidates(home: &Path, raw: &str) -> Vec<PathBuf> {
    let raw = raw.trim();
    let mut candidates = Vec::new();
    for view in [Some(raw.to_string()), crate::storage::wsl_path_alternative(raw)]
        .into_iter()
        .flatten()
    {
        let path = PathBuf::from(&view);
        let path = if rollout_view_is_absolute(&view) {
            path
        } else {
            home.join(path)
        };
        if !candidates.contains(&path) {
            candidates.push(path);
        }
    }
    candidates
}

/// 判断某一路径视角是否本身就是绝对路径（issue #1424）。
///
/// 不能直接用 `Path::is_absolute()`：它按**当前宿主**的规则判断，而这里两种视角
/// 天生来自不同宿主——`C:/...` 在 Unix 上不算绝对、`/mnt/c/...` 在 Windows 上不算绝对，
/// 用宿主规则会把另一种视角的绝对路径错误地拼到 home 后面。这两种形式各自在自己的
/// 环境里都是绝对路径，这里显式认出来。
fn rollout_view_is_absolute(view: &str) -> bool {
    let normalized = view.replace('\\', "/");
    let bytes = normalized.as_bytes();
    let drive_prefixed = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && bytes[2] == b'/';
    drive_prefixed || normalized.starts_with("/") || Path::new(view).is_absolute()
}

fn rollout_provider_state_for_path(
    path: &Path,
) -> anyhow::Result<Option<(String, HashSet<String>)>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if is_locked_io_error(&error) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    Ok(rollout_thread_provider_state(&text))
}

fn collect_session_change_for_path(
    path: &Path,
    target_provider: &str,
    source_provider: &str,
    thread_id: &str,
) -> anyhow::Result<SessionChanges> {
    let mut collected = SessionChanges::default();
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if is_locked_io_error(&error) => {
            collected
                .skipped_locked_rollout_files
                .push(path.to_path_buf());
            return Ok(collected);
        }
        Err(error) => return Err(error.into()),
    };
    let rewrite = rewrite_rollout_session_meta_providers_for_threads(
        &text,
        target_provider,
        source_provider,
        &HashSet::from([thread_id.to_string()]),
    )?;
    if rewrite.session_meta_count == 0 || rewrite.thread_id.as_deref() != Some(thread_id) {
        return Ok(collected);
    }
    if text.contains("encrypted_content") {
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
        original_text: text,
        next_text: rewrite.next_text,
        original_session_meta_lines: rewrite.original_session_meta_lines,
        rewrite_needed: rewrite.rewrite_needed,
        original_mtime,
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

fn rewrite_rollout_session_meta_providers_for_threads(
    text: &str,
    target_provider: &str,
    source_provider: &str,
    thread_ids: &HashSet<String>,
) -> anyhow::Result<RolloutRewrite> {
    let rollout_thread_id = text.lines().find_map(|line| {
        let record = serde_json::from_str::<Value>(line).ok()?;
        if record.get("type").and_then(Value::as_str) != Some("session_meta") {
            return None;
        }
        record
            .get("payload")?
            .get("id")?
            .as_str()
            .map(ToString::to_string)
    });
    if rollout_thread_id
        .as_ref()
        .is_none_or(|thread_id| !thread_ids.contains(thread_id))
    {
        return Ok(RolloutRewrite {
            next_text: text.to_string(),
            ..RolloutRewrite::default()
        });
    }

    let mut rewrite = RolloutRewrite {
        thread_id: rollout_thread_id,
        ..RolloutRewrite::default()
    };
    for segment in text.split_inclusive('\n') {
        let (line, line_ending) = split_line_ending(segment);
        let mut next_line = line.to_string();
        if !line.trim().is_empty() {
            if let Ok(mut record) = serde_json::from_str::<Value>(line) {
                if record.get("type").and_then(Value::as_str) == Some("session_meta") {
                    let Some(payload) = record.get_mut("payload").and_then(Value::as_object_mut)
                    else {
                        rewrite.next_text.push_str(&next_line);
                        rewrite.next_text.push_str(line_ending);
                        continue;
                    };
                    rewrite.session_meta_count += 1;
                    rewrite.original_session_meta_lines.push(line.to_string());
                    let provider = payload
                        .get("model_provider")
                        .and_then(Value::as_str)
                        .map(ToString::to_string);
                    rewrite
                        .providers
                        .push(provider.clone().unwrap_or_else(|| "(missing)".to_string()));
                    if provider
                        .as_deref()
                        .is_none_or(|provider| provider == source_provider)
                    {
                        payload.insert("model_provider".to_string(), json!(target_provider));
                        next_line = serde_json::to_string(&record)?;
                        rewrite.rewrite_needed = true;
                    }
                }
            }
        }
        rewrite.next_text.push_str(&next_line);
        rewrite.next_text.push_str(line_ending);
    }
    Ok(rewrite)
}

fn rollout_files(home: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for dirname in SESSION_DIRS {
        let root = home.join(dirname);
        if root.exists() {
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
        let file = match File::open(&path) {
            Ok(file) => file,
            Err(error) if is_locked_io_error(&error) => continue,
            Err(error) => return Err(error.into()),
        };
        let mut reader = BufReader::new(file);
        let mut line = String::new();
        loop {
            line.clear();
            if reader.read_line(&mut line)? == 0 {
                break;
            }
            let (line, _) = split_line_ending(&line);
            let Ok(record) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if record.get("type").and_then(Value::as_str) != Some("session_meta") {
                continue;
            }
            if let Some(id) = record
                .get("payload")
                .and_then(Value::as_object)
                .and_then(|payload| payload.get("id"))
                .and_then(Value::as_str)
                .filter(|id| !id.trim().is_empty())
            {
                ids.insert(id.to_string());
            }
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
) -> anyhow::Result<Option<SessionIndexCleanupPlan>> {
    if !path.exists() {
        return Ok(None);
    }
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut hasher = Sha256::new();
    let mut candidates = Vec::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        hasher.update(line.as_bytes());
        let (line, _) = split_line_ending(&line);
        if let Some(candidate) = known_session_index_candidate(line)
            && !live_thread_ids.contains(&candidate.id)
        {
            candidates.push(candidate);
        }
    }
    Ok(Some(SessionIndexCleanupPlan {
        path: path.to_path_buf(),
        snapshot_sha256: format!("{:x}", hasher.finalize()),
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
        let removed_entries = plan
            .candidates
            .iter()
            .filter(|candidate| selected_ids.contains(&candidate.id))
            .count();
        if removed_entries == 0 {
            return Ok(SessionIndexCleanupResult {
                pruned_entries: 0,
                backup_dir: None,
            });
        }
        let backup_dir = create_session_index_cleanup_backup(&home, &plan, removed_entries)?;
        if require_stopped_app {
            ensure_codex_app_stopped(Some(backup_dir.clone()))?;
        }
        let mut streamed_removed_entries = None;
        let write_result = codex_plus_core::settings::atomic_write_with(&plan.path, |file| {
            let mut writer = BufWriter::new(file);
            let removed = stream_filtered_session_index(
                &plan.path,
                &plan.snapshot_sha256,
                &selected_ids,
                &mut writer,
            )?;
            writer.flush()?;
            streamed_removed_entries = Some(removed);
            Ok(())
        });
        if let Err(error) = write_result {
            if error_chain_contains(&error, SESSION_INDEX_SNAPSHOT_CHANGED_ERROR) {
                return Err(cleanup_apply_error(
                    "session_index.jsonl 在写入前再次发生变化；未覆盖 Codex 新内容，请重新预览",
                    Some(backup_dir),
                ));
            }
            return Err(cleanup_apply_error(
                format!(
                    "原子写入 session_index.jsonl 失败；原文件未被主动覆盖，可从备份目录手动恢复：{error}"
                ),
                Some(backup_dir),
            ));
        }
        let _ = prune_backups(&home);
        Ok(SessionIndexCleanupResult {
            pruned_entries: streamed_removed_entries.unwrap_or(0),
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
pub fn remove_session_index_entry(
    codex_home: &Path,
    thread_id: &str,
) -> anyhow::Result<usize> {
    let path = codex_home.join("session_index.jsonl");
    if !path.exists() {
        return Ok(0);
    }
    let original_bytes = fs::read(&path)?;
    let original_text = String::from_utf8(original_bytes.clone())?;
    let plan = SessionIndexPlan {
        path,
        original_bytes,
        original_text,
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

/// 删除 Codex 侧边栏对线程的本地引用。
///
/// 线程正文由 `state_5.sqlite`/rollout 文件保存，而侧边栏还会从全局状态和
/// `sqlite/codex-dev.db` 的目录缓存读取条目。删除正文时必须同步清理这些缓存，
/// 否则重启后仍会显示一个无法恢复的“幽灵会话”。该操作按 thread id 精确匹配，
/// 可重复执行；缓存不存在或目标不存在均视为成功。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ThreadSidebarCleanupResult {
    pub global_state_entries_removed: usize,
    pub catalog_rows_removed: usize,
}

/// Capture the thread-specific sidebar data that is about to be removed.
/// The snapshot is embedded in the normal delete backup so undo can restore only
/// this thread's entries without replacing unrelated global state.
pub fn snapshot_thread_sidebar_references(
    codex_home: &Path,
    thread_id: &str,
) -> anyhow::Result<Value> {
    let thread_id = thread_id.strip_prefix("local:").unwrap_or(thread_id);
    let global_state = snapshot_thread_from_global_state(codex_home, thread_id)?;
    let catalog = snapshot_thread_from_catalog_dbs(codex_home, thread_id)?;
    Ok(json!({
        "thread_id": thread_id,
        "global_state": global_state,
        "catalog": catalog,
    }))
}

/// Restore a previously captured sidebar snapshot. Existing values are kept so
/// an undo cannot overwrite changes made after deletion.
pub fn restore_thread_sidebar_references(
    codex_home: &Path,
    snapshot: &Value,
) -> anyhow::Result<usize> {
    validate_thread_sidebar_snapshot(codex_home, snapshot)?;
    let thread_id = snapshot
        .get("thread_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut restored = restore_thread_to_global_state(codex_home, thread_id, snapshot)?;
    restored += restore_thread_to_catalog_dbs(codex_home, thread_id, snapshot)?;
    // issue #2199：撤销删除要连墓碑一起撤，否则恢复的会话在下次修复时又被跳过。
    // 放在恢复之后：恢复本身可能失败，但用户的「恢复」意图已经明确表达。
    if !thread_id.is_empty() {
        clear_thread_tombstones(
            codex_home,
            &HashSet::from([thread_id.strip_prefix("local:").unwrap_or(thread_id).to_string()]),
        );
    }
    Ok(restored)
}

pub fn validate_thread_sidebar_snapshot(
    codex_home: &Path,
    snapshot: &Value,
) -> anyhow::Result<()> {
    let thread_id = snapshot
        .get("thread_id")
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("sidebar snapshot is missing thread_id"))?;
    if let Some(catalog) = snapshot.get("catalog") {
        let entries = catalog
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("sidebar snapshot catalog must be an array"))?;
        let allowed_paths = sidebar_catalog_db_paths(codex_home)?;
        for entry in entries {
            let table = entry
                .get("table")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("sidebar catalog entry is missing table"))?;
            if !SIDEBAR_CATALOG_TABLES.contains(&table) {
                anyhow::bail!("unsupported sidebar catalog table: {table}");
            }
            let path = entry
                .get("db_path")
                .and_then(Value::as_str)
                .map(PathBuf::from)
                .ok_or_else(|| anyhow::anyhow!("sidebar catalog entry is missing db_path"))?;
            let canonical = fs::canonicalize(&path)?;
            if !allowed_paths.contains(&canonical) {
                anyhow::bail!("sidebar catalog database is not an allowed Codex database");
            }
            let rows = entry
                .get("rows")
                .and_then(Value::as_array)
                .ok_or_else(|| anyhow::anyhow!("sidebar catalog entry rows must be an array"))?;
            for row in rows {
                let row_id = row
                    .get("thread_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("sidebar catalog row is missing thread_id"))?;
                if row_id != thread_id {
                    anyhow::bail!("sidebar catalog row thread_id does not match snapshot");
                }
            }
        }
    }
    Ok(())
}

const SIDEBAR_CATALOG_TABLES: [&str; 3] = [
    "local_thread_catalog",
    "thread_timeline_ledger",
    "local_thread_catalog_scan_entries",
];

fn snapshot_thread_from_global_state(
    codex_home: &Path,
    thread_id: &str,
) -> anyhow::Result<Value> {
    let path = codex_home.join(".codex-global-state.json");
    if !path.exists() {
        return Ok(json!({}));
    }
    let value: Value = serde_json::from_slice(&fs::read(path)?)?;
    let Some(root) = value.as_object() else {
        return Ok(json!({}));
    };
    let mut snapshot = Map::new();
    if let Some(ids) = root.get("projectless-thread-ids").and_then(Value::as_array) {
        let values = ids
            .iter()
            .filter(|value| thread_value_matches(value, thread_id))
            .cloned()
            .collect::<Vec<_>>();
        if !values.is_empty() {
            snapshot.insert("projectless-thread-ids".to_string(), Value::Array(values));
        }
    }
    let mut maps = Map::new();
    for key in [
        "thread-projectless-output-directories",
        "thread-workspace-root-hints",
        "thread-writable-roots",
    ] {
        if let Some(map) = root.get(key).and_then(Value::as_object) {
            let entries = map
                .iter()
                .filter(|(key, _)| thread_key_matches(key, thread_id))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect::<Map<_, _>>();
            if !entries.is_empty() {
                maps.insert(key.to_string(), Value::Object(entries));
            }
        }
    }
    if !maps.is_empty() {
        snapshot.insert("thread_maps".to_string(), Value::Object(maps));
    }
    if let Some(atom) = root
        .get("electron-persisted-atom-state")
        .and_then(Value::as_object)
    {
        let entries = atom
            .iter()
            .filter(|(key, _)| sidebar_atom_key_matches(key, thread_id))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<Map<_, _>>();
        if !entries.is_empty() {
            snapshot.insert("atom_entries".to_string(), Value::Object(entries));
        }
    }
    Ok(Value::Object(snapshot))
}

fn snapshot_thread_from_catalog_dbs(
    codex_home: &Path,
    thread_id: &str,
) -> anyhow::Result<Value> {
    let mut entries = Vec::new();
    for path in codex_plus_core::codex_sqlite::codex_thread_reference_db_paths_from_home(codex_home)
    {
        if !path.exists() {
            continue;
        }
        let db = Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        for table in SIDEBAR_CATALOG_TABLES {
            let columns = table_columns(&db, table)?;
            if !columns.contains("thread_id") {
                continue;
            }
            let rows = select_dicts(
                &db,
                &format!("SELECT * FROM {table} WHERE thread_id = ?1"),
                &[&thread_id],
            )?;
            if !rows.is_empty() {
                entries.push(json!({
                    "db_path": path.to_string_lossy(),
                    "table": table,
                    "rows": rows,
                }));
            }
        }
    }
    Ok(Value::Array(entries))
}

fn restore_thread_to_global_state(
    codex_home: &Path,
    _thread_id: &str,
    snapshot: &Value,
) -> anyhow::Result<usize> {
    let global = snapshot.get("global_state").unwrap_or(&Value::Null);
    if !global.is_object() {
        return Ok(0);
    }
    let path = codex_home.join(".codex-global-state.json");
    let original_bytes = if path.exists() {
        fs::read(&path)?
    } else {
        Vec::new()
    };
    let mut state = if original_bytes.is_empty() {
        json!({})
    } else {
        serde_json::from_slice::<Value>(&original_bytes)?
    };
    let Some(root) = state.as_object_mut() else {
        return Ok(0);
    };
    let mut restored = 0usize;
    if let Some(values) = global
        .get("projectless-thread-ids")
        .and_then(Value::as_array)
    {
        let ids = root
            .entry("projectless-thread-ids")
            .or_insert_with(|| Value::Array(Vec::new()));
        if let Some(ids) = ids.as_array_mut() {
            for value in values {
                if !ids.iter().any(|existing| existing == value) {
                    ids.push(value.clone());
                    restored += 1;
                }
            }
        }
    }
    if let Some(maps) = global.get("thread_maps").and_then(Value::as_object) {
        restored += restore_missing_map_entries(root, maps);
    }
    if let Some(entries) = global.get("atom_entries").and_then(Value::as_object) {
        let atom = root
            .entry("electron-persisted-atom-state")
            .or_insert_with(|| Value::Object(Map::new()));
        if let Some(atom) = atom.as_object_mut() {
            for (key, value) in entries {
                if !atom.contains_key(key) {
                    atom.insert(key.clone(), value.clone());
                    restored += 1;
                }
            }
        }
    }
    if restored == 0 {
        return Ok(0);
    }
    if path.exists() && fs::read(&path)? != original_bytes {
        anyhow::bail!(".codex-global-state.json changed while restoring sidebar state");
    }
    codex_plus_core::settings::atomic_write(&path, serde_json::to_string_pretty(&state)?.as_bytes())?;
    Ok(restored)
}

fn restore_missing_map_entries(root: &mut Map<String, Value>, maps: &Map<String, Value>) -> usize {
    let mut restored = 0usize;
    for (key, entries) in maps {
        let value = root
            .entry(key.clone())
            .or_insert_with(|| Value::Object(Map::new()));
        if let (Some(target), Some(entries)) = (value.as_object_mut(), entries.as_object()) {
            for (entry_key, entry_value) in entries {
                if !target.contains_key(entry_key) {
                    target.insert(entry_key.clone(), entry_value.clone());
                    restored += 1;
                }
            }
        }
    }
    restored
}

fn restore_thread_to_catalog_dbs(
    codex_home: &Path,
    _thread_id: &str,
    snapshot: &Value,
) -> anyhow::Result<usize> {
    let Some(entries) = snapshot.get("catalog").and_then(Value::as_array) else {
        return Ok(0);
    };
    let allowed_paths = sidebar_catalog_db_paths(codex_home)?;
    let mut restored_total = 0usize;
    for entry in entries {
        let path = PathBuf::from(entry["db_path"].as_str().unwrap_or_default());
        let canonical = fs::canonicalize(&path)?;
        if !allowed_paths.contains(&canonical) {
            anyhow::bail!("sidebar catalog database is not an allowed Codex database");
        }
        let table = entry["table"].as_str().unwrap_or_default();
        let rows = entry["rows"].as_array().cloned().unwrap_or_default();
        let mut db = Connection::open(&path)?;
        let tx = db.transaction()?;
        if !has_table(&tx, table)? {
            tx.commit()?;
            continue;
        }
        let columns = table_columns(&tx, table)?;
        if !columns.contains("thread_id") {
            tx.commit()?;
            continue;
        }
        let mut restored = 0usize;
        for row in rows {
            if let Some(row) = row.as_object() {
                restored += insert_row_ignore(&tx, table, row)?;
            }
        }
        if restored > 0 {
            let metadata_columns = table_columns(&tx, "local_thread_catalog_metadata")?;
            update_local_catalog_metadata(&tx, &metadata_columns, restored)?;
        }
        tx.commit()?;
        restored_total += restored;
    }
    Ok(restored_total)
}

fn insert_row_ignore(db: &Connection, table: &str, row: &Map<String, Value>) -> anyhow::Result<usize> {
    let columns: Vec<&String> = row.keys().collect();
    if columns.is_empty() {
        return Ok(0);
    }
    let quoted = columns
        .iter()
        .map(|column| format!("\"{}\"", column.replace('\"', "\"\"")))
        .collect::<Vec<_>>()
        .join(", ");
    let marks = (0..columns.len())
        .map(|index| format!("?{}", index + 1))
        .collect::<Vec<_>>()
        .join(", ");
    let values = columns
        .iter()
        .map(|column| json_to_sql_value(&row[*column]))
        .collect::<Vec<_>>();
    let refs = values
        .iter()
        .map(|value| value as &dyn ToSql)
        .collect::<Vec<_>>();
    Ok(db.execute(
        &format!("INSERT OR IGNORE INTO \"{table}\" ({quoted}) VALUES ({marks})"),
        refs.as_slice(),
    )?)
}

fn sidebar_catalog_db_paths(codex_home: &Path) -> anyhow::Result<HashSet<PathBuf>> {
    Ok(codex_plus_core::codex_sqlite::codex_thread_reference_db_paths_from_home(codex_home)
        .into_iter()
        .filter_map(|path| fs::canonicalize(path).ok())
        .collect())
}

fn thread_value_matches(value: &Value, thread_id: &str) -> bool {
    value
        .as_str()
        .is_some_and(|value| value == thread_id || value == format!("local:{thread_id}"))
}

fn thread_key_matches(key: &str, thread_id: &str) -> bool {
    key == thread_id || key == format!("local:{thread_id}")
}

fn sidebar_atom_key_matches(key: &str, thread_id: &str) -> bool {
    let encoded_local = format!("local%3A{thread_id}");
    [
        format!("thread-client-id-v1:{thread_id}"),
        format!("thread-client-id-v1:{encoded_local}"),
        format!("thread-reference-capability:{thread_id}"),
        format!("thread-reference-capability:{encoded_local}"),
    ]
    .iter()
    .any(|candidate| key == candidate)
}

fn thread_tombstone_path(codex_home: &Path) -> PathBuf {
    codex_home.join(THREAD_TOMBSTONE_RELATIVE_PATH)
}

/// 读取删除墓碑（已删除、不得被索引重建复活的 thread id）。
///
/// 墓碑只是「修复时跳过」的抑制信息，不是用户数据：文件缺失、读不动、内容坏了都
/// 一律按「没有墓碑」处理——最坏结果是退回旧行为（可能复活），绝不因为墓碑本身
/// 让同步或删除失败。见 issue #2199。
fn load_thread_tombstones(codex_home: &Path) -> HashSet<String> {
    let path = thread_tombstone_path(codex_home);
    let Ok(bytes) = fs::read(&path) else {
        return HashSet::new();
    };
    serde_json::from_slice::<Vec<String>>(&bytes)
        .map(|ids| ids.into_iter().collect())
        .unwrap_or_default()
}

/// 记一条删除墓碑。顺序按写入先后保留，超出上限时丢掉最旧的一批。
///
/// 只写不删（除了上限淘汰），因此不存在「墓碑误删会话数据」的路径；
/// 调用方在删除流程中尽力而为，失败只影响「将来会不会复活」。
fn record_thread_tombstone(codex_home: &Path, thread_id: &str) -> anyhow::Result<()> {
    let path = thread_tombstone_path(codex_home);
    let mut ids = fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Vec<String>>(&bytes).ok())
        .unwrap_or_default();
    ids.retain(|existing| existing != thread_id);
    ids.push(thread_id.to_string());
    if ids.len() > MAX_THREAD_TOMBSTONES {
        ids.drain(..ids.len() - MAX_THREAD_TOMBSTONES);
    }
    codex_plus_core::settings::atomic_write(&path, serde_json::to_vec(&ids)?.as_slice())
}

/// 撤销删除时清除墓碑，否则刚恢复的会话会被重建流程一直挡在列表外。
fn clear_thread_tombstones(codex_home: &Path, thread_ids: &HashSet<String>) {
    if thread_ids.is_empty() {
        return;
    }
    let path = thread_tombstone_path(codex_home);
    let Ok(bytes) = fs::read(&path) else {
        return;
    };
    let Ok(mut ids) = serde_json::from_slice::<Vec<String>>(&bytes) else {
        return;
    };
    let before = ids.len();
    ids.retain(|id| !thread_ids.contains(id));
    if ids.len() == before {
        return;
    }
    // 清墓碑是尽力而为：写失败只意味着恢复出来的会话仍被跳过，不影响已恢复的数据。
    if let Ok(bytes) = serde_json::to_vec(&ids) {
        let _ = codex_plus_core::settings::atomic_write(&path, &bytes);
    }
}

pub fn remove_thread_sidebar_references(
    codex_home: &Path,
    thread_id: &str,
) -> anyhow::Result<ThreadSidebarCleanupResult> {
    let thread_id = thread_id.strip_prefix("local:").unwrap_or(thread_id);
    // issue #2199：删除侧只有「清理」，重建侧却只看资格判定（rollout 还在就补回来），
    // 于是删掉的会话切个页面又出现，而归档（archived=1）反而真的消失。
    // 这里在清理之前先记墓碑：后面两个清理步骤可能失败，但「用户删过这条」这件事
    // 必须先落盘，否则重建还会把它写回来。墓碑只抑制修复、不删数据。
    if let Err(error) = record_thread_tombstone(codex_home, thread_id) {
        let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
            "provider_sync.thread_tombstone_write_failed",
            json!({ "thread_id": thread_id, "error": error.to_string() }),
        );
    }
    let (global_state_entries_removed, global_error) =
        match remove_thread_from_global_state(codex_home, thread_id) {
            Ok(count) => (count, None),
            Err(error) => (0, Some(error)),
        };
    let (catalog_rows_removed, catalog_error) =
        match remove_thread_from_catalog_dbs(codex_home, thread_id) {
            Ok(count) => (count, None),
            Err(error) => (0, Some(error)),
        };
    if let Some(error) = global_error {
        return Err(error);
    }
    if let Some(error) = catalog_error {
        return Err(error);
    }
    Ok(ThreadSidebarCleanupResult {
        global_state_entries_removed,
        catalog_rows_removed,
    })
}

fn remove_thread_from_global_state(codex_home: &Path, thread_id: &str) -> anyhow::Result<usize> {
    let path = codex_home.join(".codex-global-state.json");
    if !path.exists() {
        return Ok(0);
    }
    let original_bytes = fs::read(&path)?;
    let mut state: Value = serde_json::from_slice(&original_bytes)?;
    let Some(root) = state.as_object_mut() else {
        return Ok(0);
    };
    let mut removed = 0usize;
    if let Some(ids) = root
        .get_mut("projectless-thread-ids")
        .and_then(Value::as_array_mut)
    {
        let before = ids.len();
        ids.retain(|value| !thread_value_matches(value, thread_id));
        removed += before.saturating_sub(ids.len());
    }
    for key in [
        "thread-projectless-output-directories",
        "thread-workspace-root-hints",
        "thread-writable-roots",
    ] {
        if let Some(map) = root.get_mut(key).and_then(Value::as_object_mut) {
            for candidate in [thread_id, &format!("local:{thread_id}")] {
                if map.remove(candidate).is_some() {
                    removed += 1;
                }
            }
        }
    }
    if let Some(atom) = root
        .get_mut("electron-persisted-atom-state")
        .and_then(Value::as_object_mut)
    {
        let encoded_local = format!("local%3A{thread_id}");
        let client_id = format!("thread-client-id-v1:{thread_id}");
        let client_id_encoded = format!("thread-client-id-v1:{encoded_local}");
        let reference_capability = format!("thread-reference-capability:{thread_id}");
        let reference_capability_encoded = format!("thread-reference-capability:{encoded_local}");
        let keys = atom
            .keys()
            .filter(|key| {
                *key == &client_id
                    || *key == &client_id_encoded
                    || *key == &reference_capability
                    || *key == &reference_capability_encoded
            })
            .cloned()
            .collect::<Vec<_>>();
        for key in keys {
            atom.remove(&key);
            removed += 1;
        }
    }
    if removed == 0 {
        return Ok(0);
    }
    if fs::read(&path)? != original_bytes {
        anyhow::bail!(".codex-global-state.json changed while deleting thread {thread_id}");
    }
    codex_plus_core::settings::atomic_write(&path, serde_json::to_string_pretty(&state)?.as_bytes())?;
    Ok(removed)
}

fn remove_thread_from_catalog_dbs(codex_home: &Path, thread_id: &str) -> anyhow::Result<usize> {
    let mut removed_total = 0usize;
    for path in codex_plus_core::codex_sqlite::codex_thread_reference_db_paths_from_home(codex_home) {
        if !path.exists() {
            continue;
        }
        let mut db = Connection::open(&path)?;
        db.busy_timeout(std::time::Duration::from_millis(500))?;
        let tx = db.transaction()?;
        let mut removed = 0usize;
        for table in [
            "local_thread_catalog",
            "thread_timeline_ledger",
            "local_thread_catalog_scan_entries",
        ] {
            let columns = table_columns(&tx, table)?;
            if !columns.contains("thread_id") {
                continue;
            }
            removed += tx.execute(
                &format!("DELETE FROM {table} WHERE thread_id = ?1"),
                [thread_id],
            )?;
        }
        if removed > 0 {
            let metadata_columns = table_columns(&tx, "local_thread_catalog_metadata")?;
            if metadata_columns.contains("catalog_revision") {
                tx.execute(
                    "UPDATE local_thread_catalog_metadata SET catalog_revision = catalog_revision + ?1",
                    [removed as i64],
                )?;
            }
        }
        tx.commit()?;
        removed_total += removed;
    }
    Ok(removed_total)
}

/// Append previously removed `session_index.jsonl` lines back (undo flow).
/// Lines whose `id` already exists are skipped. Returns the number of
/// appended lines. Best-effort: returns `Ok(0)` without writing when the
/// file changed since it was read.
pub fn restore_session_index_entries(
    codex_home: &Path,
    lines: &[String],
) -> anyhow::Result<usize> {
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
    // issue #2199：撤销删除等于用户明确要求「把这条会话还给我」，所以这里要撤掉墓碑，
    // 否则恢复出来的会话会被重建路径一直挡在列表外——撤销看起来没生效。
    let mut restored_ids = HashSet::new();
    for line in lines {
        if let Some(candidate) = known_session_index_candidate(line) {
            if existing_ids.contains(&candidate.id) {
                continue;
            }
            restored_ids.insert(candidate.id.clone());
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
    clear_thread_tombstones(codex_home, &restored_ids);
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
        let file = match File::open(&path) {
            Ok(file) => file,
            Err(error) if is_locked_io_error(&error) => continue,
            Err(error) => return Err(error.into()),
        };
        let mut reader = BufReader::new(file);
        let mut line = String::new();
        loop {
            line.clear();
            if reader.read_line(&mut line)? == 0 {
                break;
            }
            let (line, _) = split_line_ending(&line);
            let Ok(record) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if record.get("type").and_then(Value::as_str) != Some("session_meta") {
                continue;
            }
            let Some(provider) = record
                .get("payload")
                .and_then(Value::as_object)
                .and_then(|payload| payload.get("model_provider"))
                .and_then(Value::as_str)
            else {
                continue;
            };
            if is_valid_provider_id_for_discovery(provider) {
                ids.insert(provider.to_string());
            }
        }
    }
    Ok(sorted_provider_ids(ids))
}

fn collect_rollout_files(root: &Path, files: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    for entry in fs::read_dir(root)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_rollout_files(&path, files)?;
        } else if path
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
    changes: &[SessionChange],
) -> anyhow::Result<PathBuf> {
    create_backup_with_session_meta_lines(
        home,
        target_provider,
        changes.len(),
        changes.iter().map(|change| {
            (
                change.path.as_path(),
                change.original_session_meta_lines.as_slice(),
            )
        }),
    )
}

fn create_bulk_backup(
    home: &Path,
    target_provider: &str,
    rewrite_plans: &[BulkSessionRewritePlan],
) -> anyhow::Result<PathBuf> {
    create_backup_with_session_meta_lines(
        home,
        target_provider,
        rewrite_plans.len(),
        rewrite_plans.iter().map(|plan| {
            (
                plan.path.as_path(),
                plan.original_session_meta_lines.as_slice(),
            )
        }),
    )
}

fn create_backup_with_session_meta_lines<'a>(
    home: &Path,
    target_provider: &str,
    changed_session_files: usize,
    entries: impl IntoIterator<Item = (&'a Path, &'a [String])>,
) -> anyhow::Result<PathBuf> {
    let backup_root = home.join("backups_state/provider-sync");
    let mut backup_dir = backup_root.join(timestamp_name());
    let mut suffix = 0;
    while backup_dir.exists() {
        suffix += 1;
        backup_dir = backup_root.join(format!("{}-{suffix}", timestamp_name()));
    }
    fs::create_dir_all(&backup_dir)?;
    for name in [
        "config.toml",
        ".codex-global-state.json",
        ".codex-global-state.json.bak",
    ] {
        let source = home.join(name);
        if source.exists() {
            fs::copy(&source, backup_dir.join(name))?;
        }
    }
    let db_dir = backup_dir.join("db");
    let mut db_files = Vec::new();
    for db_path in provider_sync_db_paths(home) {
        for source in codex_plus_core::codex_sqlite::codex_sqlite_sidecar_paths(&db_path) {
            if !source.exists() {
                continue;
            }
            let relative = codex_plus_core::codex_sqlite::relative_to_codex_home(home, &source);
            let target = db_dir.join(&relative);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&source, &target)?;
            db_files.push(relative.to_string_lossy().replace('\\', "/"));
        }
    }
    write_session_meta_backup(&backup_dir.join("session-meta-backup.json"), entries)?;
    fs::write(
        backup_dir.join("metadata.json"),
        serde_json::to_string_pretty(&json!({
            "version": 1,
            "namespace": "provider-sync",
            "codexHome": home.to_string_lossy(),
            "targetProvider": target_provider,
            "createdAt": chrono::Utc::now().to_rfc3339(),
            "dbFiles": db_files,
            "changedSessionFiles": changed_session_files,
            "managedBy": "Codex++ provider sync"
        }))?,
    )?;
    Ok(backup_dir)
}

fn write_session_meta_backup<'a>(
    path: &Path,
    entries: impl IntoIterator<Item = (&'a Path, &'a [String])>,
) -> anyhow::Result<()> {
    let mut writer = BufWriter::new(File::create(path)?);
    writer.write_all(b"[\n")?;
    let mut first = true;
    for (entry_path, original_session_meta_lines) in entries {
        if !first {
            writer.write_all(b",\n")?;
        }
        first = false;
        let entry = SessionMetaBackupEntry {
            path: entry_path.to_string_lossy().to_string(),
            original_session_meta_lines,
        };
        let encoded = serde_json::to_string_pretty(&entry)?;
        for line in encoded.lines() {
            writer.write_all(b"  ")?;
            writer.write_all(line.as_bytes())?;
            writer.write_all(b"\n")?;
        }
    }
    writer.write_all(b"]\n")?;
    writer.flush()?;
    Ok(())
}

const BULK_SESSION_SOURCE_CHANGED_ERROR: &str =
    "rollout changed while provider metadata was being written";

struct HashingWriter<W> {
    inner: W,
    hasher: Sha256,
}

impl<W> HashingWriter<W> {
    fn new(inner: W) -> Self {
        Self {
            inner,
            hasher: Sha256::new(),
        }
    }

    fn sha256_hex(&self) -> String {
        format!("{:x}", self.hasher.clone().finalize())
    }
}

impl<W: Write> Write for HashingWriter<W> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let written = self.inner.write(buffer)?;
        self.hasher.update(&buffer[..written]);
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

fn apply_bulk_session_rewrite_plans(
    rewrite_plans: &[BulkSessionRewritePlan],
    target_provider: &str,
    total_rollout_files: usize,
    scanned_skipped_rollout_files: usize,
    report_progress: &mut dyn FnMut(ProviderSyncProgress),
) -> anyhow::Result<AppliedBulkSessionRewrites> {
    let mut applied = AppliedBulkSessionRewrites::default();
    report_provider_sync_progress(
        report_progress,
        ProviderSyncProgressPhase::Rewriting,
        total_rollout_files,
        total_rollout_files,
        rewrite_plans.len(),
        0,
        scanned_skipped_rollout_files,
    );

    for (index, plan) in rewrite_plans.iter().enumerate() {
        match rewrite_bulk_session_plan(plan, target_provider) {
            Ok(Some(rewritten_sha256)) => {
                restore_file_mtime(&plan.path, plan.original_mtime);
                applied.changes.push(AppliedBulkSessionRewrite {
                    plan: plan.clone(),
                    rewritten_sha256,
                });
            }
            Ok(None) => applied.skipped_locked_rollout_files.push(plan.path.clone()),
            Err(error) if is_locked_anyhow_error(&error) => {
                applied.skipped_locked_rollout_files.push(plan.path.clone());
            }
            Err(error) => {
                if !applied.changes.is_empty() {
                    report_provider_sync_progress(
                        report_progress,
                        ProviderSyncProgressPhase::RollingBack,
                        total_rollout_files,
                        total_rollout_files,
                        rewrite_plans.len(),
                        applied.changes.len(),
                        scanned_skipped_rollout_files
                            + applied.skipped_locked_rollout_files.len(),
                    );
                }
                if let Err(restore_error) = restore_bulk_session_rewrites(&applied.changes) {
                    return Err(anyhow::anyhow!(
                        "{error}; rollout rollback failed: {restore_error}"
                    ));
                }
                return Err(error);
            }
        }
        report_bulk_rewrite_progress(
            report_progress,
            total_rollout_files,
            rewrite_plans.len(),
            index + 1,
            applied.changes.len(),
            scanned_skipped_rollout_files + applied.skipped_locked_rollout_files.len(),
        );
    }
    Ok(applied)
}

fn report_bulk_rewrite_progress(
    report_progress: &mut dyn FnMut(ProviderSyncProgress),
    total_rollout_files: usize,
    planned_rewrite_files: usize,
    completed_rewrite_files: usize,
    applied_rewrite_files: usize,
    skipped_locked_rollout_files: usize,
) {
    if completed_rewrite_files == 1
        || completed_rewrite_files == planned_rewrite_files
        || completed_rewrite_files % PROVIDER_SYNC_PROGRESS_INTERVAL == 0
    {
        report_provider_sync_progress(
            report_progress,
            ProviderSyncProgressPhase::Rewriting,
            total_rollout_files,
            total_rollout_files,
            planned_rewrite_files,
            applied_rewrite_files,
            skipped_locked_rollout_files,
        );
    }
}

fn rewrite_bulk_session_plan(
    plan: &BulkSessionRewritePlan,
    target_provider: &str,
) -> anyhow::Result<Option<String>> {
    if sha256_file(&plan.path)? != plan.original_sha256 {
        return Ok(None);
    }

    let mut rewritten_sha256 = None;
    let write_result = codex_plus_core::settings::atomic_write_with(&plan.path, |file| {
        let mut writer = HashingWriter::new(BufWriter::new(file));
        let source_sha256 = stream_rewrite_rollout_session_meta_providers(
            &plan.path,
            target_provider,
            &mut writer,
        )?;
        writer.flush()?;
        if source_sha256 != plan.original_sha256 || sha256_file(&plan.path)? != plan.original_sha256
        {
            return Err(std::io::Error::other(BULK_SESSION_SOURCE_CHANGED_ERROR));
        }
        rewritten_sha256 = Some(writer.sha256_hex());
        Ok(())
    });
    match write_result {
        Ok(()) => Ok(rewritten_sha256),
        Err(error) if error_chain_contains(&error, BULK_SESSION_SOURCE_CHANGED_ERROR) => Ok(None),
        Err(error) => Err(error),
    }
}

fn stream_rewrite_rollout_session_meta_providers<W: Write>(
    path: &Path,
    target_provider: &str,
    writer: &mut W,
) -> std::io::Result<String> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut hasher = Sha256::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        hasher.update(line.as_bytes());
        let (record_line, line_ending) = split_line_ending(&line);
        let mut rewritten = false;
        if !record_line.trim().is_empty()
            && let Ok(mut record) = serde_json::from_str::<Value>(record_line)
            && record.get("type").and_then(Value::as_str) == Some("session_meta")
            && let Some(payload) = record.get_mut("payload").and_then(Value::as_object_mut)
            && payload.get("model_provider").and_then(Value::as_str) != Some(target_provider)
        {
            payload.insert("model_provider".to_string(), json!(target_provider));
            serde_json::to_writer(&mut *writer, &record).map_err(std::io::Error::other)?;
            writer.write_all(line_ending.as_bytes())?;
            rewritten = true;
        }
        if !rewritten {
            writer.write_all(line.as_bytes())?;
        }
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn restore_bulk_session_rewrites(changes: &[AppliedBulkSessionRewrite]) -> anyhow::Result<()> {
    let mut restore_errors = Vec::new();
    for change in changes.iter().rev() {
        if let Err(error) = restore_bulk_session_rewrite(change) {
            restore_errors.push(format!("{}: {error:#}", change.plan.path.display()));
        }
    }
    if !restore_errors.is_empty() {
        return Err(anyhow::anyhow!(
            "failed to restore {} rollout file(s): {}",
            restore_errors.len(),
            restore_errors.join("; ")
        ));
    }
    Ok(())
}

fn restore_bulk_session_rewrite(change: &AppliedBulkSessionRewrite) -> anyhow::Result<()> {
    if sha256_file(&change.plan.path)? != change.rewritten_sha256 {
        return Err(anyhow::anyhow!(
            "rollout changed before rollback: {}",
            change.plan.path.display()
        ));
    }

    codex_plus_core::settings::atomic_write_with(&change.plan.path, |file| {
        let mut writer = HashingWriter::new(BufWriter::new(file));
        let source_sha256 = stream_restore_rollout_session_meta_lines(
            &change.plan.path,
            &change.plan.original_session_meta_lines,
            &mut writer,
        )?;
        writer.flush()?;
        if source_sha256 != change.rewritten_sha256
            || sha256_file(&change.plan.path)? != change.rewritten_sha256
        {
            return Err(std::io::Error::other(BULK_SESSION_SOURCE_CHANGED_ERROR));
        }
        if writer.sha256_hex() != change.plan.original_sha256 {
            return Err(std::io::Error::other("rollout rollback hash mismatch"));
        }
        Ok(())
    })?;
    restore_file_mtime(&change.plan.path, change.plan.original_mtime);
    Ok(())
}

fn stream_restore_rollout_session_meta_lines<W: Write>(
    path: &Path,
    original_session_meta_lines: &[String],
    writer: &mut W,
) -> std::io::Result<String> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut hasher = Sha256::new();
    let mut originals = original_session_meta_lines.iter();
    let mut restored_session_meta_lines = 0usize;

    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        hasher.update(line.as_bytes());
        let (record_line, line_ending) = split_line_ending(&line);
        let valid_session_meta = !record_line.trim().is_empty()
            && serde_json::from_str::<Value>(record_line)
                .ok()
                .is_some_and(|record| {
                    record.get("type").and_then(Value::as_str) == Some("session_meta")
                        && record.get("payload").and_then(Value::as_object).is_some()
                });
        if valid_session_meta {
            let Some(original_line) = originals.next() else {
                return Err(std::io::Error::other(
                    "rollout session metadata count changed before rollback",
                ));
            };
            writer.write_all(original_line.as_bytes())?;
            writer.write_all(line_ending.as_bytes())?;
            restored_session_meta_lines += 1;
        } else {
            writer.write_all(line.as_bytes())?;
        }
    }
    if originals.next().is_some()
        || restored_session_meta_lines != original_session_meta_lines.len()
    {
        return Err(std::io::Error::other(
            "rollout session metadata count changed before rollback",
        ));
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut buffer = [0_u8; 64 * 1024];
    let mut hasher = Sha256::new();
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn is_locked_anyhow_error(error: &anyhow::Error) -> bool {
    error
        .chain()
        .filter_map(|cause| cause.downcast_ref::<std::io::Error>())
        .any(is_locked_io_error)
        || is_windows_sharing_anyhow_error(error)
}

#[cfg(windows)]
fn is_windows_sharing_anyhow_error(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        let message = cause.to_string().to_ascii_lowercase();
        message.contains("os error 32")
            || message.contains("os error 33")
            || message.contains("0x80070020")
            || message.contains("0x80070021")
    })
}

#[cfg(not(windows))]
fn is_windows_sharing_anyhow_error(_: &anyhow::Error) -> bool {
    false
}

fn error_chain_contains(error: &anyhow::Error, needle: &str) -> bool {
    error
        .chain()
        .any(|cause| cause.to_string().contains(needle))
}

fn create_session_index_cleanup_backup(
    home: &Path,
    plan: &SessionIndexCleanupPlan,
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
    let backup_index_path = backup_dir.join("session_index.jsonl");
    let copied_sha256 = copy_file_with_sha256(&plan.path, &backup_index_path)
        .map_err(|error| cleanup_apply_error(error, Some(backup_dir.clone())))?;
    if copied_sha256 != plan.snapshot_sha256 {
        return Err(cleanup_apply_error(
            "session_index.jsonl 在写入前再次发生变化；未覆盖 Codex 新内容，请重新预览",
            Some(backup_dir),
        ));
    }
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

const SESSION_INDEX_SNAPSHOT_CHANGED_ERROR: &str =
    "session index changed while cleanup was being written";

fn stream_filtered_session_index<W: Write>(
    path: &Path,
    expected_snapshot_sha256: &str,
    selected_ids: &HashSet<String>,
    writer: &mut W,
) -> std::io::Result<usize> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut hasher = Sha256::new();
    let mut removed_entries = 0usize;
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        hasher.update(line.as_bytes());
        let (record_line, _) = split_line_ending(&line);
        let remove = known_session_index_candidate(record_line)
            .is_some_and(|candidate| selected_ids.contains(&candidate.id));
        if remove {
            removed_entries += 1;
        } else {
            writer.write_all(line.as_bytes())?;
        }
    }
    if format!("{:x}", hasher.finalize()) != expected_snapshot_sha256
        || sha256_file(path)? != expected_snapshot_sha256
    {
        return Err(std::io::Error::other(SESSION_INDEX_SNAPSHOT_CHANGED_ERROR));
    }
    Ok(removed_entries)
}

fn copy_file_with_sha256(source: &Path, destination: &Path) -> std::io::Result<String> {
    let mut reader = BufReader::new(File::open(source)?);
    let mut writer = BufWriter::new(File::create(destination)?);
    let mut buffer = [0_u8; 64 * 1024];
    let mut hasher = Sha256::new();
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        writer.write_all(&buffer[..read])?;
    }
    writer.flush()?;
    Ok(format!("{:x}", hasher.finalize()))
}

fn apply_session_changes(changes: &[SessionChange]) -> anyhow::Result<AppliedSessionChanges> {
    let mut applied = AppliedSessionChanges::default();
    for change in changes {
        match replace_session_text_if_unchanged(
            &change.path,
            &change.original_text,
            &change.next_text,
        ) {
            Ok(true) => {}
            Ok(false) => {
                applied
                    .skipped_locked_rollout_files
                    .push(change.path.clone());
                continue;
            }
            Err(error) if is_locked_io_error(&error) => {
                applied
                    .skipped_locked_rollout_files
                    .push(change.path.clone());
                continue;
            }
            Err(error) => return Err(error.into()),
        }
        restore_file_mtime(&change.path, change.original_mtime);
        applied.changes.push(change.clone());
    }
    Ok(applied)
}

fn replace_session_text_if_unchanged(
    path: &Path,
    expected_text: &str,
    next_text: &str,
) -> std::io::Result<bool> {
    let mut file = open_session_file_for_update(path)?;
    file.try_lock()?;
    let mut current_text = String::new();
    file.read_to_string(&mut current_text)?;
    if current_text != expected_text {
        return Ok(false);
    }

    file.seek(SeekFrom::Start(0))?;
    file.set_len(0)?;
    file.write_all(next_text.as_bytes())?;
    file.flush()?;

    file.seek(SeekFrom::Start(0))?;
    let mut persisted_text = String::new();
    file.read_to_string(&mut persisted_text)?;
    if persisted_text != next_text {
        return Err(std::io::Error::other(
            "rollout changed while provider metadata was being written",
        ));
    }
    Ok(true)
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

fn sqlite_provider_sync_thread_kinds(
    paths: &[PathBuf],
) -> anyhow::Result<ProviderSyncThreadKinds> {
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

fn rollout_thread_provider_state(text: &str) -> Option<(String, HashSet<String>)> {
    let mut thread_id = None;
    let mut providers = HashSet::new();
    for segment in text.split_inclusive('\n') {
        let (line, _) = split_line_ending(segment);
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if record.get("type").and_then(Value::as_str) != Some("session_meta") {
            continue;
        }
        let Some(payload) = record.get("payload").and_then(Value::as_object) else {
            continue;
        };
        if thread_id.is_none() {
            thread_id = payload
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.trim().is_empty())
                .map(ToString::to_string);
        }
        providers.insert(
            payload
                .get("model_provider")
                .and_then(Value::as_str)
                .unwrap_or("(missing)")
                .to_string(),
        );
    }
    thread_id.map(|thread_id| (thread_id, providers))
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
        total += provider_update_thread_ids(
            &db,
            "threads",
            "id",
            target_provider,
            excluded_thread_ids,
        )?
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
        for thread_id in provider_update_thread_ids(
            &tx,
            "threads",
            "id",
            target_provider,
            excluded_thread_ids,
        )? {
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
    if plan.threads.is_empty()
        && (!update_full_sync_state || !plan.has_cleanup_candidates())
    {
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
    // issue #2199：删除墓碑一票否决。这里的候选会被 INSERT 回 `local_thread_catalog`，
    // 是「删了又回来」的重建入口之一。
    let thread_tombstones = load_thread_tombstones(home);
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
        // Current app-server listing uses preview; paginated histories can retain a zero
        // legacy user-event flag even when they contain real user messages.
        let has_user_content = if columns.contains("preview") {
            "CASE WHEN COALESCE(preview, '') <> '' THEN 1 ELSE 0 END".to_string()
        } else {
            text_expr(&columns, "has_user_event", "1")
        };
        let agent_role = text_expr(&columns, "agent_role", "''");
        let subagent_filter = subagent_filter(&db, "threads.id")?;
        let sql = format!(
            "SELECT id, {display_title}, {source_created_at}, {source_updated_at}, {cwd}, {source_kind}, {source_detail}, {git_branch}, {thread_source}, {archived}, {has_user_content}, {agent_role} FROM threads WHERE COALESCE(id, '') <> ''{subagent_filter}"
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
            let (thread, archived, has_user_content, agent_role) = item?;
            // issue #2199：删除墓碑一票否决。资格判定里没有「删除态」，只要 rollout
            // 文件还在（删除失败或被别的进程锁住等），被删的会话就会被重新插回 catalog。
            // 在收集阶段直接跳过，它既不进候选，也不会在下面的择优平局里胜出。
            if thread_tombstones.contains(&thread.id) {
                continue;
            }
            let marked_non_user = columns.contains("thread_source")
                && thread.thread_source.as_deref().is_some_and(|value| {
                    let value = value.trim();
                    !value.is_empty() && !value.eq_ignore_ascii_case("user")
                });
            let non_root = is_catalog_non_root_agent(&thread, &spawned_child_ids);
            let source_is_exec = thread.source_kind.trim().eq_ignore_ascii_case("exec");
            let rollout_exists = catalog_rollout_path_exists(home, &thread.source_detail);
            let eligible = archived == 0
                && has_user_content == 1
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
    // issue #1424：WSL 视角路径（/mnt/c/...）在 Windows 侧读不到，放宽到两种视角
    // 都试一次。这个函数只影响「会话是否算可修复候选」，放宽只会多保留记录、不删数据。
    rollout_path_candidates(home, rollout_path)
        .iter()
        .any(|path| path.is_file())
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
            if source_marks_non_root_agent(&source_kind) || spawned_child_ids.contains(&thread_id)
            {
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
    source_marks_non_root_agent(&thread.source_kind)
        || spawned_child_ids.contains(&thread.id)
}

fn thread_source_is_user(thread_source: Option<&str>) -> bool {
    thread_source
        .map(str::trim)
        .is_some_and(|value| value.eq_ignore_ascii_case("user"))
}

fn thread_source_marks_non_root(thread_source: Option<&str>) -> bool {
    thread_source.map(str::trim).is_some_and(|value| {
        value.eq_ignore_ascii_case("subagent")
            || value.eq_ignore_ascii_case("memory_consolidation")
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

fn global_state_snapshot(path: &Path) -> anyhow::Result<(Vec<u8>, Map<String, Value>)> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((Vec::new(), Map::new()));
        }
        Err(error) => return Err(error.into()),
    };
    // issue #2160：这里是全局状态唯一的解析入口。以前的报错是 serde_json 原文
    // （空文件/被截断时报 `expected value at line 1 column 1`），既没说是哪个文件，
    // 也没说怎么办；这条消息会一路冒到启动器的提示里，用户完全无法自助。
    // 这里统一补上文件路径和可操作建议。
    let state = serde_json::from_slice::<Value>(&bytes).map_err(|error| {
        anyhow::anyhow!(
            "{error}。{} 不是合法 JSON（常见于文件为空或被写了一半）；\
             请把它改成合法 JSON——内容丢了可以从同目录的 {} 恢复",
            global_state_path_display(path),
            GLOBAL_STATE_BACKUP_FILE_NAME,
        )
    })?;
    let state = state.as_object().cloned().unwrap_or_default();
    Ok((bytes, state))
}

/// 报错里的路径按用户视角展示，Windows 上 `C:\Users\x\.codex\...` 太具体也容易看错。
fn global_state_path_display(path: &Path) -> String {
    format!("{}（全局状态文件）", path.display())
}

fn load_global_state(path: &Path) -> anyhow::Result<Map<String, Value>> {
    Ok(global_state_snapshot(path)?.1)
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
    let (original_bytes, mut state) = global_state_snapshot(path)?;
    let next = normalized_global_state(&state);
    let count = next
        .iter()
        .filter(|(key, value)| state.get(*key) != Some(*value))
        .count();
    if count > 0 {
        for (key, value) in next {
            state.insert(key, value);
        }
        write_global_state_if_unchanged(path, &original_bytes, &state)?;
    }
    Ok(count)
}

const GLOBAL_STATE_SOURCE_CHANGED_ERROR: &str =
    "global state changed while provider sync was being written";

fn write_global_state_if_unchanged(
    path: &Path,
    original_bytes: &[u8],
    state: &Map<String, Value>,
) -> anyhow::Result<()> {
    // 会话删除/撤销也会更新此文件；不得用过期的 provider-sync 快照覆盖新侧边栏条目。
    if global_state_snapshot(path)?.0 != original_bytes {
        anyhow::bail!(GLOBAL_STATE_SOURCE_CHANGED_ERROR);
    }
    let text = serde_json::to_string_pretty(&Value::Object(state.clone()))?;
    codex_plus_core::settings::atomic_write(path, text.as_bytes())?;
    if let Some(parent) = path.parent() {
        codex_plus_core::settings::atomic_write(
            &parent.join(".codex-global-state.json.bak"),
            text.as_bytes(),
        )?;
    }
    Ok(())
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
        // `home` 来自调用方（可能是被解析坏的环境变量）。正常情况下 `path` 一定是
        // `home/backups_state/provider-sync` 的子目录，但删除不可逆，所以这里不靠
        // "正常情况"，逐个确认它确实是该 root 的直接子项再删（#2146）。
        let is_owned_child = path.parent().is_some_and(|parent| parent == root.as_path());
        if !is_owned_child {
            continue;
        }
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
mod startup_scan_tests {
    use super::*;

    #[test]
    fn metadata_parser_matches_value_semantics_without_substring_guessing() {
        for line in [
            r#"{"type":"session_meta","payload":{"id":"t","model_provider":"old"}}"#,
            r#"{"payload":{"id":"t"},"type" : "session_meta"}"#,
            r#"{"ty\u0070e":"session\u005fmeta","payload":{"id":"t"}}"#,
            r#"{"type":"response_item","type":"session_meta","payload":{"id":"t"}}"#,
            r#"{"type":"session_meta","type":"response_item","payload":{"id":"t"}}"#,
            r#"{"type":"session_meta","payload":{"id":"old"},"payload":{"id":"new"}}"#,
            r#"{"type":"response_item","payload":{"text":"\"type\":\"session_meta\""}}"#,
            r#"{"type":null,"payload":{}}"#,
            r#"{"type":42,"payload":{}}"#,
            r#"{"payload":{"type":"session_meta"}}"#,
            r#"["session_meta"]"#,
            r#"{"type":"session_meta","payload":{"id":"t"}"#,
            r#"{"type":"session_meta","payload":{"id":"t"}} trailing"#,
            r#"{"type":"response_item","payload":{"nested":[invalid]}}"#,
        ] {
            let expected = serde_json::from_str::<Value>(line)
                .ok()
                .filter(|record| record["type"].as_str() == Some("session_meta"));
            assert_eq!(parse_session_meta_record(line), expected, "{line}");
        }
    }

    #[test]
    fn bulk_scan_matches_legacy_plans_hashes_and_all_summary_facts() {
        let temp = tempfile::tempdir().unwrap();
        let sessions = temp.path().join("sessions");
        fs::create_dir(&sessions).unwrap();
        for (name, meta) in [
            ("user", r#"{"type":"session_meta","payload":{"id":"user","cwd":"C:/workspace","model_provider":"old"}}"#),
            ("child", r#"{"type":"session_meta","payload":{"id":"child","source":{"subagent":{"depth":1}},"model_provider":"old"}}"#),
            ("excluded", r#"{"type":"session_meta","payload":{"id":"excluded","model_provider":"old"}}"#),
            ("explicit", r#"{"type":"session_meta","payload":{"id":"explicit","model_provider":"old"}}"#),
            ("missing", r#"{"type":"session_meta","payload":{"id":"missing"}}"#),
            ("escaped", r#"{"payload":{"id":"escaped","model_provider":"old"},"ty\u0070e":"session\u005fmeta"}"#),
            ("same", r#"{"type":"session_meta","payload":{"id":"same","model_provider":"new"}}"#),
        ] {
            let body = format!(
                "{meta}\r\n{}\n{}\n{meta}\nmalformed\n",
                json!({"type":"event_msg","payload":{"type":"user_message"}}),
                json!({"type":"response_item","payload":{"encrypted_content":"opaque","large":["x".repeat(1024 * 1024)],"metadata_decoy":"session_meta"}}),
            );
            fs::write(sessions.join(format!("rollout-{name}.jsonl")), body).unwrap();
        }
        let excluded = HashSet::from(["excluded".into(), "explicit".into()]);
        let explicit = HashSet::from(["explicit".into()]);
        let mut legacy_progress = Vec::new();
        let legacy = scan_bulk_session_rewrites_with_parser(
            temp.path(), "new", &excluded, &explicit, &mut |p| legacy_progress.push(p),
            |line| serde_json::from_str::<Value>(line).ok(), true,
        ).unwrap();
        let mut progress = Vec::new();
        let optimized = scan_bulk_session_rewrites(
            temp.path(), "new", &excluded, &explicit, &mut |p| progress.push(p),
        ).unwrap();
        assert_eq!(optimized, legacy);
        assert_eq!(serde_json::to_value(progress).unwrap(), serde_json::to_value(legacy_progress).unwrap());
        assert_eq!(optimized.rewrite_plans.len(), 4);
        assert!(optimized.thread_ids_with_user_events.contains("explicit"));
        assert!(!optimized.thread_ids_with_user_events.contains("excluded"));
        assert!(optimized.subagent_thread_ids.contains("child"));
    }

    #[test]
    fn unchanged_scan_has_no_digest_but_late_metadata_rewrite_hashes_exact_bytes() {
        let current = b"{\"type\":\"session_meta\",\"payload\":{\"id\":\"t\",\"model_provider\":\"new\"}}\r\n";
        let mut reader = std::io::Cursor::new(current.to_vec());
        let (facts, digest) = scan_bulk_rollout(&mut reader, "new", &parse_session_meta_record, false).unwrap();
        assert!(!facts.rewrite_needed);
        assert!(digest.is_none());
        let mut bytes = current.to_vec();
        bytes.extend_from_slice(b"{\"type\":\"response_item\",\"payload\":{\"text\":\"body\"}}\n");
        bytes.extend_from_slice(b"{\"type\":\"session_meta\",\"payload\":{\"id\":\"t\",\"model_provider\":\"old\"}}\n");
        let mut reader = std::io::Cursor::new(bytes.clone());
        let (facts, digest) = scan_bulk_rollout(&mut reader, "new", &parse_session_meta_record, false).unwrap();
        assert!(facts.rewrite_needed);
        assert_eq!(facts.session_meta_count, 2);
        assert!(digest.is_none());
        let digest = hash_bulk_rollout_plan(&mut reader, "new", &parse_session_meta_record, &facts).unwrap();
        assert_eq!(digest, format!("{:x}", Sha256::digest(&bytes)));
    }

    #[test]
    fn write_plan_hash_pass_refuses_changed_metadata_or_summary_facts() {
        let original = b"{\"type\":\"session_meta\",\"payload\":{\"id\":\"t\",\"model_provider\":\"old\"}}\n";
        let mut reader = std::io::Cursor::new(original.to_vec());
        let (facts, _) = scan_bulk_rollout(&mut reader, "new", &parse_session_meta_record, false).unwrap();
        for changed in [
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"other\",\"model_provider\":\"old\"}}\n",
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"t\",\"model_provider\":\"new\"}}\n",
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"t\",\"model_provider\":\"old\"}}\n{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\"}}\n",
        ] {
            let mut reader = std::io::Cursor::new(changed.as_bytes());
            let error = hash_bulk_rollout_plan(&mut reader, "new", &parse_session_meta_record, &facts).unwrap_err();
            assert!(error.to_string().contains("Rollout changed while planning"));
        }
    }

    #[test]
    #[ignore = "read-only timing; requires CPP_PROVIDER_SCAN_HOME; never runs sync/repair"]
    fn measure_provider_scan_read_only() {
        let home = PathBuf::from(std::env::var_os("CPP_PROVIDER_SCAN_HOME").unwrap());
        let kinds = sqlite_provider_sync_thread_kinds(&provider_sync_db_paths(&home)).unwrap();
        let target = resolve_provider_sync_target_snapshot(&home.join("config.toml"), None)
            .unwrap().target_provider;
        let start = std::time::Instant::now();
        let legacy = scan_bulk_session_rewrites_with_parser(
            &home, &target, &kinds.subagent_thread_ids, &kinds.explicit_user_thread_ids,
            &mut |_| {}, |line| serde_json::from_str::<Value>(line).ok(), true,
        ).unwrap();
        let legacy_ms = start.elapsed().as_millis();
        let start = std::time::Instant::now();
        let optimized = scan_bulk_session_rewrites(
            &home, &target, &kinds.subagent_thread_ids, &kinds.explicit_user_thread_ids,
            &mut |_| {},
        ).unwrap();
        eprintln!(
            "READ_ONLY_SCAN legacy_ms={legacy_ms} optimized_ms={} legacy_files={} optimized_files={} legacy_plans={} optimized_plans={}",
            start.elapsed().as_millis(), legacy.total_rollout_files, optimized.total_rollout_files,
            legacy.rewrite_plans.len(), optimized.rewrite_plans.len(),
        );
        let start = std::time::Instant::now();
        scan_bulk_session_rewrites_with_parser(
            &home, &target, &kinds.subagent_thread_ids, &kinds.explicit_user_thread_ids,
            &mut |_| {}, |_| None, false,
        ).unwrap();
        eprintln!("READ_ONLY_SCAN parse_free_read_and_flags_ms={}", start.elapsed().as_millis());
    }
}

#[cfg(test)]
mod provider_target_snapshot_tests {
    use super::*;

    fn write_config(home: &Path, current: &str, providers: &[&str]) {
        let mut text = format!("model_provider = {current:?}\n");
        for provider in providers {
            text.push_str(&format!(
                "\n[model_providers.{provider:?}]\nname = {provider:?}\n"
            ));
        }
        fs::write(home.join("config.toml"), text).unwrap();
    }

    #[test]
    fn current_provider_change_at_first_write_boundary_leaves_history_untouched() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join(".codex");
        let rollout = home.join("sessions/rollout-race.jsonl");
        fs::create_dir_all(rollout.parent().unwrap()).unwrap();
        write_config(&home, "relay-alpha", &["relay-alpha"]);
        let original_rollout = format!(
            "{}\n{}\n",
            json!({
                "type": "session_meta",
                "payload": {
                    "id": "thread-1",
                    "model_provider": "openai",
                    "cwd": "C:/workspace"
                }
            }),
            json!({"type": "event_msg", "payload": {"type": "user_message"}}),
        );
        fs::write(&rollout, &original_rollout).unwrap();
        let config_path = home.join("config.toml");

        let result = run_provider_sync_with_target_in_home(
            home.clone(),
            None,
            false,
            || {
                write_config(&home, "relay-beta", &["relay-beta"]);
            },
            |_| {},
        );

        assert_eq!(result.status, ProviderSyncStatus::Skipped);
        assert!(result.message.contains("configuration changed"));
        assert!(result.backup_dir.is_none());
        assert_eq!(fs::read_to_string(rollout).unwrap(), original_rollout);
        assert!(!home.join("backups_state/provider-sync").exists());
        assert!(!home.join("tmp/provider-sync.lock").exists());
        assert!(config_path.exists());
    }

    #[test]
    fn adding_an_unrelated_provider_table_does_not_change_target_identity() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join(".codex");
        fs::create_dir(&home).unwrap();
        write_config(&home, "relay-alpha", &["relay-alpha"]);
        let config_path = home.join("config.toml");
        let snapshot = resolve_provider_sync_target_snapshot(&config_path, None).unwrap();

        write_config(&home, "relay-alpha", &["relay-alpha", "relay-beta"]);

        revalidate_provider_sync_target_snapshot(&config_path, None, &snapshot).unwrap();
    }
}

#[cfg(test)]
mod provider_sync_preview_tests {
    use super::*;

    fn write_config(home: &Path, current: &str) {
        fs::write(
            home.join("config.toml"),
            format!("model_provider = {current:?}\n\n[model_providers.{current:?}]\nname = {current:?}\n"),
        )
        .unwrap();
    }

    /// issue #240：预览必须只读——不建锁、不建备份、不改任何文件。
    #[test]
    fn preview_is_read_only_and_reports_backup_root() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join(".codex");
        fs::create_dir_all(&home).unwrap();
        write_config(&home, "relay-alpha");
        let config_before = fs::read(home.join("config.toml")).unwrap();

        let preview = preview_provider_sync(Some(&home)).unwrap();

        assert_eq!(preview.target_provider, "relay-alpha");
        assert_eq!(preview.backup_root, home.join("backups_state/provider-sync"));
        assert!(!home.join("tmp/provider-sync.lock").exists(), "预览不该建锁");
        assert!(!preview.backup_root.exists(), "预览不该建备份目录");
        assert_eq!(fs::read(home.join("config.toml")).unwrap(), config_before);
    }

    /// home 不存在时预览也不该报错中断，只是审计为空。
    #[test]
    fn preview_survives_missing_home() {
        let temp = tempfile::tempdir().unwrap();
        let missing = temp.path().join(".codex");

        let preview = preview_provider_sync(Some(&missing)).unwrap();

        assert_eq!(preview.audit, ProviderSyncAudit::default());
        assert!(!missing.join("tmp/provider-sync.lock").exists());
    }

    /// 没有 catalog-only 记录时，审计字段全为 0，不该虚报。
    #[test]
    fn preview_audit_is_zero_without_catalog_only_sessions() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join(".codex");
        fs::create_dir_all(&home).unwrap();
        write_config(&home, "relay-alpha");

        let preview = preview_provider_sync(Some(&home)).unwrap();

        assert_eq!(preview.audit.catalog_only_sessions, 0);
        assert_eq!(preview.audit.catalog_only_without_recovery_source, 0);
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
        assert!(marks_non_root(r#"{"subagent":{"thread_spawn":{"depth":1}}}"#));
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
mod rollback_tests {
    use super::*;

    #[test]
    fn rollback_hash_mismatch_keeps_rewritten_rollout_intact() {
        let temp = tempfile::tempdir().unwrap();
        let rollout = temp.path().join("rollout.jsonl");
        let original_meta =
            r#"{"type":"session_meta","payload":{"id":"thread-1","model_provider":"openai"}}"#;
        let rewritten_meta =
            r#"{"type":"session_meta","payload":{"id":"thread-1","model_provider":"apigather"}}"#;
        let incorrect_meta =
            r#"{"type":"session_meta","payload":{"id":"thread-1","model_provider":"incorrect"}}"#;
        let event = r#"{"type":"event_msg","payload":{"type":"user_message"}}"#;
        let original = format!("{original_meta}\n{event}\n");
        let rewritten = format!("{rewritten_meta}\n{event}\n");
        fs::write(&rollout, original).unwrap();
        let original_sha256 = sha256_file(&rollout).unwrap();
        fs::write(&rollout, &rewritten).unwrap();
        let rewritten_sha256 = sha256_file(&rollout).unwrap();
        let change = AppliedBulkSessionRewrite {
            plan: BulkSessionRewritePlan {
                path: rollout.clone(),
                original_sha256,
                original_mtime: None,
                original_session_meta_lines: vec![incorrect_meta.to_string()],
            },
            rewritten_sha256,
        };

        let error = restore_bulk_session_rewrite(&change).unwrap_err();

        assert!(error
            .chain()
            .any(|cause| cause.to_string().contains("rollout rollback hash mismatch")));
        assert_eq!(fs::read(&rollout).unwrap(), rewritten.into_bytes());
    }
}

#[cfg(test)]
mod global_state_tests {
    use super::*;

    #[test]
    fn provider_sync_global_state_update_preserves_thread_sidebar_entries() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(".codex-global-state.json");
        let original = json!({
            "electron-saved-workspace-roots": ["\\\\?\\C:\\workspace"],
            "projectless-thread-ids": ["thread-1"],
            "thread-workspace-root-hints": {"thread-1": "C:/keep"},
            "electron-persisted-atom-state": {
                "thread-client-id-v1:thread-1": {"title": "keep"}
            }
        });
        fs::write(&path, serde_json::to_vec(&original).unwrap()).unwrap();

        assert_eq!(apply_global_state_update(&path).unwrap(), 1);

        let updated: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(updated["projectless-thread-ids"], original["projectless-thread-ids"]);
        assert_eq!(
            updated["thread-workspace-root-hints"],
            original["thread-workspace-root-hints"]
        );
        assert_eq!(
            updated["electron-persisted-atom-state"],
            original["electron-persisted-atom-state"]
        );
    }

    #[test]
    fn global_state_write_refuses_to_overwrite_newer_sidebar_entries() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(".codex-global-state.json");
        fs::write(
            &path,
            serde_json::to_vec(&json!({
                "electron-saved-workspace-roots": ["C:/old"]
            }))
            .unwrap(),
        )
        .unwrap();
        let (original_bytes, mut state) = global_state_snapshot(&path).unwrap();
        state.insert(
            "electron-saved-workspace-roots".to_string(),
            json!(["C:/normalized"]),
        );
        let newer_sidebar_state = json!({
            "projectless-thread-ids": ["thread-1"],
            "thread-workspace-root-hints": {"thread-1": "C:/keep"}
        });
        let newer_sidebar_bytes = serde_json::to_vec(&newer_sidebar_state).unwrap();
        fs::write(&path, &newer_sidebar_bytes).unwrap();

        let error = write_global_state_if_unchanged(&path, &original_bytes, &state).unwrap_err();

        assert!(error.to_string().contains(GLOBAL_STATE_SOURCE_CHANGED_ERROR));
        assert_eq!(fs::read(&path).unwrap(), newer_sidebar_bytes);
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

#[cfg(test)]
mod rollout_path_view_tests {
    use super::*;

    /// issue #1424：rollout 路径要按「原样 → 另一种视角」给候选，两种都试。
    #[test]
    fn candidates_include_both_path_views() {
        let home = Path::new("/home/user/.codex");

        let windows = rollout_path_candidates(home, "C:/work/sessions/rollout-a.jsonl");
        assert_eq!(
            windows,
            vec![
                PathBuf::from("C:/work/sessions/rollout-a.jsonl"),
                PathBuf::from("/mnt/c/work/sessions/rollout-a.jsonl"),
            ],
            "Windows 视角应同时给出 WSL 互转视角"
        );

        let wsl = rollout_path_candidates(home, "/mnt/d/work/sessions/rollout-b.jsonl");
        assert_eq!(
            wsl,
            vec![
                PathBuf::from("/mnt/d/work/sessions/rollout-b.jsonl"),
                PathBuf::from("D:/work/sessions/rollout-b.jsonl"),
            ],
            "WSL 视角应同时给出 Windows 互转视角"
        );
    }

    /// 相对路径按 home 拼接，仍然给出两种视角。
    #[test]
    fn candidates_join_relative_paths_to_home() {
        let home = Path::new("/home/user/.codex");

        assert_eq!(
            rollout_path_candidates(home, "sessions/rollout-c.jsonl"),
            vec![PathBuf::from("/home/user/.codex/sessions/rollout-c.jsonl")],
            "普通相对路径只有一种视角"
        );
    }

    /// 不属于两种视角形式的路径不会凭空造出第二个候选。
    #[test]
    fn candidates_keep_single_view_for_plain_paths() {
        let home = Path::new("/home/user/.codex");

        assert_eq!(
            rollout_path_candidates(home, "/var/data/rollout-d.jsonl"),
            vec![PathBuf::from("/var/data/rollout-d.jsonl")]
        );
    }

    /// 空 rollout_path 视为「没有 rollout 信息」，按旧行为返回 true（不算缺失候选）。
    #[test]
    fn catalog_rollout_path_exists_treats_blank_as_present() {
        let home = Path::new("/home/user/.codex");

        assert!(catalog_rollout_path_exists(home, ""));
        assert!(catalog_rollout_path_exists(home, "   "));
    }

    /// 两侧视角都确实不存在时才返回 false。
    #[test]
    fn catalog_rollout_path_exists_is_false_when_both_views_missing() {
        let home = Path::new("/definitely/not/here");

        assert!(!catalog_rollout_path_exists(
            home,
            "/mnt/z/nonexistent/rollout-e.jsonl"
        ));
    }
}

#[cfg(test)]
mod canonical_thread_rebuild_tests {
    use super::*;

    fn write_config(home: &Path, provider: &str) {
        fs::write(
            home.join("config.toml"),
            format!("model_provider = {provider:?}\n\n[model_providers.{provider:?}]\nname = {provider:?}\n"),
        )
        .unwrap();
    }

    fn write_rollout(home: &Path, thread_id: &str, provider: &str) {
        let path = home
            .join("sessions/2026/10/03")
            .join(format!("rollout-2026-10-03T10-00-00-{thread_id}.jsonl"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            format!(
                "{}\n{}\n",
                json!({
                    "type": "session_meta",
                    "payload": {
                        "id": thread_id,
                        "model_provider": provider,
                        "cwd": "C:/workspace"
                    }
                }),
                json!({"type": "event_msg", "payload": {"type": "user_message"}}),
            ),
        )
        .unwrap();
    }

    fn create_catalog_db(path: &Path, rows: &[(&str, &str)]) {
        let db = Connection::open(path).unwrap();
        db.execute(
            "CREATE TABLE local_thread_catalog (
                host_id TEXT NOT NULL, thread_id TEXT NOT NULL, display_title TEXT NOT NULL,
                source_created_at REAL NOT NULL, source_updated_at REAL NOT NULL,
                cwd TEXT NOT NULL, source_kind TEXT NOT NULL, source_detail TEXT,
                model_provider TEXT NOT NULL, git_branch TEXT,
                observation_sequence INTEGER NOT NULL,
                missing_candidate INTEGER NOT NULL DEFAULT 0, thread_source TEXT,
                PRIMARY KEY (host_id, thread_id))",
            [],
        )
        .unwrap();
        db.execute(
            "CREATE TABLE local_thread_catalog_hosts (host_id TEXT PRIMARY KEY, host_kind TEXT NOT NULL)",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO local_thread_catalog_hosts VALUES ('local', 'local')",
            [],
        )
        .unwrap();
        db.execute(
            "CREATE TABLE local_thread_catalog_metadata (id INTEGER PRIMARY KEY, catalog_revision INTEGER NOT NULL DEFAULT 0)",
            [],
        )
        .unwrap();
        db.execute("INSERT INTO local_thread_catalog_metadata VALUES (1, 0)", [])
            .unwrap();
        for (index, (thread_id, provider)) in rows.iter().enumerate() {
            db.execute(
                "INSERT INTO local_thread_catalog
                 (host_id, thread_id, display_title, source_created_at, source_updated_at,
                  cwd, source_kind, source_detail, model_provider, git_branch,
                  observation_sequence, missing_candidate, thread_source)
                 VALUES ('local', ?1, ?1, 100.0, 200.0, 'C:/workspace', 'vscode', ?2,
                         ?3, 'main', ?4, 0, 'user')",
                rusqlite::params![thread_id, "", provider, index as i64 + 1],
            )
            .unwrap();
        }
    }

    fn create_threads_db(path: &Path, ids: &[&str]) {
        let db = Connection::open(path).unwrap();
        db.execute(
            "CREATE TABLE threads (
                id TEXT PRIMARY KEY, model_provider TEXT, archived INTEGER,
                has_user_event INTEGER, cwd TEXT, title TEXT, rollout_path TEXT,
                source TEXT, created_at_ms INTEGER, updated_at_ms INTEGER,
                git_branch TEXT, thread_source TEXT)",
            [],
        )
        .unwrap();
        for id in ids {
            db.execute(
                "INSERT INTO threads VALUES (?1, 'custom', 0, 1, 'C:/workspace', ?1, '', 'vscode', 100, 200, 'main', 'user')",
                [id],
            )
            .unwrap();
        }
    }

    fn thread_ids(path: &Path) -> Vec<String> {
        let db = Connection::open(path).unwrap();
        let mut ids = db
            .prepare("SELECT id FROM threads ORDER BY id")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        ids.sort();
        ids
    }

    /// rollout 还在、canonical 行缺失时补建，只增不删。
    #[test]
    fn rebuilds_canonical_row_when_rollout_still_exists() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join(".codex");
        let sqlite_dir = home.join("sqlite");
        fs::create_dir_all(&sqlite_dir).unwrap();
        write_config(&home, "custom");

        let thread_id = "01a01579-4a5d-77e3-89c0-751d38ad21f8";
        write_rollout(&home, thread_id, "custom");
        create_threads_db(&home.join("state_5.sqlite"), &["already-there"]);
        create_catalog_db(&sqlite_dir.join("codex-dev.db"), &[(thread_id, "custom")]);

        let state_db = home.join("state_5.sqlite");
        let before = thread_ids(&state_db);

        let result = run_provider_sync(Some(&home));

        assert_eq!(
            result.status,
            ProviderSyncStatus::Synced,
            "{}",
            result.message
        );
        let after = thread_ids(&state_db);
        assert!(after.contains(&thread_id.to_string()), "缺失的 canonical 行应被补建：{after:?}");
        for id in before {
            assert!(after.contains(&id), "原有行不允许丢失：{id}");
        }
    }

    /// rollout 文件已不在时**不**补建——否则会造出点开就报错的假条目。
    #[test]
    fn does_not_rebuild_without_a_rollout_file() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join(".codex");
        let sqlite_dir = home.join("sqlite");
        fs::create_dir_all(&sqlite_dir).unwrap();
        write_config(&home, "custom");

        let thread_id = "01a01579-4a5d-77e3-89c0-751d38ad21f9";
        create_threads_db(&home.join("state_5.sqlite"), &[]);
        create_catalog_db(&sqlite_dir.join("codex-dev.db"), &[(thread_id, "custom")]);

        let state_db = home.join("state_5.sqlite");
        let _ = run_provider_sync(Some(&home));

        assert!(
            !thread_ids(&state_db).contains(&thread_id.to_string()),
            "没有 rollout 的会话不该被补成 canonical 行"
        );
    }

    /// issue #240 关键回归：别的计数器都为 0、只有「缺 canonical 行」这一件事可做时，
    /// 不能被「already up to date」提前 return 掉。
    #[test]
    fn rebuilds_even_when_nothing_else_needs_work() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join(".codex");
        let sqlite_dir = home.join("sqlite");
        fs::create_dir_all(&sqlite_dir).unwrap();
        write_config(&home, "custom");

        let thread_id = "01a01579-4a5d-77e3-89c0-751d38ad21fb";
        write_rollout(&home, thread_id, "custom");
        // threads 表存在但一行都没有：没有 provider/cwd 可改，也没有可删的 catalog 行。
        create_threads_db(&home.join("state_5.sqlite"), &[]);
        create_catalog_db(&sqlite_dir.join("codex-dev.db"), &[(thread_id, "custom")]);

        let state_db = home.join("state_5.sqlite");
        let result = run_provider_sync(Some(&home));

        assert!(
            thread_ids(&state_db).contains(&thread_id.to_string()),
            "只有补建这一件事可做时也必须执行（消息：{}）",
            result.message
        );
    }

    /// 已有 canonical 行的会话不会被补建成第二行（INSERT OR IGNORE + 只补缺失 id）。
    #[test]
    fn does_not_duplicate_existing_canonical_row() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join(".codex");
        let sqlite_dir = home.join("sqlite");
        fs::create_dir_all(&sqlite_dir).unwrap();
        write_config(&home, "custom");

        let thread_id = "01a01579-4a5d-77e3-89c0-751d38ad21fa";
        write_rollout(&home, thread_id, "custom");
        let state_db = home.join("state_5.sqlite");
        create_threads_db(&state_db, &[thread_id]);
        // 该会话同时出现在 catalog 里，但它已有 canonical 行，不该被补。
        create_catalog_db(&sqlite_dir.join("codex-dev.db"), &[(thread_id, "custom")]);

        let _ = run_provider_sync(Some(&home));

        let count: i64 = Connection::open(&state_db)
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM threads WHERE id = ?1",
                [thread_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "已存在的 canonical 行不能被补成两行");
    }
}

#[cfg(test)]
mod sync_diagnostics_tests {
    use super::*;

    fn write_config(home: &Path, provider: &str) {
        fs::write(
            home.join("config.toml"),
            format!(
                "model_provider = {provider:?}\n\n[model_providers.{provider:?}]\nname = {provider:?}\n"
            ),
        )
        .unwrap();
    }

    fn write_rollout(home: &Path, dirname: &str, thread_id: &str, provider: &str) {
        let path = home
            .join(dirname)
            .join("2026/10/03")
            .join(format!("rollout-2026-10-03T10-00-00-{thread_id}.jsonl"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            format!(
                "{}\n{}\n",
                json!({
                    "type": "session_meta",
                    "payload": {
                        "id": thread_id,
                        "model_provider": provider,
                        "cwd": "C:/workspace"
                    }
                }),
                json!({"type": "event_msg", "payload": {"type": "user_message"}}),
            ),
        )
        .unwrap();
    }

    /// issue #982：没有任何会话文件时，结果必须说明「扫了哪些目录、扫到 0 个文件」，
    /// 而不是只留一句无从下手的「0 个会话文件，0 行索引」。
    #[test]
    fn audit_reports_scanned_scope_without_rollout_files() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join(".codex");
        fs::create_dir_all(&home).unwrap();
        write_config(&home, "relay-alpha");

        let result = run_provider_sync(Some(&home));

        assert_eq!(result.repair_audit.scanned_rollout_files, 0);
        assert!(result.repair_audit.scanned_session_dirs.is_empty());
        assert!(
            result.message.contains("均不存在"),
            "{}",
            result.message
        );
        assert!(
            result.message.contains("rollout 文件 0 个"),
            "{}",
            result.message
        );
    }

    /// issue #982：会话文件已全部指向目标供应商时，诊断要能区分出
    /// 「有文件、但都已就绪」，不能和「一个文件都没有」显示成同一个样子。
    #[test]
    fn audit_distinguishes_already_on_target_from_no_files() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join(".codex");
        fs::create_dir_all(&home).unwrap();
        write_config(&home, "relay-alpha");
        // 两个文件都已指向 relay-alpha，同步后无任何改写计划。
        write_rollout(
            &home,
            "sessions",
            "01a01579-4a5d-77e3-89c0-751d38ad21fa",
            "relay-alpha",
        );
        write_rollout(
            &home,
            "archived_sessions",
            "01a01579-4a5d-77e3-89c0-751d38ad21fb",
            "relay-alpha",
        );

        let result = run_provider_sync(Some(&home));

        assert_eq!(result.repair_audit.scanned_rollout_files, 2);
        assert_eq!(result.repair_audit.rollout_files_already_on_target, 2);
        assert_eq!(result.changed_session_files, 0);
        assert_eq!(
            result.repair_audit.scanned_session_dirs,
            vec!["sessions".to_string(), "archived_sessions".to_string()]
        );
        assert!(
            result.message.contains("其中 2 个已指向目标供应商无需改写"),
            "{}",
            result.message
        );
    }

    /// 诊断字段只统计「已就绪」的文件，真正需要改写的不该被算进去。
    #[test]
    fn already_on_target_count_excludes_files_needing_rewrite() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join(".codex");
        fs::create_dir_all(&home).unwrap();
        write_config(&home, "relay-alpha");
        write_rollout(
            &home,
            "sessions",
            "01a01579-4a5d-77e3-89c0-751d38ad21fa",
            "relay-alpha",
        );
        write_rollout(
            &home,
            "sessions",
            "01a01579-4a5d-77e3-89c0-751d38ad21fb",
            "openai",
        );

        let result = run_provider_sync(Some(&home));

        assert_eq!(result.repair_audit.scanned_rollout_files, 2);
        assert_eq!(
            result.repair_audit.rollout_files_already_on_target, 1,
            "只有已是 relay-alpha 的那个文件算已就绪"
        );
        assert_eq!(result.changed_session_files, 1);
    }

    /// 全局状态解析失败时，给用户的必须是「哪个文件 + 怎么修」，
    /// 不能只说一句 serde_json 的 `expected value at line 1 column 1`（issue #2160）。
    #[test]
    fn global_state_parse_failure_names_the_file_and_the_fix() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join(".codex");
        fs::create_dir_all(&home).unwrap();
        write_config(&home, "relay-alpha");
        let global_state = home.join(".codex-global-state.json");
        assert_eq!(global_state.file_name().unwrap(), ".codex-global-state.json");
        // 空文件 / 被截断的文件：解析报 `expected value at line 1 column 1`。
        fs::write(&global_state, "{\"projectless-thread-ids\": [").unwrap();

        let result = run_provider_sync(Some(&home));

        assert_eq!(result.status, ProviderSyncStatus::Skipped);
        assert!(
            result.message.contains(".codex-global-state.json"),
            "报错要带上文件路径：{}",
            result.message
        );
        assert!(
            result.message.contains("合法 JSON"),
            "报错要给出可操作建议：{}",
            result.message
        );
        assert!(
            result.message.contains(GLOBAL_STATE_BACKUP_FILE_NAME),
            "报错要指出恢复来源：{}",
            result.message
        );
    }

    /// 预览同样要带上诊断字段，且依旧只读。
    #[test]
    fn preview_reports_scanned_scope_and_stays_read_only() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join(".codex");
        fs::create_dir_all(&home).unwrap();
        write_config(&home, "relay-alpha");
        write_rollout(
            &home,
            "sessions",
            "01a01579-4a5d-77e3-89c0-751d38ad21fa",
            "openai",
        );

        let preview = preview_provider_sync(Some(&home)).unwrap();

        assert_eq!(preview.audit.scanned_rollout_files, 1);
        assert_eq!(preview.audit.scanned_session_dirs, vec!["sessions".to_string()]);
        assert!(!home.join("tmp/provider-sync.lock").exists());
        assert!(!preview.backup_root.exists());
    }

    /// 可诊断字段不改变既有审计语义：没有 catalog-only 记录时仍然全为 0。
    #[test]
    fn diagnostics_do_not_change_catalog_only_semantics() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join(".codex");
        fs::create_dir_all(&home).unwrap();
        write_config(&home, "relay-alpha");

        let result = run_provider_sync(Some(&home));

        assert_eq!(result.repair_audit.catalog_only_sessions, 0);
        assert_eq!(result.repair_audit.catalog_only_without_recovery_source, 0);
    }
}

#[cfg(test)]
mod deleted_thread_tombstone_tests {
    use super::*;

    fn write_config(home: &Path, provider: &str) {
        fs::write(
            home.join("config.toml"),
            format!(
                "model_provider = {provider:?}\n\n[model_providers.{provider:?}]\nname = {provider:?}\n"
            ),
        )
        .unwrap();
    }

    fn write_rollout(home: &Path, thread_id: &str, provider: &str) {
        let path = home
            .join("sessions/2026/10/03")
            .join(format!("rollout-2026-10-03T10-00-00-{thread_id}.jsonl"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            format!(
                "{}\n{}\n",
                json!({
                    "type": "session_meta",
                    "payload": {
                        "id": thread_id,
                        "model_provider": provider,
                        "cwd": "C:/workspace"
                    }
                }),
                json!({"type": "event_msg", "payload": {"type": "user_message"}}),
            ),
        )
        .unwrap();
    }

    fn create_catalog_db(path: &Path, thread_id: &str, provider: &str) {
        let db = Connection::open(path).unwrap();
        db.execute(
            "CREATE TABLE IF NOT EXISTS local_thread_catalog (
                host_id TEXT NOT NULL, thread_id TEXT NOT NULL, display_title TEXT NOT NULL,
                source_created_at REAL NOT NULL, source_updated_at REAL NOT NULL,
                cwd TEXT NOT NULL, source_kind TEXT NOT NULL, source_detail TEXT,
                model_provider TEXT NOT NULL, git_branch TEXT,
                observation_sequence INTEGER NOT NULL,
                missing_candidate INTEGER NOT NULL DEFAULT 0, thread_source TEXT,
                PRIMARY KEY (host_id, thread_id))",
            [],
        )
        .unwrap();
        db.execute(
            "CREATE TABLE IF NOT EXISTS local_thread_catalog_hosts (host_id TEXT PRIMARY KEY, host_kind TEXT NOT NULL)",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT OR IGNORE INTO local_thread_catalog_hosts VALUES ('local', 'local')",
            [],
        )
        .unwrap();
        db.execute(
            "CREATE TABLE IF NOT EXISTS local_thread_catalog_metadata (id INTEGER PRIMARY KEY, catalog_revision INTEGER NOT NULL DEFAULT 0)",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT OR IGNORE INTO local_thread_catalog_metadata VALUES (1, 0)",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO local_thread_catalog
             (host_id, thread_id, display_title, source_created_at, source_updated_at,
              cwd, source_kind, source_detail, model_provider, git_branch,
              observation_sequence, missing_candidate, thread_source)
             VALUES ('local', ?1, ?1, 100.0, 200.0, 'C:/workspace', 'vscode', ?2,
                     'custom', 'main', 1, 0, 'user')",
            rusqlite::params![thread_id, ""],
        )
        .unwrap();
    }

    fn create_threads_db(path: &Path) {
        let db = Connection::open(path).unwrap();
        db.execute(
            "CREATE TABLE threads (
                id TEXT PRIMARY KEY, model_provider TEXT, archived INTEGER,
                has_user_event INTEGER, cwd TEXT, title TEXT, rollout_path TEXT,
                source TEXT, created_at_ms INTEGER, updated_at_ms INTEGER,
                git_branch TEXT, thread_source TEXT)",
            [],
        )
        .unwrap();
    }

    fn canonical_ids(path: &Path) -> Vec<String> {
        let db = Connection::open(path).unwrap();
        let mut ids = db
            .prepare("SELECT id FROM threads ORDER BY id")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        ids.sort();
        ids
    }

    fn catalog_ids(path: &Path) -> Vec<String> {
        let db = Connection::open(path).unwrap();
        let mut ids = db
            .prepare("SELECT thread_id FROM local_thread_catalog ORDER BY thread_id")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        ids.sort();
        ids
    }

    /// issue #2199 回归：删掉的会话不会在下一次索引重建里复活。
    ///
    /// 复现的是真实场景：删除只清掉了 catalog 行，canonical `threads` 行（或 rollout
    /// 文件）因为删除失败/被占用留了下来。旧实现的重建只看资格判定（rollout 还在
    /// 就算数），于是重建把行写回去，用户切页面回来会话又出现了。
    #[test]
    fn deleted_thread_stays_deleted_after_a_rebuild() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join(".codex");
        let sqlite_dir = home.join("sqlite");
        fs::create_dir_all(&sqlite_dir).unwrap();
        write_config(&home, "custom");

        let thread_id = "01a01579-4a5d-77e3-89c0-751d38ad21f8";
        write_rollout(&home, thread_id, "custom");
        let state_db = home.join("state_5.sqlite");
        let catalog_db = sqlite_dir.join("codex-dev.db");
        create_threads_db(&state_db);
        create_catalog_db(&catalog_db, thread_id, "custom");

        // 第一次同步把 canonical 行补齐（既有行为，不该被墓碑影响）。
        let first = run_provider_sync(Some(&home));
        assert_eq!(
            first.status,
            ProviderSyncStatus::Synced,
            "{}",
            first.message
        );
        assert!(
            canonical_ids(&state_db).contains(&thread_id.to_string()),
            "同步正常补建 canonical 行"
        );

        // 用户删除：清侧边栏引用（catalog 行），并记下墓碑。
        remove_thread_sidebar_references(&home, thread_id).unwrap();
        assert!(
            !catalog_ids(&catalog_db).contains(&thread_id.to_string()),
            "删除应清掉 catalog 行"
        );

        // 重建：rollout 文件仍在，canonical 行仍在，旧实现会把它写回列表。
        let second = run_provider_sync(Some(&home));
        assert_eq!(
            second.status,
            ProviderSyncStatus::Synced,
            "{}",
            second.message
        );

        // catalog 行不能回来（这是列表的数据源），canonical 行也不该被当成待补建项。
        assert!(
            !catalog_ids(&catalog_db).contains(&thread_id.to_string()),
            "被删的会话不得被重建回 catalog：{:?}",
            second.message
        );
        assert!(
            collect_rebuildable_canonical_threads(&home, &[state_db.clone()], "custom")
                .unwrap()
                .iter()
                .all(|thread| thread.id != thread_id),
            "被删的会话不得进入 canonical 补建候选"
        );

        // 再同步一次，反复重建也不会复活。
        let third = run_provider_sync(Some(&home));
        assert_eq!(
            third.status,
            ProviderSyncStatus::Synced,
            "{}",
            third.message
        );
        assert!(
            !catalog_ids(&catalog_db).contains(&thread_id.to_string()),
            "反复重建依然不得复活被删的会话"
        );
    }

    /// 墓碑只挡「被删的那条」：同一批同步里的其他会话照常补建。
    #[test]
    fn tombstone_only_suppresses_the_deleted_thread() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join(".codex");
        let sqlite_dir = home.join("sqlite");
        fs::create_dir_all(&sqlite_dir).unwrap();
        write_config(&home, "custom");

        let deleted = "01a01579-4a5d-77e3-89c0-751d38ad21f8";
        let kept = "01a01579-4a5d-77e3-89c0-751d38ad21f9";
        write_rollout(&home, deleted, "custom");
        write_rollout(&home, kept, "custom");
        let state_db = home.join("state_5.sqlite");
        let catalog_db = sqlite_dir.join("codex-dev.db");
        create_threads_db(&state_db);
        create_catalog_db(&catalog_db, deleted, "custom");
        create_catalog_db(&catalog_db, kept, "custom");

        remove_thread_sidebar_references(&home, deleted).unwrap();

        let result = run_provider_sync(Some(&home));
        assert_eq!(
            result.status,
            ProviderSyncStatus::Synced,
            "{}",
            result.message
        );
        let ids = catalog_ids(&catalog_db);
        assert!(!ids.contains(&deleted.to_string()), "被删的不复活：{ids:?}");
        assert!(ids.contains(&kept.to_string()), "未删的照常补建：{ids:?}");
    }

    /// 撤销删除（undo）必须清掉墓碑，否则恢复出来的会话会被一直挡在列表外。
    #[test]
    fn undo_clears_the_tombstone() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join(".codex");
        let sqlite_dir = home.join("sqlite");
        fs::create_dir_all(&sqlite_dir).unwrap();
        fs::write(home.join("session_index.jsonl"), "").unwrap();
        write_config(&home, "custom");

        let thread_id = "01a01579-4a5d-77e3-89c0-751d38ad21f8";
        write_rollout(&home, thread_id, "custom");
        let state_db = home.join("state_5.sqlite");
        let catalog_db = sqlite_dir.join("codex-dev.db");
        create_threads_db(&state_db);
        create_catalog_db(&catalog_db, thread_id, "custom");
        // 先做一次同步，让会话的 canonical 行就位（这是「已经在列表里」的状态）。
        let _ = run_provider_sync(Some(&home));
        assert!(canonical_ids(&state_db).contains(&thread_id.to_string()));

        remove_thread_sidebar_references(&home, thread_id).unwrap();
        assert!(
            load_thread_tombstones(&home).contains(thread_id),
            "删除应写下墓碑"
        );

        // undo 走的是恢复 session_index 这一条（storage 的恢复流程）。
        let line = json!({
            "id": thread_id,
            "thread_name": thread_id,
            "updated_at": "2026-10-03T10:00:00Z"
        })
        .to_string();
        assert_eq!(restore_session_index_entries(&home, &[line]).unwrap(), 1);

        assert!(
            !load_thread_tombstones(&home).contains(thread_id),
            "撤销删除后墓碑必须清掉"
        );

        // 撤销之后重建能把会话写回列表——墓碑不残留，撤销看起来才是生效的。
        // （canonical 行本来就没被删除流程碰过，这里验证的是它不再被墓碑压制。）
        let result = run_provider_sync(Some(&home));
        assert_eq!(
            result.status,
            ProviderSyncStatus::Synced,
            "{}",
            result.message
        );
        assert!(
            canonical_ids(&state_db).contains(&thread_id.to_string()),
            "撤销后会话应回到列表"
        );
    }

    /// 墓碑文件损坏 / 不存在都不能让同步失败——最坏只是退回「可能复活」。
    #[test]
    fn damaged_tombstone_file_is_ignored() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join(".codex");
        let sqlite_dir = home.join("sqlite");
        fs::create_dir_all(&sqlite_dir).unwrap();
        write_config(&home, "custom");

        let thread_id = "01a01579-4a5d-77e3-89c0-751d38ad21f8";
        write_rollout(&home, thread_id, "custom");
        create_threads_db(&home.join("state_5.sqlite"));
        create_catalog_db(&sqlite_dir.join("codex-dev.db"), thread_id, "custom");

        // 写一半的墓碑文件。
        fs::create_dir_all(home.join("tmp")).unwrap();
        fs::write(home.join(THREAD_TOMBSTONE_RELATIVE_PATH), "[{\"thr").unwrap();

        assert!(load_thread_tombstones(&home).is_empty());
        let result = run_provider_sync(Some(&home));
        assert_eq!(
            result.status,
            ProviderSyncStatus::Synced,
            "{}",
            result.message
        );
    }

    /// 墓碑按上限裁剪，只丢最旧的；裁剪不会删除任何会话数据。
    #[test]
    fn tombstones_are_capped_by_dropping_the_oldest() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join(".codex");
        fs::create_dir_all(&home).unwrap();

        for index in 0..MAX_THREAD_TOMBSTONES + 3 {
            record_thread_tombstone(&home, &format!("thread-{index:05}")).unwrap();
        }

        let ids = load_thread_tombstones(&home);
        assert_eq!(ids.len(), MAX_THREAD_TOMBSTONES);
        assert!(!ids.contains("thread-00000"), "最旧的墓碑先被丢掉");
        assert!(ids.contains(&format!("thread-{:05}", MAX_THREAD_TOMBSTONES + 2)));
    }

    /// 同一条会话重复删除只留一条墓碑。
    #[test]
    fn repeated_deletion_records_a_single_tombstone() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join(".codex");
        fs::create_dir_all(&home).unwrap();

        record_thread_tombstone(&home, "thread-1").unwrap();
        record_thread_tombstone(&home, "thread-1").unwrap();

        let ids: Vec<String> =
            serde_json::from_slice(&fs::read(thread_tombstone_path(&home)).unwrap()).unwrap();
        assert_eq!(ids, vec!["thread-1".to_string()]);
    }
}
