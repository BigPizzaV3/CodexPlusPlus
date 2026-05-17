pub mod ads;
pub mod app_paths;
pub mod assets;
pub mod bridge;
pub mod cdp;
pub mod cli_wrapper;
pub mod install;
pub mod launcher;
pub mod models;
pub mod paths;
pub mod ports;
pub mod proxy;
pub mod relay_config;
pub mod routes;
pub mod settings;
pub mod status;
pub mod update;
pub mod user_scripts;
pub mod version;
pub mod watcher;
#[cfg(windows)]
mod windows_integration;

#[cfg(windows)]
pub fn windows_create_no_window() -> u32 {
    windows_integration::CREATE_NO_WINDOW
}

#[cfg(windows)]
pub fn windows_open_url(url: &str) -> anyhow::Result<()> {
    windows_integration::open_url(url)
}
