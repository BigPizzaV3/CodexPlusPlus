use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::Connection;

pub fn default_codex_home_dir() -> PathBuf {
    crate::codex_home::default_codex_home_dir()
}

pub fn codex_session_db_path() -> PathBuf {
    codex_session_db_path_from_home(&default_codex_home_dir())
}

pub fn codex_session_db_path_from_home(home: &Path) -> PathBuf {
    codex_sqlite_dir_session_dbs(home)
        .into_iter()
        .next()
        .unwrap_or_else(|| legacy_state_db_path(home))
}

pub fn codex_session_db_paths_from_home(home: &Path) -> Vec<PathBuf> {
    let mut paths = codex_sqlite_dir_session_dbs(home);
    let legacy = legacy_state_db_path(home);
    if !paths.iter().any(|path| path == &legacy) {
        paths.push(legacy);
    }
    paths
}

pub fn codex_sqlite_sidecar_paths(db_path: &Path) -> [PathBuf; 3] {
    [
        db_path.to_path_buf(),
        PathBuf::from(format!("{}-wal", db_path.to_string_lossy())),
        PathBuf::from(format!("{}-shm", db_path.to_string_lossy())),
    ]
}

pub fn relative_to_codex_home(home: &Path, path: &Path) -> PathBuf {
    path.strip_prefix(home).unwrap_or(path).to_path_buf()
}

fn legacy_state_db_path(home: &Path) -> PathBuf {
    home.join("state_5.sqlite")
}

fn codex_sqlite_dir_session_dbs(home: &Path) -> Vec<PathBuf> {
    let sqlite_dir = home.join("sqlite");
    let Ok(entries) = fs::read_dir(sqlite_dir) else {
        return Vec::new();
    };
    let mut candidates = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter(|path| is_sqlite_candidate(path))
        .filter(|path| has_session_table(path))
        .collect::<Vec<_>>();
    candidates.sort_by_key(|path| {
        (
            path.file_name()
                .map(|name| name != OsStr::new("codex-dev.db"))
                .unwrap_or(true),
            path.file_name().map(|name| name.to_os_string()),
        )
    });
    candidates
}

fn is_sqlite_candidate(path: &Path) -> bool {
    matches!(
        path.extension().and_then(OsStr::to_str),
        Some("db") | Some("sqlite") | Some("sqlite3")
    )
}

fn has_session_table(path: &Path) -> bool {
    let Ok(db) = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
    else {
        return false;
    };
    ["threads", "automation_runs", "inbox_items"]
        .iter()
        .any(|table| {
            db.query_row(
                "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1 LIMIT 1",
                [table],
                |_| Ok(()),
            )
            .is_ok()
        })
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SanitizeModelSuffixResult {
    pub scanned: usize,
    pub updated: usize,
}

/// 扫描 codex session 数据库中的 threads 表，把 model 字段里带合法后缀的
/// 记录改写为剥离后缀的 slug，使 codex 模型选择器不再显示带后缀的历史项。
pub fn sanitize_thread_model_suffixes(home: &Path) -> anyhow::Result<SanitizeModelSuffixResult> {
    let mut result = SanitizeModelSuffixResult::default();
    for db_path in codex_session_db_paths_from_home(home) {
        if !db_path.exists() {
            continue;
        }
        let (scanned, updated) = sanitize_thread_model_suffixes_in_db(&db_path)?;
        result.scanned += scanned;
        result.updated += updated;
    }
    Ok(result)
}

fn sanitize_thread_model_suffixes_in_db(db_path: &Path) -> anyhow::Result<(usize, usize)> {
    let mut conn = Connection::open(db_path)?;
    let tx = conn.transaction()?;
    let has_model = tx
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'threads' LIMIT 1",
            [],
            |_| Ok(()),
        )
        .is_ok()
        && tx
            .query_row(
                "SELECT 1 FROM pragma_table_info('threads') WHERE name = 'model' LIMIT 1",
                [],
                |_| Ok(()),
            )
            .is_ok();
    if !has_model {
        return Ok((0, 0));
    }

    let mut stmt = tx.prepare("SELECT id, model FROM threads WHERE model LIKE '%[%'")?;
    let rows: Vec<(String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .filter_map(Result::ok)
        .collect();
    drop(stmt);

    let scanned = rows.len();
    let mut updated = 0;
    for (id, model) in rows {
        let (slug, suffix_window) = crate::model_suffix::parse_model_suffix(&model);
        if suffix_window.is_some() && slug != model {
            tx.execute("UPDATE threads SET model = ?1 WHERE id = ?2", [&slug, &id])?;
            updated += 1;
        }
    }
    tx.commit()?;
    Ok((scanned, updated))
}
