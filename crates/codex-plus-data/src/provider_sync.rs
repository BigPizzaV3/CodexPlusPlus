use rusqlite::{Connection, OptionalExtension, params_from_iter, types::Value as SqlValue};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_PROVIDER: &str = "openai";
const SESSION_DIRS: [&str; 2] = ["sessions", "archived_sessions"];
const BACKUP_KEEP_COUNT: usize = 5;
const REMOTE_CONTROL_CREATION_WINDOW_SECS: i64 = 15 * 60;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProviderSyncLockOwner {
    pid: u32,
    started_at: u64,
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
    original_text: String,
    next_text: String,
    original_session_meta_lines: Vec<String>,
    thread_id: Option<String>,
    cwd: Option<String>,
    has_user_event: bool,
    rewrite_needed: bool,
    original_mtime: Option<SystemTime>,
}

#[derive(Debug, Default)]
struct RolloutRewrite {
    next_text: String,
    rewrite_needed: bool,
    thread_id: Option<String>,
    cwd: Option<String>,
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
struct AppliedSessionChanges {
    changes: Vec<SessionChange>,
    skipped_locked_rollout_files: Vec<PathBuf>,
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
    catalog_non_root_thread_ids: HashMap<PathBuf, HashSet<String>>,
}

impl CatalogRepairPlan {
    fn has_cleanup_candidates(&self) -> bool {
        !self.non_root_thread_ids.is_empty()
            || self
                .catalog_non_root_thread_ids
                .values()
                .any(|thread_ids| !thread_ids.is_empty())
    }

    fn cleanup_thread_ids_for_path(&self, path: &Path) -> HashSet<String> {
        let mut thread_ids = self.non_root_thread_ids.clone();
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
    if acquire_lock(&lock_dir).is_err() {
        return result(
            ProviderSyncStatus::Skipped,
            format!("Provider sync lock exists: {}", lock_dir.to_string_lossy()),
            target_provider,
            None,
            0,
            0,
        );
    }
    let thread_ids = HashSet::from([thread_id.to_string()]);
    let recovery = run_remote_control_catalog_recovery_for_threads(
        &provider_sync_db_paths(&home),
        target_provider,
        &thread_ids,
    );
    let _ = release_lock(&lock_dir);
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
    if acquire_lock(&lock_dir).is_err() {
        return result(
            ProviderSyncStatus::Skipped,
            format!("Provider sync lock exists: {}", lock_dir.to_string_lossy()),
            target_provider,
            None,
            0,
            0,
        );
    }
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
    let _ = release_lock(&lock_dir);
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
    if acquire_lock(&lock_dir).is_err() {
        return result(
            ProviderSyncStatus::Skipped,
            format!("Provider sync lock exists: {}", lock_dir.to_string_lossy()),
            &target_provider,
            None,
            0,
            0,
        );
    }
    let sync_result = (|| -> anyhow::Result<ProviderSyncResult> {
        let sqlite_paths = provider_sync_db_paths(&home);
        let excluded_thread_ids = sqlite_subagent_thread_ids(&sqlite_paths)?;
        let collected = collect_session_changes(&home, &target_provider, &excluded_thread_ids)?;
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
        )?;
        let catalog_repair_count =
            count_local_thread_catalog_repairs(&sqlite_paths, &target_provider)?;
        let global_state_update_count =
            count_global_state_updates(&home.join(".codex-global-state.json"))?;
        if rewrite_changes.is_empty()
            && sqlite_update_count == 0
            && catalog_repair_count == 0
            && global_state_update_count == 0
        {
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
            return Ok(synced);
        }
        let backup_dir = create_backup(&home, &target_provider, &rewrite_changes)?;
        let applied = apply_session_changes(&rewrite_changes)?;
        let apply_result = (|| -> anyhow::Result<(SqliteUpdateCounts, usize)> {
            let sqlite_updates = apply_sqlite_update_for_paths(
                &sqlite_paths,
                &target_provider,
                &thread_ids_with_user_events,
                &cwd_by_thread_id,
            )?;
            let mut sqlite_updates = sqlite_updates;
            let catalog_repairs =
                repair_missing_local_thread_catalog_rows(&sqlite_paths, &target_provider)?;
            sqlite_updates.catalog_insert_rows = catalog_repairs.inserted_rows;
            sqlite_updates.catalog_remove_rows = catalog_repairs.removed_rows;
            let updated_workspace_roots =
                apply_global_state_update(&home.join(".codex-global-state.json"))?;
            prune_backups(&home)?;
            Ok((sqlite_updates, updated_workspace_roots))
        })();
        let (sqlite_updates, updated_workspace_roots) = match apply_result {
            Ok(counts) => counts,
            Err(err) => {
                let _ = restore_session_changes(&applied.changes);
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
        Ok(synced)
    })();
    let _ = release_lock(&lock_dir);
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
    }
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

fn acquire_lock(path: &Path) -> std::io::Result<()> {
    fs::create_dir_all(path.parent().unwrap_or_else(|| Path::new(".")))?;
    match create_lock(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let Some((owner, isolated_path)) = isolate_stale_lock(path) else {
                return Err(error);
            };
            match create_lock(path) {
                Ok(()) => {
                    let quarantine_cleanup_failed = fs::remove_dir_all(&isolated_path).is_err();
                    let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
                        "provider_sync.stale_lock_recovered",
                        json!({
                            "owner_pid": owner.pid,
                            "owner_started_at": owner.started_at,
                            "quarantine_cleanup_failed": quarantine_cleanup_failed,
                        }),
                    );
                    Ok(())
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

fn create_lock(path: &Path) -> std::io::Result<()> {
    fs::create_dir(path)?;
    let write_result = fs::write(
        path.join("owner.json"),
        json!({"pid": std::process::id(), "startedAt": now_secs()}).to_string(),
    );
    if let Err(error) = write_result {
        let _ = fs::remove_dir_all(path);
        return Err(error);
    }
    Ok(())
}

fn isolate_stale_lock(path: &Path) -> Option<(ProviderSyncLockOwner, PathBuf)> {
    let owner = serde_json::from_slice::<ProviderSyncLockOwner>(
        &fs::read(path.join("owner.json")).ok()?,
    )
    .ok()?;
    if codex_plus_core::watcher::process_id_is_running(owner.pid) != Some(false) {
        return None;
    }
    let file_name = path.file_name()?.to_string_lossy();
    let isolated_path = path.with_file_name(format!(
        "{file_name}.stale-{}-{}",
        owner.pid,
        uuid::Uuid::new_v4()
    ));
    fs::rename(path, &isolated_path).ok()?;
    Some((owner, isolated_path))
}

fn release_lock(path: &Path) -> std::io::Result<()> {
    if path.exists() {
        fs::remove_dir_all(path)?;
    }
    Ok(())
}

fn collect_session_changes(
    home: &Path,
    target_provider: &str,
    excluded_thread_ids: &HashSet<String>,
) -> anyhow::Result<SessionChanges> {
    let mut collected = SessionChanges::default();
    for path in rollout_files(home)? {
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if is_locked_io_error(&error) => {
                collected.skipped_locked_rollout_files.push(path);
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        let rewrite = rewrite_rollout_session_meta_providers(&text, target_provider)?;
        if rewrite.session_meta_count == 0 {
            continue;
        }
        if rewrite
            .thread_id
            .as_ref()
            .is_some_and(|thread_id| excluded_thread_ids.contains(thread_id))
        {
            continue;
        }
        let has_user_event = text.contains("\"user_message\"") || text.contains("\"user_input\"");
        if text.contains("encrypted_content") {
            for provider in &rewrite.providers {
                *collected
                    .encrypted_content_counts
                    .entry(provider.clone())
                    .or_insert(0) += 1;
            }
        }
        let original_mtime = fs::metadata(&path).and_then(|m| m.modified()).ok();
        collected.changes.push(SessionChange {
            path,
            original_text: text,
            next_text: rewrite.next_text,
            original_session_meta_lines: rewrite.original_session_meta_lines,
            thread_id: rewrite.thread_id,
            cwd: rewrite.cwd,
            has_user_event,
            rewrite_needed: rewrite.rewrite_needed,
            original_mtime,
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
    let has_user_event = text.contains("\"user_message\"") || text.contains("\"user_input\"");
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
        thread_id: rewrite.thread_id,
        cwd: rewrite.cwd,
        has_user_event,
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

fn rewrite_rollout_session_meta_providers(
    text: &str,
    target_provider: &str,
) -> anyhow::Result<RolloutRewrite> {
    let mut rewrite = RolloutRewrite::default();
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
                        .unwrap_or("(missing)")
                        .to_string();
                    rewrite.providers.push(provider);
                    if payload.get("model_provider").and_then(Value::as_str)
                        != Some(target_provider)
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
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if is_locked_io_error(&error) => continue,
            Err(error) => return Err(error.into()),
        };
        for segment in text.split_inclusive('\n') {
            let (line, _) = split_line_ending(segment);
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
    acquire_lock(&lock_dir).map_err(|error| cleanup_apply_error(error, None))?;
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
    let _ = release_lock(&lock_dir);
    result
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
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if is_locked_io_error(&error) => continue,
            Err(error) => return Err(error.into()),
        };
        for segment in text.split_inclusive('\n') {
            let (line, _) = split_line_ending(segment);
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
    let manifest = changes
        .iter()
        .map(|change| {
            json!({
                "path": change.path.to_string_lossy(),
                "originalSessionMetaLines": change.original_session_meta_lines,
            })
        })
        .collect::<Vec<_>>();
    fs::write(
        backup_dir.join("session-meta-backup.json"),
        serde_json::to_string_pretty(&manifest)?,
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
            "changedSessionFiles": changes.len(),
            "managedBy": "Codex++ provider sync"
        }))?,
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

fn restore_session_changes(changes: &[SessionChange]) -> anyhow::Result<()> {
    for change in changes {
        if replace_session_text_if_unchanged(
            &change.path,
            &change.next_text,
            &change.original_text,
        )? {
            restore_file_mtime(&change.path, change.original_mtime);
        }
    }
    Ok(())
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

fn sqlite_subagent_thread_ids(paths: &[PathBuf]) -> anyhow::Result<HashSet<String>> {
    let mut ids = HashSet::new();
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
            ids.extend(
                db.prepare(&sql)?
                    .query_map([], |row| row.get::<_, String>(0))?
                    .collect::<rusqlite::Result<HashSet<_>>>()?,
            );
        }
    }
    Ok(ids)
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

fn count_sqlite_updates(
    path: &Path,
    target_provider: &str,
    user_event_thread_ids: &HashSet<String>,
    cwd_by_thread_id: &HashMap<String, String>,
) -> anyhow::Result<usize> {
    if !path.exists() {
        return Ok(0);
    }
    let db = Connection::open(path)?;
    let columns = table_columns(&db, "threads")?;
    let catalog_columns = table_columns(&db, "local_thread_catalog")?;
    let thread_filter = subagent_filter(&db, "threads.id")?;
    let catalog_filter = subagent_filter(&db, "local_thread_catalog.thread_id")?;
    let mut total = 0;
    if columns.contains("model_provider") {
        total += db.query_row(
            &format!("SELECT COUNT(*) FROM threads WHERE COALESCE(model_provider, '') <> ?1{thread_filter}"),
            [target_provider],
            |row| row.get::<_, i64>(0),
        )? as usize;
    }
    if catalog_columns.contains("model_provider") {
        total += db.query_row(
            &format!("SELECT COUNT(*) FROM local_thread_catalog WHERE COALESCE(model_provider, '') <> ?1{catalog_filter}"),
            [target_provider],
            |row| row.get::<_, i64>(0),
        )? as usize;
    }
    if columns.contains("has_user_event") {
        for thread_id in user_event_thread_ids {
            total += db.query_row(
                "SELECT COUNT(*) FROM threads WHERE id = ?1 AND COALESCE(has_user_event, 0) <> 1",
                [thread_id],
                |row| row.get::<_, i64>(0),
            )? as usize;
        }
    }
    if columns.contains("cwd") {
        for (thread_id, cwd) in cwd_by_thread_id {
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
) -> anyhow::Result<usize> {
    let mut total = 0;
    for path in paths {
        total += count_sqlite_updates(
            path,
            target_provider,
            user_event_thread_ids,
            cwd_by_thread_id,
        )?;
    }
    Ok(total)
}

fn apply_sqlite_update(
    path: &Path,
    target_provider: &str,
    user_event_thread_ids: &HashSet<String>,
    cwd_by_thread_id: &HashMap<String, String>,
) -> anyhow::Result<SqliteUpdateCounts> {
    if !path.exists() {
        return Ok(SqliteUpdateCounts::default());
    }
    let mut db = Connection::open(path)?;
    let columns = table_columns(&db, "threads")?;
    let catalog_columns = table_columns(&db, "local_thread_catalog")?;
    let thread_filter = subagent_filter(&db, "threads.id")?;
    let catalog_filter = subagent_filter(&db, "local_thread_catalog.thread_id")?;
    if !columns.contains("model_provider") && !catalog_columns.contains("model_provider") {
        return Ok(SqliteUpdateCounts::default());
    }
    let tx = db.transaction()?;
    let mut counts = SqliteUpdateCounts::default();
    if columns.contains("model_provider") {
        counts.provider_rows += tx.execute(
            &format!("UPDATE threads SET model_provider = ?1 WHERE COALESCE(model_provider, '') <> ?1{thread_filter}"),
            [target_provider],
        )?;
    }
    if catalog_columns.contains("model_provider") {
        counts.provider_rows += tx.execute(
            &format!("UPDATE local_thread_catalog SET model_provider = ?1 WHERE COALESCE(model_provider, '') <> ?1{catalog_filter}"),
            [target_provider],
        )?;
    }
    if columns.contains("has_user_event") {
        for thread_id in user_event_thread_ids {
            counts.user_event_rows += tx.execute(
                "UPDATE threads SET has_user_event = 1 WHERE id = ?1 AND COALESCE(has_user_event, 0) <> 1",
                [thread_id],
            )?;
        }
    }
    if columns.contains("cwd") {
        for (thread_id, cwd) in cwd_by_thread_id {
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
) -> anyhow::Result<SqliteUpdateCounts> {
    let mut total = SqliteUpdateCounts::default();
    for path in paths {
        total.add(apply_sqlite_update(
            path,
            target_provider,
            user_event_thread_ids,
            cwd_by_thread_id,
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
    paths: &[PathBuf],
    target_provider: &str,
) -> anyhow::Result<usize> {
    let plan = collect_catalog_repair_plan(paths, target_provider, None)?;
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
    paths: &[PathBuf],
    target_provider: &str,
) -> anyhow::Result<CatalogRepairCounts> {
    repair_missing_local_thread_catalog_rows_filtered(paths, target_provider, None, true)
}

fn repair_missing_local_thread_catalog_rows_for_threads(
    paths: &[PathBuf],
    target_provider: &str,
    thread_ids: &HashSet<String>,
) -> anyhow::Result<CatalogRepairCounts> {
    repair_missing_local_thread_catalog_rows_filtered(
        paths,
        target_provider,
        Some(thread_ids),
        false,
    )
}

fn repair_missing_local_thread_catalog_rows_filtered(
    paths: &[PathBuf],
    target_provider: &str,
    thread_ids: Option<&HashSet<String>>,
    update_full_sync_state: bool,
) -> anyhow::Result<CatalogRepairCounts> {
    let plan = collect_catalog_repair_plan(paths, target_provider, thread_ids)?;
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
    paths: &[PathBuf],
    target_provider: &str,
    thread_ids: Option<&HashSet<String>>,
) -> anyhow::Result<CatalogRepairPlan> {
    let spawned_child_ids = collect_spawned_child_thread_ids(paths)?;
    let mut catalog_non_root_thread_ids =
        collect_catalog_marked_non_root_thread_ids(paths, &spawned_child_ids)?;
    let mut threads = HashMap::new();
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
        let subagent_filter = subagent_filter(&db, "threads.id")?;
        let sql = format!(
            "SELECT id, {display_title}, {source_created_at}, {source_updated_at}, {cwd}, {source_kind}, {source_detail}, {git_branch}, {thread_source} FROM threads WHERE COALESCE(id, '') <> ''{subagent_filter}"
        );
        let mut stmt = db.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            Ok(CatalogRepairThread {
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
            })
        })?;
        for item in rows {
            let thread = item?;
            let replace = threads
                .get(&thread.id)
                .map(|current: &CatalogRepairThread| {
                    thread.source_updated_at > current.source_updated_at
                })
                .unwrap_or(true);
            if replace {
                threads.insert(thread.id.clone(), thread);
            }
        }
    }
    if let Some(thread_ids) = thread_ids {
        threads.retain(|thread_id, _| thread_ids.contains(thread_id));
    }
    let explicit_user_thread_ids = threads
        .values()
        .filter(|thread| thread_source_is_user(thread.thread_source.as_deref()))
        .map(|thread| thread.id.clone())
        .collect::<HashSet<_>>();
    let non_root_thread_ids = threads
        .values()
        .filter(|thread| is_catalog_non_root_agent(thread, &spawned_child_ids))
        .map(|thread| thread.id.clone())
        .collect::<HashSet<_>>();
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
    threads.retain(|thread_id, _| !non_root_thread_ids.contains(thread_id));
    Ok(CatalogRepairPlan {
        threads,
        non_root_thread_ids,
        catalog_non_root_thread_ids,
    })
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
            if thread_source_is_user(thread_source.as_deref()) {
                continue;
            }
            if thread_source_marks_non_root(thread_source.as_deref())
                || source_marks_non_root_agent(&source_kind)
                || spawned_child_ids.contains(&thread_id)
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
    // The explicit user marker is authoritative over legacy source and spawn-edge fallbacks.
    if thread_source_is_user(thread.thread_source.as_deref()) {
        return false;
    }
    thread_source_marks_non_root(thread.thread_source.as_deref())
        || source_marks_non_root_agent(&thread.source_kind)
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
    match serde_json::from_str::<Value>(source) {
        Ok(Value::Object(object)) => {
            object.contains_key("sub_agent")
                || object.contains_key("subagent")
                || object.contains_key("internal")
        }
        Ok(Value::String(value)) => source_text_marks_non_root_agent(&value),
        _ => false,
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
