//! 按模型聚合：发布带供应商身份的模型目录，并精确解析到成员与真实模型。
//! 模型聚合和供应商池策略互斥；未带绑定身份的请求不能进入轮转池。
//!
//! 聚合解析规则：
//! - `cpx-<成员>/<模型>` 按成员前缀精确归因；解析不出来（成员被移除、模型被移出列表、
//!   成员 id 变过）一律报错，不按模型名改派、不落到默认供应商；
//! - 不带 `cpx-` 前缀的请求（旧会话裸名、别的目录写下的 `<前缀>/<模型>`、回合前自动压缩
//!   带的上一回合模型名）交给聚合的「默认供应商」，按请求里的 id 原样转发；没设默认就报错。

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::Context;
use sha2::{Digest, Sha256};
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

/// 切到供应商池后，旧聚合身份只能映射到池中已声明的同名模型；
/// 返回裸名让原有轮转/故障转移继续决定成员，不保留旧前缀的绑定语义。
pub fn pool_model_name(settings: &BackendSettings, model: &str) -> anyhow::Result<Option<String>> {
    let Some(aggregate) = settings.active_aggregate_relay_profile()
        .filter(|aggregate| !aggregate.model_aggregation_enabled) else {
        return Ok(None);
    };
    if !is_aggregate_model(model) {
        return Ok(None);
    }
    let bare = model.split_once('/').map(|(_, bare)| bare).unwrap_or("");
    for relay in member_profiles(settings, &aggregate)? {
        if member_upstream_catalog_entries(relay)?.iter().any(|entry| entry.slug == bare) {
            return Ok(Some(bare.to_string()));
        }
    }
    Ok(None)
}

pub fn active_model_bindings(
    settings: &BackendSettings,
) -> anyhow::Result<Vec<AggregateModelBinding>> {
    let Some(aggregate) = active_model_aggregation(settings) else {
        return Ok(Vec::new());
    };
    let mut bindings = Vec::new();
    for relay in member_profiles(settings, &aggregate)? {
        for entry in member_upstream_catalog_entries(relay)? {
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
        // 解析不出来就硬报错，不做按模型名的静默改派
        // ——改派会把请求花到用户没选的那家，用户看不出来。
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
        if let Some(upstream_model) = member_upstream_catalog_entries(relay)?
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
        // 成员还在、但已经不提供这个模型（模型被移出列表）：硬报错，
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
    // 原样转发请求里的 id（走默认供应商时保留请求的 model id）。
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

/// 从聚合目录复制的模型 ID 先去掉本成员的一层前缀；跨成员引用保持原样。
fn strip_member_model_prefix(relay_id: &str, value: &str) -> String {
    value
        .strip_prefix(member_model_prefix(relay_id).as_str())
        .filter(|rest| !rest.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string())
}

/// 请求解析和绑定列表必须使用与发布目录相同的上游 slug，否则目录中的单层 ID
/// 会因原始条目仍带本成员前缀而无法解析。保留原始读取入口，供目录先按原配置键
/// 应用窗口和元数据，再归一化身份，避免已有覆盖配置失效。
fn member_upstream_catalog_entries(
    profile: &RelayProfile,
) -> anyhow::Result<Vec<ModelCatalogEntry>> {
    let mut seen = HashSet::new();
    Ok(profile_catalog_entries(profile)?
        .into_iter()
        .filter_map(|mut entry| {
            entry.slug = strip_member_model_prefix(&profile.id, &entry.slug);
            (!entry.slug.is_empty() && seen.insert(entry.slug.clone())).then_some(entry)
        })
        .collect())
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
/// 发布形态：**默认成员的模型用裸名发布**（不带前缀的请求本来就
/// 归它，客户端列表里"选到哪家"一目了然），其他成员仍用 `cpx-<成员id>/<模型>` 绑定身份。
pub fn build_catalog_json(
    settings: &BackendSettings,
    aggregate: &AggregateRelayProfile,
) -> anyhow::Result<String> {
    let default_relay_id = aggregate.default_relay_id.trim();
    let mut models = Vec::new();
    let mut pool_models = HashSet::new();
    let members = member_profiles(settings, aggregate)?;
    let default_model = members.iter().find(|relay| relay.id == default_relay_id)
        .map(|relay| member_upstream_catalog_entries(relay)).transpose()?
        .and_then(|entries| entries.into_iter().next()).map(|entry| entry.slug);
    for relay in members {
        let payload = build_member_catalog(relay)?;
        let is_default_member = !default_relay_id.is_empty() && relay.id.as_str() == default_relay_id;
        // 分组的展示名：成员没起名字时回落 relay id，避免前端渲染出无名分组表头。
        let group_name = if relay.name.trim().is_empty() {
            relay.id.clone()
        } else {
            relay.name.trim().to_string()
        };
        // 成员自己的模型 ID 前缀：成员列表里若出现 `cpx-<自己>/kimi-k3`（从聚合目录复制
        // 来的模型名很常见），再套一层前缀就会变成 `cpx-<自己>/cpx-<自己>/kimi-k3`——
        // 两层前缀直接露在客户端列表里。这里先剥掉成员自己的那一层。
        // 剥掉自指前缀后可能撞车：`model` 写 `cpx-<自己>/kimi-k3`、`model_list` 又写了裸名
        // `kimi-k3`，两条会归一成同一个上游模型。按上游 slug 去重，保留先出现的那条
        // （`model` 在列表最前，与目录原本的优先级一致）。
        let mut seen_upstream = HashSet::new();
        for mut model in payload["models"].as_array().into_iter().flatten().cloned() {
            let upstream = strip_member_model_prefix(
                &relay.id,
                model["slug"].as_str().unwrap_or_default(),
            );
            if upstream.is_empty() || !seen_upstream.insert(upstream.clone()) {
                continue;
            }
            if !aggregate.model_aggregation_enabled && !pool_models.insert(upstream.clone()) {
                continue;
            }
            let display = {
                let raw = model["display_name"].as_str().unwrap_or(&upstream).to_string();
                strip_member_model_prefix(&relay.id, &raw)
            };
            model["slug"] = if is_default_member || !aggregate.model_aggregation_enabled {
                json!(upstream)
            } else {
                json!(aggregate_model_id(&relay.id, &upstream))
            };
            // 默认成员连显示名也保持裸名：它就是"默认"，再缀「（供应商）」只会让人以为
            // 还得再挑一次供应商。其他成员保留后缀，避免同名模型分不清是哪一家。
            model["display_name"] = if is_default_member || !aggregate.model_aggregation_enabled {
                json!(display)
            } else {
                json!(format!("{display}（{}）", relay.name))
            };
            model["description"] = json!(format!("{upstream} · {}", relay.name));
            model["codex_plus_default_model"] = json!(
                default_model.as_deref() == Some(upstream.as_str())
                    && (is_default_member || !aggregate.model_aggregation_enabled)
            );
            // 供应商分组身份（前端模型选择器按它把连续模型折叠成一组）：
            // - `codex_plus_group_id` 是权威分组键，用持久化 relay id；成员已按 relay_id
            //   去重（member_profiles），同一家不会产生两组，同名成员也各自成组；
            // - `codex_plus_group` 只用于展示，默认成员同样要写，否则它的模型没有归属；
            // - `codex_plus_default` 标出默认成员：只有前端能据此打「默认」标志。
            if aggregate.model_aggregation_enabled {
                model["codex_plus_group"] = json!(group_name);
                model["codex_plus_group_id"] = json!(relay.id);
            } else if let Some(model) = model.as_object_mut() {
                model.remove("codex_plus_group");
                model.remove("codex_plus_group_id");
            }
            if is_default_member && aggregate.model_aggregation_enabled {
                model["codex_plus_default"] = json!(true);
            } else if let Some(model) = model.as_object_mut() {
                model.remove("codex_plus_default");
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
        anyhow::bail!("聚合至少需要一个成员模型，请先填写成员供应商的模型列表");
    }
    Ok(serde_json::to_string_pretty(&json!({ "models": models }))?)
}

/// apply 流程只拿到活动 RelayProfile；从持久化 settings 取出成员引用，
/// 模型聚合发布绑定身份，供应商池发布去重裸名；非聚合保持原有目录行为。
pub fn build_catalog_for_profile(profile: &RelayProfile) -> anyhow::Result<Option<String>> {
    if profile.relay_mode != RelayMode::Aggregate {
        return Ok(None);
    }
    let settings = SettingsStore::default().load()?;
    let Some(aggregate) = settings
        .aggregate_relay_profiles
        .iter()
        .find(|aggregate| aggregate.id == profile.id)
    else {
        return Ok(None);
    };
    Ok(Some(build_catalog_json(&settings, aggregate)?))
}

fn aggregate_publication_key(id: &str) -> String {
    format!("{:x}", Sha256::digest(id.as_bytes()))[..16].to_string()
}

fn refresh_journal_path(home: &Path, aggregate: &str) -> std::path::PathBuf {
    home.join("model-catalogs").join(format!(
        "refresh-{}.json",
        aggregate_publication_key(aggregate)
    ))
}

fn owned_catalog_path(home: &Path, aggregate: &str, pointer: &str) -> Option<std::path::PathBuf> {
    let path = std::path::PathBuf::from(pointer);
    let path = if path.is_absolute() {
        path
    } else {
        home.join(path)
    };
    let path = path.canonicalize().ok()?;
    let root = home.join("model-catalogs").canonicalize().ok()?;
    if path.parent()? != root || !path.is_file() {
        return None;
    }
    let name = path.file_name()?.to_str()?;
    let legacy = format!(
        "{}.json",
        crate::relay_config::sanitize_catalog_filename(aggregate)
    );
    let prefix = format!("aggregate-{}-", aggregate_publication_key(aggregate));
    (name == legacy || (name.starts_with(&prefix) && name.ends_with(".json"))).then_some(path)
}

fn config_catalog_pointer(config: &str) -> Option<String> {
    config
        .parse::<toml_edit::DocumentMut>()
        .ok()?
        .get("model_catalog_json")?
        .as_str()
        .map(str::to_string)
}

fn direct_catalog_refresh_supported(profile: &RelayProfile) -> bool {
    matches!(profile.relay_mode, RelayMode::PureApi | RelayMode::MixedApi)
}

fn direct_catalog_fields_equal(previous: &RelayProfile, next: &RelayProfile) -> bool {
    crate::relay_config::relay_profile_model(previous)
        == crate::relay_config::relay_profile_model(next)
        && previous.model == next.model
        && previous.model_list == next.model_list
        && previous.model_windows == next.model_windows
        && previous.model_auto_compact == next.model_auto_compact
        && previous.model_metadata == next.model_metadata
        && previous.context_window == next.context_window
        && previous.auto_compact_limit == next.auto_compact_limit
        && previous.protocol == next.protocol
        && previous.relay_mode == next.relay_mode
        && previous.base_url == next.base_url
        && previous.upstream_base_url == next.upstream_base_url
        && previous.official_mix_api_key == next.official_mix_api_key
}

fn active_catalog(
    settings: &BackendSettings,
) -> anyhow::Result<(String, String, String)> {
    if let Some(aggregate) = settings.active_aggregate_relay_profile() {
        let catalog = build_catalog_json(settings, &aggregate)?;
        let payload: Value = serde_json::from_str(&catalog)?;
        return Ok((aggregate.id, catalog, catalog_default_model(&payload).to_string()));
    }
    let profile = settings.active_relay_profile();
    let catalog = build_member_catalog_json(
        &profile,
        false,
        crate::relay_config::uses_official_deepseek_responses(&profile),
    )?;
    let model = crate::relay_config::relay_profile_model(&profile);
    Ok((profile.id, catalog, model))
}

fn configured_default_model(settings: &BackendSettings) -> String {
    if let Some(aggregate) = settings.active_aggregate_relay_profile() {
        if let Some(profile) = settings
            .relay_profiles
            .iter()
            .find(|profile| profile.id == aggregate.default_relay_id)
        {
            return crate::relay_config::relay_profile_model(profile);
        }
    }
    crate::relay_config::relay_profile_model(&settings.active_relay_profile())
}

/// 在读取/保存入口恢复崩溃时已提交 settings、尚未发布的目录。不会覆盖外部改动。
pub fn recover_pending_catalog_refresh(store: &SettingsStore, home: &Path) -> anyhow::Result<bool> {
    let settings = store.load()?;
    if !settings.relay_profiles_enabled {
        return Ok(false);
    }
    let id = settings.active_relay_profile().id;
    let journal = refresh_journal_path(home, &id);
    let text = match std::fs::read_to_string(&journal) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    let Ok(mut pending) = serde_json::from_str::<Value>(&text) else {
        return Ok(false);
    };
    if !matches!(
        pending["status"].as_str(),
        Some("prepared" | "settings_committed")
    ) || pending["settingsRevision"].as_str() != Some(settings.settings_revision.as_str())
        || pending["aggregateId"].as_str() != Some(id.as_str())
    {
        return Ok(false);
    }
    store.with_current_revision(&settings.settings_revision, || {
        let Some(path) = pending["catalogPath"]
            .as_str()
            .and_then(|p| owned_catalog_path(home, &id, p))
        else {
            return Ok(false);
        };
        let catalog = std::fs::read_to_string(&path)?;
        crate::relay_config::validate_managed_model_catalog(&catalog)?;
        if pending["catalogHash"].as_str()
            != Some(format!("{:x}", Sha256::digest(catalog.as_bytes())).as_str())
        {
            return Ok(false);
        }
        let config_path = home.join("config.toml");
        let config = std::fs::read_to_string(&config_path)?;
        let already_published = config_catalog_pointer(&config)
            .and_then(|pointer| owned_catalog_path(home, &id, &pointer))
            == Some(path.clone());
        if !already_published {
            if pending["previousConfigHash"].as_str()
                != Some(format!("{:x}", Sha256::digest(config.as_bytes())).as_str())
            {
                pending["status"] = json!("external_config_changed");
                crate::settings::atomic_write(&journal, &serde_json::to_vec_pretty(&pending)?)?;
                return Ok(false);
            }
            let mut doc: toml_edit::DocumentMut = config.parse()?;
            doc["model_catalog_json"] = toml_edit::value(path.to_string_lossy().as_ref());
            if let Some(model) = pending["model"].as_str() {
                doc["model"] = toml_edit::value(model);
            }
            crate::settings::atomic_write(&config_path, doc.to_string().as_bytes())?;
        }
        pending["status"] = json!("completed");
        crate::settings::atomic_write(&journal, &serde_json::to_vec_pretty(&pending)?)?;
        Ok(true)
    })
}

/// 模型编辑发布新的不可变目录及配置默认，不执行供应商切换或重启。
pub fn save_with_active_catalog_refresh(
    store: &SettingsStore,
    home: &Path,
    next: &BackendSettings,
) -> anyhow::Result<String> {
    save_with_catalog_publisher(store, home, next, |path, content| {
        crate::settings::atomic_write(path, content)
    })
}

fn save_with_catalog_publisher(
    store: &SettingsStore,
    home: &Path,
    next: &BackendSettings,
    publish: impl FnOnce(&Path, &[u8]) -> anyhow::Result<()>,
) -> anyhow::Result<String> {
    recover_pending_catalog_refresh(store, home)?;
    let previous = store.load()?;
    if !next.relay_profiles_enabled
        || previous.active_relay_id != next.active_relay_id
        || previous.active_aggregate_relay_id != next.active_aggregate_relay_id
    {
        return store.save_versioned(next);
    }
    if next.active_aggregate_relay_profile().is_none() {
        let previous_profile = previous.active_relay_profile();
        let next_profile = next.active_relay_profile();
        if !direct_catalog_refresh_supported(&next_profile)
            || direct_catalog_fields_equal(&previous_profile, &next_profile)
        {
            return store.save_versioned(next);
        }
    }
    let config_path = home.join("config.toml");
    let config = match std::fs::read_to_string(&config_path) {
        Ok(config) => config,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return store.save_versioned(next);
        }
        Err(error) => return Err(error.into()),
    };
    let active_id = next
        .active_aggregate_relay_profile()
        .map(|aggregate| aggregate.id.clone())
        .unwrap_or_else(|| next.active_relay_profile().id.clone());
    let Some(pointer) = config_catalog_pointer(&config)
        .and_then(|pointer| owned_catalog_path(home, &active_id, &pointer))
    else {
        return store.save_versioned(next);
    };
    let profile = previous.active_relay_profile();
    if !crate::relay_config::live_endpoint_matches_profile(&profile, &config)
        && !crate::relay_config::recognized_managed_proxy_endpoint(&profile, &config)
    {
        return store.save_versioned(next);
    }
    let (id, catalog, default_model) = active_catalog(next)?;
    crate::relay_config::validate_managed_model_catalog(&catalog)?;
    let mut doc: toml_edit::DocumentMut = config.parse()?;
    let live_model = doc.get("model").and_then(|item| item.as_str()).unwrap_or("");
    let old_default = configured_default_model(&previous);
    let payload: Value = serde_json::from_str(&catalog)?;
    let live_exists = payload["models"].as_array().is_some_and(|models|
        models.iter().any(|model| model["slug"].as_str() == Some(live_model)));
    let update_model = !default_model.is_empty()
        && (live_model.is_empty() || live_model == old_default || !live_exists);
    if update_model {
        doc["model"] = toml_edit::value(default_model.as_str());
    }
    let journal_model = doc
        .get("model")
        .and_then(|item| item.as_str())
        .map(str::to_string);
    if std::fs::read_to_string(&pointer)? == catalog && doc.to_string() == config {
        return store.save_versioned(next);
    }
    let digest = format!("{:x}", Sha256::digest(catalog.as_bytes()));
    let staged = home.join("model-catalogs").join(format!(
        "aggregate-{}-{digest}.json",
        aggregate_publication_key(&id)
    ));
    let journal = refresh_journal_path(home, &id);
    let write_journal = |status: &str, revision: &str| -> anyhow::Result<()> {
        crate::settings::atomic_write(
            &journal,
            &serde_json::to_vec_pretty(&json!({
                "status": status, "settingsRevision": revision, "aggregateId": id,
                "model": journal_model,
                "catalogPath": std::path::absolute(&staged)?, "catalogHash": digest,
                "previousConfigHash": format!("{:x}", Sha256::digest(config.as_bytes())),
            }))?,
        )
    };
    doc["model_catalog_json"] =
        toml_edit::value(std::path::absolute(&staged)?.to_string_lossy().as_ref());
    let result = store.save_with_prepared_commit(
        next,
        |revision| {
            if std::fs::read_to_string(&config_path)? != config {
                anyhow::bail!("实时配置已变更，未准备目录发布");
            }
            if staged.exists() {
                anyhow::ensure!(
                    std::fs::read(&staged)? == catalog.as_bytes(),
                    "不可变目录内容与哈希不一致"
                );
            } else {
                crate::settings::atomic_write(&staged, catalog.as_bytes())?;
            }
            write_journal("prepared", revision)
        },
        |revision| {
            anyhow::ensure!(store.revision()? == revision, "目录发布前设置已被外部修改");
            write_journal("settings_committed", revision)?;
            if std::fs::read_to_string(&config_path)? != config {
                anyhow::bail!("实时配置已被其他操作修改，未发布目录指针");
            }
            publish(&config_path, doc.to_string().as_bytes())
        },
    );
    match result {
        Ok(revision) => {
            let _ = write_journal("completed", &revision);
            Ok(revision)
        }
        // prepare 未执行时（例如 CAS 冲突）不能在锁外覆盖其他事务的 journal。
        // 已 prepare 但发布失败时 settings 会回滚，目标 revision 不匹配即可阻止恢复。
        Err(error) => Err(error),
    }
}
/// 注入层读取与原生 Codex 相同的目录条目，显示名与能力仍属于对应成员。
pub fn catalog_summary(
    home: &Path,
    settings: &BackendSettings,
    aggregate: &AggregateRelayProfile,
) -> anyhow::Result<Value> {
    let config = std::fs::read_to_string(home.join("config.toml")).unwrap_or_default();
    let published = config_catalog_pointer(&config)
        .and_then(|pointer| owned_catalog_path(home, &aggregate.id, &pointer))
        .and_then(|path| std::fs::read_to_string(path).ok());
    let applied = published.is_some();
    // 不急切求值候选目录：即使当前编辑无效，已发布的安全快照仍然可展示。
    let desired = build_catalog_json(settings, aggregate);
    let pending_changes = published.as_ref().is_some_and(|catalog| desired.as_ref().ok() != Some(catalog));
    let payload: Value = match published {
        Some(catalog) => serde_json::from_str(&catalog)?,
        None => serde_json::from_str(&desired?)?,
    };
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
    let default_model = catalog_default_model(&payload);
    Ok(json!({
        "status": if applied { "ok" } else { "not_applied" },
        "applied": applied,
        "pending_changes": pending_changes,
        "path": home.join("config.toml").to_string_lossy(),
        "service_tier": Value::Null,
        "model": default_model,
        "model_provider": aggregate.id,
        "codex_model_provider": crate::model_catalog::codex_model_provider_for_relay_profile(home, profile),
        "provider_name": aggregate.name,
        // 默认成员的 relay id：模型选择器用它给默认那一组打「默认」标志。
        "codex_plus_default_group_id": if applied {
            models.iter().find(|model| model["codex_plus_default"].as_bool() == Some(true))
                .and_then(|model| model["codex_plus_group_id"].as_str()).unwrap_or("")
        } else { aggregate.default_relay_id.trim() },
        "default_model": default_model,
        "models": ids,
        "modelMetadata": metadata,
        "sources": [{
            "id": format!("aggregate:{}", aggregate.id),
            "type": if aggregate.model_aggregation_enabled { "aggregate_model_catalog" } else { "aggregate_pool_catalog" },
            "name": aggregate.name,
            "base_url": "",
            "status": "ok",
            "models": models.len(),
            "responses_api": { "status": "unknown" }
        }],
        "responses_api": { "status": "unknown" }
    }))
}

pub(crate) fn catalog_default_model(payload: &Value) -> &str {
    let Some(models) = payload["models"].as_array() else { return ""; };
    models.iter().find(|model| model["codex_plus_default_model"].as_bool() == Some(true))
        .or_else(|| models.iter().find(|model| model["codex_plus_default"].as_bool() == Some(true)))
        .or_else(|| models.first())
        .and_then(|model| model["slug"].as_str()).unwrap_or("")
}

fn build_member_catalog(profile: &RelayProfile) -> anyhow::Result<Value> {
    let catalog_json = build_member_catalog_json(profile, false, false)?;
    Ok(serde_json::from_str(&catalog_json)?)
}

fn build_member_catalog_json(
    profile: &RelayProfile,
    lite_supported: bool,
    official_deepseek: bool,
) -> anyhow::Result<String> {
    let entries = profile_catalog_entries(profile)?;
    if entries.is_empty() {
        return Ok(r#"{"models":[]}"#.to_string());
    }
    let fallback = parse_window_token(&profile.context_window);
    let catalog_json = build_model_catalog_json_with_capabilities(
        &entries,
        fallback,
        None,
        (!lite_supported).then_some(false),
        official_deepseek,
    );
    let metadata = crate::relay_config::parse_model_metadata_map(&profile.model_metadata)?;
    let catalog_json =
        crate::relay_config::apply_model_metadata_overrides(&catalog_json, &metadata)?;
    // 聚合代理按成员原生协议转发，成员上游不认 tool_search 时同样要在目录里关掉；
    // 放在元数据覆盖之后，已保存的整份元数据不能把它重新打开。
    let catalog_json = if crate::relay_config::upstream_rejects_tool_search(profile) {
        crate::relay_config::disable_search_tool_in_catalog(&catalog_json)?
    } else {
        catalog_json
    };
    crate::relay_config::validate_managed_model_catalog(&catalog_json)?;
    Ok(catalog_json)
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

    #[test]
    fn default_model_follows_default_member_without_reordering_groups() {
        let mut settings = settings();
        settings.aggregate_relay_profiles[0].default_relay_id = "openai".into();
        let aggregate = settings.active_aggregate_relay_profile().unwrap();
        let payload: Value = serde_json::from_str(&build_catalog_json(&settings, &aggregate).unwrap()).unwrap();
        assert_eq!(payload["models"][0]["codex_plus_group_id"], "deepseek");
        assert_eq!(catalog_default_model(&payload), "gpt-5.6-sol");
        let temp = tempfile::tempdir().unwrap();
        let summary = catalog_summary(temp.path(), &settings, &aggregate).unwrap();
        assert_eq!(summary["model"], "gpt-5.6-sol");
        assert_eq!(summary["default_model"], "gpt-5.6-sol");
    }

    #[test]
    fn configured_default_is_independent_of_member_and_model_list_order() {
        for model_aggregation_enabled in [true, false] {
            let mut settings = settings();
            settings.aggregate_relay_profiles[0].default_relay_id = "openai".into();
            settings.aggregate_relay_profiles[0].model_aggregation_enabled = model_aggregation_enabled;
            settings.relay_profiles[1].config_contents = "model = \"configured-model\"\n".into();
            settings.relay_profiles[1].model_list = "list-first\nconfigured-model".into();
            // 池中同名模型可能先由非默认成员发布，去重不能丢掉真正的默认值。
            settings.relay_profiles[0].model_list.push_str("\nconfigured-model");
            for reverse in [false, true] {
                if reverse { settings.aggregate_relay_profiles[0].members.reverse(); }
                let aggregate = settings.active_aggregate_relay_profile().unwrap();
                let payload: Value = serde_json::from_str(&build_catalog_json(&settings, &aggregate).unwrap()).unwrap();
                assert_eq!(catalog_default_model(&payload), "configured-model");
                assert_eq!(payload["models"].as_array().unwrap().iter().filter(|model|
                    model["codex_plus_default_model"].as_bool() == Some(true)).count(), 1);
            }
        }
    }

    #[test]
    fn pool_catalog_uses_member_union_and_only_maps_declared_retired_models() {
        let mut settings = settings();
        settings.aggregate_relay_profiles[0].model_aggregation_enabled = false;
        settings.relay_profiles[1].model_list = "deepseek-reasoner".into();
        let aggregate = settings.active_aggregate_relay_profile().unwrap();
        let payload: Value = serde_json::from_str(&build_catalog_json(&settings, &aggregate).unwrap()).unwrap();
        let models = payload["models"].as_array().unwrap();
        assert_eq!(models.len(), 3);
        assert!(models.iter().all(|model| !model["slug"].as_str().unwrap().starts_with("cpx-")));
        assert!(models.iter().all(|model| model.get("codex_plus_group_id").is_none()));
        assert_eq!(pool_model_name(&settings, "cpx-old/deepseek-reasoner").unwrap().as_deref(), Some("deepseek-reasoner"));
        assert_eq!(pool_model_name(&settings, "cpx-old/unconfigured").unwrap(), None);
        let temp = tempfile::tempdir().unwrap();
        let summary = catalog_summary(temp.path(), &settings, &aggregate).unwrap();
        assert_eq!(summary["sources"][0]["type"], "aggregate_pool_catalog");
        settings.aggregate_relay_profiles[0].model_aggregation_enabled = true;
        assert_eq!(pool_model_name(&settings, "cpx-old/deepseek-reasoner").unwrap(), None);
    }

    fn publication_fixture() -> (tempfile::TempDir, SettingsStore, std::path::PathBuf) {
        let temp=tempfile::tempdir().unwrap();let store=SettingsStore::new(temp.path().join("settings.json"));
        store.save(&settings()).unwrap();let loaded=store.load().unwrap();let aggregate=active_model_aggregation(&loaded).unwrap();
        let home=temp.path().join("home");std::fs::create_dir_all(home.join("model-catalogs")).unwrap();
        let path=home.join("model-catalogs/agg.json");std::fs::write(&path,build_catalog_json(&loaded,&aggregate).unwrap()).unwrap();
        std::fs::write(home.join("config.toml"),format!(
            "model_catalog_json = {:?}\nmodel_provider = \"custom\"\n[model_providers.custom]\nbase_url = {:?}\n",
            path.to_str().unwrap(),
            crate::protocol_proxy::local_responses_proxy_base_url(crate::protocol_proxy::protocol_proxy_port()),
        )).unwrap();
        (temp,store,home)
    }

    const DIRECT_TRANSPORT: &str = "\nmodel_provider = \"custom\"\n[model_providers.custom]\nbase_url = \"https://deepseek.example/v1\"\n";

    fn direct_publication_fixture() -> (tempfile::TempDir, SettingsStore, std::path::PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(temp.path().join("settings.json"));
        let mut initial = settings();
        initial.active_relay_id = "deepseek".into();
        initial.active_aggregate_relay_id.clear();
        initial.relay_profiles[0].config_contents = "model = \"deepseek-chat\"\n".into();
        store.save(&initial).unwrap();
        let loaded = store.load().unwrap();
        let profile = loaded.active_relay_profile();
        let home = temp.path().join("home");
        std::fs::create_dir_all(home.join("model-catalogs")).unwrap();
        let catalog = serde_json::to_string_pretty(&build_member_catalog(&profile).unwrap()).unwrap();
        let path = home.join("model-catalogs/deepseek.json");
        std::fs::write(&path, &catalog).unwrap();
        std::fs::write(
            home.join("config.toml"),
            format!(
                "model = {:?}\nmodel_catalog_json = {:?}\n{DIRECT_TRANSPORT}",
                crate::relay_config::relay_profile_model(&profile),
                path.to_str().unwrap()
            ),
        )
        .unwrap();
        (temp, store, home)
    }

    #[test]
    fn direct_catalog_refresh_updates_default_but_preserves_live_choice() {
        let (_temp, store, home) = direct_publication_fixture();
        let mut next = store.load().unwrap();
        next.relay_profiles[0].model_list = "deepseek-chat\ndeepseek-reasoner".into();
        next.relay_profiles[0].model = "deepseek-reasoner".into();
        next.relay_profiles[0].config_contents = "model = \"deepseek-reasoner\"\n".into();
        save_with_active_catalog_refresh(&store, &home, &next).unwrap();
        let config = std::fs::read_to_string(home.join("config.toml")).unwrap();
        assert!(config.contains("model = \"deepseek-reasoner\""));

        let mut selected = store.load().unwrap();
        selected.relay_profiles[0].model_list = "deepseek-chat\ndeepseek-reasoner\nkimi-k3".into();
        selected.relay_profiles[0].model = "deepseek-reasoner".into();
        selected.relay_profiles[0].config_contents = "model = \"deepseek-reasoner\"\n".into();
        std::fs::write(home.join("config.toml"), format!("model = \"deepseek-chat\"\nmodel_catalog_json = \"model-catalogs/deepseek.json\"\n{DIRECT_TRANSPORT}")).unwrap();
        save_with_active_catalog_refresh(&store, &home, &selected).unwrap();
        let config = std::fs::read_to_string(home.join("config.toml")).unwrap();
        assert!(config.contains("model = \"deepseek-chat\""));
    }

    #[test]
    fn direct_catalog_refresh_recovers_model_and_pointer_from_journal() {
        let (_temp, store, home) = direct_publication_fixture();
        let mut next = store.load().unwrap();
        next.relay_profiles[0].model_list = "deepseek-reasoner\ndeepseek-chat".into();
        next.relay_profiles[0].model = "deepseek-reasoner".into();
        next.relay_profiles[0].config_contents = "model = \"deepseek-reasoner\"\n".into();
        let id = next.active_relay_profile().id;
        let catalog = serde_json::to_string_pretty(&build_member_catalog(&next.active_relay_profile()).unwrap()).unwrap();
        let digest = format!("{:x}", Sha256::digest(catalog.as_bytes()));
        let staged = home.join("model-catalogs").join(format!("aggregate-{}-{digest}.json", aggregate_publication_key(&id)));
        std::fs::write(&staged, catalog).unwrap();
        let revision = store.save_versioned(&next).unwrap();
        let journal = refresh_journal_path(&home, &id);
        std::fs::write(&journal, serde_json::to_vec(&json!({
            "status": "prepared", "settingsRevision": revision, "aggregateId": id,
            "catalogPath": std::path::absolute(&staged).unwrap(),
            "catalogHash": digest, "previousConfigHash": format!("{:x}", Sha256::digest(std::fs::read(home.join("config.toml")).unwrap())),
            "model": "deepseek-reasoner"
        })).unwrap()).unwrap();
        assert!(recover_pending_catalog_refresh(&store, &home).unwrap());
        assert!(std::fs::read_to_string(home.join("config.toml")).unwrap().contains("deepseek-reasoner"));
    }

    #[test]
    fn direct_catalog_refresh_preserves_external_transport_even_with_owned_pointer() {
        let (_temp, store, home) = direct_publication_fixture();
        let config_path = home.join("config.toml");
        let mut config: toml_edit::DocumentMut =
            std::fs::read_to_string(&config_path).unwrap().parse().unwrap();
        config["model"] = toml_edit::value("external-model");
        config["model_providers"]["custom"]["base_url"] =
            toml_edit::value("http://127.0.0.1:58077/v1");
        std::fs::write(&config_path, config.to_string()).unwrap();
        let before = std::fs::read(&config_path).unwrap();
        let catalog_path = home.join("model-catalogs/deepseek.json");
        let catalog_before = std::fs::read(&catalog_path).unwrap();
        let mut next = store.load().unwrap();
        next.relay_profiles[0].model_list.push_str("\nnew-model");
        save_with_active_catalog_refresh(&store, &home, &next).unwrap();
        assert_eq!(std::fs::read(&config_path).unwrap(), before);
        assert_eq!(std::fs::read(&catalog_path).unwrap(), catalog_before);
        assert!(store.load().unwrap().relay_profiles[0].model_list.contains("new-model"));
    }

    #[test]
    fn unrelated_direct_settings_save_preserves_catalog_bytes() {
        let (_temp, store, home) = direct_publication_fixture();
        let config_before = std::fs::read(home.join("config.toml")).unwrap();
        let catalog_before = std::fs::read(home.join("model-catalogs/deepseek.json")).unwrap();
        let mut next = store.load().unwrap();
        next.relay_preserve_nonprovider_settings = !next.relay_preserve_nonprovider_settings;
        save_with_active_catalog_refresh(&store, &home, &next).unwrap();
        assert_eq!(std::fs::read(home.join("config.toml")).unwrap(), config_before);
        assert_eq!(std::fs::read(home.join("model-catalogs/deepseek.json")).unwrap(), catalog_before);
    }

    #[test]
    fn aggregate_catalog_refresh_preserves_external_transport_with_owned_pointer() {
        for aggregation in [true, false] {
            let (_temp, store, home) = publication_fixture();
            let config_path = home.join("config.toml");
            let mut config: toml_edit::DocumentMut =
                std::fs::read_to_string(&config_path).unwrap().parse().unwrap();
            config["model_providers"]["custom"]["base_url"] =
                toml_edit::value("http://127.0.0.1:58077/v1");
            std::fs::write(&config_path, config.to_string()).unwrap();
            let before = std::fs::read(&config_path).unwrap();
            let mut next = store.load().unwrap();
            next.aggregate_relay_profiles[0].model_aggregation_enabled = aggregation;
            next.relay_profiles[0].model_list.push_str("\nnew-model");
            save_with_active_catalog_refresh(&store, &home, &next).unwrap();
            assert_eq!(std::fs::read(&config_path).unwrap(), before);
        }
    }

    #[test]
    fn catalog_publication_failure_restores_settings_and_keeps_old_pointer() {
        let (_temp,store,home)=publication_fixture();let before=store.snapshot_raw_bytes().unwrap().unwrap();
        let config=std::fs::read(home.join("config.toml")).unwrap();let mut next=store.load().unwrap();next.relay_profiles[0].model_list.push_str("\nnew-fixture-model");
        let error=save_with_catalog_publisher(&store,&home,&next,|_,_|anyhow::bail!("injected publication failure")).unwrap_err();
        assert!(error.to_string().contains("发布失败"));assert_eq!(store.snapshot_raw_bytes().unwrap().unwrap(),before);
        assert_eq!(std::fs::read(home.join("config.toml")).unwrap(),config);
        assert!(!recover_pending_catalog_refresh(&store,&home).unwrap());
    }

    #[test]
    fn prepared_catalog_recovers_after_settings_commit_but_respects_external_config_changes() {
        for external_change in [false,true] {
            let (_temp,store,home)=publication_fixture();let mut next=store.load().unwrap();next.relay_profiles[0].model_list.push_str("\nnew-fixture-model");
            let aggregate=active_model_aggregation(&next).unwrap();let catalog=build_catalog_json(&next,&aggregate).unwrap();
            let digest=format!("{:x}",Sha256::digest(catalog.as_bytes()));let staged=home.join("model-catalogs").join(format!("aggregate-{}-{digest}.json",aggregate_publication_key(&aggregate.id)));
            let config=std::fs::read_to_string(home.join("config.toml")).unwrap();let journal=refresh_journal_path(&home,&aggregate.id);
            // 人工构造实际崩溃窗口：prepare已落盘、settings已提交，publish尚未执行。
            store.save_with_prepared_commit(&next,|revision| {
                std::fs::write(&staged,&catalog)?;
                std::fs::write(&journal,serde_json::to_vec(&json!({"status":"prepared","aggregateId":aggregate.id,
                    "settingsRevision":revision,"catalogPath":staged,"catalogHash":digest,
                    "previousConfigHash":format!("{:x}",Sha256::digest(config.as_bytes()))}))?)?;
                Ok(())
            },|_|Ok(())).unwrap();
            if external_change {std::fs::write(home.join("config.toml"),format!("{config}external_setting = true\n")).unwrap();}
            let before_recovery=std::fs::read(home.join("config.toml")).unwrap();
            assert_eq!(recover_pending_catalog_refresh(&store,&home).unwrap(),!external_change);
            if external_change {assert_eq!(std::fs::read(home.join("config.toml")).unwrap(),before_recovery);}
            else {
                assert_eq!(config_catalog_pointer(&std::fs::read_to_string(home.join("config.toml")).unwrap()),Some(staged.canonicalize().unwrap().to_string_lossy().to_string()));
                assert!(!recover_pending_catalog_refresh(&store,&home).unwrap());
            }
        }
    }

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
            sync_default_member_login: false,
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

    /// Kimi 官方成员按原生 Responses 转发时上游不认 tool_search，目录里要关掉，
    /// 已保存的元数据覆盖也不能重新打开；其他成员保持模板原值。
    #[test]
    fn kimi_official_responses_member_disables_search_tool() {
        let mut settings = settings();
        settings.relay_profiles.insert(
            0,
            RelayProfile {
                id: "kimi".to_string(),
                name: "Kimi 官方".to_string(),
                model: "kimi-k3".to_string(),
                upstream_base_url: "https://api.moonshot.cn/v1".to_string(),
                base_url: "https://api.moonshot.cn/v1".to_string(),
                api_key: "sk-kimi".to_string(),
                protocol: crate::settings::RelayProtocol::Responses,
                relay_mode: RelayMode::PureApi,
                // 管理器「保存此模型」存下的整份元数据会带着模板里的 true。
                model_metadata: r#"{"kimi-k3":{"supports_search_tool":true}}"#.to_string(),
                ..RelayProfile::default()
            },
        );
        settings.aggregate_relay_profiles[0]
            .members
            .push(AggregateRelayMember {
                relay_id: "kimi".to_string(),
                weight: 1,
            });
        let aggregate = settings.active_aggregate_relay_profile().unwrap();
        let catalog: Value =
            serde_json::from_str(&build_catalog_json(&settings, &aggregate).unwrap()).unwrap();
        let models = catalog["models"].as_array().unwrap();
        let kimi = models
            .iter()
            .find(|model| model["slug"] == "cpx-kimi/kimi-k3")
            .unwrap();
        assert_eq!(kimi["supports_search_tool"], false);
        let openai = models
            .iter()
            .find(|model| model["slug"] == "cpx-openai/gpt-5.6-sol")
            .unwrap();
        assert_eq!(openai["supports_search_tool"], true);
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

    /// 不带前缀的请求全部交给默认供应商，且**原样**转发请求里的 id。
    /// 带 `cpx-` 前缀的残留 ID 不走默认供应商，硬报错。
    #[test]
    fn default_provider_serves_unbound_models_verbatim() {
        let mut settings = settings();
        settings.aggregate_relay_profiles[0].default_relay_id = "openai".to_string();

        let bare = resolve_active_model(&settings, "gpt-9.9-unknown")
            .unwrap()
            .expect("配了默认供应商后，不带前缀的请求交给它");
        assert_eq!(bare.relay_id, "openai");
        assert_eq!(bare.upstream_model, "gpt-9.9-unknown");

        // 别的管理器（`ccs-xxx/model` 这种）留下的名字也原样转发，不改名成 deepseek-flash。
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


    /// 默认成员的模型用裸名发布，其他成员仍带 `cpx-` 前缀；
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
