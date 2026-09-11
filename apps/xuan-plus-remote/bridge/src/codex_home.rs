use std::path::PathBuf;

pub fn default_codex_home_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("CODEX_HOME") {
        let path = PathBuf::from(path);
        if path.is_dir() {
            return path;
        }
    }
    directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().join(".codex"))
        .unwrap_or_else(|| PathBuf::from(".codex"))
}
