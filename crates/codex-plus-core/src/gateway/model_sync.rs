//! 供应商的模型列表与价格：用存下的凭据重新拉上游模型、手动保存模型列表、
//! 按公开价格表给没填价的模型补价格、定期检查上游有没有新模型。
//!
//! 检查结果记在 [`GatewayProvider`] 的 `upstream_new` / `upstream_gone` 里，界面据此提示；
//! 用户保存模型列表后清掉。检查只提示，不自动改用户整理过的列表。

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

/// 比较时忽略大小写。
pub fn diff_models(current: &[String], upstream: &[String]) -> ModelDiff {
    let has =
        |list: &[String], model: &str| list.iter().any(|item| item.eq_ignore_ascii_case(model));
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
    let models = match accounts::relay_account_kind(relay).as_deref() {
        Some("copilot") => {
            let prepared = accounts::prepare_relay(relay).await?;
            let token = accounts::CopilotToken {
                token: prepared.api_key.clone(),
                expires_at_ms: 0,
                api_base: prepared.base_url.trim_end_matches('#').to_string(),
                sku: String::new(),
            };
            accounts::copilot_models(&token).await
        }
        Some("grok") => {
            let prepared = accounts::prepare_relay(relay).await?;
            accounts::grok_models(&prepared.api_key).await
        }
        Some("chatgpt-api") => {
            let prepared = accounts::prepare_relay(relay).await?;
            accounts::chatgpt_api_models(
                &prepared.api_key,
                &crate::relay_config::default_codex_home_dir(),
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
            upstream_new: vec!["b".into()],
            upstream_gone: vec!["x".into()],
            models_checked_at_ms: 1,
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
