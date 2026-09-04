use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, bail};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::dream_skin_library::{DreamSkinThemeDraft, DreamSkinThemeSummary};
use crate::settings::DreamSkinThemeConfig;

pub const DEFAULT_MARKET_INDEX_URL: &str =
    "https://raw.githubusercontent.com/BigPizzaV3/CodexPlusPlus-Themes/main/index.json";
pub const DEFAULT_MARKET_RAW_BASE_URL: &str =
    "https://raw.githubusercontent.com/BigPizzaV3/CodexPlusPlus-Themes/main/";
pub const DEFAULT_MARKET_REPOSITORY_URL: &str =
    "https://github.com/BigPizzaV3/CodexPlusPlus-Themes";

const MARKET_CACHE_FILE: &str = "dream-skin/market/index.json";
const MARKET_INSTALLS_FILE: &str = "dream-skin/market/installed.json";
const MARKET_INDEX_LIMIT: usize = 1024 * 1024;
const MARKET_THEME_LIMIT: usize = 256 * 1024;
const MARKET_IMAGE_LIMIT: usize = 16 * 1024 * 1024;
const MARKET_THEME_COUNT_LIMIT: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DreamSkinMarketManifest {
    pub schema_version: u8,
    #[serde(default, alias = "updated_at")]
    pub updated_at: String,
    pub themes: Vec<DreamSkinMarketTheme>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DreamSkinMarketTheme {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    #[serde(default)]
    pub description: String,
    pub license: String,
    #[serde(alias = "source_url")]
    pub source_url: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub theme: String,
    pub image: String,
    pub preview: String,
    #[serde(alias = "theme_sha256")]
    pub theme_sha256: String,
    #[serde(alias = "image_sha256")]
    pub image_sha256: String,
    #[serde(default, skip_deserializing)]
    pub preview_url: String,
    #[serde(default, skip_deserializing)]
    pub installed: bool,
    #[serde(default, skip_deserializing)]
    pub installed_version: String,
    #[serde(default, skip_deserializing)]
    pub update_available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DreamSkinMarketLoad {
    pub manifest: DreamSkinMarketManifest,
    pub cached: bool,
    pub warning: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MarketInstallRecords {
    #[serde(default = "market_schema_version")]
    schema_version: u8,
    #[serde(default)]
    themes: BTreeMap<String, String>,
}

pub async fn load_market(state_dir: &Path) -> anyhow::Result<DreamSkinMarketLoad> {
    match fetch_market_manifest(DEFAULT_MARKET_INDEX_URL).await {
        Ok(manifest) => {
            cache_manifest(state_dir, &manifest)?;
            Ok(DreamSkinMarketLoad {
                manifest: enrich_market_manifest(state_dir, manifest),
                cached: false,
                warning: None,
            })
        }
        Err(network_error) => {
            let cached = read_cached_manifest(state_dir)
                .with_context(|| format!("主题市场加载失败，且没有可用缓存：{network_error}"))?;
            Ok(DreamSkinMarketLoad {
                manifest: enrich_market_manifest(state_dir, cached),
                cached: true,
                warning: Some(format!(
                    "远程市场暂不可用，当前显示本地缓存：{network_error}"
                )),
            })
        }
    }
}

pub async fn fetch_market_manifest(url: &str) -> anyhow::Result<DreamSkinMarketManifest> {
    let client = market_http_client()?;
    let bytes = download_limited(&client, url, MARKET_INDEX_LIMIT).await?;
    let mut manifest: DreamSkinMarketManifest =
        serde_json::from_slice(&bytes).context("主题市场清单不是有效 JSON")?;
    validate_manifest(&mut manifest)?;
    Ok(manifest)
}

pub async fn install_market_theme(
    state_dir: &Path,
    theme: &DreamSkinMarketTheme,
) -> anyhow::Result<DreamSkinThemeSummary> {
    install_market_theme_from_base(state_dir, theme, DEFAULT_MARKET_RAW_BASE_URL).await
}

pub async fn install_market_theme_from_base(
    state_dir: &Path,
    theme: &DreamSkinMarketTheme,
    raw_base_url: &str,
) -> anyhow::Result<DreamSkinThemeSummary> {
    validate_market_theme(theme)?;
    let client = market_http_client()?;
    let theme_url = market_asset_url(raw_base_url, &theme.theme)?;
    let image_url = market_asset_url(raw_base_url, &theme.image)?;
    let theme_bytes = download_limited(&client, &theme_url, MARKET_THEME_LIMIT).await?;
    verify_sha256(&theme_bytes, &theme.theme_sha256, "主题配置")?;
    let config: DreamSkinThemeConfig =
        serde_json::from_slice(&theme_bytes).context("市场主题配置不是有效 JSON")?;
    if config.id != theme.id || config.name != theme.name {
        bail!("市场主题配置与清单身份不一致");
    }

    let image_bytes = download_limited(&client, &image_url, MARKET_IMAGE_LIMIT).await?;
    verify_sha256(&image_bytes, &theme.image_sha256, "主题图片")?;
    let extension =
        detect_image_extension(&image_bytes).context("市场主题图片格式无效或不受支持")?;
    let download_path = market_download_path(state_dir, &theme.id, extension)?;
    crate::settings::atomic_write(&download_path, &image_bytes)
        .with_context(|| format!("无法暂存市场主题图片 {}", download_path.display()))?;

    let draft = DreamSkinThemeDraft {
        config,
        image_path: download_path.to_string_lossy().into_owned(),
        builtin: false,
    };
    let installed = crate::dream_skin_library::save_dream_skin_theme(state_dir, &draft);
    let _ = std::fs::remove_file(&download_path);
    let installed = installed?;
    record_market_install(state_dir, &theme.id, &theme.version)?;
    Ok(installed)
}

pub fn enrich_market_manifest(
    state_dir: &Path,
    mut manifest: DreamSkinMarketManifest,
) -> DreamSkinMarketManifest {
    let records = read_install_records(state_dir).unwrap_or_default();
    for theme in &mut manifest.themes {
        theme.preview_url =
            market_asset_url(DEFAULT_MARKET_RAW_BASE_URL, &theme.preview).unwrap_or_default();
        let local_theme = state_dir
            .join("dream-skin/themes")
            .join(&theme.id)
            .join("theme.json");
        theme.installed = local_theme.is_file();
        theme.installed_version = if theme.installed {
            records.themes.get(&theme.id).cloned().unwrap_or_default()
        } else {
            String::new()
        };
        theme.update_available = theme.installed
            && !theme.installed_version.is_empty()
            && theme.installed_version != theme.version;
    }
    manifest
}

fn validate_manifest(manifest: &mut DreamSkinMarketManifest) -> anyhow::Result<()> {
    if manifest.schema_version != 1 {
        bail!("不支持的主题市场清单版本");
    }
    if manifest.themes.len() > MARKET_THEME_COUNT_LIMIT {
        bail!("主题市场清单超过 {} 项限制", MARKET_THEME_COUNT_LIMIT);
    }
    let mut ids = HashSet::new();
    for theme in &mut manifest.themes {
        validate_market_theme(theme)?;
        if !ids.insert(theme.id.clone()) {
            bail!("主题市场清单包含重复 ID：{}", theme.id);
        }
        theme.preview_url = market_asset_url(DEFAULT_MARKET_RAW_BASE_URL, &theme.preview)?;
    }
    Ok(())
}

fn validate_market_theme(theme: &DreamSkinMarketTheme) -> anyhow::Result<()> {
    if !valid_theme_id(&theme.id) {
        bail!("主题市场包含无效 ID：{}", theme.id);
    }
    for (label, value, limit) in [
        ("名称", theme.name.as_str(), 100usize),
        ("版本", theme.version.as_str(), 40usize),
        ("作者", theme.author.as_str(), 100usize),
        ("许可证", theme.license.as_str(), 100usize),
    ] {
        if value.trim().is_empty() || value.len() > limit {
            bail!("市场主题 {} 的{}无效", theme.id, label);
        }
    }
    if theme.description.len() > 500 || theme.tags.len() > 12 {
        bail!("市场主题 {} 的描述或标签数量超过限制", theme.id);
    }
    if theme
        .tags
        .iter()
        .any(|tag| tag.trim().is_empty() || tag.len() > 32)
    {
        bail!("市场主题 {} 包含无效标签", theme.id);
    }
    let source = reqwest::Url::parse(&theme.source_url)
        .with_context(|| format!("市场主题 {} 的来源地址无效", theme.id))?;
    if !matches!(source.scheme(), "http" | "https") {
        bail!("市场主题 {} 的来源地址协议无效", theme.id);
    }
    for path in [&theme.theme, &theme.image, &theme.preview] {
        validate_market_path(path)?;
    }
    validate_sha256(&theme.theme_sha256)?;
    validate_sha256(&theme.image_sha256)?;
    Ok(())
}

fn market_http_client() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent(format!(
            "CodexPlusPlus-Themes/{}",
            env!("CARGO_PKG_VERSION")
        ))
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(30))
        .build()?)
}

async fn download_limited(
    client: &reqwest::Client,
    url: &str,
    limit: usize,
) -> anyhow::Result<Vec<u8>> {
    let response = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("请求失败：{url}"))?
        .error_for_status()
        .with_context(|| format!("服务器返回错误状态：{url}"))?;
    if response
        .content_length()
        .is_some_and(|size| size > limit as u64)
    {
        bail!("下载内容超过 {} 字节限制", limit);
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("读取下载内容失败")?;
        if bytes.len().saturating_add(chunk.len()) > limit {
            bail!("下载内容超过 {} 字节限制", limit);
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.is_empty() {
        bail!("下载内容为空");
    }
    Ok(bytes)
}

fn cache_manifest(state_dir: &Path, manifest: &DreamSkinMarketManifest) -> anyhow::Result<()> {
    let bytes = serde_json::to_vec_pretty(manifest)?;
    crate::settings::atomic_write(&state_dir.join(MARKET_CACHE_FILE), &bytes)
}

fn read_cached_manifest(state_dir: &Path) -> anyhow::Result<DreamSkinMarketManifest> {
    let path = state_dir.join(MARKET_CACHE_FILE);
    let metadata = std::fs::metadata(&path)
        .with_context(|| format!("主题市场缓存不存在：{}", path.display()))?;
    if metadata.len() > MARKET_INDEX_LIMIT as u64 {
        bail!("主题市场缓存超过大小限制");
    }
    let mut manifest: DreamSkinMarketManifest =
        serde_json::from_slice(&std::fs::read(&path)?).context("主题市场缓存不是有效 JSON")?;
    validate_manifest(&mut manifest)?;
    Ok(manifest)
}

fn read_install_records(state_dir: &Path) -> anyhow::Result<MarketInstallRecords> {
    let path = state_dir.join(MARKET_INSTALLS_FILE);
    if !path.is_file() {
        return Ok(MarketInstallRecords {
            schema_version: market_schema_version(),
            themes: BTreeMap::new(),
        });
    }
    let records: MarketInstallRecords = serde_json::from_slice(&std::fs::read(path)?)?;
    if records.schema_version != market_schema_version() {
        bail!("不支持的主题市场安装记录版本");
    }
    Ok(records)
}

fn record_market_install(state_dir: &Path, id: &str, version: &str) -> anyhow::Result<()> {
    let mut records = read_install_records(state_dir)?;
    records.themes.insert(id.to_string(), version.to_string());
    let bytes = serde_json::to_vec_pretty(&records)?;
    crate::settings::atomic_write(&state_dir.join(MARKET_INSTALLS_FILE), &bytes)
}

fn market_download_path(state_dir: &Path, id: &str, extension: &str) -> anyhow::Result<PathBuf> {
    if !valid_theme_id(id) || !matches!(extension, "png" | "jpg" | "gif" | "bmp" | "webp") {
        bail!("无效的市场主题下载文件名");
    }
    Ok(state_dir
        .join("dream-skin/market/downloads")
        .join(format!("{id}.{extension}")))
}

fn market_asset_url(base_url: &str, relative: &str) -> anyhow::Result<String> {
    validate_market_path(relative)?;
    let base = reqwest::Url::parse(base_url).context("主题市场基础地址无效")?;
    let joined = base.join(relative).context("主题市场资源地址无效")?;
    if joined.scheme() != base.scheme() || joined.host_str() != base.host_str() {
        bail!("主题市场资源地址越界");
    }
    Ok(joined.to_string())
}

fn validate_market_path(value: &str) -> anyhow::Result<()> {
    if value.is_empty()
        || value.len() > 256
        || value.starts_with('/')
        || value.contains('\\')
        || value.contains(['?', '#', '\0'])
    {
        bail!("无效的主题市场相对路径");
    }
    for segment in value.split('/') {
        if segment.is_empty()
            || matches!(segment, "." | "..")
            || !segment
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            bail!("无效的主题市场相对路径");
        }
    }
    Ok(())
}

fn valid_theme_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    (1..=64).contains(&bytes.len())
        && bytes[0].is_ascii_alphanumeric()
        && bytes.iter().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        })
}

fn validate_sha256(value: &str) -> anyhow::Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("主题市场包含无效 SHA-256");
    }
    Ok(())
}

fn verify_sha256(bytes: &[u8], expected: &str, label: &str) -> anyhow::Result<()> {
    validate_sha256(expected)?;
    let actual = format!("{:x}", Sha256::digest(bytes));
    if actual != expected {
        bail!("{label} SHA-256 校验失败");
    }
    Ok(())
}

fn detect_image_extension(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("jpg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("gif")
    } else if bytes.starts_with(b"BM") {
        Some("bmp")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else {
        None
    }
}

const fn market_schema_version() -> u8 {
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn market_paths_reject_traversal_and_remote_urls() {
        for path in [
            "../theme.json",
            "themes\\x.png",
            "https://example.com/x",
            "/x",
        ] {
            assert!(validate_market_path(path).is_err(), "accepted {path}");
        }
        assert!(validate_market_path("themes/demo/theme.json").is_ok());
    }

    #[test]
    fn image_detection_uses_file_contents() {
        assert_eq!(
            detect_image_extension(b"\x89PNG\r\n\x1a\nrest"),
            Some("png")
        );
        assert_eq!(detect_image_extension(b"not-an-image"), None);
    }
}
