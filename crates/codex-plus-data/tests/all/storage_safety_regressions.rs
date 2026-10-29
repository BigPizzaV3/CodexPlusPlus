use codex_plus_core::models::{DeleteStatus, SessionRef};
use codex_plus_data::{BackupStore, SQLiteStorageAdapter, delete_local_from_paths};
use rusqlite::Connection;
use serde_json::json;
use std::{fs, path::Path};

fn session() -> SessionRef {
    SessionRef::new("t", "fixture").unwrap()
}
fn database(path: &Path, rollout: &Path) {
    Connection::open(path)
        .unwrap()
        .execute_batch("CREATE TABLE threads(id TEXT PRIMARY KEY,title TEXT,rollout_path TEXT);")
        .unwrap();
    Connection::open(path)
        .unwrap()
        .execute(
            "INSERT INTO threads VALUES('t','fixture',?1)",
            [rollout.to_string_lossy().as_ref()],
        )
        .unwrap();
}
fn sidebar(home: &Path) {
    fs::create_dir_all(home).unwrap();
    fs::write(
        home.join("session_index.jsonl"),
        "{\"id\":\"t\",\"thread_name\":\"fixture\",\"updated_at\":\"2026-10-17T00:00:00Z\"}\n",
    )
    .unwrap();
    fs::write(
        home.join(".codex-global-state.json"),
        json!({"projectless-thread-ids":["t"],"thread-workspace-root-hints":{"t":"C:/fixture"}})
            .to_string(),
    )
    .unwrap();
}
fn count(path: &Path) -> i64 {
    Connection::open(path)
        .unwrap()
        .query_row("SELECT count(*) FROM threads", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn safety_top_level_backup_failure_does_not_remove_session_index_or_sidebar() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    sidebar(&home);
    let rollout = home.join("rollout.jsonl");
    fs::write(&rollout, b"original data").unwrap();
    let db = home.join("state.db");
    database(&db, &rollout);
    let index = fs::read(home.join("session_index.jsonl")).unwrap();
    let state = fs::read(home.join(".codex-global-state.json")).unwrap();
    let result = delete_local_from_paths(
        [db.clone()],
        BackupStore::new(temp.path().join("backups")).with_file_byte_limit(1),
        &session(),
        Some(&home),
    );
    assert_eq!(result.status, DeleteStatus::Failed, "{}", result.message);
    assert!(result.undo_token.is_none());
    assert_eq!(count(&db), 1);
    assert!(fs::read(home.join("session_index.jsonl")).unwrap() == index);
    assert!(fs::read(home.join(".codex-global-state.json")).unwrap() == state);
    assert_eq!(fs::read(&rollout).unwrap(), b"original data");
}

#[test]
fn safety_multi_database_partial_delete_keeps_failure_and_all_undo_information() {
    for fail_first in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        sidebar(&home);
        let small = home.join("small.jsonl");
        let large = home.join("large.jsonl");
        fs::write(&small, b"ok").unwrap();
        fs::write(&large, b"larger than the byte budget").unwrap();
        let a = home.join("a.db");
        let b = home.join("b.db");
        database(&a, &small);
        database(&b, &large);
        let paths = if fail_first {
            vec![b.clone(), a.clone()]
        } else {
            vec![a.clone(), b.clone()]
        };
        let store = BackupStore::new(temp.path().join("backups")).with_file_byte_limit(8);
        let result = delete_local_from_paths(paths.clone(), store.clone(), &session(), Some(&home));
        assert_eq!(
            result.status,
            DeleteStatus::Failed,
            "partial deletion must not be full success: {}",
            result.message
        );
        assert!(result.message.contains("budget"));
        assert_eq!(count(&a), 0);
        assert_eq!(count(&b), 1);
        assert!(!small.exists());
        assert!(large.exists());
        assert!(result.undo_token.is_some());
        assert!(result.backup_path.is_some());
        let restored = SQLiteStorageAdapter::new(&a, store)
            .with_allowed_db_paths(paths)
            .with_codex_home(&home)
            .undo(result.undo_token.as_deref().unwrap());
        assert_eq!(
            restored.status,
            DeleteStatus::Undone,
            "{}",
            restored.message
        );
        assert_eq!(count(&a), 1);
        assert_eq!(count(&b), 1);
        assert!(small.exists());
    }
}

#[test]
fn safety_undo_recreates_a_missing_session_index() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    sidebar(&home);
    let rollout = home.join("rollout.jsonl");
    fs::write(&rollout, b"data").unwrap();
    let db = home.join("state.db");
    database(&db, &rollout);
    let adapter = SQLiteStorageAdapter::new(&db, BackupStore::new(temp.path().join("backups")))
        .with_codex_home(&home);
    let deleted = adapter.delete_local(&session());
    assert_eq!(deleted.status, DeleteStatus::LocalDeleted);
    fs::remove_file(home.join("session_index.jsonl")).unwrap();
    let restored = adapter.undo(deleted.undo_token.as_deref().unwrap());
    assert_eq!(
        restored.status,
        DeleteStatus::Undone,
        "{}",
        restored.message
    );
    assert!(
        home.join("session_index.jsonl").is_file(),
        "Undone requires restored index"
    );
    assert!(
        fs::read_to_string(home.join("session_index.jsonl"))
            .unwrap()
            .contains("\"id\":\"t\"")
    );
    assert_eq!(count(&db), 1);
    assert!(rollout.exists());
}

#[test]
fn safety_undo_index_io_failure_is_detected_before_restoring_database() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    sidebar(&home);
    let rollout = home.join("rollout.jsonl");
    fs::write(&rollout, b"data").unwrap();
    let db = home.join("state.db");
    database(&db, &rollout);
    let adapter = SQLiteStorageAdapter::new(&db, BackupStore::new(temp.path().join("backups")))
        .with_codex_home(&home);
    let deleted = adapter.delete_local(&session());
    assert_eq!(deleted.status, DeleteStatus::LocalDeleted);
    fs::remove_file(home.join("session_index.jsonl")).unwrap();
    fs::create_dir(home.join("session_index.jsonl")).unwrap();
    let restored = adapter.undo(deleted.undo_token.as_deref().unwrap());
    assert_eq!(
        restored.status,
        DeleteStatus::Failed,
        "{}",
        restored.message
    );
    assert!(restored.undo_token.is_some());
    assert_eq!(count(&db), 0);
    assert!(!rollout.exists());
}

#[test]
fn safety_corrupt_backup_structure_never_reports_undone_or_restores_rows() {
    for corruption in [
        json!(null),
        json!({}),
        json!({"threads":[]}),
        json!({"threads":[],"__sidebar":{"thread_id":"t","global_state":{},"catalog":[]}}),
        json!({"threads":"not an array"}),
        json!({"threads":["not a row"]}),
        json!({"threads":[{}]}),
        json!({"threads":[{"id":{}}]}),
        json!({"__files":[{}]}),
        json!({"__session_index":[3]}),
        json!({"__sidebar":"bad"}),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        sidebar(&home);
        let rollout = home.join("rollout.jsonl");
        fs::write(&rollout, b"data").unwrap();
        let db = home.join("state.db");
        database(&db, &rollout);
        let store = BackupStore::new(temp.path().join("backups"));
        let adapter = SQLiteStorageAdapter::new(&db, store.clone()).with_codex_home(&home);
        let deleted = adapter.delete_local(&session());
        let token = deleted.undo_token.unwrap();
        let mut backup = store.read_backup(&token).unwrap();
        if corruption.is_null() {
            backup = json!({});
        } else {
            backup["tables"] = corruption;
        }
        fs::write(store.path_for(&token), backup.to_string()).unwrap();
        let restored = adapter.undo(&token);
        assert_eq!(
            restored.status,
            DeleteStatus::Failed,
            "{}",
            restored.message
        );
        assert!(restored.undo_token.is_some());
        assert_eq!(count(&db), 0);
        assert!(!rollout.exists());
    }
}

#[test]
fn safety_unsupported_schema_is_a_hard_failure_without_index_cleanup() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    sidebar(&home);
    let db = home.join("unknown.db");
    Connection::open(&db)
        .unwrap()
        .execute_batch("CREATE TABLE unrelated(id TEXT)")
        .unwrap();
    let index = fs::read(home.join("session_index.jsonl")).unwrap();
    let state = fs::read(home.join(".codex-global-state.json")).unwrap();
    let result = delete_local_from_paths(
        [db],
        BackupStore::new(temp.path().join("backups")),
        &session(),
        Some(&home),
    );
    assert_eq!(result.status, DeleteStatus::Failed);
    assert!(result.message.contains("Unsupported"));
    assert_eq!(fs::read(home.join("session_index.jsonl")).unwrap(), index);
    assert_eq!(
        fs::read(home.join(".codex-global-state.json")).unwrap(),
        state
    );
}

#[test]
fn safety_failed_database_transaction_keeps_its_backup_alongside_successful_backups() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    sidebar(&home);
    let small = home.join("small.jsonl");
    let blocked = home.join("blocked.jsonl");
    fs::write(&small, b"ok").unwrap();
    fs::write(&blocked, b"ok").unwrap();
    let a = home.join("a.db");
    let b = home.join("b.db");
    database(&a, &small);
    database(&b, &blocked);
    Connection::open(&b).unwrap().execute_batch("CREATE TRIGGER keep_thread BEFORE DELETE ON threads BEGIN SELECT RAISE(ABORT, 'fixture transaction blocked'); END;").unwrap();
    let store = BackupStore::new(temp.path().join("backups"));
    let result = delete_local_from_paths(
        [a.clone(), b.clone()],
        store.clone(),
        &session(),
        Some(&home),
    );
    assert_eq!(result.status, DeleteStatus::Failed);
    assert!(result.message.contains("fixture transaction blocked"));
    assert!(result.backup_path.is_none());
    let tokens: Vec<String> = serde_json::from_str(result.undo_token.as_deref().unwrap()).unwrap();
    assert_eq!(tokens.len(), 2);
    for token in tokens {
        assert!(store.path_for(&token).is_file());
        assert!(
            result
                .message
                .contains(store.path_for(&token).to_string_lossy().as_ref())
        );
    }
    assert_eq!(count(&a), 0);
    assert_eq!(count(&b), 1);
    assert!(blocked.exists());
}

#[test]
fn safety_session_index_only_backups_remain_undoable_for_legacy_and_v2() {
    for legacy in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        fs::create_dir(&home).unwrap();
        let db = home.join("state.db");
        Connection::open(&db)
            .unwrap()
            .execute_batch("CREATE TABLE threads(id TEXT PRIMARY KEY,title TEXT,rollout_path TEXT)")
            .unwrap();
        let store = BackupStore::new(temp.path().join("backups"));
        let token = store
            .write_backup(
                "t",
                &db,
                json!({"__session_index":["{\"id\":\"t\",\"thread_name\":\"fixture\"}"]}),
            )
            .unwrap();
        if legacy {
            let mut backup = store.read_backup(&token).unwrap();
            backup.as_object_mut().unwrap().remove("version");
            fs::write(store.path_for(&token), backup.to_string()).unwrap();
        }
        let result = SQLiteStorageAdapter::new(&db, store)
            .with_codex_home(&home)
            .undo(&token);
        assert_eq!(result.status, DeleteStatus::Undone, "{}", result.message);
        assert!(
            fs::read_to_string(home.join("session_index.jsonl"))
                .unwrap()
                .contains("\"id\":\"t\"")
        );
    }
}

#[test]
fn safety_grouped_undo_validates_every_manifest_before_restoring_any_rows() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    sidebar(&home);
    let a_file = home.join("a.jsonl");
    let b_file = home.join("b.jsonl");
    fs::write(&a_file, b"ok").unwrap();
    fs::write(&b_file, b"ok").unwrap();
    let a = home.join("a.db");
    let b = home.join("b.db");
    database(&a, &a_file);
    database(&b, &b_file);
    let store = BackupStore::new(temp.path().join("backups"));
    let result = delete_local_from_paths(
        [a.clone(), b.clone()],
        store.clone(),
        &session(),
        Some(&home),
    );
    assert_eq!(result.status, DeleteStatus::LocalDeleted);
    let token = result.undo_token.unwrap();
    let tokens: Vec<String> = serde_json::from_str(&token).unwrap();
    let mut corrupt = store.read_backup(&tokens[1]).unwrap();
    corrupt["tables"] = json!({"threads":"bad"});
    fs::write(store.path_for(&tokens[1]), corrupt.to_string()).unwrap();
    let restored = SQLiteStorageAdapter::new(&a, store)
        .with_allowed_db_paths([b.clone()])
        .with_codex_home(&home)
        .undo(&token);
    assert_eq!(restored.status, DeleteStatus::Failed);
    assert_eq!(restored.undo_token, Some(token));
    assert_eq!(count(&a), 0);
    assert_eq!(count(&b), 0);
    assert!(!a_file.exists());
    assert!(!b_file.exists());
}
