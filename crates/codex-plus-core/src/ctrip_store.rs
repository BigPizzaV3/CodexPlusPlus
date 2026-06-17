use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::relay_config::{CTRIP_ADA_BASE_URL, CTRIP_ADA_ENV_KEY};
use crate::settings::BackendSettings;

pub const CTRIP_CODEX_DIR: &str = ".ctripcodex";
pub const CTRIP_TOKEN_FILE: &str = "token";

pub fn default_ctrip_codex_dir() -> PathBuf {
    directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().join(CTRIP_CODEX_DIR))
        .unwrap_or_else(|| PathBuf::from(CTRIP_CODEX_DIR))
}

pub fn ctrip_token_path() -> PathBuf {
    default_ctrip_codex_dir().join(CTRIP_TOKEN_FILE)
}

pub fn load_ctrip_token() -> Option<String> {
    let path = ctrip_token_path();
    let token = std::fs::read_to_string(&path)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())?;
    Some(token)
}

pub fn save_ctrip_token(token: &str) -> anyhow::Result<()> {
    let token = token.trim();
    if token.is_empty() {
        anyhow::bail!("ADA Token 不能为空");
    }
    let path = ctrip_token_path();
    crate::settings::atomic_write(&path, token.as_bytes())
        .with_context(|| format!("写入 {} 失败", path.display()))?;
    restrict_token_file_permissions(&path)?;
    Ok(())
}

fn restrict_token_file_permissions(path: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(path)
            .with_context(|| format!("读取 {} 权限失败", path.display()))?
            .permissions();
        permissions.set_mode(0o600);
        std::fs::set_permissions(path, permissions)
            .with_context(|| format!("设置 {} 权限失败", path.display()))?;
    }
    Ok(())
}

pub fn ctrip_launch_env_vars() -> Option<Vec<(String, String)>> {
    let token = load_ctrip_token()?;
    Some(vec![
        (CTRIP_ADA_ENV_KEY.to_string(), token),
        ("OPENAI_BASE_URL".to_string(), CTRIP_ADA_BASE_URL.to_string()),
    ])
}

pub fn token_from_settings(settings: &BackendSettings) -> Option<String> {
    settings
        .relay_profiles
        .iter()
        .find(|profile| profile.id == "ctrip-ada" || profile.preset_id == "ctrip-ada")
        .and_then(|profile| {
            let token = profile.api_key.trim();
            (!token.is_empty()).then(|| token.to_string())
        })
        .or_else(|| {
            let token = settings.relay_api_key.trim();
            (!token.is_empty()).then(|| token.to_string())
        })
}

pub fn migrate_token_from_settings(settings: &BackendSettings) -> anyhow::Result<bool> {
    if ctrip_token_path().exists() {
        return Ok(false);
    }
    let Some(token) = token_from_settings(settings) else {
        return Ok(false);
    };
    save_ctrip_token(&token)?;
    Ok(true)
}

pub fn load_ctrip_token_with_migration(settings: &BackendSettings) -> Option<String> {
    if let Some(token) = load_ctrip_token() {
        return Some(token);
    }
    migrate_token_from_settings(settings).ok()?;
    load_ctrip_token()
}

pub fn ctrip_cdp_injection_enabled() -> bool {
    load_ctrip_token().is_some()
}

pub fn ensure_ctrip_launch_settings() -> anyhow::Result<()> {
    use crate::settings::SettingsStore;

    if !ctrip_cdp_injection_enabled() {
        return Ok(());
    }
    let store = SettingsStore::default();
    let mut settings = store.load()?;
    if settings.enhancements_enabled {
        return Ok(());
    }
    settings.enhancements_enabled = true;
    store.save(&settings)
}
