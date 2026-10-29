use codex_plus_core::models::SessionRef;
use codex_plus_data::{BackupStore, SQLiteStorageAdapter};
use rusqlite::Connection;
use std::{fs, path::Path};

fn database(path: &Path, thread: Option<(&str, &Path)>) {
    let db = Connection::open(path).unwrap();
    db.execute_batch("CREATE TABLE threads(id TEXT PRIMARY KEY, title TEXT, rollout_path TEXT);")
        .unwrap();
    if let Some((id, rollout)) = thread {
        db.execute(
            "INSERT INTO threads VALUES(?1,'fixture',?2)",
            (id, rollout.to_string_lossy().as_ref()),
        )
        .unwrap();
    }
}
fn rollout(path: &Path) {
    fs::write(path, "{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"total_token_usage\":{\"total_tokens\":23},\"last_token_usage\":{\"input_tokens\":11,\"output_tokens\":12,\"total_tokens\":23},\"model_context_window\":1000000}}}\n").unwrap();
}
#[test]
fn both_usage_routes_find_target_after_empty_or_other_thread_primary() {
    for other in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let primary = temp.path().join("first.db");
        let second = temp.path().join("second.db");
        let file = temp.path().join("rollout.jsonl");
        rollout(&file);
        database(&primary, other.then_some(("other", file.as_path())));
        database(&second, Some(("target", &file)));
        let adapter =
            SQLiteStorageAdapter::new(&primary, BackupStore::new(temp.path().join("backups")))
                .with_allowed_db_paths([second]);
        let session = SessionRef::new("target", "fixture").unwrap();
        let history = adapter.codex_thread_usage_history(&session);
        let live = adapter.codex_thread_usage_live(&session);
        assert_eq!(history["status"], "ok", "{history}");
        assert_eq!(live["status"], "ok", "{live}");
        assert_eq!(history["history"].as_array().unwrap().len(), 1);
        assert_eq!(live["events"].as_array().unwrap().len(), 1);
    }
}
#[test]
fn unreadable_schema_empty_path_and_missing_rollout_do_not_shadow_later_target() {
    let temp = tempfile::tempdir().unwrap();
    let broken = temp.path().join("broken.db");
    fs::write(&broken, b"not SQLite").unwrap();
    let empty = temp.path().join("empty.db");
    database(&empty, Some(("target", Path::new(""))));
    let missing = temp.path().join("missing.db");
    database(
        &missing,
        Some(("target", &temp.path().join("absent.jsonl"))),
    );
    let good = temp.path().join("good.db");
    let file = temp.path().join("rollout.jsonl");
    rollout(&file);
    database(&good, Some(("target", &file)));
    let adapter = SQLiteStorageAdapter::new(&broken, BackupStore::new(temp.path().join("backups")))
        .with_allowed_db_paths([empty, missing, good]);
    let session = SessionRef::new("target", "fixture").unwrap();
    assert_eq!(adapter.codex_thread_usage_live(&session)["status"], "ok");
    assert_eq!(
        adapter.codex_thread_usage_history(&session)["rollout_path"],
        file.to_string_lossy().as_ref()
    );
}
#[test]
fn duplicate_target_is_stable_and_all_candidate_errors_are_distinct_from_missing() {
    let temp = tempfile::tempdir().unwrap();
    let first = temp.path().join("first.db");
    let second = temp.path().join("second.db");
    let a = temp.path().join("a.jsonl");
    let b = temp.path().join("b.jsonl");
    rollout(&a);
    rollout(&b);
    database(&first, Some(("target", &a)));
    database(&second, Some(("target", &b)));
    let adapter = SQLiteStorageAdapter::new(&first, BackupStore::new(temp.path().join("backups")))
        .with_allowed_db_paths([second]);
    let session = SessionRef::new("target", "fixture").unwrap();
    assert_eq!(
        adapter.codex_thread_usage_live(&session)["rollout_path"],
        a.to_string_lossy().as_ref()
    );
    let broken = temp.path().join("broken.db");
    fs::write(&broken, b"not SQLite").unwrap();
    let adapter = SQLiteStorageAdapter::new(broken, BackupStore::new(temp.path().join("unused")));
    assert!(
        adapter.codex_thread_usage_live(&session)["message"]
            .as_str()
            .unwrap()
            .contains("candidate errors")
    );
    let adapter = SQLiteStorageAdapter::new(
        temp.path().join("not-there.db"),
        BackupStore::new(temp.path().join("unused")),
    );
    assert!(
        adapter.codex_thread_usage_live(&session)["message"]
            .as_str()
            .unwrap()
            .contains("database missing")
    );
}
