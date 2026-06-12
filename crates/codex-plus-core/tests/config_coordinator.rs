use codex_plus_core::ccs_import;
use codex_plus_core::config_coordinator;
use codex_plus_core::settings::{BackendSettings, ConfigOwnership};
use rusqlite::params;
use serde_json::json;

fn create_ccs_db(path: &std::path::Path) {
    let conn = rusqlite::Connection::open(path).unwrap();
    conn.execute(
        "CREATE TABLE providers (
            id TEXT NOT NULL,
            app_type TEXT NOT NULL,
            name TEXT NOT NULL,
            settings_config TEXT NOT NULL,
            created_at INTEGER,
            sort_index INTEGER,
            is_current BOOLEAN NOT NULL DEFAULT 0,
            PRIMARY KEY (id, app_type)
        )",
        [],
    )
    .unwrap();
}

fn insert_provider(path: &std::path::Path, id: &str, name: &str, config: serde_json::Value, current: bool) {
    let conn = rusqlite::Connection::open(path).unwrap();
    conn.execute(
        "INSERT INTO providers (id, app_type, name, settings_config, created_at, sort_index, is_current)
         VALUES (?1, 'codex', ?2, ?3, 1000, 0, ?4)",
        params![id, name, config.to_string(), current as i64],
    )
    .unwrap();
}

#[test]
fn effective_ownership_auto_prefers_ccswitch_when_linked() {
    let mut settings = BackendSettings::default();
    settings.ccs_link_enabled = true;
    settings.config_ownership = ConfigOwnership::Auto;
    assert_eq!(
        config_coordinator::effective_ownership(&settings),
        ConfigOwnership::CcSwitch
    );
}

#[test]
fn evaluate_live_write_blocks_when_ccswitch_owns_config() {
    let mut settings = BackendSettings::default();
    settings.ccs_link_enabled = true;
    settings.config_ownership = ConfigOwnership::CcSwitch;
    let decision = config_coordinator::evaluate_live_write(&settings, false);
    assert!(!decision.allowed);
    assert!(decision.message.contains("CC Switch"));
}

#[test]
fn detect_external_modification_after_foreign_write() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join(".codex");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("config.toml"), "model_provider = \"custom\"\n").unwrap();
    std::fs::write(home.join("auth.json"), "{}\n").unwrap();
    config_coordinator::record_write_marker("codexplusplus", &home).unwrap();

    std::fs::write(home.join("config.toml"), "model_provider = \"openai\"\n").unwrap();
    let conflict = config_coordinator::detect_external_modification(&home);
    assert!(conflict.is_some());
}

#[test]
fn current_codex_provider_from_db_reads_active_provider() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("cc-switch.db");
    create_ccs_db(&db);
    insert_provider(
        &db,
        "relay-a",
        "Relay A",
        json!({
            "auth": { "OPENAI_API_KEY": "sk-a" },
            "config": "model_provider = \"relay-a\"\n\n[model_providers.relay-a]\nbase_url = \"https://relay-a.example/v1\"\n"
        }),
        true,
    );
    insert_provider(
        &db,
        "relay-b",
        "Relay B",
        json!({
            "auth": { "OPENAI_API_KEY": "sk-b" },
            "config": "model_provider = \"relay-b\"\n"
        }),
        false,
    );

    let provider = ccs_import::current_codex_provider_from_db(&db).unwrap().unwrap();
    assert_eq!(provider.source_id, "relay-a");
    assert_eq!(provider.name, "Relay A");
    assert!(provider.config_contents.contains("relay-a"));
}