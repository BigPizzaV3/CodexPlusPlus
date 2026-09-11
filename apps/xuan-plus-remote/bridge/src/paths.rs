use std::path::PathBuf;

pub fn default_remote_state_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("XUAN_REMOTE_STATE_DIR") {
        let path = PathBuf::from(path);
        if !path.as_os_str().is_empty() {
            return path;
        }
    }
    if let Some(path) = std::env::var_os("XUAN_HOME") {
        let path = PathBuf::from(path);
        if !path.as_os_str().is_empty() {
            return path.join("xuan-plus-remote");
        }
    }
    if cfg!(windows)
        && let Some(roaming) = std::env::var_os("APPDATA")
    {
        return PathBuf::from(roaming).join("XuanPlusPlus").join("remote");
    }
    directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().join(".xuan-plus").join("remote"))
        .unwrap_or_else(|| PathBuf::from(".xuan-plus").join("remote"))
}

pub fn default_diagnostic_log_path() -> PathBuf {
    default_remote_state_dir().join("xuan-plus-remote.log")
}
