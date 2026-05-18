use codex_plus_core::models::{DeleteStatus, SessionRef};
use codex_plus_data::{BackupStore, SQLiteStorageAdapter};
use rusqlite::Connection;
use serde_json::json;
use std::fs;
use std::path::Path;
use tempfile::tempdir;

fn session(id: &str, title: &str) -> SessionRef {
    SessionRef::new(id, title).unwrap()
}

fn create_supported_db(path: &Path) {
    let db = Connection::open(path).unwrap();
    db.execute(
        "CREATE TABLE sessions (id TEXT PRIMARY KEY, title TEXT NOT NULL)",
        [],
    )
    .unwrap();
    db.execute(
        "CREATE TABLE messages (id INTEGER PRIMARY KEY, session_id TEXT NOT NULL, body TEXT NOT NULL)",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO sessions (id, title) VALUES ('s1', 'First')",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO messages (session_id, body) VALUES ('s1', 'hello')",
        [],
    )
    .unwrap();
}

fn create_codex_thread_db(path: &Path, rollout_path: &Path) {
    let db = Connection::open(path).unwrap();
    db.execute("CREATE TABLE threads (id TEXT PRIMARY KEY, rollout_path TEXT, title TEXT, cwd TEXT, archived INTEGER, archived_at INTEGER, updated_at INTEGER, updated_at_ms INTEGER)", []).unwrap();
    db.execute(
        "CREATE TABLE thread_dynamic_tools (thread_id TEXT NOT NULL, tool_name TEXT NOT NULL)",
        [],
    )
    .unwrap();
    db.execute(
        "CREATE TABLE thread_goals (thread_id TEXT NOT NULL, goal TEXT NOT NULL)",
        [],
    )
    .unwrap();
    db.execute("CREATE TABLE thread_spawn_edges (parent_thread_id TEXT NOT NULL, child_thread_id TEXT NOT NULL, status TEXT NOT NULL)", []).unwrap();
    db.execute(
        "CREATE TABLE stage1_outputs (thread_id TEXT NOT NULL, output TEXT NOT NULL)",
        [],
    )
    .unwrap();
    db.execute(
        "CREATE TABLE agent_job_items (id TEXT PRIMARY KEY, assigned_thread_id TEXT)",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO threads (id, rollout_path, title, cwd, archived, archived_at, updated_at, updated_at_ms) VALUES ('t1', ?1, 'Codex Thread', '/old/project', 0, NULL, 100, 100000)", [rollout_path.to_string_lossy().to_string()]).unwrap();
    db.execute(
        "INSERT INTO thread_dynamic_tools (thread_id, tool_name) VALUES ('t1', 'Read')",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO thread_goals (thread_id, goal) VALUES ('t1', 'delete me')",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO thread_spawn_edges (parent_thread_id, child_thread_id, status) VALUES ('t1', 'child', 'running')", []).unwrap();
    db.execute("INSERT INTO thread_spawn_edges (parent_thread_id, child_thread_id, status) VALUES ('parent', 't1', 'done')", []).unwrap();
    db.execute(
        "INSERT INTO stage1_outputs (thread_id, output) VALUES ('t1', 'cached')",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO agent_job_items (id, assigned_thread_id) VALUES ('job1', 't1')",
        [],
    )
    .unwrap();
}

#[test]
fn backup_store_writes_reads_and_sanitizes_tokens() {
    let tmp = tempdir().unwrap();
    let store = BackupStore::new(tmp.path());

    let token = store
        .write_backup(
            "s1",
            Path::new("C:/state/codex.sqlite"),
            json!({"sessions": [{"id": "s1", "title": "Hello"}]}),
        )
        .unwrap();
    let backup = store.read_backup(&token).unwrap();

    assert_eq!(backup["session_id"], "s1");
    assert_eq!(backup["source_db"], "C:/state/codex.sqlite");
    assert_eq!(backup["tables"]["sessions"][0]["title"], "Hello");
    assert_eq!(
        store.path_for("../bad token!").file_name().unwrap(),
        "badtoken.json"
    );
    assert!(store.read_backup("missing").is_err());
}

#[test]
fn delete_local_session_creates_backup_and_undo_restores_rows() {
    let tmp = tempdir().unwrap();
    let db_path = tmp.path().join("codex.sqlite");
    create_supported_db(&db_path);
    let adapter = SQLiteStorageAdapter::new(&db_path, BackupStore::new(tmp.path().join("backups")));

    let deleted = adapter.delete_local(&session("s1", "First"));

    assert_eq!(deleted.status, DeleteStatus::LocalDeleted);
    assert_eq!(deleted.message, "已从本地存储删除");
    let db = Connection::open(&db_path).unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM sessions", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM messages", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(db);

    let restored = adapter.undo(deleted.undo_token.as_deref().unwrap());

    assert_eq!(restored.status, DeleteStatus::Undone);
    let db = Connection::open(&db_path).unwrap();
    assert_eq!(
        db.query_row("SELECT title FROM sessions WHERE id = 's1'", [], |row| {
            row.get::<_, String>(0)
        })
        .unwrap(),
        "First"
    );
    assert_eq!(
        db.query_row(
            "SELECT body FROM messages WHERE session_id = 's1'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "hello"
    );
}

#[test]
fn undo_fails_on_existing_db_row_conflict_without_overwriting_new_row() {
    let tmp = tempdir().unwrap();
    let db_path = tmp.path().join("codex.sqlite");
    create_supported_db(&db_path);
    let adapter = SQLiteStorageAdapter::new(&db_path, BackupStore::new(tmp.path().join("backups")));
    let deleted = adapter.delete_local(&session("s1", "First"));
    let token = deleted.undo_token.as_deref().unwrap();
    let db = Connection::open(&db_path).unwrap();
    db.execute(
        "INSERT INTO sessions (id, title) VALUES ('s1', 'New Session')",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO messages (session_id, body) VALUES ('s1', 'new body')",
        [],
    )
    .unwrap();
    drop(db);

    let restored = adapter.undo(token);

    assert_eq!(restored.status, DeleteStatus::Failed);
    assert_eq!(restored.undo_token.as_deref(), Some(token));
    assert!(restored.message.to_lowercase().contains("restore conflict"));
    let db = Connection::open(&db_path).unwrap();
    assert_eq!(
        db.query_row("SELECT title FROM sessions WHERE id = 's1'", [], |row| {
            row.get::<_, String>(0)
        })
        .unwrap(),
        "New Session"
    );
    assert_eq!(
        db.query_row(
            "SELECT body FROM messages WHERE session_id = 's1'",
            [],
            |row| { row.get::<_, String>(0) }
        )
        .unwrap(),
        "new body"
    );
}

#[test]
fn undo_fails_on_existing_rollout_file_conflict_without_overwriting_new_file() {
    let tmp = tempdir().unwrap();
    let db_path = tmp.path().join("state_5.sqlite");
    let rollout_path = tmp.path().join("rollout.jsonl");
    fs::write(&rollout_path, "old rollout\n").unwrap();
    create_codex_thread_db(&db_path, &rollout_path);
    let adapter = SQLiteStorageAdapter::new(&db_path, BackupStore::new(tmp.path().join("backups")));
    let deleted = adapter.delete_local(&session("t1", "Codex Thread"));
    let token = deleted.undo_token.as_deref().unwrap();
    fs::write(&rollout_path, "new rollout\n").unwrap();

    let restored = adapter.undo(token);

    assert_eq!(restored.status, DeleteStatus::Failed);
    assert_eq!(restored.undo_token.as_deref(), Some(token));
    assert!(restored.message.to_lowercase().contains("restore conflict"));
    assert_eq!(fs::read_to_string(&rollout_path).unwrap(), "new rollout\n");
    let db = Connection::open(&db_path).unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM threads WHERE id = 't1'", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap(),
        0
    );
}

#[test]
fn undo_fails_for_unknown_backup_table_without_executing_it() {
    let tmp = tempdir().unwrap();
    let db_path = tmp.path().join("codex.sqlite");
    create_supported_db(&db_path);
    let backup_store = BackupStore::new(tmp.path().join("backups"));
    let adapter = SQLiteStorageAdapter::new(&db_path, backup_store.clone());
    let deleted = adapter.delete_local(&session("s1", "First"));
    let token = deleted.undo_token.as_deref().unwrap();
    let backup_path = backup_store.path_for(token);
    let mut backup: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&backup_path).unwrap()).unwrap();
    backup["tables"]["evil_table"] = json!([{"id": "owned"}]);
    fs::write(&backup_path, serde_json::to_string_pretty(&backup).unwrap()).unwrap();

    let restored = adapter.undo(token);

    assert_eq!(restored.status, DeleteStatus::Failed);
    assert_eq!(restored.undo_token.as_deref(), Some(token));
    assert!(
        restored
            .message
            .to_lowercase()
            .contains("unknown restore table")
    );
    let db = Connection::open(&db_path).unwrap();
    let table_exists = db
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'evil_table'",
            [],
            |_| Ok(()),
        )
        .is_ok();
    assert!(!table_exists);
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM sessions WHERE id = 's1'", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap(),
        0
    );
}

#[test]
fn generic_delete_rolls_back_when_later_delete_fails() {
    let tmp = tempdir().unwrap();
    let db_path = tmp.path().join("codex.sqlite");
    create_supported_db(&db_path);
    let db = Connection::open(&db_path).unwrap();
    db.execute(
        "CREATE TRIGGER fail_session_delete BEFORE DELETE ON sessions BEGIN SELECT RAISE(ABORT, 'boom'); END",
        [],
    )
    .unwrap();
    drop(db);
    let adapter = SQLiteStorageAdapter::new(&db_path, BackupStore::new(tmp.path().join("backups")));

    let result = adapter.delete_local(&session("s1", "First"));

    assert_eq!(result.status, DeleteStatus::Failed);
    assert!(result.undo_token.is_some());
    assert!(result.backup_path.is_some());
    let db = Connection::open(&db_path).unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM sessions WHERE id = 's1'", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap(),
        1
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM messages WHERE session_id = 's1'",
            [],
            |row| { row.get::<_, i64>(0) }
        )
        .unwrap(),
        1
    );
}

#[test]
fn delete_codex_thread_schema_removes_related_rows_file_and_undo_restores_everything() {
    let tmp = tempdir().unwrap();
    let db_path = tmp.path().join("state_5.sqlite");
    let rollout_path = tmp.path().join("rollout.jsonl");
    fs::write(&rollout_path, "{\"type\":\"message\"}\n").unwrap();
    create_codex_thread_db(&db_path, &rollout_path);
    let adapter = SQLiteStorageAdapter::new(&db_path, BackupStore::new(tmp.path().join("backups")));

    let deleted = adapter.delete_local(&session("local:t1", "Codex Thread"));

    assert_eq!(deleted.status, DeleteStatus::LocalDeleted);
    assert!(!rollout_path.exists());
    let db = Connection::open(&db_path).unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM threads WHERE id = 't1'", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "SELECT assigned_thread_id FROM agent_job_items WHERE id = 'job1'",
            [],
            |row| row.get::<_, Option<String>>(0)
        )
        .unwrap(),
        None
    );
    drop(db);

    let restored = adapter.undo(deleted.undo_token.as_deref().unwrap());

    assert_eq!(restored.status, DeleteStatus::Undone);
    assert_eq!(
        fs::read_to_string(&rollout_path).unwrap(),
        "{\"type\":\"message\"}\n"
    );
    let db = Connection::open(&db_path).unwrap();
    assert_eq!(
        db.query_row("SELECT title FROM threads WHERE id = 't1'", [], |row| {
            row.get::<_, String>(0)
        })
        .unwrap(),
        "Codex Thread"
    );
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM thread_spawn_edges WHERE parent_thread_id = 't1' OR child_thread_id = 't1'", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        db.query_row(
            "SELECT assigned_thread_id FROM agent_job_items WHERE id = 'job1'",
            [],
            |row| row.get::<_, Option<String>>(0)
        )
        .unwrap(),
        Some("t1".to_string())
    );
}

#[test]
fn undo_codex_thread_delete_fails_when_agent_job_was_reassigned() {
    let tmp = tempdir().unwrap();
    let db_path = tmp.path().join("state_5.sqlite");
    let rollout_path = tmp.path().join("rollout.jsonl");
    fs::write(&rollout_path, "{\"type\":\"message\"}\n").unwrap();
    create_codex_thread_db(&db_path, &rollout_path);
    let adapter = SQLiteStorageAdapter::new(&db_path, BackupStore::new(tmp.path().join("backups")));

    let deleted = adapter.delete_local(&session("local:t1", "Codex Thread"));

    assert_eq!(deleted.status, DeleteStatus::LocalDeleted);
    let token = deleted.undo_token.as_deref().unwrap();
    let db = Connection::open(&db_path).unwrap();
    db.execute(
        "INSERT INTO threads (id, rollout_path, title, cwd, archived, archived_at, updated_at, updated_at_ms) VALUES ('t2', NULL, 'Other Thread', '/new/project', 0, NULL, 200, 200000)",
        [],
    )
    .unwrap();
    db.execute(
        "UPDATE agent_job_items SET assigned_thread_id = 't2' WHERE id = 'job1'",
        [],
    )
    .unwrap();
    drop(db);

    let restored = adapter.undo(token);

    assert_eq!(restored.status, DeleteStatus::Failed);
    assert_eq!(restored.undo_token.as_deref(), Some(token));
    assert!(restored.message.to_lowercase().contains("restore conflict"));
    let db = Connection::open(&db_path).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT assigned_thread_id FROM agent_job_items WHERE id = 'job1'",
            [],
            |row| row.get::<_, Option<String>>(0)
        )
        .unwrap(),
        Some("t2".to_string())
    );
}

#[test]
fn codex_delete_rolls_back_when_related_delete_fails() {
    let tmp = tempdir().unwrap();
    let db_path = tmp.path().join("state_5.sqlite");
    let rollout_path = tmp.path().join("rollout.jsonl");
    fs::write(&rollout_path, "{\"type\":\"message\"}\n").unwrap();
    create_codex_thread_db(&db_path, &rollout_path);
    let db = Connection::open(&db_path).unwrap();
    db.execute(
        "CREATE TRIGGER fail_goals_delete BEFORE DELETE ON thread_goals BEGIN SELECT RAISE(ABORT, 'boom'); END",
        [],
    )
    .unwrap();
    drop(db);
    let adapter = SQLiteStorageAdapter::new(&db_path, BackupStore::new(tmp.path().join("backups")));

    let result = adapter.delete_local(&session("t1", "Codex Thread"));

    assert_eq!(result.status, DeleteStatus::Failed);
    assert!(result.undo_token.is_some());
    assert!(rollout_path.exists());
    let db = Connection::open(&db_path).unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM threads WHERE id = 't1'", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap(),
        1
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM thread_dynamic_tools WHERE thread_id = 't1'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM thread_goals WHERE thread_id = 't1'",
            [],
            |row| { row.get::<_, i64>(0) }
        )
        .unwrap(),
        1
    );
}

#[test]
fn missing_db_and_unsupported_schema_return_failed_results() {
    let tmp = tempdir().unwrap();
    let missing = tmp.path().join("missing.sqlite");
    let adapter = SQLiteStorageAdapter::new(&missing, BackupStore::new(tmp.path().join("backups")));

    let result = adapter.delete_local(&session("s1", "First"));

    assert_eq!(result.status, DeleteStatus::Failed);
    assert!(result.message.contains("Database not found"));

    let db_path = tmp.path().join("unknown.sqlite");
    let db = Connection::open(&db_path).unwrap();
    db.execute("CREATE TABLE unrelated (id TEXT PRIMARY KEY)", [])
        .unwrap();
    drop(db);
    let adapter =
        SQLiteStorageAdapter::new(&db_path, BackupStore::new(tmp.path().join("backups2")));

    let result = adapter.delete_local(&session("s1", "First"));

    assert_eq!(result.status, DeleteStatus::Failed);
    assert!(result.message.contains("Unsupported"));
}

#[test]
fn archived_lookup_workspace_move_and_sort_keys_match_python_shape() {
    let tmp = tempdir().unwrap();
    let db_path = tmp.path().join("state_5.sqlite");
    let rollout_path = tmp.path().join("rollout.jsonl");
    fs::write(
        &rollout_path,
        "{\"type\":\"session_meta\",\"payload\":{\"id\":\"t1\",\"cwd\":\"/old/project\",\"title\":\"Codex Thread\"}}\n{\"type\":\"session_meta\",\"payload\":{\"id\":\"other\",\"cwd\":\"/old/project\"}}\n",
    )
    .unwrap();
    create_codex_thread_db(&db_path, &rollout_path);
    let db = Connection::open(&db_path).unwrap();
    db.execute(
        "UPDATE threads SET archived = 1, archived_at = 123 WHERE id = 't1'",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO threads (id, rollout_path, title, cwd, archived, archived_at, updated_at, updated_at_ms) VALUES ('t2', ?1, 'Second', '/other/project', 0, NULL, 200, 200000)", [rollout_path.to_string_lossy().to_string()]).unwrap();
    drop(db);
    let adapter = SQLiteStorageAdapter::new(&db_path, BackupStore::new(tmp.path().join("backups")));

    assert_eq!(
        adapter.find_archived_thread_by_title("Codex Thread 2026年5月9日，1:19 · RustGUI"),
        Some(session("t1", "Codex Thread"))
    );

    let moved =
        adapter.move_codex_thread_workspace(&session("local:t1", "Codex Thread"), "/new/project");
    assert_eq!(moved["status"], "moved");
    assert_eq!(moved["previous_cwd"], "/old/project");
    assert_eq!(moved["target_cwd"], "/new/project");
    assert_eq!(moved["rollout_updated"], true);
    assert_eq!(moved["updated_at"], 100);
    assert_eq!(moved["updated_at_ms"], 100000);
    let text = fs::read_to_string(&rollout_path).unwrap();
    assert!(text.contains("\"id\":\"t1\",\"cwd\":\"/new/project\""));
    assert!(text.contains("\"id\":\"other\",\"cwd\":\"/old/project\""));

    assert_eq!(
        adapter.codex_thread_sort_key(&session("local:t1", "Codex Thread")),
        json!({"status": "ok", "session_id": "t1", "updated_at": 100, "updated_at_ms": 100000, "created_at_ms": null})
    );
    assert_eq!(
        adapter.codex_thread_sort_keys(&[
            session("local:t2", "Second"),
            session("local:t1", "Codex Thread")
        ]),
        json!({
            "status": "ok",
            "sort_keys": [
                {"session_id": "t2", "updated_at": 200, "updated_at_ms": 200000, "created_at_ms": null},
                {"session_id": "t1", "updated_at": 100, "updated_at_ms": 100000, "created_at_ms": null}
            ]
        })
    );
}
