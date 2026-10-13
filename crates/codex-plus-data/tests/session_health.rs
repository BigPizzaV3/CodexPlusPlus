use codex_plus_data::session_health::{recheck_session_health, scan_session_health};
use rusqlite::Connection;
use serde_json::json;
use std::fs;
use std::path::Path;
use tempfile::tempdir;

const LOST: &str = "01a083c9-ad68-7753-8e30-8271745ae918";
const KEPT: &str = "01a083c9-ad68-7753-8e30-8271745ae919";

fn database(home: &Path) -> Connection {
    let db = Connection::open(home.join("state_5.sqlite")).unwrap();
    db.execute_batch("CREATE TABLE threads (id TEXT, rollout_path TEXT, archived INTEGER); CREATE TABLE local_thread_catalog (thread_id TEXT, source_detail TEXT, host_id TEXT);").unwrap();
    db
}

fn missing(home: &Path) -> Vec<String> {
    scan_session_health(home, &home.join("backups"), &[])
        .unwrap()
        .missing_ids
}

#[test]
fn missing_rollout_and_index_only_rows_are_candidates_without_modifying_storage() {
    let dir = tempdir().unwrap();
    let db = database(dir.path());
    db.execute(
        "INSERT INTO threads VALUES (?1, 'sessions/missing.jsonl', 0)",
        [LOST],
    )
    .unwrap();
    let index = format!("{}\n", json!({"id": KEPT, "thread_name": "旧会话"}));
    fs::write(dir.path().join("session_index.jsonl"), &index).unwrap();
    assert_eq!(missing(dir.path()), vec![LOST, KEPT]);
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM threads", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("session_index.jsonl")).unwrap(),
        index
    );
}

#[test]
fn archived_and_remote_threads_are_never_candidates() {
    let dir = tempdir().unwrap();
    let db = database(dir.path());
    db.execute(
        "INSERT INTO threads VALUES (?1, 'missing.jsonl', 1)",
        [LOST],
    )
    .unwrap();
    db.execute(
        "INSERT INTO local_thread_catalog VALUES (?1, NULL, 'remote-ssh:server')",
        [KEPT],
    )
    .unwrap();
    assert!(missing(dir.path()).is_empty());
}

#[test]
fn catalog_uses_the_registered_local_host_instead_of_assuming_its_id() {
    let dir = tempdir().unwrap();
    let db = database(dir.path());
    db.execute_batch("CREATE TABLE local_thread_catalog_hosts (host_id TEXT, host_kind TEXT); INSERT INTO local_thread_catalog_hosts VALUES ('this-machine', 'local')").unwrap();
    db.execute(
        "INSERT INTO local_thread_catalog VALUES (?1, NULL, 'this-machine')",
        [LOST],
    )
    .unwrap();
    db.execute(
        "INSERT INTO local_thread_catalog VALUES (?1, NULL, 'remote-ssh:server')",
        [KEPT],
    )
    .unwrap();
    assert_eq!(missing(dir.path()), vec![LOST]);
}

#[test]
fn protects_existing_paths_and_rollouts_found_by_filename_or_metadata() {
    for (root, filename) in [
        ("sessions", format!("rollout-{LOST}.jsonl")),
        ("archived_sessions", "renamed.jsonl".into()),
        ("backups_state", "recovered.jsonl".into()),
    ] {
        let dir = tempdir().unwrap();
        let db = database(dir.path());
        db.execute(
            "INSERT INTO threads VALUES (?1, 'old-path.jsonl', 0)",
            [LOST],
        )
        .unwrap();
        fs::create_dir_all(dir.path().join(root)).unwrap();
        fs::write(
            dir.path().join(root).join(filename),
            format!(
                "{}\n",
                json!({"type":"session_meta", "payload":{"id": LOST}})
            ),
        )
        .unwrap();
        assert!(missing(dir.path()).is_empty(), "{root}");
    }
    let dir = tempdir().unwrap();
    let db = database(dir.path());
    fs::write(dir.path().join("empty.jsonl"), "").unwrap();
    db.execute("INSERT INTO threads VALUES (?1, 'empty.jsonl', 0)", [LOST])
        .unwrap();
    assert!(missing(dir.path()).is_empty());
}

#[test]
fn deletion_backups_preserve_undo_and_reappearing_files_are_rechecked() {
    let dir = tempdir().unwrap();
    let db = database(dir.path());
    db.execute(
        "INSERT INTO threads VALUES (?1, 'recovered.jsonl', 0)",
        [LOST],
    )
    .unwrap();
    db.execute(
        "INSERT INTO threads VALUES (?1, 'missing.jsonl', 0)",
        [KEPT],
    )
    .unwrap();
    fs::create_dir(dir.path().join("backups")).unwrap();
    fs::write(
        dir.path().join("backups/undo.json"),
        json!({"session_id": KEPT, "tables":{"__files":[{"content_b64":"e30="}]}}).to_string(),
    )
    .unwrap();
    assert_eq!(missing(dir.path()), vec![LOST]);
    fs::write(dir.path().join("recovered.jsonl"), "{}").unwrap();
    assert!(missing(dir.path()).is_empty());
}

#[test]
fn errors_in_databases_or_backups_abort_the_scan() {
    let dir = tempdir().unwrap();
    fs::create_dir(dir.path().join("sqlite")).unwrap();
    fs::write(dir.path().join("sqlite/broken.db"), "broken").unwrap();
    assert!(scan_session_health(dir.path(), &dir.path().join("backups"), &[LOST.into()]).is_err());
    let dir = tempdir().unwrap();
    fs::create_dir(dir.path().join("backups")).unwrap();
    fs::write(dir.path().join("backups/undo.json"), "broken").unwrap();
    assert!(scan_session_health(dir.path(), &dir.path().join("backups"), &[LOST.into()]).is_err());
}

#[test]
fn another_database_or_unavailable_external_location_protects_the_thread() {
    let dir = tempdir().unwrap();
    let external = tempdir().unwrap();
    let db = database(dir.path());
    db.execute(
        "INSERT INTO threads VALUES (?1, ?2, 0)",
        [
            LOST,
            external.path().join("unmounted.jsonl").to_str().unwrap(),
        ],
    )
    .unwrap();
    db.execute(
        "INSERT INTO threads VALUES (?1, 'missing.jsonl', 0)",
        [KEPT],
    )
    .unwrap();
    fs::create_dir(dir.path().join("sqlite")).unwrap();
    let other = Connection::open(dir.path().join("sqlite/new.db")).unwrap();
    other
        .execute_batch("CREATE TABLE threads (id TEXT, rollout_path TEXT);")
        .unwrap();
    fs::write(dir.path().join("valid.jsonl"), "{}").unwrap();
    other
        .execute("INSERT INTO threads VALUES (?1, 'valid.jsonl')", [KEPT])
        .unwrap();
    assert!(missing(dir.path()).is_empty());
}

#[test]
fn observed_ids_are_normalized_and_placeholders_are_ignored() {
    let dir = tempdir().unwrap();
    let scan = scan_session_health(
        dir.path(),
        &dir.path().join("backups"),
        &[
            format!("local:{LOST}"),
            LOST.into(),
            "client-new-thread:pending".into(),
        ],
    )
    .unwrap();
    assert_eq!(scan.scanned, 1);
    assert_eq!(scan.missing_ids, vec![LOST]);
}

#[test]
fn automatic_recheck_only_queries_observed_sessions() {
    let dir = tempdir().unwrap();
    let db = database(dir.path());
    db.execute(
        "INSERT INTO threads VALUES (?1, 'missing.jsonl', 0)",
        [LOST],
    )
    .unwrap();
    db.execute(
        "INSERT INTO threads VALUES (?1, 'also-missing.jsonl', 0)",
        [KEPT],
    )
    .unwrap();

    let scan = recheck_session_health(
        dir.path(),
        &dir.path().join("backups"),
        &[format!("local:{LOST}")],
    )
    .unwrap();

    assert_eq!(scan.scanned, 1);
    assert_eq!(scan.missing_ids, vec![LOST]);
}

#[test]
fn automatic_recheck_skips_unrelated_backup_bodies() {
    let dir = tempdir().unwrap();
    fs::create_dir(dir.path().join("backups")).unwrap();
    fs::write(
        dir.path().join("backups/unrelated.json"),
        format!(r#"{{"session_id":"{KEPT}","tables":BROKEN}}"#),
    )
    .unwrap();

    let scan = recheck_session_health(dir.path(), &dir.path().join("backups"), &[LOST.to_string()])
        .unwrap();

    assert_eq!(scan.missing_ids, vec![LOST]);
    assert!(scan_session_health(dir.path(), &dir.path().join("backups"), &[]).is_err());
}

#[test]
fn metadata_only_backups_do_not_turn_dead_threads_into_recoverable_threads() {
    let dir = tempdir().unwrap();
    let db = database(dir.path());
    db.execute(
        "INSERT INTO threads VALUES (?1, 'missing.jsonl', 0)",
        [LOST],
    )
    .unwrap();
    fs::create_dir_all(dir.path().join("backups_state/provider-sync/db")).unwrap();
    let backup = Connection::open(
        dir.path()
            .join("backups_state/provider-sync/db/state_5.sqlite"),
    )
    .unwrap();
    backup
        .execute_batch("CREATE TABLE threads (id TEXT, rollout_path TEXT)")
        .unwrap();
    backup
        .execute("INSERT INTO threads VALUES (?1, 'missing.jsonl')", [LOST])
        .unwrap();
    fs::create_dir(dir.path().join("backups")).unwrap();
    fs::write(
        dir.path().join("backups/index-only.json"),
        json!({"session_id": LOST, "tables":{"threads":[{"id":LOST}], "__files":[]}}).to_string(),
    )
    .unwrap();
    assert_eq!(missing(dir.path()), vec![LOST]);
}
