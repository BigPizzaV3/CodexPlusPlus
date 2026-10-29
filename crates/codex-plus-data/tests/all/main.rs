//! codex-plus-data 集成测试的统一入口：各文件合并成一个测试程序，只链接一次。
//!
//! 只跑某个文件的用例：`cargo test -p codex-plus-data --test all provider_sync::`

mod deletion_blob_backup;
mod markdown;
mod provider_sync;
mod storage_adapter;
mod storage_safety_regressions;
mod usage_candidate_db;
mod wsl_rollout_path;
