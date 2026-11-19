//! 「添加 / 编辑供应商」弹窗用的两个查询：从上游拉模型列表、按模型查公开价格。

use serde::Serialize;

use codex_plus_core::gateway::provider_headers::{self, ProviderHeaderInput};
use codex_plus_core::gateway::store::GatewayStore;
use codex_plus_core::model_pricing::{self, PriceMatch};
use codex_plus_core::settings::RelayProtocol;

use crate::commands::{CommandResult, failed, ok};

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderModels {
    pub models: Vec<String>,
    pub endpoint: String,
}

/// 用弹窗里填的地址、协议和第一个 Key 拉 `/v1/models`。
/// 编辑已有供应商时 Key 不回显：`api_key` 留空、传 `provider_id`，就用网关里存的 Key。
/// `custom_headers` 是弹窗里正在编辑的自定义请求头（#3075）；不传时用已保存的。
#[tauri::command]
pub async fn fetch_provider_models(
    base_url: String,
    protocol: RelayProtocol,
    api_key: String,
    provider_id: Option<String>,
    custom_headers: Option<Vec<ProviderHeaderInput>>,
) -> CommandResult<ProviderModels> {
    // OpenCode Go / Zen 的 /models 会列出 Gemini 和只给 OpenCode 自己用的免费模型：改用目录里网关能用的。
    if let Some(plan) = codex_plus_core::gateway::opencode::plan_of(&base_url) {
        if let Ok(models) = codex_plus_core::gateway::opencode::catalog_models(plan).await
            && !models.is_empty()
        {
            return ok(
                &format!(
                    "从 models.dev 目录获取到 {} 个可用模型（不含 Gemini 与免费模型）。",
                    models.len()
                ),
                ProviderModels {
                    models,
                    endpoint: codex_plus_core::model_pricing::PRICE_CATALOG_URL.to_string(),
                },
            );
        }
    }
    let result = async {
        let key = api_key
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or_default()
            .to_string();
        // 编辑已有供应商时：已保存的请求头（敏感头没改时按名字沿用）。
        let saved = match &provider_id {
            Some(id) => GatewayStore::default()
                .load()?
                .provider(id)
                .map(|provider| provider.relay.clone()),
            None => None,
        };
        let mut relay = match (&saved, key.is_empty()) {
            (Some(saved), true) => saved.clone(),
            (None, true) if provider_id.is_some() => {
                anyhow::bail!("供应商池里没有 {}", provider_id.as_deref().unwrap_or_default())
            }
            (_, true) => anyhow::bail!("先填 API Key 再获取模型列表"),
            (_, false) => codex_plus_core::provider_import::pool_relay_profile(
                "draft",
                &base_url,
                &key,
                protocol,
                "",
                "",
                &[],
            )?,
        };
        let target = if base_url.trim().is_empty() {
            relay.base_url.clone()
        } else {
            base_url.trim().to_string()
        };
        if let Some(rows) = &custom_headers {
            // 订阅账号等不开放编辑的成员仍用自己的头。
            if saved.as_ref().is_none_or(provider_headers::editable) {
                let (stored, stored_base) = saved
                    .as_ref()
                    .map(|relay| (relay.custom_headers.as_slice(), relay.base_url.as_str()))
                    .unwrap_or((&[], ""));
                relay.custom_headers =
                    provider_headers::resolve(stored, stored_base, &target, rows)?;
            }
        }
        // 弹窗里改过的地址和协议优先。
        if !base_url.trim().is_empty() {
            relay.base_url = target;
            relay.upstream_base_url.clear();
        }
        relay.protocol = protocol;
        codex_plus_core::model_catalog::fetch_relay_profile_model_ids(&relay).await
    }
    .await;
    match result {
        Ok((models, endpoint)) => ok(
            &format!("从上游获取到 {} 个模型。", models.len()),
            ProviderModels { models, endpoint },
        ),
        Err(error) => failed(
            &format!("获取模型列表失败：{error:#}"),
            ProviderModels::default(),
        ),
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelPriceEntry {
    pub model: String,
    pub price: Option<PriceMatch>,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelPrices {
    pub prices: Vec<ModelPriceEntry>,
    pub source: String,
    pub fetched_at_ms: u64,
}

/// 按供应商地址查一组模型的公开价格（每百万 token，美元）；查不到的项 `price` 为空。
#[tauri::command]
pub async fn lookup_model_prices(
    base_url: String,
    models: Vec<String>,
    refresh: Option<bool>,
) -> CommandResult<ModelPrices> {
    match model_pricing::load_catalog(refresh.unwrap_or(false)).await {
        Ok((catalog, fetched_at_ms)) => {
            let prices: Vec<_> = models
                .into_iter()
                .map(|model| ModelPriceEntry {
                    price: catalog.lookup(&base_url, &model),
                    model,
                })
                .collect();
            let found = prices.iter().filter(|entry| entry.price.is_some()).count();
            ok(
                &format!(
                    "在公开价格表里找到 {found} / {} 个模型的价格。",
                    prices.len()
                ),
                ModelPrices {
                    prices,
                    source: "models.dev".to_string(),
                    fetched_at_ms,
                },
            )
        }
        Err(error) => failed(&format!("查询价格失败：{error:#}"), ModelPrices::default()),
    }
}
