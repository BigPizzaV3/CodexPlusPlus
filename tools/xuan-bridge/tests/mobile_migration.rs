use std::fs;

use rusqlite::Connection;
use serde_json::json;
use tempfile::tempdir;

#[test]
fn migration_keeps_read_only_mobile_backup_and_creates_writable_remote_copy() {
    let source = tempdir().unwrap();
    let output = tempdir().unwrap();
    let settings = source.path().join("settings.json");
    fs::write(
        &settings,
        serde_json::to_vec(&json!({
            "mobileRemoteEnabled": true,
            "mobileRemoteAutoSync": true,
        }))
        .unwrap(),
    )
    .unwrap();
    let legacy_database = source.path().join("mobile-remote.sqlite");
    let connection = Connection::open(&legacy_database).unwrap();
    connection
        .execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE mobile_identity(
               singleton INTEGER PRIMARY KEY,
               protected_key BLOB NOT NULL,
               state TEXT NOT NULL
             );
             CREATE TABLE mobile_command_receipts(
               command_id TEXT PRIMARY KEY,
               payload_digest TEXT NOT NULL,
               status TEXT NOT NULL,
               error_code TEXT,
               created_at INTEGER NOT NULL
             );",
        )
        .unwrap();
    connection
        .execute("INSERT INTO mobile_identity VALUES(1, X'010203', '{}')", [])
        .unwrap();

    let result = xuan_bridge::migrate_legacy_settings(&settings, output.path()).unwrap();
    let writable = result["remoteDatabasePath"].as_str().unwrap();
    assert!(!fs::metadata(writable).unwrap().permissions().readonly());
    let copied = Connection::open(writable).unwrap();
    let tables: u32 = copied
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type='table' AND name IN ('mobile_identity', 'mobile_command_receipts')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(tables, 2);
    let identities: u32 = copied
        .query_row("SELECT COUNT(*) FROM mobile_identity", [], |row| row.get(0))
        .unwrap();
    assert_eq!(identities, 1);

    let backup = result["stateBackups"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|path| {
            let path = path.as_str()?;
            path.ends_with("mobile-remote.sqlite").then_some(path)
        })
        .unwrap();
    assert!(fs::metadata(backup).unwrap().permissions().readonly());
}
