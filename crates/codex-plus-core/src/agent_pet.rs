//! Agent 桌宠：内置图库目录、运行时联网取图与本地缓存。
//!
//! 图片一律在运行时下载，不打包进仓库或二进制：
//! - 内置图库来自微软 Fluent Emoji 3D（MIT），固定提交，经 jsDelivr 拉取，三个镜像依次尝试；
//! - 二次元图库来自 Wikimedia Commons 上的 CC0 作品，地址没有提交可固定，按登记的 sha256 校验内容；
//! - 自定义图片只接受 https 地址。
//!
//! 音效有三种来源：随启动器内嵌的两组（见 `assets`）、运行时下载的音效组、
//! 用户导入的本地音频（复制到状态目录下的 `agent-pet-custom`）。
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
/// 单个音效大小上限（下载与导入共用）。
pub const MAX_SOUND_BYTES: usize = 2 * 1024 * 1024;
/// 选择自定义音效时的音效组 id。
pub const CUSTOM_SOUND_ID: &str = "custom";
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

/// 二次元图库：直链 + 内容 sha256。Commons 上同名文件可能被覆盖上传，哈希不符就拒绝使用。
struct AnimeImage {
    id: &'static str,
    name: &'static str,
    url: &'static str,
    sha256: &'static str,
    author: &'static str,
    license: &'static str,
}

const ANIME_IMAGES: &[AnimeImage] = &[
    AnimeImage {
        id: "chibi-bow",
        name: "蝴蝶结",
        url: "https://upload.wikimedia.org/wikipedia/commons/c/c1/Chibiw.png",
        sha256: "62d1e3f2cce8614b98d38816285befd5c6f2a187c41312c3324698a10d332cea",
        author: "asrafil",
        license: "CC0",
    },
    AnimeImage {
        id: "chibi-girl",
        name: "Q版少女",
        url: "https://upload.wikimedia.org/wikipedia/commons/4/4c/Cute_girl.png",
        sha256: "93edfe9b7223589f026472ba46d2e8cc533a14771ae944b015610bccf8339f1d",
        author: "dcatcherex",
        license: "CC0",
    },
    AnimeImage {
        id: "anime-girl",
        name: "马尾少女",
        url: "https://upload.wikimedia.org/wikipedia/commons/e/e7/Anime_girl_publicdomainq.png",
        sha256: "d51fea3bef4cce76b07755a67de454a288be47d81a6fb87f29723ccbf8a765e2",
        author: "oksmith",
        license: "CC0",
    },
    AnimeImage {
        id: "anime-angry",
        name: "炸毛少女",
        url: "https://upload.wikimedia.org/wikipedia/commons/c/c5/Angry_anime_girl.png",
        sha256: "509cd653351f0641b05420351d37e1a48314f1c329ebd99f6c302a8057e094f2",
        author: "snoopingasusual",
        license: "CC0",
    },
];

/// 图库条目，序列化给前端。二次元图库带署名（作者 · 授权）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPetCatalogEntry {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit: Option<String>,
}

/// 取图结果：`data_uri` 可直接作为 `<img src>`。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPetImage {
    pub data_uri: String,
    pub source_url: String,
    pub cached: bool,
}

/// 内置图库目录（不含 `custom`）：Fluent Emoji 在前，二次元在后。
pub fn catalog() -> Vec<AgentPetCatalogEntry> {
    let fluent = BUILTIN_IMAGES.iter().map(|(id, name, _)| AgentPetCatalogEntry {
        id: (*id).to_string(),
        name: (*name).to_string(),
        credit: None,
    });
    let anime = ANIME_IMAGES.iter().map(|image| AgentPetCatalogEntry {
        id: image.id.to_string(),
        name: image.name.to_string(),
        credit: Some(format!("{} · {}", image.author, image.license)),
    });
    fluent.chain(anime).collect()
}

pub fn is_builtin_image(id: &str) -> bool {
    builtin_path(id).is_some() || anime_image(id).is_some()
}

fn anime_image(id: &str) -> Option<&'static AnimeImage> {
    ANIME_IMAGES.iter().find(|image| image.id == id)
}

fn builtin_path(id: &str) -> Option<&'static str> {
    BUILTIN_IMAGES
        .iter()
        .find(|(item, _, _)| *item == id)
        .map(|(_, _, path)| *path)
}

/// 可下载音效组：按压音必有，松开音可选。地址固定，运行时下载后缓存。
struct DownloadSoundSet {
    id: &'static str,
    name: &'static str,
    press: &'static str,
    release: Option<&'static str>,
    author: &'static str,
    license: &'static str,
    source: &'static str,
}

/// Freesound 上的 CC0 音效，取其公开预览直链（MP3）。
const DOWNLOAD_SOUNDS: &[DownloadSoundSet] = &[
    DownloadSoundSet {
        id: "nya",
        name: "猫娘「にゃ」",
        press: "https://cdn.freesound.org/previews/336/336012_5953264-hq.mp3",
        release: None,
        author: "Mike_bes",
        license: "CC0",
        source: "https://freesound.org/people/Mike_bes/sounds/336012/",
    },
    DownloadSoundSet {
        id: "hai",
        name: "元气「はい」",
        press: "https://cdn.freesound.org/previews/323/323194_4347097-hq.mp3",
        release: Some("https://cdn.freesound.org/previews/323/323196_4347097-hq.mp3"),
        author: "Alivvie",
        license: "CC0",
        source: "https://freesound.org/people/Alivvie/sounds/323194/",
    },
    DownloadSoundSet {
        id: "yatta",
        name: "欢呼「やった」",
        press: "https://cdn.freesound.org/previews/235/235085_950925-hq.mp3",
        release: Some("https://cdn.freesound.org/previews/235/235093_950925-hq.mp3"),
        author: "Reitanna",
        license: "CC0",
        source: "https://freesound.org/people/Reitanna/sounds/235085/",
    },
    DownloadSoundSet {
        id: "ehehe",
        name: "轻笑「えへへ」",
        press: "https://cdn.freesound.org/previews/343/343992_950925-hq.mp3",
        release: Some("https://cdn.freesound.org/previews/343/343984_950925-hq.mp3"),
        author: "Reitanna",
        license: "CC0",
        source: "https://freesound.org/people/Reitanna/sounds/343992/",
    },
    DownloadSoundSet {
        id: "pop",
        name: "啵唧",
        press: "https://cdn.freesound.org/previews/789/789793_6368615-hq.mp3",
        release: Some("https://cdn.freesound.org/previews/857/857748_71257-hq.mp3"),
        author: "quatricise / qubodup",
        license: "CC0",
        source: "https://freesound.org/people/quatricise/sounds/789793/",
    },
    DownloadSoundSet {
        id: "sparkle",
        name: "魔法闪光",
        press: "https://cdn.freesound.org/previews/618/618961_6135165-hq.mp3",
        release: Some("https://cdn.freesound.org/previews/511/511485_6890478-hq.mp3"),
        author: "Hemplock / MLaudio",
        license: "CC0",
        source: "https://freesound.org/people/Hemplock/sounds/618961/",
    },
];

/// 可下载音效组的目录条目，序列化给前端（含署名信息）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPetSoundEntry {
    pub id: String,
    pub name: String,
    pub author: String,
    pub license: String,
    pub source_url: String,
}

/// 可下载音效组目录（不含内嵌的两组和 `custom`）。
pub fn sound_catalog() -> Vec<AgentPetSoundEntry> {
    DOWNLOAD_SOUNDS
        .iter()
        .map(|set| AgentPetSoundEntry {
            id: set.id.to_string(),
            name: set.name.to_string(),
            author: set.author.to_string(),
            license: set.license.to_string(),
            source_url: set.source.to_string(),
        })
        .collect()
}

pub fn is_download_sound(id: &str) -> bool {
    DOWNLOAD_SOUNDS.iter().any(|set| set.id == id)
}

/// 一组音效的 data URI；`key` 随来源变化（自定义音效含文件名），渲染进程按它缓存解码结果。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPetSound {
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub press: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release: Option<String>,
}

/// 自定义音效的缓存键：文件名带内容哈希，换了文件键就变。
pub fn custom_sound_key(press: &str, release: &str) -> String {
    format!("{CUSTOM_SOUND_ID}:{press}:{release}")
}

/// 导入的自定义音效目录：`<状态目录>/agent-pet-custom`。
pub fn default_custom_dir() -> PathBuf {
    crate::paths::default_app_state_dir().join("agent-pet-custom")
}

/// 自定义音效槽位：按压 / 松开。
pub const CUSTOM_SOUND_SLOTS: [&str; 2] = ["press", "release"];

/// 导入文件名形如 `press-<16 位十六进制>.mp3`；只认这个形状，设置里的值不能拼出别的路径。
pub fn is_custom_sound_file(name: &str, slot: &str) -> bool {
    let Some(rest) = name.strip_prefix(slot).and_then(|rest| rest.strip_prefix('-')) else {
        return false;
    };
    let Some((hash, extension)) = rest.split_once('.') else {
        return false;
    };
    hash.len() == 16
        && hash.bytes().all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        && AudioKind::ALL.iter().any(|kind| kind.extension() == extension)
}

/// 导入一段本地音频作为自定义按压 / 松开音，返回保存后的文件名（写进设置）。
/// 同一槽位之前导入的文件会被替换掉。
pub fn import_custom_sound(slot: &str, bytes: &[u8], custom_dir: &Path) -> anyhow::Result<String> {
    if !CUSTOM_SOUND_SLOTS.contains(&slot) {
        bail!("未知的音效槽位：{slot}");
    }
    if bytes.is_empty() {
        bail!("音频文件是空的");
    }
    if bytes.len() > MAX_SOUND_BYTES {
        bail!("音频文件超过 {} 上限", size_label(MAX_SOUND_BYTES));
    }
    let Some(kind) = AudioKind::from_magic(bytes) else {
        bail!("不是有效的 {} 音频文件", AudioKind::FORMATS);
    };
    let hash = format!("{:x}", Sha256::digest(bytes));
    let name = format!("{slot}-{}.{}", &hash[..16], kind.extension());
    write_atomic(custom_dir, &name, bytes).context("保存自定义音效失败")?;
    // 清掉同槽位的旧文件，只动符合命名规则的文件。
    if let Ok(entries) = std::fs::read_dir(custom_dir) {
        for entry in entries.flatten() {
            let file = entry.file_name().to_string_lossy().to_string();
            if file != name && is_custom_sound_file(&file, slot) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    Ok(name)
}

fn read_custom_sound(custom_dir: &Path, name: &str, slot: &str) -> anyhow::Result<Option<String>> {
    if name.is_empty() {
        return Ok(None);
    }
    if !is_custom_sound_file(name, slot) {
        bail!("自定义音效文件名无效：{name}");
    }
    let bytes = std::fs::read(custom_dir.join(name))
        .with_context(|| format!("读取自定义音效失败：{name}，请重新选择文件"))?;
    let Some(kind) = AudioKind::from_magic(&bytes).filter(|_| bytes.len() <= MAX_SOUND_BYTES) else {
        bail!("自定义音效文件已损坏：{name}，请重新选择文件");
    };
    Ok(Some(data_uri(kind, &bytes)))
}

/// 按设置取一组音效：内嵌组直接给，下载组先查缓存再联网，自定义组读导入的文件。
pub async fn load_sound(
    sound_set: &str,
    custom_press: &str,
    custom_release: &str,
    cache_dir: &Path,
    custom_dir: &Path,
) -> anyhow::Result<AgentPetSound> {
    let sound_set = sound_set.trim();
    if sound_set == CUSTOM_SOUND_ID {
        let press = read_custom_sound(custom_dir, custom_press, "press")?;
        let release = read_custom_sound(custom_dir, custom_release, "release")?;
        if press.is_none() && release.is_none() {
            bail!("还没有选择自定义音效文件");
        }
        return Ok(AgentPetSound {
            key: custom_sound_key(custom_press, custom_release),
            press,
            release,
        });
    }
    if crate::settings::AGENT_PET_SOUND_SETS.contains(&sound_set) {
        let embedded = crate::assets::agent_pet_sounds();
        let slot = |name: &str| embedded[sound_set][name].as_str().map(str::to_string);
        return Ok(AgentPetSound {
            key: sound_set.to_string(),
            press: slot("press"),
            release: slot("release"),
        });
    }
    let Some(set) = DOWNLOAD_SOUNDS.iter().find(|set| set.id == sound_set) else {
        bail!("未知的桌宠音效：{sound_set}");
    };
    let download = |url: &'static str| async move {
        load_media::<AudioKind>(&[url.to_string()], None, cache_dir)
            .await
            .map(|(data_uri, _, _)| data_uri)
            .map_err(|errors| anyhow::anyhow!("桌宠音效下载失败（{}）", errors.join("；")))
    };
    let press = download(set.press).await?;
    let release = match set.release {
        Some(url) => Some(download(url).await?),
        None => None,
    };
    Ok(AgentPetSound {
        key: sound_set.to_string(),
        press: Some(press),
        release,
    })
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

/// 内置图片在全部镜像上的地址，按尝试顺序排列；二次元图库只有一个直链。
pub fn builtin_image_urls(id: &str) -> Option<Vec<String>> {
    if let Some(image) = anime_image(id) {
        return Some(vec![image.url.to_string()]);
    }
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

/// 可下载素材的类型（图片 / 音频）：按响应头和文件头双重判断。
pub trait MediaKind: Copy + PartialEq + Sized + 'static {
    /// 全部取值，读缓存时逐个尝试扩展名。
    const ALL: &'static [Self];
    /// 用在错误信息里的名称，如「图片」「音效」。
    const LABEL: &'static str;
    /// 用在错误信息里的格式列表。
    const FORMATS: &'static str;
    const ACCEPT: &'static str;
    const MAX_BYTES: usize;
    fn mime(self) -> &'static str;
    fn extension(self) -> &'static str;
    /// 按响应头的 Content-Type 判断；忽略参数与大小写。
    fn from_content_type(value: &str) -> Option<Self>;
    /// 按文件头魔数判断。
    fn from_magic(bytes: &[u8]) -> Option<Self>;
}

fn bare_mime(value: &str) -> String {
    value.split(';').next().unwrap_or_default().trim().to_ascii_lowercase()
}

fn size_label(bytes: usize) -> String {
    format!("{} MiB", bytes / 1024 / 1024)
}

/// 支持的图片类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageKind {
    Png,
    Jpeg,
    Webp,
    Gif,
}

impl MediaKind for ImageKind {
    const ALL: &'static [ImageKind] = &[ImageKind::Png, ImageKind::Jpeg, ImageKind::Webp, ImageKind::Gif];
    const LABEL: &'static str = "图片";
    const FORMATS: &'static str = "PNG / JPEG / WebP / GIF";
    const ACCEPT: &'static str = "image/png,image/jpeg,image/webp,image/gif";
    const MAX_BYTES: usize = MAX_IMAGE_BYTES;

    fn mime(self) -> &'static str {
        match self {
            ImageKind::Png => "image/png",
            ImageKind::Jpeg => "image/jpeg",
            ImageKind::Webp => "image/webp",
            ImageKind::Gif => "image/gif",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            ImageKind::Png => "png",
            ImageKind::Jpeg => "jpg",
            ImageKind::Webp => "webp",
            ImageKind::Gif => "gif",
        }
    }

    fn from_content_type(value: &str) -> Option<Self> {
        match bare_mime(value).as_str() {
            "image/png" => Some(ImageKind::Png),
            "image/jpeg" | "image/jpg" => Some(ImageKind::Jpeg),
            "image/webp" => Some(ImageKind::Webp),
            "image/gif" => Some(ImageKind::Gif),
            _ => None,
        }
    }

    fn from_magic(bytes: &[u8]) -> Option<Self> {
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

/// 支持的音频类型（WebView 的 decodeAudioData 都能解）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioKind {
    Mp3,
    Ogg,
    Wav,
    M4a,
}

impl MediaKind for AudioKind {
    const ALL: &'static [AudioKind] = &[AudioKind::Mp3, AudioKind::Ogg, AudioKind::Wav, AudioKind::M4a];
    const LABEL: &'static str = "音效";
    const FORMATS: &'static str = "MP3 / OGG / WAV / M4A";
    const ACCEPT: &'static str = "audio/mpeg,audio/ogg,audio/wav,audio/mp4";
    const MAX_BYTES: usize = MAX_SOUND_BYTES;

    fn mime(self) -> &'static str {
        match self {
            AudioKind::Mp3 => "audio/mpeg",
            AudioKind::Ogg => "audio/ogg",
            AudioKind::Wav => "audio/wav",
            AudioKind::M4a => "audio/mp4",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            AudioKind::Mp3 => "mp3",
            AudioKind::Ogg => "ogg",
            AudioKind::Wav => "wav",
            AudioKind::M4a => "m4a",
        }
    }

    fn from_content_type(value: &str) -> Option<Self> {
        match bare_mime(value).as_str() {
            "audio/mpeg" | "audio/mp3" => Some(AudioKind::Mp3),
            "audio/ogg" | "application/ogg" | "audio/opus" => Some(AudioKind::Ogg),
            "audio/wav" | "audio/x-wav" | "audio/wave" | "audio/vnd.wave" => Some(AudioKind::Wav),
            "audio/mp4" | "audio/x-m4a" | "audio/m4a" | "audio/aac" => Some(AudioKind::M4a),
            _ => None,
        }
    }

    fn from_magic(bytes: &[u8]) -> Option<Self> {
        if bytes.starts_with(b"OggS") {
            Some(AudioKind::Ogg)
        } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
            Some(AudioKind::Wav)
        } else if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
            Some(AudioKind::M4a)
        } else if bytes.starts_with(b"ID3") || (bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] & 0xE0 == 0xE0) {
            // ID3 标签开头，或直接是 MPEG 帧同步字。
            Some(AudioKind::Mp3)
        } else {
            None
        }
    }
}

/// 缓存文件名：地址的 sha256（十六进制）+ 扩展名。
pub fn cache_file_name<K: MediaKind>(url: &str, kind: K) -> String {
    format!("{:x}.{}", Sha256::digest(url.as_bytes()), kind.extension())
}

fn data_uri<K: MediaKind>(kind: K, bytes: &[u8]) -> String {
    format!(
        "data:{};base64,{}",
        kind.mime(),
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

/// 读取缓存；文件损坏（超限或魔数不符）时删掉该文件，当作未命中。
fn read_cache<K: MediaKind>(cache_dir: &Path, url: &str) -> Option<(K, Vec<u8>)> {
    for &kind in K::ALL {
        let path = cache_dir.join(cache_file_name(url, kind));
        let Ok(metadata) = std::fs::metadata(&path) else {
            continue;
        };
        if metadata.is_file() && metadata.len() > 0 && metadata.len() <= K::MAX_BYTES as u64 {
            if let Ok(bytes) = std::fs::read(&path) {
                if K::from_magic(&bytes) == Some(kind) {
                    return Some((kind, bytes));
                }
            }
        }
        let _ = std::fs::remove_file(&path);
    }
    None
}

/// 先写临时文件再改名，避免并发读到半截文件。
fn write_atomic(dir: &Path, name: &str, bytes: &[u8]) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)
        .with_context(|| format!("创建桌宠素材目录失败：{}", dir.display()))?;
    let target = dir.join(name);
    let temp = dir.join(format!(".{name}.{}.tmp", uuid::Uuid::new_v4().simple()));
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
    result
}

fn write_cache<K: MediaKind>(cache_dir: &Path, url: &str, kind: K, bytes: &[u8]) -> anyhow::Result<()> {
    write_atomic(cache_dir, &cache_file_name(url, kind), bytes)
        .with_context(|| format!("写入桌宠{}缓存失败", K::LABEL))
}

/// 重定向检查：跳转次数有上限，且目标必须仍是 https。
fn check_redirect(next: &reqwest::Url, previous_hops: usize) -> Result<(), &'static str> {
    if previous_hops >= MAX_REDIRECTS {
        return Err("素材地址重定向次数过多");
    }
    if next.scheme() != "https" {
        return Err("素材地址重定向到了非 https 地址");
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

/// 下载一个素材：校验状态码、Content-Type、大小上限与文件头魔数。
async fn fetch_media<K: MediaKind>(client: &reqwest::Client, url: &str) -> anyhow::Result<(K, Vec<u8>)> {
    let label = K::LABEL;
    let limit = size_label(K::MAX_BYTES);
    let response = client
        .get(url)
        .header(reqwest::header::ACCEPT, K::ACCEPT)
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
        bail!("{label}服务返回 HTTP {}", status.as_u16());
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    if K::from_content_type(&content_type).is_none() {
        let shown = if content_type.is_empty() { "未知" } else { content_type.as_str() };
        bail!("{label}类型不受支持：{shown}（只接受 {}）", K::FORMATS);
    }
    if response
        .content_length()
        .is_some_and(|length| length > K::MAX_BYTES as u64)
    {
        bail!("{label}超过 {limit} 上限");
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| anyhow::anyhow!("读取{label}数据失败：{error}"))?;
        if bytes.len().saturating_add(chunk.len()) > K::MAX_BYTES {
            bail!("{label}超过 {limit} 上限");
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.is_empty() {
        bail!("{label}服务返回了空内容");
    }
    // 以文件头为准；响应头声明的类型只用来提前拒绝明显不对的内容。
    let Some(kind) = K::from_magic(&bytes) else {
        bail!("下载内容不是有效的 {} {label}", K::FORMATS);
    };
    Ok((kind, bytes))
}

#[cfg(test)]
async fn fetch_image(client: &reqwest::Client, url: &str) -> anyhow::Result<(ImageKind, Vec<u8>)> {
    fetch_media(client, url).await
}

/// 先查缓存，未命中再按顺序尝试各个地址，成功后写缓存。返回 (data URI, 实际地址, 是否命中缓存)。
/// 多个镜像共用一份缓存，键取第一个地址。
async fn load_media<K: MediaKind>(
    sources: &[String],
    expected_sha256: Option<&str>,
    cache_dir: &Path,
) -> Result<(String, String, bool), Vec<String>> {
    let cache_url = sources[0].clone();
    if let Some((kind, bytes)) = read_cache::<K>(cache_dir, &cache_url) {
        return Ok((data_uri(kind, &bytes), cache_url, true));
    }
    let mut errors = Vec::new();
    for url in sources {
        let host = reqwest::Url::parse(url)
            .ok()
            .and_then(|parsed| parsed.host_str().map(str::to_string))
            .unwrap_or_default();
        let result = async {
            let client = http_client(url)?;
            let (kind, bytes) = fetch_media::<K>(&client, url).await?;
            if let Some(expected) = expected_sha256 {
                if format!("{:x}", Sha256::digest(&bytes)) != expected {
                    bail!("内容与登记的不一致，上游文件可能被替换");
                }
            }
            Ok((kind, bytes))
        }
        .await;
        match result {
            Ok((kind, bytes)) => {
                // 缓存写失败不影响本次使用，下次会重新下载。
                let _ = write_cache(cache_dir, &cache_url, kind, &bytes);
                return Ok((data_uri(kind, &bytes), url.clone(), false));
            }
            Err(error) => errors.push(format!("{host}：{error:#}")),
        }
    }
    Err(errors)
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
    let expected = anime_image(image).map(|item| item.sha256);
    match load_media::<ImageKind>(&sources, expected, cache_dir).await {
        Ok((data_uri, source_url, cached)) => Ok(AgentPetImage {
            data_uri,
            source_url,
            cached,
        }),
        Err(errors) if image == CUSTOM_IMAGE_ID => {
            bail!("自定义桌宠图片下载失败（{}）", errors.join("；"))
        }
        Err(errors) => bail!("桌宠图片下载失败，已尝试全部镜像（{}）", errors.join("；")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13];

    #[test]
    fn catalog_lists_builtin_images_in_order() {
        let entries = catalog();
        assert_eq!(entries.len(), 18 + ANIME_IMAGES.len());
        assert_eq!(
            entries[0],
            AgentPetCatalogEntry { id: "whale".into(), name: "鲸鱼".into(), credit: None }
        );
        let anime = entries.iter().find(|entry| entry.id == "chibi-bow").unwrap();
        assert_eq!(anime.credit.as_deref(), Some("asrafil · CC0"));
        assert!(is_builtin_image("anime-girl"));
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
        assert_eq!(
            builtin_image_urls("chibi-girl").unwrap(),
            vec!["https://upload.wikimedia.org/wikipedia/commons/4/4c/Cute_girl.png"]
        );
        for image in ANIME_IMAGES {
            assert!(image.url.starts_with("https://upload.wikimedia.org/"), "{}", image.id);
            assert_eq!(image.sha256.len(), 64, "{}", image.id);
            assert!(!BUILTIN_IMAGES.iter().any(|(id, _, _)| *id == image.id), "{} 与 Fluent 图库重名", image.id);
        }
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
    fn audio_kind_checks_content_type_and_magic() {
        assert_eq!(AudioKind::from_content_type("audio/mpeg"), Some(AudioKind::Mp3));
        assert_eq!(AudioKind::from_content_type("Audio/OGG; codecs=opus"), Some(AudioKind::Ogg));
        assert_eq!(AudioKind::from_content_type("audio/x-wav"), Some(AudioKind::Wav));
        assert_eq!(AudioKind::from_content_type("audio/mp4"), Some(AudioKind::M4a));
        assert_eq!(AudioKind::from_content_type("text/html"), None);

        assert_eq!(AudioKind::from_magic(b"ID3\x04\0"), Some(AudioKind::Mp3));
        assert_eq!(AudioKind::from_magic(&[0xFF, 0xFB, 0x90, 0x64]), Some(AudioKind::Mp3));
        assert_eq!(AudioKind::from_magic(b"OggS\0\x02"), Some(AudioKind::Ogg));
        assert_eq!(AudioKind::from_magic(b"RIFF\x24\0\0\0WAVEfmt "), Some(AudioKind::Wav));
        assert_eq!(AudioKind::from_magic(b"\0\0\0\x20ftypM4A "), Some(AudioKind::M4a));
        assert_eq!(AudioKind::from_magic(b"RIFF\x10\0\0\0WEBPVP8 "), None);
        assert_eq!(AudioKind::from_magic(b"<html>"), None);
        assert_eq!(AudioKind::from_magic(&[]), None);
    }

    #[test]
    fn custom_sound_import_names_by_hash_and_replaces_same_slot() {
        let dir = tempfile::tempdir().unwrap();
        let first = import_custom_sound("press", b"OggS\0first", dir.path()).unwrap();
        assert!(is_custom_sound_file(&first, "press"), "{first}");
        assert!(first.ends_with(".ogg"));
        assert_eq!(first, import_custom_sound("press", b"OggS\0first", dir.path()).unwrap());
        let release = import_custom_sound("release", b"ID3\x04\0", dir.path()).unwrap();
        let second = import_custom_sound("press", b"OggS\0second", dir.path()).unwrap();
        assert_ne!(first, second);
        assert!(!dir.path().join(&first).exists(), "同槽位旧文件被替换");
        assert!(dir.path().join(&second).exists());
        assert!(dir.path().join(&release).exists(), "另一个槽位不受影响");

        // 不是命名规则内的文件不会被清理。
        std::fs::write(dir.path().join("notes.txt"), b"keep").unwrap();
        import_custom_sound("press", b"OggS\0third", dir.path()).unwrap();
        assert!(dir.path().join("notes.txt").exists());

        for (slot, bytes, needle) in [
            ("press", &b""[..], "空的"),
            ("press", &b"<html>"[..], "不是有效的"),
            ("other", &b"OggS"[..], "未知的音效槽位"),
        ] {
            let error = import_custom_sound(slot, bytes, dir.path()).unwrap_err().to_string();
            assert!(error.contains(needle), "{error}");
        }
        let big = vec![0xFF; MAX_SOUND_BYTES + 1];
        assert!(import_custom_sound("press", &big, dir.path()).unwrap_err().to_string().contains("2 MiB"));
    }

    #[test]
    fn sound_catalog_lists_cc0_downloads_with_credit() {
        let catalog = sound_catalog();
        assert_eq!(catalog.len(), DOWNLOAD_SOUNDS.len());
        assert_eq!(catalog[0].id, "nya");
        for set in DOWNLOAD_SOUNDS {
            assert!(is_download_sound(set.id));
            assert!(!crate::settings::AGENT_PET_SOUND_SETS.contains(&set.id) && set.id != CUSTOM_SOUND_ID);
            assert!(set.press.starts_with("https://cdn.freesound.org/previews/"), "{}", set.id);
            assert!(set.release.is_none_or(|url| url.starts_with("https://cdn.freesound.org/previews/")));
            assert!(set.source.starts_with("https://freesound.org/"));
            assert_eq!(set.license, "CC0");
        }
        assert!(!is_download_sound("duck"));
    }

    #[tokio::test]
    async fn download_sound_served_from_cache_without_network() {
        let cache = tempfile::tempdir().unwrap();
        let custom = tempfile::tempdir().unwrap();
        let set = &DOWNLOAD_SOUNDS[1];
        write_cache(cache.path(), set.press, AudioKind::Mp3, b"ID3\x04press").unwrap();
        write_cache(cache.path(), set.release.unwrap(), AudioKind::Mp3, b"ID3\x04release").unwrap();
        let sound = load_sound(set.id, "", "", cache.path(), custom.path()).await.unwrap();
        assert_eq!(sound.key, set.id);
        assert!(sound.press.unwrap().starts_with("data:audio/mpeg;base64,"));
        assert!(sound.release.is_some());
    }

    #[tokio::test]
    async fn load_sound_covers_embedded_and_custom_sets() {
        let cache = tempfile::tempdir().unwrap();
        let custom = tempfile::tempdir().unwrap();
        let duck = load_sound("duck", "", "", cache.path(), custom.path()).await.unwrap();
        assert_eq!(duck.key, "duck");
        assert!(duck.press.unwrap().starts_with("data:audio/mpeg;base64,"));

        let release = import_custom_sound("release", b"RIFF\x24\0\0\0WAVEfmt ", custom.path()).unwrap();
        let sound = load_sound("custom", "", &release, cache.path(), custom.path()).await.unwrap();
        assert_eq!(sound.key, custom_sound_key("", &release));
        assert!(sound.press.is_none());
        assert!(sound.release.unwrap().starts_with("data:audio/wav;base64,"));

        for (set, press, needle) in [
            ("custom", "", "还没有选择"),
            ("custom", "../press-0123456789abcdef.mp3", "文件名无效"),
            ("custom", "press-0123456789abcdef.mp3", "请重新选择"),
            ("nope", "", "未知的桌宠音效"),
        ] {
            let error = load_sound(set, press, "", cache.path(), custom.path())
                .await
                .unwrap_err()
                .to_string();
            assert!(error.contains(needle), "{set}/{press}: {error}");
        }
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
        assert!(read_cache::<ImageKind>(dir.path(), url).is_none());
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

    /// 真实联网核对二次元图库与下载音效的地址、类型和哈希。默认跳过：
    /// `cargo test -p codex-plus-core --lib agent_pet::tests::live_downloads_resolve -- --ignored`
    #[tokio::test]
    #[ignore = "需要联网"]
    async fn live_downloads_resolve() {
        let cache = tempfile::tempdir().unwrap();
        let custom = tempfile::tempdir().unwrap();
        for image in ANIME_IMAGES {
            let loaded = load_image(image.id, "", cache.path()).await;
            assert!(loaded.is_ok(), "{}: {:?}", image.id, loaded.err());
        }
        for set in DOWNLOAD_SOUNDS {
            let sound = load_sound(set.id, "", "", cache.path(), custom.path()).await;
            let sound = sound.unwrap_or_else(|error| panic!("{}: {error:#}", set.id));
            assert_eq!(sound.release.is_some(), set.release.is_some(), "{}", set.id);
        }
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
