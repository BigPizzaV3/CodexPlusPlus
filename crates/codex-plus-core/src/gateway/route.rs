//! 网关的路由：Agent 绑定 → 模型名 → 候选上游（供应商 × Key）列表。
//!
//! 只看网关自己的供应商池和模型组（`gateway.json`），与直连模式的供应商无关。模型名有三种写法：
//! - 裸模型名：池里所有提供它的供应商按池顺序依次尝试；
//! - `<供应商 id>/<模型>`：只发给这个供应商；
//! - `group/<组 id>`：按模型组的成员与模式展开，成员也可以是另一个组，或带 `*` 的通配符。
//!
//! 模型组还会记住每个会话上次是哪个成员回答的（见 [`GroupSticky`]），下次先试它，保住上游的提示词缓存。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};

use crate::gateway::key_pool::{self, KeyLimits, KeyRef};
use crate::gateway::store::{
    AgentBinding, AutoGroupSettings, GROUP_MODEL_PREFIX, GatewayGroup, GatewayProvider,
    GatewaySettings, GroupMember, GroupMode, GroupSticky, UnknownModelPolicy,
};
use crate::settings::RelayProfile;
use crate::tools::ToolId;

/// 嵌套模型组最多展开几层，防止组互相引用时死循环。
const MAX_GROUP_DEPTH: usize = 4;
/// 「缓存有效期内固定」：上次回答后这么久内，同一会话继续先试那个成员。上游的提示词缓存一般保留几分钟到一小时。
pub const CACHE_STICKY_TTL: Duration = Duration::from_secs(10 * 60);
/// 「整个会话固定」也不会永远记着：这么久没用的会话忘掉。
const SESSION_STICKY_TTL: Duration = Duration::from_secs(24 * 60 * 60);
/// 依次 / 额度优先的组里，前面的成员答不了、换到别的成员回答时只临时固定这么久
/// （从第一次换过去算起，期间再成功也不续期），之后先回去试排在前面的成员（#2767）。
pub const FALLBACK_STICKY_TTL: Duration = Duration::from_secs(10 * 60);
/// 记住的会话数上限：到了上限先按各组的有效期清掉过期的，还多就淘汰最久没用的。
const STICKY_LIMIT: usize = 2048;
/// 淘汰时一次清到上限的这个比例，免得每个请求都整表扫一遍。
const STICKY_TRIM_TO: usize = STICKY_LIMIT * 3 / 4;

/// 一个可尝试的上游：供应商（`api_key` 已换成该 Key）+ 发给它的模型名。
#[derive(Debug, Clone)]
pub struct GatewayCandidate {
    pub relay: RelayProfile,
    pub key: KeyRef,
    pub upstream_model: String,
    /// 这家这个模型的上下文窗口（供应商池里写了才有）：请求比它还长时排到最后再试。
    pub window: Option<u64>,
    /// 这家每个 Key 的主动限速上限（没填就是不限）；候选循环发请求前按它占名额。
    pub limits: KeyLimits,
}

#[derive(Debug, Clone)]
pub struct RoutePlan {
    pub agent: ToolId,
    /// 客户端请求的模型名。
    pub source_model: String,
    /// 按绑定改写后的模型名。
    pub model: String,
    pub group_id: Option<String>,
    pub conversation: Option<String>,
    /// 模型组「固定」的有效期；不是模型组或组设成不固定时为 None（不记会话）。
    pub sticky_ttl: Option<Duration>,
    /// 「依次」组本来该先试的上游 (供应商 id, 模型)，不管它是否在冷却；别的成员回答时只算临时顶替。
    /// 其余模式为 None。
    pub preferred: Option<(String, String)>,
    /// 「额度优先」组：顺序每次按实时额度重排，不能拿排第一的当首选（额度接近时会来回判成顶替）；
    /// 只有这次请求里确实换了成员（或延续还没到期的顶替），才算临时顶替。
    pub hold_on_switch: bool,
    pub candidates: Vec<GatewayCandidate>,
    /// 候选为空且是因为 Key 都在冷却/熔断时，最近一个恢复的等待时间。
    pub cooling: Option<Duration>,
    /// 候选为空时，冷却中的 Key（按恢复先后排）：最早的那个很快恢复时，候选循环等一等再试它，
    /// 不马上回本地 429（#2802）。有可用候选时为空。
    pub waiting: Vec<WaitingCandidate>,
}

/// 冷却中的一个候选与它还要冷却多久。
#[derive(Debug, Clone)]
pub struct WaitingCandidate {
    pub candidate: GatewayCandidate,
    pub remaining: Duration,
}

/// 供应商公布的模型（默认模型 + 模型列表），去掉窗口后缀。
pub fn relay_models(relay: &RelayProfile) -> Vec<String> {
    let mut models = Vec::new();
    let default = crate::relay_config::relay_profile_model(relay);
    if !default.trim().is_empty() {
        models.push(default.trim().to_string());
    }
    for (slug, _) in crate::tools::grok::parse_grok_model_list(&relay.model_list) {
        if !models.contains(&slug) {
            models.push(slug);
        }
    }
    models
}

/// 能聊天的模型：去掉生图模型（见 [`crate::gateway::images::is_image_model`]）。
pub fn chat_models(relay: &RelayProfile) -> Vec<String> {
    relay_models(relay)
        .into_iter()
        .filter(|model| !crate::gateway::images::is_image_model(model))
        .collect()
}

fn serves(relay: &RelayProfile, model: &str) -> bool {
    relay_models(relay)
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(model))
}

/// 某个 Agent 能用的池内供应商：打开的，再去掉它「挑选」里排除的。
pub fn binding_pool_relays<'a>(
    gateway: &'a GatewaySettings,
    binding: &AgentBinding,
) -> Vec<&'a RelayProfile> {
    gateway
        .providers
        .iter()
        .filter(|provider| provider.enabled && !binding.pool_exclude.contains(&provider.relay.id))
        .map(|provider| &provider.relay)
        .collect()
}

/// 自动组的 id 前缀（`auto:<小写模型名>`），只在内部用（会话固定、轮换进度、界面）；Agent 选它用的是裸模型名。
pub const AUTO_GROUP_PREFIX: &str = "auto:";

/// 池里打开的供应商和它们各自的模型（只解析一次模型列表）。
fn enabled_relay_models(gateway: &GatewaySettings) -> Vec<(&RelayProfile, Vec<String>)> {
    gateway
        .providers
        .iter()
        .filter(|provider| provider.enabled)
        .map(|provider| (&provider.relay, relay_models(&provider.relay)))
        .collect()
}

/// 用已解析的模型列表拼出一个模型的自动组；只有一家或没人提供时返回 None。
fn build_auto_group(
    gateway: &GatewaySettings,
    lists: &[(&RelayProfile, Vec<String>)],
    model: &str,
) -> Option<GatewayGroup> {
    let members: Vec<GroupMember> = lists
        .iter()
        .filter(|(_, models)| models.iter().any(|known| known.eq_ignore_ascii_case(model)))
        .map(|(relay, _)| GroupMember {
            model: format!("{}/{model}", relay.id),
            weight: 1,
        })
        .collect();
    if members.len() < 2 {
        return None;
    }
    let key = model.to_ascii_lowercase();
    let settings = gateway.auto_groups.get(&key).cloned().unwrap_or_default();
    Some(GatewayGroup {
        id: format!("{AUTO_GROUP_PREFIX}{key}"),
        name: model.to_string(),
        mode: settings.mode,
        sticky: settings.sticky,
        members,
        hide_members: settings.hide_members,
        pick: settings.pick,
        extra: Default::default(),
    })
}

/// 池里打开的供应商里多家都有的模型，各自一个自动组（含已隐藏的），按模型第一次出现的先后。
/// 成员是每家固定的那份 `<供应商 id>/<模型>`，按池顺序；模式、会话固定等取用户改过的设置。
pub fn auto_groups(gateway: &GatewaySettings) -> Vec<GatewayGroup> {
    let lists = enabled_relay_models(gateway);
    let mut seen: Vec<String> = Vec::new();
    let mut groups = Vec::new();
    for (_, models) in &lists {
        for model in models {
            let key = model.to_ascii_lowercase();
            if seen.contains(&key) {
                continue;
            }
            seen.push(key);
            if let Some(group) = build_auto_group(gateway, &lists, model) {
                groups.push(group);
            }
        }
    }
    groups
}

/// 自动组上改过的设置（没改过返回默认）。
pub fn auto_group_settings(gateway: &GatewaySettings, model: &str) -> AutoGroupSettings {
    gateway
        .auto_groups
        .get(&model.to_ascii_lowercase())
        .cloned()
        .unwrap_or_default()
}

/// 这个裸模型名现在走不走自动组：自动建组开着、这个自动组没被隐藏、池里确实多家都有。
pub fn active_auto_group(gateway: &GatewaySettings, model: &str) -> Option<GatewayGroup> {
    if !gateway.auto_models || model.starts_with(GROUP_MODEL_PREFIX) || is_pattern(model) {
        return None;
    }
    if auto_group_settings(gateway, model).hidden {
        return None;
    }
    build_auto_group(gateway, &enabled_relay_models(gateway), model)
}

/// 隐藏了成员的组（自建的、生效中的自动组）里，不再单独出现在 Agent 模型列表里的那些成员。
/// 只算具体的模型（`<供应商 id>/<模型>` 或裸模型名），通配符和嵌套组不算。
fn unlisted_members(gateway: &GatewaySettings) -> Vec<String> {
    let mut hidden: Vec<String> = Vec::new();
    let concrete = |member: &GroupMember| {
        let model = member.model.trim();
        (!model.is_empty() && !is_pattern(model) && !model.starts_with(GROUP_MODEL_PREFIX))
            .then(|| model.to_ascii_lowercase())
    };
    for group in gateway.groups.iter().filter(|group| group.hide_members) {
        hidden.extend(group.members.iter().filter_map(concrete));
    }
    if gateway.auto_models {
        for group in auto_groups(gateway) {
            if group.hide_members && !auto_group_settings(gateway, &group.name).hidden {
                hidden.extend(group.members.iter().filter_map(concrete));
            }
        }
    }
    hidden
}

/// Agent 能用的全部模型：池内供应商的模型（去重，按池顺序），再加上有成员的路由组 `group/<id>`。
pub fn pool_models(gateway: &GatewaySettings, binding: &AgentBinding) -> Vec<String> {
    let mut models: Vec<String> = Vec::new();
    let unlisted = unlisted_members(gateway);
    for relay in binding_pool_relays(gateway, binding) {
        // 生图模型不能聊天，不进 Agent 的模型列表（在「网关设置」的生图模型里选）。
        for model in chat_models(relay) {
            if !unlisted.contains(&model.to_ascii_lowercase())
                && !models
                    .iter()
                    .any(|known| known.eq_ignore_ascii_case(&model))
            {
                models.push(model);
            }
        }
    }
    for group in &gateway.groups {
        if !group.members.is_empty() {
            models.push(group.model_id());
        }
    }
    models
}

/// Codex 的模型列表：每家供应商的模型都列在它自己名下，Codex 选择器里按供应商分组时每组都完整。
/// - 只有一家提供的模型：裸模型名（只有这一家，等于固定发给它）；
/// - 多家都提供的模型：每家各列一份 `<供应商 id>/<模型>`（固定发给那一家）；打开「多家自动切换」
///   （`auto_models`）时再单独列一份裸模型名：按池顺序依次尝试各家，出错换下一家；
/// - 最后是有成员的模型组。
pub fn codex_pool_models(gateway: &GatewaySettings, binding: &AgentBinding) -> Vec<String> {
    codex_pool_blocks(gateway, binding).concat()
}

/// 同 [`codex_pool_models`]，但把默认模型所在的那一块（某一家 / 多家自动切换 / 模型组）挪到最前面。
/// Codex 按列表顺序排模型，默认模型总是排第一（见 `merge_model_into_model_list`）；选择器的分组
/// 注入只插标题、不搬行，同一组的行必须挨在一起，否则默认模型那组会被拆成两截。
pub fn codex_pool_models_led_by(
    gateway: &GatewaySettings,
    binding: &AgentBinding,
    default_model: &str,
) -> Vec<String> {
    let mut blocks = codex_pool_blocks(gateway, binding);
    if let Some(index) = blocks.iter().position(|block| {
        block
            .iter()
            .any(|model| model.eq_ignore_ascii_case(default_model.trim()))
    }) {
        let block = blocks.remove(index);
        blocks.insert(0, block);
    }
    blocks.concat()
}

/// Codex 模型列表按分组切成的几块：每家一块、多家自动切换一块、模型组一块（空块不要）。
fn codex_pool_blocks(gateway: &GatewaySettings, binding: &AgentBinding) -> Vec<Vec<String>> {
    let relays = binding_pool_relays(gateway, binding);
    let mut seen: Vec<String> = Vec::new();
    let mut fresh = |entry: &str| {
        if seen.iter().any(|known| known.eq_ignore_ascii_case(entry)) {
            false
        } else {
            seen.push(entry.to_string());
            true
        }
    };
    let mut blocks: Vec<Vec<String>> = Vec::new();
    let mut shared: Vec<String> = Vec::new();
    // 隐藏了成员的组：成员不再单独列出，只能通过组来选。
    let unlisted = unlisted_members(gateway);
    for relay in &relays {
        let mut block = Vec::new();
        for model in chat_models(relay) {
            if serving_count(&relays, &model) > 1 {
                let pinned = format!("{}/{model}", relay.id);
                if !unlisted.contains(&pinned.to_ascii_lowercase()) && fresh(&pinned) {
                    block.push(pinned);
                }
                if !shared
                    .iter()
                    .any(|known| known.eq_ignore_ascii_case(&model))
                {
                    shared.push(model);
                }
            } else if !unlisted.contains(&model.to_ascii_lowercase()) && fresh(&model) {
                block.push(model);
            }
        }
        blocks.push(block);
    }
    // 自动组：每个生效中的自动组列一份裸模型名（隐藏了的、或关了自动建组时不列）。
    blocks.push(
        shared
            .into_iter()
            .filter(|model| gateway.auto_models && !auto_group_settings(gateway, model).hidden)
            .filter(|model| fresh(model))
            .collect(),
    );
    blocks.push(
        gateway
            .groups
            .iter()
            .filter(|group| !group.members.is_empty())
            .map(GatewayGroup::model_id)
            .collect(),
    );
    blocks.retain(|block| !block.is_empty());
    blocks
}

/// Agent 模型列表里一项的上下文窗口：它会发给的各家里写了窗口的取最小值（模型组、多家自动切换
/// 都按最短的那家算，保证 Agent 按这个长度压缩后每家都装得下）。都没写返回 None。
pub fn model_entry_window(
    gateway: &GatewaySettings,
    binding: &AgentBinding,
    model: &str,
) -> Option<u64> {
    expand_model(gateway, binding, model, 0, &mut Vec::new(), false)
        .into_iter()
        .filter_map(|(relay, upstream)| {
            gateway
                .provider(&relay.id)
                .and_then(|provider| provider.window_for(&upstream))
        })
        .min()
}

/// 某家某个模型支持的思考档位（由低到高）：用户在模型资料里写的优先，其次内置模型资料。
/// 没有资料时为空。与 Codex 模型目录、管理器的「思考等级」读同一份资料。
pub fn model_reasoning_levels(relay: &RelayProfile, model: &str) -> Vec<String> {
    let (slug, _) = crate::model_suffix::parse_model_suffix(model.trim());
    let user = crate::relay_config::parse_model_metadata_map(&relay.model_metadata)
        .ok()
        .and_then(|map| map.get(&slug.to_ascii_lowercase()).cloned());
    let Some(ui) = crate::model_suffix::model_ui_metadata_with_override(&slug, user.as_ref()) else {
        return Vec::new();
    };
    let names: Vec<String> = ui["supportedReasoningEfforts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|level| level["reasoningEffort"].as_str().map(str::to_string))
        .collect();
    crate::upstream_model_meta::normalize_levels(&names)
}

/// Agent 模型列表里一项的思考档位：它会发给的各家档位的并集（由低到高）。
/// 发到哪家时再按那家的档位套（[`crate::gateway::anthropic_inbound::fit_reasoning_effort`]）。
pub fn model_entry_reasoning_levels(
    gateway: &GatewaySettings,
    binding: &AgentBinding,
    model: &str,
) -> Vec<String> {
    let all: Vec<String> = expand_model(gateway, binding, model, 0, &mut Vec::new(), false)
        .into_iter()
        .flat_map(|(relay, upstream)| model_reasoning_levels(&relay, &upstream))
        .collect();
    crate::upstream_model_meta::normalize_levels(&all)
}

/// 模型列表里一项实际会发给哪些（供应商, 上游模型）：模型组按成员展开（不推进轮换位置）。
pub fn model_entry_members(
    gateway: &GatewaySettings,
    binding: &AgentBinding,
    model: &str,
) -> Vec<(RelayProfile, String)> {
    expand_model(gateway, binding, model, 0, &mut Vec::new(), false)
}

/// 列表里有几家供应商提供这个模型（按模型名，不分大小写）。
pub fn serving_count(relays: &[&RelayProfile], model: &str) -> usize {
    relays.iter().filter(|relay| serves(relay, model)).count()
}

/// Codex 模型列表里一项归哪家供应商、发给它的模型名：裸模型名归第一个提供它的供应商
/// （多家都有时它是「自动切换」入口，元数据取第一家的），`<供应商 id>/<模型>` 归那一家。
/// 模型组和认不出的返回 None。
pub fn codex_model_owner<'a>(relays: &[&'a RelayProfile], model: &str) -> Option<(usize, String)> {
    if let Some(index) = relays.iter().position(|relay| serves(relay, model)) {
        return Some((index, model.to_string()));
    }
    let (relay, upstream) = qualified_in(relays, model)?;
    let index = relays
        .iter()
        .position(|candidate| candidate.id == relay.id)?;
    serves(relay, &upstream).then_some((index, upstream))
}

/// 绑定的默认模型：绑定里写了就用它，否则用池里第一个供应商的默认模型。
pub fn binding_default_model(gateway: &GatewaySettings, binding: &AgentBinding) -> String {
    let (slug, _) = crate::model_suffix::parse_model_suffix(binding.model.trim());
    if !slug.trim().is_empty() {
        return slug.trim().to_string();
    }
    binding_pool_relays(gateway, binding)
        .into_iter()
        .filter_map(|relay| relay_models(relay).into_iter().next())
        .next()
        .unwrap_or_default()
}

/// 模型名是否能在池里路由：池里的模型、`group/<id>`，或 `<供应商 id>/<任意模型>`。
pub fn model_is_routable(gateway: &GatewaySettings, binding: &AgentBinding, model: &str) -> bool {
    pool_models(gateway, binding)
        .iter()
        .any(|known| known.eq_ignore_ascii_case(model))
        || qualified_provider(gateway, binding, model).is_some()
}

/// 暴露给 Agent 的模型：默认模型在前，然后是池里所有能用的模型和路由组，以及绑定里声明的模型。
pub fn binding_exposed_models(gateway: &GatewaySettings, binding: &AgentBinding) -> Vec<String> {
    let mut models = Vec::new();
    let default = binding_default_model(gateway, binding);
    if !default.is_empty() {
        models.push(default);
    }
    for model in pool_models(gateway, binding)
        .into_iter()
        .chain(binding.declared_models())
    {
        if !models.contains(&model) {
            models.push(model);
        }
    }
    models
}

/// 非 Claude 模型给 Claude Desktop 用的别名前缀。
///
/// Claude Desktop 的第三方网关模式在模型列表和发起会话时都会检查模型 id：必须含 claude / opus 等字样，
/// 且不能含 gpt、grok、codex、gemini 等一长串黑名单字样，带 `claude-` 前缀也不行。
/// 所以别名用「前缀 + 纯数字」，数字拼不出黑名单里的词；真实名字放在 `display_name` 里给选择器显示。
pub const CLAUDE_ALIAS_PREFIX: &str = "claude-gw-";

/// 有思考档位的非 Claude 模型给 Claude Desktop 用的别名前缀。
///
/// Desktop 的思考等级菜单只给它认识的模型出：网关模式下不读 `/v1/models` 里的档位，
/// 档位表写死在 app.asar 里，其中 `/^(?:claude-)?(?:fable|mythos)(?:-|$)/` 匹配到的 id
/// 给 low…max 五档（同 magpie 的 `mythos-magpie-<n>`）。`mythos` 也在它的 tier 白名单里，
/// 列表过滤同样放行；句中不带 haiku / sonnet / opus，小模型选择不受影响。
/// 数字部分与 [`CLAUDE_ALIAS_PREFIX`] 的一样，两种别名都能还原，旧会话不断。
pub const CLAUDE_EFFORT_ALIAS_PREFIX: &str = "mythos-gw-";

fn alias_number(model: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(model.as_bytes());
    let mut head = [0_u8; 8];
    head.copy_from_slice(&digest[..8]);
    format!("{:010}", u64::from_be_bytes(head) % 10_000_000_000)
}

/// Claude Desktop 选择器里这个模型用的 id：名字里带 claude 的原样，其余换成稳定的数字别名
/// （同一个模型名永远得到同一个别名，网关重启、池子变化都不影响已选的会话）。
pub fn claude_alias(model: &str) -> String {
    if model.to_ascii_lowercase().contains("claude") {
        return model.to_string();
    }
    format!("{CLAUDE_ALIAS_PREFIX}{}", alias_number(model))
}

/// 同 [`claude_alias`]，但有思考档位的模型用 [`CLAUDE_EFFORT_ALIAS_PREFIX`]，让 Desktop 给它出思考等级菜单。
pub fn claude_desktop_alias(model: &str, has_effort: bool) -> String {
    if model.to_ascii_lowercase().contains("claude") || !has_effort {
        return claude_alias(model);
    }
    format!("{CLAUDE_EFFORT_ALIAS_PREFIX}{}", alias_number(model))
}

/// 把 Claude Desktop 发来的别名还原成真实模型名；不是别名、或别名对不上这个绑定的任何模型时 None。
fn claude_alias_target(
    gateway: &GatewaySettings,
    binding: &AgentBinding,
    requested: &str,
) -> Option<String> {
    let digits = [CLAUDE_ALIAS_PREFIX, CLAUDE_EFFORT_ALIAS_PREFIX]
        .iter()
        .find_map(|prefix| {
            requested
                .get(..prefix.len())
                .filter(|head| head.eq_ignore_ascii_case(prefix))
                .map(|_| &requested[prefix.len()..])
        })?;
    binding_exposed_models(gateway, binding)
        .into_iter()
        .find(|model| alias_number(model) == digits)
}

/// Codex 起名请求的提示词开头：桌面版首次起名、桌面版定期重新起名、终端版起名。
/// 这几段由 Codex 固定拼好，作为临时线程里唯一的用户消息发出。
const CODEX_TITLE_PROMPT_PREFIXES: &[&str] = &[
    "You are a helpful assistant. You will be presented with a user prompt, and your job is to provide a short title for a task",
    "Keep the current chat title ",
    "Generate a concise, single-line task title",
];

/// 是否是 Codex 自己发起的起名请求：最后一条用户消息以起名提示词开头。
///
/// 起名请求没有专门的模型名（首次起名用内置轻量模型，重新起名用会话当前模型），
/// 只能按提示词认；只看最后一条用户消息的开头，用户正常对话里提到这些字不会误判。
pub fn is_codex_title_request(request: &serde_json::Value) -> bool {
    let Some(items) = request.get("input").and_then(serde_json::Value::as_array) else {
        return false;
    };
    let Some(message) = items.iter().rev().find(|item| {
        item.get("role").and_then(serde_json::Value::as_str) == Some("user")
            && item
                .get("type")
                .and_then(serde_json::Value::as_str)
                .is_none_or(|kind| kind == "message")
    }) else {
        return false;
    };
    let text = match message.get("content") {
        Some(serde_json::Value::String(text)) => text.clone(),
        Some(serde_json::Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| part.get("text").and_then(serde_json::Value::as_str))
            .collect::<Vec<_>>()
            .join(""),
        _ => return false,
    };
    let text = text.trim_start();
    CODEX_TITLE_PROMPT_PREFIXES
        .iter()
        .any(|prefix| text.starts_with(prefix))
}

/// 起名模型：绑定指定了起名模型、且这是 Codex 的起名请求时，返回要改发的模型；
/// 否则 None，请求照原来的模型路由。
pub fn title_model_override(
    gateway: &GatewaySettings,
    agent: &ToolId,
    request: &serde_json::Value,
) -> Option<String> {
    // 拓展用 gateway.replaces 关掉了起名模型改发：起名请求照原来的模型路由。
    if crate::extension_gateway::builtin_replaced("builtin:title-model") {
        return None;
    }
    let binding = gateway.active_binding(agent)?;
    let (title, _) = crate::model_suffix::parse_model_suffix(binding.title_model.trim());
    let title = title.trim();
    (!title.is_empty() && is_codex_title_request(request)).then(|| title.to_string())
}

/// 把客户端请求的模型名按绑定改写：认识的（绑定声明过的、池里能路由的）原样保留，
/// 不认识的按「未知模型」策略改成默认模型或原样转发。
pub fn effective_model(
    gateway: &GatewaySettings,
    binding: &AgentBinding,
    default_model: &str,
    requested: &str,
) -> String {
    effective_model_detailed(gateway, binding, default_model, requested).0
}

/// 同 [`effective_model`]，另外返回是否按「未知模型 → 默认模型」策略改写过（#2874）。
/// 这种改写会把请求整个换到默认模型所在的供应商，用户在 Codex 里看不出来，要记日志。
fn effective_model_detailed(
    gateway: &GatewaySettings,
    binding: &AgentBinding,
    default_model: &str,
    requested: &str,
) -> (String, bool) {
    let requested = requested.trim();
    if requested.is_empty() {
        return (default_model.to_string(), false);
    }
    // Claude Desktop 选的非 Claude 模型带的是数字别名，先还原成真实名字再走下面的规则。
    let aliased = claude_alias_target(gateway, binding, requested);
    let requested = aliased.as_deref().unwrap_or(requested);
    // 审批模型（#2703）：codex-auto-review 改发到绑定指定的模型，优先于目录匹配与未知模型策略
    // （池里的 ChatGPT 账号可能把 codex-auto-review 列进模型表，那样会直接落到订阅额度上）。
    if requested == crate::aggregate_catalog::CODEX_AUTO_REVIEW_MODEL {
        let (review, _) = crate::model_suffix::parse_model_suffix(binding.auto_review_model.trim());
        let review = review.trim();
        if !review.is_empty() {
            return (review.to_string(), false);
        }
    }
    let (requested_slug, _) = crate::model_suffix::parse_model_suffix(requested);
    let declared = binding.declared_models();
    if declared
        .iter()
        .any(|model| model.eq_ignore_ascii_case(&requested_slug))
        || model_is_routable(gateway, binding, &requested_slug)
    {
        return (requested_slug, false);
    }
    match binding.unknown_model {
        UnknownModelPolicy::MapToDefault if !default_model.is_empty() => {
            let mapped = !default_model.eq_ignore_ascii_case(&requested_slug);
            (default_model.to_string(), mapped)
        }
        _ => (requested_slug, false),
    }
}

/// 同一个 Agent 的同一个模型名，改写日志 60 秒最多记一次：未知模型的每个请求都会走到这里。
fn should_log_model_rewrite(agent: &str, requested: &str) -> bool {
    static LAST: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
    let key = format!("{agent}\u{0}{requested}");
    let now = Instant::now();
    let Ok(mut last) = LAST.get_or_init(|| Mutex::new(HashMap::new())).lock() else {
        return false;
    };
    if last
        .get(&key)
        .is_some_and(|at| now.duration_since(*at) < Duration::from_secs(60))
    {
        return false;
    }
    last.retain(|_, at| now.duration_since(*at) < Duration::from_secs(600));
    last.insert(key, now);
    true
}

/// `<供应商 id>/<模型>` → (供应商, 模型)。供应商必须是这个 Agent 能用的。
///
/// 模型名本身可能带 `/`（如 OpenRouter 的 `openai/gpt-5`）：池里有供应商把整串当模型名声明时，
/// 按裸模型处理，不拆前缀；没人声明整串时才当成「供应商 id/模型」。
fn qualified_provider<'a>(
    gateway: &'a GatewaySettings,
    binding: &AgentBinding,
    model: &str,
) -> Option<(&'a RelayProfile, String)> {
    qualified_in(&binding_pool_relays(gateway, binding), model)
}

pub(super) fn has_explicit_model_identity(
    gateway: &GatewaySettings,
    binding: &AgentBinding,
    model: &str,
) -> bool {
    let (model, _) = crate::model_suffix::parse_model_suffix(model.trim());
    if group_by_model(gateway, &model).is_some() {
        return true;
    }
    let relays = binding_pool_relays(gateway, binding);
    if declared_by_any(&relays, &model) {
        return false;
    }
    if model
        .strip_prefix(GROUP_MODEL_PREFIX)
        .is_some_and(|id| !id.is_empty())
    {
        return true;
    }
    let Some((provider, name)) = model.split_once('/') else {
        return false;
    };
    !name.trim().is_empty()
        && gateway
            .providers
            .iter()
            .any(|entry| entry.relay.id.eq_ignore_ascii_case(provider))
}

pub(super) fn model_identity_kind(
    gateway: &GatewaySettings,
    binding: &AgentBinding,
    model: &str,
) -> &'static str {
    if group_by_model(gateway, model).is_some() {
        "group"
    } else if qualified_provider(gateway, binding, model).is_some() {
        "provider"
    } else {
        "bare"
    }
}

/// [`qualified_provider`] 的规则，作用在给定的供应商列表上。
fn qualified_in<'a>(
    relays: &[&'a RelayProfile],
    model: &str,
) -> Option<(&'a RelayProfile, String)> {
    if model.starts_with(GROUP_MODEL_PREFIX) {
        return None;
    }
    let (prefix, rest) = model.split_once('/')?;
    if rest.trim().is_empty() || declared_by_any(relays, model) {
        return None;
    }
    relays
        .iter()
        .copied()
        .find(|relay| relay.id.eq_ignore_ascii_case(prefix))
        .map(|relay| (relay, rest.trim().to_string()))
}

/// 列表里有没有供应商把这个完整名字当模型名声明。
fn declared_by_any(relays: &[&RelayProfile], model: &str) -> bool {
    relays.iter().any(|relay| serves(relay, model))
}

/// 删除供应商 `id` 后，模型组成员 `member` 是不是还指向别的东西（不该跟着删）：
/// 只有以 `<id>/` 开头的成员才可能指向它；其中剩下的供应商把整串声明成模型名的、
/// 或是通配符且还能匹配到剩下的模型的，保留。`remaining` 是删掉之后的池（不看开关）。
pub fn member_outlives_provider(remaining: &[GatewayProvider], id: &str, member: &str) -> bool {
    let member = member.trim();
    let Some(rest) = member
        .get(..id.len())
        .filter(|prefix| prefix.eq_ignore_ascii_case(id))
        .and_then(|_| member[id.len()..].strip_prefix('/'))
    else {
        return true;
    };
    if rest.trim().is_empty() {
        return false;
    }
    let relays: Vec<&RelayProfile> = remaining.iter().map(|provider| &provider.relay).collect();
    if is_pattern(member) {
        return relays.iter().any(|relay| {
            relay_models(relay)
                .iter()
                .any(|model| pattern_hits(member, relay, model, &relays))
        });
    }
    declared_by_any(&relays, member)
}

fn group_by_model<'a>(gateway: &'a GatewaySettings, model: &str) -> Option<&'a GatewayGroup> {
    let id = model.strip_prefix(GROUP_MODEL_PREFIX)?;
    gateway
        .groups
        .iter()
        .find(|group| group.id.eq_ignore_ascii_case(id))
}

/// 这个模型名走哪个组：`group/<id>` 是自建的组；多家都有的裸模型名走它的自动组（生效时）。
fn group_for(gateway: &GatewaySettings, model: &str) -> Option<GatewayGroup> {
    group_by_model(gateway, model)
        .cloned()
        .or_else(|| active_auto_group(gateway, model))
}

/// 一个模型名展开成 (供应商, 发给它的模型) 列表。
fn expand_model(
    gateway: &GatewaySettings,
    binding: &AgentBinding,
    model: &str,
    depth: usize,
    visiting: &mut Vec<String>,
    advance: bool,
) -> Vec<(RelayProfile, String)> {
    if let Some(group) = group_for(gateway, model) {
        return expand_group(gateway, binding, &group, depth, visiting, advance);
    }
    if is_pattern(model) {
        return expand_pattern(gateway, binding, model);
    }
    if let Some((relay, upstream)) = qualified_provider(gateway, binding, model) {
        return vec![(relay.clone(), upstream)];
    }
    let pool = binding_pool_relays(gateway, binding);
    let serving: Vec<(RelayProfile, String)> = pool
        .iter()
        .filter(|relay| serves(relay, model))
        .map(|relay| ((*relay).clone(), model.to_string()))
        .collect();
    if !serving.is_empty() {
        return serving;
    }
    // `<供应商 id>/<模型>` 指向的供应商已经不在池里（删了 / 关了 / 被挑选排除），又没人把整串当
    // 模型名声明：前缀只是 Codex++ 的供应商 id，原样发给别家上游必然报「无权访问模型」（#2897）。
    // 池里有别家提供去掉前缀的同名模型时，改由它们回答。只对 Agent 直接请求的模型（depth 0）这样做：
    // 路由组成员钉在某家上，那家关了就该跳过，不能换成别家。
    if depth == 0
        && let Some((_, rest)) = model.split_once('/')
        && !model.starts_with(GROUP_MODEL_PREFIX)
        && !rest.trim().is_empty()
    {
        let rest = rest.trim();
        return pool
            .into_iter()
            .filter(|relay| serves(relay, rest))
            .map(|relay| (relay.clone(), rest.to_string()))
            .collect();
    }
    Vec::new()
}

/// 成员是不是通配符。
pub fn is_pattern(model: &str) -> bool {
    model.contains('*')
}

/// 通配符匹配：`*` 匹配任意一段字符（包括 `/`），不区分大小写。
pub fn glob_matches(pattern: &str, text: &str) -> bool {
    let pattern = pattern.to_ascii_lowercase();
    let text = text.to_ascii_lowercase();
    let parts: Vec<&str> = pattern.split('*').collect();
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if parts.len() == 1 {
        return text == first;
    }
    if !text.starts_with(first) || text.len() < first.len() + last.len() || !text.ends_with(last) {
        return false;
    }
    let mut rest = &text[first.len()..text.len() - last.len()];
    for part in &parts[1..parts.len() - 1] {
        match rest.find(part) {
            Some(index) => rest = &rest[index + part.len()..],
            None => return false,
        }
    }
    true
}

/// 通配符成员展开成池里匹配的 (供应商, 模型)：`<供应商 id>/<模型>` 或裸模型名能匹配上就算，
/// 按池顺序、各供应商自己的模型顺序。
fn expand_pattern(
    gateway: &GatewaySettings,
    binding: &AgentBinding,
    pattern: &str,
) -> Vec<(RelayProfile, String)> {
    let pool = binding_pool_relays(gateway, binding);
    let mut relays = Vec::new();
    for relay in &pool {
        for model in relay_models(relay) {
            if pattern_hits(pattern, relay, &model, &pool) {
                relays.push(((*relay).clone(), model));
            }
        }
    }
    relays
}

/// 通配符是否选中某个供应商的某个模型，口径和写死的成员一致：裸模型名匹配就算；
/// `<供应商 id>/<模型>` 匹配也算，但这串如果被池里别的供应商声明成了完整模型名
/// （写死时会按裸模型走那一家），就不再算这个供应商的。
fn pattern_hits(pattern: &str, relay: &RelayProfile, model: &str, pool: &[&RelayProfile]) -> bool {
    if glob_matches(pattern, model) {
        return true;
    }
    let qualified = format!("{}/{model}", relay.id);
    glob_matches(pattern, &qualified) && !declared_by_any(pool, &qualified)
}

/// 通配符现在匹配到的 `<供应商 id>/<模型>`（所有打开的供应商，不看 Agent 的挑选），界面显示用。
pub fn pattern_matches(gateway: &GatewaySettings, pattern: &str) -> Vec<String> {
    expand_pattern(gateway, &AgentBinding::default(), pattern)
        .into_iter()
        .map(|(relay, model)| format!("{}/{model}", relay.id))
        .collect()
}

/// 模型组展开：按模式决定从哪个成员开始，成员依次展开（嵌套组递归，出现环或太深时跳过）；
/// 「额度优先」「速度优先」展开后再按额度 / 速度给所有上游排先后。
fn expand_group(
    gateway: &GatewaySettings,
    binding: &AgentBinding,
    group: &GatewayGroup,
    depth: usize,
    visiting: &mut Vec<String>,
    advance: bool,
) -> Vec<(RelayProfile, String)> {
    if depth >= MAX_GROUP_DEPTH || visiting.contains(&group.id) {
        return Vec::new();
    }
    visiting.push(group.id.clone());
    let mut relays: Vec<(RelayProfile, String)> = Vec::new();
    for member in ordered_members(group, advance) {
        for (relay, model) in expand_model(
            gateway,
            binding,
            member.model.trim(),
            depth + 1,
            visiting,
            advance,
        ) {
            if !relays
                .iter()
                .any(|(known, known_model)| known.id == relay.id && *known_model == model)
            {
                relays.push((relay, model));
            }
        }
    }
    visiting.pop();
    match group.mode {
        GroupMode::Usage => sort_by_quota(&mut relays),
        GroupMode::Fast => sort_by_speed(&mut relays),
        _ => {}
    }
    relays
}

/// 额度优先：有余量的按剩余比例从多到少，其次是没有额度数据的（保持原顺序），用完的排最后。
fn sort_by_quota(relays: &mut [(RelayProfile, String)]) {
    relays.sort_by(|(left, _), (right, _)| {
        let rank = |relay: &RelayProfile| match crate::gateway::quota::remaining(&relay.id) {
            Some(left) if left > 0.0 => (0, -left),
            None => (1, 0.0),
            Some(_) => (2, 0.0),
        };
        let (left, right) = (rank(left), rank(right));
        left.0.cmp(&right.0).then(left.1.total_cmp(&right.1))
    });
}

/// 速度优先：还没试过的先试一次，其余按平均首包耗时从快到慢；失败过的按连续失败次数加罚时，
/// 排到能用的成员后面（试过一次失败了就不会再排第一）。
fn sort_by_speed(relays: &mut [(RelayProfile, String)]) {
    relays.sort_by_key(|(relay, model)| {
        crate::gateway::speed::rank(&relay.id, model)
            .map_or((0, Duration::ZERO), |latency| (1, latency))
    });
}

/// (组 id, 会话) → (供应商 id, 模型, 记下的时间, 这条的有效期, 是否临时顶替)。
type StickyTable = HashMap<(String, String), (String, String, Instant, Duration, bool)>;

fn sticky_table() -> &'static Mutex<StickyTable> {
    static TABLE: OnceLock<Mutex<StickyTable>> = OnceLock::new();
    TABLE.get_or_init(Mutex::default)
}

/// 记下这个会话在组里是哪个上游回答的。没有组、组不固定或没有会话标识时不记。
/// 按优先级排序的组里由后面的成员顶替回答时，只临时固定 [`FALLBACK_STICKY_TTL`]，
/// 而且从第一次顶替算起，不随后续成功续期。
pub fn remember_answer(plan: &RoutePlan, candidate: &GatewayCandidate) {
    let (Some(group), Some(conversation), Some(ttl)) =
        (&plan.group_id, &plan.conversation, plan.sticky_ttl)
    else {
        return;
    };
    let now = Instant::now();
    let mut table = sticky_table()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let key = (group.clone(), conversation.clone());
    if table.len() >= STICKY_LIMIT && !table.contains_key(&key) {
        trim_sticky(&mut table, now);
    }
    let same = |relay: &str, model: &str| {
        relay == candidate.relay.id && model == candidate.upstream_model
    };
    let fallback = match &plan.preferred {
        Some((relay, model)) => !same(relay, model),
        None if plan.hold_on_switch => {
            let switched = plan
                .candidates
                .first()
                .is_some_and(|first| !same(&first.relay.id, &first.upstream_model));
            // 顶替期内同一个成员接着回答：仍按顶替算，到期才重新按额度排。
            let holding = table.get(&key).is_some_and(|(relay, model, at, ttl, was_fallback)| {
                *was_fallback && same(relay, model) && now.duration_since(*at) < *ttl
            });
            switched || holding
        }
        None => false,
    };
    let (ttl, at) = if fallback {
        // 同一个顶替成员还在临时固定期内：保留第一次顶替的时间，到期就回去试前面的成员。
        let since = table
            .get(&key)
            .filter(|(relay, model, at, ttl, was_fallback)| {
                *was_fallback
                    && *relay == candidate.relay.id
                    && *model == candidate.upstream_model
                    && now.duration_since(*at) < *ttl
            })
            .map_or(now, |(_, _, at, _, _)| *at);
        (ttl.min(FALLBACK_STICKY_TTL), since)
    } else {
        (ttl, now)
    };
    table.insert(
        key,
        (
            candidate.relay.id.clone(),
            candidate.upstream_model.clone(),
            at,
            ttl,
            fallback,
        ),
    );
}

/// 表满了：先按各条自己的有效期清掉过期的；还多就从最久没用的开始淘汰，清到上限的 3/4，
/// 之后一段时间的请求不用再扫表。
fn trim_sticky(table: &mut StickyTable, now: Instant) {
    table.retain(|_, (_, _, at, ttl, _)| now.duration_since(*at) < *ttl);
    if table.len() < STICKY_TRIM_TO {
        return;
    }
    let mut ages: Vec<Instant> = table.values().map(|(_, _, at, _, _)| *at).collect();
    ages.sort_unstable();
    // 比这一刻早（或同时）记下的都淘汰，剩下的不超过 STICKY_TRIM_TO 条。
    let cutoff = ages[table.len() - STICKY_TRIM_TO];
    table.retain(|_, (_, _, at, _, _)| *at > cutoff);
}

/// 这个会话在组里该先试的上游 (供应商 id, 模型)：按组的固定方式，过期或不固定时返回 None。
fn recall_answer(
    group: &GatewayGroup,
    conversation: &str,
    now: Instant,
) -> Option<(String, String)> {
    let ttl = sticky_ttl(group.sticky)?;
    let table = sticky_table()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let (relay, model, at, stored_ttl, _) =
        table.get(&(group.id.clone(), conversation.to_string()))?;
    // 临时顶替记的有效期比组的短，按两者中短的算。
    (now.duration_since(*at) < ttl.min(*stored_ttl)).then(|| (relay.clone(), model.clone()))
}

/// 组的固定方式对应的有效期；不固定返回 None。
fn sticky_ttl(sticky: GroupSticky) -> Option<Duration> {
    match sticky {
        GroupSticky::Off => None,
        GroupSticky::Cache => Some(CACHE_STICKY_TTL),
        GroupSticky::Session => Some(SESSION_STICKY_TTL),
    }
}

/// 把会话上次回答的上游挪到最前面；它已经不在候选里（被移出组、关掉）就不动。
fn prefer_remembered(relays: &mut Vec<(RelayProfile, String)>, remembered: &(String, String)) {
    if let Some(index) = relays
        .iter()
        .position(|(relay, model)| relay.id == remembered.0 && *model == remembered.1)
    {
        let item = relays.remove(index);
        relays.insert(0, item);
    }
}

/// 按模型组模式排出成员的尝试顺序。轮换 / 权重的进度存在内存里，各组独立；
/// `advance` 为 false 时只看当前进度、不往前推（count_tokens 这类预览用）。
fn ordered_members(group: &GatewayGroup, advance: bool) -> Vec<GroupMember> {
    let members = group.members.clone();
    if group.mode == GroupMode::Manual {
        // 手动：只用选中的那个成员（没选或选的已不在组里时用第一个），答不了也不换。
        let picked = members
            .iter()
            .find(|member| member.model.eq_ignore_ascii_case(group.pick.trim()))
            .or_else(|| members.first())
            .cloned();
        return picked.into_iter().collect();
    }
    if members.len() < 2 {
        return members;
    }
    let start = match group.mode {
        GroupMode::InOrder | GroupMode::Usage | GroupMode::Fast | GroupMode::Manual => 0,
        GroupMode::Rotate => (group_counter(&group.id, advance) % members.len() as u64) as usize,
        GroupMode::Weighted => {
            let total: u64 = members
                .iter()
                .map(|member| u64::from(member.weight.max(1)))
                .sum();
            let mut slot = group_counter(&group.id, advance) % total.max(1);
            let mut index = 0;
            for (position, member) in members.iter().enumerate() {
                let weight = u64::from(member.weight.max(1));
                if slot < weight {
                    index = position;
                    break;
                }
                slot -= weight;
            }
            index
        }
    };
    let mut ordered = members[start..].to_vec();
    ordered.extend_from_slice(&members[..start]);
    ordered
}

/// 组的轮换进度：返回当前值，`advance` 时再加一。
fn group_counter(group_id: &str, advance: bool) -> u64 {
    static COUNTERS: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();
    let mut guard = COUNTERS
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if !advance {
        return guard.get(group_id).copied().unwrap_or_default();
    }
    let counter = guard.entry(group_id.to_string()).or_default();
    let value = *counter;
    *counter = counter.wrapping_add(1);
    value
}

/// 按绑定解析出本次请求的候选上游。会推进模型组的轮换 / 权重进度和多 Key 的轮询进度。
pub fn resolve_plan(
    gateway: &GatewaySettings,
    agent: &ToolId,
    requested_model: &str,
    conversation: Option<&str>,
) -> anyhow::Result<RoutePlan> {
    resolve_plan_with(gateway, agent, requested_model, conversation, true)
}

/// 只预览：和 [`resolve_plan`] 选出同样的第一个候选，但不推进模型组轮换 / 权重和 Key 轮询的进度。
/// 给 count_tokens 这类不真正发请求的地方用，免得它们把下一个真实请求的顺序拨走。
pub fn preview_plan(
    gateway: &GatewaySettings,
    agent: &ToolId,
    requested_model: &str,
    conversation: Option<&str>,
) -> anyhow::Result<RoutePlan> {
    resolve_plan_with(gateway, agent, requested_model, conversation, false)
}

fn resolve_plan_with(
    gateway: &GatewaySettings,
    agent: &ToolId,
    requested_model: &str,
    conversation: Option<&str>,
    advance: bool,
) -> anyhow::Result<RoutePlan> {
    let binding = gateway
        .active_binding(agent)
        .with_context(|| format!("Codex++ 网关模式没有为 {} 开启", agent.as_str()))?;
    if binding_pool_relays(gateway, binding).is_empty() {
        bail!("网关的供应商池是空的或都已关闭：请在网关的「供应商」里添加或打开供应商");
    }
    let default_model = binding_default_model(gateway, binding);
    let (model, mapped_to_default) =
        effective_model_detailed(gateway, binding, &default_model, requested_model);
    let conversation = conversation
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    let group = group_for(gateway, &model);
    let group_id = group.as_ref().map(|group| group.id.clone());
    let sticky_ttl = group.as_ref().and_then(|group| sticky_ttl(group.sticky));
    let mut relays = expand_model(gateway, binding, &model, 0, &mut Vec::new(), advance);
    // 依次：排序本身就是优先级，记下本来该先试的成员（会话固定挪动顺序之前）。
    let preferred = group
        .as_ref()
        .filter(|group| group.mode == GroupMode::InOrder)
        .and_then(|_| relays.first())
        .map(|(relay, model)| (relay.id.clone(), model.clone()));
    let hold_on_switch = group
        .as_ref()
        .is_some_and(|group| group.mode == GroupMode::Usage);
    if let (Some(group), Some(conversation)) = (group.as_ref(), conversation.as_deref())
        && group.mode != GroupMode::Manual
        && let Some(remembered) = recall_answer(group, conversation, Instant::now())
    {
        prefer_remembered(&mut relays, &remembered);
    }
    if relays.is_empty() && group_id.is_none() {
        // 原样转发的未知模型：交给默认模型所在的供应商。
        relays = expand_model(
            gateway,
            binding,
            &default_model,
            0,
            &mut Vec::new(),
            advance,
        )
        .into_iter()
        .map(|(relay, _)| (relay, model.clone()))
        .collect();
    }
    if relays.is_empty() {
        bail!("网关的供应商池里没有供应商提供模型 {model}");
    }
    let mut gate = crate::gateway::quota::DispatchGate::default();
    let mut permitted = Vec::new();
    let mut blocked = None;
    for (relay, model) in relays {
        match gate.check(gateway, &relay) {
            Ok(()) => permitted.push((relay, model)),
            Err(error) => {
                let stop = matches!(
                    &error.action,
                    &crate::gateway::store::SubscriptionLimitAction::Stop
                );
                blocked = Some(error);
                if stop {
                    break;
                }
            }
        }
    }
    if permitted.is_empty() {
        if let Some(error) = blocked {
            return Err(error.into());
        }
    }
    let preferred = preferred.filter(|(relay_id, preferred_model)| {
        permitted
            .iter()
            .any(|(relay, model)| &relay.id == relay_id && model == preferred_model)
    });
    if mapped_to_default && advance && should_log_model_rewrite(agent.as_str(), requested_model.trim()) {
        // 请求的模型池里没人提供，按「未知模型」策略整个改发到默认模型的供应商（#2874）。
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "gateway.model_mapped_to_default",
            serde_json::json!({
                "agent": agent.as_str(),
                "requestedModel": requested_model.trim(),
                "model": model,
                "provider": permitted.first().map(|(relay, _)| relay.name.as_str()).unwrap_or_default(),
                "providerId": permitted.first().map(|(relay, _)| relay.id.as_str()).unwrap_or_default(),
                "hint": "请求的模型不在网关供应商池里（或所在供应商已关闭），已按「未知模型」策略改发到默认模型；可在网关的 Agent 接入设置里改成原样转发",
            }),
        );
    }

    let now = Instant::now();
    let mut candidates = Vec::new();
    let mut cooling: Option<Duration> = None;
    // 冷却 / 熔断中的候选：只在一个可用候选都没有时才用得上。
    let mut waiting: Vec<(GatewayCandidate, Duration, key_pool::WaitReason)> = Vec::new();
    // 拓展用 gateway.replaces 关掉的内置环节：冷却 / 熔断中的 Key 照常当候选（排在可用的后面），
    // 本地限速（RPM / 并发）不再生效。要避开哪些 Key 由拓展在 route 段自己筛。
    let cooldown_off = crate::extension_gateway::builtin_replaced("builtin:key-cooldown");
    let breaker_off = crate::extension_gateway::builtin_replaced("builtin:key-breaker");
    let rate_limit_off = crate::extension_gateway::builtin_replaced("builtin:local-rate-limit");
    for (relay, upstream_model) in permitted {
        let provider = gateway.provider(&relay.id);
        let window = provider.and_then(|provider| provider.window_for(&upstream_model));
        // 限速满了的 Key 照样进候选：是否跳过、排队由候选循环在发请求时按实时名额决定。
        let limits = provider
            .filter(|_| !rate_limit_off)
            .map(GatewayProvider::key_limits)
            .unwrap_or_default();
        let order = key_pool::order_keys_detailed(
            &relay,
            gateway.key_policy.mode,
            conversation.as_deref(),
            now,
            advance,
        );
        if let Some(soonest) = order.soonest {
            cooling = Some(cooling.map_or(soonest, |current| current.min(soonest)));
        }
        let candidate = |key: key_pool::KeyCandidate| GatewayCandidate {
            relay: key.relay,
            key: key.key,
            upstream_model: upstream_model.clone(),
            window,
            limits,
        };
        for key in order.available {
            candidates.push(candidate(key));
        }
        for key in order.waiting {
            let bypass = match key.reason {
                // 自己在冷却的 Key 可能同时赶上供应商熔断：熔断没关掉时仍按熔断拦着。
                key_pool::WaitReason::Cooling => {
                    cooldown_off && (breaker_off || !key_pool::breaker_open(&key.candidate.key.relay_id, now))
                }
                key_pool::WaitReason::Breaker => breaker_off,
            };
            if bypass {
                candidates.push(candidate(key.candidate));
            } else {
                waiting.push((candidate(key.candidate), key.remaining, key.reason));
            }
        }
    }
    let mut waiting_candidates = Vec::new();
    if candidates.is_empty() {
        // 熔断是为了把请求引到别家（#2802）：一个可用候选都没有时它拦下的就是全部请求，
        // 把偶发的 5xx / 断连放大成整整一分钟的本地 429，比直连还糟。这时熔断中的供应商照常放行
        // （相当于半开探测），成功一次就关闭熔断；失败时客户端拿到的是上游的真实错误。
        for (candidate, remaining, reason) in waiting {
            match reason {
                key_pool::WaitReason::Breaker => candidates.push(candidate),
                key_pool::WaitReason::Cooling => waiting_candidates.push(WaitingCandidate {
                    candidate,
                    remaining,
                }),
            }
        }
        waiting_candidates.sort_by_key(|item| item.remaining);
        if candidates.is_empty() {
            cooling = waiting_candidates
                .first()
                .map(|item| item.remaining)
                .or(cooling);
        } else {
            // 熔断中的供应商已放行成候选：`cooling` 只在一个候选都没有时才有意义（#2761）。
            waiting_candidates.clear();
            cooling = None;
        }
    }
    Ok(RoutePlan {
        agent: agent.clone(),
        source_model: requested_model.trim().to_string(),
        model,
        group_id,
        conversation,
        sticky_ttl,
        preferred,
        hold_on_switch,
        candidates,
        cooling,
        waiting: waiting_candidates,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::RelayMode;

    fn provider(id: &str, models: &str) -> GatewayProvider {
        GatewayProvider {
            enabled: true,
            source: String::new(),
            price: None,
            model_prices: Default::default(),
            model_windows: Default::default(),
            context_window: None,
            proxy: String::new(),
            key_rpm: None,
            key_concurrency: None,
            upstream_new: Vec::new(),
            upstream_gone: Vec::new(),
            models_checked_at_ms: 0,
            newapi_access: None,
            upstream_meta: Default::default(),
            subscription_policy: None,
            relay: RelayProfile {
                id: id.to_string(),
                name: id.to_string(),
                relay_mode: RelayMode::PureApi,
                base_url: format!("https://{id}.test/v1"),
                api_key: format!("sk-{id}"),
                model_list: models.to_string(),
                ..RelayProfile::default()
            },
            extra: Default::default(),
        }
    }

    fn member(model: &str) -> GroupMember {
        GroupMember {
            model: model.to_string(),
            weight: 1,
        }
    }

    fn group(id: &str, mode: GroupMode, members: Vec<GroupMember>) -> GatewayGroup {
        GatewayGroup {
            id: id.to_string(),
            name: id.to_string(),
            mode,
            sticky: GroupSticky::Cache,
            members,
            ..GatewayGroup::default()
        }
    }

    fn gateway(model: &str) -> GatewaySettings {
        let mut gateway = GatewaySettings {
            providers: vec![
                provider("ds", "deepseek-v4-pro\ndeepseek-v4-flash"),
                provider("kimi", "kimi-k3"),
                provider("mirror", "deepseek-v4-pro"),
                GatewayProvider {
                    enabled: false,
                    ..provider("off", "off-model")
                },
            ],
            groups: vec![
                group(
                    "fast",
                    GroupMode::InOrder,
                    vec![member("ds/deepseek-v4-flash"), member("kimi-k3")],
                ),
                group(
                    "coder",
                    GroupMode::InOrder,
                    vec![member("mirror/deepseek-v4-pro"), member("group/fast")],
                ),
                group(
                    "loop",
                    GroupMode::InOrder,
                    vec![member("group/loop"), member("kimi-k3")],
                ),
                group("empty", GroupMode::InOrder, Vec::new()),
            ],
            ..GatewaySettings::default()
        };
        gateway.bindings.insert(
            "claude".to_string(),
            AgentBinding {
                enabled: true,
                model: model.to_string(),
                ..AgentBinding::default()
            },
        );
        gateway
    }

    fn routed(gateway: &GatewaySettings, model: &str) -> Vec<(String, String)> {
        resolve_plan(gateway, &ToolId::Claude, model, None)
            .unwrap()
            .candidates
            .into_iter()
            .map(|candidate| (candidate.relay.id, candidate.upstream_model))
            .collect()
    }

    #[test]
    fn unconfigured_exhausted_subscription_keeps_api_failover_candidates() {
        use base64::Engine;
        use crate::gateway::quota::{ProviderQuota, QuotaWindow, record_for_relay};
        use crate::gateway::store::SubscriptionPolicy;

        let model = "legacy-quota-failover-model";
        let mut official = provider("legacy-quota-subscription", model);
        official.relay.relay_mode = RelayMode::Official;
        official.relay.base_url.clear();
        official.relay.api_key.clear();
        let claims = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
            serde_json::json!({"iss": "https://auth.openai.com", "sub": "legacy-quota-failover"})
                .to_string(),
        );
        official.relay.auth_contents = serde_json::json!({"tokens": {
            "id_token": format!("header.{claims}.signature"),
            "access_token": "legacy-quota-access",
            "account_id": "legacy-quota-workspace"
        }}).to_string();
        record_for_relay(&official.relay, ProviderQuota {
            windows: vec![QuotaWindow {
                used_percent: 100.0,
                resets_at_ms: Some(u64::MAX),
                ..Default::default()
            }],
            updated_at_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64,
            official_allowed: Some(false),
            ..Default::default()
        });
        let mut gateway = gateway("group/legacy-quota-failover");
        gateway.providers = vec![official, provider("legacy-quota-api", model)];
        gateway.groups = vec![group(
            "legacy-quota-failover",
            GroupMode::InOrder,
            vec![member(&format!("legacy-quota-subscription/{model}")), member(&format!("legacy-quota-api/{model}"))],
        )];
        assert_eq!(
            routed(&gateway, "group/legacy-quota-failover"),
            [
                ("legacy-quota-subscription".into(), model.into()),
                ("legacy-quota-api".into(), model.into())
            ]
        );
        gateway.providers[0].subscription_policy = Some(SubscriptionPolicy::default());
        assert!(resolve_plan(
            &gateway, &ToolId::Claude, "group/legacy-quota-failover", None,
        ).is_err(), "only an explicitly configured stop policy may stop API failover");
    }

    fn pair(relay: &str, model: &str) -> (String, String) {
        (relay.to_string(), model.to_string())
    }

    #[test]
    fn pool_exposes_enabled_models_and_groups() {
        let mut gateway = gateway("");
        let binding = gateway.binding(&ToolId::Claude).unwrap().clone();
        assert_eq!(
            pool_models(&gateway, &binding),
            vec![
                "deepseek-v4-pro",
                "deepseek-v4-flash",
                "kimi-k3",
                "group/fast",
                "group/coder",
                "group/loop"
            ]
        );
        assert_eq!(binding_default_model(&gateway, &binding), "deepseek-v4-pro");
        // 「挑选」排除掉的供应商：模型不再出现，默认模型顺延。
        let binding = gateway.bindings.get_mut("claude").unwrap();
        binding.pool_exclude.push("ds".to_string());
        let binding = binding.clone();
        assert_eq!(
            pool_models(&gateway, &binding)[..2],
            ["kimi-k3".to_string(), "deepseek-v4-pro".to_string()]
        );
        assert_eq!(binding_default_model(&gateway, &binding), "kimi-k3");
    }

    #[test]
    fn bare_model_tries_every_provider_in_pool_order() {
        let gateway = gateway("deepseek-v4-pro");
        assert_eq!(
            routed(&gateway, "deepseek-v4-pro"),
            vec![
                pair("ds", "deepseek-v4-pro"),
                pair("mirror", "deepseek-v4-pro")
            ]
        );
        assert_eq!(routed(&gateway, "kimi-k3"), vec![pair("kimi", "kimi-k3")]);
        // 关掉的供应商不参与；不认识的模型改写成默认模型。
        assert_eq!(routed(&gateway, "off-model")[0].1, "deepseek-v4-pro");
        assert_eq!(routed(&gateway, "claude-opus-5-5")[0].1, "deepseek-v4-pro");
    }

    /// Claude Desktop 只认 Claude 名字：非 Claude 模型给数字别名，路由时还原成真实模型。
    #[test]
    fn claude_aliases_are_stable_digits_and_route_back() {
        // 带 claude 的名字原样；其余是「前缀 + 10 位数字」，同名永远同一个别名。
        assert_eq!(claude_alias("claude-opus-5-5"), "claude-opus-5-5");
        let alias = claude_alias("kimi-k3");
        let digits = alias.strip_prefix(CLAUDE_ALIAS_PREFIX).unwrap();
        assert_eq!(digits.len(), 10);
        assert!(digits.chars().all(|c| c.is_ascii_digit()));
        assert_eq!(claude_alias("kimi-k3"), alias);
        assert_ne!(claude_alias("deepseek-v4-pro"), alias);

        let gateway = gateway("deepseek-v4-pro");
        let binding = gateway.binding(&ToolId::Claude).unwrap().clone();
        assert_eq!(
            effective_model(&gateway, &binding, "deepseek-v4-pro", &alias.to_uppercase()),
            "kimi-k3"
        );
        assert_eq!(routed(&gateway, &alias), vec![pair("kimi", "kimi-k3")]);
        // 模型组也有别名；对不上任何模型的别名按未知模型处理。
        assert_eq!(
            routed(&gateway, &claude_alias("group/fast"))[0],
            pair("ds", "deepseek-v4-flash")
        );
        assert_eq!(
            routed(&gateway, "claude-gw-0000000000")[0].1,
            "deepseek-v4-pro"
        );
    }

    /// 有思考档位的模型用 mythos-gw- 别名（Desktop 才出思考等级菜单），旧的 claude-gw- 别名仍能还原。
    #[test]
    fn effort_aliases_use_mythos_prefix_and_route_back() {
        assert_eq!(claude_desktop_alias("claude-test-model", true), "claude-test-model");
        assert_eq!(claude_desktop_alias("kimi-k3", false), claude_alias("kimi-k3"));
        let alias = claude_desktop_alias("kimi-k3", true);
        let digits = alias.strip_prefix(CLAUDE_EFFORT_ALIAS_PREFIX).unwrap();
        let old = claude_alias("kimi-k3");
        assert_eq!(digits, old.strip_prefix(CLAUDE_ALIAS_PREFIX).unwrap());
        let gateway = gateway("deepseek-v4-pro");
        let binding = gateway.binding(&ToolId::Claude).unwrap().clone();
        for requested in [alias.to_uppercase(), old] {
            assert_eq!(
                effective_model(&gateway, &binding, "deepseek-v4-pro", &requested),
                "kimi-k3"
            );
        }
    }

    /// #2703：绑定指定了审批模型时，codex-auto-review 改发到它；没指定时行为不变
    /// （池里有家把 codex-auto-review 列进模型表就发给它，否则改写成主模型）。
    #[test]
    fn auto_review_requests_follow_binding_review_model() {
        let mut gateway = gateway("deepseek-v4-pro");
        gateway
            .providers
            .insert(0, provider("chatgpt", "gpt-5.5\ncodex-auto-review"));
        assert_eq!(
            routed(&gateway, "codex-auto-review"),
            vec![pair("chatgpt", "codex-auto-review")]
        );
        gateway.providers.remove(0);
        assert_eq!(
            routed(&gateway, "codex-auto-review"),
            vec![
                pair("ds", "deepseek-v4-pro"),
                pair("mirror", "deepseek-v4-pro")
            ]
        );

        gateway
            .providers
            .insert(0, provider("chatgpt", "gpt-5.5\ncodex-auto-review"));
        gateway.bindings.get_mut("claude").unwrap().auto_review_model =
            " kimi/kimi-k3 ".to_string();
        assert_eq!(
            routed(&gateway, "codex-auto-review"),
            vec![pair("kimi", "kimi-k3")]
        );
        // 模型组也可以：按组的成员展开。
        gateway.bindings.get_mut("claude").unwrap().auto_review_model = "group/fast".to_string();
        assert_eq!(
            routed(&gateway, "codex-auto-review"),
            vec![pair("ds", "deepseek-v4-flash"), pair("kimi", "kimi-k3")]
        );
        // 只影响审批请求，其它模型照旧。
        assert_eq!(routed(&gateway, "gpt-5.5"), vec![pair("chatgpt", "gpt-5.5")]);
    }

    fn user_turn(text: &str) -> serde_json::Value {
        serde_json::json!({
            "model": "gpt-6-luna",
            "input": [
                { "type": "message", "role": "developer", "content": [{ "type": "input_text", "text": "Keep the current chat title in developer text" }] },
                { "type": "message", "role": "user", "content": [{ "type": "input_text", "text": text }] }
            ]
        })
    }

    /// 起名请求只按最后一条用户消息的开头认：桌面版首次起名、重新起名、终端版起名都算，
    /// 正常对话里提到这些字、或别的角色消息里出现都不算。
    #[test]
    fn codex_title_requests_are_recognized_by_prompt_prefix() {
        assert!(is_codex_title_request(&user_turn(
            "You are a helpful assistant. You will be presented with a user prompt, and your job is to provide a short title for a task that will be created from that prompt.\nPrompt: 修一下登录"
        )));
        assert!(is_codex_title_request(&user_turn(
            "Keep the current chat title \"修登录\" if it still fits the conversation. If the focus has changed, write a more useful title."
        )));
        assert!(is_codex_title_request(&user_turn(
            "  Generate a concise, single-line task title of at most 36 characters"
        )));
        // 字符串形式的 content 也认。
        assert!(is_codex_title_request(&serde_json::json!({
            "input": [{ "role": "user", "content": "Keep the current chat title \"x\" if it still fits." }]
        })));

        assert!(!is_codex_title_request(&user_turn("帮我看看 Keep the current chat title 这句是什么意思")));
        assert!(!is_codex_title_request(&user_turn("修一下登录")));
        // 起名提示词在更早的用户消息里、最后一条是正常提问：不算。
        assert!(!is_codex_title_request(&serde_json::json!({
            "input": [
                { "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "Keep the current chat title \"x\"" }] },
                { "type": "message", "role": "assistant", "content": [{ "type": "output_text", "text": "好的" }] },
                { "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "继续" }] }
            ]
        })));
        assert!(!is_codex_title_request(&serde_json::json!({ "input": "Keep the current chat title" })));
    }

    /// 绑定指定了起名模型时，起名请求改发到它；没指定、或不是起名请求时不改。
    #[test]
    fn title_requests_follow_binding_title_model() {
        let mut gateway = gateway("deepseek-v4-pro");
        let title = user_turn("Keep the current chat title \"x\" if it still fits the conversation.");
        assert_eq!(title_model_override(&gateway, &ToolId::Claude, &title), None);

        gateway.bindings.get_mut("claude").unwrap().title_model = " kimi/kimi-k3 ".to_string();
        let model = title_model_override(&gateway, &ToolId::Claude, &title).unwrap();
        assert_eq!(model, "kimi/kimi-k3");
        assert_eq!(routed(&gateway, &model), vec![pair("kimi", "kimi-k3")]);
        assert_eq!(
            title_model_override(&gateway, &ToolId::Claude, &user_turn("修一下登录")),
            None
        );
        // 没开网关的 Agent 不改。
        assert_eq!(title_model_override(&gateway, &ToolId::Codex, &title), None);
    }

    #[test]
    fn qualified_model_goes_to_that_provider_even_if_unlisted() {
        let gateway = gateway("deepseek-v4-pro");
        assert_eq!(
            routed(&gateway, "mirror/deepseek-v4-pro"),
            vec![pair("mirror", "deepseek-v4-pro")]
        );
        assert_eq!(
            routed(&gateway, "kimi/kimi-k3-preview"),
            vec![pair("kimi", "kimi-k3-preview")]
        );
        // 关掉的供应商不能用前缀绕过。
        assert_eq!(routed(&gateway, "off/off-model")[0].1, "deepseek-v4-pro");
    }

    #[test]
    fn groups_expand_members_and_nest() {
        let gateway = gateway("deepseek-v4-pro");
        let plan = resolve_plan(&gateway, &ToolId::Claude, "group/coder", None).unwrap();
        assert_eq!(plan.group_id.as_deref(), Some("coder"));
        let relays = plan
            .candidates
            .into_iter()
            .map(|candidate| (candidate.relay.id, candidate.upstream_model))
            .collect::<Vec<_>>();
        assert_eq!(
            relays,
            vec![
                pair("mirror", "deepseek-v4-pro"),
                pair("ds", "deepseek-v4-flash"),
                pair("kimi", "kimi-k3"),
            ]
        );
        // 自己引用自己的组不会死循环。
        assert_eq!(
            routed(&gateway, "group/loop"),
            vec![pair("kimi", "kimi-k3")]
        );
        // 空组不出现在模型列表里，请求它按未知模型处理。
        assert_eq!(routed(&gateway, "group/empty")[0].1, "deepseek-v4-pro");
    }

    #[test]
    fn rotate_and_weighted_change_the_first_member() {
        let mut gateway = gateway("deepseek-v4-pro");
        gateway.groups.push(group(
            "spin",
            GroupMode::Rotate,
            vec![member("ds/deepseek-v4-flash"), member("kimi/kimi-k3")],
        ));
        let firsts = (0..4)
            .map(|_| routed(&gateway, "group/spin")[0].0.clone())
            .collect::<Vec<_>>();
        assert_eq!(firsts, vec!["ds", "kimi", "ds", "kimi"]);

        gateway.groups.push(group(
            "heavy",
            GroupMode::Weighted,
            vec![
                GroupMember {
                    model: "ds/deepseek-v4-flash".to_string(),
                    weight: 3,
                },
                member("kimi/kimi-k3"),
            ],
        ));
        let ds_first = (0..8)
            .map(|_| routed(&gateway, "group/heavy")[0].0.clone())
            .filter(|id| id == "ds")
            .count();
        assert_eq!(ds_first, 6, "权重 3:1，8 次里 6 次先试 ds");
    }

    /// #2874：只有「池里没人提供 → 改写成默认模型」才算改写，要能被认出来记日志。
    #[test]
    fn mapping_unknown_model_to_default_is_reported() {
        let gateway = gateway("kimi-k3");
        let binding = gateway.active_binding(&ToolId::Claude).unwrap();
        assert_eq!(
            effective_model_detailed(&gateway, binding, "kimi-k3", "gpt-unknown"),
            ("kimi-k3".to_string(), true)
        );
        assert_eq!(
            effective_model_detailed(&gateway, binding, "kimi-k3", "kimi-k3"),
            ("kimi-k3".to_string(), false),
            "池里有的模型不算改写"
        );
        assert_eq!(
            effective_model_detailed(&gateway, binding, "kimi-k3", ""),
            ("kimi-k3".to_string(), false),
            "客户端没给模型名不算改写"
        );
        assert!(should_log_model_rewrite("claude", "gpt-unknown-throttle"));
        assert!(
            !should_log_model_rewrite("claude", "gpt-unknown-throttle"),
            "同一个模型 60 秒内只记一次"
        );
    }

    /// #2897：`<已删除的供应商>/<模型>` 不能带着前缀发给别家，池里有人提供同名模型时去掉前缀交给它。
    #[test]
    fn qualified_model_of_a_removed_provider_drops_the_prefix() {
        let mut gateway = gateway("kimi-k3");
        // 绑定里还声明着删掉的供应商的模型（删除前留下的）。
        gateway.bindings.get_mut("claude").unwrap().extra_models =
            vec!["gone/deepseek-v4-flash".to_string()];
        assert_eq!(
            routed(&gateway, "gone/deepseek-v4-flash"),
            vec![pair("ds", "deepseek-v4-flash")]
        );
        // 关掉的供应商同理。
        assert_eq!(
            routed(&gateway, "off/kimi-k3"),
            vec![pair("kimi", "kimi-k3")]
        );
        // 没人提供去掉前缀的模型时不乱发：落回原来的「未知模型」处理。
        assert!(
            routed(&gateway, "gone/nobody-serves-this")
                .iter()
                .all(|(_, model)| !model.starts_with("gone/")),
            "前缀不能原样发给上游"
        );
    }

    #[test]
    fn passthrough_sends_unknown_models_to_the_default_provider() {
        let mut gateway = gateway("kimi-k3");
        gateway.bindings.get_mut("claude").unwrap().unknown_model = UnknownModelPolicy::Passthrough;
        assert_eq!(
            routed(&gateway, "kimi-k3-preview"),
            vec![pair("kimi", "kimi-k3-preview")]
        );
        gateway.providers.clear();
        assert!(resolve_plan(&gateway, &ToolId::Claude, "kimi-k3", None).is_err());
    }

    #[test]
    fn glob_matches_any_run_of_characters_ignoring_case() {
        assert!(glob_matches(
            "openrouter/*:free",
            "OpenRouter/meta/llama-4:free"
        ));
        assert!(glob_matches("*-free", "big-pickle-free"));
        assert!(glob_matches("deepseek*", "deepseek-v4-pro"));
        assert!(glob_matches("*", "anything"));
        assert!(glob_matches("a*b*c", "a-x-b-y-c"));
        assert!(!glob_matches("a*b*c", "a-x-c-y-b"));
        assert!(!glob_matches("ab*ba", "aba"), "前后缀不能重叠");
        assert!(
            !glob_matches("deepseek", "deepseek-v4"),
            "没有 * 就要整串相同"
        );
    }

    #[test]
    fn pattern_members_follow_the_pool() {
        let mut gateway = gateway("group/wild");
        gateway.groups.push(group(
            "wild",
            GroupMode::InOrder,
            vec![member("*-v4-pro"), member("kimi/*")],
        ));
        assert_eq!(
            routed(&gateway, "group/wild"),
            vec![
                pair("ds", "deepseek-v4-pro"),
                pair("mirror", "deepseek-v4-pro"),
                pair("kimi", "kimi-k3"),
            ]
        );
        // 上游新增的模型自动算进来；关掉的供应商不算。
        gateway.providers[1].relay.model_list = "kimi-k3\nkimi-k3-turbo".to_string();
        assert_eq!(routed(&gateway, "group/wild").len(), 4);
        assert_eq!(
            pattern_matches(&gateway, "off/*"),
            Vec::<String>::new(),
            "关掉的供应商不匹配"
        );
        assert_eq!(
            pattern_matches(&gateway, "kimi/*"),
            vec!["kimi/kimi-k3", "kimi/kimi-k3-turbo"]
        );
    }

    #[test]
    fn sticky_groups_keep_a_conversation_on_the_member_that_answered() {
        let mut gateway = gateway("group/sticky-spin");
        gateway.groups.push(group(
            "sticky-spin",
            GroupMode::Rotate,
            vec![member("ds/deepseek-v4-flash"), member("kimi-k3")],
        ));
        let plan = |gateway: &GatewaySettings, conversation: &str| {
            resolve_plan(
                gateway,
                &ToolId::Claude,
                "group/sticky-spin",
                Some(conversation),
            )
            .unwrap()
        };
        let first = plan(&gateway, "sticky-conv");
        let answered = first.candidates[0].clone();
        remember_answer(&first, &answered);
        for _ in 0..3 {
            assert_eq!(
                plan(&gateway, "sticky-conv").candidates[0].relay.id,
                answered.relay.id,
                "轮换组里同一会话也先试上次回答的成员"
            );
        }
        // 别的会话照常轮换；不固定时按模式重新排。
        let others: Vec<String> = (0..2)
            .map(|_| {
                plan(&gateway, "another-conv").candidates[0]
                    .relay
                    .id
                    .clone()
            })
            .collect();
        assert_ne!(others[0], others[1]);
        gateway.groups.last_mut().unwrap().sticky = GroupSticky::Off;
        let spins: Vec<String> = (0..2)
            .map(|_| plan(&gateway, "sticky-conv").candidates[0].relay.id.clone())
            .collect();
        assert_ne!(spins[0], spins[1]);
    }

    #[test]
    fn slashed_model_names_beat_provider_prefixes() {
        let mut gateway = gateway("deepseek-v4-pro");
        gateway.providers.extend([
            provider("openai", "gpt-4o\ngpt-5"),
            provider("openrouter", "openai/gpt-5\nmeta/llama-5"),
        ]);
        // 池里有人把 `openai/gpt-5` 当完整模型名声明：按裸模型走 OpenRouter，不拆成 openai + gpt-5。
        assert_eq!(
            routed(&gateway, "openai/gpt-5"),
            vec![pair("openrouter", "openai/gpt-5")]
        );
        // 没人声明整串时照旧拆前缀。
        assert_eq!(
            routed(&gateway, "openai/gpt-4o"),
            vec![pair("openai", "gpt-4o")]
        );
        assert_eq!(
            routed(&gateway, "openrouter/openai/gpt-5"),
            vec![pair("openrouter", "openai/gpt-5")]
        );
        // 通配符口径相同：`openai/*` 里的 `openai/gpt-5` 归 OpenRouter，不再算 openai 的 gpt-5。
        assert_eq!(
            pattern_matches(&gateway, "openai/*"),
            vec!["openai/gpt-4o", "openrouter/openai/gpt-5"]
        );

        // 删掉 openai：指向 OpenRouter 的 `openai/gpt-5` 和还能匹配的通配符留下，只指向 openai 的去掉。
        let mut remaining = gateway.providers.clone();
        remaining.retain(|provider| provider.relay.id != "openai");
        assert!(member_outlives_provider(
            &remaining,
            "openai",
            "openai/gpt-5"
        ));
        assert!(member_outlives_provider(&remaining, "openai", "openai/*"));
        assert!(!member_outlives_provider(
            &remaining,
            "openai",
            "openai/gpt-4o"
        ));
        assert!(!member_outlives_provider(
            &remaining,
            "openai",
            "openai/gpt-*o"
        ));
        assert!(member_outlives_provider(
            &remaining,
            "openai",
            "kimi/kimi-k3"
        ));
        assert!(member_outlives_provider(&remaining, "openai", "gpt-5"));
    }

    #[test]
    fn preview_plan_does_not_advance_rotation() {
        let mut gateway = gateway("deepseek-v4-pro");
        gateway.groups.push(group(
            "peek",
            GroupMode::Rotate,
            vec![member("ds/deepseek-v4-flash"), member("kimi/kimi-k3")],
        ));
        let preview = |gateway: &GatewaySettings| {
            preview_plan(gateway, &ToolId::Claude, "group/peek", None)
                .unwrap()
                .candidates[0]
                .relay
                .id
                .clone()
        };
        let first = preview(&gateway);
        assert_eq!(preview(&gateway), first, "预览多少次都不推进轮换");
        assert_eq!(
            routed(&gateway, "group/peek")[0].0,
            first,
            "预览看到的就是下一个真实请求先试的成员"
        );
        assert_ne!(routed(&gateway, "group/peek")[0].0, first);
    }

    #[test]
    fn in_order_groups_only_hold_a_fallback_member_briefly() {
        // #2767：依次 + 整个会话固定，第一家答不了换到第二家后，不能把会话永久钉在第二家。
        let mut gateway = gateway("group/prio");
        let mut prio = group(
            "prio",
            GroupMode::InOrder,
            vec![member("ds/deepseek-v4-flash"), member("kimi-k3")],
        );
        prio.sticky = GroupSticky::Session;
        gateway.groups.push(prio);
        let plan = |conversation: &str| {
            resolve_plan(&gateway, &ToolId::Claude, "group/prio", Some(conversation)).unwrap()
        };
        let key = ("prio".to_string(), "prio-conv".to_string());

        let first = plan("prio-conv");
        assert_eq!(
            first.preferred,
            Some(("ds".to_string(), "deepseek-v4-flash".to_string()))
        );
        let fallback = first
            .candidates
            .iter()
            .find(|candidate| candidate.relay.id == "kimi")
            .unwrap()
            .clone();
        remember_answer(&first, &fallback);
        let held = sticky_table().lock().unwrap().get(&key).cloned().unwrap();
        assert_eq!(held.3, FALLBACK_STICKY_TTL, "顶替只临时固定");
        assert!(held.4);
        assert_eq!(
            plan("prio-conv").candidates[0].relay.id,
            "kimi",
            "临时固定期内继续用顶替的成员，保住它的缓存"
        );
        // 期间再成功也不续期：仍按第一次顶替的时间算。
        remember_answer(&plan("prio-conv"), &fallback);
        assert_eq!(sticky_table().lock().unwrap().get(&key).unwrap().2, held.2);

        // 临时固定到期后先回去试排在前面的成员。
        sticky_table().lock().unwrap().get_mut(&key).unwrap().2 =
            Instant::now() - FALLBACK_STICKY_TTL - Duration::from_secs(1);
        let back = plan("prio-conv");
        assert_eq!(back.candidates[0].relay.id, "ds");
        // 排在前面的成员回答后按组的设置固定整个会话。
        let primary = back.candidates[0].clone();
        remember_answer(&back, &primary);
        let pinned = sticky_table().lock().unwrap().get(&key).cloned().unwrap();
        assert_eq!(pinned.3, SESSION_STICKY_TTL);
        assert!(!pinned.4);
        assert_eq!(plan("prio-conv").candidates[0].relay.id, "ds");
    }

    #[test]
    fn usage_groups_only_hold_when_this_request_switched_member() {
        // 额度优先每次按实时额度重排：固定在 A 后 A 的余量落到 B 后面，A 照常回答不能被判成顶替，
        // 否则额度接近的两个账号会每 10 分钟来回切、缓存反复失效。
        use crate::gateway::quota::{ProviderQuota, QuotaWindow, record};
        let quota = |used: f64| ProviderQuota {
            windows: vec![QuotaWindow {
                window_minutes: Some(300),
                used_percent: used,
                resets_at_ms: None,
                ..QuotaWindow::default()
            }],
            updated_at_ms: 0,
            ..ProviderQuota::default()
        };
        let mut gateway = gateway("group/usage-pin");
        gateway.providers.extend([
            provider("us-a", "usage-pin-model"),
            provider("us-b", "usage-pin-model"),
        ]);
        let mut usage = group("usage-pin", GroupMode::Usage, vec![member("usage-pin-model")]);
        usage.sticky = GroupSticky::Session;
        gateway.groups.push(usage);
        let plan = |gateway: &GatewaySettings| {
            resolve_plan(gateway, &ToolId::Claude, "group/usage-pin", Some("usage-conv")).unwrap()
        };
        let key = ("usage-pin".to_string(), "usage-conv".to_string());
        let entry = || sticky_table().lock().unwrap().get(&key).cloned().unwrap();

        record("us-a", quota(20.0));
        record("us-b", quota(40.0));
        let first = plan(&gateway);
        assert_eq!(first.candidates[0].relay.id, "us-a");
        assert_eq!(first.preferred, None);
        remember_answer(&first, &first.candidates[0].clone());
        assert_eq!(entry().3, SESSION_STICKY_TTL);

        // A 用掉额度后排到 B 后面，但会话记着 A、A 照常回答：仍整会话固定。
        record("us-a", quota(60.0));
        let pinned = plan(&gateway);
        assert_eq!(pinned.candidates[0].relay.id, "us-a", "会话固定把 A 挪到最前");
        remember_answer(&pinned, &pinned.candidates[0].clone());
        assert_eq!(entry().3, SESSION_STICKY_TTL);
        assert!(!entry().4);

        // 这次 A 答不了、换到 B：临时固定，期间 B 接着回答也不续期。
        let switched = plan(&gateway);
        let b = switched
            .candidates
            .iter()
            .find(|candidate| candidate.relay.id == "us-b")
            .unwrap()
            .clone();
        remember_answer(&switched, &b);
        let held = entry();
        assert_eq!(held.3, FALLBACK_STICKY_TTL);
        assert!(held.4);
        let again = plan(&gateway);
        assert_eq!(again.candidates[0].relay.id, "us-b");
        remember_answer(&again, &again.candidates[0].clone());
        assert_eq!(entry().2, held.2, "顶替期内不续期");
        assert!(entry().4);
    }

    #[test]
    fn rotating_groups_keep_session_sticky_for_any_member() {
        let mut gateway = gateway("group/spin-session");
        let mut spin = group(
            "spin-session",
            GroupMode::Rotate,
            vec![member("ds/deepseek-v4-flash"), member("kimi-k3")],
        );
        spin.sticky = GroupSticky::Session;
        gateway.groups.push(spin);
        let first = resolve_plan(
            &gateway,
            &ToolId::Claude,
            "group/spin-session",
            Some("spin-conv"),
        )
        .unwrap();
        assert_eq!(first.preferred, None, "轮换没有优先级，不算顶替");
        remember_answer(&first, &first.candidates[1].clone());
        let table = sticky_table().lock().unwrap();
        let entry = table
            .get(&("spin-session".to_string(), "spin-conv".to_string()))
            .unwrap();
        assert_eq!(entry.3, SESSION_STICKY_TTL);
        assert!(!entry.4);
    }

    #[test]
    fn sticky_off_groups_are_not_remembered_and_the_table_stays_bounded() {
        let mut gateway = gateway("deepseek-v4-pro");
        let mut off = group(
            "never-sticky",
            GroupMode::InOrder,
            vec![member("ds/deepseek-v4-flash"), member("kimi/kimi-k3")],
        );
        off.sticky = GroupSticky::Off;
        gateway.groups.push(off);
        let plan = resolve_plan(
            &gateway,
            &ToolId::Claude,
            "group/never-sticky",
            Some("off-conv"),
        )
        .unwrap();
        assert_eq!(plan.sticky_ttl, None);
        let answered = plan.candidates[1].clone();
        remember_answer(&plan, &answered);
        let table = sticky_table().lock().unwrap();
        assert!(
            !table.contains_key(&("never-sticky".to_string(), "off-conv".to_string())),
            "不固定的组不记会话"
        );
        drop(table);

        // 表满时：按各条有效期清过期的，还多就淘汰最旧的，清到上限的 3/4。
        let mut table = StickyTable::new();
        let start = Instant::now();
        for index in 0..STICKY_LIMIT {
            table.insert(
                ("g".to_string(), format!("c{index}")),
                (
                    "r".to_string(),
                    "m".to_string(),
                    start + Duration::from_millis(index as u64),
                    SESSION_STICKY_TTL,
                    false,
                ),
            );
        }
        trim_sticky(&mut table, start + Duration::from_secs(1));
        assert!(table.len() < STICKY_TRIM_TO, "{}", table.len());
        assert!(table.contains_key(&("g".to_string(), format!("c{}", STICKY_LIMIT - 1))));
        assert!(!table.contains_key(&("g".to_string(), "c0".to_string())));
        // 短有效期的条目过期后直接清掉。
        let mut table = StickyTable::new();
        table.insert(
            ("g".to_string(), "short".to_string()),
            ("r".to_string(), "m".to_string(), start, CACHE_STICKY_TTL, false),
        );
        trim_sticky(
            &mut table,
            start + CACHE_STICKY_TTL + Duration::from_secs(1),
        );
        assert!(table.is_empty());
    }

    #[test]
    fn usage_and_fast_modes_reorder_every_upstream() {
        use crate::gateway::quota::{ProviderQuota, QuotaWindow, record};
        let mut gateway = gateway("group/quota");
        gateway.providers.extend([
            provider("q-low", "same"),
            provider("q-unknown", "same"),
            provider("q-empty", "same"),
            provider("q-high", "same"),
        ]);
        let quota = |used: f64| ProviderQuota {
            windows: vec![QuotaWindow {
                window_minutes: Some(300),
                used_percent: used,
                resets_at_ms: None,
                ..QuotaWindow::default()
            }],
            updated_at_ms: 0,
            ..ProviderQuota::default()
        };
        record("q-low", quota(90.0));
        record("q-empty", quota(100.0));
        record("q-high", quota(10.0));
        gateway
            .groups
            .push(group("quota", GroupMode::Usage, vec![member("same")]));
        let order: Vec<String> = routed(&gateway, "group/quota")
            .into_iter()
            .map(|(relay, _)| relay)
            .collect();
        assert_eq!(order, ["q-high", "q-low", "q-unknown", "q-empty"]);

        gateway.providers.extend([
            provider("sp-slow", "speedy"),
            provider("sp-new", "speedy"),
            provider("sp-fast", "speedy"),
        ]);
        crate::gateway::speed::record("sp-slow", "speedy", Duration::from_millis(3000));
        crate::gateway::speed::record("sp-fast", "speedy", Duration::from_millis(400));
        gateway
            .groups
            .push(group("fastest", GroupMode::Fast, vec![member("speedy")]));
        let order: Vec<String> = routed(&gateway, "group/fastest")
            .into_iter()
            .map(|(relay, _)| relay)
            .collect();
        assert_eq!(order, ["sp-new", "sp-fast", "sp-slow"], "没测过的先试一次");

        // 没测过的试了一次失败：排到测过、能用的成员后面，不会每次都先试它。
        crate::gateway::speed::record_failure("sp-new", "speedy");
        let order: Vec<String> = routed(&gateway, "group/fastest")
            .into_iter()
            .map(|(relay, _)| relay)
            .collect();
        assert_eq!(order, ["sp-fast", "sp-slow", "sp-new"]);
    }

    #[test]
    fn codex_lists_shared_models_under_every_provider() {
        let mut gateway = gateway("deepseek-v4-pro");
        // 关了自动建组：只有各家固定的那份。
        gateway.auto_models = false;
        let binding = gateway.binding(&ToolId::Claude).unwrap().clone();
        assert!(
            !codex_pool_models(&gateway, &binding)
                .iter()
                .any(|model| model == "deepseek-v4-pro"),
            "关着时不列自动那份"
        );
        gateway.auto_models = true;
        let binding = gateway.binding(&ToolId::Claude).unwrap().clone();
        let models = codex_pool_models(&gateway, &binding);
        assert_eq!(
            models[..5],
            [
                "ds/deepseek-v4-pro".to_string(),
                "deepseek-v4-flash".to_string(),
                "kimi-k3".to_string(),
                "mirror/deepseek-v4-pro".to_string(),
                "deepseek-v4-pro".to_string(),
            ],
            "两家都有的 deepseek-v4-pro：各家名下各列一份固定的，最后一份裸名是自动切换入口"
        );
        let relays = binding_pool_relays(&gateway, &binding);
        let owner = |model: &str| {
            codex_model_owner(&relays, model)
                .map(|(index, upstream)| (relays[index].id.clone(), upstream))
        };
        assert_eq!(
            owner("ds/deepseek-v4-pro"),
            Some(pair("ds", "deepseek-v4-pro"))
        );
        assert_eq!(
            owner("mirror/deepseek-v4-pro"),
            Some(pair("mirror", "deepseek-v4-pro"))
        );
        assert_eq!(
            owner("deepseek-v4-pro"),
            Some(pair("ds", "deepseek-v4-pro"))
        );
        assert_eq!(owner("group/fast"), None);
        assert_eq!(serving_count(&relays, "DeepSeek-V4-Pro"), 2);
        // 固定的那份只发给那一家；裸名（自动）按池顺序依次尝试。
        assert_eq!(
            routed(&gateway, "ds/deepseek-v4-pro"),
            vec![pair("ds", "deepseek-v4-pro")]
        );
        assert_eq!(
            routed(&gateway, "mirror/deepseek-v4-pro"),
            vec![pair("mirror", "deepseek-v4-pro")]
        );
        assert_eq!(routed(&gateway, "deepseek-v4-pro").len(), 2);
    }

    #[test]
    fn codex_list_keeps_the_default_models_group_together() {
        let mut gateway = gateway("deepseek-v4-pro");
        gateway.auto_models = true;
        let binding = gateway.binding(&ToolId::Claude).unwrap().clone();
        // 默认模型是两家都有的 deepseek-v4-pro（自动切换入口）：自动那一块整体排到最前。
        let led = codex_pool_models_led_by(&gateway, &binding, "deepseek-v4-pro");
        assert_eq!(led[0], "deepseek-v4-pro");
        assert_eq!(led.len(), codex_pool_models(&gateway, &binding).len());
        // 默认模型是 kimi 独有的：kimi 那一块排最前，其余块的相对顺序不变。
        let led = codex_pool_models_led_by(&gateway, &binding, "kimi-k3");
        assert_eq!(led[0], "kimi-k3");
        assert_eq!(led[1], "ds/deepseek-v4-pro");
    }

    #[test]
    fn shared_bare_models_route_through_their_auto_group() {
        let mut gateway = gateway("deepseek-v4-pro");
        // 自动组：成员是两家各自固定的那份，按池顺序；默认依次。
        let auto = auto_groups(&gateway);
        assert_eq!(auto.len(), 1);
        assert_eq!(auto[0].id, "auto:deepseek-v4-pro");
        assert_eq!(
            auto[0]
                .members
                .iter()
                .map(|m| m.model.as_str())
                .collect::<Vec<_>>(),
            ["ds/deepseek-v4-pro", "mirror/deepseek-v4-pro"]
        );
        let plan =
            resolve_plan(&gateway, &ToolId::Claude, "deepseek-v4-pro", Some("conv")).unwrap();
        assert_eq!(
            plan.group_id.as_deref(),
            Some("auto:deepseek-v4-pro"),
            "裸名走自动组，会话固定按组记"
        );
        assert_eq!(routed(&gateway, "deepseek-v4-pro").len(), 2);

        // 手动：只发给选中的那一家，答不了也不换。
        gateway.auto_groups.insert(
            "deepseek-v4-pro".into(),
            AutoGroupSettings {
                mode: GroupMode::Manual,
                pick: "mirror/deepseek-v4-pro".into(),
                ..AutoGroupSettings::default()
            },
        );
        assert_eq!(
            routed(&gateway, "deepseek-v4-pro"),
            vec![pair("mirror", "deepseek-v4-pro")]
        );

        // 隐藏这个自动组：裸名回到按池顺序依次试，也不再出现在 Codex 的列表里。
        gateway
            .auto_groups
            .get_mut("deepseek-v4-pro")
            .unwrap()
            .hidden = true;
        assert_eq!(routed(&gateway, "deepseek-v4-pro").len(), 2);
        let binding = gateway.binding(&ToolId::Claude).unwrap().clone();
        assert!(
            !codex_pool_models(&gateway, &binding)
                .iter()
                .any(|m| m == "deepseek-v4-pro")
        );
    }

    #[test]
    fn hidden_members_leave_the_lists_but_still_route() {
        let mut gateway = gateway("deepseek-v4-pro");
        let binding = gateway.binding(&ToolId::Claude).unwrap().clone();
        // 自动组隐藏成员：各家固定的那份不再单独列出，只剩自动那份。
        gateway.auto_groups.insert(
            "deepseek-v4-pro".into(),
            AutoGroupSettings {
                hide_members: true,
                ..AutoGroupSettings::default()
            },
        );
        let models = codex_pool_models(&gateway, &binding);
        assert!(
            !models
                .iter()
                .any(|m| m == "ds/deepseek-v4-pro" || m == "mirror/deepseek-v4-pro")
        );
        assert!(models.iter().any(|m| m == "deepseek-v4-pro"));
        assert_eq!(
            routed(&gateway, "ds/deepseek-v4-pro"),
            vec![pair("ds", "deepseek-v4-pro")],
            "直接写名字仍能用"
        );
        // 自建组隐藏成员：成员里的裸模型名不再出现在 Claude / Grok 的列表里。
        gateway.groups[0].hide_members = true;
        let models = pool_models(&gateway, &binding);
        assert!(
            !models.iter().any(|m| m == "kimi-k3"),
            "fast 组的成员 kimi-k3 被隐藏：{models:?}"
        );
    }

    /// 主动限速：上限按供应商带到每个候选上；没填（或填 0）就是不限。限速满了的 Key 不从候选里去掉。
    #[test]
    fn candidates_carry_their_provider_key_limits() {
        let mut gateway = gateway("deepseek-v4-pro");
        gateway.providers[0].key_rpm = Some(10);
        gateway.providers[0].key_concurrency = Some(2);
        gateway.providers[2].key_rpm = Some(0);
        let plan = resolve_plan(&gateway, &ToolId::Claude, "deepseek-v4-pro", None).unwrap();
        let limits: Vec<(String, KeyLimits)> = plan
            .candidates
            .iter()
            .map(|candidate| (candidate.relay.id.clone(), candidate.limits))
            .collect();
        assert_eq!(
            limits,
            vec![
                (
                    "ds".to_string(),
                    KeyLimits {
                        rpm: Some(10),
                        concurrency: Some(2)
                    }
                ),
                ("mirror".to_string(), KeyLimits::default()),
            ]
        );
        let key = plan.candidates[0].key.clone();
        let _slot = key_pool::try_acquire(&key, plan.candidates[0].limits, Instant::now());
        let _second = key_pool::try_acquire(&key, plan.candidates[0].limits, Instant::now());
        let again = resolve_plan(&gateway, &ToolId::Claude, "deepseek-v4-pro", None).unwrap();
        assert_eq!(again.candidates.len(), 2, "并发满了也不算冷却");
        assert_eq!(again.cooling, None);
    }
}
