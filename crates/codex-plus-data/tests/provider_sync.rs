use codex_plus_data::{
    ProviderSyncStatus, ProviderSyncTargetSource, load_provider_sync_targets, run_provider_sync,
    run_provider_sync_with_target,
};
use rusqlite::Connection;
use serde_json::json;
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};
use tempfile::tempdir;

fn write_rollout(path: &Path, provider: &str, thread_id: &str, cwd: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let first = json!({
        "type": "session_meta",
        "payload": {
            "id": thread_id,
            "model_provider": provider,
            "cwd": cwd
        }
    });
    let event = json!({"type": "event_msg", "payload": {"type": "user_message"}});
    fs::write(path, format!("{first}\n{event}\n")).unwrap();
}

fn create_state_db(path: &Path) {
    let db = Connection::open(path).unwrap();
    db.execute(
        "CREATE TABLE threads (id TEXT PRIMARY KEY, model_provider TEXT, archived INTEGER, has_user_event INTEGER, cwd TEXT)",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO threads VALUES ('thread-1', 'old-provider', 0, 0, 'C:/old')",
        [],
    )
    .unwrap();
}

fn create_state_db_with_providers(path: &Path, rows: &[(&str, &str, i64)]) {
    let db = Connection::open(path).unwrap();
    db.execute(
        "CREATE TABLE threads (id TEXT PRIMARY KEY, model_provider TEXT, archived INTEGER, has_user_event INTEGER, cwd TEXT)",
        [],
    )
    .unwrap();
    for (id, provider, archived) in rows {
        db.execute(
            "INSERT INTO threads VALUES (?1, ?2, ?3, 1, 'C:/workspace')",
            (id, provider, archived),
        )
        .unwrap();
    }
}

#[test]
fn provider_sync_targets_merge_config_rollout_sqlite_and_sort_current_first() {
    let tmp = tempdir().unwrap();
    let home = tmp.path().join(".codex");
    fs::create_dir(&home).unwrap();
    fs::write(
        home.join("config.toml"),
        r#"model_provider = "custom"

[model_providers.custom]
name = "custom"

[model_providers.apigather]
name = "apigather"
"#,
    )
    .unwrap();
    write_rollout(
        &home.join("sessions/2026/rollout-openai.jsonl"),
        "openai",
        "thread-openai",
        "C:/workspace/openai",
    );
    write_rollout(
        &home.join("archived_sessions/rollout-legacy.jsonl"),
        "legacy-provider",
        "thread-legacy",
        "C:/workspace/legacy",
    );
    create_state_db_with_providers(
        &home.join("state_5.sqlite"),
        &[
            ("thread-sqlite", "sqlite-provider", 0),
            ("thread-openai", "openai", 1),
        ],
    );

    let targets = load_provider_sync_targets(Some(&home));

    assert_eq!(targets.current_provider, "custom");
    let ids = targets
        .targets
        .iter()
        .map(|target| target.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        vec![
            "custom",
            "apigather",
            "legacy-provider",
            "openai",
            "sqlite-provider",
        ]
    );
    let custom = targets
        .targets
        .iter()
        .find(|target| target.id == "custom")
        .unwrap();
    assert!(custom.is_current_provider);
    assert!(custom.sources.contains(&ProviderSyncTargetSource::Config));
    let openai = targets
        .targets
        .iter()
        .find(|target| target.id == "openai")
        .unwrap();
    assert!(openai.sources.contains(&ProviderSyncTargetSource::Config));
    assert!(openai.sources.contains(&ProviderSyncTargetSource::Rollout));
    assert!(openai.sources.contains(&ProviderSyncTargetSource::Sqlite));
    let legacy = targets
        .targets
        .iter()
        .find(|target| target.id == "legacy-provider")
        .unwrap();
    assert_eq!(legacy.sources, vec![ProviderSyncTargetSource::Rollout]);
}

#[test]
fn provider_sync_maps_official_mixed_to_custom_provider_id() {
    let tmp = tempdir().unwrap();
    let home = tmp.path().join(".codex");
    fs::create_dir(&home).unwrap();
    fs::write(
        home.join("config.toml"),
        r#"model_provider = "custom"

[model_providers.custom]
name = "custom"
wire_api = "responses"
requires_openai_auth = true
base_url = "https://example.com/v1"
experimental_bearer_token = "sk-test"
"#,
    )
    .unwrap();
    let rollout = home.join("sessions/2026/rollout-official-mix.jsonl");
    write_rollout(&rollout, "openai", "thread-1", "C:/workspace");
    create_state_db(&home.join("state_5.sqlite"));

    let result = run_provider_sync(Some(&home));

    assert_eq!(result.status, ProviderSyncStatus::Synced);
    assert_eq!(result.target_provider, "custom");
    assert_eq!(result.changed_session_files, 1);
    assert_eq!(result.sqlite_provider_rows_updated, 1);
    let first: serde_json::Value = serde_json::from_str(
        fs::read_to_string(&rollout)
            .unwrap()
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(first["payload"]["model_provider"], "custom");
    let db = Connection::open(home.join("state_5.sqlite")).unwrap();
    let provider: String = db
        .query_row(
            "SELECT model_provider FROM threads WHERE id = 'thread-1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(provider, "custom");
}

#[test]
fn provider_sync_updates_rollout_sqlite_visibility_and_creates_backup() {
    let tmp = tempdir().unwrap();
    let home = tmp.path().join(".codex");
    fs::create_dir(&home).unwrap();
    fs::write(home.join("config.toml"), "model_provider = \"apigather\"\n").unwrap();
    let rollout = home.join("sessions/2026/rollout-abc.jsonl");
    write_rollout(&rollout, "openai", "thread-1", "C:/workspace");
    create_state_db(&home.join("state_5.sqlite"));

    let result = run_provider_sync(Some(&home));

    assert_eq!(result.status, ProviderSyncStatus::Synced);
    assert_eq!(result.target_provider, "apigather");
    assert_eq!(result.changed_session_files, 1);
    assert_eq!(result.sqlite_rows_updated, 3);
    assert_eq!(result.sqlite_provider_rows_updated, 1);
    assert_eq!(result.sqlite_user_event_rows_updated, 1);
    assert_eq!(result.sqlite_cwd_rows_updated, 1);
    let first: serde_json::Value = serde_json::from_str(
        fs::read_to_string(&rollout)
            .unwrap()
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(first["payload"]["model_provider"], "apigather");
    let db = Connection::open(home.join("state_5.sqlite")).unwrap();
    let row = db
        .query_row(
            "SELECT model_provider, has_user_event, cwd FROM threads WHERE id = 'thread-1'",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(
        row,
        ("apigather".to_string(), 1, "C:/workspace".to_string())
    );
    let backup_dir = result.backup_dir.unwrap();
    assert!(backup_dir.join("session-meta-backup.json").exists());
    assert!(backup_dir.join("db/state_5.sqlite").exists());
}

#[test]
fn provider_sync_explicit_target_overrides_config_without_switching_config() {
    let tmp = tempdir().unwrap();
    let home = tmp.path().join(".codex");
    fs::create_dir(&home).unwrap();
    fs::write(home.join("config.toml"), "model_provider = \"apigather\"\n").unwrap();
    let rollout = home.join("sessions/2026/rollout-target.jsonl");
    write_rollout(&rollout, "openai", "thread-1", "C:/workspace");
    create_state_db(&home.join("state_5.sqlite"));

    let result = run_provider_sync_with_target(Some(&home), Some("custom"));

    assert_eq!(result.status, ProviderSyncStatus::Synced);
    assert_eq!(result.target_provider, "custom");
    assert_eq!(
        fs::read_to_string(home.join("config.toml")).unwrap(),
        "model_provider = \"apigather\"\n"
    );
    let first: serde_json::Value = serde_json::from_str(
        fs::read_to_string(&rollout)
            .unwrap()
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(first["payload"]["model_provider"], "custom");
    let db = Connection::open(home.join("state_5.sqlite")).unwrap();
    let provider: String = db
        .query_row(
            "SELECT model_provider FROM threads WHERE id = 'thread-1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(provider, "custom");
}

#[test]
fn provider_sync_rejects_invalid_explicit_target_before_writes() {
    let tmp = tempdir().unwrap();
    let home = tmp.path().join(".codex");
    fs::create_dir(&home).unwrap();
    fs::write(home.join("config.toml"), "model_provider = \"apigather\"\n").unwrap();
    let rollout = home.join("sessions/rollout-invalid-target.jsonl");
    write_rollout(&rollout, "openai", "thread-1", "C:/workspace");
    let original = fs::read_to_string(&rollout).unwrap();

    let result = run_provider_sync_with_target(Some(&home), Some("bad\nprovider"));

    assert_eq!(result.status, ProviderSyncStatus::Skipped);
    assert!(result.message.contains("Invalid provider sync target"));
    assert_eq!(fs::read_to_string(&rollout).unwrap(), original);
    assert!(result.backup_dir.is_none());
}

#[test]
fn provider_sync_repairs_sqlite_when_rollout_provider_matches_and_normalizes_paths() {
    let tmp = tempdir().unwrap();
    let home = tmp.path().join(".codex");
    fs::create_dir(&home).unwrap();
    fs::write(home.join("config.toml"), "model_provider = \"apigather\"\n").unwrap();
    write_rollout(
        &home.join("archived_sessions/rollout-current.jsonl"),
        "apigather",
        "thread-1",
        "\\\\?\\C:\\workspace",
    );
    create_state_db(&home.join("state_5.sqlite"));
    fs::write(
        home.join(".codex-global-state.json"),
        json!({
            "electron-saved-workspace-roots": ["\\\\?\\C:\\workspace"],
            "project-order": ["\\\\?\\C:\\workspace"],
            "active-workspace-roots": "\\\\?\\C:\\workspace",
            "electron-workspace-root-labels": {"\\\\?\\C:\\workspace": "Workspace"}
        })
        .to_string(),
    )
    .unwrap();

    let result = run_provider_sync(Some(&home));

    assert_eq!(result.status, ProviderSyncStatus::Synced);
    assert_eq!(result.changed_session_files, 0);
    assert_eq!(result.sqlite_rows_updated, 3);
    assert_eq!(result.sqlite_provider_rows_updated, 1);
    assert_eq!(result.sqlite_user_event_rows_updated, 1);
    assert_eq!(result.sqlite_cwd_rows_updated, 1);
    let db = Connection::open(home.join("state_5.sqlite")).unwrap();
    let row: String = db
        .query_row("SELECT cwd FROM threads WHERE id = 'thread-1'", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(row, "C:/workspace");
    let state: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(home.join(".codex-global-state.json")).unwrap())
            .unwrap();
    assert_eq!(
        state["electron-saved-workspace-roots"],
        json!(["C:/workspace"])
    );
    assert_eq!(state["project-order"], json!(["C:/workspace"]));
    assert_eq!(state["active-workspace-roots"], json!("C:/workspace"));
    assert_eq!(
        state["electron-workspace-root-labels"],
        json!({"C:/workspace": "Workspace"})
    );
}

#[test]
fn provider_sync_restores_rollout_first_line_when_later_step_fails() {
    let tmp = tempdir().unwrap();
    let home = tmp.path().join(".codex");
    fs::create_dir(&home).unwrap();
    fs::write(home.join("config.toml"), "model_provider = \"apigather\"\n").unwrap();
    let rollout = home.join("sessions/rollout-needs-rewrite.jsonl");
    write_rollout(&rollout, "openai", "thread-1", "C:/workspace");
    let original_first_line = fs::read_to_string(&rollout)
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .to_string();
    let db = Connection::open(home.join("state_5.sqlite")).unwrap();
    db.execute(
        "CREATE TABLE threads (id TEXT PRIMARY KEY, model_provider TEXT, archived INTEGER, has_user_event INTEGER, cwd TEXT)",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO threads VALUES ('thread-1', 'old-provider', 0, 0, 'C:/old')",
        [],
    )
    .unwrap();
    db.execute(
        "CREATE TRIGGER fail_provider_sync_update BEFORE UPDATE ON threads BEGIN SELECT RAISE(ABORT, 'boom'); END",
        [],
    )
    .unwrap();
    drop(db);

    let result = run_provider_sync(Some(&home));

    assert_eq!(result.status, ProviderSyncStatus::Skipped);
    assert!(result.message.contains("Provider sync skipped"));
    let restored_first_line = fs::read_to_string(&rollout)
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .to_string();
    assert_eq!(restored_first_line, original_first_line);
}

#[test]
fn provider_sync_rolls_back_sqlite_provider_update_when_later_update_fails() {
    let tmp = tempdir().unwrap();
    let home = tmp.path().join(".codex");
    fs::create_dir(&home).unwrap();
    fs::write(home.join("config.toml"), "model_provider = \"apigather\"\n").unwrap();
    write_rollout(
        &home.join("sessions/rollout-current.jsonl"),
        "apigather",
        "thread-1",
        "C:/workspace",
    );
    let db = Connection::open(home.join("state_5.sqlite")).unwrap();
    db.execute(
        "CREATE TABLE threads (id TEXT PRIMARY KEY, model_provider TEXT, archived INTEGER, has_user_event INTEGER, cwd TEXT)",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO threads VALUES ('thread-1', 'old-provider', 0, 1, 'C:/old')",
        [],
    )
    .unwrap();
    db.execute(
        "CREATE TRIGGER fail_cwd_update BEFORE UPDATE OF cwd ON threads BEGIN SELECT RAISE(ABORT, 'boom'); END",
        [],
    )
    .unwrap();
    drop(db);

    let result = run_provider_sync(Some(&home));

    assert_eq!(result.status, ProviderSyncStatus::Skipped);
    let db = Connection::open(home.join("state_5.sqlite")).unwrap();
    let row = db
        .query_row(
            "SELECT model_provider, has_user_event, cwd FROM threads WHERE id = 'thread-1'",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(row, ("old-provider".to_string(), 1, "C:/old".to_string()));
}

#[test]
fn provider_sync_skips_when_home_missing_or_lock_exists_and_prunes_backups() {
    let tmp = tempdir().unwrap();
    let missing = tmp.path().join(".missing");
    let result = run_provider_sync(Some(&missing));
    assert_eq!(result.status, ProviderSyncStatus::Skipped);

    let home = tmp.path().join(".codex");
    fs::create_dir(&home).unwrap();
    fs::create_dir_all(home.join("tmp/provider-sync.lock")).unwrap();
    fs::write(home.join("config.toml"), "model_provider = \"apigather\"\n").unwrap();
    let result = run_provider_sync(Some(&home));
    assert_eq!(result.status, ProviderSyncStatus::Skipped);
    assert!(result.message.to_lowercase().contains("lock"));

    fs::remove_dir_all(home.join("tmp/provider-sync.lock")).unwrap();
    let backup_root = home.join("backups_state/provider-sync");
    for index in 0..6 {
        let backup = backup_root.join(format!("2000010100000{index}"));
        fs::create_dir_all(&backup).unwrap();
        fs::write(
            backup.join("metadata.json"),
            json!({"managedBy": "Codex++ provider sync"}).to_string(),
        )
        .unwrap();
    }
    write_rollout(
        &home.join("sessions/rollout-new.jsonl"),
        "openai",
        "thread-1",
        "C:/workspace",
    );
    let result = run_provider_sync(Some(&home));
    assert_eq!(result.status, ProviderSyncStatus::Synced);
    let backups = fs::read_dir(&backup_root)
        .unwrap()
        .filter(|entry| entry.as_ref().unwrap().path().is_dir())
        .count();
    assert_eq!(backups, 5);
}

#[test]
fn provider_sync_preserves_rollout_mtime() {
    let tmp = tempdir().unwrap();
    let home = tmp.path().join(".codex");
    fs::create_dir(&home).unwrap();
    fs::write(home.join("config.toml"), "model_provider = \"apigather\"\n").unwrap();
    let rollout = home.join("sessions/2026/rollout-mtime.jsonl");
    write_rollout(&rollout, "openai", "thread-1", "C:/workspace");

    let past = SystemTime::now() - Duration::from_secs(86400);
    let file = fs::File::options().write(true).open(&rollout).unwrap();
    file.set_times(fs::FileTimes::new().set_modified(past))
        .unwrap();
    drop(file);

    let mtime_before = fs::metadata(&rollout).unwrap().modified().unwrap();

    let result = run_provider_sync(Some(&home));

    assert_eq!(result.status, ProviderSyncStatus::Synced);
    assert_eq!(result.changed_session_files, 1);

    let mtime_after = fs::metadata(&rollout).unwrap().modified().unwrap();
    let drift = mtime_after
        .duration_since(mtime_before)
        .or_else(|e| Ok::<_, std::convert::Infallible>(e.duration()))
        .unwrap();
    assert!(
        drift < Duration::from_secs(2),
        "mtime drifted by {drift:?}, expected < 2s"
    );
}
