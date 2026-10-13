//! 按模型聚合：发布带供应商身份的模型目录，并精确解析到成员与真实模型。
//! 模型聚合和供应商池策略互斥；未带绑定身份的请求不能进入轮转池。
//!
//! 解析规则对齐 opencodex / cc-switch 的聚合模式：
//! - `cpx-<成员>/<模型>` 按成员前缀精确归因；解析不出来（成员被移除、模型被移出列表、
//!   成员 id 变过）一律报错，不按模型名改派、不落到默认供应商；
//! - 不带 `cpx-` 前缀的请求（旧会话裸名、别的目录写下的 `<前缀>/<模型>`、回合前自动压缩
//!   带的上一回合模型名）交给聚合的「默认供应商」，按请求里的 id 原样转发；没设默认就报错。

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::Context;
use serde_json::{json, Map, Value};

use crate::model_suffix::{
    build_model_catalog_json_with_capabilities, collect_catalog_entries,
    migrate_model_list_with_suffixes, parse_compact_percent, parse_window_token, ModelCatalogEntry,
};
use crate::settings::{
    AggregateRelayProfile, BackendSettings, RelayMode, RelayProfile, SettingsStore,
};

pub const AGGREGATE_MODEL_PREFIX: &str = "cpx-";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AggregateModelBinding {
    pub catalog_model: String,
    pub relay_id: String,
    pub upstream_model: String,
    pub relay: RelayProfile,
}

/// 旧配置没有开关时默认关闭；只有活动聚合发布这些模型。
pub fn active_model_aggregation(settings: &BackendSettings) -> Option<AggregateRelayProfile> {
    settings
        .active_aggregate_relay_profile()
        .filter(|aggregate| aggregate.model_aggregation_enabled)
}

/// 使用持久化 relay id，而不是供应商名称。转义保持一一对应，重命名与重排不会换家。
pub fn aggregate_model_id(relay_id: &str, upstream_model: &str) -> String {
    let mut key = String::new();
    for byte in relay_id.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_') {
            key.push(byte as char);
        } else {
            key.push_str(&format!("%{byte:02X}"));
        }
    }
    format!("{AGGREGATE_MODEL_PREFIX}{key}/{}", upstream_model.trim())
}

/// 无分隔符的 cpx-* 仍可作为普通上游模型名；带分隔符的是保留的聚合 ID。
pub fn is_aggregate_model(model: &str) -> bool {
    model.starts_with(AGGREGATE_MODEL_PREFIX) && model.contains('/')
}

pub fn active_model_bindings(
    settings: &BackendSettings,
) -> anyhow::Result<Vec<AggregateModelBinding>> {
    let Some(aggregate) = active_model_aggregation(settings) else {
        return Ok(Vec::new());
    };
    let mut bindings = Vec::new();
    for relay in member_profiles(settings, &aggregate)? {
        for entry in profile_catalog_entries(relay)? {
            bindings.push(AggregateModelBinding {
                catalog_model: aggregate_model_id(&relay.id, &entry.slug),
                relay_id: relay.id.clone(),
                upstream_model: entry.slug,
                relay: relay.clone(),
            });
        }
    }
    Ok(bindings)
}

pub fn resolve_active_model(
    settings: &BackendSettings,
    catalog_model: &str,
) -> anyhow::Result<Option<AggregateModelBinding>> {
    let catalog_model = catalog_model.trim();
    if catalog_model.is_empty() {
        return Ok(None);
    }
    let Some(aggregate) = active_model_aggregation(settings) else {
        if is_aggregate_model(catalog_model) {
            anyhow::bail!("聚合模型不存在、已被移除或按模型聚合未开启：{catalog_model}");
        }
        return Ok(None);
    };
    if !is_aggregate_model(catalog_model) {
        // 不带前缀的请求一律交给「默认供应商」，且按请求里的 id 原样转发。
        // 对齐 opencodex / cc-switch 的聚合规则：解析不出来就硬报错，不做按模型名的静默改派
        // ——改派会把请求花到用户没选的那家，用户看不出来（cc-switch `mode/stack.rs` 同款理由）。
        if let Some(binding) = resolve_by_default_provider(settings, &aggregate, catalog_model)? {
            return Ok(Some(binding));
        }
        anyhow::bail!(
            "模型「{catalog_model}」不是聚合模型，且聚合没有可用的默认供应商：请在 Codex 模型列表里选择 cpx- 开头的模型，或到聚合供应商里把一家成员设为「默认供应商」"
        );
    }
    // 按成员前缀精确归因：缺 Key/URL、官方未登录、成员被删只影响它自己的模型，
    // 其他成员的模型继续可用。过去任一成员未就绪会让全部聚合请求 400。
    for member in aggregate_member_availability(settings, &aggregate) {
        if !catalog_model.starts_with(&member.prefix) {
            continue;
        }
        if let Some(issue) = &member.issue {
            anyhow::bail!(
                "聚合成员「{}」当前不可用：{issue}。请修复该成员，或改选其他模型",
                member.name
            );
        }
        let Some(relay_index) = member.relay_index else {
            anyhow::bail!("聚合成员「{}」当前不可用：成员供应商不存在", member.name);
        };
        let relay = &settings.relay_profiles[relay_index];
        if let Some(upstream_model) = profile_catalog_entries(relay)?
            .into_iter()
            .find(|entry| aggregate_model_id(&relay.id, &entry.slug) == catalog_model)
            .map(|entry| entry.slug)
        {
            return Ok(Some(AggregateModelBinding {
                catalog_model: catalog_model.to_string(),
                relay_id: relay.id.clone(),
                upstream_model,
                relay: relay.clone(),
            }));
        }
        // 成员还在、但已经不提供这个模型（模型被移出列表）：按 cc-switch 的规则硬报错，
        // 不改派给别的成员、也不落到默认供应商——那等于替用户换了供应商。
        anyhow::bail!("聚合模型不存在、已被移除或按模型聚合未开启：{catalog_model}");
    }
    // 带 `cpx-` 前缀但没有任何成员前缀命中：成员 id 变过（目录重建）等残留 ID。
    // 同样硬报错，不按模型名改派、不交默认供应商（默认供应商只接不带前缀的请求）。
    anyhow::bail!("聚合模型不存在、已被移除或按模型聚合未开启：{catalog_model}")
}

/// 默认供应商：承接所有**不带 `cpx-` 前缀**的请求（旧会话的裸名、别的目录写下的
/// `<前缀>/<模型>`、回合前自动压缩带的上一回合模型名），按请求里的 id **原样**转发。
/// 未设置默认时返回 Ok(None)，由调用方给出可操作的报错；默认配置存在但不可用时
/// 点名默认供应商，避免请求被静默吞掉。
fn resolve_by_default_provider(
    settings: &BackendSettings,
    aggregate: &AggregateRelayProfile,
    requested: &str,
) -> anyhow::Result<Option<AggregateModelBinding>> {
    let default_relay_id = aggregate.default_relay_id.trim();
    if default_relay_id.is_empty() {
        return Ok(None);
    }
    // 默认必须是当前成员；配置漂移（成员被移出成员列表）时不接管请求，落回原有报错。
    if !aggregate
        .members
        .iter()
        .any(|member| member.relay_id == default_relay_id)
    {
        return Ok(None);
    }
    let Some(relay_index) = settings
        .relay_profiles
        .iter()
        .position(|relay| relay.id == default_relay_id)
    else {
        anyhow::bail!(
            "默认供应商当前不可用：成员供应商不存在（{default_relay_id}），请检查聚合配置"
        );
    };
    let relay = &settings.relay_profiles[relay_index];
    if !crate::relay_rotation::member_relay_ready(relay) {
        anyhow::bail!(
            "默认供应商「{}」当前不可用：需要可用的 API 地址与认证配置，或完成官方登录",
            relay.name
        );
    }
    // 原样转发请求里的 id（对齐 opencodex：走 `defaultProvider` 时保留请求的 model id）。
    // 不去前缀、不剥窗口后缀：静默改名会让用户看不出请求被换成了别家的哪个模型。
    let upstream_model = requested.trim().to_string();
    let _ = crate::diagnostic_log::append_diagnostic_log(
        "aggregate.default_relay_resolved",
        json!({
            "requestedModel": requested,
            "relayId": relay.id,
            "relayName": relay.name,
            "upstreamModel": upstream_model,
        }),
    );
    Ok(Some(AggregateModelBinding {
        catalog_model: requested.to_string(),
        relay_id: relay.id.clone(),
        upstream_model,
        relay: relay.clone(),
    }))
}

/// 默认供应商配置有效（指向当前成员）时，解析不到的模型名都能兜底转发；
/// 供「最近使用」模型清理判断：此时任何目录外的名字都还有可能被服务，不再按目录清理。
pub fn default_backs_unbound_models(aggregate: &AggregateRelayProfile) -> bool {
    let default_relay_id = aggregate.default_relay_id.trim();
    !default_relay_id.is_empty()
        && aggregate
            .members
            .iter()
            .any(|member| member.relay_id == default_relay_id)
}

/// 目录生成/应用期的成员校验：普通 API 成员要 URL/Key 齐备，
/// 官方登录成员要有 ChatGPT 登录凭据；另一个聚合不能当成员。
/// 请求期由 `resolve_active_model` 按成员前缀各自归因，单个成员失效不连坐其他模型。
fn member_profiles<'a>(
    settings: &'a BackendSettings,
    aggregate: &AggregateRelayProfile,
) -> anyhow::Result<Vec<&'a RelayProfile>> {
    let mut members = Vec::new();
    let mut seen = HashSet::new();
    for member in &aggregate.members {
        if !seen.insert(&member.relay_id) {
            continue;
        }
        let relay = settings
            .relay_profiles
            .iter()
            .find(|relay| relay.id == member.relay_id)
            .with_context(|| format!("聚合成员供应商不存在：{}", member.relay_id))?;
        if !crate::relay_rotation::member_relay_ready(relay) {
            anyhow::bail!(
                "聚合成员「{}」需要可用的 API 地址与认证配置，或完成官方登录",
                relay.name
            );
        }
        members.push(relay);
    }
    Ok(members)
}

/// 请求路径的成员可用性视图：只保留模型 ID 前缀与不可用原因，避免为解析
/// 单个模型而遍历和克隆全部成员配置。
struct MemberAvailability {
    relay_index: Option<usize>,
    prefix: String,
    name: String,
    issue: Option<String>,
}

/// 聚合模型 ID 的成员前缀：`cpx-<成员键>/`。键转义与 `aggregate_model_id` 一致，
/// 所以除本成员外没有其他成员的前缀能命中同一模型 ID。
fn member_model_prefix(relay_id: &str) -> String {
    aggregate_model_id(relay_id, "")
}

fn aggregate_member_availability(
    settings: &BackendSettings,
    aggregate: &AggregateRelayProfile,
) -> Vec<MemberAvailability> {
    let mut seen = HashSet::new();
    aggregate
        .members
        .iter()
        .filter(|member| seen.insert(member.relay_id.as_str()))
        .map(|member| {
            let Some(relay_index) = settings
                .relay_profiles
                .iter()
                .position(|relay| relay.id == member.relay_id)
            else {
                return MemberAvailability {
                    relay_index: None,
                    prefix: member_model_prefix(&member.relay_id),
                    name: member.relay_id.clone(),
                    issue: Some("成员供应商不存在，请检查成员配置".to_string()),
                };
            };
            let relay = &settings.relay_profiles[relay_index];
            MemberAvailability {
                relay_index: Some(relay_index),
                prefix: member_model_prefix(&relay.id),
                name: relay.name.clone(),
                issue: (!crate::relay_rotation::member_relay_ready(relay))
                    .then(|| "需要可用的 API 地址与认证配置，或完成官方登录".to_string()),
            }
        })
        .collect()
}

pub(crate) fn profile_catalog_entries(
    profile: &RelayProfile,
) -> anyhow::Result<Vec<ModelCatalogEntry>> {
    let mut windows = parse_string_map(&profile.model_windows, "model_windows")?;
    validate_windows(&windows, "model_windows")?;
    let (list, suffix_windows) = migrate_model_list_with_suffixes(&profile.model_list);
    for (slug, window) in suffix_windows {
        windows.entry(slug).or_insert(window);
    }
    let compact = parse_string_map(&profile.model_auto_compact, "model_auto_compact")?;
    validate_compact(&compact, "model_auto_compact")?;
    Ok(collect_catalog_entries(
        &list,
        &windows,
        &compact,
        &crate::relay_config::relay_profile_model(profile),
    ))
}

/// 每个成员只构建一次目录，然后修改身份字段；先按真实模型应用元数据，
/// 避免前缀让 GPT/DeepSeek 的能力匹配失效。
///
/// 发布形态对齐 opencodex / cc-switch：**默认成员的模型用裸名发布**（不带前缀的请求本来就
/// 归它，客户端列表里"选到哪家"一目了然），其他成员仍用 `cpx-<成员id>/<模型>` 绑定身份。
pub fn build_catalog_json(
    settings: &BackendSettings,
    aggregate: &AggregateRelayProfile,
) -> anyhow::Result<String> {
    let default_relay_id = aggregate.default_relay_id.trim();
    let mut models = Vec::new();
    for relay in member_profiles(settings, aggregate)? {
        let payload = build_member_catalog(relay)?;
        let is_default_member = !default_relay_id.is_empty() && relay.id.as_str() == default_relay_id;
        // 分组的展示名：成员没起名字时回落 relay id，避免前端渲染出无名分组表头。
        let group_name = if relay.name.trim().is_empty() {
            relay.id.clone()
        } else {
            relay.name.trim().to_string()
        };
        for mut model in payload["models"].as_array().into_iter().flatten().cloned() {
            let upstream = model["slug"].as_str().unwrap_or_default().to_string();
            let display = model["display_name"]
                .as_str()
                .unwrap_or(&upstream)
                .to_string();
            model["slug"] = if is_default_member {
                json!(upstream)
            } else {
                json!(aggregate_model_id(&relay.id, &upstream))
            };
            // 默认成员连显示名也保持裸名：它就是"默认"，再缀「（供应商）」只会让人以为
            // 还得再挑一次供应商。其他成员保留后缀，避免同名模型分不清是哪一家。
            model["display_name"] = if is_default_member {
                json!(display)
            } else {
                json!(format!("{display}（{}）", relay.name))
            };
            model["description"] = json!(format!("{upstream} · {}", relay.name));
            // 供应商分组身份（前端模型选择器按它把连续模型折叠成一组）：
            // - `codex_plus_group_id` 是权威分组键，用持久化 relay id；成员已按 relay_id
            //   去重（member_profiles），同一家不会产生两组，同名成员也各自成组；
            // - `codex_plus_group` 只用于展示，默认成员同样要写，否则它的模型没有归属；
            // - `codex_plus_default` 标出默认成员：只有前端能据此打「默认」标志。
            model["codex_plus_group"] = json!(group_name);
            model["codex_plus_group_id"] = json!(relay.id);
            if is_default_member {
                model["codex_plus_default"] = json!(true);
            }
            model["comp_hash"] = json!("codex-plus-aggregate");
            // 聚合会切换上游；经典子代理任务使用明文，避免跨成员解密失败。
            model["multi_agent_version"] = json!("v1");
            model["use_responses_lite"] = json!(false);
            model["visibility"] = json!("list");
            model["priority"] = json!(1000 + models.len());
            models.push(model);
        }
    }
    if models.is_empty() {
        anyhow::bail!("按模型聚合至少需要一个成员模型，请先填写成员供应商的模型列表");
    }
    Ok(serde_json::to_string_pretty(&json!({ "models": models }))?)
}

/// apply 流程只拿到活动 RelayProfile；从持久化 settings 取出成员引用，
/// 旧的直接 apply 调用若没有开启该开关则返回 None，保持原有目录行为。
pub fn build_catalog_for_profile(profile: &RelayProfile) -> anyhow::Result<Option<String>> {
    if profile.relay_mode != RelayMode::Aggregate {
        return Ok(None);
    }
    let settings = SettingsStore::default().load()?;
    let Some(aggregate) = settings
        .aggregate_relay_profiles
        .iter()
        .find(|aggregate| aggregate.id == profile.id && aggregate.model_aggregation_enabled)
    else {
        return Ok(None);
    };
    Ok(Some(build_catalog_json(&settings, aggregate)?))
}

/// 注入层读取与原生 Codex 相同的目录条目，显示名与能力仍属于对应成员。
pub fn catalog_summary(
    home: &Path,
    settings: &BackendSettings,
    aggregate: &AggregateRelayProfile,
) -> anyhow::Result<Value> {
    let payload: Value = serde_json::from_str(&build_catalog_json(settings, aggregate)?)?;
    let mut metadata = Map::new();
    let models = payload["models"]
        .as_array()
        .context("聚合目录缺少 models")?;
    let mut ids = Vec::new();
    for model in models {
        let id = model["slug"].as_str().context("聚合目录缺少 slug")?;
        ids.push(id.to_string());
        let ui = crate::model_suffix::model_ui_metadata_with_override(id, Some(model))
            .unwrap_or_else(|| json!({}));
        metadata.insert(id.to_string(), ui);
    }
    let profile = settings
        .relay_profiles
        .iter()
        .find(|profile| profile.id == aggregate.id)
        .context("聚合供应商不存在")?;
    Ok(json!({
        "status": "ok",
        "path": home.join("config.toml").to_string_lossy(),
        "service_tier": Value::Null,
        "model": ids.first().cloned().unwrap_or_default(),
        "model_provider": aggregate.id,
        "codex_model_provider": crate::model_catalog::codex_model_provider_for_relay_profile(home, profile),
        "provider_name": aggregate.name,
        // 默认成员的 relay id：模型选择器用它给默认那一组打「默认」标志。
        "codex_plus_default_group_id": aggregate.default_relay_id.trim(),
        "default_model": ids.first().cloned().unwrap_or_default(),
        "models": ids,
        "modelMetadata": metadata,
        "sources": [{
            "id": format!("aggregate:{}", aggregate.id),
            "type": "aggregate_model_catalog",
            "name": aggregate.name,
            "base_url": "",
            "status": "ok",
            "models": models.len(),
            "responses_api": { "status": "unknown" }
        }],
        "responses_api": { "status": "unknown" }
    }))
}

fn build_member_catalog(profile: &RelayProfile) -> anyhow::Result<Value> {
    let entries = profile_catalog_entries(profile)?;
    if entries.is_empty() {
        return Ok(json!({ "models": [] }));
    }
    let fallback = parse_window_token(&profile.context_window);
    let catalog_json =
        build_model_catalog_json_with_capabilities(&entries, fallback, None, Some(false), false);
    let metadata = crate::relay_config::parse_model_metadata_map(&profile.model_metadata)?;
    let catalog_json =
        crate::relay_config::apply_model_metadata_overrides(&catalog_json, &metadata)?;
    Ok(serde_json::from_str(&catalog_json)?)
}

fn parse_string_map(raw: &str, field: &str) -> anyhow::Result<HashMap<String, String>> {
    if raw.trim().is_empty() {
        return Ok(HashMap::new());
    }
    let value: Value = serde_json::from_str(raw)
        .map_err(|error| anyhow::anyhow!("{field} JSON 解析失败：{error}"))?;
    let object = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("{field} 必须是 JSON 对象"))?;
    object
        .iter()
        .map(|(key, value)| {
            let value = value
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("{field} 的模型 {key} 值必须是字符串"))?;
            Ok((key.clone(), value.to_string()))
        })
        .collect()
}

fn validate_windows(values: &HashMap<String, String>, field: &str) -> anyhow::Result<()> {
    for (model, value) in values {
        if parse_window_token(value).is_none() {
            anyhow::bail!("{field} 的模型 {model} 窗口值无效：{value}");
        }
    }
    Ok(())
}

fn validate_compact(values: &HashMap<String, String>, field: &str) -> anyhow::Result<()> {
    for (model, value) in values {
        if parse_compact_percent(value).is_none() {
            anyhow::bail!("{field} 的模型 {model} 百分比无效：{value}");
        }
    }
    Ok(())
}

/// codex 对 `features.multi_agent_v2` 的启用判定优先于目录条目的
/// `multi_agent_version`（openai/codex config schema 原文：
/// “An enabled `features.multi_agent_v2` setting takes precedence.”）。
/// 模型聚合把子代理钉在经典工具（v1、明文任务）上以保证跨成员可用，
/// 这里识别会覆盖该契约的全局开关形态：布尔值以原值为准，
/// `[features.multi_agent_v2]` 结构化子表出现即视为启用（仅显式 `enabled = false` 视为关闭）。
pub fn multi_agent_v2_enabled(config_text: &str) -> bool {
    let Ok(doc) = config_text.parse::<toml_edit::DocumentMut>() else {
        return false;
    };
    let Some(features) = doc.get("features").and_then(|item| item.as_table_like()) else {
        return false;
    };
    let Some(item) = features.get("multi_agent_v2") else {
        return false;
    };
    if let Some(enabled) = item.as_value().and_then(|value| value.as_bool()) {
        return enabled;
    }
    item.as_table_like().is_some_and(|table| {
        table
            .get("enabled")
            .and_then(|value| value.as_bool())
            .unwrap_or(true)
    })
}

/// 模型聚合处于活动状态、且用户用 `[features] multi_agent_v2` 覆盖了经典子代理设置时，
/// 返回一条可展示给管理器的提示；无冲突或读取失败时返回 None。
pub fn subagent_conflict_warning(home: &Path, settings: &BackendSettings) -> Option<String> {
    active_model_aggregation(settings)?;
    let config_text = std::fs::read_to_string(home.join("config.toml")).ok()?;
    multi_agent_v2_enabled(&config_text).then(|| {
        "检测到 config.toml 已启用 [features] multi_agent_v2：它优先于模型聚合目录中的经典子代理设置，跨供应商的子代理任务可能报无法解密。如遇到该问题，请移除或关闭该项，或让子代理与主代理使用同一供应商的模型。"
            .to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{AggregateRelayMember, AggregateRelayStrategy, RelayMode};

    fn settings() -> BackendSettings {
        let mut settings = BackendSettings::default();
        let deepseek = RelayProfile {
            id: "deepseek".to_string(),
            name: "DeepSeek".to_string(),
            model: "deepseek-chat".to_string(),
            model_list: "deepseek-reasoner".to_string(),
            model_windows: r#"{"deepseek-chat":"128K","deepseek-reasoner":"1M"}"#.to_string(),
            base_url: "https://deepseek.example/v1".to_string(),
            api_key: "sk-deepseek".to_string(),
            relay_mode: RelayMode::PureApi,
            ..RelayProfile::default()
        };
        let openai = RelayProfile {
            id: "openai".to_string(),
            name: "OpenAI API".to_string(),
            model: "gpt-5.6-sol".to_string(),
            base_url: "https://openai.example/v1".to_string(),
            api_key: "sk-openai".to_string(),
            relay_mode: RelayMode::PureApi,
            ..RelayProfile::default()
        };
        let aggregate_profile = RelayProfile {
            id: "agg".to_string(),
            name: "模型聚合".to_string(),
            relay_mode: RelayMode::Aggregate,
            ..RelayProfile::default()
        };
        settings.relay_profiles = vec![deepseek, openai, aggregate_profile];
        settings.active_relay_id = "agg".to_string();
        settings.active_aggregate_relay_id = "agg".to_string();
        settings.aggregate_relay_profiles = vec![AggregateRelayProfile {
            id: "agg".to_string(),
            name: "模型聚合".to_string(),
            session_provider: crate::settings::RelaySessionProvider::Custom,
            model_aggregation_enabled: true,
            strategy: AggregateRelayStrategy::Failover,
            members: vec![
                AggregateRelayMember {
                    relay_id: "deepseek".to_string(),
                    weight: 1,
                },
                AggregateRelayMember {
                    relay_id: "openai".to_string(),
                    weight: 1,
                },
            ],
            routes: Vec::new(),
            default_relay_id: String::new(),
        }];
        settings
    }

    #[test]
    fn model_ids_bind_to_members_and_keep_upstream_names() {
        let settings = settings();
        let bindings = active_model_bindings(&settings).unwrap();
        let deepseek = bindings
            .iter()
            .find(|binding| binding.catalog_model == "cpx-deepseek/deepseek-reasoner")
            .unwrap();
        assert_eq!(deepseek.relay_id, "deepseek");
        assert_eq!(deepseek.upstream_model, "deepseek-reasoner");
        assert!(resolve_active_model(&settings, "cpx-openai/gpt-5.6-sol")
            .unwrap()
            .is_some());
    }

    #[test]
    fn catalog_merges_members_with_context_windows_and_plain_subagents() {
        let settings = settings();
        let aggregate = settings.active_aggregate_relay_profile().unwrap();
        let catalog: Value =
            serde_json::from_str(&build_catalog_json(&settings, &aggregate).unwrap()).unwrap();
        let models = catalog["models"].as_array().unwrap();
        assert!(models.iter().any(|model| {
            model["slug"] == "cpx-deepseek/deepseek-reasoner"
                && model["context_window"] == 1_000_000
                && model["multi_agent_version"] == "v1"
        }));
        assert!(models
            .iter()
            .any(|model| model["slug"] == "cpx-openai/gpt-5.6-sol"));
    }

    #[test]
    fn stale_prefixed_model_is_rejected_instead_of_falling_back() {
        let settings = settings();
        let error = resolve_active_model(&settings, "cpx-removed/old-model").unwrap_err();
        assert!(error.to_string().contains("cpx-removed/old-model"));
    }

    /// 不带 `cpx-` 前缀的请求不再按模型名改派：没设默认供应商就报错，并把请求的模型名带上。
    #[test]
    fn unbound_model_names_are_rejected_without_a_default_provider() {
        let settings = settings();
        let error = resolve_active_model(&settings, "ccs-ghost/nope").unwrap_err();
        assert!(error.to_string().contains("不是聚合模型"), "{error}");
        assert!(error.to_string().contains("ccs-ghost/nope"), "{error}");
        assert!(error.to_string().contains("默认供应商"), "{error}");

        // 成员确实声明过这个名字也不改派：聚合只认带前缀的绑定 ID。
        let error = resolve_active_model(&settings, "deepseek-chat").unwrap_err();
        assert!(error.to_string().contains("deepseek-chat"), "{error}");
    }

    /// 不带前缀的请求全部交给默认供应商，且**原样**转发请求里的 id（opencodex 语义）。
    /// 带 `cpx-` 前缀的残留 ID 不走默认供应商，硬报错（cc-switch 语义）。
    #[test]
    fn default_provider_serves_unbound_models_verbatim() {
        let mut settings = settings();
        settings.aggregate_relay_profiles[0].default_relay_id = "openai".to_string();

        let bare = resolve_active_model(&settings, "gpt-9.9-unknown")
            .unwrap()
            .expect("配了默认供应商后，不带前缀的请求交给它");
        assert_eq!(bare.relay_id, "openai");
        assert_eq!(bare.upstream_model, "gpt-9.9-unknown");

        // 别的目录（cc-switch 的 `ccs-xxx/model`）留下的名字也原样转发，不改名成 deepseek-flash。
        let prefixed = resolve_active_model(&settings, "ccs-ghost/deepseek-flash[1M]")
            .unwrap()
            .expect("不带 cpx- 前缀 → 交给默认供应商");
        assert_eq!(prefixed.relay_id, "openai");
        assert_eq!(prefixed.upstream_model, "ccs-ghost/deepseek-flash[1M]");

        // 旧聚合 ID 属于「带前缀但解析不出来」：不回落，直接报错。
        let stale = resolve_active_model(&settings, "cpx-removed/old-model").unwrap_err();
        assert!(stale.to_string().contains("cpx-removed/old-model"), "{stale}");
    }

    /// 成员前缀命中但成员不可用时是成员级报错，不会被默认供应商接管。
    #[test]
    fn member_attribution_and_errors_take_precedence_over_default() {
        let mut settings = settings();
        settings.aggregate_relay_profiles[0].default_relay_id = "openai".to_string();

        settings
            .relay_profiles
            .iter_mut()
            .find(|relay| relay.id == "deepseek")
            .unwrap()
            .api_key = String::new();
        let error = resolve_active_model(&settings, "cpx-deepseek/deepseek-reasoner").unwrap_err();
        assert!(error.to_string().contains("当前不可用"), "{error}");
        assert!(error.to_string().contains("DeepSeek"), "{error}");
    }

    /// 默认供应商自己不可用时点名默认供应商；默认值已不是成员（配置漂移）时不算数，落回原有 400。
    #[test]
    fn unusable_or_drifted_default_reports_clearly() {
        let mut drifted = settings();
        drifted.aggregate_relay_profiles[0].default_relay_id = "ghost".to_string();
        let error = resolve_active_model(&drifted, "gpt-9.9-unknown").unwrap_err();
        assert!(error.to_string().contains("默认供应商"), "{error}");
        assert!(error.to_string().contains("gpt-9.9-unknown"), "{error}");

        let mut unusable = settings();
        unusable.aggregate_relay_profiles[0].default_relay_id = "openai".to_string();
        unusable
            .relay_profiles
            .iter_mut()
            .find(|relay| relay.id == "openai")
            .unwrap()
            .api_key = String::new();
        let error = resolve_active_model(&unusable, "gpt-9.9-unknown").unwrap_err();
        assert!(error.to_string().contains("默认供应商"), "{error}");
        assert!(error.to_string().contains("OpenAI API"), "{error}");
    }

    /// `defaultRelayId` 按 camelCase 序列化；旧设置没有该字段时按未设置处理。
    #[test]
    fn default_relay_id_serializes_camel_case_and_stays_optional() {
        let mut settings = settings();
        settings.aggregate_relay_profiles[0].default_relay_id = "openai".to_string();
        let text = serde_json::to_string(&settings.aggregate_relay_profiles[0]).unwrap();
        assert!(text.contains(r#""defaultRelayId":"openai""#), "{text}");

        let legacy: AggregateRelayProfile = serde_json::from_str(
            r#"{"id":"agg","name":"聚合","sessionProvider":"custom","modelAggregationEnabled":true,"strategy":"failover","members":[],"routes":[]}"#,
        )
        .unwrap();
        assert!(legacy.default_relay_id.is_empty());
    }


    /// 默认成员的模型用裸名发布（对齐 opencodex / cc-switch），其他成员仍带 `cpx-` 前缀；
    /// 裸名请求因此天然落回默认成员，且按原样转发。
    #[test]
    fn default_member_publishes_bare_model_names() {
        let mut settings = settings();
        settings.aggregate_relay_profiles[0].default_relay_id = "deepseek".to_string();
        let aggregate = settings.active_aggregate_relay_profile().unwrap();
        let catalog: Value =
            serde_json::from_str(&build_catalog_json(&settings, &aggregate).unwrap()).unwrap();
        let slugs: Vec<&str> = catalog["models"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|model| model["slug"].as_str())
            .collect();

        assert!(slugs.contains(&"deepseek-reasoner"), "默认成员的模型应是裸名：{slugs:?}");
        assert!(slugs.contains(&"deepseek-chat"), "默认成员的当前模型也应带出来：{slugs:?}");
        assert!(
            !slugs.iter().any(|slug| slug.starts_with("cpx-deepseek/")),
            "默认成员不该再发布带前缀的副本：{slugs:?}"
        );
        assert!(slugs.contains(&"cpx-openai/gpt-5.6-sol"), "非默认成员仍带前缀：{slugs:?}");

        // 显示名同样分档：默认成员裸名，其他成员带「（供应商）」后缀。
        let by_slug: std::collections::HashMap<&str, &str> = catalog["models"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|model| Some((model["slug"].as_str()?, model["display_name"].as_str()?)))
            .collect();
        assert!(
            !by_slug["deepseek-reasoner"].contains("（"),
            "默认成员的显示名不该带供应商后缀：{:?}",
            by_slug["deepseek-reasoner"]
        );
        assert!(
            by_slug["cpx-openai/gpt-5.6-sol"].contains("（OpenAI"),
            "非默认成员的显示名仍带供应商后缀：{:?}",
            by_slug["cpx-openai/gpt-5.6-sol"]
        );

        // 裸名请求交给默认成员，且原样转发（不去前缀、不剥后缀）。
        let binding = resolve_active_model(&settings, "deepseek-reasoner")
            .unwrap()
            .expect("裸名交给默认供应商");
        assert_eq!(binding.relay_id, "deepseek");
        assert_eq!(binding.upstream_model, "deepseek-reasoner");
    }

    /// 活动目录发布的模型 ID：聚合是 `cpx-*`，普通供应商是它自己的目录条目加路由源模型名。
    #[test]
    fn active_published_model_ids_follow_active_provider() {
        let settings = settings();
        let ids = crate::model_catalog::active_published_model_ids(&settings);
        assert!(ids.contains("cpx-deepseek/deepseek-reasoner"));
        assert!(ids.contains("cpx-openai/gpt-5.6-sol"));
        assert!(!ids.contains("deepseek-reasoner"));

        let mut single = settings.clone();
        single.active_aggregate_relay_id = String::new();
        single.active_relay_id = "deepseek".to_string();
        single.aggregate_relay_profiles.clear();
        single.relay_profiles[0].model_routes = vec![crate::settings::RelayModelRoute {
            model: "ccs-deepseek/deepseek-flash".to_string(),
            target_relay_id: "openai".to_string(),
            target_model: String::new(),
        }];
        let ids = crate::model_catalog::active_published_model_ids(&single);
        assert!(ids.contains("deepseek-chat"));
        assert!(ids.contains("deepseek-reasoner"));
        assert!(ids.contains("ccs-deepseek/deepseek-flash"));

        // 配了有效的默认供应商后，任何解析不到的模型名都有兜底，不再按目录清理最近使用。
        let mut with_default = settings.clone();
        with_default.aggregate_relay_profiles[0].default_relay_id = "openai".to_string();
        assert!(
            crate::model_catalog::active_published_model_ids(&with_default).is_empty(),
            "默认供应商配置有效时不应清理最近使用"
        );
        with_default.aggregate_relay_profiles[0].default_relay_id = "not-a-member".to_string();
        assert!(
            !crate::model_catalog::active_published_model_ids(&with_default).is_empty(),
            "默认值已不是成员时仍按目录清理"
        );
    }

    #[test]
    fn official_login_member_publishes_models_with_member_prefix() {
        let mut settings = settings();
        settings.relay_profiles.push(RelayProfile {
            id: "official".to_string(),
            name: "官方登录".to_string(),
            model: "gpt-5.6-sol".to_string(),
            relay_mode: RelayMode::Official,
            auth_contents: r#"{"auth_mode":"chatgpt","tokens":{"access_token":"a","refresh_token":"r","account_id":"acct-1"}}"#.to_string(),
            ..RelayProfile::default()
        });
        settings.aggregate_relay_profiles[0]
            .members
            .push(AggregateRelayMember {
                relay_id: "official".to_string(),
                weight: 1,
            });

        let bindings = active_model_bindings(&settings).unwrap();
        assert!(bindings.iter().any(|binding| {
            binding.catalog_model == "cpx-official/gpt-5.6-sol"
                && binding.upstream_model == "gpt-5.6-sol"
                && binding.relay_id == "official"
        }));
        assert!(resolve_active_model(&settings, "cpx-official/gpt-5.6-sol")
            .unwrap()
            .is_some());
    }

    #[test]
    fn official_member_without_login_is_rejected() {
        let mut settings = settings();
        settings.relay_profiles.push(RelayProfile {
            id: "official".to_string(),
            name: "官方登录".to_string(),
            model: "gpt-5.6-sol".to_string(),
            relay_mode: RelayMode::Official,
            ..RelayProfile::default()
        });
        settings.aggregate_relay_profiles[0]
            .members
            .push(AggregateRelayMember {
                relay_id: "official".to_string(),
                weight: 1,
            });
        let error = active_model_bindings(&settings).unwrap_err();
        assert!(error.to_string().contains("官方登录"), "{error}");
    }

    /// 单个成员失效（缺 Key/URL）只影响它自己的模型，其他成员的模型继续可用。
    #[test]
    fn unready_member_only_blocks_its_own_models() {
        let mut settings = settings();
        settings
            .relay_profiles
            .iter_mut()
            .find(|relay| relay.id == "openai")
            .unwrap()
            .api_key = String::new();

        let binding = resolve_active_model(&settings, "cpx-deepseek/deepseek-reasoner")
            .unwrap()
            .expect("健康成员的模型应仍然可用");
        assert_eq!(binding.relay_id, "deepseek");
        assert_eq!(binding.upstream_model, "deepseek-reasoner");

        let error = resolve_active_model(&settings, "cpx-openai/gpt-5.6-sol").unwrap_err();
        assert!(error.to_string().contains("当前不可用"), "{error}");
        assert!(error.to_string().contains("OpenAI API"), "{error}");
    }

    /// 成员供应商被删除时给出成员级错误，同时不阻塞其他成员。
    #[test]
    fn missing_member_reports_member_level_error_without_blocking_others() {
        let mut settings = settings();
        settings.aggregate_relay_profiles[0]
            .members
            .push(AggregateRelayMember {
                relay_id: "ghost".to_string(),
                weight: 1,
            });

        assert!(resolve_active_model(&settings, "cpx-deepseek/deepseek-reasoner")
            .unwrap()
            .is_some());

        let error = resolve_active_model(&settings, "cpx-ghost/some-model").unwrap_err();
        assert!(error.to_string().contains("ghost"), "{error}");
        assert!(error.to_string().contains("当前不可用"), "{error}");
    }

    #[test]
    fn multi_agent_v2_flag_forms_are_detected() {
        assert!(multi_agent_v2_enabled("[features]\nmulti_agent_v2 = true\n"));
        assert!(!multi_agent_v2_enabled("[features]\nmulti_agent_v2 = false\n"));
        assert!(multi_agent_v2_enabled(
            "[features.multi_agent_v2]\nmax_concurrent_threads_per_session = 4\n"
        ));
        assert!(!multi_agent_v2_enabled(
            "[features.multi_agent_v2]\nenabled = false\n"
        ));
        assert!(!multi_agent_v2_enabled("model = \"gpt-5.6-sol\"\n"));
        assert!(!multi_agent_v2_enabled("not = [valid"));
    }

    #[test]
    fn subagent_conflict_warning_requires_active_model_aggregation_and_flag() {
        let temp = tempfile::tempdir().unwrap();
        let mut settings = settings();

        assert!(subagent_conflict_warning(temp.path(), &settings).is_none());

        std::fs::write(
            temp.path().join("config.toml"),
            "model = \"x\"\n[features]\nmulti_agent_v2 = true\n",
        )
        .unwrap();
        let warning = subagent_conflict_warning(temp.path(), &settings).unwrap();
        assert!(warning.contains("multi_agent_v2"), "{warning}");
        assert!(warning.contains("子代理"), "{warning}");

        // 关闭模型聚合（故障转移类型）后不再提示。
        settings.aggregate_relay_profiles[0].model_aggregation_enabled = false;
        assert!(subagent_conflict_warning(temp.path(), &settings).is_none());
    }
}
