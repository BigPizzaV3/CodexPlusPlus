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

use crate::gateway::key_pool::{self, KeyRef};
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
    pub candidates: Vec<GatewayCandidate>,
    /// 候选为空且是因为 Key 都在冷却/熔断时，最近一个恢复的等待时间。
    pub cooling: Option<Duration>,
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
    let requested = requested.trim();
    if requested.is_empty() {
        return default_model.to_string();
    }
    // 审批模型（#2703）：codex-auto-review 改发到绑定指定的模型，优先于目录匹配与未知模型策略
    // （池里的 ChatGPT 账号可能把 codex-auto-review 列进模型表，那样会直接落到订阅额度上）。
    if requested == crate::aggregate_catalog::CODEX_AUTO_REVIEW_MODEL {
        let (review, _) = crate::model_suffix::parse_model_suffix(binding.auto_review_model.trim());
        let review = review.trim();
        if !review.is_empty() {
            return review.to_string();
        }
    }
    let (requested_slug, _) = crate::model_suffix::parse_model_suffix(requested);
    let declared = binding.declared_models();
    if declared
        .iter()
        .any(|model| model.eq_ignore_ascii_case(&requested_slug))
        || model_is_routable(gateway, binding, &requested_slug)
    {
        return requested_slug;
    }
    match binding.unknown_model {
        UnknownModelPolicy::MapToDefault if !default_model.is_empty() => default_model.to_string(),
        _ => requested_slug,
    }
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
    binding_pool_relays(gateway, binding)
        .into_iter()
        .filter(|relay| serves(relay, model))
        .map(|relay| (relay.clone(), model.to_string()))
        .collect()
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

/// (组 id, 会话) → (供应商 id, 模型, 记下的时间, 这条的有效期)。
type StickyTable = HashMap<(String, String), (String, String, Instant, Duration)>;

fn sticky_table() -> &'static Mutex<StickyTable> {
    static TABLE: OnceLock<Mutex<StickyTable>> = OnceLock::new();
    TABLE.get_or_init(Mutex::default)
}

/// 记下这个会话在组里是哪个上游回答的。没有组、组不固定或没有会话标识时不记。
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
    table.insert(
        key,
        (
            candidate.relay.id.clone(),
            candidate.upstream_model.clone(),
            now,
            ttl,
        ),
    );
}

/// 表满了：先按各条自己的有效期清掉过期的；还多就从最久没用的开始淘汰，清到上限的 3/4，
/// 之后一段时间的请求不用再扫表。
fn trim_sticky(table: &mut StickyTable, now: Instant) {
    table.retain(|_, (_, _, at, ttl)| now.duration_since(*at) < *ttl);
    if table.len() < STICKY_TRIM_TO {
        return;
    }
    let mut ages: Vec<Instant> = table.values().map(|(_, _, at, _)| *at).collect();
    ages.sort_unstable();
    // 比这一刻早（或同时）记下的都淘汰，剩下的不超过 STICKY_TRIM_TO 条。
    let cutoff = ages[table.len() - STICKY_TRIM_TO];
    table.retain(|_, (_, _, at, _)| *at > cutoff);
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
    let (relay, model, at, _) = table.get(&(group.id.clone(), conversation.to_string()))?;
    (now.duration_since(*at) < ttl).then(|| (relay.clone(), model.clone()))
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
    let model = effective_model(gateway, binding, &default_model, requested_model);
    let conversation = conversation
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    let group = group_for(gateway, &model);
    let group_id = group.as_ref().map(|group| group.id.clone());
    let sticky_ttl = group.as_ref().and_then(|group| sticky_ttl(group.sticky));
    let mut relays = expand_model(gateway, binding, &model, 0, &mut Vec::new(), advance);
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

    let now = Instant::now();
    let mut candidates = Vec::new();
    let mut cooling: Option<Duration> = None;
    for (relay, upstream_model) in relays {
        let window = gateway
            .provider(&relay.id)
            .and_then(|provider| provider.window_for(&upstream_model));
        let (keys, soonest) = key_pool::order_keys_with(
            &relay,
            gateway.key_policy.mode,
            conversation.as_deref(),
            now,
            advance,
        );
        if let Some(soonest) = soonest {
            cooling = Some(cooling.map_or(soonest, |current| current.min(soonest)));
        }
        for key in keys {
            candidates.push(GatewayCandidate {
                relay: key.relay,
                key: key.key,
                upstream_model: upstream_model.clone(),
                window,
            });
        }
    }
    Ok(RoutePlan {
        agent: agent.clone(),
        source_model: requested_model.trim().to_string(),
        model,
        group_id,
        conversation,
        sticky_ttl,
        candidates,
        cooling,
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
            upstream_new: Vec::new(),
            upstream_gone: Vec::new(),
            models_checked_at_ms: 0,
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
            ("r".to_string(), "m".to_string(), start, CACHE_STICKY_TTL),
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
            }],
            updated_at_ms: 0,
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
}
