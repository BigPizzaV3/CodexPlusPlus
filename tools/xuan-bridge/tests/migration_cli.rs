use std::fs;
use std::process::Command;

use rusqlite::Connection;
use serde_json::Value;
use tempfile::tempdir;
use xuan_bridge::DATABASE_SCHEMA_VERSION;

#[test]
fn migration_cli_creates_config_database_and_read_only_backup() {
    let output_root = tempdir().unwrap();
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/legacy-settings.json");
    let output = Command::new(env!("CARGO_BIN_EXE_xuan-bridge"))
        .arg("migrate")
        .arg(&fixture)
        .arg(output_root.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "migration failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    let config = fs::read_to_string(result["configPath"].as_str().unwrap()).unwrap();
    assert!(config.contains("xuan-workspace-search"));
    assert!(config.contains("owlai"));
    assert!(config.contains("gpt-test"));
    assert!(config.contains("chat-completions"));
    assert!(config.contains("credentialMigrationRequired"));
    assert!(!config.contains("must-not-be-migrated"));
    assert!(
        fs::metadata(result["backupPath"].as_str().unwrap())
            .unwrap()
            .permissions()
            .readonly()
    );

    let connection = Connection::open(result["databasePath"].as_str().unwrap()).unwrap();
    let user_version: u32 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    let migration_count: u32 = connection
        .query_row("SELECT COUNT(*) FROM migration_runs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(user_version, DATABASE_SCHEMA_VERSION);
    assert_eq!(migration_count, 1);
}
