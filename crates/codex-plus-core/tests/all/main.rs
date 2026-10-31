//! codex-plus-core 集成测试的统一入口。
//!
//! 这些文件原本各是一个独立测试程序，每个都要把整个 core 和依赖完整链接一次，
//! cargo 还会逐个串行执行；合并成一个程序后只链接一次，各文件的用例在同一进程里并行。
//!
//! 合并带来的约束：
//! - 改进程级状态（设置路径覆盖、诊断日志路径、`CODEX_HOME` 等环境变量）的用例，
//!   一律先拿 [`common::global_state_lock`]，不要再在单个文件里另起一把锁。
//! - 会改 `CODEX_PLUS_STATE_DIR`、启动常驻 helper 单例或改代理端口环境变量的用例
//!   需要独占进程，仍作为独立程序留在 `tests/` 下（gateway_e2e、resident_helper、
//!   protocol_proxy_port_env）。
//!
//! 只跑某个文件的用例：`cargo test -p codex-plus-core --test all relay_config::`

mod common;

mod ads;
mod agent_collaboration;
mod agent_collaboration_integration;
mod agent_collaboration_queue;
mod agent_collaboration_refresh;
mod agent_pet_settings;
mod aggregate_default_login_sync;
mod anthropic_upstream;
mod api_quota_gate;
mod auto_takeover_live;
mod bridge_routes;
mod capacity_auto_retry_settings;
mod cdp_bridge;
mod codex_app_state;
mod codex_sqlite;
mod composer_readiness;
mod diagnostic_log;
mod dream_skin;
mod dream_skin_library;
mod dream_skin_market;
mod dream_skin_package;
mod dream_skin_runtime;
mod e2e_codex_real_cli;
mod floating_panel_appearance;
mod floating_panel_core_shell;
mod floating_panel_outline;
mod floating_panel_runtime;
mod floating_panel_stepwise;
mod force_chinese_locale_settings;
mod foreign_reasoning;
mod full_whale_contract;
mod gateway_account_proxy;
mod gateway_apply;
mod gateway_breaker;
mod gateway_codex_identity;
mod gateway_forced_relay;
mod gateway_key_limits;
mod gateway_meta_sync;
mod gateway_model_sync;
mod gateway_newapi_access;
mod gateway_provider_keys;
mod gateway_provider_probe;
mod gateway_rate_limit;
mod gateway_restore;
mod helper_body_limit_settings;
mod history_turn_recovery;
mod installers;
mod issue_20261018_compat;
mod issue_renderer_regressions;
mod issue_repair_safety;
mod launcher;
mod live_anthropic_upstream;
mod live_endpoint_conflict;
mod live_settings_migration;
mod local_compaction;
mod local_compaction_settings;
mod local_config_provider;
mod managed_session_provider;
mod map_unlisted_models_proxy;
mod map_unlisted_models_settings;
mod model_catalog;
mod model_catalog_paths;
mod model_group_identity;
mod model_suffix;
mod named_api_keys;
mod native_api_quota_gate;
mod native_browser_settings;
mod native_fast_settings;
mod native_fast_unlock;
mod native_predictions;
mod node_repl_flattened_profile;
mod official_usage_lifecycle;
mod paste_fix_settings;
mod plugin_install_persistence;
mod plugin_market_live;
mod plugin_navigation;
mod protocol_proxy;
mod protocol_safety;
mod provider_switch_isolation;
mod proxy_auto_retry_settings;
mod pure_api_provider_auth;
mod real_catalog_probe;
mod relay_config;
mod relay_config_safety;
mod relay_rotation;
mod relay_switch;
mod responses_catalog_identity;
mod responses_exec_compatibility;
mod retired_layout;
mod script_market;
mod settings_load_recursion;
mod side_chat;
mod skills_lifecycle;
mod ssh_remote_loopback;
mod standalone_quota_settings;
mod tool_shards;
mod typing_effect_settings;
mod ui_issue_regressions;
mod update_reminder_settings;
mod updater;
mod upstream_connection_reuse;
mod upstream_theme_assets;
mod upstream_worktree;
mod usage_status_line_settings;
mod user_scripts_reload;
mod watcher;
mod whale_settings;
mod windows_console_spawn;
mod zed_remote;
