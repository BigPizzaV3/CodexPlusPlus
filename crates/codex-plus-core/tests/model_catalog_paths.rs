use std::path::{Path, PathBuf};

use codex_plus_core::relay_config::{
    apply_relay_profile_to_home_with_switch_rules, migrate_managed_model_catalog_pointer_in_home,
};
use codex_plus_core::settings::{
    AggregateRelayMember, AggregateRelayProfile, AggregateRelayStrategy, BackendSettings,
    RelayMode, RelayProfile, RelaySessionProvider, SettingsStore,
};

fn profile() -> RelayProfile {
    RelayProfile {
        id: "catalog-path".to_string(),
        relay_mode: RelayMode::PureApi,
        base_url: "https://relay.example/v1".to_string(),
        api_key: "test-catalog-key".to_string(),
        model: "path-test-model".to_string(),
        model_list: "path-test-model[200K]".to_string(),
        ..RelayProfile::default()
    }
}

fn pointer(home: &Path) -> PathBuf {
    let text = std::fs::read_to_string(home.join("config.toml")).unwrap();
    let config = text.parse::<toml_edit::DocumentMut>().unwrap();
    PathBuf::from(config["model_catalog_json"].as_str().unwrap())
}

fn assert_absolute_managed_pointer(home: &Path, profile_id: &str) {
    let actual = pointer(home);
    assert!(
        actual.is_absolute(),
        "catalog 指针必须是绝对路径：{actual:?}"
    );
    assert_eq!(
        actual,
        std::path::absolute(
            home.join("model-catalogs")
                .join(format!("{profile_id}.json"))
        )
        .unwrap()
    );
    assert!(actual.is_file());
}

#[test]
fn generated_model_catalog_pointer_is_absolute() {
    let temp = tempfile::tempdir().unwrap();
    apply_relay_profile_to_home_with_switch_rules(temp.path(), &profile(), "").unwrap();
    assert_absolute_managed_pointer(temp.path(), "catalog-path");
}

#[test]
fn wsl_catalog_paths_survive_apply_and_startup_migration() {
    let temp = tempfile::tempdir().unwrap();
    let config = "[desktop]\nrunCodexInWindowsSubsystemForLinux = true\n";
    std::fs::write(temp.path().join("config.toml"), config).unwrap();
    let profile = profile();
    apply_relay_profile_to_home_with_switch_rules(temp.path(), &profile, "").unwrap();
    assert_eq!(pointer(temp.path()), PathBuf::from("model-catalogs/catalog-path.json"));
    assert!(temp.path().join(pointer(temp.path())).is_file());
    let settings = BackendSettings { relay_profiles_enabled: true, ..Default::default() };
    assert!(!migrate_managed_model_catalog_pointer_in_home(temp.path(), &settings).unwrap());

    // 同一 home 从原生 Windows 切到 WSL 时，已生成的绝对指针也可窄迁移。
    let text = std::fs::read_to_string(temp.path().join("config.toml")).unwrap();
    let mut doc = text.parse::<toml_edit::DocumentMut>().unwrap();
    doc["model_catalog_json"] = toml_edit::value(temp.path().join(pointer(temp.path())).to_string_lossy().as_ref());
    std::fs::write(temp.path().join("config.toml"), doc.to_string()).unwrap();
    assert!(migrate_managed_model_catalog_pointer_in_home(temp.path(), &settings).unwrap());
    assert_eq!(pointer(temp.path()), PathBuf::from("model-catalogs/catalog-path.json"));
}

#[test]
fn live_desktop_mode_outranks_stale_provider_snapshot_in_both_directions() {
    for live_wsl in [true, false] {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("config.toml"), format!(
            "[desktop]\nrunCodexInWindowsSubsystemForLinux = {live_wsl}\n"
        )).unwrap();
        let mut profile = profile();
        profile.config_contents = format!(
            "[desktop]\nrunCodexInWindowsSubsystemForLinux = {}\n", !live_wsl
        );
        apply_relay_profile_to_home_with_switch_rules(temp.path(), &profile, "").unwrap();
        let text = std::fs::read_to_string(temp.path().join("config.toml")).unwrap();
        let doc: toml_edit::DocumentMut = text.parse().unwrap();
        assert_eq!(doc["desktop"]["runCodexInWindowsSubsystemForLinux"].as_bool(), Some(live_wsl));
        assert_eq!(pointer(temp.path()).is_absolute(), !live_wsl);
    }
}

#[test]
fn copied_external_catalog_pointers_are_absolute_without_changing_source() {
    for from_live in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let external = temp.path().join("user-catalog.json");
        let original = r#"{"models":[{"slug":"path-test-model","base_instructions":"You are a coding assistant.","supports_parallel_tool_calls":true,"use_responses_lite":true}]}"#;
        std::fs::write(&external, original).unwrap();
        let mut config = toml_edit::DocumentMut::new();
        config["model_catalog_json"] = toml_edit::value(external.to_string_lossy().as_ref());
        let mut profile = profile();
        // 不使用 per-model 覆盖，才能进入外部目录的兼容复制分支。
        profile.model_list = profile.model.clone();
        if from_live {
            std::fs::write(temp.path().join("config.toml"), config.to_string()).unwrap();
        } else {
            profile.config_contents = config.to_string();
        }

        apply_relay_profile_to_home_with_switch_rules(temp.path(), &profile, "").unwrap();

        assert_absolute_managed_pointer(temp.path(), &profile.id);
        assert_eq!(std::fs::read_to_string(&external).unwrap(), original);
    }
}

#[test]
fn aggregate_model_catalog_pointer_is_absolute() {
    let temp = tempfile::tempdir().unwrap();
    let member = profile();
    let aggregate = RelayProfile {
        id: "catalog-aggregate".to_string(),
        relay_mode: RelayMode::Aggregate,
        api_key: "test-aggregate-key".to_string(),
        ..RelayProfile::default()
    };
    let settings_path = temp.path().join("settings.json");
    let settings = BackendSettings {
        active_relay_id: aggregate.id.clone(),
        active_aggregate_relay_id: aggregate.id.clone(),
        relay_profiles: vec![member.clone(), aggregate.clone()],
        aggregate_relay_profiles: vec![AggregateRelayProfile {
            id: aggregate.id.clone(),
            name: "Catalog aggregate".to_string(),
            session_provider: RelaySessionProvider::Custom,
            model_aggregation_enabled: true,
            strategy: AggregateRelayStrategy::Failover,
            members: vec![AggregateRelayMember {
                relay_id: member.id.clone(),
                weight: 1,
            }],
            default_relay_id: member.id.clone(),
            routes: Vec::new(),
            sync_default_member_login: false,
        }],
        ..BackendSettings::default()
    };
    SettingsStore::new(settings_path.clone())
        .save(&settings)
        .unwrap();
    let previous = codex_plus_core::paths::set_settings_path_for_tests(Some(settings_path));
    let result = apply_relay_profile_to_home_with_switch_rules(temp.path(), &aggregate, "");
    codex_plus_core::paths::set_settings_path_for_tests(previous);
    result.unwrap();
    assert_absolute_managed_pointer(temp.path(), &aggregate.id);
}

#[test]
fn relative_home_with_spaces_still_generates_an_absolute_catalog_pointer() {
    let cwd = std::env::current_dir().unwrap();
    let temp = tempfile::Builder::new()
        .prefix("catalog home ")
        .tempdir_in(&cwd)
        .unwrap();
    let home = temp.path().strip_prefix(&cwd).unwrap();
    assert!(!home.is_absolute());
    apply_relay_profile_to_home_with_switch_rules(home, &profile(), "").unwrap();
    assert_absolute_managed_pointer(home, "catalog-path");
}

#[test]
fn disabling_per_model_overrides_removes_the_existing_absolute_managed_pointer() {
    let temp = tempfile::tempdir().unwrap();
    let mut profile = profile();
    apply_relay_profile_to_home_with_switch_rules(temp.path(), &profile, "").unwrap();
    profile.config_contents = std::fs::read_to_string(temp.path().join("config.toml")).unwrap();
    profile.model_list = profile.model.clone();

    apply_relay_profile_to_home_with_switch_rules(temp.path(), &profile, "").unwrap();

    let config = std::fs::read_to_string(temp.path().join("config.toml")).unwrap();
    assert!(!config.contains("model_catalog_json"));
}

fn write_pointer(home: &Path, value: &Path) -> String {
    let mut config = toml_edit::DocumentMut::new();
    config["model_catalog_json"] = toml_edit::value(value.to_string_lossy().as_ref());
    config["model"] = toml_edit::value("manually-selected-model");
    config["model_context_window"] = toml_edit::value(234567);
    let config = format!("# keep user comment\n{config}");
    std::fs::write(home.join("config.toml"), &config).unwrap();
    config
}

#[test]
fn migration_only_changes_the_existing_managed_catalog_pointer_and_is_idempotent() {
    let cwd = std::env::current_dir().unwrap();
    let temp = tempfile::Builder::new()
        .prefix("catalog migration ")
        .tempdir_in(&cwd)
        .unwrap();
    let home = temp.path().strip_prefix(&cwd).unwrap();
    std::fs::create_dir(home.join("model-catalogs")).unwrap();
    let catalog = home.join("model-catalogs").join("legacy.json");
    let contents = r#"{"models":[{"slug":"legacy"}]}"#;
    std::fs::write(&catalog, contents).unwrap();
    let original = write_pointer(home, Path::new("./model-catalogs/legacy.json"));
    let auth = r#"{"OPENAI_API_KEY":"test-unchanged"}"#;
    std::fs::write(home.join("auth.json"), auth).unwrap();
    let settings = BackendSettings::default();

    assert!(migrate_managed_model_catalog_pointer_in_home(home, &settings).unwrap());

    assert_eq!(pointer(home), std::path::absolute(&catalog).unwrap());
    let updated = std::fs::read_to_string(home.join("config.toml")).unwrap();
    let mut expected = original.parse::<toml_edit::DocumentMut>().unwrap();
    expected["model_catalog_json"] = toml_edit::value(pointer(home).to_string_lossy().as_ref());
    assert_eq!(updated, expected.to_string());
    assert_eq!(std::fs::read_to_string(&catalog).unwrap(), contents);
    assert_eq!(
        std::fs::read_to_string(home.join("auth.json")).unwrap(),
        auth
    );
    assert!(!migrate_managed_model_catalog_pointer_in_home(home, &settings).unwrap());
    assert_eq!(
        std::fs::read_to_string(home.join("config.toml")).unwrap(),
        updated
    );
}

#[test]
fn migration_preserves_disabled_external_missing_absolute_and_escaping_pointers() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    std::fs::create_dir_all(home.join("model-catalogs").join("nested")).unwrap();
    std::fs::create_dir_all(home.join("external/model-catalogs")).unwrap();
    for relative in [
        "model-catalogs/existing.json",
        "user-catalog.json",
        "external/model-catalogs/custom.json",
        "model-catalogs/nested/custom.json",
    ] {
        std::fs::write(home.join(relative), "{}").unwrap();
    }
    let settings = BackendSettings::default();
    let disabled = BackendSettings {
        relay_profiles_enabled: false,
        ..settings.clone()
    };
    let original = write_pointer(home, Path::new("model-catalogs/existing.json"));
    assert!(!migrate_managed_model_catalog_pointer_in_home(home, &disabled).unwrap());
    assert_eq!(
        std::fs::read_to_string(home.join("config.toml")).unwrap(),
        original
    );

    for value in [
        PathBuf::from("user-catalog.json"),
        PathBuf::from("external/model-catalogs/custom.json"),
        PathBuf::from("model-catalogs/nested/custom.json"),
        PathBuf::from("model-catalogs/../user-catalog.json"),
        PathBuf::from("model-catalogs/missing.json"),
        home.join("model-catalogs").join("existing.json"),
    ] {
        let original = write_pointer(home, &value);
        assert!(
            !migrate_managed_model_catalog_pointer_in_home(home, &settings).unwrap(),
            "{value:?}"
        );
        assert_eq!(
            std::fs::read_to_string(home.join("config.toml")).unwrap(),
            original
        );
    }
}

#[test]
fn migration_does_not_create_missing_config_or_rewrite_invalid_config() {
    let temp = tempfile::tempdir().unwrap();
    let settings = BackendSettings::default();
    assert!(!migrate_managed_model_catalog_pointer_in_home(temp.path(), &settings).unwrap());
    assert!(!temp.path().join("config.toml").exists());
    let invalid = "model_catalog_json = [\n";
    std::fs::write(temp.path().join("config.toml"), invalid).unwrap();
    assert!(migrate_managed_model_catalog_pointer_in_home(temp.path(), &settings).is_err());
    assert_eq!(
        std::fs::read_to_string(temp.path().join("config.toml")).unwrap(),
        invalid
    );
}

#[cfg(unix)]
#[test]
fn migration_does_not_follow_catalog_directory_or_file_symlinks() {
    for directory_link in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex-home");
        let external = temp.path().join("external");
        std::fs::create_dir(&home).unwrap();
        std::fs::create_dir(&external).unwrap();
        std::fs::write(external.join("custom.json"), "{}").unwrap();
        if directory_link {
            std::os::unix::fs::symlink(&external, home.join("model-catalogs")).unwrap();
        } else {
            std::fs::create_dir(home.join("model-catalogs")).unwrap();
            std::os::unix::fs::symlink(
                external.join("custom.json"),
                home.join("model-catalogs").join("custom.json"),
            )
            .unwrap();
        }
        let original = write_pointer(&home, Path::new("model-catalogs/custom.json"));
        assert!(
            !migrate_managed_model_catalog_pointer_in_home(&home, &BackendSettings::default())
                .unwrap()
        );
        assert_eq!(
            std::fs::read_to_string(home.join("config.toml")).unwrap(),
            original
        );
    }
}

#[cfg(unix)]
#[test]
fn migration_preserves_a_symlinked_user_config() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex-home");
    std::fs::create_dir_all(home.join("model-catalogs")).unwrap();
    std::fs::write(home.join("model-catalogs").join("existing.json"), "{}").unwrap();
    let external_config = temp.path().join("user-config.toml");
    let original = "model_catalog_json = \"model-catalogs/existing.json\"\n";
    std::fs::write(&external_config, original).unwrap();
    std::os::unix::fs::symlink(&external_config, home.join("config.toml")).unwrap();

    assert!(
        !migrate_managed_model_catalog_pointer_in_home(&home, &BackendSettings::default()).unwrap()
    );

    assert!(
        std::fs::symlink_metadata(home.join("config.toml"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(std::fs::read_to_string(&external_config).unwrap(), original);
}

#[test]
fn wsl_aggregate_catalog_pointer_stays_relative() {
    // #2591：聚合成员的目录发布走另一条写指针的路径，也必须遵守 WSL 的相对路径。
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("config.toml"),
        "[desktop]\nrunCodexInWindowsSubsystemForLinux = true\n",
    )
    .unwrap();
    let api = RelayProfile {
        id: "api".to_string(),
        relay_mode: RelayMode::PureApi,
        base_url: "https://api.example/v1".to_string(),
        api_key: "sk-api".to_string(),
        model: "agg-model".to_string(),
        model_list: "agg-model".to_string(),
        ..RelayProfile::default()
    };
    let settings = BackendSettings {
        relay_profiles_enabled: true,
        active_relay_id: "agg".to_string(),
        relay_profiles: vec![api],
        aggregate_relay_profiles: vec![AggregateRelayProfile {
            id: "agg".to_string(),
            name: "聚合供应商".to_string(),
            session_provider: RelaySessionProvider::Custom,
            model_aggregation_enabled: true,
            strategy: AggregateRelayStrategy::Failover,
            members: vec![AggregateRelayMember { relay_id: "api".to_string(), weight: 1 }],
            routes: Vec::new(),
            default_relay_id: String::new(),
            sync_default_member_login: false,
        }],
        active_aggregate_relay_id: "agg".to_string(),
        ..BackendSettings::default()
    };
    // 聚合目录已发布过（旧版发布的 legacy 名字），这次保存应重写指针。
    std::fs::create_dir_all(temp.path().join("model-catalogs")).unwrap();
    std::fs::write(temp.path().join("model-catalogs/agg.json"), "{\"models\":[]}").unwrap();
    let mut config = std::fs::read_to_string(temp.path().join("config.toml"))
        .unwrap()
        .parse::<toml_edit::DocumentMut>()
        .unwrap();
    config["model_catalog_json"] = toml_edit::value("model-catalogs/agg.json");
    std::fs::write(temp.path().join("config.toml"), config.to_string()).unwrap();
    let store = SettingsStore::new(temp.path().join("settings.json"));
    store.save(&settings).unwrap();
    let mut saved = settings.clone();
    saved.settings_revision = store.revision().unwrap();
    codex_plus_core::aggregate_catalog::save_with_active_catalog_refresh(
        &store,
        temp.path(),
        &saved,
    )
    .unwrap();
    let actual = pointer(temp.path());
    assert!(
        actual.is_relative(),
        "WSL 下聚合目录指针不能写成 Windows 绝对路径：{actual:?}"
    );
    assert!(temp.path().join(&actual).is_file(), "{actual:?}");
}
