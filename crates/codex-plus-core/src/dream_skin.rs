use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use toml_edit::{DocumentMut, InlineTable, Item, Table, Value, value};

use crate::settings::DreamSkinThemeConfig;

const BACKUP_FILE: &str = "dream-skin-base-theme-backup.json";
const BACKUP_SCHEMA_VERSION: u32 = 2;
const MANAGED_THEME_DIR: &str = "dream-skin/theme";
const MANAGED_IMAGE_PREFIX: &str = "current.";
pub const DREAM_SKIN_SOURCE_LIMIT: u64 = 50 * 1024 * 1024;
pub const DREAM_SKIN_PREPARED_LIMIT: u64 = 16 * 1024 * 1024;
const APPEARANCE_KEYS: [&str; 3] = [
    "appearanceTheme",
    "appearanceLightCodeThemeId",
    "appearanceLightChromeTheme",
];

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DreamSkinThemeBackup {
    schema_version: u32,
    config_path: String,
    desktop_existed: bool,
    values: BTreeMap<String, Option<String>>,
}

pub fn import_dream_skin_image(source: &Path, state_dir: &Path) -> anyhow::Result<PathBuf> {
    let managed_dir = state_dir.join(MANAGED_THEME_DIR);
    let destination = prepare_dream_skin_image_for_directory(source, &managed_dir, "current")?;
    remove_other_managed_images(&managed_dir, &destination)?;
    Ok(destination)
}

pub(crate) fn prepare_dream_skin_image_for_directory(
    source: &Path,
    destination_dir: &Path,
    destination_stem: &str,
) -> anyhow::Result<PathBuf> {
    if destination_stem.is_empty()
        || !destination_stem
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        bail!("invalid Dream Skin destination name");
    }
    let metadata = std::fs::symlink_metadata(source)
        .with_context(|| format!("failed to read image metadata {}", source.display()))?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        bail!("Dream Skin image is not a file");
    }
    if metadata.len() == 0 {
        bail!("Dream Skin image is empty");
    }
    if metadata.len() > DREAM_SKIN_SOURCE_LIMIT {
        bail!("Dream Skin source image exceeds 50 MiB");
    }

    let extension = supported_image_extension(source)?;
    std::fs::create_dir_all(destination_dir).with_context(|| {
        format!(
            "failed to create Dream Skin theme directory {}",
            destination_dir.display()
        )
    })?;

    let (prepared_path, prepared_extension) = prepare_image(source, destination_dir, &extension)?;
    let prepared_metadata = std::fs::metadata(&prepared_path).with_context(|| {
        format!(
            "failed to read prepared image metadata {}",
            prepared_path.display()
        )
    })?;
    if prepared_metadata.len() == 0 {
        remove_prepared_copy(source, &prepared_path);
        bail!("prepared Dream Skin image is empty");
    }
    if prepared_metadata.len() > DREAM_SKIN_PREPARED_LIMIT {
        remove_prepared_copy(source, &prepared_path);
        bail!("prepared Dream Skin image exceeds 16 MiB");
    }

    let destination = destination_dir.join(format!("{destination_stem}.{prepared_extension}"));
    let bytes = std::fs::read(&prepared_path)
        .with_context(|| format!("failed to read prepared image {}", prepared_path.display()))?;
    crate::settings::atomic_write(&destination, &bytes)
        .with_context(|| format!("failed to store Dream Skin image {}", destination.display()))?;
    remove_prepared_copy(source, &prepared_path);
    Ok(destination)
}

pub fn clear_managed_dream_skin_image(state_dir: &Path) -> anyhow::Result<()> {
    let managed_dir = state_dir.join(MANAGED_THEME_DIR);
    if !managed_dir.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(&managed_dir)
        .with_context(|| format!("failed to read {}", managed_dir.display()))?
    {
        let path = entry?.path();
        if is_managed_image_name(&path) && path.is_file() {
            std::fs::remove_file(&path)
                .with_context(|| format!("failed to remove {}", path.display()))?;
        }
    }
    Ok(())
}

pub fn is_managed_dream_skin_image(path: &Path, state_dir: &Path) -> bool {
    if !is_managed_image_name(path) || !path.is_file() {
        return false;
    }
    let Ok(path) = std::fs::canonicalize(path) else {
        return false;
    };
    let Ok(root) = std::fs::canonicalize(state_dir.join(MANAGED_THEME_DIR)) else {
        return false;
    };
    path.parent().is_some_and(|parent| parent == root)
}

fn supported_image_extension(path: &Path) -> anyhow::Result<String> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| anyhow::anyhow!("unsupported Dream Skin image format"))?;
    #[cfg(target_os = "macos")]
    let supported = matches!(
        extension.as_str(),
        "png" | "jpg" | "jpeg" | "heic" | "tif" | "tiff" | "webp"
    );
    #[cfg(not(target_os = "macos"))]
    let supported = matches!(
        extension.as_str(),
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp"
    );
    if !supported {
        bail!("unsupported Dream Skin image format: {extension}");
    }
    Ok(extension)
}

#[cfg(target_os = "macos")]
fn prepare_image(
    source: &Path,
    managed_dir: &Path,
    _extension: &str,
) -> anyhow::Result<(PathBuf, String)> {
    let prepared = managed_dir.join(".dream-skin-import.jpg");
    let output = std::process::Command::new("/usr/bin/sips")
        .args(["-s", "format", "jpeg"])
        .arg(source)
        .arg("--out")
        .arg(&prepared)
        .output()
        .context("failed to run macOS image converter")?;
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
        bail!("macOS could not convert the Dream Skin image: {message}");
    }
    Ok((prepared, "jpg".to_string()))
}

#[cfg(not(target_os = "macos"))]
fn prepare_image(
    source: &Path,
    _managed_dir: &Path,
    extension: &str,
) -> anyhow::Result<(PathBuf, String)> {
    Ok((source.to_path_buf(), extension.to_string()))
}

fn remove_prepared_copy(source: &Path, prepared: &Path) {
    if prepared != source {
        let _ = std::fs::remove_file(prepared);
    }
}

fn remove_other_managed_images(managed_dir: &Path, current: &Path) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(managed_dir)
        .with_context(|| format!("failed to read {}", managed_dir.display()))?
    {
        let path = entry?.path();
        if path != current && is_managed_image_name(&path) && path.is_file() {
            std::fs::remove_file(&path)
                .with_context(|| format!("failed to remove old image {}", path.display()))?;
        }
    }
    Ok(())
}

fn is_managed_image_name(path: &Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|name| name.starts_with(MANAGED_IMAGE_PREFIX))
}

pub fn sync_default_dream_skin_base_theme(
    enabled: bool,
    theme: &DreamSkinThemeConfig,
) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        return sync_dream_skin_base_theme_in_home(
            &crate::codex_home::default_codex_home_dir(),
            &crate::paths::default_app_state_dir(),
            enabled,
            theme,
        );
    }
    #[cfg(not(windows))]
    {
        let _ = enabled;
        let _ = theme;
        Ok(())
    }
}

pub fn sync_dream_skin_base_theme_in_home(
    home: &Path,
    state_dir: &Path,
    enabled: bool,
    theme: &DreamSkinThemeConfig,
) -> anyhow::Result<()> {
    let config_path = home.join("config.toml");
    let backup_path = state_dir.join(BACKUP_FILE);
    if enabled {
        apply_base_theme(&config_path, &backup_path, theme)
    } else {
        restore_base_theme(&config_path, &backup_path)
    }
}

fn apply_base_theme(
    config_path: &Path,
    backup_path: &Path,
    theme: &DreamSkinThemeConfig,
) -> anyhow::Result<()> {
    let existing = read_config_or_empty(config_path)?;
    let mut document = parse_config(&existing, config_path)?;
    let desktop_existed = document.get("desktop").is_some();

    if backup_path.exists() {
        validate_backup_identity(backup_path, config_path)?;
    } else {
        let values = APPEARANCE_KEYS
            .iter()
            .map(|key| {
                let item = document
                    .get("desktop")
                    .and_then(Item::as_table)
                    .and_then(|desktop| desktop.get(key))
                    .map(serialize_item);
                ((*key).to_string(), item)
            })
            .collect();
        let backup = DreamSkinThemeBackup {
            schema_version: BACKUP_SCHEMA_VERSION,
            config_path: config_path.to_string_lossy().to_string(),
            desktop_existed,
            values,
        };
        let bytes = serde_json::to_vec_pretty(&backup)?;
        crate::settings::atomic_write(backup_path, &bytes).with_context(|| {
            format!(
                "failed to back up Dream Skin theme to {}",
                backup_path.display()
            )
        })?;
    }

    let backup = read_backup(backup_path)?;
    let profile = target_base_theme(theme);
    let desktop = desktop_table_mut(&mut document)?;
    match profile.appearance_theme {
        Some(appearance) => desktop["appearanceTheme"] = value(appearance),
        None => restore_backup_setting(desktop, &backup, "appearanceTheme")?,
    }
    if uses_native_chrome_theme(theme) {
        restore_backup_setting(desktop, &backup, "appearanceLightCodeThemeId")?;
        restore_backup_setting(desktop, &backup, "appearanceLightChromeTheme")?;
    } else {
        desktop["appearanceLightCodeThemeId"] = value("codex");
        desktop["appearanceLightChromeTheme"] =
            Item::Value(Value::InlineTable(target_chrome_theme(theme, profile)));
    }
    write_config(config_path, document.to_string().as_bytes())
}

fn restore_base_theme(config_path: &Path, backup_path: &Path) -> anyhow::Result<()> {
    if !backup_path.exists() {
        return Ok(());
    }
    let backup = read_backup(backup_path)?;
    if backup.config_path != config_path.to_string_lossy() {
        bail!("Dream Skin theme backup belongs to a different config.toml");
    }

    let existing = read_config_or_empty(config_path)?;
    let mut document = parse_config(&existing, config_path)?;
    if document.get("desktop").is_none() {
        document["desktop"] = Item::Table(Table::new());
    }
    let desktop = desktop_table_mut(&mut document)?;
    for key in APPEARANCE_KEYS {
        match backup.values.get(key).and_then(Option::as_deref) {
            Some(serialized) => {
                desktop[key] = parse_backup_item(&backup, serialized)
                    .with_context(|| format!("failed to restore Dream Skin setting {key}"))?;
            }
            None => {
                desktop.remove(key);
            }
        }
    }
    if !backup.desktop_existed && desktop.is_empty() {
        document.remove("desktop");
    }
    write_config(config_path, document.to_string().as_bytes())?;
    std::fs::remove_file(backup_path).with_context(|| {
        format!(
            "failed to remove restored Dream Skin backup {}",
            backup_path.display()
        )
    })?;
    Ok(())
}

fn desktop_table_mut(document: &mut DocumentMut) -> anyhow::Result<&mut Table> {
    if document.get("desktop").is_none() {
        document["desktop"] = Item::Table(Table::new());
    }
    document
        .get_mut("desktop")
        .and_then(Item::as_table_mut)
        .context("[desktop] must be a regular TOML table")
}

#[derive(Clone, Copy)]
struct TargetBaseTheme {
    appearance_theme: Option<&'static str>,
    accent: &'static str,
    contrast: i64,
    ink: &'static str,
    opaque_windows: bool,
    diff_added: &'static str,
    diff_removed: &'static str,
    skill: &'static str,
    surface: &'static str,
}

fn target_base_theme(theme: &DreamSkinThemeConfig) -> TargetBaseTheme {
    let preset = crate::settings::resolve_dream_skin_style_preset(&theme.id, &theme.style_preset);
    let mut profile = match preset.as_str() {
        "codex-snow" => TargetBaseTheme {
            appearance_theme: Some("light"),
            accent: "#1F7FE8",
            contrast: 64,
            ink: "#10263F",
            opaque_windows: true,
            diff_added: "#A9DFC5",
            diff_removed: "#F1AEB5",
            skill: "#54A8E8",
            surface: "#F7FCFF",
        },
        "glass-vision" => TargetBaseTheme {
            appearance_theme: Some("light"),
            accent: "#4B91D8",
            contrast: 54,
            ink: "#18334F",
            opaque_windows: false,
            diff_added: "#BFEBD9",
            diff_removed: "#F1C4CC",
            skill: "#77C8E8",
            surface: "#EAF7FF",
        },
        _ => TargetBaseTheme {
            appearance_theme: None,
            accent: "#B65CFF",
            contrast: 64,
            ink: "#4A235F",
            opaque_windows: true,
            diff_added: "#BCE8CF",
            diff_removed: "#F7B8CE",
            skill: "#C47BFF",
            surface: "#FFF4FA",
        },
    };
    profile.appearance_theme = match theme
        .extra_fields
        .get("appearance")
        .and_then(serde_json::Value::as_str)
    {
        Some("light") => Some("light"),
        Some("dark") => Some("dark"),
        _ => profile.appearance_theme,
    };
    profile
}

fn target_palette_accent(theme: &DreamSkinThemeConfig) -> Option<&str> {
    theme
        .extra_fields
        .get("palette")
        .and_then(serde_json::Value::as_object)
        .and_then(|palette| palette.get("accent"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|accent| !accent.is_empty())
}

fn uses_native_chrome_theme(theme: &DreamSkinThemeConfig) -> bool {
    let preset = crate::settings::resolve_dream_skin_style_preset(&theme.id, &theme.style_preset);
    !matches!(preset.as_str(), "codex-snow" | "glass-vision")
        && target_palette_accent(theme).is_none()
}

fn restore_backup_setting(
    desktop: &mut Table,
    backup: &DreamSkinThemeBackup,
    key: &str,
) -> anyhow::Result<()> {
    match backup.values.get(key).and_then(Option::as_deref) {
        Some(serialized) => {
            desktop[key] = parse_backup_item(backup, serialized)
                .with_context(|| format!("failed to restore Dream Skin setting {key}"))?;
        }
        None => {
            desktop.remove(key);
        }
    }
    Ok(())
}

fn target_chrome_theme(
    theme_config: &DreamSkinThemeConfig,
    profile: TargetBaseTheme,
) -> InlineTable {
    let accent = target_palette_accent(theme_config).unwrap_or(profile.accent);
    let mut fonts = InlineTable::new();
    fonts.insert("code", "Cascadia Code".into());
    fonts.insert("ui", "Microsoft YaHei UI".into());

    let mut semantic_colors = InlineTable::new();
    semantic_colors.insert("diffAdded", profile.diff_added.into());
    semantic_colors.insert("diffRemoved", profile.diff_removed.into());
    semantic_colors.insert("skill", profile.skill.into());

    let mut theme = InlineTable::new();
    theme.insert("accent", accent.into());
    theme.insert("contrast", profile.contrast.into());
    theme.insert("fonts", Value::InlineTable(fonts));
    theme.insert("ink", profile.ink.into());
    theme.insert("opaqueWindows", profile.opaque_windows.into());
    theme.insert("semanticColors", Value::InlineTable(semantic_colors));
    theme.insert("surface", profile.surface.into());
    theme
}

fn serialize_item(item: &Item) -> String {
    let mut document = DocumentMut::new();
    document["value"] = item.clone();
    document.to_string()
}

fn parse_backup_item(backup: &DreamSkinThemeBackup, serialized: &str) -> anyhow::Result<Item> {
    match backup.schema_version {
        1 => parse_legacy_item(serialized),
        BACKUP_SCHEMA_VERSION => parse_document_item(serialized),
        _ => bail!("unsupported Dream Skin backup schema"),
    }
}

fn parse_legacy_item(serialized: &str) -> anyhow::Result<Item> {
    match format!("value = {serialized}\n").parse::<DocumentMut>() {
        Ok(mut document) => document
            .remove("value")
            .context("serialized Dream Skin backup value is missing"),
        Err(_) => parse_legacy_table(serialized),
    }
}

fn parse_legacy_table(serialized: &str) -> anyhow::Result<Item> {
    let document = serialized.parse::<DocumentMut>()?;
    Ok(Item::Table(document.as_table().clone()))
}

fn parse_document_item(serialized: &str) -> anyhow::Result<Item> {
    let mut document = serialized.parse::<DocumentMut>()?;
    document
        .remove("value")
        .context("serialized Dream Skin backup value is missing")
}

fn read_backup(path: &Path) -> anyhow::Result<DreamSkinThemeBackup> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("failed to read Dream Skin backup {}", path.display()))?;
    let backup: DreamSkinThemeBackup = serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to parse Dream Skin backup {}", path.display()))?;
    if !matches!(backup.schema_version, 1 | BACKUP_SCHEMA_VERSION) {
        bail!("unsupported Dream Skin backup schema");
    }
    Ok(backup)
}

fn validate_backup_identity(backup_path: &Path, config_path: &Path) -> anyhow::Result<()> {
    let backup = read_backup(backup_path)?;
    if backup.config_path != config_path.to_string_lossy() {
        bail!("Dream Skin theme backup belongs to a different config.toml");
    }
    Ok(())
}

fn read_config_or_empty(path: &Path) -> anyhow::Result<String> {
    match std::fs::read_to_string(path) {
        Ok(contents) => Ok(contents),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error).with_context(|| format!("failed to read {}", path.display())),
    }
}

fn parse_config(contents: &str, path: &Path) -> anyhow::Result<DocumentMut> {
    contents
        .parse::<DocumentMut>()
        .with_context(|| format!("failed to parse {}", path.display()))
}

fn write_config(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    crate::settings::atomic_write(path, bytes).with_context(|| {
        format!(
            "failed to write Dream Skin base theme to {}",
            path.display()
        )
    })
}
