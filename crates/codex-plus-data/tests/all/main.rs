//! codex-plus-data 集成测试的统一入口：各文件合并成一个测试程序，只链接一次。
//!
//! 只跑某个文件的用例：`cargo test -p codex-plus-data --test all provider_sync::`

mod deletion_blob_backup;
mod markdown;
mod provider_sync;
mod session_relocate;
mod storage_adapter;
mod storage_safety_regressions;
mod usage_candidate_db;
mod usage_history;
mod wsl_rollout_path;

/// 测试不许碰用户的真实目录：main 之前进家目录沙箱（见 `codex_plus_core::test_sandbox`）。
#[ctor::ctor]
fn enter_test_sandbox() {
    codex_plus_core::test_sandbox::enter();
}
