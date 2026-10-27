//! Agent 桌宠：内置图库目录、运行时联网取图与本地缓存。
//!
//! 图片一律在运行时下载，不打包进仓库或二进制：
//! - 内置图库来自微软 Fluent Emoji 3D（MIT），固定提交，经 jsDelivr 拉取，三个镜像依次尝试；
//! - 自定义图片只接受 https 地址。
//!
//! 下载结果按「地址的 sha256 + 扩展名」缓存在状态目录下的 `agent-pet-cache`，
//! 命中缓存时不再联网。

use std::io::Write;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, bail};
use base64::Engine;
use futures_util::StreamExt;
use sha2::{Digest, Sha256};

/// 默认图片 id。
pub const DEFAULT_IMAGE_ID: &str = "whale";
/// 选择自定义图片时的 id。
pub const CUSTOM_IMAGE_ID: &str = "custom";
/// Fluent Emoji 仓库的固定提交，保证图库内容不随上游变化。
pub const FLUENT_EMOJI_COMMIT: &str = "1ffb34c752ecf5d402f04cfb4b392c77f57c54bc";
/// jsDelivr 镜像，按顺序尝试。
pub const CDN_HOSTS: [&str; 3] = [
    "cdn.jsdelivr.net",
    "fastly.jsdelivr.net",
    "gcore.jsdelivr.net",
];
/// 单张图片大小上限。
pub const MAX_IMAGE_BYTES: usize = 4 * 1024 * 1024;
/// 最多跟随的重定向次数。
pub const MAX_REDIRECTS: usize = 3;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_CUSTOM_URL_LEN: usize = 2048;

/// 内置图库条目：id、中文名、Fluent Emoji 仓库 `assets/` 下的相对路径。
const BUILTIN_IMAGES: &[(&str, &str, &str)] = &[
    ("whale", "鲸鱼", "Spouting whale/3D/spouting_whale_3d.png"),
    ("cat", "小猫", "Cat/3D/cat_3d.png"),
    ("dog", "小狗", "Dog face/3D/dog_face_3d.png"),
    ("fox", "狐狸", "Fox/3D/fox_3d.png"),
    ("panda", "熊猫", "Panda/3D/panda_3d.png"),
    ("penguin", "企鹅", "Penguin/3D/penguin_3d.png"),
    ("hamster", "仓鼠", "Hamster/3D/hamster_3d.png"),
    ("rabbit", "兔子", "Rabbit face/3D/rabbit_face_3d.png"),
    ("frog", "青蛙", "Frog/3D/frog_3d.png"),
    ("octopus", "章鱼", "Octopus/3D/octopus_3d.png"),
    ("chick", "小鸡", "Baby chick/3D/baby_chick_3d.png"),
    ("duck", "鸭子", "Duck/3D/duck_3d.png"),
    ("dolphin", "海豚", "Dolphin/3D/dolphin_3d.png"),
    ("otter", "水獭", "Otter/3D/otter_3d.png"),
    ("unicorn", "独角兽", "Unicorn/3D/unicorn_3d.png"),
    ("dino", "恐龙", "Sauropod/3D/sauropod_3d.png"),
    ("robot", "机器人", "Robot/3D/robot_3d.png"),
    ("ghost", "小幽灵", "Ghost/3D/ghost_3d.png"),
];

/// 图库条目，序列化给前端。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPetCatalogEntry {
    pub id: String,
    pub name: String,
}

/// 取图结果：`data_uri` 可直接作为 `<img src>`。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPetImage {
    pub data_uri: String,
    pub source_url: String,
    pub cached: bool,
}

/// 内置图库目录（不含 `custom`）。
pub fn catalog() -> Vec<AgentPetCatalogEntry> {
    BUILTIN_IMAGES
        .iter()
        .map(|(id, name, _)| AgentPetCatalogEntry {
            id: (*id).to_string(),
            name: (*name).to_string(),
        })
        .collect()
}

pub fn is_builtin_image(id: &str) -> bool {
    builtin_path(id).is_some()
}

fn builtin_path(id: &str) -> Option<&'static str> {
    BUILTIN_IMAGES
        .iter()
        .find(|(item, _, _)| *item == id)
        .map(|(_, _, path)| *path)
}

/// 默认缓存目录：`<状态目录>/agent-pet-cache`。
pub fn default_cache_dir() -> PathBuf {
    crate::paths::default_app_state_dir().join("agent-pet-cache")
}

/// 设置里是否为指定 Agent 开启了桌宠。
pub fn enabled_for(settings: &crate::settings::AgentPetSettings, agent: &str) -> bool {
    settings.enabled_for(agent)
}

/// 对路径逐段做 URL 编码，保留 `/` 分隔符。
fn encode_path(path: &str) -> String {
    path.split('/')
        .map(|segment| {
            let mut encoded = String::with_capacity(segment.len());
            for byte in segment.bytes() {
                if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
                    encoded.push(byte as char);
                } else {
                    encoded.push_str(&format!("%{byte:02X}"));
                }
            }
            encoded
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// 内置图片在指定镜像上的地址。
pub fn builtin_image_url(id: &str, host: &str) -> Option<String> {
    let path = builtin_path(id)?;
    Some(format!(
        "https://{host}/gh/microsoft/fluentui-emoji@{FLUENT_EMOJI_COMMIT}/assets/{}",
        encode_path(path)
    ))
}

/// 内置图片在全部镜像上的地址，按尝试顺序排列。
pub fn builtin_image_urls(id: &str) -> Option<Vec<String>> {
    CDN_HOSTS
        .iter()
        .map(|host| builtin_image_url(id, host))
        .collect()
}

/// 校验自定义图片地址：只接受 https、不带用户名密码，且不指向本机或内网地址。
pub fn validate_custom_url(raw: &str) -> anyhow::Result<reqwest::Url> {
    let raw = raw.trim();
    if raw.is_empty() {
        bail!("还没有填写自定义图片地址");
    }
    if raw.len() > MAX_CUSTOM_URL_LEN {
        bail!("自定义图片地址过长");
    }
    let url = reqwest::Url::parse(raw).map_err(|error| anyhow::anyhow!("自定义图片地址无效：{error}"))?;
    if url.scheme() != "https" {
        bail!("自定义图片只支持 https:// 地址");
    }
    if !url.username().is_empty() || url.password().is_some() {
        bail!("自定义图片地址不能包含用户名或密码");
    }
    let Some(host) = url.host_str() else {
        bail!("自定义图片地址缺少主机名");
    };
    if is_local_host(host) {
        bail!("自定义图片地址不能指向本机或内网地址");
    }
    Ok(url)
}

fn is_local_host(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if host.eq_ignore_ascii_case("localhost") || host.to_ascii_lowercase().ends_with(".localhost") {
        return true;
    }
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => {
            ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_broadcast()
        }
        Ok(IpAddr::V6(ip)) => {
            let segments = ip.segments();
            ip.is_loopback()
                || ip.is_unspecified()
                // fc00::/7 唯一本地地址、fe80::/10 链路本地地址
                || (segments[0] & 0xfe00) == 0xfc00
                || (segments[0] & 0xffc0) == 0xfe80
                || ip.to_ipv4_mapped().is_some_and(|v4| {
                    v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified()
                })
        }
        Err(_) => false,
    }
}

/// 支持的图片类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageKind {
    Png,
    Jpeg,
    Webp,
    Gif,
}

impl ImageKind {
    const ALL: [ImageKind; 4] = [ImageKind::Png, ImageKind::Jpeg, ImageKind::Webp, ImageKind::Gif];

    pub fn mime(self) -> &'static str {
        match self {
            ImageKind::Png => "image/png",
            ImageKind::Jpeg => "image/jpeg",
            ImageKind::Webp => "image/webp",
            ImageKind::Gif => "image/gif",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            ImageKind::Png => "png",
            ImageKind::Jpeg => "jpg",
            ImageKind::Webp => "webp",
            ImageKind::Gif => "gif",
        }
    }

    /// 按响应头的 Content-Type 判断；忽略参数与大小写。
    pub fn from_content_type(value: &str) -> Option<Self> {
        let mime = value.split(';').next().unwrap_or_default().trim().to_ascii_lowercase();
        match mime.as_str() {
            "image/png" => Some(ImageKind::Png),
            "image/jpeg" | "image/jpg" => Some(ImageKind::Jpeg),
            "image/webp" => Some(ImageKind::Webp),
            "image/gif" => Some(ImageKind::Gif),
            _ => None,
        }
    }

    /// 按文件头魔数判断。
    pub fn from_magic(bytes: &[u8]) -> Option<Self> {
        if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
            Some(ImageKind::Png)
        } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            Some(ImageKind::Jpeg)
        } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
            Some(ImageKind::Gif)
        } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
            Some(ImageKind::Webp)
        } else {
            None
        }
    }
}

/// 缓存文件名：地址的 sha256（十六进制）+ 扩展名。
pub fn cache_file_name(url: &str, kind: ImageKind) -> String {
    format!("{:x}.{}", Sha256::digest(url.as_bytes()), kind.extension())
}

fn data_uri(kind: ImageKind, bytes: &[u8]) -> String {
    format!(
        "data:{};base64,{}",
        kind.mime(),
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

/// 读取缓存；文件损坏（超限或魔数不符）时删掉该文件，当作未命中。
fn read_cache(cache_dir: &Path, url: &str) -> Option<(ImageKind, Vec<u8>)> {
    for kind in ImageKind::ALL {
        let path = cache_dir.join(cache_file_name(url, kind));
        let Ok(metadata) = std::fs::metadata(&path) else {
            continue;
        };
        if metadata.is_file() && metadata.len() > 0 && metadata.len() <= MAX_IMAGE_BYTES as u64 {
            if let Ok(bytes) = std::fs::read(&path) {
                if ImageKind::from_magic(&bytes) == Some(kind) {
                    return Some((kind, bytes));
                }
            }
        }
        let _ = std::fs::remove_file(&path);
    }
    None
}

/// 写缓存：先写临时文件再改名，避免并发读到半截文件。
fn write_cache(cache_dir: &Path, url: &str, kind: ImageKind, bytes: &[u8]) -> anyhow::Result<()> {
    std::fs::create_dir_all(cache_dir)
        .with_context(|| format!("创建桌宠图片缓存目录失败：{}", cache_dir.display()))?;
    let name = cache_file_name(url, kind);
    let target = cache_dir.join(&name);
    let temp = cache_dir.join(format!(".{name}.{}.tmp", uuid::Uuid::new_v4().simple()));
    let result = (|| -> anyhow::Result<()> {
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temp, &target)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result.context("写入桌宠图片缓存失败")
}

/// 重定向检查：跳转次数有上限，且目标必须仍是 https。
fn check_redirect(next: &reqwest::Url, previous_hops: usize) -> Result<(), &'static str> {
    if previous_hops >= MAX_REDIRECTS {
        return Err("图片地址重定向次数过多");
    }
    if next.scheme() != "https" {
        return Err("图片地址重定向到了非 https 地址");
    }
    Ok(())
}

fn redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        match check_redirect(attempt.url(), attempt.previous().len()) {
            Ok(()) => attempt.follow(),
            Err(message) => attempt.error(message),
        }
    })
}

fn http_client(url: &str) -> anyhow::Result<reqwest::Client> {
    crate::http_client::proxied_client_for_url_with_redirect(
        &format!("CodexPlusPlus/{} AgentPet", env!("CARGO_PKG_VERSION")),
        url,
        CONNECT_TIMEOUT,
        FETCH_TIMEOUT,
        redirect_policy(),
    )
}

/// 下载一张图片：校验状态码、Content-Type、大小上限与文件头魔数。
async fn fetch_image(client: &reqwest::Client, url: &str) -> anyhow::Result<(ImageKind, Vec<u8>)> {
    let response = client
        .get(url)
        .header(
            reqwest::header::ACCEPT,
            "image/png,image/jpeg,image/webp,image/gif",
        )
        .send()
        .await
        .map_err(|error| {
            if error.is_timeout() {
                anyhow::anyhow!("请求超时")
            } else if error.is_redirect() {
                anyhow::anyhow!("重定向被拒绝：{error}")
            } else {
                anyhow::anyhow!("网络请求失败：{error}")
            }
        })?;
    let status = response.status();
    if !status.is_success() {
        bail!("图片服务返回 HTTP {}", status.as_u16());
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    if ImageKind::from_content_type(&content_type).is_none() {
        let shown = if content_type.is_empty() { "未知" } else { content_type.as_str() };
        bail!("图片类型不受支持：{shown}（只接受 PNG / JPEG / WebP / GIF）");
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_IMAGE_BYTES as u64)
    {
        bail!("图片超过 4 MiB 上限");
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| anyhow::anyhow!("读取图片数据失败：{error}"))?;
        if bytes.len().saturating_add(chunk.len()) > MAX_IMAGE_BYTES {
            bail!("图片超过 4 MiB 上限");
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.is_empty() {
        bail!("图片服务返回了空内容");
    }
    // 以文件头为准；响应头声明的类型只用来提前拒绝明显不是图片的内容。
    let Some(kind) = ImageKind::from_magic(&bytes) else {
        bail!("下载内容不是有效的 PNG / JPEG / WebP / GIF 图片");
    };
    Ok((kind, bytes))
}

/// 按设置取桌宠图片：先查缓存，未命中再联网下载并写入缓存。
///
/// - `image` 为内置图库 id 时，依次尝试 jsDelivr 的几个镜像；
/// - `image` 为 `custom` 时，使用 `custom_url`（只接受 https）。
pub async fn load_image(
    image: &str,
    custom_url: &str,
    cache_dir: &Path,
) -> anyhow::Result<AgentPetImage> {
    let image = image.trim();
    let sources = if image == CUSTOM_IMAGE_ID {
        vec![validate_custom_url(custom_url)?.to_string()]
    } else if let Some(urls) = builtin_image_urls(image) {
        urls
    } else {
        bail!("未知的内置桌宠图片：{image}");
    };
    // 内置图片的多个镜像共用一份缓存，键取第一个镜像的地址。
    let cache_url = sources[0].clone();
    if let Some((kind, bytes)) = read_cache(cache_dir, &cache_url) {
        return Ok(AgentPetImage {
            data_uri: data_uri(kind, &bytes),
            source_url: cache_url,
            cached: true,
        });
    }

    let mut errors = Vec::new();
    for url in &sources {
        let host = reqwest::Url::parse(url)
            .ok()
            .and_then(|parsed| parsed.host_str().map(str::to_string))
            .unwrap_or_default();
        let result = async {
            let client = http_client(url)?;
            fetch_image(&client, url).await
        }
        .await;
        match result {
            Ok((kind, bytes)) => {
                // 缓存写失败不影响本次显示，下次会重新下载。
                let _ = write_cache(cache_dir, &cache_url, kind, &bytes);
                return Ok(AgentPetImage {
                    data_uri: data_uri(kind, &bytes),
                    source_url: url.clone(),
                    cached: false,
                });
            }
            Err(error) => errors.push(format!("{host}：{error:#}")),
        }
    }
    if image == CUSTOM_IMAGE_ID {
        bail!("自定义桌宠图片下载失败（{}）", errors.join("；"));
    }
    bail!("桌宠图片下载失败，已尝试全部镜像（{}）", errors.join("；"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13];

    #[test]
    fn catalog_lists_builtin_images_in_order() {
        let entries = catalog();
        assert_eq!(entries.len(), 18);
        assert_eq!(entries[0], AgentPetCatalogEntry { id: "whale".into(), name: "鲸鱼".into() });
        assert!(entries.iter().any(|entry| entry.id == "ghost" && entry.name == "小幽灵"));
        assert!(!entries.iter().any(|entry| entry.id == CUSTOM_IMAGE_ID));
        let json = serde_json::to_value(&entries[0]).unwrap();
        assert_eq!(json, serde_json::json!({"id": "whale", "name": "鲸鱼"}));
        assert!(is_builtin_image(DEFAULT_IMAGE_ID));
        assert!(!is_builtin_image("custom"));
    }

    #[test]
    fn builtin_urls_encode_path_and_try_every_mirror() {
        let urls = builtin_image_urls("whale").unwrap();
        assert_eq!(
            urls,
            vec![
                "https://cdn.jsdelivr.net/gh/microsoft/fluentui-emoji@1ffb34c752ecf5d402f04cfb4b392c77f57c54bc/assets/Spouting%20whale/3D/spouting_whale_3d.png",
                "https://fastly.jsdelivr.net/gh/microsoft/fluentui-emoji@1ffb34c752ecf5d402f04cfb4b392c77f57c54bc/assets/Spouting%20whale/3D/spouting_whale_3d.png",
                "https://gcore.jsdelivr.net/gh/microsoft/fluentui-emoji@1ffb34c752ecf5d402f04cfb4b392c77f57c54bc/assets/Spouting%20whale/3D/spouting_whale_3d.png",
            ]
        );
        assert!(builtin_image_urls("nope").is_none());
        assert_eq!(encode_path("Baby chick/3D/a+b(1).png"), "Baby%20chick/3D/a%2Bb%281%29.png");
    }

    #[test]
    fn custom_url_requires_plain_public_https() {
        assert_eq!(
            validate_custom_url(" https://example.com/pet.png ").unwrap().as_str(),
            "https://example.com/pet.png"
        );
        for (raw, needle) in [
            ("", "还没有填写"),
            ("http://example.com/pet.png", "https://"),
            ("ftp://example.com/pet.png", "https://"),
            ("data:image/png;base64,AAAA", "https://"),
            ("https://user:pass@example.com/pet.png", "用户名或密码"),
            ("https://user@example.com/pet.png", "用户名或密码"),
            ("https://localhost/pet.png", "本机或内网"),
            ("https://127.0.0.1/pet.png", "本机或内网"),
            ("https://192.168.1.2/pet.png", "本机或内网"),
            ("https://[::1]/pet.png", "本机或内网"),
            ("not a url", "无效"),
        ] {
            let error = validate_custom_url(raw).unwrap_err().to_string();
            assert!(error.contains(needle), "{raw}: {error}");
        }
        let long = format!("https://example.com/{}", "a".repeat(MAX_CUSTOM_URL_LEN));
        assert!(validate_custom_url(&long).is_err());
    }

    #[test]
    fn image_kind_checks_content_type_and_magic() {
        assert_eq!(ImageKind::from_content_type("image/png"), Some(ImageKind::Png));
        assert_eq!(ImageKind::from_content_type("IMAGE/JPEG; charset=binary"), Some(ImageKind::Jpeg));
        assert_eq!(ImageKind::from_content_type("image/webp"), Some(ImageKind::Webp));
        assert_eq!(ImageKind::from_content_type("image/gif"), Some(ImageKind::Gif));
        assert_eq!(ImageKind::from_content_type("image/svg+xml"), None);
        assert_eq!(ImageKind::from_content_type("text/html"), None);
        assert_eq!(ImageKind::from_content_type(""), None);

        assert_eq!(ImageKind::from_magic(PNG), Some(ImageKind::Png));
        assert_eq!(ImageKind::from_magic(&[0xFF, 0xD8, 0xFF, 0xE0]), Some(ImageKind::Jpeg));
        assert_eq!(ImageKind::from_magic(b"GIF89a...."), Some(ImageKind::Gif));
        assert_eq!(ImageKind::from_magic(b"GIF87a...."), Some(ImageKind::Gif));
        assert_eq!(ImageKind::from_magic(b"RIFF\x10\0\0\0WEBPVP8 "), Some(ImageKind::Webp));
        assert_eq!(ImageKind::from_magic(b"RIFF\x10\0\0\0WAVEfmt "), None);
        assert_eq!(ImageKind::from_magic(b"<svg xmlns="), None);
        assert_eq!(ImageKind::from_magic(&[]), None);
    }

    #[test]
    fn cache_key_is_url_sha256_with_extension() {
        let name = cache_file_name("https://example.com/a.png", ImageKind::Png);
        assert_eq!(name.len(), 64 + 4);
        assert!(name.ends_with(".png"));
        assert_eq!(name, cache_file_name("https://example.com/a.png", ImageKind::Png));
        assert_ne!(name, cache_file_name("https://example.com/b.png", ImageKind::Png));
        assert!(cache_file_name("https://example.com/a", ImageKind::Jpeg).ends_with(".jpg"));
    }

    #[test]
    fn redirect_must_stay_https_and_bounded() {
        let https = reqwest::Url::parse("https://example.com/a.png").unwrap();
        let http = reqwest::Url::parse("http://example.com/a.png").unwrap();
        assert!(check_redirect(&https, 0).is_ok());
        assert!(check_redirect(&https, MAX_REDIRECTS - 1).is_ok());
        assert!(check_redirect(&https, MAX_REDIRECTS).is_err());
        assert!(check_redirect(&http, 0).is_err());
    }

    #[tokio::test]
    async fn builtin_image_served_from_cache_without_network() {
        let dir = tempfile::tempdir().unwrap();
        let url = builtin_image_url("cat", CDN_HOSTS[0]).unwrap();
        write_cache(dir.path(), &url, ImageKind::Png, PNG).unwrap();
        let image = load_image("cat", "", dir.path()).await.unwrap();
        assert!(image.cached);
        assert_eq!(image.source_url, url);
        assert!(image.data_uri.starts_with("data:image/png;base64,"));
    }

    #[tokio::test]
    async fn corrupted_cache_entry_is_discarded() {
        let dir = tempfile::tempdir().unwrap();
        let url = "https://example.com/pet.png";
        let path = dir.path().join(cache_file_name(url, ImageKind::Png));
        std::fs::write(&path, b"<html>not an image</html>").unwrap();
        assert!(read_cache(dir.path(), url).is_none());
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn invalid_choices_fail_before_network() {
        let dir = tempfile::tempdir().unwrap();
        let error = load_image("custom", "http://example.com/a.png", dir.path())
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("https://"), "{error}");
        let error = load_image("nope", "", dir.path()).await.unwrap_err().to_string();
        assert!(error.contains("未知的内置桌宠图片"), "{error}");
    }

    // 以下用本机回环的模拟服务验证下载校验，不访问外网。
    mod download {
        use super::*;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        fn local_client() -> reqwest::Client {
            reqwest::Client::builder().no_proxy().build().unwrap()
        }

        async fn serve(content_type: &str, body: Vec<u8>) -> MockServer {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/pet"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .insert_header("content-type", content_type)
                        .set_body_bytes(body),
                )
                .mount(&server)
                .await;
            server
        }

        #[tokio::test]
        async fn accepts_matching_image() {
            let server = serve("image/png", PNG.to_vec()).await;
            let (kind, bytes) = fetch_image(&local_client(), &format!("{}/pet", server.uri()))
                .await
                .unwrap();
            assert_eq!(kind, ImageKind::Png);
            assert_eq!(bytes, PNG);
        }

        #[tokio::test]
        async fn rejects_wrong_content_type() {
            let server = serve("text/html", PNG.to_vec()).await;
            let error = fetch_image(&local_client(), &format!("{}/pet", server.uri()))
                .await
                .unwrap_err()
                .to_string();
            assert!(error.contains("图片类型不受支持"), "{error}");
        }

        #[tokio::test]
        async fn rejects_bytes_that_are_not_an_image() {
            let server = serve("image/png", b"<svg></svg>".to_vec()).await;
            let error = fetch_image(&local_client(), &format!("{}/pet", server.uri()))
                .await
                .unwrap_err()
                .to_string();
            assert!(error.contains("不是有效的"), "{error}");
        }

        #[tokio::test]
        async fn rejects_oversized_image() {
            let mut body = PNG.to_vec();
            body.resize(MAX_IMAGE_BYTES + 1, 0);
            let server = serve("image/png", body).await;
            let error = fetch_image(&local_client(), &format!("{}/pet", server.uri()))
                .await
                .unwrap_err()
                .to_string();
            assert!(error.contains("4 MiB"), "{error}");
        }

        #[tokio::test]
        async fn reports_http_status() {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(404))
                .mount(&server)
                .await;
            let error = fetch_image(&local_client(), &format!("{}/pet", server.uri()))
                .await
                .unwrap_err()
                .to_string();
            assert!(error.contains("HTTP 404"), "{error}");
        }
    }
}
