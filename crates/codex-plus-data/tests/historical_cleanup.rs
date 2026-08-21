use codex_plus_data::{
    apply_historical_cleanup, preview_historical_cleanup, undo_historical_cleanup,
};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

const GHOST: &str = "019f8d0f-068d-7b11-86ce-727daba1f76b";

fn catalog_db(home: &Path) -> PathBuf {
    let path = home.join("sqlite/codex-dev.db");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let db = Connection::open(&path).unwrap();
    db.execute_batch(
        "CREATE TABLE local_thread_catalog (
            host_id TEXT NOT NULL,
            thread_id TEXT NOT NULL,
            display_title TEXT,
            cwd TEXT,
            source_updated_at TEXT,
            source_detail TEXT,
            PRIMARY KEY(host_id, thread_id)
        );
        CREATE TABLE thread_timeline_ledger (
            host_id TEXT NOT NULL,
            thread_id TEXT NOT NULL,
            updated_at TEXT,
            PRIMARY KEY(host_id, thread_id)
        );
        CREATE TABLE local_thread_catalog_metadata (catalog_revision INTEGER NOT NULL);
        CREATE TABLE local_thread_catalog_sync_state (host_id TEXT PRIMARY KEY, sync_cursor TEXT);
        INSERT INTO local_thread_catalog_metadata VALUES (7);",
    )
    .unwrap();
    db.execute(
        "INSERT INTO local_thread_catalog_sync_state VALUES ('local', 'cursor-keep')",
        [],
    )
    .unwrap();
    path
}

fn add_ghost(db_path: &Path, source_detail: &str) {
    let db = Connection::open(db_path).unwrap();
    db.execute(
        "INSERT INTO local_thread_catalog VALUES ('local', ?1, '旧会话标题', 'D:/work/demo', '2026-08-01T12:00:00Z', ?2)",
        (GHOST, source_detail),
    )
    .unwrap();
    db.execute(
        "INSERT INTO thread_timeline_ledger VALUES ('local', ?1, '2026-08-01T12:00:00Z')",
        [GHOST],
    )
    .unwrap();
}

fn write_index_and_global_state(home: &Path) {
    fs::write(
        home.join("session_index.jsonl"),
        format!(
            "{}\n{}\n",
            json!({"id": GHOST, "thread_name": "旧会话标题", "updated_at": "2026-08-01T12:00:00Z"}),
            json!({"id": "keep", "thread_name": "保留"})
        ),
    )
    .unwrap();
    let state = json!({
        "threadBindings": {(GHOST): {"selected": true}, "keep": "keep"},
        "activeThread": GHOST,
        "recentThreads": [GHOST, "keep"],
        "promptHistory": [format!("普通提示文本提到了 {GHOST}，不应删除")]
    });
    fs::write(
        home.join(".codex-global-state.json"),
        serde_json::to_vec_pretty(&state).unwrap(),
    )
    .unwrap();
    fs::write(
        home.join(".codex-global-state.json.bak"),
        serde_json::to_vec_pretty(&state).unwrap(),
    )
    .unwrap();
}

fn count(db_path: &Path, table: &str, id: &str) -> i64 {
    Connection::open(db_path)
        .unwrap()
        .query_row(
            &format!("SELECT COUNT(*) FROM {table} WHERE thread_id = ?1"),
            [id],
            |row| row.get(0),
        )
        .unwrap()
}

fn rollout(home: &Path, root: &str, id: &str) {
    let path = home
        .join(root)
        .join(format!("2026/08/rollout-2026-08-01T00-00-00-{id}.jsonl"));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        path,
        format!(
            "{}\n",
            json!({"type": "session_meta", "payload": {"id": id}})
        ),
    )
    .unwrap();
}

#[test]
fn historical_shell_is_cleaned_safely_and_can_be_undone() {
    let temp = tempdir().unwrap();
    let home = temp.path();
    let db_path = catalog_db(home);
    add_ghost(&db_path, "D:/missing/rollout.jsonl");
    write_index_and_global_state(home);

    let preview = preview_historical_cleanup(Some(home)).unwrap();
    assert_eq!(preview.catalog_revision, 7);
    assert_eq!(preview.candidates.len(), 1);
    assert_eq!(preview.candidates[0].id, GHOST);
    assert_eq!(preview.candidates[0].workspace, "D:/work/demo");
    assert_eq!(
        preview.candidates[0].sources,
        [
            "catalog",
            "global_state",
            "global_state_bak",
            "session_index",
            "timeline"
        ]
    );

    let result =
        apply_historical_cleanup(Some(home), &preview.snapshot_sha256, &[GHOST.to_string()])
            .unwrap();
    assert_eq!(result.catalog_rows, 1);
    assert_eq!(result.timeline_rows, 1);
    assert_eq!(result.session_index_entries, 1);
    assert!(result.global_state_references >= 3);
    assert_eq!(count(&db_path, "local_thread_catalog", GHOST), 0);
    assert_eq!(count(&db_path, "thread_timeline_ledger", GHOST), 0);
    let revision: i64 = Connection::open(&db_path)
        .unwrap()
        .query_row(
            "SELECT catalog_revision FROM local_thread_catalog_metadata",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(revision, 8);
    let sync_cursor: String = Connection::open(&db_path)
        .unwrap()
        .query_row(
            "SELECT sync_cursor FROM local_thread_catalog_sync_state WHERE host_id = 'local'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(sync_cursor, "cursor-keep");
    let state: Value =
        serde_json::from_slice(&fs::read(home.join(".codex-global-state.json")).unwrap()).unwrap();
    let expected_prompt = format!("普通提示文本提到了 {GHOST}，不应删除");
    assert_eq!(
        state.pointer("/promptHistory/0").and_then(Value::as_str),
        Some(expected_prompt.as_str())
    );
    assert!(
        !state
            .to_string()
            .contains(&format!("\"activeThread\":\"{GHOST}\""))
    );

    let backup = result.backup_dir.unwrap();
    assert!(backup.join("manifest.json").is_file());
    undo_historical_cleanup(Some(home), &backup).unwrap();
    assert_eq!(count(&db_path, "local_thread_catalog", GHOST), 1);
    assert_eq!(count(&db_path, "thread_timeline_ledger", GHOST), 1);
    assert!(
        fs::read_to_string(home.join("session_index.jsonl"))
            .unwrap()
            .contains(GHOST)
    );
    let restored: Value =
        serde_json::from_slice(&fs::read(home.join(".codex-global-state.json")).unwrap()).unwrap();
    assert_eq!(
        restored.pointer("/activeThread").and_then(Value::as_str),
        Some(GHOST)
    );
}

#[test]
fn real_database_thread_is_not_a_candidate() {
    let temp = tempdir().unwrap();
    let home = temp.path();
    let db_path = catalog_db(home);
    add_ghost(&db_path, "");
    let state = Connection::open(home.join("state_5.sqlite")).unwrap();
    state
        .execute("CREATE TABLE threads (id TEXT PRIMARY KEY)", [])
        .unwrap();
    state
        .execute("INSERT INTO threads VALUES (?1)", [GHOST])
        .unwrap();
    assert!(
        preview_historical_cleanup(Some(home))
            .unwrap()
            .candidates
            .is_empty()
    );
}

#[test]
fn message_body_source_is_not_a_candidate() {
    let temp = tempdir().unwrap();
    let home = temp.path();
    let db_path = catalog_db(home);
    add_ghost(&db_path, "");
    let state = Connection::open(home.join("state_5.sqlite")).unwrap();
    state
        .execute("CREATE TABLE messages (session_id TEXT, body TEXT)", [])
        .unwrap();
    state
        .execute("INSERT INTO messages VALUES (?1, '正文')", [GHOST])
        .unwrap();
    assert!(
        preview_historical_cleanup(Some(home))
            .unwrap()
            .candidates
            .is_empty()
    );
}

#[test]
fn legacy_session_index_only_shell_remains_cleanable() {
    let temp = tempdir().unwrap();
    let home = temp.path();
    write_index_and_global_state(home);
    let preview = preview_historical_cleanup(Some(home)).unwrap();
    assert_eq!(preview.candidates.len(), 1);
    assert_eq!(preview.candidates[0].id, GHOST);
    assert_eq!(
        preview.candidates[0].sources,
        ["global_state", "global_state_bak", "session_index"]
    );
}

#[test]
fn remote_host_catalog_thread_is_not_a_candidate() {
    let temp = tempdir().unwrap();
    let home = temp.path();
    let db_path = catalog_db(home);
    Connection::open(&db_path)
        .unwrap()
        .execute(
            "INSERT INTO local_thread_catalog VALUES ('remote-ssh', ?1, '远程会话', '', '', '')",
            [GHOST],
        )
        .unwrap();
    write_index_and_global_state(home);
    assert!(
        preview_historical_cleanup(Some(home))
            .unwrap()
            .candidates
            .is_empty()
    );
}

#[test]
fn active_and_archived_rollouts_are_not_candidates() {
    for root in ["sessions", "archived_sessions"] {
        let temp = tempdir().unwrap();
        let home = temp.path();
        let db_path = catalog_db(home);
        add_ghost(&db_path, "");
        rollout(home, root, GHOST);
        assert!(
            preview_historical_cleanup(Some(home))
                .unwrap()
                .candidates
                .is_empty(),
            "{root}"
        );
    }
}

#[test]
fn changed_source_rejects_apply_and_undo_rejects_same_id_conflict() {
    let temp = tempdir().unwrap();
    let home = temp.path();
    let db_path = catalog_db(home);
    add_ghost(&db_path, "");
    write_index_and_global_state(home);
    let preview = preview_historical_cleanup(Some(home)).unwrap();
    fs::write(home.join("session_index.jsonl"), "{\"id\":\"changed\"}\n").unwrap();
    let error =
        apply_historical_cleanup(Some(home), &preview.snapshot_sha256, &[GHOST.to_string()])
            .unwrap_err();
    assert!(error.message.contains("发生变化"));

    write_index_and_global_state(home);
    let preview = preview_historical_cleanup(Some(home)).unwrap();
    let result =
        apply_historical_cleanup(Some(home), &preview.snapshot_sha256, &[GHOST.to_string()])
            .unwrap();
    let state = Connection::open(home.join("state_5.sqlite")).unwrap();
    state
        .execute("CREATE TABLE threads (id TEXT PRIMARY KEY)", [])
        .unwrap();
    state
        .execute("INSERT INTO threads VALUES (?1)", [GHOST])
        .unwrap();
    let error =
        undo_historical_cleanup(Some(home), result.backup_dir.as_ref().unwrap()).unwrap_err();
    assert!(error.message.contains("同 ID"));
    assert_eq!(count(&db_path, "local_thread_catalog", GHOST), 0);
}

#[test]
fn sqlite_failure_is_reported_and_backup_remains_available() {
    let temp = tempdir().unwrap();
    let home = temp.path();
    let db_path = catalog_db(home);
    add_ghost(&db_path, "");
    write_index_and_global_state(home);
    let db = Connection::open(&db_path).unwrap();
    db.execute_batch(
        "CREATE TRIGGER reject_catalog_delete BEFORE DELETE ON local_thread_catalog
         BEGIN SELECT RAISE(ABORT, 'blocked'); END;",
    )
    .unwrap();
    drop(db);
    let preview = preview_historical_cleanup(Some(home)).unwrap();
    let error =
        apply_historical_cleanup(Some(home), &preview.snapshot_sha256, &[GHOST.to_string()])
            .unwrap_err();
    assert!(
        error
            .backup_dir
            .as_ref()
            .is_some_and(|path| path.join("manifest.json").is_file())
    );
    assert_eq!(count(&db_path, "local_thread_catalog", GHOST), 1);
    assert_eq!(count(&db_path, "thread_timeline_ledger", GHOST), 1);
}

#[test]
fn later_sqlite_failure_can_be_undone_from_the_reported_backup() {
    let temp = tempdir().unwrap();
    let home = temp.path();
    let first_db = catalog_db(home);
    add_ghost(&first_db, "");
    let second_db = home.join("sqlite/z-failing.db");
    fs::copy(&first_db, &second_db).unwrap();
    Connection::open(&second_db)
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER reject_catalog_delete BEFORE DELETE ON local_thread_catalog
             BEGIN SELECT RAISE(ABORT, 'second database blocked'); END;",
        )
        .unwrap();
    write_index_and_global_state(home);

    let preview = preview_historical_cleanup(Some(home)).unwrap();
    let error =
        apply_historical_cleanup(Some(home), &preview.snapshot_sha256, &[GHOST.to_string()])
            .unwrap_err();
    let backup = error.backup_dir.as_ref().unwrap();

    assert_eq!(error.partial_result.catalog_rows, 1);
    assert_eq!(error.partial_result.timeline_rows, 1);
    assert_eq!(count(&first_db, "local_thread_catalog", GHOST), 0);
    assert_eq!(count(&second_db, "local_thread_catalog", GHOST), 1);
    assert!(
        !fs::read_to_string(home.join("session_index.jsonl"))
            .unwrap()
            .contains(GHOST)
    );

    undo_historical_cleanup(Some(home), backup).unwrap();

    assert_eq!(count(&first_db, "local_thread_catalog", GHOST), 1);
    assert_eq!(count(&second_db, "local_thread_catalog", GHOST), 1);
    assert!(
        fs::read_to_string(home.join("session_index.jsonl"))
            .unwrap()
            .contains(GHOST)
    );
}

#[test]
fn json_write_failure_is_reported_before_database_changes() {
    let temp = tempdir().unwrap();
    let home = temp.path();
    let db_path = catalog_db(home);
    add_ghost(&db_path, "");
    write_index_and_global_state(home);
    fs::create_dir(home.join("session_index.jsonl.tmp")).unwrap();

    let preview = preview_historical_cleanup(Some(home)).unwrap();
    let error =
        apply_historical_cleanup(Some(home), &preview.snapshot_sha256, &[GHOST.to_string()])
            .unwrap_err();
    assert!(
        error
            .backup_dir
            .as_ref()
            .is_some_and(|path| path.join("manifest.json").is_file())
    );
    assert_eq!(count(&db_path, "local_thread_catalog", GHOST), 1);
    assert_eq!(count(&db_path, "thread_timeline_ledger", GHOST), 1);
    assert!(
        fs::read_to_string(home.join("session_index.jsonl"))
            .unwrap()
            .contains(GHOST)
    );
}
