use codex_plus_core::models::{DeleteStatus, SessionRef};
use codex_plus_data::{BackupStore, SQLiteStorageAdapter};
use rusqlite::Connection;
use serde_json::json;
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};

fn database(path: &Path, rollout: &Path) {
    let db = Connection::open(path).unwrap();
    db.execute_batch("CREATE TABLE threads(id TEXT PRIMARY KEY,title TEXT,rollout_path TEXT);")
        .unwrap();
    db.execute(
        "INSERT INTO threads VALUES('t','fixture',?1)",
        [rollout.to_string_lossy().as_ref()],
    )
    .unwrap();
}
fn session() -> SessionRef {
    SessionRef::new("t", "fixture").unwrap()
}
fn write_large(path: &Path) {
    let mut file = fs::File::create(path).unwrap();
    for _ in 0..97 {
        file.write_all(&[b'x'; 65536]).unwrap();
    }
    file.write_all(b"last bytes").unwrap();
}
#[test]
fn large_rollout_uses_small_manifest_and_streaming_blob_then_undo_restores_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("rollout.jsonl");
    write_large(&path);
    let size = fs::metadata(&path).unwrap().len();
    let db = temp.path().join("state.db");
    database(&db, &path);
    let store = BackupStore::new(temp.path().join("backups"));
    let adapter = SQLiteStorageAdapter::new(&db, store.clone());
    let deleted = adapter.delete_local(&session());
    assert_eq!(
        deleted.status,
        DeleteStatus::LocalDeleted,
        "{}",
        deleted.message
    );
    assert!(!path.exists());
    let token = deleted.undo_token.as_ref().unwrap();
    assert!(fs::metadata(store.path_for(token)).unwrap().len() < 8192);
    let backup = store.read_backup(token).unwrap();
    let file = &backup["tables"]["__files"][0];
    assert!(file.get("content_b64").is_none());
    assert_eq!(file["size_bytes"], size);
    assert_eq!(backup["version"], 2);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(temp.path().join("backups"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(store.path_for(token))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let blob = temp
            .path()
            .join("backups")
            .join(file["blob_ref"].as_str().unwrap());
        assert_eq!(
            fs::metadata(blob.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(blob).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    assert_eq!(adapter.undo(token).status, DeleteStatus::Undone);
    assert_eq!(fs::metadata(&path).unwrap().len(), size);
    let mut input = fs::File::open(&path).unwrap();
    let mut bytes = [0; 65536];
    let mut total = 0_u64;
    loop {
        let n = input.read(&mut bytes).unwrap();
        if n == 0 {
            break;
        }
        total += n as u64;
    }
    assert_eq!(total, size);
}
#[test]
fn byte_budget_failure_leaves_database_and_rollout_untouched() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("rollout.jsonl");
    write_large(&path);
    let db = temp.path().join("state.db");
    database(&db, &path);
    let adapter = SQLiteStorageAdapter::new(
        &db,
        BackupStore::new(temp.path().join("backups")).with_file_byte_limit(1024),
    );
    let result = adapter.delete_local(&session());
    assert_eq!(result.status, DeleteStatus::Failed);
    assert!(path.exists());
    assert!(result.undo_token.is_none());
    assert_eq!(
        Connection::open(db)
            .unwrap()
            .query_row("SELECT count(*) FROM threads", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
}
#[test]
fn corrupted_or_escaped_blob_is_rejected_before_restoring_rows() {
    for escape in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("rollout.jsonl");
        fs::write(&path, b"fixture").unwrap();
        let db = temp.path().join("state.db");
        database(&db, &path);
        let store = BackupStore::new(temp.path().join("backups"));
        let adapter = SQLiteStorageAdapter::new(&db, store.clone());
        let result = adapter.delete_local(&session());
        let token = result.undo_token.unwrap();
        let mut backup = store.read_backup(&token).unwrap();
        if escape {
            backup["tables"]["__files"][0]["blob_ref"] = json!("../outside");
            fs::write(store.path_for(&token), backup.to_string()).unwrap();
        } else {
            let relative = backup["tables"]["__files"][0]["blob_ref"].as_str().unwrap();
            fs::write(temp.path().join("backups").join(relative), b"damaged").unwrap();
        }
        assert_eq!(adapter.undo(&token).status, DeleteStatus::Failed);
        assert!(!path.exists());
        assert_eq!(
            Connection::open(&db)
                .unwrap()
                .query_row("SELECT count(*) FROM threads", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}
#[test]
fn old_base64_backup_remains_restorable_and_existing_target_is_not_replaced() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("rollout.jsonl");
    let db = temp.path().join("state.db");
    database(&db, &path);
    Connection::open(&db)
        .unwrap()
        .execute("DELETE FROM threads", [])
        .unwrap();
    let store = BackupStore::new(temp.path().join("backups"));
    let token=store.write_backup("t",&db,json!({"threads":[{"id":"t","title":"fixture","rollout_path":path}],"__files":[{"path":path,"content_b64":"bGVnYWN5"}]})).unwrap();
    let adapter = SQLiteStorageAdapter::new(&db, store);
    fs::write(&path, b"concurrent").unwrap();
    assert_eq!(adapter.undo(&token).status, DeleteStatus::Failed);
    assert_eq!(fs::read(&path).unwrap(), b"concurrent");
    fs::remove_file(&path).unwrap();
    assert_eq!(adapter.undo(&token).status, DeleteStatus::Undone);
    assert_eq!(fs::read(&path).unwrap(), b"legacy");
}
