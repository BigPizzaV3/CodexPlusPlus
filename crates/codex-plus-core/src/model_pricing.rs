//! 模型价格自动查询：用公开的 models.dev 价格表（按供应商分组，单位为每百万 token、美元）。
//!
//! 匹配顺序，越靠前越准：
//! 1. 供应商地址的主机名 == 价格表里某个供应商的 `api` 主机（例如 api.deepseek.com、api.moonshot.cn），
//!    在这个供应商下按模型名找；
//! 2. 全表按模型名找，优先取模型名前缀对应的原厂（claude→anthropic、glm→zhipuai……）。
//!
//! 模型名会依次尝试：原样、去掉 `[1M]` 这类窗口后缀、去掉 `vendor/` 前缀、去掉日期后缀。
//! 价格表下载后缓存一天；拉不到时用旧缓存，都没有就报错，由界面提示「手动填写」。

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, bail};
use serde::Serialize;
use serde_json::Value;

pub const PRICE_CATALOG_URL: &str = "https://models.dev/api.json";
const CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30);

/// 一次匹配结果。价格单位：每百万 token，美元。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PriceMatch {
    pub provider_id: String,
    pub provider_name: String,
    pub model_id: String,
    pub currency: String,
    pub input: f64,
    pub cached_input: f64,
    pub output: f64,
    /// 上下文窗口（token），价格表里有才给。
    pub context: Option<u64>,
}

#[derive(Debug, Clone)]
struct CatalogModel {
    id: String,
    input: f64,
    output: f64,
    cache_read: Option<f64>,
    context: Option<u64>,
}

#[derive(Debug, Clone)]
struct CatalogProvider {
    id: String,
    name: String,
    host: Option<String>,
    models: Vec<CatalogModel>,
}

#[derive(Debug, Clone, Default)]
pub struct PriceCatalog {
    providers: Vec<CatalogProvider>,
}

impl PriceCatalog {
    pub fn parse(json: &str) -> anyhow::Result<Self> {
        let root: Value = serde_json::from_str(json).context("价格表不是合法 JSON")?;
        let Some(object) = root.as_object() else {
            bail!("价格表格式不对");
        };
        let mut providers = Vec::new();
        for (key, provider) in object {
            let Some(models) = provider.get("models").and_then(Value::as_object) else { continue };
            let models = models
                .iter()
                .filter_map(|(model_key, model)| {
                    let cost = model.get("cost")?;
                    let input = cost.get("input").and_then(Value::as_f64)?;
                    let output = cost.get("output").and_then(Value::as_f64)?;
                    Some(CatalogModel {
                        id: model.get("id").and_then(Value::as_str).unwrap_or(model_key).to_string(),
                        input,
                        output,
                        cache_read: cost.get("cache_read").and_then(Value::as_f64),
                        context: model.pointer("/limit/context").and_then(Value::as_u64),
                    })
                })
                .collect::<Vec<_>>();
            if models.is_empty() {
                continue;
            }
            providers.push(CatalogProvider {
                id: provider.get("id").and_then(Value::as_str).unwrap_or(key).to_string(),
                name: provider.get("name").and_then(Value::as_str).unwrap_or(key).to_string(),
                host: provider.get("api").and_then(Value::as_str).and_then(host_of),
                models,
            });
        }
        providers.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(Self { providers })
    }

    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }

    /// 按供应商地址与模型名查价格。
    pub fn lookup(&self, base_url: &str, model: &str) -> Option<PriceMatch> {
        let names = name_candidates(model);
        if names.is_empty() {
            return None;
        }
        if let Some(host) = host_of(base_url) {
            // 原厂官方接口在价格表里没写 api 地址，用固定对照补上。
            let official = OFFICIAL_HOSTS.iter().find(|(official, _)| hosts_match(official, &host)).map(|(_, id)| *id);
            let by_host = |provider: &&CatalogProvider| {
                provider.host.as_deref().is_some_and(|api| hosts_match(api, &host)) || official == Some(provider.id.as_str())
            };
            for provider in self.providers.iter().filter(by_host) {
                if let Some(found) = find_model(provider, &names) {
                    return Some(to_match(provider, found));
                }
            }
        }
        let hint = vendor_hint(model);
        let mut fallback = None;
        for provider in &self.providers {
            if let Some(found) = find_model(provider, &names) {
                if hint.is_some_and(|hint| provider.id == hint) {
                    return Some(to_match(provider, found));
                }
                fallback.get_or_insert_with(|| to_match(provider, found));
            }
        }
        fallback
    }
}

fn to_match(provider: &CatalogProvider, model: &CatalogModel) -> PriceMatch {
    PriceMatch {
        provider_id: provider.id.clone(),
        provider_name: provider.name.clone(),
        model_id: model.id.clone(),
        currency: "USD".to_string(),
        input: model.input,
        // 没单列缓存价的按输入价算，宁可高估。
        cached_input: model.cache_read.unwrap_or(model.input),
        output: model.output,
        context: model.context,
    }
}

fn find_model<'a>(provider: &'a CatalogProvider, names: &[String]) -> Option<&'a CatalogModel> {
    names.iter().find_map(|name| provider.models.iter().find(|model| model.id.eq_ignore_ascii_case(name)))
}

/// 模型名的几种写法，按精确程度排列并去重。
fn name_candidates(model: &str) -> Vec<String> {
    let mut base = model.trim().to_lowercase();
    if let Some(index) = base.find('[') {
        base.truncate(index);
        base = base.trim().to_string();
    }
    if base.is_empty() {
        return Vec::new();
    }
    let without_vendor = base.rsplit('/').next().unwrap_or(&base).to_string();
    let mut names = Vec::new();
    for name in [base.clone(), without_vendor.clone(), strip_date(&base), strip_date(&without_vendor)] {
        if !name.is_empty() && !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// 去掉结尾的日期：`-20251101`、`-2025-11-01`。
fn strip_date(name: &str) -> String {
    let bytes = name.as_bytes();
    let digits = |slice: &[u8]| slice.iter().all(u8::is_ascii_digit);
    if bytes.len() > 9 && bytes[bytes.len() - 9] == b'-' && digits(&bytes[bytes.len() - 8..]) {
        return name[..name.len() - 9].to_string();
    }
    if bytes.len() > 11
        && bytes[bytes.len() - 11] == b'-'
        && digits(&bytes[bytes.len() - 10..bytes.len() - 6])
        && bytes[bytes.len() - 6] == b'-'
        && bytes[bytes.len() - 3] == b'-'
    {
        return name[..name.len() - 11].to_string();
    }
    name.to_string()
}

/// 价格表里没有 `api` 字段的原厂：官方主机 → 供应商 id。
const OFFICIAL_HOSTS: [(&str, &str); 5] = [
    ("api.anthropic.com", "anthropic"),
    ("api.openai.com", "openai"),
    ("api.x.ai", "xai"),
    ("generativelanguage.googleapis.com", "google"),
    ("api.mistral.ai", "mistral"),
];

/// 模型名前缀 → 价格表里原厂的供应商 id，用于全表查找时优先取原厂价。
fn vendor_hint(model: &str) -> Option<&'static str> {
    let name = model.trim().to_lowercase();
    let name = name.rsplit('/').next().unwrap_or(&name);
    let hints: [(&str, &str); 12] = [
        ("claude", "anthropic"),
        ("gpt", "openai"),
        ("o1", "openai"),
        ("o3", "openai"),
        ("o4", "openai"),
        ("gemini", "google"),
        ("deepseek", "deepseek"),
        ("kimi", "moonshotai"),
        ("moonshot", "moonshotai"),
        ("glm", "zhipuai"),
        ("qwen", "alibaba"),
        ("grok", "xai"),
    ];
    hints.iter().find(|(prefix, _)| name.starts_with(prefix)).map(|(_, vendor)| *vendor)
}

fn host_of(url: &str) -> Option<String> {
    let trimmed = url.trim();
    let without_scheme = trimmed.split_once("://").map(|(_, rest)| rest).unwrap_or(trimmed);
    let host = without_scheme.split(['/', '?', '#']).next()?.split('@').last()?.split(':').next()?.to_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
    (!host.is_empty()).then_some(host)
}

/// 主机名相同，或一方是另一方的子域（例如 api.example.com 与 example.com）。
fn hosts_match(api: &str, host: &str) -> bool {
    api == host || host.ends_with(&format!(".{api}")) || api.ends_with(&format!(".{host}"))
}

// ---------------------------------------------------------------------------
// 下载与缓存
// ---------------------------------------------------------------------------

pub fn cache_path() -> PathBuf {
    crate::paths::default_app_state_dir().join("cache").join("model-prices.json")
}

/// 价格表与它的获取时间（毫秒）。`refresh` 为真时忽略缓存时效、强制重新下载。
pub async fn load_catalog(refresh: bool) -> anyhow::Result<(PriceCatalog, u64)> {
    let path = cache_path();
    let cached = std::fs::read_to_string(&path).ok();
    let cached_at = std::fs::metadata(&path).and_then(|metadata| metadata.modified()).ok();
    let fresh = cached_at.is_some_and(|time| time.elapsed().map(|age| age < CACHE_TTL).unwrap_or(false));
    if let (Some(text), true, false) = (&cached, fresh, refresh) {
        if let Ok(catalog) = PriceCatalog::parse(text) {
            return Ok((catalog, millis(cached_at.unwrap_or(SystemTime::now()))));
        }
    }
    match download().await {
        Ok(text) => {
            let catalog = PriceCatalog::parse(&text)?;
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = crate::settings::atomic_write(&path, text.as_bytes());
            Ok((catalog, millis(SystemTime::now())))
        }
        Err(error) => match cached.as_deref().map(PriceCatalog::parse) {
            Some(Ok(catalog)) => Ok((catalog, millis(cached_at.unwrap_or(SystemTime::now())))),
            _ => Err(error.context("下载价格表失败，也没有本地缓存")),
        },
    }
}

async fn download() -> anyhow::Result<String> {
    let client = crate::http_client::proxied_client("codex-plus-plus")?;
    let response = tokio::time::timeout(DOWNLOAD_TIMEOUT, client.get(PRICE_CATALOG_URL).send())
        .await
        .context("下载价格表超时")??;
    if !response.status().is_success() {
        bail!("下载价格表失败：HTTP {}", response.status());
    }
    Ok(tokio::time::timeout(DOWNLOAD_TIMEOUT, response.text()).await.context("读取价格表超时")??)
}

fn millis(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH).map(|value| value.as_millis() as u64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"{
      "deepseek": { "id": "deepseek", "name": "DeepSeek", "api": "https://api.deepseek.com",
        "models": { "deepseek-chat": { "id": "deepseek-chat", "cost": { "input": 0.27, "output": 1.1, "cache_read": 0.07 }, "limit": { "context": 128000 } } } },
      "anthropic": { "id": "anthropic", "name": "Anthropic",
        "models": { "claude-sonnet-4-5": { "id": "claude-sonnet-4-5", "cost": { "input": 3, "output": 15, "cache_read": 0.3 } } } },
      "openrouter": { "id": "openrouter", "name": "OpenRouter", "api": "https://openrouter.ai/api/v1",
        "models": { "anthropic/claude-sonnet-4-5": { "id": "anthropic/claude-sonnet-4-5", "cost": { "input": 3.3, "output": 16 } } } },
      "zhipuai": { "id": "zhipuai", "name": "Zhipu AI", "api": "https://open.bigmodel.cn/api/paas/v4",
        "models": { "glm-4.6": { "id": "glm-4.6", "cost": { "input": 0.6, "output": 2.2 } } } },
      "empty": { "id": "empty", "name": "No prices", "models": { "x": { "id": "x" } } }
    }"#;

    fn catalog() -> PriceCatalog {
        PriceCatalog::parse(FIXTURE).unwrap()
    }

    #[test]
    fn host_match_wins_and_window_suffix_is_ignored() {
        let found = catalog().lookup("https://api.deepseek.com/v1", "deepseek-chat[128K]").unwrap();
        assert_eq!(found.provider_id, "deepseek");
        assert_eq!((found.input, found.cached_input, found.output), (0.27, 0.07, 1.1));
        assert_eq!(found.context, Some(128000));
        assert_eq!(found.currency, "USD");
    }

    #[test]
    fn relay_host_uses_its_own_price_list() {
        let found = catalog().lookup("https://openrouter.ai/api/v1", "anthropic/claude-sonnet-4-5").unwrap();
        assert_eq!(found.provider_id, "openrouter");
        assert_eq!(found.input, 3.3);
        assert_eq!(found.cached_input, 3.3, "没有缓存价时按输入价");
    }

    #[test]
    fn unknown_host_falls_back_to_original_vendor() {
        let found = catalog().lookup("https://relay.example.com/v1", "claude-sonnet-4-5-20250929").unwrap();
        assert_eq!(found.provider_id, "anthropic", "去掉日期后缀、优先原厂");
        let found = catalog().lookup("https://relay.example.com/v1", "zhipu/GLM-4.6").unwrap();
        assert_eq!(found.provider_id, "zhipuai", "去掉 vendor 前缀、大小写不敏感");
    }

    #[test]
    fn official_hosts_without_api_field_still_match_by_host() {
        let found = catalog().lookup("https://api.anthropic.com", "claude-sonnet-4-5").unwrap();
        assert_eq!(found.provider_id, "anthropic");
    }

    #[test]
    fn misses_and_models_without_cost() {
        assert!(catalog().lookup("https://api.deepseek.com", "not-a-model").is_none());
        assert!(catalog().lookup("https://x.com", "x").is_none(), "没有价格的模型不算");
        assert!(catalog().lookup("https://x.com", "  ").is_none());
    }

    /// 用真实价格表跑一遍常见组合：`MODELS_DEV_JSON=<api.json 路径> cargo test -- --ignored real_catalog`。
    #[test]
    #[ignore]
    fn real_catalog_smoke() {
        let path = std::env::var("MODELS_DEV_JSON").expect("设置 MODELS_DEV_JSON");
        let catalog = PriceCatalog::parse(&std::fs::read_to_string(path).unwrap()).unwrap();
        for (url, model) in [
            ("https://api.deepseek.com/v1", "deepseek-v4-flash"),
            ("https://api.moonshot.cn/v1", "kimi-k2.7-code"),
            ("https://open.bigmodel.cn/api/paas/v4", "glm-4.7"),
            ("https://api.anthropic.com", "claude-sonnet-4-5-20250929"),
            ("https://relay.example.com/v1", "gpt-5"),
        ] {
            let found = catalog.lookup(url, model);
            eprintln!("{url} {model} => {found:?}");
            assert!(found.is_some(), "{url} {model}");
        }
    }

    #[test]
    fn helpers() {
        assert_eq!(strip_date("claude-opus-4-5-20251101"), "claude-opus-4-5");
        assert_eq!(strip_date("gpt-4o-2024-08-06"), "gpt-4o");
        assert_eq!(strip_date("glm-4.6"), "glm-4.6");
        assert_eq!(host_of("https://user@api.deepseek.com:443/v1").as_deref(), Some("api.deepseek.com"));
        assert!(hosts_match("bigmodel.cn", "open.bigmodel.cn"));
    }
}
