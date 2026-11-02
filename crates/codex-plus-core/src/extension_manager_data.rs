//! 管理器拓展的公开只读视图。逐字段构造，不序列化内部配置后删敏感字段；
//! 新增内部字段（包括 flatten 的未知字段）不会自动成为拓展契约。

use serde_json::{Value, json};

use crate::gateway::{
    activity::ActivitySnapshot,
    key_pool::PoolSnapshot,
    quota::ProviderQuota,
    speed::SpeedView,
    store::{GatewayGroup, GatewaySettings},
};
use crate::skills::SkillEntry;

pub fn gateway_providers(gateway: &GatewaySettings) -> Value {
    let providers: Vec<Value> = gateway.providers.iter().map(|provider| {
        let relay = &provider.relay;
        json!({
            "id": relay.id, "name": relay.name, "enabled": provider.enabled,
            "protocol": relay.protocol, "models": crate::gateway::route::relay_models(relay),
            "modelWindows": provider.model_windows, "contextWindow": provider.context_window,
            "keyRpm": provider.key_rpm, "keyConcurrency": provider.key_concurrency,
            "price": provider.price.as_ref().map(price),
            "modelPrices": provider.model_prices.iter().map(|(model, value)| (model.clone(), price(value))).collect::<serde_json::Map<String, Value>>(),
        })
    }).collect();
    json!({"providers": providers})
}

fn price(value: &crate::gateway::store::ProviderPrice) -> Value {
    json!({"currency": value.currency, "input": value.input, "cachedInput": value.cached_input, "output": value.output})
}

fn group(value: &GatewayGroup) -> Value {
    let members: Vec<Value> = value
        .members
        .iter()
        .map(|member| json!({"model": member.model, "weight": member.weight}))
        .collect();
    json!({"id": value.id, "name": value.name, "mode": value.mode, "sticky": value.sticky,
        "members": members, "hideMembers": value.hide_members, "pick": value.pick})
}

pub fn gateway_routing(gateway: &GatewaySettings) -> Value {
    let groups: Vec<Value> = gateway.groups.iter().map(group).collect();
    let auto_groups: Vec<Value> = crate::gateway::route::auto_groups(gateway).iter().map(|value| {
        let options = gateway.auto_groups.get(&value.name.to_ascii_lowercase());
        json!({"group": group(value), "hidden": options.is_some_and(|options| options.hidden), "customized": options.is_some()})
    }).collect();
    json!({"groups": groups, "autoGroups": auto_groups})
}

/// 仅展示本管理器进程已采集的快照；不探测供应商、不启动 helper、不读 Key 或错误原文。
pub fn gateway_health(
    gateway: &GatewaySettings,
    health: &PoolSnapshot,
    speeds: &[SpeedView],
    activity: &ActivitySnapshot,
) -> Value {
    let known = |id: &str| {
        gateway
            .providers
            .iter()
            .any(|provider| provider.relay.id == id)
    };
    let providers: Vec<Value> = health.providers.iter().filter(|value| known(&value.relay_id)).map(|value| {
        json!({"relayId": value.relay_id, "breakerOpen": value.breaker_open, "remainingMs": value.remaining_ms, "consecutiveErrors": value.consecutive_errors})
    }).collect();
    let speeds: Vec<Value> = speeds.iter().filter(|value| known(&value.relay_id)).map(|value| {
        json!({"relayId": value.relay_id, "model": value.model, "ms": value.ms, "samples": value.samples})
    }).collect();
    let agents: Vec<Value> = activity.agents.iter().map(|value| {
        json!({"agent": value.agent, "requests": value.requests, "rerouted": value.rerouted,
            "errors": value.errors, "inFlight": value.in_flight.iter().filter(|id| known(id)).collect::<Vec<_>>()})
    }).collect();
    json!({"scope": "managerProcess", "providers": providers, "speeds": speeds, "agents": agents})
}

/// 只取已有额度缓存，不访问上游。未采集到的额度必须是 null，不能当作余额为零。
pub fn gateway_quota(
    gateway: &GatewaySettings,
    get: impl Fn(&str) -> Option<ProviderQuota>,
) -> Value {
    let providers: Vec<Value> = gateway.providers.iter().map(|provider| {
        let quota = get(&provider.relay.id).map(|quota| {
            let windows: Vec<Value> = quota.windows.iter().map(|window| json!({
                "windowMinutes": window.window_minutes, "usedPercent": window.used_percent, "resetsAtMs": window.resets_at_ms,
            })).collect();
            json!({"windows": windows, "updatedAtMs": quota.updated_at_ms})
        });
        json!({"id": provider.relay.id, "name": provider.relay.name, "quota": quota})
    }).collect();
    json!({"scope": "managerProcess", "cachedOnly": true, "providers": providers})
}

pub fn installed_skills(entries: &[SkillEntry]) -> Value {
    let skills: Vec<Value> = entries.iter().filter(|entry| entry.installed).map(|entry| {
        json!({"id": entry.id, "name": entry.name, "enabled": entry.enabled, "bundled": entry.bundled,
            "managed": entry.managed, "agents": entry.agents, "contentHash": entry.content_hash})
    }).collect();
    json!({"skills": skills})
}
