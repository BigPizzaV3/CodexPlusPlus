//! 网关的路由：Agent 绑定 → 模型名 → 候选上游（供应商 × Key）列表。
//!
//! 只看网关自己的供应商池和路由组（`gateway.json`），与直连模式的供应商无关。模型名有三种写法：
//! - 裸模型名：池里所有提供它的供应商按池顺序依次尝试；
//! - `<供应商 id>/<模型>`：只发给这个供应商；
//! - `group/<组 id>`：按路由组的成员与模式展开，成员也可以是另一个组。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};

use crate::gateway::key_pool::{self, KeyRef};
use crate::gateway::store::{
    AgentBinding, GROUP_MODEL_PREFIX, GatewayGroup, GatewaySettings, GroupMember, GroupMode,
    UnknownModelPolicy,
};
use crate::settings::RelayProfile;
use crate::tools::ToolId;

/// 嵌套路由组最多展开几层，防止组互相引用时死循环。
const MAX_GROUP_DEPTH: usize = 4;

/// 一个可尝试的上游：供应商（`api_key` 已换成该 Key）+ 发给它的模型名。
#[derive(Debug, Clone)]
pub struct GatewayCandidate {
    pub relay: RelayProfile,
    pub key: KeyRef,
    pub upstream_model: String,
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

/// Agent 能用的全部模型：池内供应商的模型（去重，按池顺序），再加上有成员的路由组 `group/<id>`。
pub fn pool_models(gateway: &GatewaySettings, binding: &AgentBinding) -> Vec<String> {
    let mut models: Vec<String> = Vec::new();
    for relay in binding_pool_relays(gateway, binding) {
        for model in relay_models(relay) {
            if !models
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
fn qualified_provider<'a>(
    gateway: &'a GatewaySettings,
    binding: &AgentBinding,
    model: &str,
) -> Option<(&'a RelayProfile, String)> {
    if model.starts_with(GROUP_MODEL_PREFIX) {
        return None;
    }
    let (prefix, rest) = model.split_once('/')?;
    if rest.trim().is_empty() {
        return None;
    }
    binding_pool_relays(gateway, binding)
        .into_iter()
        .find(|relay| relay.id.eq_ignore_ascii_case(prefix))
        .map(|relay| (relay, rest.trim().to_string()))
}

fn group_by_model<'a>(gateway: &'a GatewaySettings, model: &str) -> Option<&'a GatewayGroup> {
    let id = model.strip_prefix(GROUP_MODEL_PREFIX)?;
    gateway
        .groups
        .iter()
        .find(|group| group.id.eq_ignore_ascii_case(id))
}

/// 一个模型名展开成 (供应商, 发给它的模型) 列表。
fn expand_model(
    gateway: &GatewaySettings,
    binding: &AgentBinding,
    model: &str,
    depth: usize,
    visiting: &mut Vec<String>,
) -> Vec<(RelayProfile, String)> {
    if let Some(group) = group_by_model(gateway, model) {
        return expand_group(gateway, binding, group, depth, visiting);
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

/// 路由组展开：按模式决定从哪个成员开始，成员依次展开（嵌套组递归，出现环或太深时跳过）。
fn expand_group(
    gateway: &GatewaySettings,
    binding: &AgentBinding,
    group: &GatewayGroup,
    depth: usize,
    visiting: &mut Vec<String>,
) -> Vec<(RelayProfile, String)> {
    if depth >= MAX_GROUP_DEPTH || visiting.contains(&group.id) {
        return Vec::new();
    }
    visiting.push(group.id.clone());
    let mut relays: Vec<(RelayProfile, String)> = Vec::new();
    for member in ordered_members(group) {
        for (relay, model) in
            expand_model(gateway, binding, member.model.trim(), depth + 1, visiting)
        {
            if !relays
                .iter()
                .any(|(known, known_model)| known.id == relay.id && *known_model == model)
            {
                relays.push((relay, model));
            }
        }
    }
    visiting.pop();
    relays
}

/// 按路由组模式排出成员的尝试顺序。轮换 / 权重的进度存在内存里，各组独立。
fn ordered_members(group: &GatewayGroup) -> Vec<GroupMember> {
    let members = group.members.clone();
    if members.len() < 2 {
        return members;
    }
    let start = match group.mode {
        GroupMode::InOrder => 0,
        GroupMode::Rotate => (next_counter(&group.id) % members.len() as u64) as usize,
        GroupMode::Weighted => {
            let total: u64 = members
                .iter()
                .map(|member| u64::from(member.weight.max(1)))
                .sum();
            let mut slot = next_counter(&group.id) % total.max(1);
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

fn next_counter(group_id: &str) -> u64 {
    static COUNTERS: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();
    let mut guard = COUNTERS
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let counter = guard.entry(group_id.to_string()).or_default();
    let value = *counter;
    *counter = counter.wrapping_add(1);
    value
}

/// 按绑定解析出本次请求的候选上游。
pub fn resolve_plan(
    gateway: &GatewaySettings,
    agent: &ToolId,
    requested_model: &str,
    conversation: Option<&str>,
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

    let group_id = group_by_model(gateway, &model).map(|group| group.id.clone());
    let mut relays = expand_model(gateway, binding, &model, 0, &mut Vec::new());
    if relays.is_empty() && group_id.is_none() {
        // 原样转发的未知模型：交给默认模型所在的供应商。
        relays = expand_model(gateway, binding, &default_model, 0, &mut Vec::new())
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
        let (keys, soonest) = key_pool::order_keys(
            &relay,
            gateway.key_policy.mode,
            conversation.as_deref(),
            now,
        );
        if let Some(soonest) = soonest {
            cooling = Some(cooling.map_or(soonest, |current| current.min(soonest)));
        }
        for key in keys {
            candidates.push(GatewayCandidate {
                relay: key.relay,
                key: key.key,
                upstream_model: upstream_model.clone(),
            });
        }
    }
    Ok(RoutePlan {
        agent: agent.clone(),
        source_model: requested_model.trim().to_string(),
        model,
        group_id,
        conversation,
        candidates,
        cooling,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::store::GatewayProvider;
    use crate::settings::RelayMode;

    fn provider(id: &str, models: &str) -> GatewayProvider {
        GatewayProvider {
            enabled: true,
            source: String::new(),
            price: None,
            proxy: String::new(),
            relay: RelayProfile {
                id: id.to_string(),
                name: id.to_string(),
                relay_mode: RelayMode::PureApi,
                base_url: format!("https://{id}.test/v1"),
                api_key: format!("sk-{id}"),
                model_list: models.to_string(),
                ..RelayProfile::default()
            },
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
            members,
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
}
