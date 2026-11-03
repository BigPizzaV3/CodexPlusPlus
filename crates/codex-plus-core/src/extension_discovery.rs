//! 社区拓展：GitHub 上打了 `codexpp-extension` topic 的拓展包仓库。
//!
//! 列表来自 GitHub 搜索，逐个读仓库默认分支的 `codexpp.json`，内存外再存一份磁盘缓存（10 分钟）。
//! 安装分两步：`prepare` 解析版本（release → tag → 默认分支）、按提交下载、解包校验，把实际要装的
//! 版本的权限交给界面确认；`install` 再把暂存的包换进用户拓展目录。安装来源记在
//! `extension_community.json`，不动 `user_scripts.json` 的结构。
//!
//! 设计见 `docs/plans/2026-10-30-extension-publish-discovery-design.md`。

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, bail};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::user_scripts::{
    EXTENSION_API_VERSION, EXTENSION_MANIFEST_FILE, UserScriptManager, extension_package_key,
    validate_extension_id,
};

/// 让仓库出现在社区栏的 GitHub topic。
pub const TOPIC: &str = "codexpp-extension";
/// 列表缓存多久（秒）：短到新打 topic 的仓库几分钟内可见，长到不撞搜索 API 的限额。
pub const CACHE_TTL_SECS: u64 = 600;
/// 解包后的上限。
pub const PACKAGE_MAX_FILES: usize = 2000;
pub const PACKAGE_MAX_BYTES: u64 = 10 * 1024 * 1024;
/// 下载的压缩包上限。
const DOWNLOAD_MAX_BYTES: usize = 20 * 1024 * 1024;
const SEARCH_PAGE: usize = 100;
/// GitHub 搜索最多给 1000 条。
const SEARCH_MAX_RESULTS: usize = 1000;
const MANIFEST_CONCURRENCY: usize = 8;
const MANIFEST_TIMEOUT: Duration = Duration::from_secs(15);
/// 国内直连 api.github.com 单页可能要几十秒。
const SEARCH_TIMEOUT: Duration = Duration::from_secs(30);
const MANIFEST_MAX_BYTES: usize = 256 * 1024;

const STATE_FILE: &str = "extension_community.json";
const CACHE_FILE: &str = "extension_discovery.json";
const BACKUP_DIR: &str = "extension_backups";
/// 暂存目录放在配置目录下，和用户拓展目录同盘，安装时可以直接改名。
const STAGING_DIR: &str = ".extension_staging";
const STAGING_META: &str = "prepare.json";

/// GitHub 的几个地址。测试指向本地夹具。
#[derive(Debug, Clone)]
pub struct Endpoints {
    pub api: String,
    pub raw: String,
    pub codeload: String,
    /// 只发往 `api`，用于提高限额。
    pub token: Option<String>,
}

impl Endpoints {
    pub fn github() -> Self {
        Self {
            api: "https://api.github.com".into(),
            raw: "https://raw.githubusercontent.com".into(),
            codeload: "https://codeload.github.com".into(),
            token: std::env::var("GITHUB_TOKEN")
                .or_else(|_| std::env::var("GH_TOKEN"))
                .ok()
                .map(|token| token.trim().to_string())
                .filter(|token| !token.is_empty()),
        }
    }

    /// 换一个 API 令牌（维护者用本机 gh 的令牌，限额高），只发往 `api`。
    pub fn with_token(mut self, token: Option<String>) -> Self {
        if token.is_some() {
            self.token = token;
        }
        self
    }

    fn client(&self) -> anyhow::Result<reqwest::Client> {
        let agent = format!("Codex++/{}", crate::version::VERSION);
        if self.api.starts_with("https://") {
            crate::http_client::proxied_client_for_url_with_timeouts(
                &agent,
                &self.api,
                Duration::from_secs(10),
                Duration::from_secs(120),
            )
        } else {
            Ok(reqwest::Client::builder().user_agent(agent).timeout(Duration::from_secs(30)).build()?)
        }
    }

    fn api_get(&self, client: &reqwest::Client, path: &str) -> reqwest::RequestBuilder {
        let request = client
            .get(format!("{}{path}", self.api))
            .header(reqwest::header::ACCEPT, "application/vnd.github+json");
        match &self.token {
            Some(token) => request.header(reqwest::header::AUTHORIZATION, format!("Bearer {token}")),
            None => request,
        }
    }
}

// ---------------------------------------------------------------------------
// 列表
// ---------------------------------------------------------------------------

/// 搜索结果里的一个仓库。
#[derive(Debug, Clone, PartialEq)]
pub struct SearchRepo {
    pub repo: String,
    pub url: String,
    pub owner: String,
    pub owner_avatar: String,
    pub description: String,
    pub stars: u64,
    pub license: String,
    pub default_branch: String,
    pub pushed_at: String,
}

/// 社区栏的一个拓展：仓库信息加默认分支上的清单。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityEntry {
    pub repo: String,
    pub url: String,
    pub owner: String,
    #[serde(default)]
    pub owner_avatar: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub stars: u64,
    #[serde(default)]
    pub license: String,
    pub default_branch: String,
    #[serde(default)]
    pub pushed_at: String,
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub homepage: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub keywords: Vec<String>,
    /// 整数表示拓展 API 版本，字符串表示 Codex++ 的 semver 范围。
    pub engine: Value,
    /// 本机拓展 API 或 Codex++ 版本满足 `engines.codexpp`。
    pub compatible: bool,
    /// 默认分支清单声明的能力；实际安装版本的权限在 prepare 时重新读。
    pub permissions: Value,
}

/// 解析一页搜索结果：去掉归档和 fork 的仓库。返回仓库与总数。
pub fn parse_search_page(raw: &Value) -> anyhow::Result<(Vec<SearchRepo>, usize)> {
    let total = raw.get("total_count").and_then(Value::as_u64).unwrap_or(0) as usize;
    let items = raw.get("items").and_then(Value::as_array).context("搜索结果缺少 items")?;
    let repos = items
        .iter()
        .filter(|item| {
            !item.get("archived").and_then(Value::as_bool).unwrap_or(false)
                && !item.get("fork").and_then(Value::as_bool).unwrap_or(false)
        })
        .filter_map(|item| {
            let repo = item.get("full_name")?.as_str()?.to_string();
            if repo.split('/').count() != 2 {
                return None;
            }
            let text = |key: &str| item.get(key).and_then(Value::as_str).unwrap_or("").to_string();
            let owner = item.get("owner").cloned().unwrap_or(Value::Null);
            Some(SearchRepo {
                url: text("html_url"),
                owner: owner.get("login").and_then(Value::as_str).unwrap_or("").to_string(),
                owner_avatar: owner.get("avatar_url").and_then(Value::as_str).unwrap_or("").to_string(),
                description: text("description"),
                stars: item.get("stargazers_count").and_then(Value::as_u64).unwrap_or(0),
                // GitHub 认不出的许可证给 NOASSERTION，当作没有，回退到清单里的 license。
                license: item
                    .get("license")
                    .and_then(|license| license.get("spdx_id"))
                    .and_then(Value::as_str)
                    .filter(|id| *id != "NOASSERTION")
                    .unwrap_or("")
                    .to_string(),
                default_branch: item
                    .get("default_branch")
                    .and_then(Value::as_str)
                    .unwrap_or("main")
                    .to_string(),
                pushed_at: text("pushed_at"),
                repo,
            })
        })
        .collect();
    Ok((repos, total))
}

/// 清单声明了哪些能力，按安装确认框的展示顺序给出。
pub fn manifest_permissions(manifest: &Value) -> Value {
    let strings = |value: Option<&Value>| -> Vec<String> {
        match value {
            Some(Value::String(text)) => vec![text.clone()],
            Some(Value::Array(items)) => items.iter().filter_map(Value::as_str).map(str::to_string).collect(),
            _ => Vec::new(),
        }
    };
    let mut hooks = Vec::new();
    if let Some(map) = manifest.get("hooks").and_then(Value::as_object) {
        for (event, items) in map {
            for item in items.as_array().into_iter().flatten() {
                let matcher = item.get("matcher").and_then(Value::as_str).unwrap_or("");
                hooks.push(json!({ "event": event, "matcher": matcher }));
            }
        }
    }
    let gateway = |key: &str| {
        manifest
            .get("gateway")
            .and_then(|gateway| gateway.get(key))
            .and_then(Value::as_array)
            .map_or(0, Vec::len)
    };
    let tools = manifest
        .get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|tool| tool.get("name").and_then(Value::as_str).map(str::to_string))
        .collect::<Vec<_>>();
    json!({
        "backend": manifest.get("backend").and_then(|backend| backend.get("script")).and_then(Value::as_str),
        "hooks": hooks,
        "gatewayRequest": gateway("request"),
        "gatewayResponse": gateway("response"),
        "tools": tools,
        "routes": strings(manifest.get("permissions").and_then(|permissions| permissions.get("routes"))),
        "renderer": strings(manifest.get("renderer")),
    })
}

/// 由搜索结果与默认分支上的清单组成一条社区拓展；清单不合格时报错（列表里不显示）。
pub fn community_entry(repo: &SearchRepo, raw_base: &str, manifest: &Value) -> anyhow::Result<CommunityEntry> {
    let map = manifest.as_object().with_context(|| format!("{EXTENSION_MANIFEST_FILE} 顶层必须是对象"))?;
    let text = |key: &str| map.get(key).and_then(Value::as_str).map(str::trim).unwrap_or("").to_string();
    let id = text("id");
    validate_extension_id(&id)?;
    let version = text("version");
    if version.is_empty() {
        bail!("清单缺少 version");
    }
    let engine = map
        .get("engines")
        .and_then(|engines| engines.get("codexpp"))
        .cloned()
        .unwrap_or_else(|| json!(EXTENSION_API_VERSION));
    let compatible = match &engine {
        Value::String(range) => crate::extension_semver::satisfies(
            crate::extension_semver::codexpp_version(),
            range,
        )
        .context("engines.codexpp 版本范围无效")?,
        value => {
            value.as_u64().context("engines.codexpp 必须是正整数或 semver 范围字符串")?
                <= EXTENSION_API_VERSION
        }
    };
    let icon = text("icon");
    // 图标只认 GitHub 上的地址：任意外链会让每个打开市场的人都去访问作者指定的主机。
    let icon = if is_github_asset_url(&icon) {
        icon
    } else if !icon.is_empty() && !icon.contains("..") && !icon.starts_with('/') && !icon.contains(':') {
        format!("{raw_base}/{}/{}/{icon}", repo.repo, repo.default_branch)
    } else {
        String::new()
    };
    let name = text("name");
    let description = text("description");
    let license = text("license");
    Ok(CommunityEntry {
        repo: repo.repo.clone(),
        url: repo.url.clone(),
        owner: repo.owner.clone(),
        owner_avatar: repo.owner_avatar.clone(),
        description: if description.is_empty() { repo.description.clone() } else { description },
        stars: repo.stars,
        license: if repo.license.is_empty() { license } else { repo.license.clone() },
        default_branch: repo.default_branch.clone(),
        pushed_at: repo.pushed_at.clone(),
        name: if name.is_empty() { id.clone() } else { name },
        version,
        author: text("author"),
        homepage: text("homepage"),
        icon,
        keywords: map
            .get("keywords")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        compatible,
        engine,
        permissions: manifest_permissions(manifest),
        id,
    })
}

fn is_github_asset_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    host == "github.com" || host == "raw.githubusercontent.com" || host.ends_with(".githubusercontent.com")
}

/// 搜索全部打了 topic 的仓库并读各自的清单。第二项是读不到的仓库（网络问题），下次再试。
pub async fn search(endpoints: &Endpoints) -> anyhow::Result<(Vec<CommunityEntry>, Vec<String>)> {
    let client = endpoints.client()?;
    let query = format!("topic:{TOPIC} archived:false fork:false");
    let mut repos = Vec::new();
    for page in 1..=SEARCH_MAX_RESULTS / SEARCH_PAGE {
        let response = endpoints
            .api_get(&client, "/search/repositories")
            .query(&[
                ("q", query.as_str()),
                ("sort", "stars"),
                ("order", "desc"),
                ("per_page", &SEARCH_PAGE.to_string()),
                ("page", &page.to_string()),
            ])
            .timeout(SEARCH_TIMEOUT)
            .send()
            .await
            .context("连不上 GitHub 搜索")?;
        let raw = github_json(response).await?;
        let (items, total) = parse_search_page(&raw)?;
        let count = raw.get("items").and_then(Value::as_array).map_or(0, Vec::len);
        repos.extend(items);
        if count < SEARCH_PAGE || page * SEARCH_PAGE >= total {
            break;
        }
    }
    let reads = futures_util::stream::iter(repos.into_iter().map(|repo| {
        let client = client.clone();
        async move {
            let result = read_default_manifest(&client, endpoints, &repo).await;
            (repo, result)
        }
    }))
    .buffered(MANIFEST_CONCURRENCY)
    .collect::<Vec<_>>()
    .await;
    let mut entries = Vec::new();
    let mut problems = Vec::new();
    let mut seen = BTreeSet::new();
    for (repo, result) in reads {
        match result {
            Ok(Some(manifest)) => {
                // 清单不合格的仓库不是可装的拓展，静默跳过。
                if let Ok(entry) = community_entry(&repo, &endpoints.raw, &manifest) {
                    if seen.insert(entry.repo.clone()) {
                        entries.push(entry);
                    }
                }
            }
            Ok(None) => {}
            Err(error) => problems.push(format!("{}：{error:#}", repo.repo)),
        }
    }
    Ok((entries, problems))
}

/// 默认分支上的 codexpp.json；仓库没有这个文件时为 None。
async fn read_default_manifest(
    client: &reqwest::Client,
    endpoints: &Endpoints,
    repo: &SearchRepo,
) -> anyhow::Result<Option<Value>> {
    let url = format!("{}/{}/{}/{EXTENSION_MANIFEST_FILE}", endpoints.raw, repo.repo, repo.default_branch);
    let response = client.get(&url).timeout(MANIFEST_TIMEOUT).send().await?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let response = response.error_for_status()?;
    let bytes = read_limited(response, MANIFEST_MAX_BYTES).await?;
    Ok(serde_json::from_slice(&bytes).ok())
}

async fn github_json(response: reqwest::Response) -> anyhow::Result<Value> {
    let status = response.status();
    if status.is_success() {
        return response.json::<Value>().await.context("GitHub 返回的不是 JSON");
    }
    let limited = response
        .headers()
        .get("x-ratelimit-remaining")
        .and_then(|value| value.to_str().ok())
        == Some("0");
    if limited || status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        bail!("GitHub API 限流，请稍后再试；设置 GITHUB_TOKEN 环境变量可以提高限额");
    }
    let body = response.text().await.unwrap_or_default();
    let message = serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|value| value.get("message").and_then(Value::as_str).map(str::to_string))
        .unwrap_or(body);
    bail!("GitHub 返回 HTTP {}：{}", status.as_u16(), message.chars().take(200).collect::<String>())
}

async fn read_limited(mut response: reqwest::Response, limit: usize) -> anyhow::Result<Vec<u8>> {
    if response.content_length().is_some_and(|length| length as usize > limit) {
        bail!("文件超过 {} KiB", limit / 1024);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        bytes.extend_from_slice(&chunk);
        if bytes.len() > limit {
            bail!("文件超过 {} KiB", limit / 1024);
        }
    }
    Ok(bytes)
}

// ---------------------------------------------------------------------------
// 本地状态：开关、安装来源、列表缓存
// ---------------------------------------------------------------------------

/// 一次从 GitHub 安装的记录。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallRecord {
    pub repo: String,
    pub version: String,
    #[serde(default)]
    pub tag: Option<String>,
    pub sha: String,
    /// 作者没发布版本，装的是默认分支当时的提交。
    #[serde(default)]
    pub from_default_branch: bool,
    /// 从 release 的包文件安装（计入下载量），否则从 codeload 按提交下载。
    #[serde(default)]
    pub from_asset: bool,
    pub installed_at: u64,
    /// 安装渠道：`community`（社区栏，未审核）、`curated`（精选清单，锁定到审核过的提交）、`official`（官方）。
    #[serde(default = "community_channel")]
    pub channel: String,
}

fn community_channel() -> String {
    CHANNEL_COMMUNITY.to_string()
}

pub const CHANNEL_COMMUNITY: &str = "community";
pub const CHANNEL_CURATED: &str = "curated";
pub const CHANNEL_OFFICIAL: &str = "official";
/// 只用于展示：本机就是这个仓库的源码，不是从 GitHub 装的。
pub const CHANNEL_LOCAL: &str = "local";

/// 精选清单给社区栏的约束：下架名单、已收进精选的仓库（社区栏不再重复显示）、精选拓展的 id。
#[derive(Debug, Clone, Default)]
pub struct CuratedFilter {
    /// `owner/repo`，小写比较。
    pub blocked: Vec<String>,
    pub curated_repos: Vec<String>,
    /// 精选拓展 id → (仓库, 是否官方)。
    pub curated_ids: BTreeMap<String, (String, bool)>,
    /// 官方所有者（小写）。
    pub official_owners: Vec<String>,
}

impl CuratedFilter {
    fn has_repo(list: &[String], repo: &str) -> bool {
        list.iter().any(|item| item.eq_ignore_ascii_case(repo))
    }

    pub fn is_blocked(&self, repo: &str) -> bool {
        Self::has_repo(&self.blocked, repo)
    }

    /// 社区条目的警示：与精选 / 官方拓展同 id、非官方仓库自称官方。
    pub fn warnings(&self, entry: &CommunityEntry) -> Vec<String> {
        let mut warnings = Vec::new();
        if let Some((repo, official)) = self.curated_ids.get(&entry.id) {
            if !repo.eq_ignore_ascii_case(&entry.repo) {
                warnings.push(if *official {
                    format!("与官方拓展同 id（官方版本来自 {repo}），可能是仿冒")
                } else {
                    format!("与精选拓展同 id（精选版本来自 {repo}）")
                });
            }
        }
        let owner_official = self.official_owners.iter().any(|owner| owner.eq_ignore_ascii_case(&entry.owner));
        let text = format!("{} {} {}", entry.name, entry.description, entry.author).to_lowercase();
        if !owner_official && ["官方", "official", "codex++ 出品", "codex++出品"].iter().any(|word| text.contains(word)) {
            warnings.push("自称官方，但不是 Codex++ 官方仓库".to_string());
        }
        warnings
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityState {
    /// 社区栏开关。关闭后不发任何搜索请求。
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 键是拓展 id。
    #[serde(default)]
    pub installs: BTreeMap<String, InstallRecord>,
}

impl Default for CommunityState {
    fn default() -> Self {
        Self { enabled: true, installs: BTreeMap::new() }
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DiscoveryCache {
    fetched_at: u64,
    entries: Vec<CommunityEntry>,
    #[serde(default)]
    problems: Vec<String>,
}

/// 社区拓展的本地文件都在 Codex++ 用户脚本配置目录下。
#[derive(Debug, Clone)]
pub struct CommunityStore {
    config_dir: PathBuf,
}

impl CommunityStore {
    pub fn new(config_dir: impl Into<PathBuf>) -> Self {
        Self { config_dir: config_dir.into() }
    }

    pub fn load_state(&self) -> CommunityState {
        std::fs::read(self.config_dir.join(STATE_FILE))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn save_state(&self, state: &CommunityState) -> anyhow::Result<()> {
        std::fs::create_dir_all(&self.config_dir)?;
        crate::settings::atomic_write(&self.config_dir.join(STATE_FILE), serde_json::to_string_pretty(state)?.as_bytes())
    }

    pub fn set_enabled(&self, enabled: bool) -> anyhow::Result<()> {
        let mut state = self.load_state();
        state.enabled = enabled;
        self.save_state(&state)
    }

    fn load_cache(&self) -> Option<DiscoveryCache> {
        std::fs::read(self.config_dir.join(CACHE_FILE))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    }

    fn save_cache(&self, cache: &DiscoveryCache) -> anyhow::Result<()> {
        std::fs::create_dir_all(&self.config_dir)?;
        crate::settings::atomic_write(&self.config_dir.join(CACHE_FILE), serde_json::to_vec(cache)?.as_slice())
    }

    /// 作者下架或删除了某个仓库：直接从缓存里拿掉，社区栏不用等缓存过期（GitHub 搜索索引也有延迟）。
    pub fn forget_repo(&self, repo: &str) -> anyhow::Result<()> {
        let Some(mut cache) = self.load_cache() else {
            return Ok(());
        };
        let before = cache.entries.len();
        cache.entries.retain(|entry| !entry.repo.eq_ignore_ascii_case(repo));
        if cache.entries.len() != before {
            self.save_cache(&cache)?;
        }
        Ok(())
    }

    /// 社区栏列表。缓存未过期且不强制刷新时不联网；搜索失败时退回上次的缓存。
    pub async fn list(&self, endpoints: &Endpoints, manager: &UserScriptManager, force: bool) -> Value {
        self.list_filtered(endpoints, manager, force, &CuratedFilter::default()).await
    }

    /// 带精选清单约束的列表：下架的仓库与已收进精选的仓库不显示，可疑条目带警示。
    pub async fn list_filtered(
        &self,
        endpoints: &Endpoints,
        manager: &UserScriptManager,
        force: bool,
        filter: &CuratedFilter,
    ) -> Value {
        let state = self.load_state();
        if !state.enabled {
            return json!({
                "enabled": false, "entries": [], "fetchedAt": 0, "stale": false, "problems": [],
                "message": "社区拓展已关闭",
            });
        }
        let cached = self.load_cache();
        let now = now_secs();
        let fresh = cached
            .as_ref()
            .is_some_and(|cache| now.saturating_sub(cache.fetched_at) < CACHE_TTL_SECS);
        let (cache, stale, message) = if fresh && !force {
            (cached.unwrap_or_default(), false, String::new())
        } else {
            match search(endpoints).await {
                Ok((entries, problems)) => {
                    let cache = DiscoveryCache { fetched_at: now, entries, problems };
                    let _ = self.save_cache(&cache);
                    (cache, false, String::new())
                }
                Err(error) => match cached {
                    Some(cache) => {
                        let minutes = now.saturating_sub(cache.fetched_at) / 60;
                        let message = format!("GitHub 暂时连不上（{error:#}），显示的是 {minutes} 分钟前的列表");
                        (cache, true, message)
                    }
                    None => (DiscoveryCache::default(), true, format!("读取社区拓展失败：{error:#}")),
                },
            }
        };
        let entries = cache
            .entries
            .iter()
            .filter(|entry| !filter.is_blocked(&entry.repo) && !CuratedFilter::has_repo(&filter.curated_repos, &entry.repo))
            .map(|entry| {
                let mut item = serde_json::to_value(entry).unwrap_or(Value::Null);
                item["installed"] = installed_status(&state, manager, &entry.id, &entry.repo);
                item["warnings"] = json!(filter.warnings(entry));
                item
            })
            .collect::<Vec<_>>();
        json!({
            "enabled": true,
            "entries": entries,
            "fetchedAt": cache.fetched_at,
            "stale": stale,
            "problems": cache.problems,
            "message": message,
        })
    }

    // -----------------------------------------------------------------------
    // 安装
    // -----------------------------------------------------------------------

    /// 解析要装的版本、下载、解包并校验，暂存起来等用户确认。返回暂存令牌与实际版本的权限。
    pub async fn prepare(
        &self,
        endpoints: &Endpoints,
        manager: &UserScriptManager,
        repo: &str,
    ) -> anyhow::Result<Value> {
        validate_repo(repo)?;
        let client = endpoints.client()?;
        let entry = match self.load_cache().and_then(|cache| cache.entries.into_iter().find(|entry| entry.repo == repo)) {
            Some(entry) => entry,
            None => fetch_entry(&client, endpoints, repo).await?,
        };
        if !entry.compatible {
            if let Some(range) = entry.engine.as_str() {
                bail!(
                    "这个拓展需要 Codex++ {range}，当前版本为 {}",
                    crate::extension_semver::codexpp_version()
                );
            }
            bail!(
                "这个拓展需要拓展 API v{}，当前 Codex++ 提供 v{}，请先升级 Codex++",
                entry.engine,
                EXTENSION_API_VERSION
            );
        }
        let resolved = resolve_version(&client, endpoints, repo, &entry.id, &entry.default_branch).await?;
        let bytes = download(&client, endpoints, repo, &resolved).await?;
        self.stage(manager, repo, &entry.id, &resolved, &bytes, CHANNEL_COMMUNITY)
    }

    /// 精选 / 官方拓展：清单给定仓库与审核过的提交，直接按提交下载，不解析最新版本。
    pub async fn prepare_pinned(
        &self,
        endpoints: &Endpoints,
        manager: &UserScriptManager,
        pinned: &PinnedPackage,
    ) -> anyhow::Result<Value> {
        validate_repo(&pinned.repo)?;
        validate_extension_id(&pinned.id)?;
        if pinned.sha.len() != 40 || !pinned.sha.chars().all(|ch| ch.is_ascii_hexdigit()) {
            bail!("精选清单里 {} 的提交 sha 无效", pinned.id);
        }
        let client = endpoints.client()?;
        let resolved = ResolvedVersion { tag: pinned.tag.clone(), sha: pinned.sha.to_lowercase(), asset: None, from_default_branch: false };
        let bytes = download(&client, endpoints, &pinned.repo, &resolved).await?;
        self.stage(manager, &pinned.repo, &pinned.id, &resolved, &bytes, &pinned.channel)
    }

    /// 解包、校验并暂存，返回给确认框的信息。
    fn stage(
        &self,
        manager: &UserScriptManager,
        repo: &str,
        id: &str,
        resolved: &ResolvedVersion,
        bytes: &[u8],
        channel: &str,
    ) -> anyhow::Result<Value> {
        self.prune_staging();
        let token = format!("{id}-{}", now_millis());
        let staging = self.staging_dir().join(&token);
        let package_dir = staging.join("user").join(id);
        let result = (|| -> anyhow::Result<Value> {
            extract_package(bytes, id, &package_dir)?;
            let manifest = validate_staged(&staging, id)?;
            let version = manifest.get("version").and_then(Value::as_str).unwrap_or("").to_string();
            let meta = json!({
                "repo": repo,
                "id": id,
                "version": version,
                "tag": resolved.tag,
                "sha": resolved.sha,
                "fromDefaultBranch": resolved.from_default_branch,
                "fromAsset": resolved.asset.is_some(),
                "channel": channel,
            });
            std::fs::write(staging.join(STAGING_META), serde_json::to_vec_pretty(&meta)?)?;
            Ok(json!({
                "token": token,
                "repo": repo,
                "id": id,
                "name": manifest.get("name").and_then(Value::as_str).unwrap_or(id),
                "version": version,
                "tag": resolved.tag,
                "sha": resolved.sha,
                "fromDefaultBranch": resolved.from_default_branch,
                "fromAsset": resolved.asset.is_some(),
                "channel": channel,
                "permissions": manifest_permissions(&manifest),
                "conflict": local_conflict(&self.load_state(), manager, id, repo),
            }))
        })();
        if result.is_err() {
            let _ = std::fs::remove_dir_all(&staging);
        }
        result
    }

    /// 把 prepare 暂存的包换进用户拓展目录。同 id 的本地拓展来自别处时，需要 `replace` 确认。
    pub fn install(
        &self,
        manager: &UserScriptManager,
        token: &str,
        enable: bool,
        replace: bool,
    ) -> anyhow::Result<Value> {
        if token.is_empty() || !token.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_') {
            bail!("暂存令牌无效");
        }
        let staging = self.staging_dir().join(token);
        let meta: Value = serde_json::from_slice(
            &std::fs::read(staging.join(STAGING_META)).context("暂存的安装包已失效，请重新安装")?,
        )?;
        let text = |key: &str| meta.get(key).and_then(Value::as_str).unwrap_or("").to_string();
        let (repo, id) = (text("repo"), text("id"));
        validate_extension_id(&id)?;
        let mut state = self.load_state();
        let channel = meta.get("channel").and_then(Value::as_str).unwrap_or(CHANNEL_COMMUNITY).to_string();
        // 已装的官方 / 精选版本不能被社区版本替换（防止同 id 的仿冒包顶掉审核过的版本）。
        if channel == CHANNEL_COMMUNITY {
            if let Some(existing) = state.installs.get(&id).filter(|record| record.channel != CHANNEL_COMMUNITY) {
                if existing.repo != repo {
                    bail!("本机已装 {} 的{}版本 {id}，社区里的同 id 拓展不能替换它", existing.repo, channel_label(&existing.channel));
                }
            }
        }
        if !local_conflict(&state, manager, &id, &repo).is_null() && !replace {
            bail!("本机已有同 id 的拓展 {id}，确认替换后再安装");
        }
        let key = extension_package_key(&id);
        let target = manager.user_dir().join(&id);
        // 先写开关再放文件：没有开关记录的拓展默认是启用的。
        manager.set_script_enabled(&key, enable)?;
        std::fs::create_dir_all(manager.user_dir())?;
        let mut backup = None;
        if std::fs::symlink_metadata(&target).is_ok() {
            let backups = self.config_dir.join(BACKUP_DIR);
            std::fs::create_dir_all(&backups)?;
            let destination = backups.join(format!("{id}-{}", now_millis()));
            std::fs::rename(&target, &destination)
                .with_context(|| format!("无法把旧版本移到备份目录 {}", destination.display()))?;
            backup = Some(destination);
        }
        if let Err(error) = std::fs::rename(staging.join("user").join(&id), &target) {
            // 新包放不进去时把旧版本放回原处。
            if let Some(backup) = &backup {
                let _ = std::fs::rename(backup, &target);
            }
            return Err(error).with_context(|| format!("无法写入拓展目录 {}", target.display()));
        }
        let _ = std::fs::remove_dir_all(&staging);
        let record = InstallRecord {
            repo: repo.clone(),
            version: text("version"),
            tag: meta.get("tag").and_then(Value::as_str).map(str::to_string),
            sha: text("sha"),
            from_default_branch: meta.get("fromDefaultBranch").and_then(Value::as_bool).unwrap_or(false),
            from_asset: meta.get("fromAsset").and_then(Value::as_bool).unwrap_or(false),
            installed_at: now_secs(),
            channel,
        };
        state.installs.insert(id.clone(), record.clone());
        self.save_state(&state)?;
        let manifest: Value = std::fs::read(target.join(EXTENSION_MANIFEST_FILE))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or(Value::Null);
        let has_hooks = manifest
            .get("hooks")
            .and_then(Value::as_object)
            .is_some_and(|hooks| !hooks.is_empty());
        Ok(json!({
            "id": id,
            "key": key,
            "enabled": enable,
            "record": record,
            "hooks": has_hooks,
            "backup": backup.map(|path| path.to_string_lossy().to_string()),
        }))
    }

    /// 用户取消安装：清掉暂存。
    /// prepare 暂存的包：包目录与暂存元数据（repo / id / version / tag / sha）。精选审核在暂存的包上检查。
    pub fn staged(&self, token: &str) -> anyhow::Result<(PathBuf, Value)> {
        if token.is_empty() || !token.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_') {
            bail!("暂存令牌无效");
        }
        let staging = self.staging_dir().join(token);
        let meta: Value = serde_json::from_slice(&std::fs::read(staging.join(STAGING_META)).context("暂存的包已过期，请重新审核")?)?;
        let id = meta.get("id").and_then(Value::as_str).context("暂存元数据缺少 id")?;
        Ok((staging.join("user").join(id), meta))
    }

    /// 社区缓存里的条目（仓库信息与默认分支清单）。
    pub fn cached_entry(&self, repo: &str) -> Option<CommunityEntry> {
        self.load_cache()?.entries.into_iter().find(|entry| entry.repo.eq_ignore_ascii_case(repo))
    }

    pub fn discard(&self, token: &str) {
        if !token.is_empty() && token.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_') {
            let _ = std::fs::remove_dir_all(self.staging_dir().join(token));
        }
    }

    fn staging_dir(&self) -> PathBuf {
        self.config_dir.join(STAGING_DIR)
    }

    /// 用户没确认就关掉管理器时，暂存会留下来；超过一天的清掉。
    fn prune_staging(&self) {
        let Ok(entries) = std::fs::read_dir(self.staging_dir()) else {
            return;
        };
        for entry in entries.filter_map(Result::ok) {
            let stale = entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .ok()
                .and_then(|modified| modified.elapsed().ok())
                .is_some_and(|age| age > Duration::from_secs(24 * 3600));
            if stale && entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
}

fn installed_status(state: &CommunityState, manager: &UserScriptManager, id: &str, repo: &str) -> Value {
    let present = manager.user_dir().join(id).join(EXTENSION_MANIFEST_FILE).is_file();
    match state.installs.get(id) {
        Some(record) if record.repo == repo && present => json!({
            "version": record.version,
            "tag": record.tag,
            "sha": record.sha,
            "fromDefaultBranch": record.from_default_branch,
            "channel": record.channel,
        }),
        // 本机就是源码（自己写的、从这里发布上去的）：没有安装记录，但清单的 repository 指向这个仓库。
        _ => match local_source_version(manager, id, repo) {
            Some(version) => json!({ "version": version, "tag": null, "sha": "", "fromDefaultBranch": false, "channel": CHANNEL_LOCAL }),
            None => Value::Null,
        },
    }
}

/// 本机同 id 的拓展包是这个仓库的源码（清单 `repository` 指向它，比如一键发布写回的）时，返回本地版本号。
pub fn local_source_version(manager: &UserScriptManager, id: &str, repo: &str) -> Option<String> {
    let text = std::fs::read_to_string(manager.user_dir().join(id).join(EXTENSION_MANIFEST_FILE)).ok()?;
    let manifest: Value = serde_json::from_str(&text).ok()?;
    let declared = manifest.get("repository").and_then(Value::as_str)?.trim();
    let declared = declared.trim_start_matches("https://github.com/").trim_end_matches(".git").trim_end_matches('/');
    if !declared.eq_ignore_ascii_case(repo) {
        return None;
    }
    Some(manifest.get("version").and_then(Value::as_str).unwrap_or_default().to_string())
}

/// 本机同 id 的拓展是否来自别处（手写、对话生成或另一个仓库）。是的话返回它的来源说明。
fn local_conflict(state: &CommunityState, manager: &UserScriptManager, id: &str, repo: &str) -> Value {
    if std::fs::symlink_metadata(manager.user_dir().join(id)).is_err() {
        return Value::Null;
    }
    match state.installs.get(id) {
        Some(record) if record.repo == repo => Value::Null,
        Some(record) => json!({ "source": "github", "repo": record.repo, "channel": record.channel }),
        None => json!({ "source": "local" }),
    }
}

fn channel_label(channel: &str) -> &'static str {
    match channel {
        CHANNEL_OFFICIAL => "官方",
        CHANNEL_CURATED => "精选",
        _ => "社区",
    }
}

/// 精选清单里一个锁定版本的拓展包。
#[derive(Debug, Clone, PartialEq)]
pub struct PinnedPackage {
    pub repo: String,
    pub id: String,
    pub tag: Option<String>,
    pub sha: String,
    /// `curated` 或 `official`。
    pub channel: String,
}

/// 本机从 GitHub 安装过的拓展：id → 记录。精选面板用它算「已安装 / 有更新」。
pub fn installed_records(store: &CommunityStore, manager: &UserScriptManager) -> BTreeMap<String, InstallRecord> {
    store
        .load_state()
        .installs
        .into_iter()
        .filter(|(id, _)| manager.user_dir().join(id).join(EXTENSION_MANIFEST_FILE).is_file())
        .collect()
}

fn validate_repo(repo: &str) -> anyhow::Result<()> {
    let parts = repo.split('/').collect::<Vec<_>>();
    let valid = parts.len() == 2
        && parts.iter().all(|part| {
            !part.is_empty()
                && *part != "."
                && *part != ".."
                && part.chars().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
        });
    if !valid {
        bail!("仓库必须写成 owner/repo");
    }
    Ok(())
}

/// 缓存里没有这个仓库时（比如刚打 topic），直接读仓库信息和默认分支清单。
async fn fetch_entry(client: &reqwest::Client, endpoints: &Endpoints, repo: &str) -> anyhow::Result<CommunityEntry> {
    let response = endpoints.api_get(client, &format!("/repos/{repo}")).send().await?;
    let raw = github_json(response).await?;
    let wrapped = json!({ "total_count": 1, "items": [raw] });
    let (repos, _) = parse_search_page(&wrapped)?;
    let found = repos.into_iter().next().context("仓库已归档或是 fork，不能安装")?;
    let manifest = read_default_manifest(client, endpoints, &found)
        .await?
        .with_context(|| format!("{repo} 的默认分支上没有 {EXTENSION_MANIFEST_FILE}"))?;
    community_entry(&found, &endpoints.raw, &manifest)
}

// ---------------------------------------------------------------------------
// 版本解析与下载
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseAsset {
    pub name: String,
    pub url: String,
    /// `sha256:<hex>`，GitHub 给出时校验。
    pub digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedVersion {
    pub tag: Option<String>,
    pub sha: String,
    pub asset: Option<ReleaseAsset>,
    pub from_default_branch: bool,
}

/// 发布流程上传的包文件名。
pub fn asset_name(id: &str, tag: &str) -> String {
    format!("{id}-{tag}.zip")
}

/// 从 tag 名里挑最新的 `v<semver>`：有正式版只看正式版，否则取最新的预发布版。
pub fn pick_latest_tag<'a>(names: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let parsed = names
        .into_iter()
        .filter_map(|name| parse_version_tag(name).map(|version| (version, name)))
        .collect::<Vec<_>>();
    let stable = parsed.iter().filter(|(version, _)| version.3.is_none()).max_by(|a, b| a.0.cmp(&b.0));
    stable
        .or_else(|| parsed.iter().max_by(|a, b| a.0.cmp(&b.0)))
        .map(|(_, name)| name.to_string())
}

type TagVersion = (u64, u64, u64, Option<String>);

fn parse_version_tag(name: &str) -> Option<TagVersion> {
    let rest = name.strip_prefix('v')?;
    let rest = rest.split('+').next()?;
    let (core, pre) = match rest.split_once('-') {
        Some((core, pre)) => (core, Some(pre.to_string())),
        None => (rest, None),
    };
    let mut parts = core.split('.').map(|part| part.parse::<u64>().ok());
    let version = (parts.next()??, parts.next()??, parts.next()??, pre);
    if parts.next().is_some() {
        return None;
    }
    Some(version)
}

/// 要装的版本：最新 release → 最新 `v*` tag → 默认分支，都解析成提交 SHA。
/// 仓库当前的最新版本（release → tag → 默认分支）。精选维护者检查新版本用。
pub async fn latest_version(endpoints: &Endpoints, repo: &str, id: &str, default_branch: &str) -> anyhow::Result<ResolvedVersion> {
    validate_repo(repo)?;
    let client = endpoints.client()?;
    resolve_version(&client, endpoints, repo, id, default_branch).await
}

pub async fn resolve_version(
    client: &reqwest::Client,
    endpoints: &Endpoints,
    repo: &str,
    id: &str,
    default_branch: &str,
) -> anyhow::Result<ResolvedVersion> {
    let response = endpoints.api_get(client, &format!("/repos/{repo}/releases/latest")).send().await?;
    let mut tag = None;
    let mut asset = None;
    if response.status() != reqwest::StatusCode::NOT_FOUND {
        let release = github_json(response).await?;
        if let Some(name) = release.get("tag_name").and_then(Value::as_str) {
            let wanted = asset_name(id, name);
            asset = release
                .get("assets")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .find(|item| item.get("name").and_then(Value::as_str) == Some(wanted.as_str()))
                .and_then(|item| {
                    Some(ReleaseAsset {
                        name: wanted.clone(),
                        url: item.get("browser_download_url")?.as_str()?.to_string(),
                        digest: item.get("digest").and_then(Value::as_str).map(str::to_string),
                    })
                });
            tag = Some(name.to_string());
        }
    }
    if tag.is_none() {
        let response = endpoints.api_get(client, &format!("/repos/{repo}/tags?per_page=100")).send().await?;
        let tags = github_json(response).await?;
        tag = pick_latest_tag(
            tags.as_array().into_iter().flatten().filter_map(|item| item.get("name").and_then(Value::as_str)),
        );
    }
    let reference = tag.clone().unwrap_or_else(|| default_branch.to_string());
    let response = endpoints
        .api_get(client, &format!("/repos/{repo}/commits/{}", encode_ref(&reference)))
        .send()
        .await?;
    let commit = github_json(response).await?;
    let sha = commit
        .get("sha")
        .and_then(Value::as_str)
        .filter(|sha| sha.len() == 40 && sha.chars().all(|ch| ch.is_ascii_hexdigit()))
        .with_context(|| format!("无法解析 {reference} 对应的提交"))?
        .to_string();
    Ok(ResolvedVersion { from_default_branch: tag.is_none(), tag, sha, asset })
}

fn encode_ref(reference: &str) -> String {
    reference
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (byte as char).to_string(),
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

/// 下载要装的版本：有包文件就下包文件并校验摘要，否则按提交从 codeload 下载仓库压缩包。
async fn download(
    client: &reqwest::Client,
    endpoints: &Endpoints,
    repo: &str,
    resolved: &ResolvedVersion,
) -> anyhow::Result<Vec<u8>> {
    let url = match &resolved.asset {
        Some(asset) => asset.url.clone(),
        None => format!("{}/{repo}/zip/{}", endpoints.codeload, resolved.sha),
    };
    let response = client.get(&url).send().await.with_context(|| format!("下载 {url} 失败"))?;
    let response = response.error_for_status().with_context(|| format!("下载 {url} 失败"))?;
    let bytes = read_limited(response, DOWNLOAD_MAX_BYTES).await?;
    if let Some(digest) = resolved.asset.as_ref().and_then(|asset| asset.digest.as_deref()) {
        if let Some(expected) = digest.strip_prefix("sha256:") {
            let actual = format!("{:x}", Sha256::digest(&bytes));
            if !actual.eq_ignore_ascii_case(expected) {
                bail!("包文件的 sha256 与 GitHub 记录的不一致，已拒绝安装");
            }
        }
    }
    Ok(bytes)
}

// ---------------------------------------------------------------------------
// 解包与校验
// ---------------------------------------------------------------------------

/// 把压缩包解到 `dest`。包根目录可以是压缩包根，也可以是唯一的顶层目录（codeload 的格式）。
/// 拒绝软链、越界路径、超过数量或体积上限的包，以及清单 id 与预期不符的包。
pub fn extract_package(bytes: &[u8], expected_id: &str, dest: &Path) -> anyhow::Result<()> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).context("压缩包无法解析")?;
    let mut names = Vec::with_capacity(archive.len());
    for index in 0..archive.len() {
        let file = archive.by_index(index)?;
        if file.is_symlink() {
            bail!("压缩包里有软链 {}，已拒绝安装", file.name());
        }
        let path = file
            .enclosed_name()
            .with_context(|| format!("压缩包里的路径 {} 越界，已拒绝安装", file.name()))?;
        // macOS 自带的压缩会多出 __MACOSX 资源目录，不属于包。
        let resource_fork = path.components().next().is_some_and(|part| part.as_os_str() == "__MACOSX");
        names.push((path, file.is_dir() || resource_fork));
    }
    let manifest = Path::new(EXTENSION_MANIFEST_FILE);
    let prefix = if names.iter().any(|(path, dir)| !dir && path == manifest) {
        PathBuf::new()
    } else {
        let tops = names
            .iter()
            .filter(|(path, _)| !path.starts_with("__MACOSX"))
            .filter_map(|(path, _)| path.components().next().map(|part| PathBuf::from(part.as_os_str())))
            .collect::<BTreeSet<_>>();
        match tops.into_iter().collect::<Vec<_>>().as_slice() {
            [top] if names.iter().any(|(path, dir)| !dir && *path == top.join(manifest)) => top.clone(),
            _ => bail!("压缩包根目录没有 {EXTENSION_MANIFEST_FILE}"),
        }
    };
    let mut files = 0usize;
    let mut total = 0u64;
    std::fs::create_dir_all(dest)?;
    for index in 0..archive.len() {
        let mut file = archive.by_index(index)?;
        let (path, dir) = &names[index];
        let Ok(relative) = path.strip_prefix(&prefix) else {
            continue;
        };
        if relative.as_os_str().is_empty() || *dir {
            continue;
        }
        files += 1;
        if files > PACKAGE_MAX_FILES {
            bail!("拓展包超过 {PACKAGE_MAX_FILES} 个文件");
        }
        let target = dest.join(relative);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut contents = Vec::new();
        (&mut file).take(PACKAGE_MAX_BYTES + 1 - total.min(PACKAGE_MAX_BYTES)).read_to_end(&mut contents)?;
        total += contents.len() as u64;
        if total > PACKAGE_MAX_BYTES {
            bail!("拓展包解开后超过 {} MiB", PACKAGE_MAX_BYTES / 1024 / 1024);
        }
        std::fs::write(&target, contents)?;
    }
    let raw: Value = serde_json::from_slice(&std::fs::read(dest.join(EXTENSION_MANIFEST_FILE))?)
        .with_context(|| format!("{EXTENSION_MANIFEST_FILE} 不是合法的 JSON"))?;
    let id = raw.get("id").and_then(Value::as_str).unwrap_or("");
    if id != expected_id {
        bail!("这个版本的清单 id 是 {id}，与列表里的 {expected_id} 不一致，已拒绝安装");
    }
    Ok(())
}

/// 用拓展加载器校验暂存的包（`staging/user/<id>/`），返回它的清单。
fn validate_staged(staging: &Path, id: &str) -> anyhow::Result<Value> {
    let manager = UserScriptManager::new(
        staging.join("builtin"),
        staging.join("user"),
        staging.join("user_scripts.json"),
    );
    let inventory = manager.inventory()?;
    let key = extension_package_key(id);
    let item = inventory
        .get("scripts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|item| item.get("key").and_then(Value::as_str) == Some(key.as_str()))
        .context("拓展加载器没有认出这个包")?;
    if let Some(error) = item.get("error").and_then(Value::as_str).filter(|error| !error.is_empty()) {
        bail!("拓展包校验失败：{error}");
    }
    Ok(serde_json::from_slice(&std::fs::read(staging.join("user").join(id).join(EXTENSION_MANIFEST_FILE))?)?)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn now_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}
