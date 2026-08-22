use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, bail};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::dream_skin_library::DreamSkinThemeSummary;

pub const COMMUNITY_API_ORIGIN: &str = "https://api.dreamskin.cc";
pub const COMMUNITY_GALLERY_URL: &str = "https://dreamskin.cc/gallery";
pub const COMMUNITY_STUDIO_URL: &str = "https://dreamskin.cc/studio";

const CACHE_FILE: &str = "dream-skin/community/catalog.json";
const PENDING_LINK_FILE: &str = "dream-skin/community/pending-apply.json";
const PAGE_SIZE: usize = 48;
const CATALOG_LIMIT: usize = 500;
const CATALOG_BYTES_LIMIT: usize = 1024 * 1024;
const METADATA_BYTES_LIMIT: usize = 65_536;
const PACKAGE_BYTES_LIMIT: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DreamSkinCommunityCatalog {
    pub items: Vec<DreamSkinCommunityTheme>,
    pub total: usize,
    pub fetched_at: String,
    #[serde(default)]
    pub cached: bool,
    #[serde(default)]
    pub warning: String,
    #[serde(default)]
    pub installed_theme_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DreamSkinCommunityTheme {
    pub apply_compatible: bool,
    pub author_display_name: String,
    #[serde(default)]
    pub author_user_id: String,
    #[serde(default)]
    pub display_meta: serde_json::Value,
    #[serde(default)]
    pub download_count: usize,
    pub id: String,
    pub license: String,
    pub name: String,
    pub package_bytes: usize,
    pub package_sha256: String,
    #[serde(default)]
    pub reviewed_at: String,
    #[serde(default)]
    pub slug: String,
    #[serde(default)]
    pub submitted_at: String,
    pub theme_id: String,
    pub version: String,
    #[serde(default)]
    pub preview_url: String,
    #[serde(default)]
    pub installed: bool,
    #[serde(default)]
    pub installed_version: String,
    #[serde(default)]
    pub update_available: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CommunityPage {
    items: Vec<DreamSkinCommunityTheme>,
    total: usize,
}

pub async fn load_community_catalog(state_dir: &Path) -> anyhow::Result<DreamSkinCommunityCatalog> {
    match fetch_community_catalog().await {
        Ok(mut catalog) => {
            enrich_catalog(state_dir, &mut catalog);
            cache_catalog(state_dir, &catalog)?;
            Ok(catalog)
        }
        Err(network_error) => {
            let mut cached = read_cached_catalog(state_dir).with_context(|| {
                format!("DreamSkin 社区加载失败，且没有可用缓存：{network_error}")
            })?;
            cached.cached = true;
            cached.warning = format!("DreamSkin 社区暂不可用，当前显示本地缓存：{network_error}");
            enrich_catalog(state_dir, &mut cached);
            Ok(cached)
        }
    }
}

pub async fn fetch_community_catalog() -> anyhow::Result<DreamSkinCommunityCatalog> {
    let client = community_http_client(Duration::from_secs(30))?;
    let mut items = Vec::new();
    let mut offset = 0usize;
    let total = loop {
        let url = format!(
            "{COMMUNITY_API_ORIGIN}/v1/themes?limit={PAGE_SIZE}&offset={offset}&sort=recent"
        );
        let bytes =
            download_limited(&client, &url, CATALOG_BYTES_LIMIT, "application/json").await?;
        let page: CommunityPage =
            serde_json::from_slice(&bytes).context("DreamSkin 社区清单不是有效 JSON")?;
        let page_total = page.total.min(CATALOG_LIMIT);
        let count = page.items.len();
        items.extend(page.items);
        if count == 0 || items.len() >= page_total || items.len() >= CATALOG_LIMIT {
            break page_total;
        }
        offset = items.len();
    };
    items.truncate(CATALOG_LIMIT);
    validate_catalog(&items)?;
    Ok(DreamSkinCommunityCatalog {
        total,
        items,
        fetched_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .to_string(),
        cached: false,
        warning: String::new(),
        installed_theme_id: String::new(),
    })
}

pub async fn install_community_theme(
    state_dir: &Path,
    version_id: &str,
) -> anyhow::Result<DreamSkinThemeSummary> {
    validate_version_id(version_id)?;
    let client = community_http_client(Duration::from_secs(120))?;
    let metadata_url = format!("{COMMUNITY_API_ORIGIN}/v1/themes/{version_id}");
    let metadata_bytes = download_limited(
        &client,
        &metadata_url,
        METADATA_BYTES_LIMIT,
        "application/json",
    )
    .await?;
    let metadata: DreamSkinCommunityTheme =
        serde_json::from_slice(&metadata_bytes).context("DreamSkin 主题元数据不是有效 JSON")?;
    validate_community_theme(&metadata)?;
    if metadata.id != version_id {
        bail!("DreamSkin 主题版本与请求不一致");
    }
    if !metadata.apply_compatible {
        bail!("该主题是旧格式，只支持在线预览或手动下载");
    }

    let download_url = format!("{COMMUNITY_API_ORIGIN}/v1/themes/{version_id}/download");
    let package = download_limited(
        &client,
        &download_url,
        metadata.package_bytes,
        "application/zip",
    )
    .await?;
    if package.len() != metadata.package_bytes {
        bail!("DreamSkin 主题包实际大小与审核元数据不一致");
    }
    let actual_sha = format!("{:x}", Sha256::digest(&package));
    if !actual_sha.eq_ignore_ascii_case(&metadata.package_sha256) {
        bail!("DreamSkin 主题包 SHA-256 与审核元数据不一致");
    }
    let validated = crate::dream_skin_package::validate_and_read_package(
        &package,
        if cfg!(windows) { "windows" } else { "macos" },
    )?;
    if validated.manifest.theme_id != metadata.theme_id
        || validated.manifest.version != metadata.version
    {
        bail!("DreamSkin 主题包身份与审核元数据不一致");
    }
    crate::dream_skin_library::save_validated_dream_skin_package(state_dir, &validated)
}

pub fn import_theme_package(
    state_dir: &Path,
    archive_path: &Path,
) -> anyhow::Result<DreamSkinThemeSummary> {
    let metadata = std::fs::symlink_metadata(archive_path)
        .with_context(|| format!("无法读取主题包：{}", archive_path.display()))?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() == 0
        || metadata.len() as usize > PACKAGE_BYTES_LIMIT
    {
        bail!("主题包必须是 32 MiB 以内的普通 ZIP 文件");
    }
    let bytes = std::fs::read(archive_path)
        .with_context(|| format!("无法读取主题包：{}", archive_path.display()))?;
    let validated = crate::dream_skin_package::validate_and_read_package(
        &bytes,
        if cfg!(windows) { "windows" } else { "macos" },
    )?;
    crate::dream_skin_library::save_validated_dream_skin_package(state_dir, &validated)
}

pub fn save_pending_community_link(url: &str) -> anyhow::Result<String> {
    let version_id = version_id_from_link(url)?;
    let path = pending_link_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::settings::atomic_write(
        &path,
        &serde_json::to_vec(&serde_json::json!({ "versionId": version_id }))?,
    )?;
    Ok(version_id)
}

fn version_id_from_link(url: &str) -> anyhow::Result<String> {
    let version_id = url
        .strip_prefix("dreamskin://apply?version=")
        .or_else(|| url.strip_prefix("dreamskin://apply/?version="))
        .context("一键换肤链接无效")?;
    validate_version_id(version_id)?;
    Ok(version_id.to_string())
}

pub fn load_pending_community_link() -> anyhow::Result<Option<String>> {
    let path = pending_link_path();
    if !path.exists() {
        return Ok(None);
    }
    let metadata = std::fs::symlink_metadata(&path)?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() || metadata.len() > 4096
    {
        bail!("待处理的一键换肤记录无效");
    }
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let version_id = value
        .get("versionId")
        .and_then(serde_json::Value::as_str)
        .context("待处理的一键换肤记录缺少版本 ID")?
        .to_string();
    validate_version_id(&version_id)?;
    Ok(Some(version_id))
}

pub fn clear_pending_community_link() -> anyhow::Result<()> {
    let path = pending_link_path();
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn pending_link_path() -> std::path::PathBuf {
    crate::paths::default_app_state_dir().join(PENDING_LINK_FILE)
}

fn validate_catalog(items: &[DreamSkinCommunityTheme]) -> anyhow::Result<()> {
    if items.len() > CATALOG_LIMIT {
        bail!("DreamSkin 社区主题数量超过限制");
    }
    for item in items {
        validate_community_theme(item)?;
    }
    Ok(())
}

fn validate_community_theme(theme: &DreamSkinCommunityTheme) -> anyhow::Result<()> {
    validate_version_id(&theme.id)?;
    if !safe_text(&theme.theme_id, 80)
        || !safe_text(&theme.name, 120)
        || !safe_text(&theme.author_display_name, 120)
        || !safe_text(&theme.license, 80)
        || !valid_semver(&theme.version)
        || theme.package_bytes == 0
        || theme.package_bytes > PACKAGE_BYTES_LIMIT
        || !valid_sha256(&theme.package_sha256)
    {
        bail!("DreamSkin 社区主题元数据无效：{}", theme.id);
    }
    Ok(())
}

fn validate_version_id(value: &str) -> anyhow::Result<()> {
    let Some(suffix) = value.strip_prefix("ver_") else {
        bail!("DreamSkin 主题版本 ID 无效");
    };
    if !(8..=64).contains(&suffix.len())
        || !suffix
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
    {
        bail!("DreamSkin 主题版本 ID 无效");
    }
    Ok(())
}

fn safe_text(value: &str, maximum: usize) -> bool {
    let trimmed = value.trim();
    value == trimmed
        && !value.is_empty()
        && value.chars().count() <= maximum
        && value.chars().all(|character| {
            !character.is_control()
                && !matches!(
                    character as u32,
                    0x061c | 0x200b..=0x200f | 0x2028..=0x202e | 0x2060..=0x206f | 0xfeff
                )
        })
}

fn valid_semver(value: &str) -> bool {
    let parts = value.split('.').collect::<Vec<_>>();
    parts.len() == 3
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.len() <= 16
                && part.bytes().all(|byte| byte.is_ascii_digit())
                && (*part == "0" || !part.starts_with('0'))
        })
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn enrich_catalog(state_dir: &Path, catalog: &mut DreamSkinCommunityCatalog) {
    for item in &mut catalog.items {
        item.preview_url = format!(
            "{COMMUNITY_API_ORIGIN}/v1/themes/{}/preview/thumbnail",
            item.id
        );
        let manifest = state_dir
            .join("dream-skin/themes")
            .join(&item.theme_id)
            .join("manifest.json");
        item.installed_version = std::fs::read(&manifest)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .and_then(|value| value.get("version")?.as_str().map(str::to_string))
            .unwrap_or_default();
        item.installed = !item.installed_version.is_empty();
        item.update_available = item.installed && item.installed_version != item.version;
    }
}

fn cache_catalog(state_dir: &Path, catalog: &DreamSkinCommunityCatalog) -> anyhow::Result<()> {
    let path = state_dir.join(CACHE_FILE);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::settings::atomic_write(&path, &serde_json::to_vec(catalog)?)
}

fn read_cached_catalog(state_dir: &Path) -> anyhow::Result<DreamSkinCommunityCatalog> {
    let path = state_dir.join(CACHE_FILE);
    let metadata = std::fs::symlink_metadata(&path)?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() as usize > CATALOG_BYTES_LIMIT
    {
        bail!("DreamSkin 社区缓存无效");
    }
    let catalog: DreamSkinCommunityCatalog = serde_json::from_slice(&std::fs::read(path)?)?;
    validate_catalog(&catalog.items)?;
    Ok(catalog)
}

fn community_http_client(timeout: Duration) -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent(format!(
            "CodexPlusPlus/{} DreamSkin",
            env!("CARGO_PKG_VERSION")
        ))
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(8))
        .timeout(timeout)
        .build()?)
}

async fn download_limited(
    client: &reqwest::Client,
    url: &str,
    limit: usize,
    expected_content_type: &str,
) -> anyhow::Result<Vec<u8>> {
    let response = client
        .get(url)
        .header(reqwest::header::ACCEPT, expected_content_type)
        .send()
        .await
        .with_context(|| format!("请求失败：{url}"))?;
    if !response.status().is_success() {
        bail!("DreamSkin 服务返回 HTTP {}", response.status());
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::trim)
        .unwrap_or_default();
    if content_type != expected_content_type {
        bail!("DreamSkin 服务返回了不支持的内容类型：{content_type}");
    }
    if response
        .content_length()
        .is_some_and(|length| length == 0 || length > limit as u64)
    {
        bail!("DreamSkin 响应超过大小限制");
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if bytes.len().saturating_add(chunk.len()) > limit {
            bail!("DreamSkin 响应超过大小限制");
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.is_empty() {
        bail!("DreamSkin 服务返回空响应");
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::version_id_from_link;

    #[test]
    fn community_link_accepts_only_canonical_version_id() {
        assert_eq!(
            version_id_from_link("dreamskin://apply?version=ver_1234abcd").unwrap(),
            "ver_1234abcd"
        );
        assert_eq!(
            version_id_from_link("dreamskin://apply/?version=ver_1234abcd").unwrap(),
            "ver_1234abcd"
        );
        for invalid in [
            "https://api.dreamskin.cc/v1/themes/ver_1234abcd",
            "dreamskin://apply?version=ver_1234abcd&url=https://example.com",
            "dreamskin://apply/?version=ver_1234abcd&url=https://example.com",
            "dreamskin://apply//?version=ver_1234abcd",
            "dreamskin://apply?url=https://example.com",
            "dreamskin://apply?version=../theme",
            "dreamskin://apply?version=%76er_1234abcd",
        ] {
            assert!(version_id_from_link(invalid).is_err(), "accepted {invalid}");
        }
    }
}
