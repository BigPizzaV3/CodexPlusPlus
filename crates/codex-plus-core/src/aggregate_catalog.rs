//! 按模型聚合：发布带供应商身份的模型目录，并精确解析到成员与真实模型。
//! 模型聚合和供应商池策略互斥；未带绑定身份的请求不能进入轮转池。

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
    if catalog_model.trim().is_empty() {
        return Ok(None);
    }
    if !is_aggregate_model(catalog_model) {
        if active_model_aggregation(settings).is_some() {
            anyhow::bail!("按模型聚合需要选择已绑定供应商的模型，请在 Codex 模型列表中重新选择");
        }
        return Ok(None);
    }
    active_model_bindings(settings)?
        .into_iter()
        .find(|item| item.catalog_model == catalog_model)
        .map(Some)
        .ok_or_else(|| {
            anyhow::anyhow!("聚合模型不存在、已被移除或按模型聚合未开启：{catalog_model}")
        })
}

/// 生成前和请求前使用相同的成员校验：普通 API 成员要 URL/Key 齐备，
/// 官方登录成员要有 ChatGPT 登录凭据；另一个聚合不能当成员。
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
pub fn build_catalog_json(
    settings: &BackendSettings,
    aggregate: &AggregateRelayProfile,
) -> anyhow::Result<String> {
    let mut models = Vec::new();
    for relay in member_profiles(settings, aggregate)? {
        let payload = build_member_catalog(relay)?;
        for mut model in payload["models"].as_array().into_iter().flatten().cloned() {
            let upstream = model["slug"].as_str().unwrap_or_default().to_string();
            let display = model["display_name"]
                .as_str()
                .unwrap_or(&upstream)
                .to_string();
            model["slug"] = json!(aggregate_model_id(&relay.id, &upstream));
            model["display_name"] = json!(format!("{display}（{}）", relay.name));
            model["description"] = json!(format!("{upstream} · {}", relay.name));
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
        let plain_error = resolve_active_model(&settings, "deepseek-chat").unwrap_err();
        assert!(plain_error.to_string().contains("按模型聚合需要选择"));
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
}
