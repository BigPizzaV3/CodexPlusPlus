//! Codex 直连供应商在共享供应商里的「引用」（不复制）。
//!
//! 能共享的 Codex 直连供应商（见 [`linkable`]）默认就在共享供应商里。gateway.json 只存一条引用记录：
//! `linkedCodexRelay` 指向直连供应商的 id，外加价格、代理、限速、开关这些只属于共享供应商的设置；
//! 地址、协议、Key、模型列表、模型资料、上下文窗口和自动压缩阈值归直连供应商所有，每次读 gateway.json
//! 时从 settings.json 现取（[`sync`]），写盘时去掉（[`strip_for_storage`]），gateway.json 里没有第二份。
//! 在 Codex「连接与供应商」里改了，Claude 等经本地服务的 Agent 下一个请求就用上；删掉直连供应商
//! （或改成聚合、官方登录这些不能共享的），共享里的这一项连同模型组、Agent 绑定里指向它的引用一起清掉。
//!
//! 旧版「加入共享供应商」复制出来的（来源 `codex:<id>`）在直连供应商还在、能共享时原地转成引用，
//! id 不变，模型组成员、绑定里的 `<id>/<模型>` 照常有效。
//!
//! 同步放在读 gateway.json 的唯一入口（`store::read_store`）里：网关路由、模型列表、Agent 配置、
//! 界面数据、测试、额度都从这里读，不会有哪一处拿到旧数据；结构有变（新引用、转换、删除）时
//! 由 `GatewayStore::load` / `update` 顺手写回，id 从此固定。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::bail;
use serde_json::{Map, Value};

use super::store::{GatewayProvider, GatewaySettings, ProviderAutoCompact};
use crate::settings::{RelayMode, RelayProfile};

/// 引用记录（和旧版复制品）的来源前缀：`codex:<直连供应商 id>`。
pub const SOURCE_PREFIX: &str = "codex:";

/// 供应商归哪个 Agent 的页面（界面「显示其他 Agent 的供应商」按它筛）。
pub const OWNER_CODEX: &str = "codex";
pub const OWNER_CLAUDE: &str = "claude";

/// 从 Claude 页那几个来源导入的（Claude Code、CC Switch、ClaudeMom、Magpie），没记归属时也算 Claude 的。
const CLAUDE_SOURCES: &[&str] = &["claude:", "ccswitch:", "claudemom:", "magpie:"];

/// 这个直连供应商不能共享的原因；能共享返回 None。
///
/// 规则沿用旧版「加入共享供应商」：聚合供应商、用 Codex 自己的 ChatGPT 登录（auth.json）的官方账号、
/// 本地服务托管的那一条、指向本地服务本身的都不行；另外要有能用的连接（地址 + Key，或不校验 Key 的本机服务）。
/// 带 API Key 的官方混合供应商照旧可以。
pub fn rejection(relay: &RelayProfile) -> Option<String> {
    let name = if relay.name.trim().is_empty() { relay.id.as_str() } else { relay.name.as_str() };
    if relay.id == super::apply::CODEX_GATEWAY_RELAY_ID || relay.base_url.contains("/gw/") {
        return Some(format!("「{name}」指向本地服务本身，不能共享"));
    }
    if relay.relay_mode == RelayMode::Aggregate {
        return Some(format!("「{name}」是聚合供应商，本地服务的组请在「路由」页里新建"));
    }
    if crate::official_login::member_uses_official_login_transport(relay) {
        return Some(format!(
            "「{name}」是官方登录账号，不能共享：两边共用登录会互相把对方刷失效，请在本地服务里单独登录"
        ));
    }
    let has_key = !relay.api_key.trim().is_empty()
        || relay.api_keys.iter().any(|key| !key.api_key.trim().is_empty());
    if relay.base_url.trim().is_empty() || !(has_key || relay.uses_no_auth()) {
        return Some(format!("「{name}」没有 API Key（用的是 Codex 自己的登录），不能共享"));
    }
    None
}

/// 能不能作为共享供应商被引用。
pub fn linkable(relay: &RelayProfile) -> bool {
    rejection(relay).is_none()
}

/// 供应商的归属：引用 Codex 直连供应商的（和旧版复制品）是 `codex`；从 Claude 页添加或从 Claude 那几个来源
/// 导入的是 `claude`；在「本地服务」页添加的、登录的订阅账号没有归属（两边都列）。
pub fn owner_of(provider: &GatewayProvider) -> Option<&'static str> {
    if provider.linked_codex_relay.is_some() {
        return Some(OWNER_CODEX);
    }
    match provider.owner.trim() {
        OWNER_CODEX => return Some(OWNER_CODEX),
        OWNER_CLAUDE => return Some(OWNER_CLAUDE),
        _ => {}
    }
    if provider.source.starts_with(SOURCE_PREFIX) {
        return Some(OWNER_CODEX);
    }
    CLAUDE_SOURCES
        .iter()
        .any(|prefix| provider.source.starts_with(prefix))
        .then_some(OWNER_CLAUDE)
}

/// 新建供应商时调用方传的归属：只认 `claude`（Claude 页传），其余一律不记。
pub fn clean_owner(owner: Option<&str>) -> String {
    match owner.map(str::trim) {
        Some(OWNER_CLAUDE) => OWNER_CLAUDE.to_string(),
        _ => String::new(),
    }
}

/// 改引用记录里归直连供应商所有的东西时给的话。
pub fn locked_message(provider: &GatewayProvider) -> String {
    format!(
        "「{}」引用的是 Codex 直连供应商：地址、协议、Key、模型列表、上下文窗口和模型资料请到 Codex「连接与供应商」里改，这里会跟着变",
        provider.relay.name
    )
}

/// 引用记录不能改连接资料：直接报错，提示去 Codex「连接与供应商」。
pub fn ensure_unlinked(provider: &GatewayProvider) -> anyhow::Result<()> {
    if provider.linked_codex_relay.is_some() {
        bail!("{}", locked_message(provider));
    }
    Ok(())
}

/// 引用记录不能从共享供应商里单独删：它跟着直连供应商走。
pub fn ensure_deletable(provider: &GatewayProvider) -> anyhow::Result<()> {
    if provider.linked_codex_relay.is_some() {
        bail!(
            "「{}」引用的是 Codex 直连供应商，不能在这里删除：在 Codex「连接与供应商」里删掉它，这里会一起去掉；只是不想给某个 Agent 用，关掉它的开关即可",
            provider.relay.name
        );
    }
    Ok(())
}

/// 一次同步的结果。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SyncReport {
    /// 引用记录有增删或旧复制品转成了引用：要写回 gateway.json。
    pub changed: bool,
    /// 因为直连供应商不在了（或不能共享了）被去掉的引用的 id；写回后清掉内存里按 id 记的 Key 健康、额度、速度。
    pub removed: Vec<String>,
}

/// 按直连供应商列表整理共享供应商里的引用，并把连接资料现取进来（幂等，每次读 gateway.json 都跑）：
/// 1. 引用的直连供应商不在了或不能共享了：去掉这一项，模型组、绑定里指向它的引用一并清掉（同删除）；
/// 2. 旧版复制品（来源 `codex:<id>`）在直连供应商还在、能共享、还没有别的引用时原地转成引用，id 不变；
/// 3. 还没进共享供应商的、能共享的直连供应商新建一条引用（id 同旧版复制时的取法，按名字取不重复的短名），
///    关了「添加供应商时自动加入」的 Agent 照旧记进排除名单；
/// 4. 所有引用从直连供应商取地址、协议、Key、模型、模型资料、窗口与压缩阈值。
pub fn sync(gateway: &mut GatewaySettings, profiles: &[RelayProfile]) -> SyncReport {
    let mut report = SyncReport::default();
    let usable = |relay_id: &str| profiles.iter().find(|relay| relay.id == relay_id && linkable(relay));

    let gone: Vec<String> = gateway
        .providers
        .iter()
        .filter(|provider| {
            provider
                .linked_codex_relay
                .as_deref()
                .is_some_and(|relay_id| usable(relay_id).is_none())
        })
        .map(|provider| provider.relay.id.clone())
        .collect();
    for id in gone {
        super::apply::forget_provider_in(gateway, &id);
        report.changed = true;
        report.removed.push(id);
    }

    for index in 0..gateway.providers.len() {
        let provider = &gateway.providers[index];
        if provider.linked_codex_relay.is_some() {
            continue;
        }
        let Some(relay_id) = provider.source.strip_prefix(SOURCE_PREFIX).map(str::to_string) else {
            continue;
        };
        if usable(&relay_id).is_none() || linked_to(gateway, &relay_id) {
            continue;
        }
        gateway.providers[index].linked_codex_relay = Some(relay_id);
        report.changed = true;
    }

    for relay in profiles.iter().filter(|relay| linkable(relay)) {
        if linked_to(gateway, &relay.id) {
            continue;
        }
        let id = gateway.unique_slug(&relay.name, false);
        let mut provider = GatewayProvider::linked(&id, &relay.id);
        provider.relay.name = relay.name.clone();
        gateway.providers.push(provider);
        super::apply::exclude_new_provider(gateway, &id, None);
        report.changed = true;
    }

    for provider in &mut gateway.providers {
        let Some(relay_id) = provider.linked_codex_relay.clone() else {
            continue;
        };
        if let Some(relay) = usable(&relay_id) {
            hydrate(provider, relay);
        }
    }
    report
}

fn linked_to(gateway: &GatewaySettings, relay_id: &str) -> bool {
    gateway
        .providers
        .iter()
        .any(|provider| provider.linked_codex_relay.as_deref() == Some(relay_id))
}

/// 把直连供应商的连接资料放进引用记录：上游配置整份换成直连供应商的（id 保持共享供应商自己的），
/// 窗口、自动压缩阈值按直连供应商的重算；价格、代理、限速、开关这些共享供应商自己的设置不动。
fn hydrate(provider: &mut GatewayProvider, relay: &RelayProfile) {
    let id = std::mem::take(&mut provider.relay.id);
    provider.relay = relay.clone();
    provider.relay.id = id;
    provider.model_windows.clear();
    provider.context_window = None;
    provider.adopt_relay_windows();
    provider.auto_compact = auto_compact_of(relay);
}

/// 直连供应商的按模型自动压缩百分比（`{"模型":"85%"}`）→ 共享供应商的阈值。写错的项跳过。
fn auto_compact_of(relay: &RelayProfile) -> ProviderAutoCompact {
    let map: BTreeMap<String, String> =
        serde_json::from_str(&relay.model_auto_compact).unwrap_or_default();
    let models = map
        .into_iter()
        .filter_map(|(model, token)| {
            let scaled = crate::model_suffix::parse_compact_percent(&token)?;
            let (slug, _) = crate::model_suffix::parse_model_suffix(model.trim());
            let slug = slug.trim().to_string();
            (!slug.is_empty()).then_some((slug, scaled as f64 / 1_000_000.0))
        })
        .collect();
    ProviderAutoCompact { models, fallback: None }
}

/// 写盘前去掉引用记录里归直连供应商所有的部分：上游配置只留 id 和名字，窗口、压缩阈值不写。
pub(crate) fn strip_for_storage(provider: &mut Value) {
    let Some(object) = provider.as_object_mut() else {
        return;
    };
    let relay = object.get("relay").and_then(Value::as_object);
    let mut kept = Map::new();
    for key in ["id", "name"] {
        if let Some(value) = relay.and_then(|relay| relay.get(key)) {
            kept.insert(key.to_string(), value.clone());
        }
    }
    object.insert("relay".to_string(), Value::Object(kept));
    for key in ["modelWindows", "contextWindow", "autoCompact"] {
        object.remove(key);
    }
}

/// gateway.json 对应的 settings.json：默认位置的 gateway.json 用默认（可能被指针改过）的设置文件，
/// 别处的（测试、多开）用同目录的 `settings.json`，与用量归属读这两份文件时的对应一致。
fn settings_path_for(gateway_path: &Path) -> PathBuf {
    #[cfg(not(test))]
    if gateway_path == super::store::GatewayStore::default_path() {
        return crate::paths::default_settings_path();
    }
    gateway_path.with_file_name("settings.json")
}

type CachedProfiles = (PathBuf, Vec<u8>, Arc<Vec<RelayProfile>>);
static PROFILES: Mutex<Option<CachedProfiles>> = Mutex::new(None);

thread_local! {
    static LOADING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// 读 gateway.json 对应的直连供应商列表。网关每个请求都会走到这里：文件内容没变就用上次解析的结果。
/// 文件不存在时是空列表（引用全部去掉）；读不了、解析不了时返回 None，这次不动引用（免得一次读坏就清掉引用）。
pub(crate) fn codex_profiles_for(gateway_path: &Path) -> Option<Arc<Vec<RelayProfile>>> {
    if LOADING.with(|loading| loading.get()) {
        return None;
    }
    let path = settings_path_for(gateway_path);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Some(Arc::new(Vec::new()));
        }
        Err(_) => return None,
    };
    {
        let cache = PROFILES.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some((cached_path, cached_bytes, profiles)) = cache.as_ref()
            && cached_path == &path
            && cached_bytes == &bytes
        {
            return Some(profiles.clone());
        }
    }
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            LOADING.with(|loading| loading.set(false));
        }
    }
    LOADING.with(|loading| loading.set(true));
    let _reset = Reset;
    let settings = crate::settings::SettingsStore::new(path.clone()).load().ok()?;
    let profiles = Arc::new(settings.relay_profiles);
    *PROFILES.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) =
        Some((path, bytes, profiles.clone()));
    Some(profiles)
}
