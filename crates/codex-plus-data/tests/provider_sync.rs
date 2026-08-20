use codex_plus_data::{
    ProviderSyncStatus, ProviderSyncTargetSource, apply_session_index_cleanup,
    load_provider_sync_targets, preview_session_index_cleanup,
    remote_control_session_recovery_candidate_exists, run_provider_sync,
    run_provider_sync_with_target,
    run_remote_control_session_catalog_recovery_for_thread_with_target,
    run_remote_control_session_finalization_for_thread_with_target,
};
use rusqlite::Connection;
use serde_json::json;
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tempfile::tempdir;

static CODEX_HOME_ENV_LOCK: Mutex<()> = Mutex::new(());

struct CodexHomeEnvGuard {
    previous: Option<OsString>,
}

impl CodexHomeEnvGuard {
    fn set(path: &Path) -> Self {
        let previous = std::env::var_os("CODEX_HOME");
        unsafe {
            std::env::set_var("CODEX_HOME", path);
        }
        Self { previous }
    }
}

impl Drop for CodexHomeEnvGuard {
    fn drop(&mut self) {
        unsafe {
            match &self.previous {
                Some(value) => std::env::set_var("CODEX_HOME", value),
                None => std::env::remove_var("CODEX_HOME"),
            }
        }
    }
}

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

fn write_subagent_rollout(path: &Path, provider: &str, thread_id: &str, cwd: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let first = json!({
        "type": "session_meta",
        "payload": {
            "id": thread_id,
            "model_provider": provider,
            "cwd": cwd,
            "source": { "subagent": { "thread_spawn": { "depth": 1 } } }
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

fn create_remote_control_state_db(path: &Path, rows: &[(&str, &str, i64, &Path)]) {
    let db = Connection::open(path).unwrap();
    db.execute(
        "CREATE TABLE threads (
            id TEXT PRIMARY KEY, model_provider TEXT, archived INTEGER, has_user_event INTEGER,
            cwd TEXT, title TEXT, rollout_path TEXT, source TEXT, created_at_ms INTEGER,
            updated_at_ms INTEGER, thread_source TEXT, git_branch TEXT
        )",
        [],
    )
    .unwrap();
    for (id, provider, archived, rollout_path) in rows {
        db.execute(
            "INSERT INTO threads VALUES (?1, ?2, ?3, 1, 'C:/workspace', ?1, ?4, 'vscode', 100000, 200000, NULL, NULL)",
            (
                id,
                provider,
                archived,
                rollout_path.to_string_lossy().to_string(),
            ),
        )
        .unwrap();
    }
}

fn create_local_thread_catalog_db(path: &Path, rows: &[(&str, &str)]) {
    let db = Connection::open(path).unwrap();
    db.execute(
        "CREATE TABLE local_thread_catalog (
            host_id TEXT NOT NULL,
            thread_id TEXT NOT NULL,
            display_title TEXT NOT NULL,
            source_created_at REAL NOT NULL,
            source_updated_at REAL NOT NULL,
            cwd TEXT NOT NULL,
            source_kind TEXT NOT NULL,
            source_detail TEXT,
            model_provider TEXT NOT NULL,
            git_branch TEXT,
            observation_sequence INTEGER NOT NULL,
            missing_candidate INTEGER NOT NULL DEFAULT 0,
            thread_source TEXT,
            PRIMARY KEY (host_id, thread_id)
        )",
        [],
    )
    .unwrap();
    db.execute(
        "CREATE TABLE local_thread_catalog_hosts (host_id TEXT PRIMARY KEY, host_kind TEXT NOT NULL)",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO local_thread_catalog_hosts VALUES ('local', 'local')",
        [],
    )
    .unwrap();
    db.execute(
        "CREATE TABLE local_thread_catalog_metadata (id INTEGER PRIMARY KEY, catalog_revision INTEGER NOT NULL DEFAULT 0)",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO local_thread_catalog_metadata VALUES (1, 0)",
        [],
    )
    .unwrap();
    db.execute(
        "CREATE TABLE local_thread_catalog_sync_state (
            host_id TEXT PRIMARY KEY,
            watermark_updated_at REAL,
            initial_build_complete INTEGER NOT NULL DEFAULT 0,
            observation_sequence INTEGER NOT NULL DEFAULT 0,
            last_full_reconciled_at INTEGER
        )",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO local_thread_catalog_sync_state VALUES ('local', 100, 1, 0, 100)",
        [],
    )
    .unwrap();
    for (index, (thread_id, provider)) in rows.iter().enumerate() {
        db.execute(
            "INSERT INTO local_thread_catalog (
                host_id, thread_id, display_title, source_created_at, source_updated_at, cwd,
                source_kind, source_detail, model_provider, git_branch, observation_sequence,
                missing_candidate, thread_source
            ) VALUES ('local', ?1, ?1, 100, 100, 'C:/workspace', 'cli', '', ?2, NULL, ?3, 0, 'user')",
            (thread_id, provider, index as i64 + 1),
        )
        .unwrap();
    }
}

#[test]
fn remote_control_recovery_candidate_requires_a_recent_unarchived_openai_thread() {
    let tmp = tempdir().unwrap();
    let home = tmp.path().join(".codex");
    fs::create_dir_all(&home).unwrap();
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let db = Connection::open(home.join("state_5.sqlite")).unwrap();
    db.execute(
        "CREATE TABLE threads (
            id TEXT PRIMARY KEY,
            model_provider TEXT,
            archived INTEGER,
            created_at_ms INTEGER
        )",
        [],
    )
    .unwrap();
    for (id, provider, archived, created_at_ms) in [
        ("recent", "openai", 0, now_ms),
        ("stale", "openai", 0, now_ms - 16 * 60 * 1000),
        ("archived", "openai", 1, now_ms),
        ("custom", "custom", 0, now_ms),
    ] {
        db.execute(
            "INSERT INTO threads VALUES (?1, ?2, ?3, ?4)",
            (id, provider, archived, created_at_ms),
        )
        .unwrap();
    }
    drop(db);

    assert!(remote_control_session_recovery_candidate_exists(Some(&home), "recent").unwrap());
    assert!(!remote_control_session_recovery_candidate_exists(Some(&home), "stale").unwrap());
    assert!(!remote_control_session_recovery_candidate_exists(Some(&home), "archived").unwrap());
    assert!(!remote_control_session_recovery_candidate_exists(Some(&home), "custom").unwrap());
}

#[test]
fn provider_sync_targets_default_to_codex_home_env() {
    let _lock = CODEX_HOME_ENV_LOCK.lock().unwrap();
    let tmp = tempdir().unwrap();
    let home = tmp.path().join("custom-codex-home");
    fs::create_dir_all(&home).unwrap();
    fs::write(home.join("config.toml"), "model_provider = \"custom\"\n").unwrap();
    let _guard = CodexHomeEnvGuard::set(&home);

    let targets = load_provider_sync_targets(None);

    assert_eq!(targets.current_provider, "custom");
    assert!(targets.targets.iter().any(|target| target.id == "custom"));
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
fn provider_sync_ignores_spawned_subagent_threads() {
    let tmp = tempdir().unwrap();
    let home = tmp.path().join(".codex");
    fs::create_dir(&home).unwrap();
    fs::write(home.join("config.toml"), "model_provider = \"apigather\"\n").unwrap();
    let parent_rollout = home.join("sessions/2026/rollout-parent.jsonl");
    let child_rollout = home.join("sessions/2026/rollout-child.jsonl");
    write_rollout(&parent_rollout, "openai", "parent", "C:/workspace");
    write_rollout(&child_rollout, "openai", "child", "C:/child-new");
    let state = home.join("state_5.sqlite");
    let db = Connection::open(&state).unwrap();
    db.execute(
        "CREATE TABLE threads (id TEXT PRIMARY KEY, model_provider TEXT, archived INTEGER, has_user_event INTEGER, cwd TEXT)",
        [],
    )
    .unwrap();
    db.execute(
        "CREATE TABLE thread_spawn_edges (parent_thread_id TEXT, child_thread_id TEXT)",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO threads VALUES ('parent', 'openai', 0, 1, 'C:/workspace')",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO threads VALUES ('child', 'openai', 0, 0, 'C:/child-old')",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO thread_spawn_edges VALUES ('parent', 'child')",
        [],
    )
    .unwrap();
    drop(db);

    let result = run_provider_sync(Some(&home));

    assert_eq!(result.status, ProviderSyncStatus::Synced);
    assert_eq!(result.changed_session_files, 1);
    let child_first: serde_json::Value = serde_json::from_str(
        fs::read_to_string(&child_rollout)
            .unwrap()
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(child_first["payload"]["model_provider"], "openai");
    let db = Connection::open(state).unwrap();
    let child: (String, i64, String) = db
        .query_row(
            "SELECT model_provider, has_user_event, cwd FROM threads WHERE id = 'child'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        child,
        ("openai".to_string(), 0, "C:/child-old".to_string())
    );
}

#[test]
fn provider_sync_preserves_marked_subagents_and_explicit_user_priority() {
    let tmp = tempdir().unwrap();
    let home = tmp.path().join(".codex");
    fs::create_dir(&home).unwrap();
    fs::write(home.join("config.toml"), "model_provider = \"apigather\"\n").unwrap();

    let structured_rollout = home.join("sessions/2026/rollout-structured-child.jsonl");
    let rollout_child = home.join("sessions/2026/rollout-source-child.jsonl");
    let marked_rollout = home.join("sessions/2026/rollout-marked-child.jsonl");
    let explicit_user_rollout = home.join("sessions/2026/rollout-explicit-user.jsonl");
    write_rollout(
        &structured_rollout,
        "openai",
        "structured-child",
        "C:/structured-new",
    );
    write_subagent_rollout(
        &rollout_child,
        "openai",
        "rollout-child",
        "C:/rollout-new",
    );
    write_rollout(
        &marked_rollout,
        "openai",
        "marked-child",
        "C:/marked-new",
    );
    write_subagent_rollout(
        &explicit_user_rollout,
        "openai",
        "explicit-user",
        "C:/user-new",
    );

    let state = home.join("state_5.sqlite");
    let db = Connection::open(&state).unwrap();
    db.execute(
        "CREATE TABLE threads (
            id TEXT PRIMARY KEY, model_provider TEXT, archived INTEGER, has_user_event INTEGER,
            cwd TEXT, source TEXT, thread_source TEXT
        )",
        [],
    )
    .unwrap();
    let structured_source = json!({"subagent": {"thread_spawn": {"depth": 1}}}).to_string();
    for (id, cwd, source, thread_source) in [
        (
            "structured-child",
            "C:/structured-old",
            structured_source.as_str(),
            None,
        ),
        ("rollout-child", "C:/rollout-old", "cli", None),
        ("marked-child", "C:/marked-old", "cli", Some("subagent")),
        (
            "explicit-user",
            "C:/user-old",
            structured_source.as_str(),
            Some("user"),
        ),
    ] {
        db.execute(
            "INSERT INTO threads VALUES (?1, 'openai', 0, 0, ?2, ?3, ?4)",
            rusqlite::params![id, cwd, source, thread_source],
        )
        .unwrap();
    }
    db.execute(
        "CREATE TABLE thread_spawn_edges (parent_thread_id TEXT, child_thread_id TEXT)",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO thread_spawn_edges VALUES ('parent', 'explicit-user')",
        [],
    )
    .unwrap();
    drop(db);

    let result = run_provider_sync(Some(&home));

    assert_eq!(result.status, ProviderSyncStatus::Synced);
    assert_eq!(result.changed_session_files, 1);
    for (path, provider) in [
        (&structured_rollout, "openai"),
        (&rollout_child, "openai"),
        (&marked_rollout, "openai"),
        (&explicit_user_rollout, "apigather"),
    ] {
        let first: serde_json::Value = serde_json::from_str(
            fs::read_to_string(path).unwrap().lines().next().unwrap(),
        )
        .unwrap();
        assert_eq!(first["payload"]["model_provider"], provider);
    }

    let db = Connection::open(state).unwrap();
    for (id, expected) in [
        (
            "structured-child",
            ("openai", 0_i64, "C:/structured-old"),
        ),
        ("rollout-child", ("openai", 0_i64, "C:/rollout-old")),
        ("marked-child", ("openai", 0_i64, "C:/marked-old")),
        ("explicit-user", ("apigather", 1_i64, "C:/user-new")),
    ] {
        let actual: (String, i64, String) = db
            .query_row(
                "SELECT model_provider, has_user_event, cwd FROM threads WHERE id = ?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(actual, (expected.0.to_string(), expected.1, expected.2.to_string()));
    }
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
fn provider_sync_updates_and_discovers_local_thread_catalog() {
    let tmp = tempdir().unwrap();
    let home = tmp.path().join(".codex");
    let sqlite_dir = home.join("sqlite");
    fs::create_dir_all(&sqlite_dir).unwrap();
    fs::write(home.join("config.toml"), "model_provider = \"apigather\"\n").unwrap();
    let db_path = sqlite_dir.join("codex-dev.db");
    create_local_thread_catalog_db(&db_path, &[("thread-1", "openai"), ("thread-2", "custom")]);

    let targets = load_provider_sync_targets(Some(&home));
    let ids = targets
        .targets
        .iter()
        .map(|target| target.id.as_str())
        .collect::<Vec<_>>();
    assert!(ids.contains(&"openai"));
    assert!(ids.contains(&"custom"));

    let result = run_provider_sync(Some(&home));

    assert_eq!(result.status, ProviderSyncStatus::Synced);
    assert_eq!(result.sqlite_rows_updated, 2);
    assert_eq!(result.sqlite_provider_rows_updated, 2);
    let db = Connection::open(&db_path).unwrap();
    let remaining = db
        .query_row(
            "SELECT COUNT(*) FROM local_thread_catalog WHERE model_provider <> 'apigather'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap();
    assert_eq!(remaining, 0);
    let backup_dir = result.backup_dir.unwrap();
    assert!(backup_dir.join("db/sqlite/codex-dev.db").exists());
}

#[test]
fn provider_sync_repairs_missing_local_thread_catalog_rows_from_threads() {
    let tmp = tempdir().unwrap();
    let home = tmp.path().join(".codex");
    let sqlite_dir = home.join("sqlite");
    fs::create_dir_all(&sqlite_dir).unwrap();
    fs::write(home.join("config.toml"), "model_provider = \"apigather\"\n").unwrap();
    let state_db = home.join("state_5.sqlite");
    let db = Connection::open(&state_db).unwrap();
    db.execute(
        "CREATE TABLE threads (
            id TEXT PRIMARY KEY,
            model_provider TEXT,
            archived INTEGER,
            has_user_event INTEGER,
            cwd TEXT,
            title TEXT,
            rollout_path TEXT,
            source TEXT,
            created_at_ms INTEGER,
            updated_at_ms INTEGER,
            thread_source TEXT,
            git_branch TEXT
        )",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO threads VALUES (
            'thread-1', 'old-provider', 0, 1, 'C:/workspace', 'Thread One',
            'C:/rollout.jsonl', 'cli', 100000, 200000, 'user', 'main'
        )",
        [],
    )
    .unwrap();
    drop(db);
    let catalog_db = sqlite_dir.join("codex-dev.db");
    create_local_thread_catalog_db(&catalog_db, &[]);

    let result = run_provider_sync(Some(&home));

    assert_eq!(result.status, ProviderSyncStatus::Synced);
    assert_eq!(result.sqlite_catalog_rows_inserted, 1);
    assert_eq!(result.sqlite_rows_updated, 2);
    let db = Connection::open(&catalog_db).unwrap();
    let row = db
        .query_row(
            "SELECT display_title, source_created_at, source_updated_at, cwd, source_kind, source_detail, model_provider, git_branch, thread_source FROM local_thread_catalog WHERE thread_id = 'thread-1'",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, f64>(1)?,
                    row.get::<_, f64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(row.0, "Thread One");
    assert_eq!(row.1, 100.0);
    assert_eq!(row.2, 200.0);
    assert_eq!(row.3, "C:/workspace");
    assert_eq!(row.4, "cli");
    assert_eq!(row.5, "C:/rollout.jsonl");
    assert_eq!(row.6, "apigather");
    assert_eq!(row.7, "main");
    assert_eq!(row.8, "user");
    let sync_state = db
        .query_row(
            "SELECT initial_build_complete, watermark_updated_at >= 200, observation_sequence FROM local_thread_catalog_sync_state WHERE host_id = 'local'",
            [],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?)),
        )
        .unwrap();
    assert_eq!(sync_state.0, 1);
    assert_eq!(sync_state.1, 1);
    assert_eq!(sync_state.2, 1);
}

#[test]
fn provider_sync_catalogs_user_threads_but_skips_subagents() {
    let tmp = tempdir().unwrap();
    let home = tmp.path().join(".codex");
    let sqlite_dir = home.join("sqlite");
    fs::create_dir_all(&sqlite_dir).unwrap();
    fs::write(home.join("config.toml"), "model_provider = \"apigather\"\n").unwrap();

    let state_db = home.join("state_5.sqlite");
    let db = Connection::open(&state_db).unwrap();
    db.execute(
        "CREATE TABLE threads (
            id TEXT PRIMARY KEY, model_provider TEXT, archived INTEGER, has_user_event INTEGER,
            cwd TEXT, title TEXT, rollout_path TEXT, source TEXT, created_at_ms INTEGER,
            updated_at_ms INTEGER, thread_source TEXT, git_branch TEXT
        )",
        [],
    )
    .unwrap();
    for (id, source, thread_source, updated_at) in [
        ("user-one", "vscode", "user", 200000_i64),
        (
            "explicit-user",
            r#"{"sub_agent":{"other":"review"}}"#,
            "user",
            205000_i64,
        ),
        ("marked-child", "vscode", "subagent", 210000_i64),
        (
            "memory-child",
            "internal_memory_consolidation",
            "memory_consolidation",
            215000_i64,
        ),
    ] {
        db.execute(
            "INSERT INTO threads VALUES (
                ?1, 'apigather', 0, 1, 'C:/workspace', 'Same title',
                ?2, ?3, 100000, ?4, ?5, 'main'
            )",
            rusqlite::params![id, format!("C:/{id}.jsonl"), source, updated_at, thread_source],
        )
        .unwrap();
    }
    db.execute(
        "INSERT INTO threads VALUES (
            'null-source-child', 'apigather', 0, 1, 'C:/workspace', 'Same title',
            'C:/null-source-child.jsonl', '{\"sub_agent\":{\"other\":\"guardian\"}}',
            100000, 217000, NULL, 'main'
        )",
        [],
    )
    .unwrap();
    drop(db);

    let legacy_state_db = sqlite_dir.join("state_5.sqlite");
    let db = Connection::open(&legacy_state_db).unwrap();
    db.execute(
        "CREATE TABLE threads (
            id TEXT PRIMARY KEY, model_provider TEXT, archived INTEGER, has_user_event INTEGER,
            cwd TEXT, title TEXT, rollout_path TEXT, source TEXT, created_at_ms INTEGER,
            updated_at_ms INTEGER, git_branch TEXT
        )",
        [],
    )
    .unwrap();
    for (id, source, updated_at) in [
        ("user-two", "custom-subagent-bridge", 220000_i64),
        ("malformed-user", r#"{"sub_agent":"#, 221000_i64),
        ("nested-user", r#"{"origin":"subagent"}"#, 222000_i64),
        (
            "source-child",
            r#"{"subagent":{"other":"guardian"}}"#,
            230000_i64,
        ),
        (
            "serde-source-child",
            r#"{"sub_agent":{"other":"review"}}"#,
            235000_i64,
        ),
        ("internal-child", "internal_memory_consolidation", 237000_i64),
        ("edge-child", "cli", 240000_i64),
    ] {
        db.execute(
            "INSERT INTO threads VALUES (
                ?1, 'apigather', 0, 1, 'C:/workspace', 'Same title',
                ?2, ?3, 100000, ?4, 'main'
            )",
            rusqlite::params![id, format!("C:/{id}.jsonl"), source, updated_at],
        )
        .unwrap();
    }
    db.execute(
        "CREATE TABLE thread_spawn_edges (
            parent_thread_id TEXT NOT NULL, child_thread_id TEXT NOT NULL, status TEXT
        )",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO thread_spawn_edges VALUES
            ('user-one', 'edge-child', 'open'),
            ('user-one', 'explicit-user', 'open')",
        [],
    )
    .unwrap();
    drop(db);

    let catalog_db = sqlite_dir.join("codex-dev.db");
    create_local_thread_catalog_db(&catalog_db, &[]);

    let result = run_provider_sync(Some(&home));

    assert_eq!(result.status, ProviderSyncStatus::Synced);
    assert_eq!(result.sqlite_catalog_rows_inserted, 5);
    assert_eq!(result.sqlite_catalog_rows_removed, 0);
    assert_eq!(result.sqlite_rows_updated, 5);
    let db = Connection::open(&catalog_db).unwrap();
    let mut stmt = db
        .prepare("SELECT thread_id FROM local_thread_catalog WHERE host_id = 'local' ORDER BY thread_id")
        .unwrap();
    let ids = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        ids,
        vec![
            "explicit-user",
            "malformed-user",
            "nested-user",
            "user-one",
            "user-two",
        ]
    );
    let duplicate_titles = db
        .query_row(
            "SELECT COUNT(*) FROM local_thread_catalog WHERE display_title = 'Same title'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap();
    assert_eq!(duplicate_titles, 5);
}

#[test]
fn provider_sync_prunes_existing_local_subagent_catalog_rows() {
    let tmp = tempdir().unwrap();
    let home = tmp.path().join(".codex");
    let sqlite_dir = home.join("sqlite");
    fs::create_dir_all(&sqlite_dir).unwrap();
    fs::write(home.join("config.toml"), "model_provider = \"apigather\"\n").unwrap();

    let state_db = home.join("state_5.sqlite");
    let db = Connection::open(&state_db).unwrap();
    db.execute(
        "CREATE TABLE threads (
            id TEXT PRIMARY KEY, model_provider TEXT, archived INTEGER, has_user_event INTEGER,
            cwd TEXT, title TEXT, rollout_path TEXT, source TEXT, created_at_ms INTEGER,
            updated_at_ms INTEGER, thread_source TEXT, git_branch TEXT
        )",
        [],
    )
    .unwrap();
    for (id, thread_source) in [("root", "user"), ("child", "subagent")] {
        db.execute(
            "INSERT INTO threads VALUES (
                ?1, 'apigather', 0, 1, 'C:/workspace', ?1,
                ?2, 'vscode', 100000, 200000, ?3, 'main'
            )",
            rusqlite::params![id, format!("C:/{id}.jsonl"), thread_source],
        )
        .unwrap();
    }
    drop(db);

    let catalog_db = sqlite_dir.join("codex-dev.db");
    create_local_thread_catalog_db(
        &catalog_db,
        &[
            ("root", "apigather"),
            ("child", "apigather"),
            ("orphan", "apigather"),
            ("stale-child", "apigather"),
        ],
    );
    let secondary_catalog_db = sqlite_dir.join("state_5.sqlite");
    create_local_thread_catalog_db(
        &secondary_catalog_db,
        &[("root", "apigather"), ("stale-child", "apigather")],
    );
    let db = Connection::open(&catalog_db).unwrap();
    db.execute(
        "UPDATE local_thread_catalog SET source_kind = 'subagent_review', thread_source = 'subagent' WHERE thread_id = 'stale-child'",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO local_thread_catalog_hosts VALUES ('remote', 'ssh')",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO local_thread_catalog (
            host_id, thread_id, display_title, source_created_at, source_updated_at, cwd,
            source_kind, source_detail, model_provider, git_branch, observation_sequence,
            missing_candidate, thread_source
        ) VALUES ('remote', 'child', 'Remote child', 100, 100, '/remote', 'cli', '',
            'apigather', NULL, 1, 0, 'subagent')",
        [],
    )
    .unwrap();
    drop(db);

    let result = run_provider_sync(Some(&home));

    assert_eq!(result.status, ProviderSyncStatus::Synced);
    assert_eq!(result.sqlite_catalog_rows_inserted, 0);
    assert_eq!(result.sqlite_catalog_rows_removed, 2);
    assert_eq!(result.sqlite_rows_updated, 2);
    let backup_dir = result.backup_dir.unwrap();
    assert!(backup_dir.join("db/sqlite/codex-dev.db").exists());

    let db = Connection::open(&catalog_db).unwrap();
    let mut stmt = db
        .prepare(
            "SELECT host_id, thread_id FROM local_thread_catalog ORDER BY host_id, thread_id",
        )
        .unwrap();
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        rows,
        vec![
            ("local".to_string(), "orphan".to_string()),
            ("local".to_string(), "root".to_string()),
            ("remote".to_string(), "child".to_string()),
        ]
    );
    let revision = db
        .query_row(
            "SELECT catalog_revision FROM local_thread_catalog_metadata WHERE id = 1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap();
    assert_eq!(revision, 2);
    let sync_state = db
        .query_row(
            "SELECT watermark_updated_at, initial_build_complete, observation_sequence FROM local_thread_catalog_sync_state WHERE host_id = 'local'",
            [],
            |row| {
                Ok((
                    row.get::<_, f64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(sync_state, (100.0, 1, 4));
    drop(stmt);
    drop(db);
    let secondary = Connection::open(&secondary_catalog_db).unwrap();
    let secondary_stale_row = secondary
        .query_row(
            "SELECT thread_source FROM local_thread_catalog WHERE thread_id = 'stale-child'",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap();
    assert_eq!(secondary_stale_row, "user");
    let secondary_rows = secondary
        .prepare("SELECT thread_id FROM local_thread_catalog ORDER BY thread_id")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(secondary_rows, vec!["root", "stale-child"]);
    let secondary_revision = secondary
        .query_row(
            "SELECT catalog_revision FROM local_thread_catalog_metadata WHERE id = 1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap();
    assert_eq!(secondary_revision, 0);
    let secondary_sync_state = secondary
        .query_row(
            "SELECT watermark_updated_at, initial_build_complete, observation_sequence FROM local_thread_catalog_sync_state WHERE host_id = 'local'",
            [],
            |row| {
                Ok((
                    row.get::<_, f64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(secondary_sync_state, (100.0, 1, 0));
    drop(secondary);

    let second = run_provider_sync(Some(&home));
    assert_eq!(second.status, ProviderSyncStatus::Synced);
    assert_eq!(second.sqlite_catalog_rows_inserted, 0);
    assert_eq!(second.sqlite_catalog_rows_removed, 0);
    assert_eq!(second.sqlite_rows_updated, 0);
    assert!(second.backup_dir.is_none());
}

#[test]
fn remote_control_catalog_recovery_for_thread_does_not_touch_other_candidates() {
    let tmp = tempdir().unwrap();
    let home = tmp.path().join(".codex");
    let sqlite_dir = home.join("sqlite");
    fs::create_dir_all(&sqlite_dir).unwrap();
    fs::write(home.join("config.toml"), "model_provider = \"custom\"\n").unwrap();

    let state_db = home.join("state_5.sqlite");
    let db = Connection::open(&state_db).unwrap();
    db.execute(
        "CREATE TABLE threads (
            id TEXT PRIMARY KEY, model_provider TEXT, archived INTEGER, has_user_event INTEGER,
            cwd TEXT, title TEXT, rollout_path TEXT, source TEXT, created_at_ms INTEGER,
            updated_at_ms INTEGER, thread_source TEXT, git_branch TEXT
        )",
        [],
    )
    .unwrap();
    for id in ["mobile-one", "mobile-two"] {
        let rollout = home.join(format!("sessions/rollout-{id}.jsonl"));
        write_rollout(&rollout, "openai", id, "C:/workspace");
        db.execute(
            "INSERT INTO threads VALUES (?1, 'openai', 0, 1, 'C:/workspace', ?1, ?2, 'vscode', 100000, 200000, NULL, NULL)",
            (id, rollout.to_string_lossy().to_string()),
        )
        .unwrap();
    }
    drop(db);
    let catalog_db = sqlite_dir.join("codex-dev.db");
    create_local_thread_catalog_db(&catalog_db, &[]);

    let result = run_remote_control_session_catalog_recovery_for_thread_with_target(
        Some(&home),
        "mobile-one",
        "custom",
    );

    assert_eq!(result.status, ProviderSyncStatus::Synced);
    assert_eq!(result.changed_session_files, 0);
    assert_eq!(result.sqlite_catalog_rows_inserted, 1);
    let db = Connection::open(&state_db).unwrap();
    let providers = ["mobile-one", "mobile-two"]
        .into_iter()
        .map(|id| {
            db.query_row(
                "SELECT model_provider FROM threads WHERE id = ?1",
                [id],
                |row| row.get::<_, String>(0),
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(providers, vec!["openai", "openai"]);
    let catalog = Connection::open(&catalog_db).unwrap();
    assert_eq!(
        catalog
            .query_row(
                "SELECT COUNT(*) FROM local_thread_catalog WHERE thread_id = 'mobile-one' AND model_provider = 'custom'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
    assert_eq!(
        catalog
            .query_row(
                "SELECT COUNT(*) FROM local_thread_catalog WHERE thread_id = 'mobile-two'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
    for id in ["mobile-one", "mobile-two"] {
        let rollout = home.join(format!("sessions/rollout-{id}.jsonl"));
        let first: serde_json::Value =
            serde_json::from_str(fs::read_to_string(rollout).unwrap().lines().next().unwrap())
                .unwrap();
        assert_eq!(first["payload"]["model_provider"], "openai");
    }
}

#[test]
fn remote_control_catalog_recovery_does_not_insert_subagent() {
    let tmp = tempdir().unwrap();
    let home = tmp.path().join(".codex");
    let sqlite_dir = home.join("sqlite");
    fs::create_dir_all(&sqlite_dir).unwrap();
    fs::write(home.join("config.toml"), "model_provider = \"custom\"\n").unwrap();

    let state_db = home.join("state_5.sqlite");
    let db = Connection::open(&state_db).unwrap();
    db.execute(
        "CREATE TABLE threads (
            id TEXT PRIMARY KEY, model_provider TEXT, archived INTEGER, has_user_event INTEGER,
            cwd TEXT, title TEXT, rollout_path TEXT, source TEXT, created_at_ms INTEGER,
            updated_at_ms INTEGER, thread_source TEXT, git_branch TEXT
        )",
        [],
    )
    .unwrap();
    for id in ["requested-child", "existing-child"] {
        db.execute(
            "INSERT INTO threads VALUES (
                ?1, 'openai', 0, 1, 'C:/workspace', 'Parent title',
                ?2, 'vscode', 100000, 200000, 'subagent', NULL
            )",
            rusqlite::params![id, format!("C:/{id}.jsonl")],
        )
        .unwrap();
    }
    drop(db);

    let catalog_db = sqlite_dir.join("codex-dev.db");
    create_local_thread_catalog_db(&catalog_db, &[("existing-child", "openai")]);
    Connection::open(&catalog_db)
        .unwrap()
        .execute(
            "UPDATE local_thread_catalog SET source_kind = 'subagent_review', thread_source = 'subagent' WHERE thread_id = 'existing-child'",
            [],
        )
        .unwrap();

    let result = run_remote_control_session_catalog_recovery_for_thread_with_target(
        Some(&home),
        "requested-child",
        "custom",
    );

    assert_eq!(result.status, ProviderSyncStatus::Synced);
    assert_eq!(result.sqlite_catalog_rows_inserted, 0);
    assert_eq!(result.sqlite_catalog_rows_removed, 0);
    assert_eq!(result.sqlite_rows_updated, 0);
    let catalog = Connection::open(&catalog_db).unwrap();
    let ids = catalog
        .prepare("SELECT thread_id FROM local_thread_catalog ORDER BY thread_id")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(ids, vec!["existing-child"]);
    drop(catalog);

    let existing = run_remote_control_session_catalog_recovery_for_thread_with_target(
        Some(&home),
        "existing-child",
        "custom",
    );
    assert_eq!(existing.status, ProviderSyncStatus::Synced);
    assert_eq!(existing.sqlite_provider_rows_updated, 1);
    assert_eq!(existing.sqlite_catalog_rows_inserted, 0);
    assert_eq!(existing.sqlite_catalog_rows_removed, 0);
    assert_eq!(existing.sqlite_rows_updated, 1);
    let catalog = Connection::open(&catalog_db).unwrap();
    let existing_provider = catalog
        .query_row(
            "SELECT model_provider FROM local_thread_catalog WHERE thread_id = 'existing-child'",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap();
    assert_eq!(existing_provider, "custom");
    assert_eq!(
        catalog
            .query_row(
                "SELECT COUNT(*) FROM local_thread_catalog WHERE thread_id = 'requested-child'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
}

#[test]
fn remote_control_catalog_recovery_for_thread_only_repairs_the_local_catalog_host() {
    let tmp = tempdir().unwrap();
    let home = tmp.path().join(".codex");
    let sqlite_dir = home.join("sqlite");
    fs::create_dir_all(&sqlite_dir).unwrap();
    fs::write(home.join("config.toml"), "model_provider = \"custom\"\n").unwrap();
    let rollout = home.join("sessions/rollout-mobile.jsonl");
    write_rollout(&rollout, "openai", "mobile", "C:/workspace");
    create_state_db_with_providers(&home.join("state_5.sqlite"), &[("mobile", "openai", 0)]);

    let catalog_db = sqlite_dir.join("codex-dev.db");
    create_local_thread_catalog_db(&catalog_db, &[]);
    let before_sync_state = Connection::open(&catalog_db)
        .unwrap()
        .query_row(
            "SELECT watermark_updated_at, initial_build_complete, observation_sequence, last_full_reconciled_at FROM local_thread_catalog_sync_state WHERE host_id = 'local'",
            [],
            |row| {
                Ok((
                    row.get::<_, f64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .unwrap();
    let db = Connection::open(&catalog_db).unwrap();
    db.execute(
        "INSERT INTO local_thread_catalog_hosts VALUES ('aaa-remote', 'ssh')",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO local_thread_catalog (
            host_id, thread_id, display_title, source_created_at, source_updated_at, cwd,
            source_kind, source_detail, model_provider, git_branch, observation_sequence,
            missing_candidate, thread_source
        ) VALUES ('aaa-remote', 'mobile', 'Remote copy', 100, 100, '/remote', 'cli', '', 'openai', NULL, 1, 0, 'user')",
        [],
    )
    .unwrap();
    drop(db);

    let result = run_remote_control_session_catalog_recovery_for_thread_with_target(
        Some(&home),
        "mobile",
        "custom",
    );

    assert_eq!(result.status, ProviderSyncStatus::Synced);
    assert_eq!(result.changed_session_files, 0);
    assert_eq!(result.sqlite_catalog_rows_inserted, 1);
    let db = Connection::open(&catalog_db).unwrap();
    let rows = db
        .prepare(
            "SELECT host_id, model_provider FROM local_thread_catalog WHERE thread_id = 'mobile' ORDER BY host_id",
        )
        .unwrap()
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(
        rows,
        vec![
            ("aaa-remote".to_string(), "openai".to_string()),
            ("local".to_string(), "custom".to_string()),
        ]
    );
    let state = Connection::open(home.join("state_5.sqlite")).unwrap();
    let provider: String = state
        .query_row(
            "SELECT model_provider FROM threads WHERE id = 'mobile'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(provider, "openai");
    let after_sync_state = Connection::open(&catalog_db)
        .unwrap()
        .query_row(
            "SELECT watermark_updated_at, initial_build_complete, observation_sequence, last_full_reconciled_at FROM local_thread_catalog_sync_state WHERE host_id = 'local'",
            [],
            |row| {
                Ok((
                    row.get::<_, f64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(after_sync_state, before_sync_state);
}

#[test]
fn remote_control_finalization_uses_only_recorded_rollout_and_preserves_full_sync_state() {
    let tmp = tempdir().unwrap();
    let home = tmp.path().join(".codex");
    let sqlite_dir = home.join("sqlite");
    fs::create_dir_all(&sqlite_dir).unwrap();
    let target_rollout = home.join("sessions/rollout-mobile.jsonl");
    let other_rollout = home.join("sessions/rollout-other.jsonl");
    write_rollout(&target_rollout, "openai", "mobile", "C:/workspace");
    write_rollout(&other_rollout, "openai", "other", "C:/workspace");
    create_remote_control_state_db(
        &home.join("state_5.sqlite"),
        &[
            ("mobile", "openai", 0, &target_rollout),
            ("other", "openai", 0, &other_rollout),
        ],
    );
    let catalog_db = sqlite_dir.join("codex-dev.db");
    create_local_thread_catalog_db(&catalog_db, &[]);

    let before_sync_state = Connection::open(&catalog_db)
        .unwrap()
        .query_row(
            "SELECT watermark_updated_at, initial_build_complete, observation_sequence, last_full_reconciled_at FROM local_thread_catalog_sync_state WHERE host_id = 'local'",
            [],
            |row| {
                Ok((
                    row.get::<_, f64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .unwrap();

    let result = run_remote_control_session_finalization_for_thread_with_target(
        Some(&home),
        "mobile",
        "custom",
    );

    assert_eq!(result.status, ProviderSyncStatus::Synced);
    assert_eq!(result.changed_session_files, 1);
    assert_eq!(result.sqlite_catalog_rows_inserted, 1);
    let target_first: serde_json::Value = serde_json::from_str(
        fs::read_to_string(&target_rollout)
            .unwrap()
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    let other_first: serde_json::Value = serde_json::from_str(
        fs::read_to_string(&other_rollout)
            .unwrap()
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(target_first["payload"]["model_provider"], "custom");
    assert_eq!(other_first["payload"]["model_provider"], "openai");

    let state = Connection::open(home.join("state_5.sqlite")).unwrap();
    let providers = ["mobile", "other"]
        .into_iter()
        .map(|id| {
            state
                .query_row(
                    "SELECT model_provider FROM threads WHERE id = ?1",
                    [id],
                    |row| row.get::<_, String>(0),
                )
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(providers, vec!["custom", "openai"]);

    let catalog = Connection::open(&catalog_db).unwrap();
    assert_eq!(
        catalog
            .query_row(
                "SELECT COUNT(*) FROM local_thread_catalog WHERE thread_id = 'mobile' AND model_provider = 'custom'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
    assert_eq!(
        catalog
            .query_row(
                "SELECT COUNT(*) FROM local_thread_catalog WHERE thread_id = 'other'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
    let after_sync_state = catalog
        .query_row(
            "SELECT watermark_updated_at, initial_build_complete, observation_sequence, last_full_reconciled_at FROM local_thread_catalog_sync_state WHERE host_id = 'local'",
            [],
            |row| {
                Ok((
                    row.get::<_, f64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(after_sync_state, before_sync_state);
}

#[test]
fn remote_control_finalization_ignores_archived_and_other_provider_threads() {
    let tmp = tempdir().unwrap();
    let home = tmp.path().join(".codex");
    let sqlite_dir = home.join("sqlite");
    fs::create_dir_all(&sqlite_dir).unwrap();
    let archived_rollout = home.join("sessions/rollout-archived.jsonl");
    let other_rollout = home.join("sessions/rollout-other.jsonl");
    write_rollout(&archived_rollout, "openai", "archived", "C:/workspace");
    write_rollout(&other_rollout, "other", "other", "C:/workspace");
    create_remote_control_state_db(
        &home.join("state_5.sqlite"),
        &[
            ("archived", "openai", 1, &archived_rollout),
            ("other", "other", 0, &other_rollout),
        ],
    );
    let catalog_db = sqlite_dir.join("codex-dev.db");
    create_local_thread_catalog_db(&catalog_db, &[]);

    let archived = run_remote_control_session_finalization_for_thread_with_target(
        Some(&home),
        "archived",
        "custom",
    );
    let other = run_remote_control_session_finalization_for_thread_with_target(
        Some(&home),
        "other",
        "custom",
    );

    assert_eq!(archived.status, ProviderSyncStatus::Synced);
    assert_eq!(other.status, ProviderSyncStatus::Synced);
    assert!(archived.message.contains("archived"));
    assert!(other.message.contains("another provider"));
    for (id, provider) in [("archived", "openai"), ("other", "other")] {
        let first: serde_json::Value = serde_json::from_str(
            fs::read_to_string(home.join(format!("sessions/rollout-{id}.jsonl")))
                .unwrap()
                .lines()
                .next()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(first["payload"]["model_provider"], provider);
    }
    assert_eq!(
        Connection::open(&catalog_db)
            .unwrap()
            .query_row("SELECT COUNT(*) FROM local_thread_catalog", [], |row| row
                .get::<_, i64>(
                0
            ),)
            .unwrap(),
        0
    );
}

#[test]
fn remote_control_finalization_defers_when_rollout_changes_after_collection() {
    let tmp = tempdir().unwrap();
    let home = tmp.path().join(".codex");
    let sqlite_dir = home.join("sqlite");
    fs::create_dir_all(&sqlite_dir).unwrap();
    fs::write(home.join("config.toml"), "model_provider = \"custom\"\n").unwrap();
    let rollout = home.join("sessions/rollout-mobile.jsonl");
    write_rollout(&rollout, "openai", "mobile", "C:/workspace");
    let state_db = home.join("state_5.sqlite");
    create_remote_control_state_db(&state_db, &[("mobile", "openai", 0, &rollout)]);
    let db = Connection::open(&state_db).unwrap();
    db.execute("CREATE TABLE backup_padding (data BLOB)", [])
        .unwrap();
    db.execute("INSERT INTO backup_padding VALUES (zeroblob(33554432))", [])
        .unwrap();
    drop(db);
    let catalog_db = sqlite_dir.join("codex-dev.db");
    create_local_thread_catalog_db(&catalog_db, &[]);

    let backup_root = home.join("backups_state/provider-sync");
    let watched_rollout = rollout.clone();
    let writer = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let backup_started = backup_root.exists()
                && fs::read_dir(&backup_root)
                    .map(|mut entries| entries.next().is_some())
                    .unwrap_or(false);
            if backup_started {
                let mut file = fs::OpenOptions::new()
                    .append(true)
                    .open(&watched_rollout)
                    .unwrap();
                use std::io::Write as _;
                writeln!(
                    file,
                    "{}",
                    json!({"type": "event_msg", "payload": {"type": "task_started"}})
                )
                .unwrap();
                return;
            }
            assert!(Instant::now() < deadline, "backup did not start in time");
            std::thread::sleep(Duration::from_millis(1));
        }
    });

    let result = run_remote_control_session_finalization_for_thread_with_target(
        Some(&home),
        "mobile",
        "custom",
    );
    writer.join().unwrap();

    assert_eq!(result.status, ProviderSyncStatus::Skipped);
    assert_eq!(result.changed_session_files, 0);
    assert_eq!(result.skipped_locked_rollout_files.len(), 1);
    assert_eq!(
        fs::canonicalize(&result.skipped_locked_rollout_files[0]).unwrap(),
        fs::canonicalize(&rollout).unwrap()
    );
    let text = fs::read_to_string(&rollout).unwrap();
    assert!(text.contains("task_started"));
    let first: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    assert_eq!(first["payload"]["model_provider"], "openai");
    let state = Connection::open(&state_db).unwrap();
    let provider: String = state
        .query_row(
            "SELECT model_provider FROM threads WHERE id = 'mobile'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(provider, "openai");
    let catalog = Connection::open(&catalog_db).unwrap();
    let catalog_rows: i64 = catalog
        .query_row(
            "SELECT COUNT(*) FROM local_thread_catalog WHERE thread_id = 'mobile'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(catalog_rows, 0);
}

#[test]
fn remote_control_finalization_retries_after_catalog_only_partial_commit() {
    let tmp = tempdir().unwrap();
    let home = tmp.path().join(".codex");
    let sqlite_dir = home.join("sqlite");
    fs::create_dir_all(&sqlite_dir).unwrap();
    fs::write(home.join("config.toml"), "model_provider = \"custom\"\n").unwrap();
    let rollout = home.join("sessions/rollout-mobile.jsonl");
    write_rollout(&rollout, "openai", "mobile", "C:/workspace");
    let state_db = home.join("state_5.sqlite");
    create_remote_control_state_db(&state_db, &[("mobile", "openai", 0, &rollout)]);
    let db = Connection::open(&state_db).unwrap();
    db.execute(
        "CREATE TRIGGER fail_remote_recovery BEFORE UPDATE OF model_provider ON threads BEGIN SELECT RAISE(ABORT, 'boom'); END",
        [],
    )
    .unwrap();
    drop(db);
    let catalog_db = sqlite_dir.join("codex-dev.db");
    create_local_thread_catalog_db(&catalog_db, &[]);

    let first = run_remote_control_session_finalization_for_thread_with_target(
        Some(&home),
        "mobile",
        "custom",
    );

    assert_eq!(first.status, ProviderSyncStatus::Skipped);
    let first_line: serde_json::Value = serde_json::from_str(
        fs::read_to_string(&rollout)
            .unwrap()
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(first_line["payload"]["model_provider"], "custom");
    let catalog = Connection::open(&catalog_db).unwrap();
    let catalog_provider: String = catalog
        .query_row(
            "SELECT model_provider FROM local_thread_catalog WHERE host_id = 'local' AND thread_id = 'mobile'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(catalog_provider, "custom");
    let state = Connection::open(&state_db).unwrap();
    let state_provider: String = state
        .query_row(
            "SELECT model_provider FROM threads WHERE id = 'mobile'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(state_provider, "openai");
    state
        .execute("DROP TRIGGER fail_remote_recovery", [])
        .unwrap();
    drop(state);

    let second = run_remote_control_session_finalization_for_thread_with_target(
        Some(&home),
        "mobile",
        "custom",
    );

    assert_eq!(second.status, ProviderSyncStatus::Synced);
    assert_eq!(second.changed_session_files, 0);
    let state = Connection::open(&state_db).unwrap();
    let state_provider: String = state
        .query_row(
            "SELECT model_provider FROM threads WHERE id = 'mobile'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(state_provider, "custom");
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

#[cfg(any(windows, target_os = "linux", target_os = "macos"))]
#[test]
fn provider_sync_recovers_lock_owned_by_dead_process() {
    let tmp = tempdir().unwrap();
    let home = tmp.path().join(".codex");
    let lock_dir = home.join("tmp/provider-sync.lock");
    fs::create_dir_all(&lock_dir).unwrap();
    fs::write(home.join("config.toml"), "model_provider = \"apigather\"\n").unwrap();
    fs::write(
        lock_dir.join("owner.json"),
        json!({"pid": u32::MAX, "startedAt": 1234}).to_string(),
    )
    .unwrap();
    let log_path = tmp.path().join("codex-plus.log");
    codex_plus_core::diagnostic_log::set_diagnostic_log_path_for_tests(Some(log_path.clone()));

    let result = run_provider_sync(Some(&home));

    codex_plus_core::diagnostic_log::set_diagnostic_log_path_for_tests(None);
    assert_eq!(result.status, ProviderSyncStatus::Synced);
    assert!(!lock_dir.exists());
    assert!(
        fs::read_to_string(log_path)
            .unwrap()
            .contains("provider_sync.stale_lock_recovered")
    );
    assert!(fs::read_dir(home.join("tmp")).unwrap().next().is_none());
}

#[test]
fn provider_sync_preserves_lock_owned_by_live_process() {
    let tmp = tempdir().unwrap();
    let home = tmp.path().join(".codex");
    let lock_dir = home.join("tmp/provider-sync.lock");
    fs::create_dir_all(&lock_dir).unwrap();
    fs::write(home.join("config.toml"), "model_provider = \"apigather\"\n").unwrap();
    fs::write(
        lock_dir.join("owner.json"),
        json!({"pid": std::process::id(), "startedAt": 1234}).to_string(),
    )
    .unwrap();

    let result = run_provider_sync(Some(&home));

    assert_eq!(result.status, ProviderSyncStatus::Skipped);
    assert!(result.message.to_lowercase().contains("lock"));
    assert!(lock_dir.exists());
}

#[test]
fn provider_sync_preserves_lock_with_malformed_owner() {
    let tmp = tempdir().unwrap();
    let home = tmp.path().join(".codex");
    let lock_dir = home.join("tmp/provider-sync.lock");
    fs::create_dir_all(&lock_dir).unwrap();
    fs::write(home.join("config.toml"), "model_provider = \"apigather\"\n").unwrap();
    fs::write(lock_dir.join("owner.json"), "{not-json").unwrap();

    let result = run_provider_sync(Some(&home));

    assert_eq!(result.status, ProviderSyncStatus::Skipped);
    assert!(result.message.to_lowercase().contains("lock"));
    assert!(lock_dir.exists());
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
