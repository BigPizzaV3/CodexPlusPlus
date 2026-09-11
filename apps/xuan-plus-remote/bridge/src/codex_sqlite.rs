use std::path::{Path, PathBuf};

pub fn codex_db_candidate_paths_from_home(home: &Path) -> Vec<PathBuf> {
    let root = std::env::var_os("CODEX_SQLITE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
        .unwrap_or_else(|| home.to_path_buf());
    vec![
        root.join("sqlite").join("codex-dev.db"),
        root.join("state_5.sqlite"),
    ]
}
