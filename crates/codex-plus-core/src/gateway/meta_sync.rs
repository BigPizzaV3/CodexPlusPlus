//! 「从上游同步上下文窗口 / 思考等级」（#2807）：读供应商 `/models` 响应里每个模型的元数据，
//! 和这家现在的设置比较，列出差异（新值、旧值、来源字段）；用户勾选确认后才写入。
//!
//! 和「从上游同步模型列表」一样分两步：[`preview_provider_meta`] 只读不写，
//! [`apply_window_sync`] / [`apply_reasoning_sync`] 只写用户勾选的。不在后台自动改配置。
//!
//! 区分手动值：每次同步写入的值记在 [`GatewayProvider::upstream_meta`] 里。现值和记录一样
//! 说明是上次同步来的，默认勾选覆盖；不一样（或没有记录）就是用户手动设的，默认不勾。
//!
//! 写到哪里：窗口写进 `model_windows`（Agent 的模型列表带 `[窗口]` 后缀、路由按窗口排序都用它）；
//! 思考等级写进这家的模型资料（`model_metadata` 的 `supported_reasoning_levels` /
//! `default_reasoning_level`），生成 Codex 模型目录时覆盖进去。

use std::collections::BTreeMap;

use anyhow::bail;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::gateway::store::{GatewayProvider, GatewaySettings, GatewayStore, SyncedReasoning};
use crate::settings::RelayProfile;
use crate::upstream_model_meta::{self as meta, IgnoredReason, UpstreamModelMeta};

/// 模型现在的值从哪来。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CurrentSource {
    /// 用户单独给这个模型设的（手动值）。
    Custom,
    /// 上次从上游同步写入、之后没改过的。
    Synced,
    /// 这家供应商「其它模型」的默认窗口（只用于窗口）。
    ProviderDefault,
    /// 内置模型资料。
    Builtin,
    /// 没有任何设置。
    None,
}

/// 一个模型的窗口差异。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowChange {
    /// 列表里的模型名。
    pub model: String,
    /// 上游给的窗口。
    pub window: u64,
    /// 上游字段，如 `context_length`。
    pub source: String,
    /// 上游 `context_windows` 的可选档位（参考用）。
    pub options: Vec<u64>,
    /// 现在生效的窗口。
    pub current: Option<u64>,
    pub current_source: CurrentSource,
    /// 现值是用户手动设的：默认不覆盖。
    pub manual: bool,
}

/// 一个模型的思考等级差异。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReasoningChange {
    pub model: String,
    pub levels: Vec<String>,
    pub default_level: Option<String>,
    pub source: String,
    pub default_source: String,
    pub current_levels: Vec<String>,
    pub current_default: Option<String>,
    pub current_source: CurrentSource,
    pub manual: bool,
}

/// 被跳过的上游字段值（哪个模型、哪个字段、原值、原因）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IgnoredNote {
    pub model: String,
    pub field: String,
    pub value: String,
    pub reason: IgnoredReason,
}

/// 同步预览：只计算、不写入。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaSyncPreview {
    /// 上游返回了几个模型。
    pub upstream_models: usize,
    /// 列表里有几个模型在上游找到了。
    pub matched: usize,
    /// 其中几个上游给了窗口 / 思考等级。
    pub with_window: usize,
    pub with_reasoning: usize,
    /// 和现值不一样的（一样的不列）。
    pub windows: Vec<WindowChange>,
    pub reasoning: Vec<ReasoningChange>,
    /// 列表里模型的元数据中被跳过的字段值。
    pub ignored: Vec<IgnoredNote>,
}

/// 内置资料里一个模型的窗口和思考等级。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BuiltinMeta {
    pub window: Option<u64>,
    pub levels: Vec<String>,
    pub default_level: Option<String>,
}

/// 比较和记录用的键：小写、去掉 `[1M]` 后缀。
fn slug_key(model: &str) -> String {
    crate::model_suffix::parse_model_suffix(model.trim())
        .0
        .trim()
        .to_ascii_lowercase()
}

/// 模型名去掉 `[1M]` 后缀（保留大小写）。
fn slug(model: &str) -> String {
    crate::model_suffix::parse_model_suffix(model.trim())
        .0
        .trim()
        .to_string()
}

/// 这家单独给某个模型写的窗口（忽略大小写和后缀）。
fn own_window(provider: &GatewayProvider, model: &str) -> Option<u64> {
    let key = slug_key(model);
    provider
        .model_windows
        .iter()
        .find(|(name, _)| slug_key(name) == key)
        .map(|(_, window)| *window)
        .filter(|window| *window > 0)
}

/// 模型资料（原样的 JSON 对象）；空串为空对象，解析不了返回 None。
fn metadata_object(relay: &RelayProfile) -> Option<Map<String, Value>> {
    if relay.model_metadata.trim().is_empty() {
        return Some(Map::new());
    }
    match serde_json::from_str::<Value>(&relay.model_metadata) {
        Ok(Value::Object(map)) => Some(map),
        _ => None,
    }
}

/// 模型资料里这个模型的条目（忽略大小写和后缀）。
fn metadata_entry<'a>(map: &'a Map<String, Value>, model: &str) -> Option<&'a Map<String, Value>> {
    let key = slug_key(model);
    map.iter()
        .rev()
        .find(|(name, _)| slug_key(name) == key)
        .and_then(|(_, value)| value.as_object())
}

/// 用户在模型资料里给这个模型设的思考等级：（档位，默认档）；没设返回 None。
fn custom_reasoning(entry: Option<&Map<String, Value>>) -> Option<(Vec<String>, Option<String>)> {
    let levels = entry?.get("supported_reasoning_levels")?;
    let entry = entry?;
    let names: Vec<String> = levels
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    item.as_str()
                        .or_else(|| item.get("effort").and_then(Value::as_str))
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default();
    let levels = meta::normalize_levels(&names);
    let default = entry
        .get("default_reasoning_level")
        .and_then(Value::as_str)
        .and_then(meta::normalize_level)
        .map(str::to_string);
    Some((levels, default))
}

/// 用户选了「思考参数：不发送」：这家不给上游发思考参数，同步来的档位没有意义，当作手动设置。
fn reasoning_omitted(entry: Option<&Map<String, Value>>) -> bool {
    entry
        .and_then(|entry| entry.get("codex_plus_reasoning_parameters"))
        .and_then(Value::as_str)
        == Some("omit")
}

/// 按上游元数据算预览（纯计算，便于测试）。`builtin` 的键是 [`slug_key`]。
pub fn build_preview(
    provider: &GatewayProvider,
    metas: &[UpstreamModelMeta],
    builtin: &BTreeMap<String, BuiltinMeta>,
) -> MetaSyncPreview {
    let models = crate::gateway::route::relay_models(&provider.relay);
    let metadata = metadata_object(&provider.relay);
    let record = &provider.upstream_meta;
    let mut preview = MetaSyncPreview {
        upstream_models: metas.len(),
        ..MetaSyncPreview::default()
    };
    for model in &models {
        let key = slug_key(model);
        let Some(upstream) = metas.iter().find(|meta| slug_key(&meta.model) == key) else {
            continue;
        };
        preview.matched += 1;
        for ignored in &upstream.ignored {
            preview.ignored.push(IgnoredNote {
                model: model.clone(),
                field: ignored.field.clone(),
                value: ignored.value.clone(),
                reason: ignored.reason,
            });
        }
        let base = builtin.get(&key).cloned().unwrap_or_default();

        if let Some(window) = upstream.context_window {
            preview.with_window += 1;
            let (current, current_source) = match own_window(provider, model) {
                Some(own) if record.windows.get(&key) == Some(&own) => {
                    (Some(own), CurrentSource::Synced)
                }
                Some(own) => (Some(own), CurrentSource::Custom),
                None => match provider.context_window.filter(|window| *window > 0) {
                    Some(fallback) => (Some(fallback), CurrentSource::ProviderDefault),
                    None => match base.window {
                        Some(builtin) => (Some(builtin), CurrentSource::Builtin),
                        None => (None, CurrentSource::None),
                    },
                },
            };
            if current != Some(window) {
                preview.windows.push(WindowChange {
                    model: model.clone(),
                    window,
                    source: upstream.context_window_source.clone(),
                    options: upstream.context_window_options.clone(),
                    current,
                    current_source,
                    manual: current_source == CurrentSource::Custom,
                });
            }
        }

        if !upstream.reasoning_levels.is_empty() {
            preview.with_reasoning += 1;
            let entry = metadata.as_ref().and_then(|map| metadata_entry(map, model));
            let omitted = reasoning_omitted(entry);
            let (current_levels, current_default, current_source) = match custom_reasoning(entry) {
                Some((levels, default)) => {
                    let synced = SyncedReasoning {
                        levels: levels.clone(),
                        default_level: default.clone(),
                    };
                    let source = if !omitted && record.reasoning.get(&key) == Some(&synced) {
                        CurrentSource::Synced
                    } else {
                        CurrentSource::Custom
                    };
                    (levels, default, source)
                }
                None if omitted => (Vec::new(), None, CurrentSource::Custom),
                None if base.levels.is_empty() => (Vec::new(), None, CurrentSource::None),
                None => (
                    base.levels.clone(),
                    base.default_level.clone(),
                    CurrentSource::Builtin,
                ),
            };
            // 上游没给默认档时只比档位；给了就连默认档一起比。
            let default_differs = upstream
                .default_reasoning_level
                .as_ref()
                .is_some_and(|level| current_default.as_ref() != Some(level));
            if current_levels != upstream.reasoning_levels || default_differs {
                preview.reasoning.push(ReasoningChange {
                    model: model.clone(),
                    levels: upstream.reasoning_levels.clone(),
                    default_level: upstream.default_reasoning_level.clone(),
                    source: upstream.reasoning_levels_source.clone(),
                    default_source: upstream.default_reasoning_source.clone(),
                    current_levels,
                    current_default,
                    current_source,
                    manual: current_source == CurrentSource::Custom,
                });
            }
        }
    }
    preview
}

/// 内置资料里各模型的窗口和思考等级（不含用户改过的）。算不出来时返回空表，不影响预览。
pub fn builtin_meta(provider: &GatewayProvider) -> BTreeMap<String, BuiltinMeta> {
    let mut relay = provider.relay.clone();
    relay.model_metadata.clear();
    relay.model_windows.clear();
    relay.context_window.clear();
    let Ok(catalog) = crate::aggregate_catalog::build_member_catalog(&relay) else {
        return BTreeMap::new();
    };
    let mut builtin = BTreeMap::new();
    for entry in catalog["models"].as_array().into_iter().flatten() {
        let Some(model) = entry["slug"].as_str() else {
            continue;
        };
        let names: Vec<String> = entry["supported_reasoning_levels"]
            .as_array()
            .map(|levels| {
                levels
                    .iter()
                    .filter_map(|level| {
                        level["effort"]
                            .as_str()
                            .or_else(|| level.as_str())
                            .map(str::to_string)
                    })
                    .collect()
            })
            .unwrap_or_default();
        builtin.insert(
            slug_key(model),
            BuiltinMeta {
                window: entry["context_window"]
                    .as_u64()
                    .filter(|window| *window > 0),
                levels: meta::normalize_levels(&names),
                default_level: entry["default_reasoning_level"]
                    .as_str()
                    .and_then(meta::normalize_level)
                    .map(str::to_string),
            },
        );
    }
    builtin
}

/// 拉上游 `/models` 并解析出每个模型的元数据。订阅账号、OpenCode、官方登录的模型接口
/// 不是通用的 `/models`，不提供这些元数据，直接报错说明。
pub async fn upstream_model_metas(relay: &RelayProfile) -> anyhow::Result<Vec<UpstreamModelMeta>> {
    if crate::official_login::member_uses_official_login_transport(relay)
        || crate::gateway::accounts::relay_account_kind(relay).is_some()
        || crate::gateway::opencode::plan_of(&relay.base_url).is_some()
    {
        bail!("这类供应商的模型接口不提供上下文窗口和思考等级，请手动填写");
    }
    let payload = crate::model_catalog::fetch_relay_profile_models_payload(relay).await?;
    let metas = meta::parse_upstream_model_metas(&payload);
    if metas.is_empty() {
        bail!("上游没有返回模型");
    }
    Ok(metas)
}

/// 「从上游同步上下文窗口 / 思考等级」的第一步：用存下的凭据拉 `/models`，算出预览。只读。
pub async fn preview_provider_meta(
    store: &GatewayStore,
    id: &str,
) -> anyhow::Result<MetaSyncPreview> {
    let relay = store
        .load()?
        .provider(id)
        .map(|provider| provider.relay.clone())
        .ok_or_else(|| anyhow::anyhow!("供应商不存在：{id}"))?;
    let metas = upstream_model_metas(&relay).await?;
    // 拉取期间可能有别的写入：按最新数据比较。
    let gateway = store.load()?;
    let provider = gateway
        .provider(id)
        .ok_or_else(|| anyhow::anyhow!("供应商不存在：{id}"))?;
    Ok(build_preview(provider, &metas, &builtin_meta(provider)))
}

/// 用户勾选要写入的一个窗口。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowPick {
    pub model: String,
    pub window: u64,
}

/// 用户勾选要写入的一组思考等级。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReasoningPick {
    pub model: String,
    pub levels: Vec<String>,
    #[serde(default)]
    pub default_level: Option<String>,
}

/// 列表里和 `model` 同名的模型（忽略大小写和后缀），返回去掉后缀的名字。
fn listed_slug(provider: &GatewayProvider, model: &str) -> Option<String> {
    let key = slug_key(model);
    if key.is_empty() {
        return None;
    }
    crate::gateway::route::relay_models(&provider.relay)
        .iter()
        .find(|item| slug_key(item) == key)
        .map(|item| slug(item))
}

fn with_provider<T>(
    store: &GatewayStore,
    id: &str,
    change: impl FnOnce(&mut GatewayProvider) -> anyhow::Result<T>,
) -> anyhow::Result<(GatewaySettings, T)> {
    store.update(|gateway| {
        let Some(provider) = gateway
            .providers
            .iter_mut()
            .find(|provider| provider.relay.id == id)
        else {
            bail!("供应商不存在：{id}");
        };
        change(provider)
    })
}

/// 写入用户勾选的窗口：按模型写进 `model_windows`（同名的旧记录一并替换），并记下是同步来的。
/// 不在列表里的模型、不合理的窗口跳过。返回实际写了几个模型。
pub fn apply_window_sync(
    store: &GatewayStore,
    id: &str,
    picks: &[WindowPick],
) -> anyhow::Result<(GatewaySettings, usize)> {
    with_provider(store, id, |provider| {
        let mut written = 0;
        for pick in picks {
            if !meta::is_sane_window(pick.window) {
                continue;
            }
            let Some(name) = listed_slug(provider, &pick.model) else {
                continue;
            };
            let key = slug_key(&name);
            provider
                .model_windows
                .retain(|model, _| slug_key(model) != key);
            provider.model_windows.insert(name, pick.window);
            provider.upstream_meta.windows.insert(key, pick.window);
            written += 1;
        }
        Ok(written)
    })
}

/// 写入用户勾选的思考等级：整组替换这个模型的 `supported_reasoning_levels`；上游给了默认档
/// （且在档位里）就用它，没给就保留原来的默认档（不在新档位里则去掉）。模型资料里的其它字段不动。
/// 模型资料解析不了时报错、不改（不能把用户的资料覆盖掉）。返回实际写了几个模型。
pub fn apply_reasoning_sync(
    store: &GatewayStore,
    id: &str,
    picks: &[ReasoningPick],
) -> anyhow::Result<(GatewaySettings, usize)> {
    with_provider(store, id, |provider| {
        let Some(mut map) = metadata_object(&provider.relay) else {
            bail!("模型资料不是有效的 JSON，请先在「思考等级」里修正");
        };
        let mut records = Vec::new();
        for pick in picks {
            let levels = meta::normalize_levels(&pick.levels);
            if levels.is_empty() {
                continue;
            }
            let Some(name) = listed_slug(provider, &pick.model) else {
                continue;
            };
            let key = slug_key(&name);
            // 已有条目沿用原来的键（可能带大小写或后缀），没有就用小写名。
            let entry_key = map
                .keys()
                .rev()
                .find(|existing| slug_key(existing) == key)
                .cloned()
                .unwrap_or_else(|| key.clone());
            let mut entry = map
                .get(&entry_key)
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            let wanted = pick
                .default_level
                .as_deref()
                .and_then(meta::normalize_level)
                .map(str::to_string)
                .filter(|level| levels.contains(level));
            let kept = entry
                .get("default_reasoning_level")
                .and_then(Value::as_str)
                .and_then(meta::normalize_level)
                .map(str::to_string)
                .filter(|level| levels.contains(level));
            let default = wanted.or(kept);
            entry.insert(
                "supported_reasoning_levels".into(),
                Value::Array(
                    levels
                        .iter()
                        .map(|level| {
                            json!({
                                "effort": level,
                                "description": meta::reasoning_level_description(level),
                            })
                        })
                        .collect(),
                ),
            );
            match &default {
                Some(level) => {
                    entry.insert("default_reasoning_level".into(), json!(level));
                }
                None => {
                    entry.remove("default_reasoning_level");
                }
            }
            map.insert(entry_key, Value::Object(entry));
            records.push((
                key,
                SyncedReasoning {
                    levels,
                    default_level: default,
                },
            ));
        }
        if records.is_empty() {
            return Ok(0);
        }
        let metadata = serde_json::to_string_pretty(&Value::Object(map))?;
        crate::relay_config::parse_model_metadata_map(&metadata)?;
        provider.relay.model_metadata = metadata;
        let written = records.len();
        for (key, synced) in records {
            provider.upstream_meta.reasoning.insert(key, synced);
        }
        Ok(written)
    })
}
