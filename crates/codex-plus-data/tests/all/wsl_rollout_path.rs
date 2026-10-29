//! issue #162 回归测试：WSL 模式下 `threads.rollout_path` 是 `/mnt/<盘符>/...`
//! 视角路径时，删除会话必须真正删除 rollout 文件，且撤销可以完整还原。

use codex_plus_core::models::{DeleteStatus, SessionRef};
use codex_plus_data::{BackupStore, SQLiteStorageAdapter};
use rusqlite::Connection;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

fn session(id: &str, title: &str) -> SessionRef {
    SessionRef::new(id, title).unwrap()
}

/// 把 Windows 路径转成 WSL 视角：C:/Users/a.jsonl -> /mnt/c/Users/a.jsonl
/// （复刻 WSL 模式下 codex 写入 sqlite 的路径形式）
fn wsl_view_of(path: &Path) -> String {
    let s = path.to_string_lossy().replace('\\', "/");
    let bytes = s.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' {
        let drive = s.chars().next().unwrap().to_ascii_lowercase();
        format!("/mnt/{}/{}", drive, s[2..].trim_start_matches('/'))
    } else {
        s
    }
}

fn create_thread_db(path: &Path, rollout_path: &str) {
    let db = Connection::open(path).unwrap();
    db.execute(
        "CREATE TABLE threads (id TEXT PRIMARY KEY, rollout_path TEXT, title TEXT, cwd TEXT, archived INTEGER, archived_at INTEGER, updated_at INTEGER, updated_at_ms INTEGER)",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO threads (id, rollout_path, title, cwd, archived, archived_at, updated_at, updated_at_ms) VALUES ('t1', ?1, 'WSL Thread', '/old/project', 0, NULL, 100, 100000)",
        [rollout_path.to_string()],
    )
    .unwrap();
}

fn thread_count(path: &Path) -> i64 {
    let db = Connection::open(path).unwrap();
    db.query_row("SELECT COUNT(*) FROM threads WHERE id = 't1'", [], |row| {
        row.get::<_, i64>(0)
    })
    .unwrap()
}

fn find_backup_jsons(dir: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                find_backup_jsons(&p, out);
            } else if p.extension().and_then(|e| e.to_str()) == Some("json") {
                out.push(p);
            }
        }
    }
}

/// 修复后：WSL 视角路径的 rollout 文件被真正删除，备份含 `__files`，撤销可还原。
#[cfg(windows)]
#[test]
fn wsl_rollout_path_delete_removes_file_and_undo_restores() {
    let holder = tempdir().unwrap();
    let rollout = holder.path().join("rollout.jsonl");
    fs::write(&rollout, "{\"type\":\"message\",\"content\":\"secret\"}\n").unwrap();
    let wsl_path = wsl_view_of(&rollout);

    let tmp = tempdir().unwrap();
    let db_path = tmp.path().join("state_5.sqlite");
    create_thread_db(&db_path, &wsl_path);
    let backups_dir = tmp.path().join("backups");
    let adapter = SQLiteStorageAdapter::new(&db_path, BackupStore::new(backups_dir.clone()));

    let deleted = adapter.delete_local(&session("local:t1", "WSL Thread"));

    assert_eq!(
        deleted.status,
        DeleteStatus::LocalDeleted,
        "{}",
        deleted.message
    );
    assert!(!rollout.exists(), "rollout 文件必须被真正删除");
    assert_eq!(thread_count(&db_path), 0, "数据库行必须被删除");

    // 备份必须包含 __files，且条目 path 指向可读写视角、保留原始 WSL 写法
    let mut backups = Vec::new();
    find_backup_jsons(&backups_dir, &mut backups);
    assert!(!backups.is_empty(), "删除必须产生备份");
    let body = fs::read_to_string(&backups[0]).unwrap();
    let backup: serde_json::Value = serde_json::from_str(&body).unwrap();
    let files = backup
        .pointer("/tables/__files")
        .and_then(|v| v.as_array())
        .expect("备份必须包含 __files");
    let entry = &files[0];
    let entry_path = entry.get("path").and_then(|v| v.as_str()).unwrap();
    assert_eq!(
        PathBuf::from(entry_path),
        rollout,
        "备份条目 path 指向真实文件"
    );
    assert_eq!(
        entry.get("source_path").and_then(|v| v.as_str()),
        Some(wsl_path.as_str()),
        "备份条目保留 DB 里的原始 WSL 写法"
    );

    // 撤销：数据库行与 rollout 文件都要能还原
    let restored = adapter.undo(deleted.undo_token.as_deref().unwrap());
    assert_eq!(
        restored.status,
        DeleteStatus::Undone,
        "{}",
        restored.message
    );
    assert_eq!(
        fs::read_to_string(&rollout).unwrap(),
        "{\"type\":\"message\",\"content\":\"secret\"}\n",
        "撤销后 rollout 文件按可读写视角写回原位"
    );
    assert_eq!(thread_count(&db_path), 1, "撤销后数据库行恢复");
}

/// 对照组：Windows 视角路径（当前进程可直接读到）的既有行为不变。
#[test]
fn control_windows_path_delete_succeeds_and_file_removed() {
    let tmp = tempdir().unwrap();
    let db_path = tmp.path().join("state_5.sqlite");
    let rollout = tmp.path().join("rollout.jsonl");
    fs::write(&rollout, "{\"type\":\"message\"}\n").unwrap();

    create_thread_db(&db_path, &rollout.to_string_lossy());
    let adapter = SQLiteStorageAdapter::new(&db_path, BackupStore::new(tmp.path().join("backups")));

    let deleted = adapter.delete_local(&session("local:t1", "control"));

    assert_eq!(
        deleted.status,
        DeleteStatus::LocalDeleted,
        "{}",
        deleted.message
    );
    assert!(!rollout.exists());
    assert_eq!(thread_count(&db_path), 0);
}

/// 兼容性：rollout 文件在两种视角下都不存在（例如早已被手动清理）时，
/// 删除仍然成功，只清理数据库行——与旧行为一致。
#[test]
fn missing_rollout_file_delete_still_succeeds() {
    let tmp = tempdir().unwrap();
    let db_path = tmp.path().join("state_5.sqlite");
    create_thread_db(&db_path, "/mnt/q/definitely/not/there/rollout.jsonl");
    let adapter = SQLiteStorageAdapter::new(&db_path, BackupStore::new(tmp.path().join("backups")));

    let deleted = adapter.delete_local(&session("local:t1", "missing"));

    assert_eq!(
        deleted.status,
        DeleteStatus::LocalDeleted,
        "{}",
        deleted.message
    );
    assert_eq!(thread_count(&db_path), 0);
}
