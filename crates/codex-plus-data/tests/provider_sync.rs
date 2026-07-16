use codex_plus_data::{
    ProviderSyncStatus, ProviderSyncTargetSource, apply_session_index_cleanup,
    load_provider_sync_targets, preview_session_index_cleanup, run_provider_sync,
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

fn session_index_line(id: &str, title: &str) -> String {
    json!({
        "id": id,
        "thread_name": title,
        "updated_at": "2026-07-13T12:00:00.000Z"
    })
    .to_string()
}

fn write_rollout_with_providers(path: &Path, providers: &[&str], thread_id: &str, cwd: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut lines = Vec::new();
    for provider in providers {
        lines.push(
            json!({
                "type": "session_meta",
                "payload": {
                    "id": thread_id,
                    "model_provider": provider,
                    "cwd": cwd
                }
            })
            .to_string(),
        );
        lines.push(json!({"type": "event_msg", "payload": {"type": "task_started"}}).to_string());
    }
    lines.push(json!({"type": "event_msg", "payload": {"type": "user_message"}}).to_string());
    fs::write(path, format!("{}\n", lines.join("\n"))).unwrap();
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
fn provider_sync_rewrites_all_session_meta_model_providers() {
    let tmp = tempdir().unwrap();
    let home = tmp.path().join(".codex");
    fs::create_dir(&home).unwrap();
    fs::write(home.join("config.toml"), "model_provider = \"apigather\"\n").unwrap();
    let rollout = home.join("sessions/2026/rollout-multi-meta.jsonl");
    write_rollout_with_providers(
        &rollout,
        &["openai", "ccx", "CodexPlusPlus"],
        "thread-1",
        "C:/workspace",
    );
    create_state_db(&home.join("state_5.sqlite"));

    let result = run_provider_sync(Some(&home));

    assert_eq!(result.status, ProviderSyncStatus::Synced);
    assert_eq!(result.target_provider, "apigather");
    assert_eq!(result.changed_session_files, 1);

    let providers = fs::read_to_string(&rollout)
        .unwrap()
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|record| record["type"] == "session_meta")
        .map(|record| {
            record["payload"]["model_provider"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect::<Vec<_>>();
    assert_eq!(providers, vec!["apigather", "apigather", "apigather"]);
}

#[test]
fn provider_sync_target_discovery_reads_all_session_meta_providers() {
    let tmp = tempdir().unwrap();
    let home = tmp.path().join(".codex");
    fs::create_dir(&home).unwrap();
    fs::write(home.join("config.toml"), "model_provider = \"custom\"\n").unwrap();
    write_rollout_with_providers(
        &home.join("sessions/2026/rollout-multi-meta.jsonl"),
        &["openai", "ccx", "CodexPlusPlus"],
        "thread-1",
        "C:/workspace",
    );

    let targets = load_provider_sync_targets(Some(&home));
    let ids = targets
        .targets
        .iter()
        .map(|target| target.id.as_str())
        .collect::<Vec<_>>();

    assert!(ids.contains(&"openai"));
    assert!(ids.contains(&"ccx"));
    assert!(ids.contains(&"CodexPlusPlus"));
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
fn provider_sync_updates_new_codex_sqlite_directory_db() {
    let tmp = tempdir().unwrap();
    let home = tmp.path().join(".codex");
    let sqlite_dir = home.join("sqlite");
    fs::create_dir_all(&sqlite_dir).unwrap();
    fs::write(home.join("config.toml"), "model_provider = \"apigather\"\n").unwrap();
    let rollout = home.join("sessions/2026/rollout-abc.jsonl");
    write_rollout(&rollout, "openai", "thread-1", "C:/workspace");
    let db_path = sqlite_dir.join("codex-dev.db");
    create_state_db(&db_path);

    let result = run_provider_sync(Some(&home));

    assert_eq!(result.status, ProviderSyncStatus::Synced);
    assert_eq!(result.sqlite_rows_updated, 3);
    let db = Connection::open(&db_path).unwrap();
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
    assert!(backup_dir.join("db/sqlite/codex-dev.db").exists());
}

#[test]
fn provider_sync_backup_metadata_contains_reference_fields_and_managed_marker() {
    let tmp = tempdir().unwrap();
    let home = tmp.path().join(".codex");
    fs::create_dir(&home).unwrap();
    fs::write(home.join("config.toml"), "model_provider = \"apigather\"\n").unwrap();
    write_rollout(
        &home.join("sessions/rollout-backup.jsonl"),
        "openai",
        "thread-1",
        "C:/workspace",
    );
    create_state_db(&home.join("state_5.sqlite"));

    let result = run_provider_sync(Some(&home));

    assert_eq!(result.status, ProviderSyncStatus::Synced);
    let backup_dir = result.backup_dir.unwrap();
    let metadata: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(backup_dir.join("metadata.json")).unwrap())
            .unwrap();
    assert_eq!(metadata["version"], 1);
    assert_eq!(metadata["namespace"], "provider-sync");
    assert_eq!(metadata["codexHome"], home.to_string_lossy().to_string());
    assert_eq!(metadata["targetProvider"], "apigather");
    assert_eq!(metadata["changedSessionFiles"], 1);
    assert_eq!(metadata["managedBy"], "Codex++ provider sync");
    assert!(metadata["createdAt"].as_str().unwrap().contains('T'));
    assert!(
        metadata["dbFiles"]
            .as_array()
            .unwrap()
            .contains(&json!("state_5.sqlite"))
    );
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
fn provider_sync_does_not_restore_cwd_for_projectless_threads() {
    let tmp = tempdir().unwrap();
    let home = tmp.path().join(".codex");
    fs::create_dir(&home).unwrap();
    fs::write(home.join("config.toml"), "model_provider = \"apigather\"\n").unwrap();
    write_rollout(
        &home.join("sessions/rollout-projectless.jsonl"),
        "apigather",
        "thread-1",
        "C:/old/project",
    );
    create_state_db(&home.join("state_5.sqlite"));
    fs::write(
        home.join(".codex-global-state.json"),
        json!({
            "projectless-thread-ids": ["thread-1"]
        })
        .to_string(),
    )
    .unwrap();

    let result = run_provider_sync(Some(&home));

    assert_eq!(result.status, ProviderSyncStatus::Synced);
    assert_eq!(result.sqlite_cwd_rows_updated, 0);
    let db = Connection::open(home.join("state_5.sqlite")).unwrap();
    let row: String = db
        .query_row("SELECT cwd FROM threads WHERE id = 'thread-1'", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(row, "C:/old");
}

#[test]
fn provider_sync_normalizes_open_in_target_preferences_per_path() {
    let tmp = tempdir().unwrap();
    let home = tmp.path().join(".codex");
    fs::create_dir(&home).unwrap();
    fs::write(home.join("config.toml"), "model_provider = \"apigather\"\n").unwrap();
    write_rollout(
        &home.join("sessions/rollout-current.jsonl"),
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
            "active-workspace-roots": ["\\\\?\\C:\\workspace"],
            "electron-workspace-root-labels": {"\\\\?\\C:\\workspace": "Workspace"},
            "open-in-target-preferences": {
                "perPath": {
                    "\\\\?\\C:\\workspace": "terminal"
                }
            }
        })
        .to_string(),
    )
    .unwrap();

    let result = run_provider_sync(Some(&home));

    assert_eq!(result.status, ProviderSyncStatus::Synced);
    let state: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(home.join(".codex-global-state.json")).unwrap())
            .unwrap();
    assert_eq!(
        state["open-in-target-preferences"]["perPath"],
        json!({"C:/workspace": "terminal"})
    );
    assert!(home.join(".codex-global-state.json.bak").exists());
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
fn provider_sync_restores_global_state_when_later_step_fails() {
    let tmp = tempdir().unwrap();
    let home = tmp.path().join(".codex");
    fs::create_dir(&home).unwrap();
    fs::write(home.join("config.toml"), "model_provider = \"apigather\"\n").unwrap();
    write_rollout(
        &home.join("sessions/rollout-current.jsonl"),
        "apigather",
        "thread-1",
        "\\\\?\\C:\\workspace",
    );
    create_state_db(&home.join("state_5.sqlite"));
    let state_path = home.join(".codex-global-state.json");
    let original_state = json!({
        "electron-saved-workspace-roots": ["\\\\?\\C:\\workspace"],
        "project-order": ["\\\\?\\C:\\workspace"]
    })
    .to_string();
    fs::write(&state_path, &original_state).unwrap();
    fs::create_dir_all(home.join("backups_state/provider-sync/blocker")).unwrap();
    fs::write(
        home.join("backups_state/provider-sync/blocker/metadata.json"),
        json!({"managedBy": "Codex++ provider sync"}).to_string(),
    )
    .unwrap();

    let result = run_provider_sync_with_target(Some(&home), Some("bad/provider"));

    assert_eq!(result.status, ProviderSyncStatus::Skipped);
    assert_eq!(fs::read_to_string(&state_path).unwrap(), original_state);
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

#[test]
fn provider_sync_never_prunes_unconfirmed_or_delayed_index_entries() {
    let tmp = tempdir().unwrap();
    let home = tmp.path().join(".codex");
    fs::create_dir(&home).unwrap();
    fs::write(home.join("config.toml"), "model_provider = \"custom\"\n").unwrap();
    let stale_id = "019f4e36-490e-7ae0-8e78-a8b3ab33a428";
    let original_index = format!("{}\n", session_index_line(stale_id, "可能仍在云端同步"));
    fs::write(home.join("session_index.jsonl"), &original_index).unwrap();

    let result = run_provider_sync(Some(&home));

    assert_eq!(result.status, ProviderSyncStatus::Synced);
    assert_eq!(
        fs::read_to_string(home.join("session_index.jsonl")).unwrap(),
        original_index
    );
    let preview = preview_session_index_cleanup(Some(&home)).unwrap();
    assert_eq!(preview.candidates.len(), 1);
    assert_eq!(
        fs::read_to_string(home.join("session_index.jsonl")).unwrap(),
        original_index
    );
}

#[test]
fn session_index_cleanup_preserves_all_local_sources_and_unknown_records() {
    let tmp = tempdir().unwrap();
    let home = tmp.path().join(".codex");
    let sqlite_dir = home.join("sqlite");
    fs::create_dir_all(&sqlite_dir).unwrap();
    let rollout_id = "019f480d-bbc6-7b62-8a46-99597db8bde7";
    let threads_id = "019f4844-43aa-7862-b51c-e04d5686700e";
    let catalog_id = "019f52f8-7c7e-7bd3-91f0-d662451867be";
    let automation_id = "019f52f8-7c7e-7bd3-91f0-d662451867bf";
    let inbox_id = "019f52f8-7c7e-7bd3-91f0-d662451867c0";
    let stale_id = "019f4e36-490e-7ae0-8e78-a8b3ab33a428";
    let rollout = home.join(format!(
        "sessions/rollout-2026-07-12T04-57-28-{rollout_id}.jsonl"
    ));
    fs::create_dir_all(rollout.parent().unwrap()).unwrap();
    fs::write(&rollout, "{\"type\":\"event_msg\"}\n").unwrap();
    create_state_db_with_providers(&home.join("state_5.sqlite"), &[(threads_id, "custom", 0)]);
    let db = Connection::open(sqlite_dir.join("codex-dev.db")).unwrap();
    db.execute(
        "CREATE TABLE local_thread_catalog (thread_id TEXT PRIMARY KEY)",
        [],
    )
    .unwrap();
    db.execute(
        "CREATE TABLE automation_runs (thread_id TEXT PRIMARY KEY)",
        [],
    )
    .unwrap();
    db.execute(
        "CREATE TABLE inbox_items (id TEXT PRIMARY KEY, thread_id TEXT)",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO local_thread_catalog VALUES (?1)", [catalog_id])
        .unwrap();
    db.execute("INSERT INTO automation_runs VALUES (?1)", [automation_id])
        .unwrap();
    db.execute("INSERT INTO inbox_items VALUES ('item-1', ?1)", [inbox_id])
        .unwrap();
    drop(db);
    let unknown = json!({"id": "future-record", "kind": "cloud_task"}).to_string();
    let malformed = "not-json";
    let original_index = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{unknown}\n{malformed}\n",
        session_index_line(rollout_id, "rollout"),
        session_index_line(threads_id, "threads"),
        session_index_line(catalog_id, "catalog"),
        session_index_line(automation_id, "automation"),
        session_index_line(inbox_id, "inbox"),
        session_index_line(stale_id, "stale"),
    );
    fs::write(home.join("session_index.jsonl"), &original_index).unwrap();

    let preview = preview_session_index_cleanup(Some(&home)).unwrap();

    assert_eq!(preview.candidates.len(), 1);
    assert_eq!(preview.candidates[0].id, stale_id);
    let result = apply_session_index_cleanup(
        Some(&home),
        &preview.snapshot_sha256,
        &[stale_id.to_string()],
    )
    .unwrap();
    assert_eq!(result.pruned_entries, 1);
    let next_index = fs::read_to_string(home.join("session_index.jsonl")).unwrap();
    for id in [rollout_id, threads_id, catalog_id, automation_id, inbox_id] {
        assert!(next_index.contains(id));
    }
    assert!(!next_index.contains(stale_id));
    assert!(next_index.contains(&unknown));
    assert!(next_index.contains(malformed));
    let backup = result.backup_dir.expect("cleanup backup");
    assert_eq!(
        fs::read_to_string(backup.join("session_index.jsonl")).unwrap(),
        original_index
    );
}

#[test]
fn session_index_cleanup_aborts_when_codex_changes_index_after_preview() {
    let tmp = tempdir().unwrap();
    let home = tmp.path().join(".codex");
    fs::create_dir(&home).unwrap();
    let stale_id = "019f4e36-490e-7ae0-8e78-a8b3ab33a428";
    fs::write(
        home.join("session_index.jsonl"),
        format!("{}\n", session_index_line(stale_id, "stale")),
    )
    .unwrap();
    let preview = preview_session_index_cleanup(Some(&home)).unwrap();
    let new_id = "019f5e36-490e-7ae0-8e78-a8b3ab33a429";
    let changed = format!(
        "{}\n{}\n",
        session_index_line(stale_id, "stale"),
        session_index_line(new_id, "Codex 新建任务"),
    );
    fs::write(home.join("session_index.jsonl"), &changed).unwrap();

    let error = apply_session_index_cleanup(
        Some(&home),
        &preview.snapshot_sha256,
        &[stale_id.to_string()],
    )
    .unwrap_err();

    assert!(error.message.contains("发生变化"));
    assert!(error.backup_dir.is_none());
    assert_eq!(
        fs::read_to_string(home.join("session_index.jsonl")).unwrap(),
        changed
    );
}

#[test]
fn session_index_preview_preserves_relation_only_sqlite_thread_references() {
    let tmp = tempdir().unwrap();
    let home = tmp.path().join(".codex");
    let sqlite_dir = home.join("sqlite");
    fs::create_dir_all(&sqlite_dir).unwrap();
    let db = Connection::open(sqlite_dir.join("codex-related.db")).unwrap();
    db.execute("CREATE TABLE sessions (id TEXT PRIMARY KEY)", [])
        .unwrap();
    db.execute("CREATE TABLE messages (session_id TEXT)", [])
        .unwrap();
    db.execute("CREATE TABLE thread_dynamic_tools (thread_id TEXT)", [])
        .unwrap();
    db.execute("CREATE TABLE thread_goals (thread_id TEXT)", [])
        .unwrap();
    db.execute(
        "CREATE TABLE thread_spawn_edges (parent_thread_id TEXT, child_thread_id TEXT)",
        [],
    )
    .unwrap();
    db.execute("CREATE TABLE stage1_outputs (thread_id TEXT)", [])
        .unwrap();
    db.execute("CREATE TABLE agent_job_items (assigned_thread_id TEXT)", [])
        .unwrap();
    let ids = [
        "019f6000-0000-7000-8000-000000000001",
        "019f6000-0000-7000-8000-000000000002",
        "019f6000-0000-7000-8000-000000000003",
        "019f6000-0000-7000-8000-000000000004",
        "019f6000-0000-7000-8000-000000000005",
        "019f6000-0000-7000-8000-000000000006",
        "019f6000-0000-7000-8000-000000000007",
        "019f6000-0000-7000-8000-000000000008",
    ];
    db.execute("INSERT INTO sessions VALUES (?1)", [ids[0]])
        .unwrap();
    db.execute("INSERT INTO messages VALUES (?1)", [ids[1]])
        .unwrap();
    db.execute("INSERT INTO thread_dynamic_tools VALUES (?1)", [ids[2]])
        .unwrap();
    db.execute("INSERT INTO thread_goals VALUES (?1)", [ids[3]])
        .unwrap();
    db.execute(
        "INSERT INTO thread_spawn_edges VALUES (?1, ?2)",
        [ids[4], ids[5]],
    )
    .unwrap();
    db.execute("INSERT INTO stage1_outputs VALUES (?1)", [ids[6]])
        .unwrap();
    db.execute("INSERT INTO agent_job_items VALUES (?1)", [ids[7]])
        .unwrap();
    drop(db);

    let relation_db = sqlite_dir.join("codex-related.db");
    assert!(
        !codex_plus_core::codex_sqlite::codex_session_db_paths_from_home(&home)
            .contains(&relation_db),
        "relation-only databases must not enter the shared local-session path list"
    );
    assert!(
        codex_plus_core::codex_sqlite::codex_thread_reference_db_paths_from_home(&home)
            .contains(&relation_db),
        "ghost-index cleanup must still discover relation-only thread references"
    );
    let index = ids
        .iter()
        .map(|id| session_index_line(id, "related"))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    fs::write(home.join("session_index.jsonl"), index).unwrap();

    let preview = preview_session_index_cleanup(Some(&home)).unwrap();

    assert!(preview.candidates.is_empty());
}

#[test]
fn session_index_cleanup_write_failure_reports_backup_and_preserves_original() {
    let tmp = tempdir().unwrap();
    let home = tmp.path().join(".codex");
    fs::create_dir(&home).unwrap();
    let stale_id = "019f4e36-490e-7ae0-8e78-a8b3ab33a428";
    let original = format!("{}\n", session_index_line(stale_id, "stale"));
    fs::write(home.join("session_index.jsonl"), &original).unwrap();
    let preview = preview_session_index_cleanup(Some(&home)).unwrap();
    fs::create_dir(home.join("session_index.jsonl.tmp")).unwrap();

    let error = apply_session_index_cleanup(
        Some(&home),
        &preview.snapshot_sha256,
        &[stale_id.to_string()],
    )
    .unwrap_err();

    assert!(error.message.contains("原子写入"));
    let backup = error.backup_dir.expect("failure must expose backup");
    assert_eq!(
        fs::read_to_string(backup.join("session_index.jsonl")).unwrap(),
        original
    );
    assert_eq!(
        fs::read_to_string(home.join("session_index.jsonl")).unwrap(),
        original
    );
}
