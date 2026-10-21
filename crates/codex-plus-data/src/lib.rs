pub mod backup;
pub mod markdown;
pub mod provider_sync;
pub mod session_index_repair;
mod session_index_scan;
pub mod storage;
mod sync_journal;
pub mod whale_history;
pub mod whale_usage;
pub use session_index_repair::{
    SessionIndexRepairReport, load_session_index_repair_report, repair_session_index,
};

pub use backup::BackupStore;
pub use markdown::{MarkdownExportService, export_markdown_from_paths};
pub use provider_sync::{
    ProviderSyncAudit, ProviderSyncDatabaseOutcome, ProviderSyncLifecycleGuard,
    ProviderSyncLockState, ProviderSyncProgress, ProviderSyncProgressPhase,
    ProviderSyncRecoveryReport, ProviderSyncResult, ProviderSyncStatus, ProviderSyncTargetList,
    ProviderSyncTargetOption, ProviderSyncTargetSource, SessionIndexCleanupApplyError,
    SessionIndexCleanupCandidate, SessionIndexCleanupPreview, SessionIndexCleanupResult,
    ThreadSidebarCleanupResult, apply_session_index_cleanup, inspect_provider_sync_lock,
    load_provider_sync_targets, pending_recovery_journal, preview_session_index_cleanup,
    recover_provider_sync_from_backup, remote_control_session_recovery_candidate_exists,
    remove_thread_sidebar_references, run_provider_sync, run_provider_sync_with_target,
    run_provider_sync_with_target_and_progress,
    run_remote_control_session_catalog_recovery_for_thread_with_target,
    run_remote_control_session_finalization_for_thread_with_target,
    try_acquire_provider_sync_lifecycle_guard, validate_provider_sync_target,
};
pub use storage::{
    LocalSession, LocalSessionPage, LocalSessionPager, SQLiteStorageAdapter, delete_local_from_paths,
    local_session_titles,
};
