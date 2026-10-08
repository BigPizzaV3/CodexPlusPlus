pub mod ads;
pub mod agent_cache;
pub mod app_paths;
pub mod assets;
pub mod bridge;
pub mod ccs_import;
pub mod cdp;
pub mod channel_protection;
pub mod codex_app_state;
pub mod codex_home;
pub mod codex_local_storage;
pub mod codex_sqlite;
pub mod connect;
pub mod diagnostic_log;
pub mod dictation;
pub mod dream_skin;
pub mod dream_skin_community;
pub mod dream_skin_library;
pub mod dream_skin_market;
pub mod dream_skin_package;
pub mod dream_skin_runtime;
pub mod env_conflicts;
pub mod grok_config;
pub mod http_client;
pub mod install;
pub mod launcher;
pub mod manager_navigation;
pub mod mcp_config;
pub mod model_catalog;
pub mod model_suffix;
pub mod models;
pub mod native_browser;
pub mod native_browser_connection;
pub mod paths;
pub mod ports;
// 独立插件市场按需下载安装，不调用原生授权安装流程。
pub mod plugin_market;
pub mod protocol_proxy;
pub mod provider_import;
pub mod proxy;
pub mod relay_config;
pub mod relay_environment;
pub mod relay_headers;
pub mod relay_rotation;
pub mod relay_switch;
pub mod remote_control_recovery;
pub mod routes;
pub mod script_market;
pub mod session_share;
pub mod settings;
pub mod share;
pub mod skills;
pub mod status;
pub mod stepwise;
pub mod sub2api;
pub mod tools;
pub mod update;
pub mod user_scripts;
pub mod version;
pub mod vision;
pub mod watcher;
pub mod whale;
pub mod whale_full;
// 不加 `#[cfg(windows)]`：模块内部各项已各自标注平台门控，在非 Windows 平台上
// 是一个只含少数无平台依赖项（如 current_process_is_elevated 的桩实现）的空模块。
// 门控在模块级会导致 `if cfg!(windows)` 这类运行时分支在非 Windows 平台找不到符号。
mod windows_integration;

#[cfg(windows)]
pub fn windows_create_no_window() -> u32 {
    windows_integration::CREATE_NO_WINDOW
}

#[cfg(windows)]
pub fn windows_open_url(url: &str) -> anyhow::Result<()> {
    windows_integration::open_url(url)
}

#[cfg(windows)]
pub fn windows_activate_process_window(process_id: u32) -> bool {
    windows_integration::activate_process_window(process_id)
}

#[cfg(windows)]
pub fn windows_apply_codexplusplus_icon_to_process_window(
    process_id: u32,
    icon_resource_path: std::path::PathBuf,
) -> bool {
    windows_integration::apply_codexplusplus_icon_to_process_window(process_id, icon_resource_path)
}

#[cfg(windows)]
pub fn windows_enumerate_processes() -> Vec<windows_integration::WindowsProcessInfo> {
    windows_integration::enumerate_processes()
}
