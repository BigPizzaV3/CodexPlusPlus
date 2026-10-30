use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::user_scripts::UserScriptManager;

pub const DEFAULT_MARKET_INDEX_URL: &str =
    "https://raw.githubusercontent.com/BigPizzaV3/CodexPlusPlusScriptMarket/main/index.json";

/// 官方拓展的仓库所有者。精选清单里标了 `official` 的条目，仓库也必须属于这里列出的账号或组织，
/// 才显示「官方」：清单写错或被误改时，别人的仓库也拿不到官方徽章。
pub const OFFICIAL_OWNERS: &[&str] = &["BigPizzaV3"];

const MARKET_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const MARKET_TOTAL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScriptMarketManifest {
    pub version: u64,
    pub updated_at: Option<String>,
    pub scripts: Vec<MarketScript>,
    /// 精选拓展包（清单里 `kind: "package"` 的条目），锁定到审核过的提交。
    pub packages: Vec<MarketPackage>,
    /// 下架名单：`owner/repo`。社区栏不显示，已装的提示停用。
    pub blocked: Vec<String>,
}

/// 精选清单里的一个拓展包。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MarketPackage {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub author: String,
    pub tags: Vec<String>,
    pub homepage: String,
    pub icon: String,
    pub updated_at: String,
    /// `owner/repo`。
    pub repo: String,
    /// 审核过的版本 tag（展示用），安装以 `sha` 为准。
    pub tag: Option<String>,
    /// 审核过的提交，安装只认它。
    pub sha: String,
    /// 清单声明为官方；还要仓库属于 [`OFFICIAL_OWNERS`] 才算官方。
    pub official_claimed: bool,
}

impl MarketPackage {
    pub fn owner(&self) -> &str {
        self.repo.split('/').next().unwrap_or("")
    }

    pub fn is_official(&self) -> bool {
        self.official_claimed && OFFICIAL_OWNERS.iter().any(|owner| owner.eq_ignore_ascii_case(self.owner()))
    }

    pub fn channel(&self) -> &'static str {
        if self.is_official() {
            crate::extension_discovery::CHANNEL_OFFICIAL
        } else {
            crate::extension_discovery::CHANNEL_CURATED
        }
    }

    pub fn pinned(&self) -> crate::extension_discovery::PinnedPackage {
        crate::extension_discovery::PinnedPackage {
            repo: self.repo.clone(),
            id: self.id.clone(),
            tag: self.tag.clone(),
            sha: self.sha.clone(),
            channel: self.channel().to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarketScript {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub version: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub homepage: String,
    pub script_url: String,
    /// 清单里的可选字段，详情页展示用；老条目没有则为空。
    #[serde(default)]
    pub requirements: Vec<String>,
    #[serde(default)]
    pub limitations: Vec<String>,
    /// 可选图标 URL。清单目前还没提供，留好通路：有就用，没有回退默认字形。
    #[serde(default)]
    pub icon: String,
    /// 该脚本最近一次更新的时间（ISO 8601，可只写日期），由市场清单逐条提供（#2781）。
    /// 老清单没有则为空，前端不显示、按更新时间排序时排到末尾。
    #[serde(default)]
    pub updated_at: String,
}

pub fn parse_market_manifest(raw: Value) -> anyhow::Result<ScriptMarketManifest> {
    let version = raw.get("version").and_then(Value::as_u64).unwrap_or(1);
    let updated_at = raw
        .get("updated_at")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let entries = raw.get("scripts").and_then(Value::as_array).cloned().unwrap_or_default();
    let is_package = |entry: &Value| entry.get("kind").and_then(Value::as_str) == Some("package");
    let scripts = entries.iter().filter(|entry| !is_package(entry)).cloned().filter_map(parse_market_script).collect();
    // 拓展包既可以混在 scripts 里（kind: "package"），也可以单独放在 packages 里。
    let packages = entries
        .iter()
        .filter(|entry| is_package(entry))
        .chain(raw.get("packages").and_then(Value::as_array).into_iter().flatten())
        .filter_map(parse_market_package)
        .collect();
    let blocked = string_list(&raw, "blocked")
        .into_iter()
        .filter(|repo| repo.split('/').count() == 2)
        .collect();

    Ok(ScriptMarketManifest {
        version,
        updated_at,
        scripts,
        packages,
        blocked,
    })
}

/// 脚本市场固定访问 GitHub raw，必须走系统代理 / 环境变量代理，
/// 否则需要代理的网络里会一直连不上（#1329 / #1339）。
fn market_http_client(url: &str) -> anyhow::Result<reqwest::Client> {
    crate::http_client::proxied_client_for_url_with_timeouts(
        "CodexPlusPlus/script-market",
        url,
        MARKET_CONNECT_TIMEOUT,
        MARKET_TOTAL_TIMEOUT,
    )
    .context("无法建立脚本市场连接")
}

pub async fn fetch_market_manifest(url: &str) -> anyhow::Result<ScriptMarketManifest> {
    let raw = market_http_client(url)?
        .get(url)
        .send()
        .await
        .with_context(|| format!("failed to request script market index {url}"))?
        .error_for_status()
        .with_context(|| format!("script market index returned an error status {url}"))?
        .json::<Value>()
        .await
        .context("failed to decode script market index JSON")?;
    parse_market_manifest(raw)
}

pub async fn download_script(url: &str) -> anyhow::Result<Vec<u8>> {
    Ok(market_http_client(url)?
        .get(url)
        .send()
        .await
        .with_context(|| format!("failed to request script {url}"))?
        .error_for_status()
        .with_context(|| format!("script download returned an error status {url}"))?
        .bytes()
        .await
        .context("failed to read script download body")?
        .to_vec())
}

pub fn install_market_script_content(
    manager: &UserScriptManager,
    script: &MarketScript,
    content: &[u8],
) -> anyhow::Result<()> {
    let path = manager.user_script_path_for_market_id(&script.id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create user script directory {}",
                parent.display()
            )
        })?;
    }
    crate::settings::atomic_write(&path, content)
        .with_context(|| format!("failed to write script {}", path.display()))?;
    manager.record_market_install(script)?;
    Ok(())
}

pub async fn install_market_script(
    manager: &UserScriptManager,
    script: &MarketScript,
) -> anyhow::Result<()> {
    let content = download_script(&script.script_url).await?;
    install_market_script_content(manager, script, &content)
}

/// 拉清单并合并本地状态。两个 `BridgeRuntimeService` 实现共用，避免各写一遍。
pub async fn list_market_scripts(manager: &UserScriptManager) -> anyhow::Result<Value> {
    let manifest = cached_market_manifest(true).await?;
    Ok(market_scripts_payload(
        manager,
        &manifest,
        "ok",
        "脚本市场已刷新。",
    ))
}

/// 按 id 安装，返回合并后的市场清单 + 本地脚本清单。
///
/// 带上本地清单是为了让前端装完即可刷新「已安装」分组，省一次往返。
pub async fn install_market_script_by_id(
    manager: &UserScriptManager,
    id: &str,
) -> anyhow::Result<Value> {
    let manifest = fetch_market_manifest(DEFAULT_MARKET_INDEX_URL).await?;
    let Some(script) = manifest.scripts.iter().find(|script| script.id == id) else {
        anyhow::bail!("市场清单中未找到该脚本");
    };
    install_market_script(manager, script).await?;
    let mut result = market_scripts_payload(manager, &manifest, "ok", "脚本已安装。");
    result["user_scripts"] = manager.inventory()?;
    Ok(result)
}

/// 把市场清单与本地已安装状态合成给前端的 payload。
///
/// 已安装状态来自 `UserScriptManager` 的清单（`installed` / `version` 字段），
/// 这样前端不必自己比对，也能顺带算出「有更新」。
pub fn market_scripts_payload(
    manager: &UserScriptManager,
    manifest: &ScriptMarketManifest,
    status: &str,
    message: &str,
) -> Value {
    let installed = manager
        .inventory()
        .ok()
        .map(|inventory| installed_market_versions(&inventory))
        .unwrap_or_default();
    let scripts = manifest
        .scripts
        .iter()
        .map(|script| {
            let local_version = installed.get(script.id.as_str());
            serde_json::json!({
                "id": script.id,
                "name": script.name,
                "description": script.description,
                "version": script.version,
                "author": script.author,
                "tags": script.tags,
                "homepage": script.homepage,
                "requirements": script.requirements,
                "limitations": script.limitations,
                "icon": script.icon,
                "updatedAt": script.updated_at,
                "installed": local_version.is_some(),
                "installedVersion": local_version.cloned().unwrap_or_default(),
                "updateAvailable": local_version
                    .map(|version| version.as_str() != script.version)
                    .unwrap_or(false),
            })
        })
        .collect::<Vec<_>>();
    let (packages, blocked_installed) = market_packages_payload(manager, manifest);
    serde_json::json!({
        "status": status,
        "message": message,
        "indexUrl": DEFAULT_MARKET_INDEX_URL,
        "updatedAt": manifest.updated_at.clone().unwrap_or_default(),
        "scripts": scripts,
        "packages": packages,
        "blocked": manifest.blocked,
        "blockedInstalled": blocked_installed,
    })
}

/// 从用户脚本地盘清单里取出 `market_id -> version`，用于判断已安装与有更新。
fn installed_market_versions(
    inventory: &Value,
) -> std::collections::BTreeMap<String, String> {
    inventory
        .get("scripts")
        .and_then(Value::as_array)
        .map(|scripts| {
            scripts
                .iter()
                .filter_map(|script| {
                    let id = script.get("market_id").and_then(Value::as_str)?.trim();
                    if id.is_empty() || script.get("installed") != Some(&Value::Bool(true)) {
                        return None;
                    }
                    let version = script
                        .get("version")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    Some((id.to_string(), version.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_market_package(raw: &Value) -> Option<MarketPackage> {
    let id = required_string(raw, "id")?;
    let repo = required_string(raw, "repo")?;
    let sha = required_string(raw, "sha")?;
    let valid_repo = repo.split('/').count() == 2 && repo.split('/').all(|part| !part.is_empty());
    let valid_sha = sha.len() == 40 && sha.chars().all(|ch| ch.is_ascii_hexdigit());
    if !valid_repo || !valid_sha {
        return None;
    }
    Some(MarketPackage {
        name: required_string(raw, "name").unwrap_or_else(|| id.clone()),
        id,
        description: optional_string(raw, "description"),
        version: optional_string(raw, "version"),
        author: optional_string(raw, "author"),
        tags: string_list(raw, "tags"),
        homepage: optional_string(raw, "homepage"),
        icon: optional_string(raw, "icon"),
        updated_at: market_script_updated_at(raw),
        repo,
        tag: Some(optional_string(raw, "tag")).filter(|tag| !tag.is_empty()),
        sha: sha.to_lowercase(),
        official_claimed: raw.get("official").and_then(Value::as_bool).unwrap_or(false),
    })
}

/// 最近一次成功拉到的清单。精选栏、社区栏的下架过滤都用它；10 分钟内不重复拉，拉失败时退回它。
static MANIFEST_CACHE: std::sync::Mutex<Option<(std::time::Instant, ScriptMarketManifest)>> = std::sync::Mutex::new(None);
const MANIFEST_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(600);

/// 维护者刚写进市场仓库的清单。raw.githubusercontent 有几分钟 CDN 缓存，这段时间内以它为准，
/// 免得刚收录的拓展在刷新后又消失。
static PUBLISHED_MANIFEST: std::sync::Mutex<Option<(std::time::Instant, ScriptMarketManifest)>> = std::sync::Mutex::new(None);
const PUBLISHED_MANIFEST_TTL: std::time::Duration = std::time::Duration::from_secs(600);

/// 记下刚写进市场仓库的清单（精选收录 / 更新后调用）。
pub fn remember_published_manifest(manifest: ScriptMarketManifest) {
    *MANIFEST_CACHE.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some((std::time::Instant::now(), manifest.clone()));
    *PUBLISHED_MANIFEST.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some((std::time::Instant::now(), manifest));
}

/// 带缓存地取清单。`force` 时跳过缓存；拉取失败且有旧清单时返回旧清单。
pub async fn cached_market_manifest(force: bool) -> anyhow::Result<ScriptMarketManifest> {
    if let Some((at, manifest)) = PUBLISHED_MANIFEST.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).as_ref() {
        if at.elapsed() < PUBLISHED_MANIFEST_TTL {
            return Ok(manifest.clone());
        }
    }
    if !force {
        if let Some((at, manifest)) = MANIFEST_CACHE.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).as_ref() {
            if at.elapsed() < MANIFEST_CACHE_TTL {
                return Ok(manifest.clone());
            }
        }
    }
    match fetch_market_manifest(DEFAULT_MARKET_INDEX_URL).await {
        Ok(manifest) => {
            *MANIFEST_CACHE.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some((std::time::Instant::now(), manifest.clone()));
            Ok(manifest)
        }
        Err(error) => match MANIFEST_CACHE.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).as_ref() {
            Some((_, manifest)) => Ok(manifest.clone()),
            None => Err(error),
        },
    }
}

/// 精选清单给社区栏的约束。
pub fn curated_filter(manifest: &ScriptMarketManifest) -> crate::extension_discovery::CuratedFilter {
    crate::extension_discovery::CuratedFilter {
        blocked: manifest.blocked.clone(),
        curated_repos: manifest.packages.iter().map(|package| package.repo.clone()).collect(),
        curated_ids: manifest
            .packages
            .iter()
            .map(|package| (package.id.clone(), (package.repo.clone(), package.is_official())))
            .collect(),
        official_owners: OFFICIAL_OWNERS.iter().map(|owner| owner.to_string()).collect(),
    }
}

/// 精选拓展包与下架提示给前端的部分。已安装状态来自社区安装记录（同仓库才算）。
pub fn market_packages_payload(manager: &UserScriptManager, manifest: &ScriptMarketManifest) -> (Vec<Value>, Vec<Value>) {
    let config_dir = manager.user_dir().parent().map(std::path::Path::to_path_buf).unwrap_or_default();
    let store = crate::extension_discovery::CommunityStore::new(config_dir);
    let installed = crate::extension_discovery::installed_records(&store, manager);
    let mut packages = manifest
        .packages
        .iter()
        .map(|package| {
            let record = installed.get(&package.id).filter(|record| record.repo.eq_ignore_ascii_case(&package.repo));
            // 没有安装记录时，本机可能就是它的源码（自己发布的）。
            let local_source = record.is_none().then(|| crate::extension_discovery::local_source_version(manager, &package.id, &package.repo)).flatten();
            serde_json::json!({
                "kind": "package",
                "id": package.id,
                "name": package.name,
                "description": package.description,
                "version": package.version,
                "author": package.author,
                "tags": package.tags,
                "homepage": package.homepage,
                "icon": package.icon,
                "updatedAt": package.updated_at,
                "repo": package.repo,
                "tag": package.tag,
                "sha": package.sha,
                "official": package.is_official(),
                "channel": package.channel(),
                "installed": record.is_some() || local_source.is_some(),
                "localSource": local_source.is_some(),
                "installedVersion": record.map(|record| record.version.clone()).or(local_source).unwrap_or_default(),
                "installedSha": record.map(|record| record.sha.clone()).unwrap_or_default(),
                "updateAvailable": record.is_some_and(|record| !record.sha.eq_ignore_ascii_case(&package.sha)),
            })
        })
        .collect::<Vec<_>>();
    // 官方排在精选前面。
    packages.sort_by_key(|package| package["official"] != true);
    let blocked_installed = installed
        .iter()
        .filter(|(_, record)| manifest.blocked.iter().any(|repo| repo.eq_ignore_ascii_case(&record.repo)))
        .map(|(id, record)| serde_json::json!({ "id": id, "repo": record.repo }))
        .collect();
    (packages, blocked_installed)
}

fn parse_market_script(raw: Value) -> Option<MarketScript> {
    let id = required_string(&raw, "id")?;
    let name = required_string(&raw, "name")?;
    let version = required_string(&raw, "version")?;
    let script_url = required_string(&raw, "script_url")?;
    Some(MarketScript {
        id,
        name,
        description: optional_string(&raw, "description"),
        version,
        author: optional_string(&raw, "author"),
        tags: raw
            .get("tags")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        homepage: optional_string(&raw, "homepage"),
        script_url,
        requirements: string_list(&raw, "requirements"),
        limitations: string_list(&raw, "limitations"),
        icon: optional_string(&raw, "icon"),
        updated_at: market_script_updated_at(&raw),
    })
}

/// 逐条脚本的更新时间。清单手工维护，兼容 `updated_at` / `updatedAt` 两种写法；
/// 不在这里校验格式，前端解析失败时按「未标注」处理。
fn market_script_updated_at(raw: &Value) -> String {
    ["updated_at", "updatedAt"]
        .iter()
        .map(|key| optional_string(raw, key))
        .find(|value| !value.is_empty())
        .unwrap_or_default()
}

/// 取字符串数组字段，去掉空项。清单里 requirements/limitations 都是可选的。
fn string_list(raw: &Value, key: &str) -> Vec<String> {
    raw.get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn required_string(raw: &Value, key: &str) -> Option<String> {
    raw.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn optional_string(raw: &Value, key: &str) -> String {
    raw.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string()
}
