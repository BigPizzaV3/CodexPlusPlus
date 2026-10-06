use std::io::{Cursor, Read};
use std::path::{Component, Path, PathBuf};

use anyhow::Context;
use toml_edit::{DocumentMut, Item, Table};

const OPENAI_CURATED_MARKETPLACE: &str = "openai-curated";
const OPENAI_API_CURATED_MARKETPLACE: &str = "openai-api-curated";
/// 内置插件包在 zip 里自带的名字。这是 codex 的保留名，注册在它下面会被静默忽略，
/// 所以落盘时会被改写成 `CODEX_PLUS_MARKETPLACE`；这里只用于识别和清理历史遗留配置。
const LEGACY_REMOTE_MARKETPLACE: &str = "openai-curated-remote";
/// 内置插件包实际注册用的 marketplace 名。
///
/// **不能用 `openai-*` 开头的名字。** codex 把这些当保留名，注册在它们下面的本地
/// marketplace 会被完全忽略——`codex plugin marketplace list` 直接不列出，用户在
/// 插件市场里就看不到、装不了任何插件（issue #1974 / #1968）。
///
/// 实测（同一目录、同一份 marketplace.json，只改 name）：
///
/// | name | 结果 |
/// |---|---|
/// | `openai-curated` / `openai-curated-remote` | 被忽略 |
/// | `openai-bundled` / `openai-bundled-alpha` | 被忽略 |
/// | `openai-primary-runtime` / `openai-api-curated` | 被忽略 |
/// | `codex-plus-curated` | 可用 |
const CODEX_PLUS_MARKETPLACE: &str = "codex-plus-curated";
const ROLE_SPECIFIC_PLUGINS_MARKETPLACE: &str = "role-specific-plugins";
const OPENAI_PLUGINS_ZIP_URL: &str =
    "https://codeload.github.com/openai/plugins/zip/refs/heads/main";
const OPENAI_PLUGINS_DOWNLOAD_LIMIT_BYTES: usize = 128 * 1024 * 1024;
const CODEX_PLUS_MARKETPLACE_ZIP: &[u8] =
    include_bytes!("../../../assets/plugin-marketplaces/openai-curated-remote.zip");

pub fn ensure_openai_curated_marketplace_config(home: &Path) -> anyhow::Result<bool> {
    let mut changed = cleanup_managed_reserved_marketplace_configs(home)?;
    if let Some(remote_marketplace_root) = local_openai_curated_remote_marketplace_root(home)? {
        // 必须和写 config 一起做：老用户目录早就解压好了、不会再走安装分支，
        // 里面的 marketplace.json 还是旧的保留名。只改 config 不改磁盘，两边名字
        // 对不上，codex 照样整个市场不认 —— 表现和改名前一样，插件一个都列不出来。
        rewrite_marketplace_name(&remote_marketplace_root)?;
        // 注册名以磁盘快照的真实 name 为准（#2138），而不是写死的常量：两边逐字一致
        // 是 codex 认这个市场的唯一条件。
        let marketplace_name = managed_marketplace_name_for_root(&remote_marketplace_root)?;
        changed |= ensure_marketplace_configs(
            home,
            &[marketplace_name.as_str()],
            &remote_marketplace_root,
        )?;
    }
    Ok(changed)
}

pub fn ensure_openai_curated_remote_marketplace_config(home: &Path) -> anyhow::Result<bool> {
    let Some(marketplace_root) = local_openai_curated_remote_marketplace_root(home)? else {
        return Ok(false);
    };
    // 老用户的目录早就解压好了，不会再走安装分支，但里面的 marketplace.json 还是
    // 旧的保留名。config 注册的是新名，两边对不上 codex 一样不认，所以这里无条件
    // 规范化一次。
    rewrite_marketplace_name(&marketplace_root)?;
    let marketplace_name = managed_marketplace_name_for_root(&marketplace_root)?;
    ensure_marketplace_configs(home, &[marketplace_name.as_str()], &marketplace_root)
}

pub fn ensure_role_specific_plugins_marketplace_config(home: &Path) -> anyhow::Result<bool> {
    let Some(marketplace_root) = local_role_specific_plugins_marketplace_root(home)? else {
        return Ok(false);
    };
    let plugin_ids =
        local_marketplace_plugin_names(&marketplace_root, ROLE_SPECIFIC_PLUGINS_MARKETPLACE)?
            .into_iter()
            .map(|name| format!("{name}@{ROLE_SPECIFIC_PLUGINS_MARKETPLACE}"))
            .collect::<Vec<_>>();
    ensure_marketplace_configs_with_plugins(
        home,
        &[ROLE_SPECIFIC_PLUGINS_MARKETPLACE],
        &marketplace_root,
        &plugin_ids,
    )
}

pub fn ensure_openai_curated_remote_marketplace_available(
    home: &Path,
) -> anyhow::Result<MarketplaceEnsureResult> {
    let mut initialized = false;
    if local_openai_curated_remote_marketplace_root(home)?.is_none() {
        install_openai_curated_remote_marketplace_zip(home, CODEX_PLUS_MARKETPLACE_ZIP)?;
        initialized = true;
    }
    let configured = ensure_openai_curated_remote_marketplace_config(home)?;
    Ok(MarketplaceEnsureResult {
        initialized,
        configured,
    })
}

pub fn preserve_openai_curated_remote_marketplace_config(
    home: &Path,
    config_text: &str,
) -> anyhow::Result<String> {
    let Some(marketplace_root) = local_openai_curated_remote_marketplace_root(home)? else {
        return Ok(config_text.to_string());
    };
    // 写回用磁盘快照的真实 name（#2138），否则会把一份对不上的注册表写回模板。
    let marketplace_name = managed_marketplace_name_for_root(&marketplace_root)?;
    merge_marketplace_configs_into_text(
        config_text,
        &[marketplace_name.as_str()],
        &marketplace_root,
    )
}

pub fn openai_curated_marketplace_status(home: &Path) -> MarketplaceStatus {
    let marketplace_root = local_openai_curated_marketplace_root(home).ok().flatten();
    let remote_marketplace_root = local_openai_curated_remote_marketplace_root(home)
        .ok()
        .flatten();
    // 判定「已注册」同样以磁盘快照的真实 name 为准（#2138）：写回侧已经改成读快照，
    // 状态侧若还按写死的常量比对，就会把自己刚写好的配置判成未注册，进而反复触发
    // 修复写入。
    let registered_name = remote_marketplace_root
        .as_deref()
        .and_then(|root| managed_marketplace_name_for_root(root).ok());
    let config_registered = !managed_reserved_marketplace_config_present(home)
        && match (
            registered_name.as_deref(),
            remote_marketplace_root.as_deref(),
        ) {
            (Some(name), Some(remote_root)) => {
                marketplace_config_points_to_root(home, name, remote_root)
            }
            _ => true,
        };
    MarketplaceStatus {
        marketplace_root,
        config_registered,
    }
}

pub fn openai_curated_remote_marketplace_status(home: &Path) -> MarketplaceStatus {
    let marketplace_root = local_openai_curated_remote_marketplace_root(home)
        .ok()
        .flatten();
    // 与写回侧同一个名字（#2138）：写的是快照真实 name，这里也按它比对，
    // 否则刚写好的配置会被判成未注册、每次启动都触发无谓的修复写入。
    let config_registered = marketplace_root
        .as_deref()
        .and_then(|root| {
            managed_marketplace_name_for_root(root)
                .ok()
                .map(|name| marketplace_config_points_to_root(home, &name, root))
        })
        .unwrap_or(false);
    MarketplaceStatus {
        marketplace_root,
        config_registered,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketplaceStatus {
    pub marketplace_root: Option<PathBuf>,
    pub config_registered: bool,
}

impl MarketplaceStatus {
    pub fn needs_repair(&self) -> bool {
        self.marketplace_root.is_none() || !self.config_registered
    }
}

pub async fn initialize_openai_curated_marketplace_and_configure(
    home: &Path,
) -> anyhow::Result<MarketplaceEnsureResult> {
    let mut initialized = false;
    if local_openai_curated_marketplace_root(home)?.is_none() {
        initialize_openai_curated_marketplace_from_github(home).await?;
        initialized = true;
    }
    let configured = ensure_openai_curated_marketplace_config(home)?;
    Ok(MarketplaceEnsureResult {
        initialized,
        configured,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarketplaceEnsureResult {
    pub initialized: bool,
    pub configured: bool,
}

fn local_openai_curated_marketplace_root(home: &Path) -> anyhow::Result<Option<PathBuf>> {
    let root = home.join(".tmp").join("plugins");
    let marketplace_path = root
        .join(".agents")
        .join("plugins")
        .join("marketplace.json");
    if !marketplace_path.is_file() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&marketplace_path)
        .with_context(|| format!("failed to read {}", marketplace_path.display()))?;
    let marketplace: serde_json::Value = serde_json::from_str(&text)
        .with_context(|| format!("failed to parse {}", marketplace_path.display()))?;
    if marketplace.get("name").and_then(serde_json::Value::as_str)
        != Some(OPENAI_CURATED_MARKETPLACE)
    {
        return Ok(None);
    }
    let has_plugins = marketplace
        .get("plugins")
        .and_then(serde_json::Value::as_array)
        .map(|plugins| !plugins.is_empty())
        .unwrap_or(false);
    if !has_plugins || !root.join("plugins").is_dir() {
        return Ok(None);
    }
    Ok(Some(root))
}

fn local_role_specific_plugins_marketplace_root(home: &Path) -> anyhow::Result<Option<PathBuf>> {
    let root = home
        .join(".tmp")
        .join("marketplaces")
        .join(ROLE_SPECIFIC_PLUGINS_MARKETPLACE);
    local_marketplace_root_from_root(&root, ROLE_SPECIFIC_PLUGINS_MARKETPLACE)
}

fn local_marketplace_root_from_root(
    root: &Path,
    marketplace_name: &str,
) -> anyhow::Result<Option<PathBuf>> {
    let marketplace_path = root
        .join(".agents")
        .join("plugins")
        .join("marketplace.json");
    if !marketplace_path.is_file() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&marketplace_path)
        .with_context(|| format!("failed to read {}", marketplace_path.display()))?;
    let marketplace: serde_json::Value = serde_json::from_str(&text)
        .with_context(|| format!("failed to parse {}", marketplace_path.display()))?;
    if marketplace.get("name").and_then(serde_json::Value::as_str) != Some(marketplace_name) {
        return Ok(None);
    }
    let has_plugins = marketplace
        .get("plugins")
        .and_then(serde_json::Value::as_array)
        .map(|plugins| !plugins.is_empty())
        .unwrap_or(false);
    if !has_plugins || !root.join("plugins").is_dir() {
        return Ok(None);
    }
    Ok(Some(root.to_path_buf()))
}

fn local_marketplace_plugin_names(
    root: &Path,
    marketplace_name: &str,
) -> anyhow::Result<Vec<String>> {
    let marketplace_path = root
        .join(".agents")
        .join("plugins")
        .join("marketplace.json");
    let text = std::fs::read_to_string(&marketplace_path)
        .with_context(|| format!("failed to read {}", marketplace_path.display()))?;
    let marketplace: serde_json::Value = serde_json::from_str(&text)
        .with_context(|| format!("failed to parse {}", marketplace_path.display()))?;
    let name = marketplace.get("name").and_then(serde_json::Value::as_str);
    // 内置市场的名字要认两档：磁盘上可能还是旧的保留名。
    // 清理路径（cleanup_managed_reserved_marketplace_configs）在改写磁盘之前
    // 就需要读这份插件清单，只认新名会在升级的第一次启动上拿到空表，
    // 迁移就被整段跳过了（#2072）。
    let name_matches = name == Some(marketplace_name)
        || (marketplace_name == CODEX_PLUS_MARKETPLACE && is_codex_plus_marketplace_name(name));
    if !name_matches {
        return Ok(Vec::new());
    }
    Ok(marketplace
        .get("plugins")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|plugin| {
            plugin
                .get("name")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_string)
        })
        .collect())
}

fn local_openai_curated_remote_marketplace_root(home: &Path) -> anyhow::Result<Option<PathBuf>> {
    let root = home.join(".tmp").join("plugins-remote");
    let marketplace_path = root
        .join(".agents")
        .join("plugins")
        .join("marketplace.json");
    if !marketplace_path.is_file() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&marketplace_path)
        .with_context(|| format!("failed to read {}", marketplace_path.display()))?;
    let marketplace: serde_json::Value = serde_json::from_str(&text)
        .with_context(|| format!("failed to parse {}", marketplace_path.display()))?;
    // 认新旧两种名（zip 里自带旧名、历史目录也是旧名），也认任何**非保留名**：
    // 快照里完全可能是第三方工具或用户自己写进去的名字，而 config 注册名必须以
    // 它为准（#2138）。旧的「只认 CODEX_PLUS_MARKETPLACE / LEGACY_REMOTE」白名单
    // 会把这类目录整个判成「不存在」，于是永远走不到注册那一步——市场在 Codex++
    // 里看着有、在 codex CLI 里完全没有。
    // 保留名（`openai-*`）仍然拒绝：注册在它下面会被 codex 静默忽略
    // （#1974 / #1968）。
    if !marketplace
        .get("name")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|name| !is_reserved_marketplace_name(name))
    {
        return Ok(None);
    }
    let has_plugins = marketplace
        .get("plugins")
        .and_then(serde_json::Value::as_array)
        .map(|plugins| !plugins.is_empty())
        .unwrap_or(false);
    if !has_plugins || !root.join("plugins").is_dir() {
        return Ok(None);
    }
    Ok(Some(root))
}

async fn initialize_openai_curated_marketplace_from_github(home: &Path) -> anyhow::Result<()> {
    let bytes = download_openai_plugins_zip().await?;
    install_openai_plugins_zip(home, &bytes)
}

async fn download_openai_plugins_zip() -> anyhow::Result<Vec<u8>> {
    let client =
        crate::http_client::proxied_client(&format!("Codex++/{}", crate::version::VERSION))?;
    let bytes = client
        .get(OPENAI_PLUGINS_ZIP_URL)
        .header(reqwest::header::ACCEPT, "application/zip")
        .send()
        .await
        .context("failed to download openai/plugins marketplace")?
        .error_for_status()
        .context("openai/plugins marketplace download returned an error status")?
        .bytes()
        .await
        .context("failed to read openai/plugins marketplace download body")?;
    if bytes.len() > OPENAI_PLUGINS_DOWNLOAD_LIMIT_BYTES {
        anyhow::bail!(
            "openai/plugins marketplace download is too large: {} bytes",
            bytes.len()
        );
    }
    Ok(bytes.to_vec())
}

fn install_openai_plugins_zip(home: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let destination = home.join(".tmp").join("plugins");
    let staging_parent = home.join(".tmp");
    std::fs::create_dir_all(&staging_parent)
        .with_context(|| format!("failed to create {}", staging_parent.display()))?;
    let staging = staging_parent.join(format!(
        "plugins-download-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    ));
    if staging.exists() {
        std::fs::remove_dir_all(&staging)
            .with_context(|| format!("failed to remove stale {}", staging.display()))?;
    }
    std::fs::create_dir_all(&staging)
        .with_context(|| format!("failed to create {}", staging.display()))?;

    let result = extract_openai_plugins_zip(bytes, &staging)
        .and_then(|_| validate_openai_plugins_marketplace_root(&staging))
        .and_then(|_| replace_directory(&staging, &destination));
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    result
}

/// 内置插件包的 marketplace.json 认新旧两种 name：zip 里自带旧名，
/// 历史版本解压出来的目录也是旧名。
fn is_codex_plus_marketplace_name(name: Option<&str>) -> bool {
    matches!(
        name,
        Some(CODEX_PLUS_MARKETPLACE) | Some(LEGACY_REMOTE_MARKETPLACE)
    )
}

/// 一个 marketplace 名字是不是 codex 的保留名（注册在它下面会被静默忽略）。
///
/// `openai-curated-remote` 是历史遗留的例外：它是 zip 自带的旧名，我们自己的
/// 托管快照会以它落盘（随后由 `rewrite_marketplace_name` 改写），所以它不算
/// 「别人的保留名」，要正常认。
fn is_reserved_marketplace_name(name: &str) -> bool {
    let name = name.trim().to_ascii_lowercase();
    name.starts_with("openai-") && name != LEGACY_REMOTE_MARKETPLACE
}

/// 从磁盘快照的 `marketplace.json` 里读出**真实**的 marketplace name（#2138）。
///
/// codex 认的是插件目录里那份清单的 `name`，config.toml 的 `[marketplaces.<name>]`
/// 必须和它逐字相同，否则市场整个不认。过去两边各写各的：config 侧一律用写死的
/// `CODEX_PLUS_MARKETPLACE`，磁盘侧只保证被 `rewrite_marketplace_name` 改写过的
/// 目录是新名——快照里出现第三种名字（历史版本、用户手工改过、别的工具投影过来的）
/// 时，两边对不上，而且**没有任何地方会报错**，用户只看到「插件列表是空的」。
/// 所以写回一律以磁盘快照的实际 name 为准。
fn marketplace_name_from_root(root: &Path) -> anyhow::Result<Option<String>> {
    let path = root
        .join(".agents")
        .join("plugins")
        .join("marketplace.json");
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let marketplace: serde_json::Value = serde_json::from_str(&text)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(marketplace
        .get("name")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string))
}

/// 写 config 时要用的 marketplace name：以磁盘快照的真实 name 为准（#2138），
/// 例外情况映射回 `CODEX_PLUS_MARKETPLACE`：
///
/// - 快照里是 `openai-*` **保留名**（我们不允许注册的名字，codex 会静默忽略）；
/// - 快照里是我们自己的**旧名** `openai-curated-remote`，它是保留名的历史例外
///   （见 `is_reserved_marketplace_name`），等价于改名后的 `CODEX_PLUS_MARKETPLACE`。
///   这一条保证「磁盘已改名」与「磁盘还是旧名」两条时序走到同一个注册名——写模板
///   的路径不一定先经过 `rewrite_marketplace_name`。
///
/// 其余非保留名原样采用。
fn managed_marketplace_name_for_root(root: &Path) -> anyhow::Result<String> {
    let Some(name) = marketplace_name_from_root(root)? else {
        return Ok(CODEX_PLUS_MARKETPLACE.to_string());
    };
    if is_reserved_marketplace_name(&name) || name == LEGACY_REMOTE_MARKETPLACE {
        return Ok(CODEX_PLUS_MARKETPLACE.to_string());
    }
    Ok(name)
}

/// 把解压出来的 marketplace.json 的 name 改写成非保留名。
///
/// zip 里自带的是 `openai-curated-remote`——codex 的保留名，注册在它下面会被
/// 静默忽略。改名后 codex 才会真正加载这些插件（#1974 / #1968）。
///
/// 这是**磁盘规范化**：托管目录统一收敛到 `CODEX_PLUS_MARKETPLACE`（历史目录与
/// zip 自带的旧名都在这里被拉到同一条线上），后续 `local_*_marketplace_root` 的
/// 可用性判定才有单一判据。config 注册名则一律读改写后的快照（#2138），两边永远
/// 一致。
fn rewrite_marketplace_name(root: &Path) -> anyhow::Result<()> {
    let path = root
        .join(".agents")
        .join("plugins")
        .join("marketplace.json");
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let mut marketplace: serde_json::Value = serde_json::from_str(&text)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    if marketplace.get("name").and_then(serde_json::Value::as_str) == Some(CODEX_PLUS_MARKETPLACE) {
        return Ok(());
    }
    // 已经是**用户/第三方写入的非保留名**（且不是我们自己的旧名）就保持原样：
    // config 注册名会跟着它走（#2138）。需要改写的只有两类——codex 保留名
    // （别的 `openai-*`，注册在它下面会被静默忽略，#1974 / #1968）和我们自己的
    // 旧名 `openai-curated-remote`（zip 自带，必须收敛到当前常量）。
    if marketplace
        .get("name")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|name| {
            !is_reserved_marketplace_name(name) && name != LEGACY_REMOTE_MARKETPLACE
        })
    {
        return Ok(());
    }
    marketplace["name"] = serde_json::Value::String(CODEX_PLUS_MARKETPLACE.to_string());
    let encoded = serde_json::to_string_pretty(&marketplace)
        .with_context(|| format!("failed to encode {}", path.display()))?;
    std::fs::write(&path, encoded)
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

fn install_openai_curated_remote_marketplace_zip(home: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let destination = home.join(".tmp").join("plugins-remote");
    let staging_parent = home.join(".tmp");
    std::fs::create_dir_all(&staging_parent)
        .with_context(|| format!("failed to create {}", staging_parent.display()))?;
    let staging = staging_parent.join(format!(
        "plugins-remote-embedded-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    ));
    if staging.exists() {
        std::fs::remove_dir_all(&staging)
            .with_context(|| format!("failed to remove stale {}", staging.display()))?;
    }
    std::fs::create_dir_all(&staging)
        .with_context(|| format!("failed to create {}", staging.display()))?;

    let result = extract_zip_exact(bytes, &staging)
        .and_then(|_| rewrite_marketplace_name(&staging))
        .and_then(|_| validate_openai_curated_remote_marketplace_root(&staging))
        .and_then(|_| {
            replace_directory_with_backup_name(
                &staging,
                &destination,
                "plugins-remote.previous-codex-plus",
            )
        });
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    result
}

fn extract_openai_plugins_zip(bytes: &[u8], destination: &Path) -> anyhow::Result<()> {
    let cursor = Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor).context("failed to read openai/plugins zip")?;
    for index in 0..archive.len() {
        let mut file = archive
            .by_index(index)
            .with_context(|| format!("failed to read zip entry {index}"))?;
        let Some(relative_path) = zip_entry_relative_path(file.name()) else {
            continue;
        };
        let output_path = destination.join(relative_path);
        if file.is_dir() {
            std::fs::create_dir_all(&output_path)
                .with_context(|| format!("failed to create {}", output_path.display()))?;
            continue;
        }
        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let mut contents = Vec::new();
        file.read_to_end(&mut contents)
            .with_context(|| format!("failed to read zip entry {}", file.name()))?;
        std::fs::write(&output_path, contents)
            .with_context(|| format!("failed to write {}", output_path.display()))?;
    }
    Ok(())
}

fn extract_zip_exact(bytes: &[u8], destination: &Path) -> anyhow::Result<()> {
    let cursor = Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor).context("failed to read embedded plugin zip")?;
    for index in 0..archive.len() {
        let mut file = archive
            .by_index(index)
            .with_context(|| format!("failed to read zip entry {index}"))?;
        let relative_path = safe_zip_path(file.name())?;
        let output_path = destination.join(relative_path);
        if file.is_dir() {
            std::fs::create_dir_all(&output_path)
                .with_context(|| format!("failed to create {}", output_path.display()))?;
            continue;
        }
        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let mut contents = Vec::new();
        file.read_to_end(&mut contents)
            .with_context(|| format!("failed to read zip entry {}", file.name()))?;
        std::fs::write(&output_path, contents)
            .with_context(|| format!("failed to write {}", output_path.display()))?;
    }
    Ok(())
}

fn safe_zip_path(name: &str) -> anyhow::Result<PathBuf> {
    let path = Path::new(name);
    let mut relative = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => relative.push(value),
            Component::CurDir => {}
            _ => anyhow::bail!("zip entry escapes destination: {name}"),
        }
    }
    if relative.as_os_str().is_empty() {
        anyhow::bail!("zip entry has empty path");
    }
    Ok(relative)
}

pub(crate) fn zip_entry_relative_path(name: &str) -> Option<PathBuf> {
    let path = Path::new(name);
    let mut components = path.components();
    match components.next()? {
        Component::Normal(_) => {}
        _ => return None,
    }
    let mut relative = PathBuf::new();
    for component in components {
        match component {
            Component::Normal(value) => relative.push(value),
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!relative.as_os_str().is_empty()).then_some(relative)
}

fn validate_openai_plugins_marketplace_root(root: &Path) -> anyhow::Result<()> {
    let marketplace = local_openai_curated_marketplace_root_from_root(root)?
        .ok_or_else(|| anyhow::anyhow!("downloaded openai/plugins marketplace is invalid"))?;
    if marketplace != root {
        anyhow::bail!("downloaded openai/plugins marketplace root mismatch");
    }
    Ok(())
}

fn validate_openai_curated_remote_marketplace_root(root: &Path) -> anyhow::Result<()> {
    let marketplace = local_openai_curated_remote_marketplace_root_from_root(root)?
        .ok_or_else(|| anyhow::anyhow!("embedded official remote plugin marketplace is invalid"))?;
    if marketplace != root {
        anyhow::bail!("embedded official remote plugin marketplace root mismatch");
    }
    Ok(())
}

fn local_openai_curated_marketplace_root_from_root(root: &Path) -> anyhow::Result<Option<PathBuf>> {
    let marketplace_path = root
        .join(".agents")
        .join("plugins")
        .join("marketplace.json");
    if !marketplace_path.is_file() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&marketplace_path)
        .with_context(|| format!("failed to read {}", marketplace_path.display()))?;
    let marketplace: serde_json::Value = serde_json::from_str(&text)
        .with_context(|| format!("failed to parse {}", marketplace_path.display()))?;
    if marketplace.get("name").and_then(serde_json::Value::as_str)
        != Some(OPENAI_CURATED_MARKETPLACE)
    {
        return Ok(None);
    }
    let has_plugins = marketplace
        .get("plugins")
        .and_then(serde_json::Value::as_array)
        .map(|plugins| !plugins.is_empty())
        .unwrap_or(false);
    if !has_plugins || !root.join("plugins").is_dir() {
        return Ok(None);
    }
    Ok(Some(root.to_path_buf()))
}

fn local_openai_curated_remote_marketplace_root_from_root(
    root: &Path,
) -> anyhow::Result<Option<PathBuf>> {
    let marketplace_path = root
        .join(".agents")
        .join("plugins")
        .join("marketplace.json");
    if !marketplace_path.is_file() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&marketplace_path)
        .with_context(|| format!("failed to read {}", marketplace_path.display()))?;
    let marketplace: serde_json::Value = serde_json::from_str(&text)
        .with_context(|| format!("failed to parse {}", marketplace_path.display()))?;
    // 与 local_openai_curated_remote_marketplace_root 同一条判据：认非保留名；
    // 保留名（zip 自带的 `openai-curated-remote`、或别的 `openai-*`）一律不算
    // 可用市场（#2138）。
    if !marketplace
        .get("name")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|name| !is_reserved_marketplace_name(name))
    {
        return Ok(None);
    }
    let has_plugins = marketplace
        .get("plugins")
        .and_then(serde_json::Value::as_array)
        .map(|plugins| !plugins.is_empty())
        .unwrap_or(false);
    if !has_plugins || !root.join("plugins").is_dir() {
        return Ok(None);
    }
    Ok(Some(root.to_path_buf()))
}

fn replace_directory(source: &Path, destination: &Path) -> anyhow::Result<()> {
    replace_directory_with_backup_name(source, destination, "plugins.previous-codex-plus")
}

fn replace_directory_with_backup_name(
    source: &Path,
    destination: &Path,
    backup_name: &str,
) -> anyhow::Result<()> {
    let backup = destination.with_file_name(backup_name);
    if backup.exists() {
        std::fs::remove_dir_all(&backup)
            .with_context(|| format!("failed to remove {}", backup.display()))?;
    }
    if destination.exists() {
        std::fs::rename(destination, &backup).with_context(|| {
            format!(
                "failed to move {} to {}",
                destination.display(),
                backup.display()
            )
        })?;
    }
    match std::fs::rename(source, destination) {
        Ok(()) => {
            if backup.exists() {
                let _ = std::fs::remove_dir_all(&backup);
            }
            Ok(())
        }
        Err(error) => {
            if backup.exists() {
                let _ = std::fs::rename(&backup, destination);
            }
            Err(error).with_context(|| {
                format!(
                    "failed to move {} to {}",
                    source.display(),
                    destination.display()
                )
            })
        }
    }
}

fn ensure_marketplace_configs(
    home: &Path,
    marketplace_names: &[&str],
    marketplace_root: &Path,
) -> anyhow::Result<bool> {
    ensure_marketplace_configs_with_plugins(home, marketplace_names, marketplace_root, &[])
}

fn ensure_marketplace_configs_with_plugins(
    home: &Path,
    marketplace_names: &[&str],
    marketplace_root: &Path,
    plugin_ids: &[String],
) -> anyhow::Result<bool> {
    let config_path = home.join("config.toml");
    let existing = match std::fs::read(&config_path) {
        Ok(bytes) => String::from_utf8(bytes)
            .with_context(|| format!("failed to read UTF-8 {}", config_path.display()))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", config_path.display()));
        }
    };
    let without_bom = existing.trim_start_matches('\u{feff}');
    let updated = merge_marketplace_configs_and_plugins_into_text(
        without_bom,
        marketplace_names,
        marketplace_root,
        plugin_ids,
    )?;
    if updated.as_bytes() == without_bom.as_bytes() {
        return Ok(false);
    }
    crate::settings::atomic_write(&config_path, updated.as_bytes())?;
    Ok(true)
}

pub fn cleanup_managed_reserved_marketplace_configs(home: &Path) -> anyhow::Result<bool> {
    let config_path = home.join("config.toml");
    let existing = match std::fs::read_to_string(&config_path) {
        Ok(existing) => existing,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", config_path.display()));
        }
    };
    let mut doc = parse_toml_document(&existing)?;
    let managed_root = home.join(".tmp").join("plugins");
    let remote_root = home.join(".tmp").join("plugins-remote");
    // 全是 codex 的保留名，注册在它们下面会被静默忽略。只清我们自己写进去的
    // （source 指向我们托管的目录），用户手工加的同名条目不动。
    let managed_entries = [
        (OPENAI_CURATED_MARKETPLACE, managed_root.as_path()),
        (OPENAI_API_CURATED_MARKETPLACE, managed_root.as_path()),
        (LEGACY_REMOTE_MARKETPLACE, remote_root.as_path()),
    ];
    let mut changed = false;
    let mut remove_marketplaces_table = false;
    if let Some(marketplaces) = doc.get_mut("marketplaces").and_then(Item::as_table_mut) {
        for (marketplace_name, root) in managed_entries {
            let managed = marketplaces
                .get(marketplace_name)
                .and_then(Item::as_table)
                .is_some_and(|table| marketplace_table_points_to_root(table, root));
            if managed {
                marketplaces.remove(marketplace_name);
                changed = true;
            }
        }
        remove_marketplaces_table = marketplaces.is_empty();
    }
    if remove_marketplaces_table {
        doc.as_table_mut().remove("marketplaces");
    }
    // 市场条目和它下面的插件条目是一对：只删市场不改插件，`x@openai-curated`
    // 就成了悬空条目，codex 直接不列出，用户表现为「插件装了但列表和会话工具里都没有」
    // （#2072）。改名（02a23e14 / 6b65c094）只处理了市场侧，这里补上插件侧迁移。
    changed |= migrate_orphaned_reserved_marketplace_plugins(&mut doc, home)?;
    if !changed {
        return Ok(false);
    }
    crate::settings::atomic_write(
        &config_path,
        ensure_trailing_newline(doc.to_string()).as_bytes(),
    )?;
    Ok(true)
}

/// 把仍挂在保留市场名下的插件条目迁移到 `CODEX_PLUS_MARKETPLACE`。
///
/// 只处理「插件 ID 的 `@` 后半段是旧市场名」的条目：改名前落盘的都是这种形状
/// （`github@openai-curated`）。`@` 前是同名插件时保留原条目（含 `enabled`），
/// 否则改写 ID；新市场里已经没有这个插件时整个删除——留着它 codex 依然不认。
///
/// 迁移目标只认当前托管目录里真实存在的插件，避免把用户自建的同名市场条目
/// 误当成我们的历史遗留改掉。
fn migrate_orphaned_reserved_marketplace_plugins(
    doc: &mut DocumentMut,
    home: &Path,
) -> anyhow::Result<bool> {
    let legacy_names = [
        OPENAI_CURATED_MARKETPLACE,
        OPENAI_API_CURATED_MARKETPLACE,
        LEGACY_REMOTE_MARKETPLACE,
    ];
    // 新市场里实际提供的插件名。取不到就整段跳过：没有清单我们无法判断
    // 哪个插件还存在，此时宁可不动配置。
    let Some(marketplace_root) = local_openai_curated_remote_marketplace_root(home)? else {
        return Ok(false);
    };
    let available = local_marketplace_plugin_names(&marketplace_root, CODEX_PLUS_MARKETPLACE)?;
    if available.is_empty() {
        return Ok(false);
    }

    let Some(plugins) = doc.get_mut("plugins").and_then(Item::as_table_mut) else {
        return Ok(false);
    };
    let orphaned = plugins
        .iter()
        .filter_map(|(key, _)| {
            let (name, marketplace) = key.split_once('@')?;
            legacy_names
                .contains(&marketplace)
                .then(|| name.to_string())
        })
        .collect::<Vec<_>>();
    let mut changed = false;
    for name in orphaned {
        for legacy in legacy_names {
            let legacy_id = format!("{name}@{legacy}");
            let Some(entry) = plugins.remove(&legacy_id) else {
                continue;
            };
            changed = true;
            if !available.contains(&name) {
                // 新市场里没有这个插件了，条目留着也是悬空。
                continue;
            }
            let migrated_id = format!("{name}@{CODEX_PLUS_MARKETPLACE}");
            // 新 ID 已经存在时不覆盖：用户在新市场里重新装过一次，以它为准。
            if plugins.get(&migrated_id).is_none() {
                plugins.insert(&migrated_id, entry);
            }
        }
    }
    if plugins.is_empty() {
        doc.as_table_mut().remove("plugins");
    }
    Ok(changed)
}

fn managed_reserved_marketplace_config_present(home: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(home.join("config.toml")) else {
        return false;
    };
    let Ok(doc) = parse_toml_document(&text) else {
        return false;
    };
    let Some(marketplaces) = doc.get("marketplaces").and_then(Item::as_table) else {
        return false;
    };
    let managed_root = home.join(".tmp").join("plugins");
    [OPENAI_CURATED_MARKETPLACE, OPENAI_API_CURATED_MARKETPLACE]
        .into_iter()
        .any(|marketplace_name| {
            marketplaces
                .get(marketplace_name)
                .and_then(Item::as_table)
                .is_some_and(|table| marketplace_table_points_to_root(table, &managed_root))
        })
}

fn marketplace_table_points_to_root(table: &Table, root: &Path) -> bool {
    let source_type = table
        .get("source_type")
        .and_then(Item::as_str)
        .unwrap_or_default();
    let source = table
        .get("source")
        .and_then(Item::as_str)
        .unwrap_or_default();
    source_type == "local" && managed_marketplace_path_matches(source, root)
}

fn merge_marketplace_configs_into_text(
    config_text: &str,
    marketplace_names: &[&str],
    marketplace_root: &Path,
) -> anyhow::Result<String> {
    merge_marketplace_configs_and_plugins_into_text(
        config_text,
        marketplace_names,
        marketplace_root,
        &[],
    )
}

fn merge_marketplace_configs_and_plugins_into_text(
    config_text: &str,
    marketplace_names: &[&str],
    marketplace_root: &Path,
    plugin_ids: &[String],
) -> anyhow::Result<String> {
    let mut doc = parse_toml_document(config_text)?;
    let marketplaces = table_mut_or_insert(&mut doc, "marketplaces")?;
    for marketplace_name in marketplace_names {
        if marketplaces
            .get(marketplace_name)
            .and_then(Item::as_table)
            .is_none()
        {
            marketplaces[marketplace_name] = toml_edit::table();
        }
        marketplaces[marketplace_name]["source_type"] = toml_edit::value("local");
        marketplaces[marketplace_name]["source"] =
            toml_edit::value(marketplace_config_path(marketplace_root));
    }
    if !plugin_ids.is_empty() {
        let plugins = table_mut_or_insert(&mut doc, "plugins")?;
        for plugin_id in plugin_ids {
            let existing_enabled = plugins
                .get(plugin_id)
                .and_then(Item::as_table)
                .and_then(|table| table.get("enabled"))
                .and_then(Item::as_bool);
            if plugins.get(plugin_id).and_then(Item::as_table).is_none() {
                plugins[plugin_id] = toml_edit::table();
            }
            if existing_enabled.is_none() {
                plugins[plugin_id]["enabled"] = toml_edit::value(true);
            }
        }
    }
    Ok(ensure_trailing_newline(doc.to_string()))
}

fn marketplace_config_points_to_root(home: &Path, marketplace_name: &str, root: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(home.join("config.toml")) else {
        return false;
    };
    let Ok(doc) = text.trim_start_matches('\u{feff}').parse::<DocumentMut>() else {
        return false;
    };
    let Some(table) = doc
        .get("marketplaces")
        .and_then(Item::as_table)
        .and_then(|marketplaces| marketplaces.get(marketplace_name))
        .and_then(Item::as_table)
    else {
        return false;
    };
    let source_type = table
        .get("source_type")
        .and_then(Item::as_str)
        .unwrap_or_default();
    let source = table
        .get("source")
        .and_then(Item::as_str)
        .unwrap_or_default();
    source_type == "local" && marketplace_config_path_matches(source, root)
}

fn marketplace_config_path_matches(value: &str, path: &Path) -> bool {
    value == marketplace_config_path(path)
}

fn managed_marketplace_path_matches(value: &str, path: &Path) -> bool {
    let native = path.to_string_lossy();
    value == native || value.strip_prefix(r"\\?\") == Some(native.as_ref())
}

fn marketplace_config_path(path: &Path) -> String {
    let value = path.to_string_lossy();
    if !cfg!(windows) || value.starts_with(r"\\?\") {
        value.into_owned()
    } else {
        format!(r"\\?\{value}")
    }
}

fn parse_toml_document(contents: &str) -> anyhow::Result<DocumentMut> {
    let contents = contents.trim_start_matches('\u{feff}');
    if contents.trim().is_empty() {
        Ok(DocumentMut::new())
    } else {
        contents
            .parse::<DocumentMut>()
            .map_err(|error| anyhow::anyhow!("config.toml TOML parse failed: {error}"))
    }
}

fn table_mut_or_insert<'a>(doc: &'a mut DocumentMut, key: &str) -> anyhow::Result<&'a mut Table> {
    if !doc.as_table().contains_key(key) {
        doc[key] = toml_edit::table();
    }
    if doc.get(key).and_then(Item::as_table).is_none() {
        doc[key] = toml_edit::table();
    }
    doc.get_mut(key)
        .and_then(Item::as_table_mut)
        .ok_or_else(|| anyhow::anyhow!("{key} must be a TOML table"))
}

fn ensure_trailing_newline(mut contents: String) -> String {
    if !contents.ends_with('\n') {
        contents.push('\n');
    }
    contents
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expected_marketplace_path(path: &Path) -> String {
        if cfg!(windows) {
            format!(r"\\?\{}", path.display())
        } else {
            path.to_string_lossy().into_owned()
        }
    }

    fn write_marketplace(home: &Path) {
        let root = home.join(".tmp").join("plugins");
        std::fs::create_dir_all(root.join(".agents").join("plugins")).unwrap();
        std::fs::create_dir_all(root.join("plugins").join("gmail")).unwrap();
        std::fs::write(
            root.join(".agents")
                .join("plugins")
                .join("marketplace.json"),
            r#"{"name":"openai-curated","plugins":[{"name":"gmail","path":"./plugins/gmail"}]}"#,
        )
        .unwrap();
    }

    fn write_remote_marketplace(home: &Path) {
        let root = home.join(".tmp").join("plugins-remote");
        std::fs::create_dir_all(root.join(".agents").join("plugins")).unwrap();
        std::fs::create_dir_all(root.join("plugins").join("product-design")).unwrap();
        std::fs::write(
            root.join(".agents")
                .join("plugins")
                .join("marketplace.json"),
            r#"{"name":"openai-curated-remote","plugins":[{"name":"product-design","path":"./plugins/product-design"}]}"#,
        )
        .unwrap();
    }

    fn write_role_specific_marketplace(home: &Path) {
        let root = home
            .join(".tmp")
            .join("marketplaces")
            .join("role-specific-plugins");
        std::fs::create_dir_all(root.join(".agents").join("plugins")).unwrap();
        for plugin in [
            "sales",
            "data-analytics",
            "product-design",
            "financial-markets",
            "customer-support",
        ] {
            std::fs::create_dir_all(root.join("plugins").join(plugin)).unwrap();
        }
        std::fs::write(
            root.join(".agents")
                .join("plugins")
                .join("marketplace.json"),
            r#"{"name":"role-specific-plugins","plugins":[{"name":"sales"},{"name":"data-analytics"},{"name":"product-design"},{"name":"financial-markets"},{"name":"customer-support"}]}"#,
        )
        .unwrap();
    }

    #[test]
    fn ensure_openai_curated_marketplace_config_removes_managed_reserved_entries() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        write_marketplace(home);
        write_remote_marketplace(home);
        let plugins_source = home.join(".tmp").join("plugins").display().to_string();
        let extended_plugins_source = expected_marketplace_path(&home.join(".tmp").join("plugins"));
        std::fs::write(
            home.join("config.toml"),
            format!(
                r#"[marketplaces.openai-curated]
source_type = "local"
source = {}

[marketplaces.openai-api-curated]
source_type = "local"
source = {}
"#,
                toml_edit::value(plugins_source),
                toml_edit::value(extended_plugins_source),
            ),
        )
        .unwrap();

        let changed = ensure_openai_curated_marketplace_config(home).unwrap();

        assert!(changed);
        let config = std::fs::read_to_string(home.join("config.toml")).unwrap();
        let parsed = config.parse::<DocumentMut>().unwrap();
        assert!(parsed["marketplaces"].get("openai-curated").is_none());
        assert!(parsed["marketplaces"].get("openai-api-curated").is_none());
        assert_eq!(
            parsed["marketplaces"]["codex-plus-curated"]["source_type"].as_str(),
            Some("local")
        );
        assert_eq!(
            parsed["marketplaces"]["codex-plus-curated"]["source"].as_str(),
            Some(expected_marketplace_path(&home.join(".tmp").join("plugins-remote")).as_str())
        );
    }

    /// #2072：市场改名后，`x@openai-curated` 这类旧插件条目会悬空，
    /// codex 既不列在插件列表也不挂到会话工具上。清理市场条目时要一并迁移。
    #[test]
    fn cleanup_migrates_orphaned_reserved_marketplace_plugin_entries() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        write_marketplace(home);
        write_remote_marketplace(home);
        let plugins_source = home.join(".tmp").join("plugins").display().to_string();
        std::fs::write(
            home.join("config.toml"),
            format!(
                r#"[marketplaces.openai-curated]
source_type = "local"
source = {}

[plugins."product-design@openai-curated-remote"]
enabled = false

[plugins."ghost@openai-curated"]
enabled = true
"#,
                toml_edit::value(plugins_source),
            ),
        )
        .unwrap();

        let changed = cleanup_managed_reserved_marketplace_configs(home).unwrap();

        assert!(changed);
        let config = std::fs::read_to_string(home.join("config.toml")).unwrap();
        let parsed = config.parse::<DocumentMut>().unwrap();
        // product-design 在新市场清单里，迁移到新市场名并保留用户关掉的开关。
        assert_eq!(
            parsed["plugins"][format!("product-design@{CODEX_PLUS_MARKETPLACE}")]["enabled"]
                .as_bool(),
            Some(false)
        );
        assert!(
            parsed["plugins"]
                .get("product-design@openai-curated-remote")
                .is_none(),
            "旧市场名下的条目不应残留"
        );
        // ghost 不在任何托管市场清单里，悬空条目直接清掉。
        assert!(parsed["plugins"].get("ghost@openai-curated").is_none());
    }

    /// 磁盘上仍是旧的保留名时（升级后第一次启动），清理路径也必须能读到插件清单，
    /// 否则迁移会被 `available.is_empty()` 整段跳过（#2072）。
    #[test]
    fn cleanup_migrates_plugins_while_disk_still_has_the_legacy_marketplace_name() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        write_remote_marketplace(home);
        std::fs::write(
            home.join("config.toml"),
            "[plugins.\"product-design@openai-curated-remote\"]\nenabled = true\n",
        )
        .unwrap();

        let changed = cleanup_managed_reserved_marketplace_configs(home).unwrap();

        assert!(changed);
        let config = std::fs::read_to_string(home.join("config.toml")).unwrap();
        let parsed = config.parse::<DocumentMut>().unwrap();
        assert_eq!(
            parsed["plugins"][format!("product-design@{CODEX_PLUS_MARKETPLACE}")]["enabled"]
                .as_bool(),
            Some(true)
        );
    }

    #[test]
    fn cleanup_keeps_plugin_entries_when_no_managed_marketplace_is_installed() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        // 没有托管市场目录：无法判断插件是否存在，配置必须原样不动。
        std::fs::write(
            home.join("config.toml"),
            "[plugins.\"github@openai-curated\"]\nenabled = true\n",
        )
        .unwrap();

        let changed = cleanup_managed_reserved_marketplace_configs(home).unwrap();

        assert!(!changed);
        let config = std::fs::read_to_string(home.join("config.toml")).unwrap();
        assert!(config.contains("[plugins.\"github@openai-curated\"]"));
    }

    #[test]
    fn ensure_openai_curated_marketplace_config_preserves_user_reserved_entry() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        std::fs::write(
            home.join("config.toml"),
            r#"[marketplaces.openai-curated]
source_type = "local"
source = "/opt/user-marketplace"
"#,
        )
        .unwrap();

        let changed = ensure_openai_curated_marketplace_config(home).unwrap();

        assert!(!changed);
        let config = std::fs::read_to_string(home.join("config.toml")).unwrap();
        assert!(config.contains(r#"source = "/opt/user-marketplace""#));
    }

    #[test]
    fn ensure_openai_curated_marketplace_config_skips_when_snapshot_missing() {
        let temp = tempfile::tempdir().unwrap();

        let changed = ensure_openai_curated_marketplace_config(temp.path()).unwrap();

        assert!(!changed);
        assert!(!temp.path().join("config.toml").exists());
    }

    #[test]
    fn ensure_role_specific_plugins_marketplace_config_repairs_installed_plugin_entries() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        write_role_specific_marketplace(home);
        std::fs::write(
            home.join("config.toml"),
            "model_provider = \"custom\"\nexperimental_bearer_token = \"sk-redacted\"\n",
        )
        .unwrap();

        let changed = ensure_role_specific_plugins_marketplace_config(home).unwrap();

        assert!(changed);
        let config = std::fs::read_to_string(home.join("config.toml")).unwrap();
        let parsed = config.parse::<DocumentMut>().unwrap();
        assert_eq!(
            parsed["marketplaces"]["role-specific-plugins"]["source_type"].as_str(),
            Some("local")
        );
        assert_eq!(
            parsed["marketplaces"]["role-specific-plugins"]["source"].as_str(),
            Some(
                expected_marketplace_path(
                    &home
                        .join(".tmp")
                        .join("marketplaces")
                        .join("role-specific-plugins"),
                )
                .as_str(),
            )
        );
        for plugin in [
            "sales@role-specific-plugins",
            "data-analytics@role-specific-plugins",
            "product-design@role-specific-plugins",
            "financial-markets@role-specific-plugins",
            "customer-support@role-specific-plugins",
        ] {
            assert_eq!(parsed["plugins"][plugin]["enabled"].as_bool(), Some(true));
        }
        assert_eq!(
            parsed["experimental_bearer_token"].as_str(),
            Some("sk-redacted")
        );
    }

    #[test]
    fn ensure_role_specific_plugins_marketplace_config_preserves_disabled_plugin_choice() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        write_role_specific_marketplace(home);
        std::fs::write(
            home.join("config.toml"),
            "[plugins.\"sales@role-specific-plugins\"]\nenabled = false\n",
        )
        .unwrap();

        let changed = ensure_role_specific_plugins_marketplace_config(home).unwrap();

        assert!(changed);
        let config = std::fs::read_to_string(home.join("config.toml")).unwrap();
        let parsed = config.parse::<DocumentMut>().unwrap();
        assert_eq!(
            parsed["plugins"]["sales@role-specific-plugins"]["enabled"].as_bool(),
            Some(false)
        );
        assert_eq!(
            parsed["plugins"]["customer-support@role-specific-plugins"]["enabled"].as_bool(),
            Some(true)
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn ensure_marketplace_configs_migrate_legacy_windows_paths_on_unix() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        write_remote_marketplace(home);
        write_role_specific_marketplace(home);
        let remote_root = home.join(".tmp").join("plugins-remote");
        let role_root = home
            .join(".tmp")
            .join("marketplaces")
            .join("role-specific-plugins");
        std::fs::write(
            home.join("config.toml"),
            format!(
                r#"[marketplaces.openai-curated-remote]
source_type = "local"
source = '\\?\{}'

[marketplaces.role-specific-plugins]
source_type = "local"
source = '\\?\{}'
"#,
                remote_root.display(),
                role_root.display(),
            ),
        )
        .unwrap();

        assert!(ensure_openai_curated_remote_marketplace_config(home).unwrap());
        assert!(ensure_role_specific_plugins_marketplace_config(home).unwrap());

        let config = std::fs::read_to_string(home.join("config.toml")).unwrap();
        let parsed = config.parse::<DocumentMut>().unwrap();
        assert_eq!(
            parsed["marketplaces"]["codex-plus-curated"]["source"].as_str(),
            Some(remote_root.to_string_lossy().as_ref())
        );
        assert_eq!(
            parsed["marketplaces"]["role-specific-plugins"]["source"].as_str(),
            Some(role_root.to_string_lossy().as_ref())
        );
    }

    #[test]
    fn openai_curated_marketplace_status_accepts_absent_reserved_config() {
        let temp = tempfile::tempdir().unwrap();
        write_marketplace(temp.path());

        let status = openai_curated_marketplace_status(temp.path());

        assert!(status.marketplace_root.is_some());
        assert!(status.config_registered);
        assert!(!status.needs_repair());
    }

    #[test]
    fn openai_curated_marketplace_status_detects_managed_reserved_config() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let root = home.join(".tmp").join("plugins");
        write_marketplace(home);
        ensure_marketplace_configs(home, &[OPENAI_CURATED_MARKETPLACE], &root).unwrap();

        let status = openai_curated_marketplace_status(home);

        assert!(status.marketplace_root.is_some());
        assert!(!status.config_registered);
        assert!(status.needs_repair());
    }

    #[test]
    fn openai_curated_remote_marketplace_status_detects_cached_marketplace() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        write_remote_marketplace(home);

        let status = openai_curated_remote_marketplace_status(home);

        assert_eq!(
            status.marketplace_root,
            Some(home.join(".tmp").join("plugins-remote"))
        );
        assert!(!status.config_registered);
        assert!(status.needs_repair());
    }

    /// #2138：快照名不是写死常量时，config 注册名必须以快照真实 `name` 为准。
    /// 两边逐字不一致的话 codex 整个市场都不认，而且没有任何地方会报错。
    #[test]
    fn ensure_openai_curated_remote_marketplace_config_registers_snapshot_name() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        write_remote_marketplace(home);
        // 模拟快照里不是默认名的形态（历史版本/用户手工改过）。
        let marketplace_json = home
            .join(".tmp")
            .join("plugins-remote")
            .join(".agents")
            .join("plugins")
            .join("marketplace.json");
        std::fs::write(
            &marketplace_json,
            r#"{"name":"codex-plus-remote-test","plugins":[{"name":"product-design","path":"./plugins/product-design"}]}"#,
        )
        .unwrap();

        assert!(ensure_openai_curated_remote_marketplace_config(home).unwrap());

        let config = std::fs::read_to_string(home.join("config.toml")).unwrap();
        let parsed = config.parse::<DocumentMut>().unwrap();
        assert_eq!(
            parsed["marketplaces"]["codex-plus-remote-test"]["source_type"].as_str(),
            Some("local"),
            "注册名应取自磁盘快照：{config}"
        );
        assert!(
            parsed["marketplaces"].get("codex-plus-curated").is_none(),
            "写死常量不得再出现在注册表里：{config}"
        );
        // 幂等：第二次调用不应再判定为需要修复。
        assert!(!ensure_openai_curated_remote_marketplace_config(home).unwrap());
    }

    /// 快照仍是 codex 保留名时不能照抄进 config——注册在保留名下会被静默忽略
    /// （#1974 / #1968）。
    #[test]
    fn managed_marketplace_name_falls_back_for_reserved_snapshot_name() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("plugins-remote");
        std::fs::create_dir_all(root.join(".agents").join("plugins")).unwrap();
        std::fs::write(
            root.join(".agents")
                .join("plugins")
                .join("marketplace.json"),
            r#"{"name":"openai-api-curated","plugins":[{"name":"x"}]}"#,
        )
        .unwrap();

        assert_eq!(
            managed_marketplace_name_for_root(&root).unwrap(),
            CODEX_PLUS_MARKETPLACE
        );
    }

    /// 状态判定与写回必须用同一个名字，否则会把刚写好的配置判成未注册、
    /// 每次启动都触发一次无谓的修复写入。
    #[test]
    fn openai_curated_remote_marketplace_status_uses_snapshot_name() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        write_remote_marketplace(home);
        let marketplace_json = home
            .join(".tmp")
            .join("plugins-remote")
            .join(".agents")
            .join("plugins")
            .join("marketplace.json");
        std::fs::write(
            &marketplace_json,
            r#"{"name":"codex-plus-remote-test","plugins":[{"name":"product-design","path":"./plugins/product-design"}]}"#,
        )
        .unwrap();

        assert!(ensure_openai_curated_remote_marketplace_config(home).unwrap());

        let registered =
            managed_marketplace_name_for_root(&home.join(".tmp").join("plugins-remote")).unwrap();
        let config = std::fs::read_to_string(home.join("config.toml")).unwrap();
        assert!(
            config.contains(&format!("[marketplaces.{registered}]")),
            "注册名必须与快照名逐字一致：{config}"
        );

        let status = openai_curated_remote_marketplace_status(home);

        assert!(
            status.config_registered,
            "写入后状态判定必须为已注册（以快照真实 name 比对）"
        );
        assert!(!status.needs_repair());
    }

    #[test]
    fn ensure_openai_curated_remote_marketplace_config_registers_remote_only() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        write_remote_marketplace(home);

        let changed = ensure_openai_curated_remote_marketplace_config(home).unwrap();

        assert!(changed);
        let config = std::fs::read_to_string(home.join("config.toml")).unwrap();
        let parsed = config.parse::<DocumentMut>().unwrap();
        assert!(
            parsed
                .get("marketplaces")
                .and_then(Item::as_table)
                .and_then(|marketplaces| marketplaces.get("openai-curated"))
                .is_none()
        );
        assert_eq!(
            parsed["marketplaces"]["codex-plus-curated"]["source_type"].as_str(),
            Some("local")
        );
        assert_eq!(
            parsed["marketplaces"]["codex-plus-curated"]["source"].as_str(),
            Some(expected_marketplace_path(&home.join(".tmp").join("plugins-remote")).as_str())
        );
    }

    #[test]
    fn ensure_openai_curated_remote_marketplace_available_installs_embedded_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();

        let result = ensure_openai_curated_remote_marketplace_available(home).unwrap();

        assert!(result.initialized);
        assert!(result.configured);
        let root = home.join(".tmp").join("plugins-remote");
        assert!(
            root.join(".agents")
                .join("plugins")
                .join("marketplace.json")
                .is_file()
        );
        assert!(
            root.join("plugins")
                .join("product-design")
                .join(".codex-plugin")
                .join("plugin.json")
                .is_file()
        );
        let config = std::fs::read_to_string(home.join("config.toml")).unwrap();
        let parsed = config.parse::<DocumentMut>().unwrap();
        assert_eq!(
            parsed["marketplaces"]["codex-plus-curated"]["source_type"].as_str(),
            Some("local")
        );
        assert_eq!(
            parsed["marketplaces"]["codex-plus-curated"]["source"].as_str(),
            Some(expected_marketplace_path(&home.join(".tmp").join("plugins-remote")).as_str())
        );
    }

    /// codex 会静默忽略注册在保留名下的本地 marketplace——`codex plugin marketplace
    /// list` 直接不列出，用户在插件市场里看不到也装不了任何插件（#1974 / #1968）。
    ///
    /// 实测确认被忽略的名字：openai-curated、openai-curated-remote、openai-bundled、
    /// openai-bundled-alpha、openai-primary-runtime、openai-api-curated。
    /// 所以我们注册用的名字绝不能落在这个集合里。
    #[test]
    fn codex_plus_marketplace_name_is_not_a_codex_reserved_name() {
        const CODEX_RESERVED: [&str; 6] = [
            "openai-curated",
            "openai-curated-remote",
            "openai-bundled",
            "openai-bundled-alpha",
            "openai-primary-runtime",
            "openai-api-curated",
        ];
        assert!(
            !CODEX_RESERVED.contains(&CODEX_PLUS_MARKETPLACE),
            "{CODEX_PLUS_MARKETPLACE} 是 codex 的保留名，注册后会被静默忽略"
        );
        // codex 保留的是整个 openai-* 前缀，别再踩进去
        assert!(
            !CODEX_PLUS_MARKETPLACE.starts_with("openai-"),
            "不要用 openai- 前缀的 marketplace 名"
        );
        // 旧名当年就是踩了这个坑，保留它只为清理历史配置
        assert!(CODEX_RESERVED.contains(&LEGACY_REMOTE_MARKETPLACE));
    }

    /// 内置 zip 里自带的是保留名，解压后必须被改写，否则 codex 加载不到。
    /// 启动路径也必须改写磁盘上的 name，不能只写 config。
    ///
    /// 02a23e1 只在 repair_remote_plugin_marketplace（用户手动点「修复」才会调）
    /// 里做了迁移，正常启动走的 ensure_openai_curated_marketplace_config 没做，
    /// 于是老用户升级后 config 是新名、磁盘还是旧名，两边对不上 codex 整个市场
    /// 不认——表现和改名前完全一样，插件一个都装不了（#1993）。
    #[test]
    fn startup_path_rewrites_a_legacy_marketplace_name_on_disk() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        write_remote_marketplace(home);
        // 还原成老用户的状态：磁盘上是旧的保留名
        let manifest = home
            .join(".tmp")
            .join("plugins-remote")
            .join(".agents")
            .join("plugins")
            .join("marketplace.json");
        let raw = std::fs::read_to_string(&manifest).unwrap();
        assert!(
            raw.contains(LEGACY_REMOTE_MARKETPLACE),
            "前置条件：磁盘应为旧名"
        );

        ensure_openai_curated_marketplace_config(home).unwrap();

        let rewritten: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&manifest).unwrap()).unwrap();
        assert_eq!(
            rewritten["name"].as_str(),
            Some(CODEX_PLUS_MARKETPLACE),
            "启动路径没有改写磁盘上的 marketplace 名"
        );
        // config 与磁盘必须一致，否则 codex 不认
        let config = std::fs::read_to_string(home.join("config.toml")).unwrap();
        assert!(config.contains(&format!("[marketplaces.{CODEX_PLUS_MARKETPLACE}]")));
    }

    #[test]
    fn extracted_marketplace_name_is_rewritten_off_the_reserved_name() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let dir = root.join(".agents").join("plugins");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("marketplace.json"),
            r#"{"name":"openai-curated-remote","plugins":[{"name":"a","path":"./plugins/a"}]}"#,
        )
        .unwrap();

        rewrite_marketplace_name(root).unwrap();

        let parsed: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("marketplace.json")).unwrap())
                .unwrap();
        assert_eq!(parsed["name"].as_str(), Some(CODEX_PLUS_MARKETPLACE));
        // 插件清单不能在改名过程中丢掉
        assert_eq!(parsed["plugins"].as_array().map(Vec::len), Some(1));
    }

    #[test]
    fn zip_entry_relative_path_strips_archive_root_and_rejects_escape() {
        assert_eq!(
            zip_entry_relative_path("plugins-main/plugins/gmail/file.txt"),
            Some(PathBuf::from("plugins").join("gmail").join("file.txt"))
        );
        assert_eq!(zip_entry_relative_path("plugins-main/../evil.txt"), None);
        assert_eq!(zip_entry_relative_path("../evil.txt"), None);
    }

    #[test]
    fn install_openai_plugins_zip_installs_valid_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        let mut bytes = Cursor::new(Vec::<u8>::new());
        {
            let mut writer = zip::ZipWriter::new(&mut bytes);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            writer
                .start_file("plugins-main/.agents/plugins/marketplace.json", options)
                .unwrap();
            std::io::Write::write_all(
                &mut writer,
                br#"{"name":"openai-curated","plugins":[{"name":"gmail","path":"./plugins/gmail"}]}"#,
            )
            .unwrap();
            writer
                .start_file(
                    "plugins-main/plugins/gmail/.codex-plugin/plugin.json",
                    options,
                )
                .unwrap();
            std::io::Write::write_all(&mut writer, br#"{"name":"gmail"}"#).unwrap();
            writer.finish().unwrap();
        }

        install_openai_plugins_zip(temp.path(), bytes.get_ref()).unwrap();
        let changed = ensure_openai_curated_marketplace_config(temp.path()).unwrap();

        assert!(!changed);
        assert!(
            temp.path()
                .join(".tmp/plugins/.agents/plugins/marketplace.json")
                .is_file()
        );
        assert!(
            temp.path()
                .join(".tmp/plugins/plugins/gmail/.codex-plugin/plugin.json")
                .is_file()
        );
    }
}
