use codex_plus_core::models::{DeleteStatus, SessionRef};
use codex_plus_data::{BackupStore, SQLiteStorageAdapter, delete_local_from_paths};
use rusqlite::Connection;
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
};
use tempfile::{TempDir, tempdir};

struct Fixture {
    temp: TempDir,
    db: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempdir().unwrap();
        let db = temp.path().join("state_5.sqlite");
        Connection::open(&db).unwrap().execute_batch("CREATE TABLE threads(id TEXT PRIMARY KEY, title TEXT, rollout_path TEXT, archived INTEGER DEFAULT 0, archived_at INTEGER)").unwrap();
        Self { temp, db }
    }
    fn home(&self) -> &Path {
        self.temp.path()
    }
    fn path(&self, id: &str) -> PathBuf {
        self.home().join(format!("sessions/rollout-{id}.jsonl"))
    }
    fn add(&self, id: &str, base: Option<&str>) {
        let path = self.path(id);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut payload = json!({"id":id});
        if let Some(base) = base {
            payload["history_base"] =
                json!({"thread_id":base,"end_ordinal_exclusive":1,"end_byte_offset":100});
        }
        fs::write(
            &path,
            format!("{}\n", json!({"type":"session_meta","payload":payload})),
        )
        .unwrap();
        Connection::open(&self.db)
            .unwrap()
            .execute(
                "INSERT INTO threads(id,title,rollout_path) VALUES (?1,?1,?2)",
                [id, path.to_str().unwrap()],
            )
            .unwrap();
    }
    fn adapter(&self) -> SQLiteStorageAdapter {
        SQLiteStorageAdapter::new(&self.db, BackupStore::new(self.home().join("backups")))
            .with_codex_home(self.home())
    }
    fn delete(&self, id: &str) -> String {
        let r = self
            .adapter()
            .delete_local(&SessionRef::new(format!("local:{id}"), id).unwrap());
        assert_eq!(r.status, DeleteStatus::LocalDeleted, "{}", r.message);
        r.undo_token.unwrap()
    }
    fn undo(&self, token: &str) {
        let r = self.adapter().undo(token);
        assert_eq!(r.status, DeleteStatus::Undone, "{}", r.message);
    }
    fn visible(&self, id: &str) -> bool {
        self.adapter()
            .list_local_sessions()
            .unwrap()
            .iter()
            .any(|s| s.id == id)
    }
}

#[test]
fn soft_deleted_job_assignment_undo_is_idempotent_but_rejects_changes() {
    for change in [None, Some("assignment"), Some("content")] {
        let f = Fixture::new();
        f.add("a", None);
        f.add("b", Some("a"));
        let db = Connection::open(&f.db).unwrap();
        db.execute_batch("CREATE TABLE agent_job_items(id TEXT PRIMARY KEY, assigned_thread_id TEXT, status TEXT); INSERT INTO agent_job_items VALUES ('job1', 'a', 'pending')").unwrap();
        let token = f.delete("a");
        assert!(f.path("a").exists());
        assert!(!f.visible("a"));
        match change {
            Some("assignment") => {
                db.execute("UPDATE agent_job_items SET assigned_thread_id='other'", [])
                    .unwrap();
            }
            Some("content") => {
                db.execute("UPDATE agent_job_items SET status='changed'", [])
                    .unwrap();
            }
            _ => {}
        }
        let result = f.adapter().undo(&token);
        if change.is_none() {
            assert_eq!(result.status, DeleteStatus::Undone, "{}", result.message);
            f.undo(&token);
        } else {
            assert_eq!(result.status, DeleteStatus::Failed, "{}", result.message);
            assert!(!f.visible("a"));
        }
        let deleted: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM codex_plus_deleted_threads WHERE thread_id='a'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(deleted, i64::from(change.is_some()));
        let row = db
            .query_row(
                "SELECT assigned_thread_id, status FROM agent_job_items WHERE id='job1'",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .unwrap();
        assert_eq!(
            row.0,
            if change == Some("assignment") {
                "other"
            } else {
                "a"
            }
        );
        assert_eq!(
            row.1,
            if change == Some("content") {
                "changed"
            } else {
                "pending"
            }
        );
        assert!(f.path("a").exists());
        assert!(f.visible("b"));
    }
}

#[test]
fn chain_is_retained_until_last_reference_and_leaf_undo_restores_ancestors_hidden() {
    let f = Fixture::new();
    f.add("a", None);
    f.add("b", Some("a"));
    f.add("c", Some("b"));
    let original = fs::read(f.path("a")).unwrap();
    f.delete("a");
    assert!(f.path("a").exists());
    assert!(!f.visible("a"));
    f.delete("b");
    assert!(f.path("a").exists());
    assert!(f.path("b").exists());
    let token = f.delete("c");
    for id in ["a", "b", "c"] {
        assert!(!f.path(id).exists(), "{id}");
    }
    f.undo(&token);
    for id in ["a", "b", "c"] {
        assert!(f.path(id).exists(), "{id}");
    }
    assert_eq!(fs::read(f.path("a")).unwrap(), original);
    assert!(!f.visible("a"));
    assert!(!f.visible("b"));
    assert!(f.visible("c"));
    f.delete("c");
    for id in ["a", "b", "c"] {
        assert!(!f.path(id).exists());
    }
}

#[test]
fn shared_source_and_soft_undo_preserve_appended_bytes() {
    use std::io::Write;
    let f = Fixture::new();
    f.add("a", None);
    f.add("b", Some("a"));
    f.add("c", Some("a"));
    let token = f.delete("a");
    f.delete("b");
    assert!(f.path("a").exists());
    fs::OpenOptions::new()
        .append(true)
        .open(f.path("a"))
        .unwrap()
        .write_all(b"{\"type\":\"event_msg\"}\n")
        .unwrap();
    let appended = fs::read(f.path("a")).unwrap();
    f.undo(&token);
    assert_eq!(fs::read(f.path("a")).unwrap(), appended);
    assert!(f.visible("a"));
    f.delete("c");
    assert!(f.path("a").exists());
}

#[test]
fn corrupt_metadata_cannot_remove_rows_or_sidebar_index() {
    let f = Fixture::new();
    f.add("a", None);
    fs::write(f.path("a"), "{broken\n").unwrap();
    let index = f.home().join("session_index.jsonl");
    fs::write(&index, "{\"id\":\"a\"}\n").unwrap();
    let result = delete_local_from_paths(
        [f.db.clone()],
        BackupStore::new(f.home().join("backups")),
        &SessionRef::new("local:a", "a").unwrap(),
        Some(f.home()),
    );
    assert_eq!(result.status, DeleteStatus::Failed);
    assert!(fs::read_to_string(index).unwrap().contains("a"));
    assert!(f.visible("a"));
    assert!(f.path("a").exists());
}

#[test]
fn archived_dependency_and_unrelated_fork_are_handled_by_history_base() {
    let f = Fixture::new();
    f.add("a", None);
    f.add("b", Some("a"));
    let archived = f.home().join("archived_sessions/rollout-a.jsonl");
    fs::create_dir_all(archived.parent().unwrap()).unwrap();
    fs::rename(f.path("a"), &archived).unwrap();
    Connection::open(&f.db)
        .unwrap()
        .execute(
            "UPDATE threads SET rollout_path=?1 WHERE id='a'",
            [archived.to_str().unwrap()],
        )
        .unwrap();
    f.delete("a");
    assert!(archived.exists());
    f.delete("b");
    assert!(!archived.exists());
    f.add("x", None);
    f.add("y", None);
    fs::write(
        f.path("y"),
        format!(
            "{}\n",
            json!({"type":"session_meta","payload":{"id":"y","forked_from_id":"x"}})
        ),
    )
    .unwrap();
    f.delete("x");
    assert!(!f.path("x").exists());
    assert!(f.path("y").exists());
}

#[test]
fn stale_undo_and_changed_history_are_rejected() {
    let f = Fixture::new();
    f.add("a", None);
    f.add("b", Some("a"));
    let first = f.delete("a");
    f.undo(&first);
    let second = f.delete("a");
    assert_eq!(f.adapter().undo(&first).status, DeleteStatus::Failed);
    fs::write(f.path("a"), "different").unwrap();
    assert_eq!(f.adapter().undo(&second).status, DeleteStatus::Failed);
    assert!(!f.visible("a"));
}

#[test]
fn index_only_history_uses_same_retention_and_undo_contract() {
    let f = Fixture::new();
    f.add("a", None);
    f.add("b", Some("a"));
    Connection::open(&f.db)
        .unwrap()
        .execute("DELETE FROM threads WHERE id='a'", [])
        .unwrap();
    fs::write(f.home().join("session_index.jsonl"), "{\"id\":\"a\"}\n").unwrap();
    let r = delete_local_from_paths(
        [f.db.clone()],
        BackupStore::new(f.home().join("backups")),
        &SessionRef::new("local:a", "a").unwrap(),
        Some(f.home()),
    );
    assert_eq!(r.status, DeleteStatus::LocalDeleted, "{}", r.message);
    assert!(f.path("a").exists());
    let token = r.undo_token.unwrap();
    f.undo(&token);
    assert!(
        fs::read_to_string(f.home().join("session_index.jsonl"))
            .unwrap()
            .contains("a")
    );
}

#[test]
fn references_in_another_database_protect_the_source() {
    let f = Fixture::new();
    f.add("a", None);
    f.add("b", Some("a"));
    let other = f.home().join("sqlite/codex-dev.db");
    fs::create_dir_all(other.parent().unwrap()).unwrap();
    let db = Connection::open(&f.db).unwrap();
    db.execute("ATTACH DATABASE ?1 AS other", [other.to_str().unwrap()])
        .unwrap();
    db.execute_batch("CREATE TABLE other.threads AS SELECT * FROM threads WHERE id='b'; DELETE FROM threads WHERE id='b'; DETACH DATABASE other;").unwrap();
    f.delete("a");
    assert!(f.path("a").exists());
    let r = delete_local_from_paths(
        [f.db.clone(), other.clone()],
        BackupStore::new(f.home().join("backups")),
        &SessionRef::new("local:b", "b").unwrap(),
        Some(f.home()),
    );
    assert_eq!(r.status, DeleteStatus::LocalDeleted, "{}", r.message);
    assert!(!f.path("a").exists());
}

/// 手动验收接口：仅对调用者显式提供的隔离夹具执行，便于与原生 app-server 联合验证。
#[test]
#[ignore = "requires an explicitly provisioned isolated native fixture"]
fn isolated_native_delete_or_undo() {
    let home = PathBuf::from(std::env::var_os("CODEX_HISTORY_TEST_HOME").expect("isolated home"));
    assert!(
        home.join("ISOLATED_HISTORY_FIXTURE").is_file(),
        "refuse an unmarked home"
    );
    let input: serde_json::Value =
        serde_json::from_slice(&fs::read(home.join("operation.json")).unwrap()).unwrap();
    let db = home.join("state_5.sqlite");
    let adapter = SQLiteStorageAdapter::new(&db, BackupStore::new(home.join("backups")))
        .with_codex_home(&home);
    let result = if let Some(token) = input["undo"].as_str() {
        adapter.undo(token)
    } else {
        delete_local_from_paths(
            [db],
            BackupStore::new(home.join("backups")),
            &SessionRef::new(input["id"].as_str().unwrap(), "Native test").unwrap(),
            Some(&home),
        )
    };
    fs::write(
        home.join("operation-result.json"),
        serde_json::to_vec(&result).unwrap(),
    )
    .unwrap();
    assert!(
        matches!(
            result.status,
            DeleteStatus::LocalDeleted | DeleteStatus::Undone
        ),
        "{}",
        result.message
    );
}

#[test]
fn managed_grouped_undo_restores_shared_rollout_once_and_both_rows() {
    let f = Fixture::new();
    f.add("a", None);
    let other = f.home().join("sqlite/codex-dev.db");
    fs::create_dir_all(other.parent().unwrap()).unwrap();
    let db = Connection::open(&f.db).unwrap();
    db.execute("ATTACH DATABASE ?1 AS other", [other.to_str().unwrap()])
        .unwrap();
    db.execute_batch("CREATE TABLE other.threads AS SELECT * FROM threads; DETACH DATABASE other;")
        .unwrap();
    let paths = [f.db.clone(), other.clone()];
    let result = delete_local_from_paths(
        paths.clone(),
        BackupStore::new(f.home().join("backups")),
        &SessionRef::new("local:a", "a").unwrap(),
        Some(f.home()),
    );
    assert_eq!(
        result.status,
        DeleteStatus::LocalDeleted,
        "{}",
        result.message
    );
    assert!(!f.path("a").exists());
    let undo = f
        .adapter()
        .with_allowed_db_paths(paths)
        .undo(result.undo_token.as_deref().unwrap());
    assert_eq!(undo.status, DeleteStatus::Undone, "{}", undo.message);
    assert!(f.path("a").exists());
    assert!(f.visible("a"));
    assert_eq!(
        Connection::open(other)
            .unwrap()
            .query_row("SELECT COUNT(*) FROM threads WHERE id='a'", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn malformed_history_base_and_missing_ancestor_stop_deletion() {
    for metadata in [
        json!({"id":"a","history_base":{"end_byte_offset":3}}),
        json!({"id":"a","history_base":{"thread_id":"missing"}}),
    ] {
        let f = Fixture::new();
        f.add("a", None);
        fs::write(
            f.path("a"),
            format!("{}\n", json!({"type":"session_meta","payload":metadata})),
        )
        .unwrap();
        let result = f
            .adapter()
            .delete_local(&SessionRef::new("local:a", "a").unwrap());
        assert_eq!(result.status, DeleteStatus::Failed);
        assert!(f.visible("a"));
        assert!(f.path("a").exists());
    }
}

fn duplicate_db(f: &Fixture) -> PathBuf {
    let other = f.home().join("sqlite/codex-dev.db");
    fs::create_dir_all(other.parent().unwrap()).unwrap();
    let db = Connection::open(&f.db).unwrap();
    db.execute("ATTACH DATABASE ?1 AS other", [other.to_str().unwrap()])
        .unwrap();
    db.execute_batch("CREATE TABLE other.threads AS SELECT * FROM threads; DETACH DATABASE other;")
        .unwrap();
    other
}

#[test]
fn partial_database_delete_keeps_live_duplicate_and_its_history() {
    let f = Fixture::new();
    f.add("a", None);
    let other = duplicate_db(&f);
    Connection::open(&other).unwrap().execute_batch("CREATE TRIGGER reject_delete BEFORE UPDATE ON threads BEGIN SELECT RAISE(ABORT, 'injected failure'); END;").unwrap();
    let r = delete_local_from_paths(
        [f.db.clone(), other.clone()],
        BackupStore::new(f.home().join("backups")),
        &SessionRef::new("local:a", "a").unwrap(),
        Some(f.home()),
    );
    assert_eq!(r.status, DeleteStatus::LocalDeleted, "{}", r.message);
    assert!(
        f.path("a").exists(),
        "undeleted duplicate still owns rollout"
    );
    Connection::open(&other)
        .unwrap()
        .execute_batch("DROP TRIGGER reject_delete")
        .unwrap();
    let r = delete_local_from_paths(
        [f.db.clone(), other],
        BackupStore::new(f.home().join("backups")),
        &SessionRef::new("local:a", "a").unwrap(),
        Some(f.home()),
    );
    assert_eq!(r.status, DeleteStatus::LocalDeleted, "{}", r.message);
    assert!(!f.path("a").exists());
}

#[test]
fn grouped_snapshots_choose_longest_and_reject_divergence_before_restore() {
    use base64::Engine;
    for divergent in [false, true] {
        let f = Fixture::new();
        f.add("a", None);
        let original = fs::read(f.path("a")).unwrap();
        let other = duplicate_db(&f);
        let store = BackupStore::new(f.home().join("backups"));
        let r = delete_local_from_paths(
            [f.db.clone(), other.clone()],
            store.clone(),
            &SessionRef::new("local:a", "a").unwrap(),
            Some(f.home()),
        );
        let token = r.undo_token.unwrap();
        let tokens: Vec<String> = serde_json::from_str(&token).unwrap();
        let mut second = store.read_backup(&tokens[1]).unwrap();
        let mut longer = original.clone();
        if divergent {
            longer[0] = b'!';
        } else {
            longer.extend_from_slice(b"{\"type\":\"event_msg\"}\n");
        }
        for key in ["__files", "__history_files"] {
            for file in second["tables"][key].as_array_mut().unwrap() {
                file["content_b64"] =
                    json!(base64::engine::general_purpose::STANDARD.encode(&longer));
            }
        }
        fs::write(
            store.path_for(&tokens[1]),
            serde_json::to_vec(&second).unwrap(),
        )
        .unwrap();
        let result = f
            .adapter()
            .with_allowed_db_paths([f.db.clone(), other])
            .undo(&token);
        if divergent {
            assert_eq!(result.status, DeleteStatus::Failed);
            assert!(!f.path("a").exists());
            assert!(!f.visible("a"));
        } else {
            assert_eq!(result.status, DeleteStatus::Undone, "{}", result.message);
            assert_eq!(fs::read(f.path("a")).unwrap(), longer);
        }
    }
}

#[test]
fn failed_index_restore_keeps_marker_and_can_retry_after_rows_restored() {
    let f = Fixture::new();
    f.add("a", None);
    f.add("b", Some("a"));
    let index = f.home().join("session_index.jsonl");
    fs::write(&index, "{\"id\":\"a\"}\n").unwrap();
    let token = f.delete("a");
    let store = BackupStore::new(f.home().join("backups"));
    let mut backup = store.read_backup(&token).unwrap();
    backup["tables"]["__session_index"] = json!(["{\"id\":\"a\"}"]);
    fs::write(store.path_for(&token), serde_json::to_vec(&backup).unwrap()).unwrap();
    fs::write(&index, [0xff]).unwrap();
    let r = f.adapter().undo(&token);
    assert_eq!(r.status, DeleteStatus::Failed, "{}", r.message);
    assert!(!f.visible("a"));
    fs::rename(&index, index.with_extension("invalid-backup")).unwrap();
    f.undo(&token);
    assert!(f.visible("a"));
    assert!(fs::read_to_string(index).unwrap().contains("a"));
}

#[test]
fn grouped_finish_failure_can_retry_completed_databases_without_accepting_stale_delete() {
    let f = Fixture::new();
    f.add("a", None);
    let other = duplicate_db(&f);
    let r = delete_local_from_paths(
        [f.db.clone(), other.clone()],
        BackupStore::new(f.home().join("backups")),
        &SessionRef::new("local:a", "a").unwrap(),
        Some(f.home()),
    );
    let token = r.undo_token.unwrap();
    Connection::open(&other).unwrap().execute_batch("CREATE TRIGGER reject_finish BEFORE DELETE ON codex_plus_deleted_threads BEGIN SELECT RAISE(ABORT, 'injected failure'); END;").unwrap();
    let adapter = f
        .adapter()
        .with_allowed_db_paths([f.db.clone(), other.clone()]);
    assert_eq!(adapter.undo(&token).status, DeleteStatus::Failed);
    Connection::open(&other)
        .unwrap()
        .execute_batch("DROP TRIGGER reject_finish")
        .unwrap();
    let r = adapter.undo(&token);
    assert_eq!(r.status, DeleteStatus::Undone, "{}", r.message);
    f.delete("a");
    assert_eq!(adapter.undo(&token).status, DeleteStatus::Failed);
}

#[cfg(windows)]
#[test]
fn extended_length_home_preserves_dependencies_and_undo() {
    let f = Fixture::new();
    f.add("a", None);
    f.add("b", Some("a"));
    let home = fs::canonicalize(f.home()).unwrap();
    assert!(home.to_string_lossy().starts_with(r"\\?\"));
    let adapter = SQLiteStorageAdapter::new(&f.db, BackupStore::new(f.home().join("backups")))
        .with_codex_home(&home);
    let delete = |id: &str| {
        let result = adapter.delete_local(&SessionRef::new(format!("local:{id}"), id).unwrap());
        assert_eq!(
            result.status,
            DeleteStatus::LocalDeleted,
            "{}",
            result.message
        );
        result.undo_token.unwrap()
    };
    delete("a");
    assert!(f.path("a").exists());
    let token = delete("b");
    assert!(!f.path("a").exists());
    let result = adapter.undo(&token);
    assert_eq!(result.status, DeleteStatus::Undone, "{}", result.message);
    assert!(f.path("a").exists());
    assert!(f.path("b").exists());
    assert!(!f.visible("a"));
    assert!(f.visible("b"));
}

#[cfg(windows)]
#[test]
fn differently_cased_home_preserves_dependencies_and_undo() {
    let f = Fixture::new();
    f.add("a", None);
    f.add("b", Some("a"));
    let home = PathBuf::from(f.home().to_string_lossy().to_uppercase());
    let adapter = SQLiteStorageAdapter::new(&f.db, BackupStore::new(f.home().join("backups")))
        .with_codex_home(home);
    for id in ["a", "b"] {
        let result = adapter.delete_local(&SessionRef::new(format!("local:{id}"), id).unwrap());
        assert_eq!(
            result.status,
            DeleteStatus::LocalDeleted,
            "{}",
            result.message
        );
        if id == "a" {
            assert!(f.path("a").exists());
        } else {
            assert!(!f.path("a").exists());
            let result = adapter.undo(&result.undo_token.unwrap());
            assert_eq!(result.status, DeleteStatus::Undone, "{}", result.message);
        }
    }
    assert!(f.path("a").exists());
    assert!(f.path("b").exists());
    assert!(!f.visible("a"));
    assert!(f.visible("b"));
}
