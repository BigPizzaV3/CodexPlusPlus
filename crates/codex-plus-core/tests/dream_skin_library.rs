use std::path::Path;

use codex_plus_core::dream_skin_library::{
    DreamSkinThemeDraft, DreamSkinThemeKind, create_dream_skin_theme_from_image,
    delete_dream_skin_theme, list_dream_skin_themes, load_stored_dream_skin_theme,
    prepare_dream_skin_activation, rename_dream_skin_theme, save_dream_skin_theme,
};
use codex_plus_core::settings::{BackendSettings, DreamSkinThemeConfig};

fn write_test_png(path: &Path) {
    std::fs::write(
        path,
        include_bytes!("../../../assets/inject/dream-skin-default.png"),
    )
    .unwrap();
}

fn write_theme_pack(state_dir: &Path, directory_id: &str, config_id: &str, name: &str) {
    let dir = state_dir.join("dream-skin/themes").join(directory_id);
    std::fs::create_dir_all(&dir).unwrap();
    let mut theme = DreamSkinThemeConfig::default();
    theme.id = config_id.into();
    theme.name = name.into();
    std::fs::write(
        dir.join("theme.json"),
        serde_json::to_vec_pretty(&theme).unwrap(),
    )
    .unwrap();
    write_test_png(&dir.join("image.png"));
}

fn stored_test_draft(state_dir: &Path, id: &str, name: &str) -> DreamSkinThemeDraft {
    let image = state_dir.join(format!("{id}.png"));
    write_test_png(&image);
    let mut config = DreamSkinThemeConfig::default();
    config.id = id.into();
    config.name = name.into();
    DreamSkinThemeDraft {
        config,
        image_path: image.to_string_lossy().into_owned(),
        builtin: false,
    }
}

#[test]
fn library_always_lists_builtin_theme_first() {
    let temp = tempfile::tempdir().unwrap();
    let settings = BackendSettings::default();

    let library = list_dream_skin_themes(temp.path(), &settings).unwrap();

    assert_eq!(library.themes[0].kind, DreamSkinThemeKind::Builtin);
    assert_eq!(
        library.themes[0].name,
        if cfg!(windows) {
            "桥本有菜"
        } else {
            "Dream Skin"
        }
    );
    assert!(library.themes[0].builtin);
}

#[test]
fn library_scans_valid_theme_directory() {
    let temp = tempfile::tempdir().unwrap();
    write_theme_pack(temp.path(), "night-desk", "night-desk", "Night Desk");

    let library = list_dream_skin_themes(temp.path(), &BackendSettings::default()).unwrap();

    assert!(library.themes.iter().any(|item| item.id == "night-desk"));
}

#[test]
fn scan_ignores_invalid_ids_and_mismatched_theme_json() {
    let temp = tempfile::tempdir().unwrap();
    write_theme_pack(temp.path(), "Bad-ID", "Bad-ID", "Invalid");
    write_theme_pack(temp.path(), "valid-id", "other-id", "Mismatch");

    let library = list_dream_skin_themes(temp.path(), &BackendSettings::default()).unwrap();

    assert_eq!(library.themes.len(), 1);
}

#[test]
fn legacy_active_settings_are_exposed_as_unsaved() {
    let temp = tempfile::tempdir().unwrap();
    let mut settings = BackendSettings::default();
    settings.codex_app_dream_skin_theme_config.id = "legacy-custom".into();
    settings.codex_app_dream_skin_theme_config.name = "Legacy Custom".into();

    let library = list_dream_skin_themes(temp.path(), &settings).unwrap();

    let active = library.themes.iter().find(|item| item.active).unwrap();
    assert_eq!(active.kind, DreamSkinThemeKind::ActiveUnsaved);
    assert_eq!(active.name, "Legacy Custom");
}

#[test]
fn creates_theme_from_image_with_unique_stable_id() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("Night Desk.png");
    write_test_png(&source);

    let first = create_dream_skin_theme_from_image(&source, temp.path()).unwrap();
    let second = create_dream_skin_theme_from_image(&source, temp.path()).unwrap();

    assert_eq!(first.config.id, "night-desk");
    assert_eq!(second.config.id, "night-desk-2");
    assert!(Path::new(&first.image_path).is_file());
}

#[test]
fn saving_and_renaming_never_changes_theme_id() {
    let temp = tempfile::tempdir().unwrap();
    let draft = stored_test_draft(temp.path(), "stable-id", "Before");
    save_dream_skin_theme(temp.path(), &draft).unwrap();

    let renamed = rename_dream_skin_theme(temp.path(), "stable-id", "After").unwrap();

    assert_eq!(renamed.id, "stable-id");
    let stored = load_stored_dream_skin_theme(temp.path(), "stable-id").unwrap();
    assert_eq!(stored.config.name, "After");
}

#[test]
fn save_rejects_path_traversal_id() {
    let temp = tempfile::tempdir().unwrap();
    let mut draft = stored_test_draft(temp.path(), "safe", "Safe");
    draft.config.id = "../escape".into();

    let error = save_dream_skin_theme(temp.path(), &draft).unwrap_err();

    assert!(error.to_string().contains("theme id"));
    assert!(!temp.path().join("dream-skin/escape").exists());
}

#[test]
fn failed_update_preserves_existing_theme_pack() {
    let temp = tempfile::tempdir().unwrap();
    let original = stored_test_draft(temp.path(), "stable-id", "Before");
    save_dream_skin_theme(temp.path(), &original).unwrap();
    let mut broken = original.clone();
    broken.config.name = "After".into();
    broken.image_path = temp
        .path()
        .join("missing.png")
        .to_string_lossy()
        .into_owned();

    assert!(save_dream_skin_theme(temp.path(), &broken).is_err());
    let stored = load_stored_dream_skin_theme(temp.path(), "stable-id").unwrap();
    assert_eq!(stored.config.name, "Before");
}

#[test]
fn delete_rejects_builtin_current_and_unknown_files() {
    let temp = tempfile::tempdir().unwrap();
    let draft = stored_test_draft(temp.path(), "current", "Current");
    save_dream_skin_theme(temp.path(), &draft).unwrap();
    let dir = temp.path().join("dream-skin/themes/current");

    assert!(delete_dream_skin_theme(temp.path(), "builtin", Some("current")).is_err());
    assert!(delete_dream_skin_theme(temp.path(), "current", Some("current")).is_err());

    std::fs::write(dir.join("keep.txt"), b"unknown").unwrap();
    assert!(delete_dream_skin_theme(temp.path(), "current", None).is_err());
    assert!(dir.join("keep.txt").exists());
}

#[test]
fn prepare_activation_does_not_replace_current_image_on_failure() {
    let temp = tempfile::tempdir().unwrap();
    let active_dir = temp.path().join("dream-skin/theme");
    std::fs::create_dir_all(&active_dir).unwrap();
    std::fs::write(active_dir.join("current.png"), b"before").unwrap();
    let mut draft = stored_test_draft(temp.path(), "missing", "Missing");
    draft.image_path = temp
        .path()
        .join("does-not-exist.png")
        .to_string_lossy()
        .into_owned();

    let error = prepare_dream_skin_activation(temp.path(), &draft).unwrap_err();

    assert!(error.to_string().contains("does-not-exist.png"));
    assert_eq!(
        std::fs::read(active_dir.join("current.png")).unwrap(),
        b"before"
    );
}

#[test]
fn prepare_activation_copies_theme_into_current_slot() {
    let temp = tempfile::tempdir().unwrap();
    let draft = stored_test_draft(temp.path(), "ready", "Ready");

    let activation = prepare_dream_skin_activation(temp.path(), &draft).unwrap();

    assert_eq!(activation.config.id, "ready");
    assert!(Path::new(&activation.active_image_path).is_file());
    assert!(
        Path::new(&activation.active_image_path).starts_with(temp.path().join("dream-skin/theme"))
    );
}

#[test]
fn save_rejects_css_injection_in_color_fields() {
    let temp = tempfile::tempdir().unwrap();
    let mut draft = stored_test_draft(temp.path(), "unsafe-color", "Unsafe Color");
    draft
        .config
        .colors
        .get_or_insert_with(codex_plus_core::settings::DreamSkinColors::default)
        .accent = "red; background:url(file:///secret)".into();

    let error = save_dream_skin_theme(temp.path(), &draft).unwrap_err();

    assert!(error.to_string().contains("theme color"));
    assert!(!temp.path().join("dream-skin/themes/unsafe-color").exists());
}

#[test]
fn save_rejects_invalid_style_preset() {
    let temp = tempfile::tempdir().unwrap();
    let mut draft = stored_test_draft(temp.path(), "unsafe-style", "Unsafe Style");
    draft.config.style_preset = "../../outside".into();

    let error = save_dream_skin_theme(temp.path(), &draft).unwrap_err();

    assert!(error.to_string().contains("style preset"));
    assert!(!temp.path().join("dream-skin/themes/unsafe-style").exists());
}

#[test]
fn activated_stored_theme_is_not_reported_as_modified() {
    let temp = tempfile::tempdir().unwrap();
    let draft = stored_test_draft(temp.path(), "stable", "Stable");
    save_dream_skin_theme(temp.path(), &draft).unwrap();
    let stored = load_stored_dream_skin_theme(temp.path(), "stable").unwrap();
    let activation = prepare_dream_skin_activation(temp.path(), &stored).unwrap();
    let mut settings = BackendSettings::default();
    settings.codex_app_dream_skin_theme_config = activation.config;
    settings.codex_app_dream_skin_image_path = activation.active_image_path;

    let library = list_dream_skin_themes(temp.path(), &settings).unwrap();
    let active = library.themes.iter().find(|item| item.active).unwrap();

    assert_eq!(active.id, "stable");
    assert!(!active.modified);
}

#[test]
fn scan_ignores_symbolic_link_theme_directory() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("external-theme");
    std::fs::create_dir_all(&target).unwrap();
    let themes = temp.path().join("dream-skin/themes");
    std::fs::create_dir_all(&themes).unwrap();
    let link = themes.join("linked-theme");
    if create_directory_symlink(&target, &link).is_err() {
        return;
    }

    let library = list_dream_skin_themes(temp.path(), &BackendSettings::default()).unwrap();

    assert!(!library.themes.iter().any(|item| item.id == "linked-theme"));
}

#[cfg(windows)]
fn create_directory_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
}

#[cfg(unix)]
fn create_directory_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}
