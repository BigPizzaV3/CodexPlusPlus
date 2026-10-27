//! OpenCode 的模型网关：OpenCode Go（按月订阅的开源编码模型）与 OpenCode Zen（按量计费）。
//!
//! 两者都在 opencode.ai 下，用 OpenCode 控制台里的 Key。同一个网关里每个模型只在一种协议上提供：
//! GPT / Grok / Muse 走 Responses，Claude、MiniMax 与部分 Qwen 走 Anthropic Messages，Gemini 走
//! Google 的 generateContent，其余走 Chat Completions；问错协议会被拒（Model does not support this
//! protocol）。每个模型用哪种协议、发到哪个地址，以 models.dev 目录里该模型的 SDK 包与地址为准
//! （与 OpenCode 自己的取法一致），目录读不到时按模型名推断。
//!
//! 网关还要求每个请求带 `x-opencode-session`（缺了回 400），会话 id 按对话保持不变，
//! 上游按它路由和缓存。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use serde_json::Value;

use crate::settings::{RelayHeaderKeyValue, RelayProfile, RelayProtocol};

const HOST: &str = "opencode.ai";
/// models.dev 里两个网关的目录 id。
const GO_CATALOG: &str = "opencode-go";
const ZEN_CATALOG: &str = "opencode";
/// 推断时用的推理 API 根地址。
const GO_ROOT: &str = "https://opencode.ai/inference/go";
const ZEN_ROOT: &str = "https://opencode.ai/inference";

/// 地址属于哪个 OpenCode 网关。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    /// OpenCode Go：订阅额度。
    Go,
    /// OpenCode Zen：工作区的按量额度。
    Zen,
}

impl Plan {
    fn catalog_id(self) -> &'static str {
        match self {
            Plan::Go => GO_CATALOG,
            Plan::Zen => ZEN_CATALOG,
        }
    }

    fn root(self) -> &'static str {
        match self {
            Plan::Go => GO_ROOT,
            Plan::Zen => ZEN_ROOT,
        }
    }
}

/// `https://opencode.ai/zen/go/v1`、`/inference/go/...` 是 Go；`/zen/...`、`/inference/...` 是 Zen；
/// 其它地址（包括 `/inference/custom` 这种用户自带厂商的）返回 None。
pub fn plan_of(base_url: &str) -> Option<Plan> {
    let rest = base_url.trim().trim_end_matches('#');
    let rest = rest.split_once("://").map_or(rest, |(_, rest)| rest);
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = host
        .split(':')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if host != HOST && !host.ends_with(".opencode.ai") {
        return None;
    }
    let mut segments = path.split('/').filter(|segment| !segment.is_empty());
    match segments.next() {
        Some("zen" | "inference") => {}
        _ => return None,
    }
    match segments.next() {
        Some("go") => Some(Plan::Go),
        Some("custom") => None,
        _ => Some(Plan::Zen),
    }
}

/// 一个模型在 OpenCode 网关上的去处。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    pub protocol: RelayProtocol,
    pub base_url: String,
}

/// 目录里的一个模型。`route` 为 None 表示网关不支持它的协议（Gemini）。
#[derive(Debug, Clone, PartialEq)]
pub struct CatalogEntry {
    pub id: String,
    pub route: Option<Route>,
    pub deprecated: bool,
    /// 免费模型只对 OpenCode 自己的客户端开放。
    pub free: bool,
}

/// SDK 包 → 协议：`@ai-sdk/openai` 用 Responses，`@ai-sdk/anthropic` 用 Messages，
/// `@ai-sdk/google` 是 Gemini 的接口（不支持），其余都是 OpenAI 兼容的 Chat。
fn protocol_of_npm(npm: &str) -> Option<RelayProtocol> {
    match npm {
        "@ai-sdk/openai" => Some(RelayProtocol::Responses),
        "@ai-sdk/anthropic" => Some(RelayProtocol::AnthropicMessages),
        "@ai-sdk/google" | "@ai-sdk/google-vertex" => None,
        _ => Some(RelayProtocol::ChatCompletions),
    }
}

/// 从 models.dev 的完整目录里取出某个网关的模型。
pub fn parse_catalog(root: &Value, plan: Plan) -> Vec<CatalogEntry> {
    let Some(provider) = root.get(plan.catalog_id()) else {
        return Vec::new();
    };
    let default_npm = provider
        .get("npm")
        .and_then(Value::as_str)
        .unwrap_or("@ai-sdk/openai-compatible");
    let default_api = provider
        .get("api")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut entries: Vec<CatalogEntry> = provider
        .get("models")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .map(|(key, model)| {
            let id = model
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or(key)
                .to_string();
            let npm = model
                .pointer("/provider/npm")
                .and_then(Value::as_str)
                .unwrap_or(default_npm);
            let api = model
                .pointer("/provider/api")
                .and_then(Value::as_str)
                .unwrap_or(default_api);
            let route = protocol_of_npm(npm)
                .filter(|_| api.starts_with("https://"))
                .map(|protocol| Route {
                    protocol,
                    base_url: api.trim_end_matches('/').to_string(),
                });
            let cost = |field: &str| {
                model
                    .pointer(&format!("/cost/{field}"))
                    .and_then(Value::as_f64)
            };
            let free = id.ends_with("-free")
                || (cost("input") == Some(0.0) && cost("output") == Some(0.0));
            CatalogEntry {
                deprecated: model.get("status").and_then(Value::as_str) == Some("deprecated"),
                free,
                route,
                id,
            }
        })
        .collect();
    entries.sort_by(|left, right| left.id.cmp(&right.id));
    entries
}

/// 目录读不到时按模型名推断（OpenCode 的取法：GPT / Grok / Muse 用 Responses，Claude 用 Messages，
/// Gemini 走不了，其余用 Chat）。MiniMax、部分 Qwen 在 Go 上其实走 Messages，只能靠目录。
pub fn guess_route(plan: Plan, model: &str) -> Option<Route> {
    let name = model.to_ascii_lowercase();
    let root = plan.root();
    let (protocol, path) = if name.starts_with("gemini") {
        return None;
    } else if name.starts_with("claude") {
        (RelayProtocol::AnthropicMessages, "anthropic/v1")
    } else if name.starts_with("gpt") || name.starts_with("grok") || name.starts_with("muse") {
        (RelayProtocol::Responses, "openai/v1")
    } else {
        (RelayProtocol::ChatCompletions, "openai/v1")
    };
    Some(Route {
        protocol,
        base_url: format!("{root}/{path}"),
    })
}

/* —————————————— 目录缓存 —————————————— */

type Routes = HashMap<String, Option<Route>>;

#[derive(Default)]
struct Cache {
    /// 按网关的模型 → 去处；从磁盘上的 models.dev 缓存读出。
    go: Option<Routes>,
    zen: Option<Routes>,
    /// 磁盘缓存的修改时间，变了就重读。
    stamp: Option<std::time::SystemTime>,
    refreshing: bool,
}

fn route_cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

fn routes_of(entries: Vec<CatalogEntry>) -> Routes {
    entries
        .into_iter()
        .map(|entry| (entry.id, entry.route))
        .collect()
}

/// 从磁盘上的 models.dev 缓存（价格表共用的那份）里查某个模型的去处。
/// `Some(None)` 表示目录里有这个模型但协议不支持；`None` 表示目录里没有（或还没有缓存）。
fn cached_route(plan: Plan, model: &str) -> Option<Option<Route>> {
    let path = crate::model_pricing::cache_path();
    let stamp = std::fs::metadata(&path)
        .and_then(|meta| meta.modified())
        .ok();
    let mut cache = route_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if stamp.is_some() && cache.stamp != stamp {
        if let Some(root) = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        {
            cache.go = Some(routes_of(parse_catalog(&root, Plan::Go)));
            cache.zen = Some(routes_of(parse_catalog(&root, Plan::Zen)));
        }
        cache.stamp = stamp;
    }
    let missing = stamp.is_none();
    if missing && !cache.refreshing {
        // 还没有缓存：后台下载一份，这一次先按模型名推断。
        cache.refreshing = true;
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async {
                let _ = crate::model_pricing::load_catalog(false).await;
                route_cache()
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .refreshing = false;
            });
        }
    }
    let routes = match plan {
        Plan::Go => cache.go.as_ref(),
        Plan::Zen => cache.zen.as_ref(),
    }?;
    routes.get(model).cloned()
}

/// 某个模型的去处：先查目录，目录里没有就推断。Err 表示这个模型在网关里走不了。
pub fn route_for(plan: Plan, model: &str) -> Result<Route, String> {
    let route = match cached_route(plan, model) {
        Some(route) => route,
        None => guess_route(plan, model),
    };
    route.ok_or_else(|| format!("OpenCode 的 {model} 只走 Google Gemini 接口，网关暂不支持"))
}

/// 「从上游获取模型」：网关能用的模型（排除已下线、免费、Gemini）。读不到目录时返回 Err。
pub async fn catalog_models(plan: Plan) -> anyhow::Result<Vec<String>> {
    crate::model_pricing::load_catalog(false).await?;
    let text = std::fs::read_to_string(crate::model_pricing::cache_path())?;
    let root: Value = serde_json::from_str(&text)?;
    Ok(usable_models(&parse_catalog(&root, plan)))
}

pub fn usable_models(entries: &[CatalogEntry]) -> Vec<String> {
    entries
        .iter()
        .filter(|entry| !entry.deprecated && !entry.free && entry.route.is_some())
        .map(|entry| entry.id.clone())
        .collect()
}

/* —————————————— 会话 id —————————————— */

const BASE62: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
/// 记住多少个对话的会话 id。
const KEEP_SESSIONS: usize = 1000;

/// OpenCode 生成会话 id 的格式：`ses_` + 12 位十六进制（毫秒时间 × 0x1000 + 计数，按位取反让新的排前面）
/// + 14 位 base62 随机字符。
fn new_session_id() -> String {
    static COUNTER: OnceLock<Mutex<(u64, u64)>> = OnceLock::new();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_millis() as u64)
        .unwrap_or_default();
    let count = {
        let mut state = COUNTER
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.0 != now {
            *state = (now, 0);
        }
        state.1 += 1;
        state.1
    };
    let current = !(now.wrapping_mul(0x1000).wrapping_add(count)) & 0xffff_ffff_ffff;
    let random = uuid::Uuid::new_v4();
    let tail: String = random
        .as_bytes()
        .iter()
        .take(14)
        .map(|byte| BASE62[*byte as usize % BASE62.len()] as char)
        .collect();
    format!("ses_{current:012x}{tail}")
}

/// 同一个对话（`seed`）每一轮用同一个会话 id；没有 seed 时每次新给一个。
pub fn session_id(seed: Option<&str>) -> String {
    let Some(seed) = seed.map(str::trim).filter(|seed| !seed.is_empty()) else {
        return new_session_id();
    };
    if seed.starts_with("ses_") {
        return seed.to_string();
    }
    static SESSIONS: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    let mut sessions = SESSIONS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(id) = sessions.get(seed) {
        return id.clone();
    }
    if sessions.len() >= KEEP_SESSIONS {
        sessions.clear();
    }
    let id = new_session_id();
    sessions.insert(seed.to_string(), id.clone());
    id
}

/// 发往 OpenCode 网关前：按模型换成它所在的协议和地址，并带上会话头。不是 OpenCode 的地址原样返回。
pub fn prepare(relay: &mut RelayProfile, model: &str, seed: Option<&str>) -> Result<(), String> {
    let base = if relay.upstream_base_url.trim().is_empty() {
        relay.base_url.clone()
    } else {
        relay.upstream_base_url.clone()
    };
    let Some(plan) = plan_of(&base) else {
        return Ok(());
    };
    let route = route_for(plan, model)?;
    relay.protocol = route.protocol;
    relay.base_url = route.base_url;
    relay.upstream_base_url.clear();
    let id = session_id(seed);
    relay.custom_headers.retain(|header| {
        !header.key.eq_ignore_ascii_case("x-opencode-session")
            && !header.key.eq_ignore_ascii_case("x-opencode-session-id")
    });
    for key in ["x-opencode-session", "x-opencode-session-id"] {
        relay.custom_headers.push(RelayHeaderKeyValue {
            key: key.to_string(),
            value: id.clone(),
        });
    }
    Ok(())
}

/* —————————————— 从 OpenCode 导入 Key —————————————— */

/// OpenCode 自己保存的 Key（`opencode auth login` 写的 auth.json）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredKey {
    /// OpenCode 里的供应商 id：`opencode-go` / `opencode`。
    pub id: String,
    pub name: String,
}

/// auth.json 的位置：`$XDG_DATA_HOME/opencode`，否则 `~/.local/share/opencode`（Windows 上也一样）。
pub fn auth_path() -> std::path::PathBuf {
    if let Some(dir) = std::env::var_os("XDG_DATA_HOME").filter(|dir| !dir.is_empty()) {
        return std::path::PathBuf::from(dir)
            .join("opencode")
            .join("auth.json");
    }
    directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().to_path_buf())
        .unwrap_or_default()
        .join(".local")
        .join("share")
        .join("opencode")
        .join("auth.json")
}

/// 能导入的 OpenCode Go / Zen Key（只认 `type: api`；登录类的不是 Key）。
pub fn stored_keys(auth: &Value) -> Vec<(StoredKey, String)> {
    [(GO_CATALOG, "OpenCode Go"), (ZEN_CATALOG, "OpenCode Zen")]
        .into_iter()
        .filter_map(|(id, name)| {
            let entry = auth.get(id)?;
            if entry.get("type").and_then(Value::as_str) != Some("api") {
                return None;
            }
            let key = entry.get("key").and_then(Value::as_str)?.trim();
            (!key.is_empty()).then(|| {
                (
                    StoredKey {
                        id: id.to_string(),
                        name: name.to_string(),
                    },
                    key.to_string(),
                )
            })
        })
        .collect()
}

pub fn load_stored_keys() -> Vec<(StoredKey, String)> {
    std::fs::read_to_string(auth_path())
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')).ok())
        .map(|auth| stored_keys(&auth))
        .unwrap_or_default()
}

/// 两个网关在「添加供应商」里用的地址。
pub fn base_url_of(id: &str) -> Option<&'static str> {
    match id {
        GO_CATALOG => Some("https://opencode.ai/zen/go/v1"),
        ZEN_CATALOG => Some("https://opencode.ai/zen/v1"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn catalog() -> Value {
        json!({
            "opencode-go": {
                "npm": "@ai-sdk/openai-compatible",
                "api": "https://opencode.ai/inference/go/openai/v1",
                "models": {
                    "glm-5.3": { "id": "glm-5.3", "provider": { "api": "https://opencode.ai/inference/go/openai/v1" }, "cost": { "input": 1.4, "output": 4 } },
                    "minimax-m3": { "id": "minimax-m3", "provider": { "npm": "@ai-sdk/anthropic", "api": "https://opencode.ai/inference/go/anthropic/v1" }, "cost": { "input": 0.3, "output": 1.2 } },
                    "gpt-6-luna": { "id": "gpt-6-luna", "provider": { "npm": "@ai-sdk/openai", "api": "https://opencode.ai/inference/go/openai/v1" }, "cost": { "input": 0.1, "output": 0.4 } },
                    "gemini-4-argon": { "id": "gemini-4-argon", "provider": { "npm": "@ai-sdk/google", "api": "https://opencode.ai/inference/go/google/v1beta" }, "cost": { "input": 2, "output": 8 } },
                    "beam-free": { "id": "beam-free", "cost": { "input": 0, "output": 0 } },
                    "space-bunny": { "id": "space-bunny", "status": "deprecated", "cost": { "input": 0.15, "output": 0.6 } }
                }
            }
        })
    }

    #[test]
    fn plans_are_told_by_the_address() {
        assert_eq!(plan_of("https://opencode.ai/zen/go/v1"), Some(Plan::Go));
        assert_eq!(
            plan_of("https://opencode.ai/inference/go/anthropic/v1"),
            Some(Plan::Go)
        );
        assert_eq!(plan_of("https://opencode.ai/zen/v1"), Some(Plan::Zen));
        assert_eq!(
            plan_of("https://opencode.ai/inference/openai/v1#"),
            Some(Plan::Zen)
        );
        assert_eq!(plan_of("https://opencode.ai/inference/custom/v1"), None);
        assert_eq!(plan_of("https://opencode.ai/docs"), None);
        assert_eq!(plan_of("https://api.deepseek.com/v1"), None);
    }

    #[test]
    fn each_model_goes_to_the_protocol_the_catalog_names() {
        let entries = parse_catalog(&catalog(), Plan::Go);
        let route = |id: &str| {
            entries
                .iter()
                .find(|entry| entry.id == id)
                .unwrap()
                .route
                .clone()
        };
        assert_eq!(
            route("minimax-m3"),
            Some(Route {
                protocol: RelayProtocol::AnthropicMessages,
                base_url: "https://opencode.ai/inference/go/anthropic/v1".into()
            })
        );
        assert_eq!(
            route("gpt-6-luna").unwrap().protocol,
            RelayProtocol::Responses
        );
        assert_eq!(
            route("glm-5.3").unwrap().protocol,
            RelayProtocol::ChatCompletions
        );
        assert_eq!(route("gemini-4-argon"), None);
        assert_eq!(
            usable_models(&entries),
            vec!["glm-5.3", "gpt-6-luna", "minimax-m3"]
        );

        assert_eq!(
            guess_route(Plan::Zen, "claude-sonnet-5").unwrap().base_url,
            "https://opencode.ai/inference/anthropic/v1"
        );
        assert_eq!(
            guess_route(Plan::Go, "grok-4.7").unwrap().protocol,
            RelayProtocol::Responses
        );
        assert_eq!(
            guess_route(Plan::Go, "kimi-k3").unwrap().protocol,
            RelayProtocol::ChatCompletions
        );
        assert!(guess_route(Plan::Go, "gemini-4-argon").is_none());
    }

    #[test]
    fn prepare_switches_protocol_and_adds_the_session() {
        let mut relay = RelayProfile {
            base_url: "https://opencode.ai/zen/go/v1".into(),
            protocol: RelayProtocol::ChatCompletions,
            ..RelayProfile::default()
        };
        prepare(&mut relay, "claude-haiku-5-5", Some("conv-1")).unwrap();
        assert_eq!(relay.protocol, RelayProtocol::AnthropicMessages);
        assert!(
            relay
                .base_url
                .starts_with("https://opencode.ai/inference/go/")
        );
        let session = relay
            .custom_headers
            .iter()
            .find(|header| header.key == "x-opencode-session")
            .unwrap()
            .value
            .clone();
        assert!(
            session.starts_with("ses_") && session.len() == 4 + 12 + 14,
            "{session}"
        );
        // 同一个对话下一轮还是同一个会话。
        let mut again = relay.clone();
        prepare(&mut again, "claude-haiku-5-5", Some("conv-1")).unwrap();
        assert_eq!(
            again
                .custom_headers
                .iter()
                .filter(|header| header.key == "x-opencode-session")
                .count(),
            1
        );
        assert_eq!(session_id(Some("conv-1")), session);
        assert_ne!(session_id(Some("conv-2")), session);

        let mut other = RelayProfile {
            base_url: "https://api.deepseek.com/v1".into(),
            ..RelayProfile::default()
        };
        prepare(&mut other, "deepseek-v4", None).unwrap();
        assert!(other.custom_headers.is_empty());
    }

    #[test]
    fn stored_keys_are_read_from_opencode_auth() {
        let auth = json!({
            "opencode-go": { "type": "api", "key": " sk-go " },
            "opencode": { "type": "oauth", "access": "x" },
            "openai": { "type": "api", "key": "sk-openai" }
        });
        let keys = stored_keys(&auth);
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].0.name, "OpenCode Go");
        assert_eq!(keys[0].1, "sk-go");
        assert_eq!(
            base_url_of("opencode-go"),
            Some("https://opencode.ai/zen/go/v1")
        );
    }
}
