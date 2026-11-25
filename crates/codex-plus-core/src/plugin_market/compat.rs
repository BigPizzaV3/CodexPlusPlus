//! 给本地账号的兼容服务（[`crate::chatgpt_compat`]）用的插件目录：目录来自完整库的缓存索引，
//! 插件包按需从仓库取下来、打成 tar.gz 交给 Codex 自己安装。
//!
//! 本地账号是假的 ChatGPT 登录，Codex 会去「官方云端目录」（`/ps/plugins/*`）读插件、装插件。
//! 兼容服务用这里的数据回答那些接口，装插件就走 Codex 原生的远程插件流程，不用注入层拦截。

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::Context;

use super::{
    PluginIndex, Source, download_package, ensure_directory, load_index, market_root,
    package_root, prepare_home, remove_owned_directory, validate_payload,
};

/// 目录里的一个插件（索引条目里兼容服务用得到的字段）。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CatalogPlugin {
    pub id: String,
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub short_description: String,
    pub version: String,
    pub author: String,
    pub category: String,
    pub keywords: Vec<String>,
    pub icon_url: String,
    pub icon_url_dark: String,
    pub skills: u64,
    /// 包里带 `.app.json`：连接器由 ChatGPT 托管，本地账号连不上。
    pub has_apps: bool,
    /// 包里带 `.mcp.json`：MCP 服务能在本机登录。
    pub has_mcp: bool,
}

/// 官方目录的一个分组：标题按语言，成员是插件 id。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CatalogSection {
    pub slug: String,
    pub title: BTreeMap<String, String>,
    pub plugins: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Catalog {
    pub revision: String,
    pub updated_at: String,
    pub plugins: Vec<CatalogPlugin>,
    pub sections: Vec<CatalogSection>,
}

impl Catalog {
    pub fn plugin(&self, id: &str) -> Option<&CatalogPlugin> {
        self.plugins.iter().find(|plugin| plugin.id == id)
    }
}

fn catalog_from(index: &PluginIndex) -> Catalog {
    Catalog {
        revision: index.revision.clone(),
        updated_at: index.updated_at.clone(),
        plugins: index
            .plugins
            .iter()
            .map(|plugin| CatalogPlugin {
                id: plugin.id.clone(),
                name: plugin.name.clone(),
                display_name: if plugin.display_name.trim().is_empty() {
                    plugin.name.clone()
                } else {
                    plugin.display_name.clone()
                },
                description: plugin.description.clone(),
                short_description: plugin.short_description.clone(),
                version: plugin.version.clone(),
                author: plugin.author.clone(),
                category: plugin.category.clone(),
                keywords: plugin.tags.clone(),
                icon_url: plugin.icon_url.clone(),
                icon_url_dark: plugin.icon_url_dark.clone(),
                skills: plugin.skills,
                has_apps: plugin.has_apps,
                has_mcp: plugin.has_mcp,
            })
            .collect(),
        sections: index
            .sections
            .iter()
            .map(|section| CatalogSection {
                slug: section.slug.clone(),
                title: section.title.clone(),
                plugins: section.plugins.clone(),
            })
            .collect(),
    }
}

/// 读过的目录：同一份索引（revision 一样）只解析一次，一万多条每个请求都解析太慢。
static CATALOG: Mutex<Option<Arc<Catalog>>> = Mutex::new(None);

/// 完整库的目录。先用本机缓存的索引，没有缓存才联网取。
pub(crate) async fn catalog(home: &Path) -> anyhow::Result<Arc<Catalog>> {
    let home = prepare_home(home)?;
    let full = Source::parse("full")?;
    let (index, _) = load_index(&home, full, false).await?;
    let mut cached = CATALOG.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(catalog) = cached.as_ref().filter(|catalog| catalog.revision == index.revision) {
        return Ok(catalog.clone());
    }
    let catalog = Arc::new(catalog_from(&index));
    *cached = Some(catalog.clone());
    Ok(catalog)
}

/// 每次只打一个包：同一个插件被连着请求时第二次直接读缓存，也不会同时跑两份 git。
static BUNDLE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// 一个插件的 tar.gz：插件根目录（含 `.codex-plugin/plugin.json`）就是归档根。
/// 打好的包按「id + 目录树哈希」缓存，下次直接读。
pub(crate) async fn bundle(home: &Path, id: &str) -> anyhow::Result<Vec<u8>> {
    let home = prepare_home(home)?;
    let full = Source::parse("full")?;
    let (index, _) = load_index(&home, full, false).await?;
    let plugin = index
        .plugins
        .iter()
        .find(|plugin| plugin.id == id)
        .with_context(|| format!("目录里没有插件 {id}"))?
        .clone();
    let _guard = BUNDLE_LOCK.lock().await;
    let bundles = market_root(&home).join("bundles");
    ensure_directory(&home, &bundles)?;
    let cached = bundles.join(format!("{}-{}.tar.gz", plugin.id, plugin.tree.to_ascii_lowercase()));
    if let Ok(bytes) = std::fs::read(&cached) {
        return Ok(bytes);
    }
    // 已经从快照装过同一版本：直接用本机的文件，不用再下载。
    let installed = package_root(&home, full, &plugin);
    let bytes = if validate_payload(&installed, &plugin).is_ok() {
        tar_gz(&installed)?
    } else {
        let downloaded = download_package(&home, full, &index, &plugin).await?;
        let result = validate_payload(&downloaded, &plugin).and_then(|()| tar_gz(&downloaded));
        // 下载目录是这次新建的一次性目录（downloads/<uuid>/<插件路径>），用完就删。
        if let Some(work) = download_work_dir(&home, &downloaded) {
            let _ = remove_owned_directory(&home, &market_root(&home).join("downloads"), &work);
        }
        result?
    };
    crate::settings::atomic_write(&cached, &bytes).context("无法缓存插件包")?;
    Ok(bytes)
}

/// `downloads/<uuid>/…/<插件目录>` 里的 `downloads/<uuid>`。
fn download_work_dir(home: &Path, downloaded: &Path) -> Option<PathBuf> {
    let downloads = market_root(home).join("downloads");
    let relative = downloaded.strip_prefix(&downloads).ok()?;
    let first = relative.components().next()?;
    Some(downloads.join(first.as_os_str()))
}

/// 把目录打成 tar.gz（POSIX ustar）：只收普通文件和目录，跳过符号链接；条目路径相对目录本身。
fn tar_gz(root: &Path) -> anyhow::Result<Vec<u8>> {
    let mut entries = Vec::new();
    collect(root, root, &mut entries)?;
    entries.sort();
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    for relative in entries {
        let path = root.join(&relative);
        let metadata = std::fs::symlink_metadata(&path)?;
        let name = relative.to_string_lossy().replace('\\', "/");
        if metadata.is_dir() {
            encoder.write_all(&ustar_header(&format!("{name}/"), 0, true, 0o755)?)?;
        } else {
            let bytes = std::fs::read(&path).with_context(|| format!("无法读取 {name}"))?;
            let mode = if is_executable(&metadata) { 0o755 } else { 0o644 };
            encoder.write_all(&ustar_header(&name, bytes.len() as u64, false, mode)?)?;
            encoder.write_all(&bytes)?;
            let pad = (512 - bytes.len() % 512) % 512;
            encoder.write_all(&vec![0u8; pad])?;
        }
    }
    // 归档结尾：两个全零块。
    encoder.write_all(&[0u8; 1024])?;
    Ok(encoder.finish()?)
}

fn collect(root: &Path, directory: &Path, out: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            continue;
        }
        let path = entry.path();
        let relative = path.strip_prefix(root)?.to_path_buf();
        if kind.is_dir() {
            out.push(relative);
            collect(root, &path, out)?;
        } else if kind.is_file() {
            out.push(relative);
        }
    }
    Ok(())
}

#[cfg(unix)]
fn is_executable(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_metadata: &std::fs::Metadata) -> bool {
    false
}

/// 一个 ustar 头。名字超过 100 字节时拆到 155 字节的前缀里，再长就报错（插件包里不会有）。
fn ustar_header(name: &str, size: u64, directory: bool, mode: u32) -> anyhow::Result<[u8; 512]> {
    let mut header = [0u8; 512];
    let bytes = name.as_bytes();
    let (prefix, base) = if bytes.len() <= 100 {
        (&[][..], bytes)
    } else {
        let split = name
            .char_indices()
            .filter(|(index, ch)| *ch == '/' && *index <= 155 && bytes.len() - index - 1 <= 100)
            .map(|(index, _)| index)
            .last()
            .with_context(|| format!("插件包里的路径太长：{name}"))?;
        (&bytes[..split], &bytes[split + 1..])
    };
    header[..base.len()].copy_from_slice(base);
    let octal = |field: &mut [u8], value: u64| {
        let text = format!("{value:0width$o}\0", width = field.len() - 1);
        field.copy_from_slice(text.as_bytes());
    };
    octal(&mut header[100..108], u64::from(mode));
    octal(&mut header[108..116], 0);
    octal(&mut header[116..124], 0);
    octal(&mut header[124..136], size);
    octal(&mut header[136..148], 0);
    header[156] = if directory { b'5' } else { b'0' };
    header[257..263].copy_from_slice(b"ustar\0");
    header[263..265].copy_from_slice(b"00");
    header[345..345 + prefix.len()].copy_from_slice(prefix);
    // 校验和：先把校验和字段当空格算。
    header[148..156].copy_from_slice(b"        ");
    let sum: u32 = header.iter().map(|byte| u32::from(*byte)).sum();
    let text = format!("{sum:06o}\0 ");
    header[148..156].copy_from_slice(text.as_bytes());
    Ok(header)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 不依赖 tar 库：按 ustar 格式自己读回来核对。
    fn read_back(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
        use std::io::Read;
        let mut raw = Vec::new();
        flate2::read::GzDecoder::new(bytes).read_to_end(&mut raw).unwrap();
        let mut out = Vec::new();
        let mut offset = 0;
        while offset + 512 <= raw.len() {
            let header = &raw[offset..offset + 512];
            if header.iter().all(|byte| *byte == 0) {
                break;
            }
            let field = |range: std::ops::Range<usize>| {
                String::from_utf8_lossy(&header[range]).trim_end_matches('\0').to_string()
            };
            let mut name = field(0..100);
            let prefix = field(345..500);
            if !prefix.is_empty() {
                name = format!("{prefix}/{name}");
            }
            let size = u64::from_str_radix(field(124..135).trim(), 8).unwrap() as usize;
            let sum: u32 = header
                .iter()
                .enumerate()
                .map(|(index, byte)| if (148..156).contains(&index) { 32 } else { u32::from(*byte) })
                .sum();
            assert_eq!(u32::from_str_radix(field(148..154).trim(), 8).unwrap(), sum, "校验和");
            offset += 512;
            out.push((name, raw[offset..offset + size].to_vec()));
            offset += size.div_ceil(512) * 512;
        }
        out
    }

    #[test]
    fn tar_gz_keeps_the_plugin_root_at_the_archive_root() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        std::fs::create_dir_all(root.join(".codex-plugin")).unwrap();
        std::fs::write(root.join(".codex-plugin/plugin.json"), br#"{"name":"demo"}"#).unwrap();
        let deep = "skills/".to_string() + &"a".repeat(60) + "/" + &"b".repeat(60);
        std::fs::create_dir_all(root.join(&deep)).unwrap();
        std::fs::write(root.join(&deep).join("SKILL.md"), b"# hi").unwrap();
        let entries = read_back(&tar_gz(root).unwrap());
        let names: Vec<&str> = entries.iter().map(|(name, _)| name.as_str()).collect();
        assert!(names.contains(&".codex-plugin/plugin.json"), "{names:?}");
        assert!(names.contains(&format!("{deep}/SKILL.md").as_str()), "长路径拆到前缀里：{names:?}");
        let manifest = entries.iter().find(|(name, _)| name == ".codex-plugin/plugin.json").unwrap();
        assert_eq!(manifest.1, br#"{"name":"demo"}"#);
    }

    #[test]
    fn download_work_dir_is_the_uuid_directory() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let downloads = market_root(home).join("downloads");
        let inside = downloads.join("1234").join("markets/01/plugins/x");
        assert_eq!(download_work_dir(home, &inside), Some(downloads.join("1234")));
        assert_eq!(download_work_dir(home, Path::new("/elsewhere")), None);
    }
}
