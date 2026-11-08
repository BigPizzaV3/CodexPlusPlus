use codex_plus_core::settings::{BackendSettings, SettingsStore};
use codex_plus_manager_lib::commands::load_settings;

#[test]
fn load_settings_survives_failed_catalog_migration() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    std::fs::create_dir_all(home.join("model-catalogs")).unwrap();
    let config = "model_catalog_json = 'model-catalogs/broken.json'\n";
    std::fs::write(home.join("config.toml"), config).unwrap();
    std::fs::write(home.join("model-catalogs/broken.json"), "{}").unwrap();
    let log_path = temp.path().join("diagnostic.log");
    unsafe {
        std::env::set_var("CODEX_HOME", &home);
        std::env::set_var("CODEX_PLUS_STATE_DIR", temp.path().join("state"));
    }
    codex_plus_core::diagnostic_log::set_diagnostic_log_path_for_tests(Some(log_path.clone()));
    let settings = BackendSettings {
        relay_profiles_enabled: true,
        local_config_provider_imported: true,
        ..BackendSettings::default()
    };
    SettingsStore::default().save(&settings).unwrap();

    let result = load_settings();

    codex_plus_core::diagnostic_log::set_diagnostic_log_path_for_tests(None);
    assert_eq!(result.status, "ok");
    assert!(result.payload.settings.relay_profiles_enabled);
    assert_eq!(std::fs::read_to_string(home.join("config.toml")).unwrap(), config);
    assert!(
        std::fs::read_to_string(log_path)
            .unwrap()
            .contains("manager.model_catalog_migration_failed")
    );
}
