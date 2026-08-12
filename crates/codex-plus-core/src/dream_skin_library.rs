use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

use crate::settings::{BackendSettings, DreamSkinThemeConfig};

const THEMES_DIR: &str = "dream-skin/themes";
const THEME_CONFIG_FILE: &str = "theme.json";
const THEME_CONFIG_LIMIT: u64 = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DreamSkinThemeKind {
    Builtin,
    Stored,
    ActiveUnsaved,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DreamSkinThemeSummary {
    pub key: String,
    pub id: String,
    pub name: String,
    pub preview_path: String,
    pub kind: DreamSkinThemeKind,
    pub builtin: bool,
    pub active: bool,
    pub modified: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DreamSkinThemeDraft {
    pub config: DreamSkinThemeConfig,
    pub image_path: String,
    pub builtin: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DreamSkinThemeLibrary {
    pub themes: Vec<DreamSkinThemeSummary>,
    pub active_draft: DreamSkinThemeDraft,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DreamSkinActivation {
    pub config: DreamSkinThemeConfig,
    pub active_image_path: String,
}

pub fn list_dream_skin_themes(
    state_dir: &Path,
    settings: &BackendSettings,
) -> anyhow::Result<DreamSkinThemeLibrary> {
    let default_theme = DreamSkinThemeConfig::default();
    let active_config = settings.codex_app_dream_skin_theme_config.clone();
    let active_image_path = settings.codex_app_dream_skin_image_path.trim().to_string();
    let mut themes = vec![DreamSkinThemeSummary {
        key: "builtin".to_string(),
        id: default_theme.id.clone(),
        name: default_theme.name.clone(),
        preview_path: String::new(),
        kind: DreamSkinThemeKind::Builtin,
        builtin: true,
        active: active_config == default_theme && active_image_path.is_empty(),
        modified: false,
    }];

    let themes_dir = state_dir.join(THEMES_DIR);
    if themes_dir.is_dir() {
        for entry in std::fs::read_dir(&themes_dir)
            .with_context(|| format!("failed to read {}", themes_dir.display()))?
        {
            let entry = entry?;
            if let Some(summary) =
                read_theme_summary(&entry.path(), &active_config, &active_image_path)
            {
                themes.push(summary);
            }
        }
    }

    if !themes.iter().any(|theme| theme.active) {
        themes.push(DreamSkinThemeSummary {
            key: "active-unsaved".to_string(),
            id: active_config.id.clone(),
            name: active_config.name.clone(),
            preview_path: active_image_path.clone(),
            kind: DreamSkinThemeKind::ActiveUnsaved,
            builtin: false,
            active: true,
            modified: true,
        });
    }

    themes[1..].sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.id.cmp(&right.id))
    });

    Ok(DreamSkinThemeLibrary {
        themes,
        active_draft: DreamSkinThemeDraft {
            config: active_config,
            image_path: active_image_path,
            builtin: false,
        },
    })
}

fn read_theme_summary(
    directory: &Path,
    active_config: &DreamSkinThemeConfig,
    active_image_path: &str,
) -> Option<DreamSkinThemeSummary> {
    let metadata = std::fs::symlink_metadata(directory).ok()?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return None;
    }
    let id = directory.file_name()?.to_str()?;
    if !valid_theme_id(id) {
        return None;
    }
    let config_path = directory.join(THEME_CONFIG_FILE);
    let config_metadata = std::fs::symlink_metadata(&config_path).ok()?;
    if !config_metadata.file_type().is_file()
        || config_metadata.file_type().is_symlink()
        || config_metadata.len() > THEME_CONFIG_LIMIT
    {
        return None;
    }
    let config: DreamSkinThemeConfig =
        serde_json::from_slice(&std::fs::read(&config_path).ok()?).ok()?;
    if config.id != id || config.name.trim().is_empty() {
        return None;
    }
    let image_path = find_theme_image(directory)?;
    let active = config.id == active_config.id;
    let modified = active
        && (config != *active_config || !image_matches_active(&image_path, active_image_path));
    Some(DreamSkinThemeSummary {
        key: format!("stored:{id}"),
        id: config.id,
        name: config.name,
        preview_path: image_path.to_string_lossy().into_owned(),
        kind: DreamSkinThemeKind::Stored,
        builtin: false,
        active,
        modified,
    })
}

fn find_theme_image(directory: &Path) -> Option<PathBuf> {
    let mut images = std::fs::read_dir(directory)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            let Ok(metadata) = std::fs::symlink_metadata(path) else {
                return false;
            };
            metadata.file_type().is_file()
                && !metadata.file_type().is_symlink()
                && supported_image_extension(path)
        });
    let image = images.next()?;
    if images.next().is_some() {
        return None;
    }
    Some(image)
}

fn supported_image_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|extension| {
            matches!(
                extension.as_str(),
                "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp"
            )
        })
}

fn stored_theme_image_extension(state_dir: &Path, source: &Path) -> Option<String> {
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .filter(|extension| {
            matches!(
                extension.as_str(),
                "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp"
            )
        })?;
    let source = std::fs::canonicalize(source).ok()?;
    let themes_dir = std::fs::canonicalize(state_dir.join(THEMES_DIR)).ok()?;
    let theme_dir = source.parent()?;
    if theme_dir.parent()? != themes_dir {
        return None;
    }
    let theme_id = theme_dir.file_name()?.to_str()?;
    valid_theme_id(theme_id).then_some(extension)
}

fn valid_theme_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    (1..=64).contains(&bytes.len())
        && bytes[0].is_ascii_alphanumeric()
        && bytes.iter().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_' | b'.')
        })
}

pub fn create_dream_skin_theme_from_image(
    source: &Path,
    state_dir: &Path,
) -> anyhow::Result<DreamSkinThemeDraft> {
    let stem = source
        .file_stem()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("Dream Skin");
    let base_id = slugify_theme_id(stem);
    let themes_dir = state_dir.join(THEMES_DIR);
    let mut id = base_id.clone();
    let mut suffix = 2;
    while themes_dir.join(&id).exists() {
        id = format!("{base_id}-{suffix}");
        suffix += 1;
    }
    let mut config = DreamSkinThemeConfig::default();
    config.id = id;
    config.name = stem.to_string();
    let draft = DreamSkinThemeDraft {
        config,
        image_path: source.to_string_lossy().into_owned(),
        builtin: false,
    };
    save_dream_skin_theme(state_dir, &draft)?;
    load_stored_dream_skin_theme(state_dir, &draft.config.id)
}

pub fn save_dream_skin_theme(
    state_dir: &Path,
    draft: &DreamSkinThemeDraft,
) -> anyhow::Result<DreamSkinThemeSummary> {
    validate_theme_draft(draft)?;
    let themes_dir = state_dir.join(THEMES_DIR);
    std::fs::create_dir_all(&themes_dir)
        .with_context(|| format!("failed to create {}", themes_dir.display()))?;
    reject_symlink(&themes_dir)?;

    let suffix = unique_suffix();
    let staging = themes_dir.join(format!(".stage-{}-{suffix}", draft.config.id));
    std::fs::create_dir(&staging).with_context(|| {
        format!(
            "failed to create theme staging directory {}",
            staging.display()
        )
    })?;

    let staged = (|| -> anyhow::Result<()> {
        if draft.image_path.trim().is_empty() {
            let (_, bytes) = crate::assets::dream_skin_default_image();
            crate::settings::atomic_write(&staging.join("image.png"), bytes)?;
        } else {
            crate::dream_skin::prepare_dream_skin_image_for_directory(
                Path::new(draft.image_path.trim()),
                &staging,
                "image",
            )?;
        }
        let config = serde_json::to_vec_pretty(&draft.config)?;
        if config.len() as u64 > THEME_CONFIG_LIMIT {
            bail!("Dream Skin theme config exceeds 256 KiB");
        }
        crate::settings::atomic_write(&staging.join(THEME_CONFIG_FILE), &config)?;
        Ok(())
    })();
    if let Err(error) = staged {
        let _ = remove_known_theme_directory(&staging);
        return Err(error);
    }

    let target = themes_dir.join(&draft.config.id);
    replace_theme_directory(&staging, &target, &suffix)?;
    let stored = load_stored_dream_skin_theme(state_dir, &draft.config.id)?;
    Ok(summary_from_draft(&stored, false, false))
}

pub fn save_validated_dream_skin_package(
    state_dir: &Path,
    package: &crate::dream_skin_package::ValidatedDreamSkinPackage,
) -> anyhow::Result<DreamSkinThemeSummary> {
    let config: DreamSkinThemeConfig = serde_json::from_value(package.theme.clone())
        .context("主题包 theme.json 与 Codex++ 主题配置不兼容")?;
    let draft = DreamSkinThemeDraft {
        config: config.clone(),
        image_path: String::new(),
        builtin: false,
    };
    validate_theme_draft(&draft)?;

    let themes_dir = state_dir.join(THEMES_DIR);
    std::fs::create_dir_all(&themes_dir)
        .with_context(|| format!("failed to create {}", themes_dir.display()))?;
    reject_symlink(&themes_dir)?;
    let target = themes_dir.join(&config.id);
    if target.exists() {
        let existing = load_stored_dream_skin_theme(state_dir, &config.id)
            .context("同 ID 本地主题无法确认身份，已拒绝覆盖")?;
        if existing.config.id != config.id {
            bail!("同 ID 本地主题身份不一致，已拒绝覆盖");
        }
    }
    let suffix = unique_suffix();
    let staging = themes_dir.join(format!(".stage-{}-{suffix}", config.id));
    std::fs::create_dir(&staging).with_context(|| {
        format!(
            "failed to create Dream Skin package staging directory {}",
            staging.display()
        )
    })?;
    let staged = (|| -> anyhow::Result<()> {
        let image_extension = match package.image_name.as_str() {
            "background.webp" => "webp",
            "background.jpg" => "jpg",
            "background.png" => "png",
            other => bail!("unsupported Dream Skin package image: {other}"),
        };
        crate::settings::atomic_write(
            &staging.join(format!("image.{image_extension}")),
            &package.image_bytes,
        )?;
        crate::settings::atomic_write(
            &staging.join(THEME_CONFIG_FILE),
            &serde_json::to_vec_pretty(&config)?,
        )?;
        crate::settings::atomic_write(&staging.join("theme.css"), package.css.as_bytes())?;
        crate::settings::atomic_write(&staging.join("manifest.json"), &package.manifest_bytes)?;
        if let Some(license) = &package.license_bytes {
            crate::settings::atomic_write(&staging.join("LICENSE.txt"), license)?;
        }
        Ok(())
    })();
    if let Err(error) = staged {
        let _ = remove_known_theme_directory(&staging);
        return Err(error);
    }
    if let Err(error) = replace_theme_directory(&staging, &target, &suffix) {
        let _ = remove_known_theme_directory(&staging);
        return Err(error);
    }
    let stored = load_stored_dream_skin_theme(state_dir, &config.id)?;
    Ok(summary_from_draft(&stored, false, false))
}

pub fn load_stored_dream_skin_theme(
    state_dir: &Path,
    id: &str,
) -> anyhow::Result<DreamSkinThemeDraft> {
    if !valid_theme_id(id) {
        bail!("invalid Dream Skin theme id");
    }
    let directory = state_dir.join(THEMES_DIR).join(id);
    let metadata = std::fs::symlink_metadata(&directory)
        .with_context(|| format!("Dream Skin theme not found: {id}"))?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        bail!("Dream Skin theme path is not a safe directory");
    }
    let config_path = directory.join(THEME_CONFIG_FILE);
    let metadata = std::fs::symlink_metadata(&config_path)
        .with_context(|| format!("Dream Skin theme config not found: {id}"))?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > THEME_CONFIG_LIMIT
    {
        bail!("invalid Dream Skin theme config");
    }
    let config: DreamSkinThemeConfig = serde_json::from_slice(&std::fs::read(&config_path)?)?;
    if config.id != id {
        bail!("Dream Skin theme id does not match directory");
    }
    let image = find_theme_image(&directory)
        .ok_or_else(|| anyhow::anyhow!("Dream Skin theme must contain exactly one image"))?;
    let draft = DreamSkinThemeDraft {
        config,
        image_path: image.to_string_lossy().into_owned(),
        builtin: false,
    };
    validate_theme_draft(&draft)?;
    Ok(draft)
}

pub fn rename_dream_skin_theme(
    state_dir: &Path,
    id: &str,
    name: &str,
) -> anyhow::Result<DreamSkinThemeSummary> {
    let mut draft = load_stored_dream_skin_theme(state_dir, id)?;
    draft.config.name = name.trim().to_string();
    save_dream_skin_theme(state_dir, &draft)
}

pub fn prepare_dream_skin_activation(
    state_dir: &Path,
    draft: &DreamSkinThemeDraft,
) -> anyhow::Result<DreamSkinActivation> {
    if draft.builtin {
        if draft.config != DreamSkinThemeConfig::default() || !draft.image_path.trim().is_empty() {
            bail!("invalid built-in Dream Skin theme draft");
        }
        crate::dream_skin::clear_managed_dream_skin_image(state_dir)?;
        return Ok(DreamSkinActivation {
            config: draft.config.clone(),
            active_image_path: String::new(),
        });
    }
    validate_theme_draft(draft)?;
    let managed_dir = state_dir.join("dream-skin/theme");
    let source = Path::new(draft.image_path.trim());
    let active_image = if let Some(extension) = stored_theme_image_extension(state_dir, source) {
        std::fs::create_dir_all(&managed_dir)
            .with_context(|| format!("failed to create {}", managed_dir.display()))?;
        let destination = managed_dir.join(format!("current.{extension}"));
        let bytes = std::fs::read(source)
            .with_context(|| format!("failed to read Dream Skin image {}", source.display()))?;
        crate::settings::atomic_write(&destination, &bytes).with_context(|| {
            format!("failed to store Dream Skin image {}", destination.display())
        })?;
        destination
    } else {
        crate::dream_skin::prepare_dream_skin_image_for_directory(source, &managed_dir, "current")?
    };
    let active_css = managed_dir.join("current.css");
    let source_css = source.parent().map(|parent| parent.join("theme.css"));
    if let Some(source_css) = source_css {
        let metadata = std::fs::symlink_metadata(&source_css).ok();
        if metadata.is_some_and(|item| {
            item.file_type().is_file() && !item.file_type().is_symlink() && item.len() <= 262_144
        }) {
            let css = std::fs::read(&source_css).with_context(|| {
                format!(
                    "failed to read Dream Skin Safe CSS {}",
                    source_css.display()
                )
            })?;
            crate::dream_skin_package::validate_safe_css(
                std::str::from_utf8(&css).context("Dream Skin Safe CSS is not valid UTF-8")?,
            )?;
            crate::settings::atomic_write(&active_css, &css)?;
        } else {
            let _ = std::fs::remove_file(&active_css);
        }
    } else {
        let _ = std::fs::remove_file(&active_css);
    }
    for entry in std::fs::read_dir(&managed_dir)? {
        let path = entry?.path();
        let is_other_current = path != active_image
            && path != active_css
            && path.is_file()
            && path
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| name.starts_with("current."));
        if is_other_current {
            std::fs::remove_file(&path)
                .with_context(|| format!("failed to remove old image {}", path.display()))?;
        }
    }
    Ok(DreamSkinActivation {
        config: draft.config.clone(),
        active_image_path: active_image.to_string_lossy().into_owned(),
    })
}

pub fn delete_dream_skin_theme(
    state_dir: &Path,
    id: &str,
    active_id: Option<&str>,
) -> anyhow::Result<()> {
    if !valid_theme_id(id) {
        bail!("invalid Dream Skin theme id");
    }
    if active_id.is_some_and(|active| active == id) {
        bail!("current Dream Skin theme cannot be deleted");
    }
    let directory = state_dir.join(THEMES_DIR).join(id);
    let metadata = std::fs::symlink_metadata(&directory)
        .with_context(|| format!("Dream Skin theme not found: {id}"))?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        bail!("Dream Skin theme path is not a safe directory");
    }
    let stored = load_stored_dream_skin_theme(state_dir, id)?;
    if stored.config.id == DreamSkinThemeConfig::default().id {
        bail!("built-in Dream Skin theme cannot be deleted");
    }
    remove_known_theme_directory(&directory)
}

fn validate_theme_draft(draft: &DreamSkinThemeDraft) -> anyhow::Result<()> {
    if draft.builtin {
        bail!("built-in Dream Skin theme is read-only");
    }
    if !valid_theme_id(&draft.config.id) {
        bail!("invalid Dream Skin theme id");
    }
    if draft.config.schema_version != 1 {
        bail!("unsupported Dream Skin theme schema");
    }
    if draft.config.name.trim().is_empty() {
        bail!("Dream Skin theme name is empty");
    }
    if !draft.config.style_preset.is_empty() && !valid_theme_id(&draft.config.style_preset) {
        bail!("invalid Dream Skin style preset");
    }
    if let Some(colors) = &draft.config.colors {
        for color in [
            &colors.background,
            &colors.panel,
            &colors.panel_alt,
            &colors.accent,
            &colors.accent_alt,
            &colors.secondary,
            &colors.highlight,
            &colors.text,
            &colors.muted,
            &colors.line,
        ] {
            if !valid_css_color(color) {
                bail!("invalid Dream Skin theme color: {color}");
            }
        }
    }
    Ok(())
}

fn valid_css_color(value: &str) -> bool {
    let value = value.trim();
    if let Some(hex) = value.strip_prefix('#') {
        return matches!(hex.len(), 3 | 4 | 6 | 8)
            && hex.bytes().all(|byte| byte.is_ascii_hexdigit());
    }
    let body = value
        .strip_prefix("rgb(")
        .or_else(|| value.strip_prefix("rgba("))
        .and_then(|value| value.strip_suffix(')'));
    body.is_some_and(|body| {
        !body.is_empty()
            && body.bytes().all(|byte| {
                byte.is_ascii_digit()
                    || byte.is_ascii_whitespace()
                    || matches!(byte, b',' | b'.' | b'%')
            })
    })
}

fn slugify_theme_id(value: &str) -> String {
    let mut slug = String::new();
    let mut separator = false;
    for character in value.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            if separator && !slug.is_empty() {
                slug.push('-');
            }
            slug.push(character);
            separator = false;
        } else {
            separator = true;
        }
    }
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        format!("theme-{}", unique_suffix())
    } else {
        slug.chars().take(56).collect()
    }
}

fn unique_suffix() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{}-{nanos}", std::process::id())
}

fn replace_theme_directory(staging: &Path, target: &Path, suffix: &str) -> anyhow::Result<()> {
    if !target.exists() {
        std::fs::rename(staging, target)
            .with_context(|| format!("failed to install Dream Skin theme {}", target.display()))?;
        return Ok(());
    }
    reject_symlink(target)?;
    ensure_known_theme_directory(target)?;
    let parent = target
        .parent()
        .context("Dream Skin theme target has no parent")?;
    let id = target
        .file_name()
        .and_then(|value| value.to_str())
        .context("Dream Skin theme target has invalid id")?;
    let backup = parent.join(format!(".backup-{id}-{suffix}"));
    std::fs::rename(target, &backup)
        .with_context(|| format!("failed to back up Dream Skin theme {id}"))?;
    if let Err(error) = std::fs::rename(staging, target) {
        let _ = std::fs::rename(&backup, target);
        return Err(error).context("failed to replace Dream Skin theme directory");
    }
    remove_known_theme_directory(&backup)?;
    Ok(())
}

fn ensure_known_theme_directory(directory: &Path) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(directory)? {
        let path = entry?.path();
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        let metadata = std::fs::symlink_metadata(&path)?;
        let known = metadata.file_type().is_file()
            && !metadata.file_type().is_symlink()
            && (name == THEME_CONFIG_FILE
                || name == ".dream-skin-import.jpg"
                || (name.starts_with("image.") && supported_image_extension(&path))
                || matches!(name, "theme.css" | "manifest.json" | "LICENSE.txt"));
        if !known {
            bail!("Dream Skin theme directory contains unknown entry: {name}");
        }
    }
    Ok(())
}

fn remove_known_theme_directory(directory: &Path) -> anyhow::Result<()> {
    if !directory.exists() {
        return Ok(());
    }
    ensure_known_theme_directory(directory)?;
    for entry in std::fs::read_dir(directory)? {
        let path = entry?.path();
        std::fs::remove_file(&path)
            .with_context(|| format!("failed to remove {}", path.display()))?;
    }
    std::fs::remove_dir(directory)
        .with_context(|| format!("failed to remove {}", directory.display()))?;
    Ok(())
}

fn reject_symlink(path: &Path) -> anyhow::Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        bail!("Dream Skin path must not be a symbolic link");
    }
    Ok(())
}

fn summary_from_draft(
    draft: &DreamSkinThemeDraft,
    active: bool,
    modified: bool,
) -> DreamSkinThemeSummary {
    DreamSkinThemeSummary {
        key: format!("stored:{}", draft.config.id),
        id: draft.config.id.clone(),
        name: draft.config.name.clone(),
        preview_path: draft.image_path.clone(),
        kind: DreamSkinThemeKind::Stored,
        builtin: false,
        active,
        modified,
    }
}

fn image_matches_active(stored: &Path, active_path: &str) -> bool {
    let Ok(stored_bytes) = std::fs::read(stored) else {
        return false;
    };
    if active_path.trim().is_empty() {
        return stored_bytes == crate::assets::dream_skin_default_image().1;
    }
    std::fs::read(active_path).is_ok_and(|active_bytes| active_bytes == stored_bytes)
}
