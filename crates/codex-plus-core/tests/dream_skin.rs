use codex_plus_core::dream_skin::{
    import_dream_skin_image, is_managed_dream_skin_image, sync_dream_skin_base_theme_in_home,
};
use codex_plus_core::settings::{BackendSettings, DreamSkinThemeConfig, SettingsStore};

#[test]
fn backend_settings_defaults_to_upstream_platform_theme_config() {
    let settings = BackendSettings::default();
    let theme = settings.codex_app_dream_skin_theme_config;

    assert_eq!(theme.schema_version, 1);
    assert!(theme.style_preset.is_empty());
    assert_eq!(theme.brand_subtitle, "CODEX DREAM SKIN");
    if cfg!(windows) {
        assert_eq!(theme.id, "preset-arina-hashimoto");
        assert_eq!(theme.name, "桥本有菜");
        assert_eq!(theme.tagline, "把柔光与玫瑰带进今天的工作台。");
        assert!(theme.colors.is_none());
        assert_eq!(theme.extra_fields["appearance"], "auto");
        assert_eq!(theme.extra_fields["art"]["safeArea"], "left");
    } else {
        assert_eq!(theme.id, "custom-1784123441349");
        assert_eq!(theme.name, "Dream Skin");
        assert_eq!(theme.tagline, "把喜欢的画面变成可交互的 Codex 工作台。");
        assert_eq!(theme.colors.as_ref().unwrap().accent, "#E25563");
        assert_eq!(theme.extra_fields["promoSub"], "passion8.cc");
    }
    assert!(!settings.codex_app_dream_skin_paused);
}

#[test]
fn target_theme_fields_survive_deserialize_and_serialize() {
    let source = serde_json::json!({
        "schemaVersion": 1,
        "id": "target-theme",
        "name": "Target Theme",
        "brandSubtitle": "TARGET",
        "tagline": "Target tagline",
        "projectPrefix": "project · ",
        "projectLabel": "Select project",
        "statusText": "ONLINE",
        "quote": "EXACT",
        "image": "background.webp",
        "appearance": "dark",
        "art": {
            "focusX": 0.72,
            "focusY": 0.45,
            "safeArea": "left",
            "taskMode": "ambient"
        },
        "palette": { "accent": "#123456", "custom": "keep" },
        "promoTitle": "Sponsor",
        "promoSub": "sponsor.example",
        "promoUrl": "https://sponsor.example",
        "customTargetField": { "nested": true }
    });

    let theme: DreamSkinThemeConfig = serde_json::from_value(source.clone()).unwrap();
    let saved = serde_json::to_value(theme).unwrap();

    for key in [
        "image",
        "appearance",
        "art",
        "palette",
        "promoTitle",
        "promoSub",
        "promoUrl",
        "customTargetField",
    ] {
        assert_eq!(saved[key], source[key], "target field changed: {key}");
    }
    assert!(saved.get("colors").is_none());
    assert!(saved.get("stylePreset").is_none());
}

#[test]
fn legacy_theme_name_migrates_without_losing_other_settings() {
    let temp = tempfile::tempdir().unwrap();
    let settings_path = temp.path().join("settings.json");
    std::fs::write(
        &settings_path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "codexAppDreamSkinEnabled": true,
            "codexAppDreamSkinTheme": "miku",
            "relayTestModel": "keep-me"
        }))
        .unwrap(),
    )
    .unwrap();

    let settings = SettingsStore::new(settings_path).load().unwrap();

    assert!(settings.codex_app_dream_skin_enabled);
    assert_eq!(settings.codex_app_dream_skin_theme_config.id, "miku");
    assert_eq!(settings.relay_test_model, "keep-me");
}

#[test]
fn imports_supported_image_into_managed_theme_directory() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.png");
    let image = include_bytes!("../../../assets/inject/dream-skin-default.png");
    std::fs::write(&source, image).unwrap();

    let imported = import_dream_skin_image(&source, temp.path()).unwrap();

    assert!(imported.starts_with(temp.path().join("dream-skin/theme")));
    assert_eq!(
        imported.file_name().and_then(|name| name.to_str()),
        Some("current.png")
    );
    assert_eq!(std::fs::read(&imported).unwrap(), image);
    assert!(is_managed_dream_skin_image(&imported, temp.path()));
}

#[test]
fn rejects_source_larger_than_fifty_mebibytes() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("too-large.png");
    let file = std::fs::File::create(&source).unwrap();
    file.set_len(50 * 1024 * 1024 + 1).unwrap();

    let error = import_dream_skin_image(&source, temp.path()).unwrap_err();

    assert!(error.to_string().contains("50 MiB"));
}

#[test]
fn rejects_prepared_image_larger_than_sixteen_mebibytes() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("prepared-too-large.png");
    let file = std::fs::File::create(&source).unwrap();
    file.set_len(16 * 1024 * 1024 + 1).unwrap();

    let error = import_dream_skin_image(&source, temp.path()).unwrap_err();

    assert!(error.to_string().contains("16 MiB"));
}

#[test]
fn failed_import_preserves_existing_managed_image() {
    let temp = tempfile::tempdir().unwrap();
    let managed_dir = temp.path().join("dream-skin/theme");
    std::fs::create_dir_all(&managed_dir).unwrap();
    let current = managed_dir.join("current.png");
    std::fs::write(&current, b"keep-current").unwrap();
    let unsupported = temp.path().join("source.svg");
    std::fs::write(&unsupported, b"<svg/>").unwrap();

    let error = import_dream_skin_image(&unsupported, temp.path()).unwrap_err();

    assert!(error.to_string().contains("unsupported"));
    assert_eq!(std::fs::read(current).unwrap(), b"keep-current");
}

#[test]
fn base_theme_apply_and_restore_preserve_unrelated_config() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex-home");
    let state = temp.path().join("state");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(
        home.join("config.toml"),
        r##"model = "gpt-5"

[desktop]
appearanceTheme = "dark"
appearanceLightCodeThemeId = "custom-light"
appearanceLightChromeTheme = { accent = "#112233", opaqueWindows = false }
keep = "before"
"##,
    )
    .unwrap();

    let theme = DreamSkinThemeConfig::default();

    sync_dream_skin_base_theme_in_home(&home, &state, true, &theme).unwrap();
    let backup: serde_json::Value = serde_json::from_slice(
        &std::fs::read(state.join("dream-skin-base-theme-backup.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(backup["schemaVersion"], 2);
    let applied = std::fs::read_to_string(home.join("config.toml")).unwrap();
    let applied_doc = applied.parse::<toml_edit::DocumentMut>().unwrap();
    assert_eq!(
        applied_doc["desktop"]["appearanceTheme"].as_str(),
        Some("dark")
    );
    assert!(applied.contains("appearanceLightCodeThemeId = \"custom-light\""));
    assert!(applied.contains("accent = \"#112233\""));
    assert!(applied.contains("opaqueWindows = false"));
    assert!(applied.contains("keep = \"before\""));

    std::fs::write(
        home.join("config.toml"),
        applied.replace("keep = \"before\"", "keep = \"after\""),
    )
    .unwrap();
    sync_dream_skin_base_theme_in_home(&home, &state, false, &theme).unwrap();

    let restored = std::fs::read_to_string(home.join("config.toml")).unwrap();
    let restored_doc = restored.parse::<toml_edit::DocumentMut>().unwrap();
    let desktop = restored_doc["desktop"].as_table().unwrap();
    assert_eq!(desktop["appearanceTheme"].as_str(), Some("dark"));
    assert_eq!(
        desktop["appearanceLightCodeThemeId"].as_str(),
        Some("custom-light")
    );
    let chrome = desktop["appearanceLightChromeTheme"]
        .as_inline_table()
        .unwrap();
    assert_eq!(chrome["accent"].as_str(), Some("#112233"));
    assert_eq!(chrome["opaqueWindows"].as_bool(), Some(false));
    assert!(restored.contains("keep = \"after\""));
    assert!(!state.join("dream-skin-base-theme-backup.json").exists());
}

#[test]
fn base_theme_restores_current_codex_nested_chrome_theme() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex-home");
    let state = temp.path().join("state");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(
        home.join("config.toml"),
        r##"model = "gpt-5"

[desktop]
appearanceTheme = "dark"
appearanceLightCodeThemeId = "custom-light"
keep = "before"

[desktop.appearanceLightChromeTheme]
accent = "#339cff"
contrast = 100
opaqueWindows = false

[desktop.appearanceLightChromeTheme.fonts]
code = "Berkeley Mono"
ui = "Inter"

[desktop.appearanceLightChromeTheme.semanticColors]
diffAdded = "#123456"
diffRemoved = "#654321"
"##,
    )
    .unwrap();

    let theme = DreamSkinThemeConfig::default();
    sync_dream_skin_base_theme_in_home(&home, &state, true, &theme).unwrap();
    sync_dream_skin_base_theme_in_home(&home, &state, false, &theme).unwrap();

    let restored = std::fs::read_to_string(home.join("config.toml")).unwrap();
    let restored_doc = restored.parse::<toml_edit::DocumentMut>().unwrap();
    let chrome = restored_doc["desktop"]["appearanceLightChromeTheme"]
        .as_table()
        .unwrap();
    assert_eq!(chrome["accent"].as_str(), Some("#339cff"));
    assert_eq!(chrome["contrast"].as_integer(), Some(100));
    assert_eq!(chrome["opaqueWindows"].as_bool(), Some(false));
    assert_eq!(chrome["fonts"]["code"].as_str(), Some("Berkeley Mono"));
    assert_eq!(chrome["fonts"]["ui"].as_str(), Some("Inter"));
    assert_eq!(
        chrome["semanticColors"]["diffAdded"].as_str(),
        Some("#123456")
    );
    assert_eq!(
        chrome["semanticColors"]["diffRemoved"].as_str(),
        Some("#654321")
    );
    assert_eq!(restored_doc["desktop"]["keep"].as_str(), Some("before"));
}

#[test]
fn base_theme_recovers_regular_table_from_legacy_backup() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex-home");
    let state = temp.path().join("state");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&state).unwrap();
    let config_path = home.join("config.toml");
    let original = r##"[desktop]
appearanceTheme = "dark"
appearanceLightCodeThemeId = "custom-light"

[desktop.appearanceLightChromeTheme]
accent = "#339cff"
contrast = 100

[desktop.appearanceLightChromeTheme.fonts]
code = "Berkeley Mono"

[desktop.appearanceLightChromeTheme.semanticColors]
diffAdded = "#123456"
"##;
    let original_doc = original.parse::<toml_edit::DocumentMut>().unwrap();
    let legacy_chrome = original_doc["desktop"]["appearanceLightChromeTheme"].to_string();
    std::fs::write(
        &config_path,
        r##"[desktop]
appearanceTheme = "dark"
appearanceLightCodeThemeId = "codex"
appearanceLightChromeTheme = { accent = "#B65CFF" }
"##,
    )
    .unwrap();
    let backup = serde_json::json!({
        "schemaVersion": 1,
        "configPath": config_path.to_string_lossy(),
        "desktopExisted": true,
        "values": {
            "appearanceTheme": "\"dark\"",
            "appearanceLightCodeThemeId": "\"custom-light\"",
            "appearanceLightChromeTheme": legacy_chrome
        }
    });
    std::fs::write(
        state.join("dream-skin-base-theme-backup.json"),
        serde_json::to_vec_pretty(&backup).unwrap(),
    )
    .unwrap();

    sync_dream_skin_base_theme_in_home(&home, &state, false, &DreamSkinThemeConfig::default())
        .unwrap();

    let restored = std::fs::read_to_string(config_path).unwrap();
    let restored_doc = restored.parse::<toml_edit::DocumentMut>().unwrap();
    let chrome = restored_doc["desktop"]["appearanceLightChromeTheme"]
        .as_table()
        .unwrap();
    assert_eq!(
        restored_doc["desktop"]["appearanceTheme"].as_str(),
        Some("dark")
    );
    assert_eq!(
        restored_doc["desktop"]["appearanceLightCodeThemeId"].as_str(),
        Some("custom-light")
    );
    assert_eq!(chrome["accent"].as_str(), Some("#339cff"));
    assert_eq!(chrome["contrast"].as_integer(), Some(100));
    assert!(!state.join("dream-skin-base-theme-backup.json").exists());
}

#[test]
fn snow_base_theme_matches_the_target_project_and_switching_back_restores_appearance() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex-home");
    let state = temp.path().join("state");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(
        home.join("config.toml"),
        "[desktop]\nappearanceTheme = \"dark\"\n",
    )
    .unwrap();
    let snow = DreamSkinThemeConfig {
        id: "codex-snow-skin".to_string(),
        style_preset: "codex-snow".to_string(),
        ..Default::default()
    };

    sync_dream_skin_base_theme_in_home(&home, &state, true, &snow).unwrap();
    let snow_config = std::fs::read_to_string(home.join("config.toml")).unwrap();
    assert!(snow_config.contains("appearanceTheme = \"light\""));
    assert!(snow_config.contains("accent = \"#1F7FE8\""));
    assert!(snow_config.contains("ink = \"#10263F\""));
    assert!(snow_config.contains("surface = \"#F7FCFF\""));

    sync_dream_skin_base_theme_in_home(&home, &state, true, &DreamSkinThemeConfig::default())
        .unwrap();
    let dream_config = std::fs::read_to_string(home.join("config.toml")).unwrap();
    let dream_doc = dream_config.parse::<toml_edit::DocumentMut>().unwrap();
    assert_eq!(
        dream_doc["desktop"]["appearanceTheme"].as_str(),
        Some("dark")
    );
    assert!(
        dream_doc["desktop"]
            .get("appearanceLightChromeTheme")
            .is_none()
    );
}

#[test]
fn glass_vision_base_theme_matches_the_target_project() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex-home");
    let state = temp.path().join("state");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("config.toml"), "[desktop]\n").unwrap();
    let glass = DreamSkinThemeConfig {
        id: "glass-vision".to_string(),
        style_preset: "glass-vision".to_string(),
        ..Default::default()
    };

    sync_dream_skin_base_theme_in_home(&home, &state, true, &glass).unwrap();
    let config = std::fs::read_to_string(home.join("config.toml")).unwrap();
    assert!(config.contains("appearanceTheme = \"light\""));
    assert!(config.contains("accent = \"#4B91D8\""));
    assert!(config.contains("contrast = 54"));
    assert!(config.contains("opaqueWindows = false"));
    assert!(config.contains("surface = \"#EAF7FF\""));
}

#[test]
fn base_theme_applies_explicit_appearance_and_palette_accent() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex-home");
    let state = temp.path().join("state");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(
        home.join("config.toml"),
        "[desktop]\nappearanceTheme = \"light\"\n",
    )
    .unwrap();
    let mut theme = DreamSkinThemeConfig::default();
    theme.extra_fields.insert(
        "appearance".to_string(),
        serde_json::Value::String("dark".to_string()),
    );
    theme.extra_fields.insert(
        "palette".to_string(),
        serde_json::json!({ "accent": "#123456" }),
    );

    sync_dream_skin_base_theme_in_home(&home, &state, true, &theme).unwrap();

    let config = std::fs::read_to_string(home.join("config.toml")).unwrap();
    let document = config.parse::<toml_edit::DocumentMut>().unwrap();
    assert_eq!(
        document["desktop"]["appearanceTheme"].as_str(),
        Some("dark")
    );
    assert_eq!(
        document["desktop"]["appearanceLightChromeTheme"]["accent"].as_str(),
        Some("#123456")
    );
}

#[test]
fn non_fixed_preset_without_accent_preserves_native_chrome_theme() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex-home");
    let state = temp.path().join("state");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(
        home.join("config.toml"),
        "[desktop]\nappearanceLightCodeThemeId = \"native\"\nappearanceLightChromeTheme = { accent = \"#112233\" }\n",
    )
    .unwrap();
    let mut theme = DreamSkinThemeConfig::default();
    theme.id = "preset-cyber-neon".to_string();
    theme.style_preset = "cyber-neon".to_string();

    sync_dream_skin_base_theme_in_home(&home, &state, true, &theme).unwrap();

    let config = std::fs::read_to_string(home.join("config.toml")).unwrap();
    assert!(config.contains("appearanceLightCodeThemeId = \"native\""));
    assert!(config.contains("accent = \"#112233\""));
    assert!(!config.contains("#B65CFF"));
}

#[test]
fn base_theme_restore_removes_generated_desktop_table() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("codex-home");
    let state = temp.path().join("state");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("config.toml"), "model = \"gpt-5\"\n").unwrap();

    let theme = DreamSkinThemeConfig::default();
    sync_dream_skin_base_theme_in_home(&home, &state, true, &theme).unwrap();
    sync_dream_skin_base_theme_in_home(&home, &state, false, &theme).unwrap();

    let restored = std::fs::read_to_string(home.join("config.toml")).unwrap();
    assert_eq!(restored, "model = \"gpt-5\"\n");
}
