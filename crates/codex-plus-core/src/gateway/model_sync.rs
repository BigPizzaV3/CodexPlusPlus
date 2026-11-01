//! 供应商的模型列表与价格：用存下的凭据重新拉上游模型、手动保存模型列表、
//! 按公开价格表给没填价的模型补价格、定期检查上游有没有新模型。
//!
//! 检查结果记在 [`GatewayProvider`] 的 `upstream_new` / `upstream_gone` 里，界面据此提示；
//! 用户保存模型列表后清掉。后台检查只提示，不自动改用户整理过的列表；用户点「从上游同步」
//! 先看预览（[`preview_upstream_sync`]），勾选要删、要加的再写入（[`apply_upstream_sync`]）。

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::bail;
use serde::Serialize;

use crate::gateway::accounts;
use crate::gateway::store::{GatewaySettings, GatewayStore, ProviderPrice};
use crate::settings::RelayProfile;

/// 上游模型列表和现有列表的差异。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelDiff {
    /// 上游现在的全部模型。
    pub upstream: Vec<String>,
    /// 上游有、列表里没有的。
    pub added: Vec<String>,
    /// 列表里有、上游已经没有的。
    pub removed: Vec<String>,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

/// 模型名去掉 `[1M]` 这类窗口后缀：列表里的 `claude-opus-4-6[1M]` 和上游的 `claude-opus-4-6` 是同一个模型。
fn model_slug(model: &str) -> String {
    crate::model_suffix::parse_model_suffix(model.trim())
        .0
        .trim()
        .to_string()
}

/// `list` 里有没有和 `model` 同名的（忽略大小写和窗口后缀）。
fn has_model(list: &[String], model: &str) -> bool {
    let slug = model_slug(model);
    list.iter()
        .any(|item| model_slug(item).eq_ignore_ascii_case(&slug))
}

/// 比较时忽略大小写和 `[1M]` 这类窗口后缀。
pub fn diff_models(current: &[String], upstream: &[String]) -> ModelDiff {
    let has = |list: &[String], model: &str| has_model(list, model);
    ModelDiff {
        upstream: upstream.to_vec(),
        added: upstream
            .iter()
            .filter(|model| !has(current, model))
            .cloned()
            .collect(),
        removed: current
            .iter()
            .filter(|model| !has(upstream, model))
            .cloned()
            .collect(),
    }
}

/// 用存下的凭据重新拉一个网关供应商的模型：订阅账号走各自的接口，OpenCode 用目录，
/// 其它供应商拉 `/models`。
pub async fn upstream_models(relay: &RelayProfile) -> anyhow::Result<Vec<String>> {
    if crate::official_login::member_uses_official_login_transport(relay) {
        return Ok(crate::model_catalog::fetch_relay_profile_model_ids(relay)
            .await?
            .0);
    }
    // 模型列表也按供应商自己的代理设置拉（#2775）。
    let proxy = crate::relay_proxy::proxy_for_relay(relay);
    let models = match accounts::relay_account_kind(relay).as_deref() {
        Some("copilot") => {
            let prepared = accounts::prepare_relay(relay).await?;
            let token = accounts::CopilotToken {
                token: prepared.api_key.clone(),
                expires_at_ms: 0,
                api_base: prepared.base_url.trim_end_matches('#').to_string(),
                sku: String::new(),
            };
            accounts::copilot_models(&token, proxy.as_deref()).await
        }
        Some("grok") => {
            let prepared = accounts::prepare_relay(relay).await?;
            accounts::grok_models(&prepared.api_key, proxy.as_deref()).await
        }
        Some("chatgpt-api") => {
            let prepared = accounts::prepare_relay(relay).await?;
            accounts::chatgpt_api_models(
                &prepared.api_key,
                &crate::relay_config::default_codex_home_dir(),
                proxy.as_deref(),
            )
            .await
        }
        _ => match crate::gateway::opencode::plan_of(&relay.base_url) {
            Some(plan) => crate::gateway::opencode::catalog_models(plan).await?,
            None => {
                crate::model_catalog::fetch_relay_profile_model_ids(relay)
                    .await?
                    .0
            }
        },
    };
    if models.is_empty() {
        bail!("上游没有返回模型");
    }
    Ok(models)
}

/// 拉上游模型并和现有列表比较；结果记进供应商（界面据此提示新模型）。
pub async fn check_provider(store: &GatewayStore, id: &str) -> anyhow::Result<ModelDiff> {
    let relay = store
        .load()?
        .provider(id)
        .map(|provider| provider.relay.clone())
        .ok_or_else(|| anyhow::anyhow!("供应商不存在：{id}"))?;
    let fetched = upstream_models(&relay).await;
    // 拉的这段时间里可能有别的写入：在锁里重新读一次再改。
    let (_, result) = store.update(|gateway| {
        let Some(provider) = gateway
            .providers
            .iter_mut()
            .find(|provider| provider.relay.id == id)
        else {
            bail!("供应商不存在：{id}");
        };
        provider.models_checked_at_ms = now_ms();
        Ok(fetched.map(|upstream| {
            let diff = diff_models(
                &crate::gateway::route::relay_models(&provider.relay),
                &upstream,
            );
            provider.upstream_new = diff.added.clone();
            provider.upstream_gone = diff.removed.clone();
            diff
        }))
    })?;
    result
}

/// 去掉上游列表里的空名；一个都不剩时报错——拉取失败或上游临时返回空，不能据此清空用户的列表。
fn clean_upstream(upstream: &[String]) -> anyhow::Result<Vec<String>> {
    let upstream: Vec<String> = upstream
        .iter()
        .map(|model| model.trim())
        .filter(|model| !model.is_empty())
        .map(str::to_string)
        .collect();
    if upstream.is_empty() {
        bail!("上游没有返回模型，列表保持不变");
    }
    Ok(upstream)
}

/// 「从上游同步」的预览（#2785）：上游列表和现有列表的差异，只计算、不写入。
/// 界面据此让用户勾选要删的和要加的，确认后用 [`apply_upstream_sync`] 写入。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSyncPreview {
    /// 上游现在的全部模型。
    pub upstream: Vec<String>,
    /// 现在的列表（默认模型在最前）。
    pub current: Vec<String>,
    /// 现在的默认模型。
    pub default_model: String,
    /// 上游有、列表里没有的（忽略大小写和 `[1M]` 后缀；上游自己重名的只算一次）。
    pub added: Vec<String>,
    /// 列表里有、上游已经没有的。
    pub removed: Vec<String>,
}

/// 实际改动：加进列表的和从列表删掉的模型。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelChanges {
    pub added: Vec<String>,
    pub removed: Vec<String>,
}

/// 按上游列表算出同步预览（不改任何数据）。上游列表为空时报错。
pub fn preview_upstream_sync(
    store: &GatewayStore,
    id: &str,
    upstream: &[String],
) -> anyhow::Result<ModelSyncPreview> {
    let upstream = clean_upstream(upstream)?;
    let gateway = store.load()?;
    let provider = gateway
        .provider(id)
        .ok_or_else(|| anyhow::anyhow!("供应商不存在：{id}"))?;
    let current = crate::gateway::route::relay_models(&provider.relay);
    let diff = diff_models(&current, &upstream);
    let mut added: Vec<String> = Vec::new();
    for model in diff.added {
        if !has_model(&added, &model) {
            added.push(model);
        }
    }
    Ok(ModelSyncPreview {
        default_model: crate::relay_config::relay_profile_model(&provider.relay)
            .trim()
            .to_string(),
        upstream,
        current,
        added,
        removed: diff.removed,
    })
}

/// 改一个供应商的模型列表：删掉 `remove` 里的（忽略大小写和窗口后缀），把 `add` 里列表还没有的追加到末尾，
/// 其余保持原顺序。默认模型还在就不变，被删了换成列表第一个。被删模型单独填的价格、上下文窗口和
/// 模型资料（思考等级等）一并清掉，同名的还留在列表里（比如只差 `[1M]` 后缀）的不动。
/// 改完列表就是用户看过的，清掉上游检查的提示。删完一个不剩时报错、不改。
fn apply_model_changes(
    provider: &mut crate::gateway::store::GatewayProvider,
    remove: &[String],
    add: &[String],
) -> anyhow::Result<ModelChanges> {
    let current = crate::gateway::route::relay_models(&provider.relay);
    let removed: Vec<String> = current
        .iter()
        .filter(|model| has_model(remove, model))
        .cloned()
        .collect();
    let mut list: Vec<String> = current
        .iter()
        .filter(|model| !has_model(remove, model))
        .cloned()
        .collect();
    // 重名（只差大小写或后缀）的只加一次，报告的新增也按实际加进去的算。
    let mut added = Vec::new();
    for model in add
        .iter()
        .map(|model| model.trim())
        .filter(|model| !model.is_empty())
    {
        if !has_model(&list, model) {
            list.push(model.to_string());
            added.push(model.to_string());
        }
    }
    if list.is_empty() {
        bail!("改完列表里一个模型都不剩了，至少保留一个");
    }
    // 当前默认模型按 config 快照优先的口径取（同 set_provider_models）；
    // 默认模型同时写进快照，只改字段的话读盘时会被快照里的旧值改回来。
    let default = crate::relay_config::relay_profile_model(&provider.relay);
    let default = list
        .iter()
        .find(|model| model.eq_ignore_ascii_case(default.trim()))
        .or(list.first())
        .cloned()
        .unwrap_or_default();
    crate::relay_config::set_relay_profile_model(&mut provider.relay, &default)?;
    provider.relay.model_list = list.join("\n");
    // 被删的模型单独填的配置：列表里已经没有同名模型的才清。
    let gone = |name: &str| has_model(&removed, name) && !has_model(&list, name);
    provider.model_prices.retain(|name, _| !gone(name));
    provider.model_windows.retain(|name, _| !gone(name));
    provider.upstream_meta.retain_models(|name| !gone(name));
    provider.relay.model_metadata = prune_model_metadata(&provider.relay.model_metadata, &gone);
    provider.upstream_new.clear();
    provider.upstream_gone.clear();
    Ok(ModelChanges { added, removed })
}

/// 应用用户在同步预览里勾选的改动（#2785）：只删 `remove` 里的、只加 `add` 里的，
/// 没勾的上游新增不加、没勾的下架模型留着。规则见 [`apply_model_changes`]。返回实际的改动。
pub fn apply_upstream_sync(
    store: &GatewayStore,
    id: &str,
    remove: &[String],
    add: &[String],
) -> anyhow::Result<(GatewaySettings, ModelChanges)> {
    store.update(|gateway| {
        let Some(provider) = gateway
            .providers
            .iter_mut()
            .find(|provider| provider.relay.id == id)
        else {
            bail!("供应商不存在：{id}");
        };
        apply_model_changes(provider, remove, add)
    })
}

/// 按上游同步模型列表（多删少补，不经用户挑选）：上游已没有的从列表删掉，上游新增的全部追加到末尾，
/// 其余保持原顺序；默认模型、被删模型资料的处理同 [`apply_model_changes`]。网页导入链接加进池后
/// 自动拉模型时用；供应商详情里的「从上游同步」走 [`preview_upstream_sync`] + [`apply_upstream_sync`]。
///
/// 返回实际新增和删掉的模型（在锁里按最新列表重新比较，`upstream` 为上游的全部模型）。
/// 上游列表为空时报错、不改列表：拉取失败或上游临时返回空，不能把用户的列表清空。
pub fn sync_upstream_models(
    store: &GatewayStore,
    id: &str,
    upstream: &[String],
) -> anyhow::Result<(GatewaySettings, ModelDiff)> {
    let upstream = clean_upstream(upstream)?;
    store.update(|gateway| {
        let Some(provider) = gateway
            .providers
            .iter_mut()
            .find(|provider| provider.relay.id == id)
        else {
            bail!("供应商不存在：{id}");
        };
        let diff = diff_models(
            &crate::gateway::route::relay_models(&provider.relay),
            &upstream,
        );
        let changes = apply_model_changes(provider, &diff.removed, &diff.added)?;
        Ok(ModelDiff {
            upstream: upstream.clone(),
            added: changes.added,
            removed: changes.removed,
        })
    })
}

/// 从模型资料 JSON 里去掉 `gone` 判定为已删除的模型。解析不了就原样返回（不因此丢掉用户的资料）。
fn prune_model_metadata(metadata: &str, gone: &dyn Fn(&str) -> bool) -> String {
    let Ok(serde_json::Value::Object(mut map)) =
        serde_json::from_str::<serde_json::Value>(metadata)
    else {
        return metadata.to_string();
    };
    let before = map.len();
    map.retain(|name, _| !gone(name));
    if map.len() == before {
        return metadata.to_string();
    }
    if map.is_empty() {
        return String::new();
    }
    serde_json::to_string_pretty(&serde_json::Value::Object(map))
        .unwrap_or_else(|_| metadata.to_string())
}

/// 拉上游模型，按上游多删少补（见 [`sync_upstream_models`]），不经用户挑选。
/// 返回的差异是实际新增和删掉的模型。
pub async fn fetch_provider_models(
    store: &GatewayStore,
    id: &str,
) -> anyhow::Result<(GatewaySettings, ModelDiff)> {
    let diff = check_provider(store, id).await?;
    sync_upstream_models(store, id, &diff.upstream)
}

/// 供应商详情里「从上游同步」的第一步：用存下的凭据拉上游模型（订阅账号、OpenCode 等各走
/// [`upstream_models`] 里自己的接口），算出预览。只读，不改列表、也不记检查结果。
pub async fn preview_provider_models(
    store: &GatewayStore,
    id: &str,
) -> anyhow::Result<ModelSyncPreview> {
    let relay = store
        .load()?
        .provider(id)
        .map(|provider| provider.relay.clone())
        .ok_or_else(|| anyhow::anyhow!("供应商不存在：{id}"))?;
    let upstream = upstream_models(&relay).await?;
    preview_upstream_sync(store, id, &upstream)
}

/// 检查所有打开的、超过 `max_age` 没检查过的供应商。单个失败不影响其它。返回检查了几个。
pub async fn check_due(store: &GatewayStore, max_age: Duration) -> usize {
    let Ok(gateway) = store.load() else {
        return 0;
    };
    let due: Vec<String> = gateway
        .providers
        .iter()
        .filter(|provider| {
            provider.enabled
                && now_ms().saturating_sub(provider.models_checked_at_ms)
                    >= max_age.as_millis() as u64
        })
        .map(|provider| provider.relay.id.clone())
        .collect();
    for id in &due {
        let _ = check_provider(store, id).await;
    }
    due.len()
}

/// 常驻任务：启动时补一次价格，之后每小时看一眼，超过一天没检查的供应商重新拉上游模型。
pub async fn watch(store: GatewayStore) {
    let _ = fill_missing_prices(&store, None).await;
    loop {
        check_due(&store, Duration::from_secs(24 * 60 * 60)).await;
        tokio::time::sleep(Duration::from_secs(60 * 60)).await;
    }
}

/// 保存手动编辑或刷新后的模型列表：默认模型不在列表里就换成第一个；清掉上游检查的提示。
pub fn set_provider_models(
    store: &GatewayStore,
    id: &str,
    models: &[String],
    default_model: Option<&str>,
) -> anyhow::Result<GatewaySettings> {
    let mut list: Vec<String> = Vec::new();
    for model in models {
        let model = model.trim();
        if !model.is_empty() && !list.iter().any(|item| item.eq_ignore_ascii_case(model)) {
            list.push(model.to_string());
        }
    }
    let (gateway, ()) = store.update(|gateway| {
        let Some(provider) = gateway
            .providers
            .iter_mut()
            .find(|provider| provider.relay.id == id)
        else {
            bail!("供应商不存在：{id}");
        };
        // 当前默认模型按 config 快照优先的口径取，和读盘、路由看到的一致。
        let current = crate::relay_config::relay_profile_model(&provider.relay);
        // 指定了默认模型（且在列表里）就用它；否则原来的默认模型还在就保留，不在就换成第一个。
        let wanted = default_model
            .map(str::trim)
            .and_then(|model| list.iter().find(|item| item.eq_ignore_ascii_case(model)))
            .cloned();
        let model = match wanted {
            Some(model) => model,
            None => list
                .iter()
                .find(|model| model.eq_ignore_ascii_case(current.trim()))
                .or(list.first())
                .cloned()
                .unwrap_or_default(),
        };
        // 默认模型同时写进 config 快照；只改字段的话读盘时会被快照里的旧值改回来。
        crate::relay_config::set_relay_profile_model(&mut provider.relay, &model)?;
        provider.relay.model_list = list.join("\n");
        provider.upstream_new.clear();
        provider.upstream_gone.clear();
        Ok(())
    })?;
    Ok(gateway)
}

/// 按公开价格表给没填价的模型补价格。只补「供应商没有统一价格、这个模型也没单独填」的；
/// `only` 为 `None` 时处理全部供应商。返回补了几个模型。
/// 公开价格表的一条匹配 → 供应商价格（没写币种的按美元）。
fn price_from_match(found: crate::model_pricing::PriceMatch) -> ProviderPrice {
    ProviderPrice {
        currency: if found.currency.is_empty() {
            "USD".to_string()
        } else {
            found.currency
        },
        input: found.input,
        cached_input: found.cached_input,
        output: found.output,
    }
}

/// 一键同步：按公开价格表更新一个供应商所有模型的价格。查得到的覆盖（包括手动填过的），
/// 查不到的保留原值。返回（查到几个，一共几个模型）。
pub async fn sync_public_prices(store: &GatewayStore, id: &str) -> anyhow::Result<(usize, usize)> {
    let relay = store
        .load()?
        .provider(id)
        .map(|provider| provider.relay.clone())
        .ok_or_else(|| anyhow::anyhow!("供应商不存在：{id}"))?;
    let models = crate::gateway::route::relay_models(&relay);
    let (catalog, _) = crate::model_pricing::load_catalog(false).await?;
    let found: Vec<(String, ProviderPrice)> = models
        .iter()
        .filter_map(|model| {
            catalog
                .lookup(&relay.base_url, model)
                .map(|hit| (model.clone(), price_from_match(hit)))
        })
        .collect();
    if !found.is_empty() {
        store.update(|gateway| {
            let Some(provider) = gateway
                .providers
                .iter_mut()
                .find(|provider| provider.relay.id == id)
            else {
                bail!("供应商不存在：{id}");
            };
            for (model, price) in &found {
                // 大小写不同的旧记录一并替换，免得同一个模型存两份。
                provider
                    .model_prices
                    .retain(|name, _| !name.eq_ignore_ascii_case(model));
                provider.model_prices.insert(model.clone(), price.clone());
            }
            Ok(())
        })?;
    }
    Ok((found.len(), models.len()))
}

pub async fn fill_missing_prices(
    store: &GatewayStore,
    only: Option<&str>,
) -> anyhow::Result<usize> {
    let gateway = store.load()?;
    let wanted: Vec<(String, String, Vec<String>)> = gateway
        .providers
        .iter()
        .filter(|provider| {
            only.is_none_or(|id| provider.relay.id == id) && provider.price.is_none()
        })
        .map(|provider| {
            let missing = crate::gateway::route::relay_models(&provider.relay)
                .into_iter()
                .filter(|model| provider.price_for(model).is_none())
                .collect::<Vec<_>>();
            (
                provider.relay.id.clone(),
                provider.relay.base_url.clone(),
                missing,
            )
        })
        .filter(|(_, _, missing)| !missing.is_empty())
        .collect();
    if wanted.is_empty() {
        return Ok(0);
    }
    let (catalog, _) = crate::model_pricing::load_catalog(false).await?;
    // 拉价格表期间可能有别的写入：在锁里重新读一次再补。
    let (_, filled) = store.update(|gateway| {
        let mut filled = 0;
        for (id, base_url, missing) in wanted {
            let Some(provider) = gateway
                .providers
                .iter_mut()
                .find(|provider| provider.relay.id == id)
            else {
                continue;
            };
            for model in missing {
                if provider.price_for(&model).is_some() {
                    continue;
                }
                if let Some(found) = catalog.lookup(&base_url, &model) {
                    provider.model_prices.insert(model, price_from_match(found));
                    filled += 1;
                }
            }
        }
        Ok(filled)
    })?;
    Ok(filled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::store::GatewayProvider;

    #[test]
    fn diff_ignores_case_and_reports_both_sides() {
        let current = vec!["GPT-6".to_string(), "old".to_string()];
        let upstream = vec!["gpt-6".to_string(), "new".to_string()];
        let diff = diff_models(&current, &upstream);
        assert_eq!(diff.added, vec!["new"]);
        assert_eq!(diff.removed, vec!["old"]);
    }

    #[test]
    fn saving_models_keeps_default_when_present_and_clears_hints() {
        let temp = tempfile::tempdir().unwrap();
        let store = GatewayStore::at(temp.path().join("gateway.json"));
        let mut gateway = GatewaySettings::default();
        gateway.providers.push(GatewayProvider {
            enabled: true,
            source: String::new(),
            price: None,
            model_prices: Default::default(),
            model_windows: Default::default(),
            context_window: None,
            proxy: String::new(),
            key_rpm: None,
            key_concurrency: None,
            upstream_new: vec!["b".into()],
            upstream_gone: vec!["x".into()],
            models_checked_at_ms: 1,
            newapi_access: None,
            upstream_meta: Default::default(),
            relay: RelayProfile {
                id: "p".into(),
                model: "a".into(),
                model_list: "a\nx".into(),
                ..RelayProfile::default()
            },
            extra: Default::default(),
        });
        store.save(&gateway).unwrap();

        let saved = set_provider_models(
            &store,
            "p",
            &[" a ".into(), "b".into(), "B".into(), "".into()],
            None,
        )
        .unwrap();
        let provider = saved.provider("p").unwrap();
        assert_eq!(provider.relay.model_list, "a\nb");
        assert_eq!(provider.relay.model, "a");
        assert!(provider.upstream_new.is_empty() && provider.upstream_gone.is_empty());

        let saved = set_provider_models(&store, "p", &["c".into()], None).unwrap();
        assert_eq!(
            saved.provider("p").unwrap().relay.model,
            "c",
            "默认模型不在列表里就换成第一个"
        );

        let saved = set_provider_models(&store, "p", &["c".into(), "d".into()], Some("d")).unwrap();
        assert_eq!(saved.provider("p").unwrap().relay.model, "d");
    }

    fn provider_with(relay: RelayProfile) -> GatewayProvider {
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
            models_checked_at_ms: 1,
            newapi_access: None,
            upstream_meta: Default::default(),
            relay,
            extra: Default::default(),
        }
    }

    fn price(input: f64) -> ProviderPrice {
        serde_json::from_value(serde_json::json!({ "input": input, "output": input })).unwrap()
    }

    #[test]
    fn diff_treats_window_suffix_as_the_same_model() {
        let diff = diff_models(&["claude-opus-4-6[1M]".into()], &["Claude-Opus-4-6".into()]);
        assert!(diff.added.is_empty() && diff.removed.is_empty(), "{diff:?}");
    }

    #[test]
    fn syncing_upstream_models_removes_gone_and_appends_new() {
        let temp = tempfile::tempdir().unwrap();
        let store = GatewayStore::at(temp.path().join("gateway.json"));
        let mut gateway = GatewaySettings::default();
        let mut provider = provider_with(RelayProfile {
            id: "p".into(),
            // 默认模型存在配置快照里（`model` 字段不落盘）。
            config_contents: "model = \"b\"\n".into(),
            model_list: "a\nb\nold\nlong".into(),
            model_metadata: r#"{"old":{"default_reasoning_level":"high"},"a":{"default_reasoning_level":"low"}}"#.into(),
            ..RelayProfile::default()
        });
        provider.upstream_new = vec!["c".into()];
        provider.upstream_gone = vec!["old".into()];
        provider.model_prices.insert("old".into(), price(1.0));
        provider.model_prices.insert("A".into(), price(2.0));
        provider.model_windows.insert("old[1M]".into(), 1_000_000);
        provider.model_windows.insert("long".into(), 1_000_000);
        gateway.providers.push(provider);
        store.save(&gateway).unwrap();

        let (saved, diff) = sync_upstream_models(
            &store,
            "p",
            &[
                "A".into(),
                "b".into(),
                "long".into(),
                "codex-auto-review".into(),
                "B".into(),
            ],
        )
        .unwrap();
        assert_eq!(diff.added, vec!["codex-auto-review"]);
        assert_eq!(diff.removed, vec!["old"]);
        let provider = saved.provider("p").unwrap();
        // 上游已没有的 old 删掉，其余保持原顺序（默认模型在前），新增的追加在后面。
        assert_eq!(provider.relay.model_list, "b\na\nlong\ncodex-auto-review");
        assert_eq!(
            crate::relay_config::relay_profile_model(&provider.relay),
            "b"
        );
        assert!(provider.upstream_new.is_empty() && provider.upstream_gone.is_empty());
        // 被删模型的价格、窗口、模型资料一并清掉，留下的不动。
        assert_eq!(provider.model_prices.keys().collect::<Vec<_>>(), vec!["A"]);
        assert_eq!(
            provider.model_windows.keys().collect::<Vec<_>>(),
            vec!["long"]
        );
        assert!(
            !provider.relay.model_metadata.contains("old"),
            "{}",
            provider.relay.model_metadata
        );
        assert!(
            provider.relay.model_metadata.contains("\"a\""),
            "{}",
            provider.relay.model_metadata
        );
    }

    #[test]
    fn syncing_upstream_models_falls_back_when_the_default_is_gone() {
        let temp = tempfile::tempdir().unwrap();
        let store = GatewayStore::at(temp.path().join("gateway.json"));
        let mut gateway = GatewaySettings::default();
        gateway.providers.push(provider_with(RelayProfile {
            id: "p".into(),
            config_contents: "model = \"retired\"\n".into(),
            model_list: "retired\nkeep".into(),
            ..RelayProfile::default()
        }));
        store.save(&gateway).unwrap();

        let (saved, _) =
            sync_upstream_models(&store, "p", &["fresh".into(), "keep".into()]).unwrap();
        let relay = &saved.provider("p").unwrap().relay;
        assert_eq!(relay.model_list, "keep\nfresh");
        assert_eq!(crate::relay_config::relay_profile_model(relay), "keep");
        // 重新读盘后旧默认模型不能被快照改回来、也不能并回列表。
        let loaded = store.load().unwrap();
        assert_eq!(
            crate::gateway::route::relay_models(&loaded.provider("p").unwrap().relay),
            vec!["keep", "fresh"]
        );
    }

    #[test]
    fn syncing_with_an_empty_upstream_keeps_the_list() {
        let temp = tempfile::tempdir().unwrap();
        let store = GatewayStore::at(temp.path().join("gateway.json"));
        let mut gateway = GatewaySettings::default();
        gateway.providers.push(provider_with(RelayProfile {
            id: "p".into(),
            model: "a".into(),
            model_list: "a\nb".into(),
            ..RelayProfile::default()
        }));
        store.save(&gateway).unwrap();

        assert!(sync_upstream_models(&store, "p", &[" ".into()]).is_err());
        assert_eq!(
            store
                .load()
                .unwrap()
                .provider("p")
                .unwrap()
                .relay
                .model_list,
            "a\nb"
        );
    }

    #[test]
    fn saving_models_rewrites_the_default_in_the_config_snapshot() {
        use crate::gateway::apply::{ProviderInput, create_provider};
        let temp = tempfile::tempdir().unwrap();
        let store = GatewayStore::at(temp.path().join("gateway.json"));
        let input = ProviderInput {
            name: "Snapshot".into(),
            base_url: "https://snapshot.test/v1".into(),
            protocol: crate::settings::RelayProtocol::ChatCompletions,
            keys: vec!["sk-snapshot".into()],
            model: "old-model".into(),
            model_list: "old-model\nkeep-model".into(),
            ..ProviderInput::default()
        };
        let (_, id) = create_provider(&store, &input, "", None).unwrap();
        let created = store.load().unwrap();
        assert!(
            created
                .provider(&id)
                .unwrap()
                .relay
                .config_contents
                .contains("old-model"),
            "create_provider 建出来的供应商带 config 快照"
        );

        // 换默认模型并删掉旧默认：重新读盘后旧模型不能被改回来、也不能并回列表。
        set_provider_models(
            &store,
            &id,
            &["keep-model".into(), "new-model".into()],
            Some("new-model"),
        )
        .unwrap();
        let loaded = store.load().unwrap();
        let relay = &loaded.provider(&id).unwrap().relay;
        assert_eq!(relay.model, "new-model");
        assert_eq!(crate::relay_config::relay_profile_model(relay), "new-model");
        assert_eq!(
            crate::gateway::route::relay_models(relay),
            vec!["new-model", "keep-model"]
        );
        assert!(
            !relay.model_list.contains("old-model"),
            "{}",
            relay.model_list
        );

        // 没指定默认模型、原默认被删：换成列表第一个，同样落到快照里。
        set_provider_models(&store, &id, &["keep-model".into()], None).unwrap();
        let loaded = store.load().unwrap();
        let relay = &loaded.provider(&id).unwrap().relay;
        assert_eq!(relay.model, "keep-model");
        assert_eq!(relay.model_list, "keep-model");
        assert!(!relay.config_contents.contains("new-model"));
    }
}
