//! 兼容服务里的插件接口：用 Codex++ 的完整缓存目录回答 Codex 的官方插件目录请求（`/ps/plugins/*`），
//! 装插件、卸插件走 Codex 原生的远程插件流程，插件包由这里现打现给。
//!
//! 接口和字段照 Codex 开源客户端（codex-rs `core-plugins/src/remote.rs`）里的请求与解析写：
//! - 目录：`GET ps/plugins/list?scope=…&limit=…&pageToken=…`、`GET ps/plugins/search?q=…`，
//!   桌面首页的分组是 `GET ps/plugins/home`；
//! - 详情：`GET ps/plugins/{id}`（`includeDownloadUrls=true` 时带插件包地址）；
//! - 已安装：`GET ps/plugins/installed`，安装 / 卸载：`POST ps/plugins/{id}/install|uninstall`；
//! - 插件包：不带签名信息时 Codex 按旧版不校验签名，地址要是 https，所以就挂在本服务上。
//!
//! 只由 ChatGPT 托管连接器组成的插件（没有本地 MCP、也没有技能）本地账号连不上，照常列出但标成不可用。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::plugin_market::compat::{Catalog, CatalogPlugin};

/// 插件包的下载路径前缀（`{前缀}{id}.tar.gz`）。
pub const BUNDLE_PREFIX: &str = "/codexplus/plugin-bundles/";

/// 目录一页最多多少条（Codex 自己按 200 一页取）。
const MAX_PAGE: usize = 200;
/// 桌面首页每组显示几个（和官方一样，多的点进分组看）。
const HOME_SECTION_SIZE: usize = 9;

/// 一个应答：JSON 或插件包。
pub struct PluginReply {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

fn json_reply(status: u16, value: Value) -> PluginReply {
    PluginReply {
        status,
        content_type: "application/json",
        body: serde_json::to_vec(&value).unwrap_or_else(|_| b"{}".to_vec()),
    }
}

/// 插件相关的请求由这里回答；不是插件请求返回 None，交回录制 / 合成应答。
pub async fn reply(method: &str, target: &str, headers: &str, body: &[u8]) -> Option<PluginReply> {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let query = parse_query(query);
    if let Some(id) = path.strip_prefix(BUNDLE_PREFIX).and_then(|rest| rest.strip_suffix(".tar.gz")) {
        return Some(bundle_reply(id).await);
    }
    let rest = path.strip_prefix("/backend-api/ps/plugins")?;
    let home = codex_home();
    match (method, rest) {
        ("GET", "/home") => Some(with_catalog(&home, |catalog| home_reply(catalog, &language(headers))).await),
        ("GET", "/list") => Some(
            with_catalog(&home, |catalog| list_reply(catalog, &query, &installed_store().load())).await,
        ),
        ("GET", "/search") => Some(
            with_catalog(&home, |catalog| search_reply(catalog, &query)).await,
        ),
        ("GET", "/installed") => Some(
            with_catalog(&home, |catalog| installed_reply(catalog, &query, &installed_store().load()))
                .await,
        ),
        ("GET", "/workspace/shared") | ("GET", "/workspace/created") => {
            Some(json_reply(200, empty_page()))
        }
        // 桌面拿搜索结果、首页条目后，按 id 批量补齐详情。
        ("POST", "/batch") => Some(
            with_catalog(&home, |catalog| batch_reply(catalog, body, &installed_store().load())).await,
        ),
        ("GET", "/suggested/codex") => Some(json_reply(200, json!({ "enabled": false, "plugins": [] }))),
        // 插件包不带签名信息，用不到签名公钥；万一被问到就给空表。
        ("GET", "/signing-keys") => Some(json_reply(200, json!({ "keys": [] }))),
        _ => {
            let rest = rest.strip_prefix('/')?;
            let (id, action) = rest.split_once('/').unwrap_or((rest, ""));
            if !valid_id(id) {
                return None;
            }
            match (method, action) {
                ("GET", "") => Some(
                    with_catalog(&home, |catalog| detail_reply(catalog, id, &installed_store().load()))
                        .await,
                ),
                ("POST", "install") => Some(
                    with_catalog(&home, |catalog| install_reply(catalog, id, &installed_store())).await,
                ),
                ("POST", "uninstall") => Some(uninstall_reply(id, &installed_store())),
                _ => None,
            }
        }
    }
}

fn codex_home() -> PathBuf {
    crate::codex_home::default_codex_home_dir()
}

/// 读目录后交给 `build`；目录读不到（还没缓存、也联不上网）时回 503，Codex 会显示加载失败。
async fn with_catalog(home: &Path, build: impl FnOnce(&Catalog) -> PluginReply) -> PluginReply {
    match crate::plugin_market::compat::catalog(home).await {
        Ok(catalog) => build(&catalog),
        Err(error) => json_reply(
            503,
            json!({ "detail": format!("Codex++ 插件目录暂时读不到：{error:#}") }),
        ),
    }
}

async fn bundle_reply(id: &str) -> PluginReply {
    if !valid_id(id) {
        return json_reply(404, json!({ "detail": "not found" }));
    }
    match crate::plugin_market::compat::bundle(&codex_home(), id).await {
        Ok(bytes) => PluginReply { status: 200, content_type: "application/gzip", body: bytes },
        Err(error) => json_reply(502, json!({ "detail": format!("插件包准备失败：{error:#}") })),
    }
}

/// 远程插件 id 只允许字母、数字、`_`、`-`、`~`（和 Codex 的校验一致）。
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 200
        && id.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'~'))
}

fn parse_query(query: &str) -> BTreeMap<String, String> {
    // 借 reqwest 的 Url 解查询串（core 没有直接依赖 url）。
    reqwest::Url::parse(&format!("https://localhost/?{query}"))
        .map(|url| url.query_pairs().into_owned().collect())
        .unwrap_or_default()
}

/// 请求头里的语言：中文界面用中文分组标题。
fn language(headers: &str) -> String {
    headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim().eq_ignore_ascii_case("accept-language").then(|| value.trim().to_ascii_lowercase())
        })
        .unwrap_or_default()
}

/// 只由 ChatGPT 托管连接器组成的插件：本地账号连不上。
fn hosted_only(plugin: &CatalogPlugin) -> bool {
    plugin.has_apps && !plugin.has_mcp && plugin.skills == 0
}

fn opt(value: &str) -> Value {
    let value = value.trim();
    if value.is_empty() { Value::Null } else { json!(value) }
}

fn short(plugin: &CatalogPlugin) -> &str {
    if plugin.short_description.trim().is_empty() {
        &plugin.description
    } else {
        &plugin.short_description
    }
}

/// 版本号给 Codex 当目录名用：只留它认的字符，空的给个占位。
fn version(plugin: &CatalogPlugin) -> String {
    let cleaned: String = plugin
        .version
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_' | '+'))
        .collect();
    if cleaned.is_empty() { "0.0.0".to_string() } else { cleaned }
}

/// 目录里的一个插件（`RemotePluginDirectoryItem`）。
fn directory_item(plugin: &CatalogPlugin, download_urls: bool) -> Value {
    let unavailable = hosted_only(plugin);
    json!({
        "id": plugin.id,
        "name": plugin.name,
        "scope": "GLOBAL",
        "canonical_app_id": null,
        "discoverability": "LISTED",
        "creator_name": opt(&plugin.author),
        "creator_account_user_id": null,
        "share_url": null,
        "installation_policy": if unavailable { "NOT_AVAILABLE" } else { "AVAILABLE" },
        "installation_policy_source": null,
        "must_show_installation_interstitial": false,
        "authentication_policy": "ON_USE",
        "status": if unavailable { "DISABLED_BY_ADMIN" } else { "ENABLED" },
        "disabled_reason": if unavailable { json!("required_app_unavailable") } else { Value::Null },
        "eligible_plan_types": null,
        "release": {
            "version": version(plugin),
            "display_name": plugin.display_name,
            "description": plugin.description,
            "bundle_download_url": if download_urls && !unavailable {
                json!(format!("https://localhost:{}{BUNDLE_PREFIX}{}.tar.gz", super::PORT, plugin.id))
            } else {
                Value::Null
            },
            "keywords": plugin.keywords,
            // 托管连接器本地账号连不上：不声明 app，装上后只用包里的技能和 MCP。
            "app_ids": [],
            "app_manifest": null,
            "app_templates": [],
            "interface": {
                "short_description": opt(short(plugin)),
                "long_description": opt(&plugin.description),
                "developer_name": opt(&plugin.author),
                "category": opt(&plugin.category),
                "capabilities": [],
                "website_url": null,
                "privacy_policy_url": null,
                "terms_of_service_url": null,
                "brand_color": null,
                "default_prompt": null,
                "default_prompts": null,
                "composer_icon_url": opt(&plugin.icon_url),
                "logo_url": opt(&plugin.icon_url),
                "logo_url_dark": opt(&plugin.icon_url_dark),
                "screenshot_urls": [],
            },
            "skills": [],
            "onboarding_skill_name": null,
            "mcp_servers": [],
            "scheduled_tasks": null,
        },
    })
}

/// 桌面首页分组里的一项（比目录条目轻）。
fn home_item(plugin: &CatalogPlugin) -> Value {
    let unavailable = hosted_only(plugin);
    json!({
        "id": plugin.id,
        "canonical_app_id": null,
        "display_name": plugin.display_name,
        "requires_local_executor": false,
        "short_description": short(plugin),
        "keywords": plugin.keywords,
        "icon_url": opt(&plugin.icon_url),
        "icon_url_dark": opt(&plugin.icon_url_dark),
        "status": if unavailable { "DISABLED_BY_ADMIN" } else { "ENABLED" },
        "installation_policy": if unavailable { "NOT_AVAILABLE" } else { "AVAILABLE" },
        "disabled_reason": if unavailable { json!("required_app_unavailable") } else { Value::Null },
        "eligible_plan_types": null,
        "must_show_installation_interstitial": false,
    })
}

fn home_reply(catalog: &Catalog, language: &str) -> PluginReply {
    let chinese = language.starts_with("zh");
    let sections: Vec<Value> = catalog
        .sections
        .iter()
        .map(|section| {
            let title = (if chinese { section.title.get("zh") } else { None })
                .or_else(|| section.title.get("en"))
                .cloned()
                .unwrap_or_else(|| section.slug.clone());
            // 能装的排在前面：首页每组只露 9 个，别全是连不上的。
            let mut members: Vec<&CatalogPlugin> =
                section.plugins.iter().filter_map(|id| catalog.plugin(id)).collect();
            members.sort_by_key(|plugin| hosted_only(plugin));
            json!({
                "id": section.slug,
                "url_slug": section.slug,
                "title": title,
                "description": null,
                "plugins": members.into_iter().take(HOME_SECTION_SIZE).map(home_item).collect::<Vec<_>>(),
            })
        })
        .collect();
    json_reply(200, json!({ "sections": sections }))
}

fn empty_page() -> Value {
    json!({ "plugins": [], "pagination": { "next_page_token": null } })
}

/// 翻页：页码就是起始下标。
fn page<'a, T>(items: &'a [T], query: &BTreeMap<String, String>) -> (&'a [T], Option<String>) {
    let limit = query
        .get("limit")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(MAX_PAGE)
        .clamp(1, MAX_PAGE);
    let start = query
        .get("pageToken")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0)
        .min(items.len());
    let end = (start + limit).min(items.len());
    let next = (end < items.len()).then(|| end.to_string());
    (&items[start..end], next)
}

fn global_scope(query: &BTreeMap<String, String>) -> bool {
    query.get("scope").is_none_or(|scope| scope.eq_ignore_ascii_case("GLOBAL"))
}

/// 按分组 / 分类取：`collection`、`category`、`section` 任一指定时只列那一组。
fn filtered<'a>(catalog: &'a Catalog, query: &BTreeMap<String, String>) -> Vec<&'a CatalogPlugin> {
    let group = ["collection", "section", "category"]
        .iter()
        .find_map(|key| query.get(*key))
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty());
    match group {
        None => catalog.plugins.iter().collect(),
        Some(group) => {
            if let Some(section) = catalog.sections.iter().find(|section| section.slug.eq_ignore_ascii_case(&group)) {
                return section.plugins.iter().filter_map(|id| catalog.plugin(id)).collect();
            }
            catalog
                .plugins
                .iter()
                .filter(|plugin| plugin.category.eq_ignore_ascii_case(&group))
                .collect()
        }
    }
}

fn list_reply(
    catalog: &Catalog,
    query: &BTreeMap<String, String>,
    _installed: &InstalledPlugins,
) -> PluginReply {
    if !global_scope(query) {
        return json_reply(200, empty_page());
    }
    let plugins = filtered(catalog, query);
    let (items, next) = page(&plugins, query);
    json_reply(
        200,
        json!({
            "plugins": items.iter().map(|plugin| directory_item(plugin, false)).collect::<Vec<_>>(),
            "pagination": { "next_page_token": next },
        }),
    )
}

fn search_reply(catalog: &Catalog, query: &BTreeMap<String, String>) -> PluginReply {
    if !global_scope(query) {
        return json_reply(200, empty_page());
    }
    let needle = query.get("q").map(|value| value.trim().to_lowercase()).unwrap_or_default();
    let mut hits: Vec<(u8, &CatalogPlugin)> = catalog
        .plugins
        .iter()
        .filter_map(|plugin| {
            if needle.is_empty() {
                return Some((3, plugin));
            }
            let name = plugin.display_name.to_lowercase();
            let rank = if name == needle || plugin.name.to_lowercase() == needle {
                0
            } else if name.contains(&needle) || plugin.name.to_lowercase().contains(&needle) {
                1
            } else if plugin.keywords.iter().any(|keyword| keyword.to_lowercase().contains(&needle))
                || plugin.description.to_lowercase().contains(&needle)
            {
                2
            } else {
                return None;
            };
            Some((rank, plugin))
        })
        .collect();
    hits.sort_by_key(|(rank, plugin)| (*rank, hosted_only(plugin)));
    let plugins: Vec<&CatalogPlugin> = hits.into_iter().map(|(_, plugin)| plugin).collect();
    let (items, next) = page(&plugins, query);
    json_reply(
        200,
        json!({
            "plugins": items.iter().map(|plugin| directory_item(plugin, false)).collect::<Vec<_>>(),
            "pagination": { "next_page_token": next },
        }),
    )
}

fn detail_reply(catalog: &Catalog, id: &str, _installed: &InstalledPlugins) -> PluginReply {
    match catalog.plugin(id) {
        Some(plugin) => json_reply(200, directory_item(plugin, true)),
        None => json_reply(404, json!({ "detail": "plugin not found" })),
    }
}

/// `POST ps/plugins/batch`：正文 `{"plugin_ids": [...]}`，回 `{"results": [{"requested_plugin_id", "plugin"}]}`。
/// 目录里没有的 id 不列（桌面按「查不到」处理）；装过的带上安装状态。
fn batch_reply(catalog: &Catalog, body: &[u8], installed: &InstalledPlugins) -> PluginReply {
    let ids: Vec<String> = serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|value| value.get("plugin_ids").cloned())
        .and_then(|ids| serde_json::from_value(ids).ok())
        .unwrap_or_default();
    let results: Vec<Value> = ids
        .iter()
        .take(MAX_PAGE)
        .filter_map(|id| {
            let plugin = catalog.plugin(id)?;
            let mut item = directory_item(plugin, false);
            if let Some(record) = installed.plugins.get(id) {
                item["installed_at"] = json!(record.installed_at);
                item["enabled"] = json!(record.enabled);
                item["disabled_skill_names"] = json!([]);
            }
            Some(json!({ "requested_plugin_id": id, "plugin": item }))
        })
        .collect();
    json_reply(200, json!({ "results": results }))
}

fn installed_reply(
    catalog: &Catalog,
    query: &BTreeMap<String, String>,
    installed: &InstalledPlugins,
) -> PluginReply {
    if !global_scope(query) {
        return json_reply(200, empty_page());
    }
    let download_urls = query.get("includeDownloadUrls").is_some_and(|value| value == "true");
    let items: Vec<Value> = installed
        .plugins
        .iter()
        .filter_map(|(id, record)| {
            let plugin = catalog.plugin(id)?;
            let mut item = directory_item(plugin, download_urls);
            item["installed_at"] = json!(record.installed_at);
            item["enabled"] = json!(record.enabled);
            item["disabled_skill_names"] = json!([]);
            Some(item)
        })
        .collect();
    let (items, next) = page(&items, query);
    json_reply(200, json!({ "plugins": items, "pagination": { "next_page_token": next } }))
}

fn install_reply(catalog: &Catalog, id: &str, store: &InstalledStore) -> PluginReply {
    let Some(plugin) = catalog.plugin(id) else {
        return json_reply(404, json!({ "detail": "plugin not found" }));
    };
    if hosted_only(plugin) {
        return json_reply(
            403,
            json!({ "detail": "这个插件只有 ChatGPT 托管的连接器，本地账号连不上" }),
        );
    }
    store.update(|plugins| {
        plugins.plugins.insert(
            id.to_string(),
            InstalledRecord { installed_at: now_rfc3339(), enabled: true },
        );
    });
    json_reply(200, json!({ "id": id, "enabled": true, "app_ids_needing_auth": [] }))
}

fn uninstall_reply(id: &str, store: &InstalledStore) -> PluginReply {
    store.update(|plugins| {
        plugins.plugins.remove(id);
    });
    json_reply(200, json!({ "id": id, "enabled": false, "app_ids_needing_auth": null }))
}

/// 本地账号「装过」的插件：Codex 每次同步已安装列表时按这里给，装 / 卸时更新。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct InstalledPlugins {
    #[serde(default)]
    pub plugins: BTreeMap<String, InstalledRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstalledRecord {
    pub installed_at: String,
    pub enabled: bool,
}

/// 已安装列表存在状态目录里；测试里换成临时文件。
pub struct InstalledStore {
    path: PathBuf,
}

static STORE_LOCK: Mutex<()> = Mutex::new(());

impl InstalledStore {
    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn load(&self) -> InstalledPlugins {
        std::fs::read(&self.path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn update(&self, edit: impl FnOnce(&mut InstalledPlugins)) {
        let _guard = STORE_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let mut plugins = self.load();
        edit(&mut plugins);
        if let Ok(bytes) = serde_json::to_vec_pretty(&plugins) {
            let _ = crate::settings::atomic_write(&self.path, &bytes);
        }
    }
}

fn installed_store() -> InstalledStore {
    InstalledStore::at(crate::paths::default_app_state_dir().join("chatgpt-compat-plugins.json"))
}

fn now_rfc3339() -> String {
    super::rfc3339_now()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin_market::compat::CatalogSection;

    fn plugin(id: &str, name: &str, apps: bool, mcp: bool, skills: u64) -> CatalogPlugin {
        CatalogPlugin {
            id: id.into(),
            name: name.into(),
            display_name: name.to_uppercase(),
            description: format!("{name} does things"),
            short_description: String::new(),
            version: "1.2.3".into(),
            author: "Someone".into(),
            category: "Productivity".into(),
            keywords: vec!["tool".into()],
            icon_url: "https://example.com/i.png".into(),
            icon_url_dark: String::new(),
            skills,
            has_apps: apps,
            has_mcp: mcp,
        }
    }

    fn catalog() -> Catalog {
        let mut plugins = vec![
            plugin("plugin_a", "alpha", false, true, 0),
            plugin("plugin_b", "beta", true, false, 0),
            plugin("plugin_c", "gamma", true, false, 2),
        ];
        for index in 0..250 {
            plugins.push(plugin(&format!("plugin_x{index}"), &format!("extra{index}"), false, false, 1));
        }
        Catalog {
            revision: "r1".into(),
            updated_at: "2026-11-01T00:00:00Z".into(),
            plugins,
            sections: vec![CatalogSection {
                slug: "featured".into(),
                title: BTreeMap::from([("en".into(), "Featured".into()), ("zh".into(), "热门".into())]),
                plugins: vec!["plugin_b".into(), "plugin_a".into(), "missing".into()],
            }],
        }
    }

    fn body(reply: PluginReply) -> Value {
        serde_json::from_slice(&reply.body).unwrap()
    }

    fn query(text: &str) -> BTreeMap<String, String> {
        parse_query(text)
    }

    #[test]
    fn list_pages_the_whole_catalog_in_global_scope_only() {
        let catalog = catalog();
        let first = body(list_reply(&catalog, &query("scope=GLOBAL&limit=200"), &InstalledPlugins::default()));
        assert_eq!(first["plugins"].as_array().unwrap().len(), 200);
        assert_eq!(first["pagination"]["next_page_token"], "200");
        let second = body(list_reply(&catalog, &query("scope=GLOBAL&limit=200&pageToken=200"), &InstalledPlugins::default()));
        assert_eq!(second["plugins"].as_array().unwrap().len(), 53);
        assert!(second["pagination"]["next_page_token"].is_null());
        let workspace = body(list_reply(&catalog, &query("scope=WORKSPACE"), &InstalledPlugins::default()));
        assert!(workspace["plugins"].as_array().unwrap().is_empty());
    }

    #[test]
    fn directory_items_have_the_fields_codex_parses() {
        let catalog = catalog();
        let item = directory_item(catalog.plugin("plugin_a").unwrap(), true);
        for key in ["id", "name", "scope", "installation_policy", "authentication_policy", "status", "release"] {
            assert!(item.get(key).is_some(), "{key}");
        }
        assert_eq!(item["scope"], "GLOBAL");
        assert_eq!(item["release"]["version"], "1.2.3");
        assert_eq!(item["release"]["interface"]["short_description"], "alpha does things");
        let url = item["release"]["bundle_download_url"].as_str().unwrap();
        assert!(url.starts_with("https://localhost:8000/codexplus/plugin-bundles/plugin_a.tar.gz"));
        assert!(item["release"].get("signing").is_none(), "不带签名信息，Codex 按旧版不校验");
        let listed = directory_item(catalog.plugin("plugin_a").unwrap(), false);
        assert!(listed["release"]["bundle_download_url"].is_null());
    }

    #[test]
    fn hosted_only_connectors_are_listed_but_unavailable() {
        let catalog = catalog();
        let hosted = directory_item(catalog.plugin("plugin_b").unwrap(), true);
        assert_eq!(hosted["status"], "DISABLED_BY_ADMIN");
        assert_eq!(hosted["installation_policy"], "NOT_AVAILABLE");
        assert_eq!(hosted["disabled_reason"], "required_app_unavailable");
        assert!(hosted["release"]["bundle_download_url"].is_null());
        // 托管连接器加技能的照常能装（技能能用）。
        let mixed = directory_item(catalog.plugin("plugin_c").unwrap(), true);
        assert_eq!(mixed["status"], "ENABLED");
        let store = InstalledStore::at(tempfile::tempdir().unwrap().keep().join("installed.json"));
        assert_eq!(install_reply(&catalog, "plugin_b", &store).status, 403);
    }

    #[test]
    fn home_uses_the_language_and_puts_installable_first() {
        let catalog = catalog();
        let zh = body(home_reply(&catalog, "zh-cn,zh;q=0.9"));
        let section = &zh["sections"][0];
        assert_eq!(section["title"], "热门");
        assert_eq!(section["url_slug"], "featured");
        let ids: Vec<&str> = section["plugins"].as_array().unwrap().iter().map(|item| item["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["plugin_a", "plugin_b"], "能装的在前，目录里没有的跳过");
        let en = body(home_reply(&catalog, "en-US"));
        assert_eq!(en["sections"][0]["title"], "Featured");
    }

    #[test]
    fn search_ranks_name_matches_first() {
        let catalog = catalog();
        let found = body(search_reply(&catalog, &query("q=alpha&limit=50")));
        assert_eq!(found["plugins"][0]["id"], "plugin_a");
        let none = body(search_reply(&catalog, &query("q=zzzz")));
        assert!(none["plugins"].as_array().unwrap().is_empty());
    }

    #[test]
    fn install_and_uninstall_update_the_installed_list() {
        let catalog = catalog();
        let store = InstalledStore::at(tempfile::tempdir().unwrap().keep().join("installed.json"));
        let installed = body(install_reply(&catalog, "plugin_a", &store));
        assert_eq!(installed, json!({ "id": "plugin_a", "enabled": true, "app_ids_needing_auth": [] }));
        let listed = body(installed_reply(&catalog, &query("limit=200&includeDownloadUrls=true"), &store.load()));
        let item = &listed["plugins"][0];
        assert_eq!(item["id"], "plugin_a");
        assert_eq!(item["enabled"], true);
        assert!(item["installed_at"].as_str().unwrap().contains('T'));
        assert!(item["release"]["bundle_download_url"].is_string());
        let removed = body(uninstall_reply("plugin_a", &store));
        assert_eq!(removed["enabled"], false);
        assert!(store.load().plugins.is_empty());
    }

    #[test]
    fn batch_returns_known_ids_with_install_state() {
        let catalog = catalog();
        let mut installed = InstalledPlugins::default();
        installed.plugins.insert(
            "plugin_a".into(),
            InstalledRecord { installed_at: "2026-11-01T00:00:00Z".into(), enabled: true },
        );
        let reply = body(batch_reply(&catalog, br#"{"plugin_ids":["plugin_a","plugin_b","nope"]}"#, &installed));
        let results = reply["results"].as_array().unwrap();
        assert_eq!(results.len(), 2, "目录里没有的不列");
        assert_eq!(results[0]["requested_plugin_id"], "plugin_a");
        assert_eq!(results[0]["plugin"]["enabled"], true);
        assert!(results[1]["plugin"].get("enabled").is_none(), "没装的不带安装状态");
        assert_eq!(body(batch_reply(&catalog, b"not json", &installed))["results"], json!([]));
    }

    #[test]
    fn only_plugin_paths_are_handled_here() {
        assert!(valid_id("plugin_asdk_app_6adb~x-1"));
        assert!(!valid_id("../etc"));
        assert!(!valid_id(""));
    }
}
