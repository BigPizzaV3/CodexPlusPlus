pub mod ads;
pub mod app_paths;
pub mod assets;
pub mod bridge;
pub mod ccs_import;
pub mod config_coordinator;
pub mod cdp;
pub mod cli_wrapper;
pub mod diagnostic_log;
pub mod http_client;
pub mod install;
pub mod launcher;
pub mod model_catalog;
pub mod models;
pub mod paths;
pub mod ports;
pub mod protocol_proxy;
pub mod proxy;
pub mod relay_config;
pub mod relay_rotation;
pub mod routes;
pub mod script_market;
pub mod settings;
pub mod status;
pub mod update;
pub mod upstream_worktree;
pub mod user_scripts;
pub mod version;
pub mod watcher;
#[cfg(windows)]
mod windows_integration;
pub mod zed_remote;

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
