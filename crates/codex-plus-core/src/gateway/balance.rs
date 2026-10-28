//! 网关供应商的账户余额：只查有公开余额接口的几家（DeepSeek、Kimi / Moonshot、OpenRouter、SiliconFlow），
//! 用该供应商的第一个 Key。其它供应商不显示余额卡片。

use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

use crate::gateway::store::GatewaySettings;
use crate::settings::RelayProfile;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderBalance {
    pub provider_id: String,
    pub provider: String,
    pub base_url: String,
    pub amount: Option<f64>,
    pub currency: String,
    pub error: Option<String>,
}

/// 余额接口：地址和解析方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BalanceApi {
    DeepSeek,
    Moonshot { global: bool },
    OpenRouter,
    SiliconFlow { global: bool },
}

impl BalanceApi {
    pub fn url(&self) -> &'static str {
        match self {
            BalanceApi::DeepSeek => "https://api.deepseek.com/user/balance",
            BalanceApi::Moonshot { global: false } => "https://api.moonshot.cn/v1/users/me/balance",
            BalanceApi::Moonshot { global: true } => "https://api.moonshot.ai/v1/users/me/balance",
            BalanceApi::OpenRouter => "https://openrouter.ai/api/v1/credits",
            BalanceApi::SiliconFlow { global: false } => "https://api.siliconflow.cn/v1/user/info",
            BalanceApi::SiliconFlow { global: true } => "https://api.siliconflow.com/v1/user/info",
        }
    }

    /// 解析接口返回，得到 (金额, 币种)。
    pub fn parse(&self, value: &Value) -> Option<(f64, String)> {
        match self {
            BalanceApi::DeepSeek => {
                let rows = value.get("balance_infos")?.as_array()?;
                let row = rows
                    .iter()
                    .find(|row| row.get("currency").and_then(Value::as_str) == Some("CNY"))
                    .or_else(|| rows.first())?;
                Some((
                    number(row.get("total_balance")?)?,
                    row.get("currency")
                        .and_then(Value::as_str)
                        .unwrap_or("CNY")
                        .to_string(),
                ))
            }
            BalanceApi::Moonshot { global } => Some((
                number(value.pointer("/data/available_balance")?)?,
                if *global { "USD" } else { "CNY" }.to_string(),
            )),
            BalanceApi::OpenRouter => {
                let total = number(value.pointer("/data/total_credits")?)?;
                let used = value
                    .pointer("/data/total_usage")
                    .and_then(number)
                    .unwrap_or(0.0);
                Some((total - used, "USD".to_string()))
            }
            BalanceApi::SiliconFlow { global } => Some((
                number(value.pointer("/data/totalBalance")?)?,
                if *global { "USD" } else { "CNY" }.to_string(),
            )),
        }
    }
}

fn number(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()))
}

/// 按供应商地址判断有没有余额接口。
pub fn balance_api(relay: &RelayProfile) -> Option<BalanceApi> {
    let base = relay.base_url.trim().to_ascii_lowercase();
    if base.contains("deepseek.com") {
        Some(BalanceApi::DeepSeek)
    } else if base.contains("moonshot.cn") {
        Some(BalanceApi::Moonshot { global: false })
    } else if base.contains("moonshot.ai") {
        Some(BalanceApi::Moonshot { global: true })
    } else if base.contains("openrouter.ai") {
        Some(BalanceApi::OpenRouter)
    } else if base.contains("siliconflow.cn") {
        Some(BalanceApi::SiliconFlow { global: false })
    } else if base.contains("siliconflow.com") {
        Some(BalanceApi::SiliconFlow { global: true })
    } else {
        None
    }
}

/// 查池里所有打开、且有余额接口的供应商。并发请求，每个最多等 10 秒。
pub async fn fetch_balances(gateway: &GatewaySettings) -> Vec<ProviderBalance> {
    let tasks = gateway
        .providers
        .iter()
        .filter(|provider| provider.enabled)
        .filter_map(|provider| {
            let api = balance_api(&provider.relay)?;
            let relay = provider.relay.clone();
            Some(tokio::spawn(async move {
                let result = fetch_one(&relay, api).await;
                ProviderBalance {
                    provider_id: relay.id.clone(),
                    provider: relay.name.clone(),
                    base_url: relay.base_url.clone(),
                    amount: result.as_ref().ok().map(|(amount, _)| *amount),
                    currency: result
                        .as_ref()
                        .map(|(_, currency)| currency.clone())
                        .unwrap_or_default(),
                    error: result.err(),
                }
            }))
        })
        .collect::<Vec<_>>();
    let mut balances = Vec::new();
    for task in tasks {
        if let Ok(balance) = task.await {
            balances.push(balance);
        }
    }
    balances
}

async fn fetch_one(relay: &RelayProfile, api: BalanceApi) -> Result<(f64, String), String> {
    let key = relay.api_key.trim();
    if key.is_empty() {
        return Err("没有 Key".to_string());
    }
    let client = crate::http_client::proxied_client("CodexPlusPlus")
        .map_err(|error| format!("{error:#}"))?;
    let response = client
        .get(api.url())
        .bearer_auth(key)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|_| "查询余额失败：连不上".to_string())?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("查询余额失败：HTTP {}", status.as_u16()));
    }
    let value: Value = response
        .json()
        .await
        .map_err(|_| "余额响应不是 JSON".to_string())?;
    api.parse(&value)
        .ok_or_else(|| "余额响应里没有金额".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn relay(base_url: &str) -> RelayProfile {
        RelayProfile {
            base_url: base_url.to_string(),
            ..RelayProfile::default()
        }
    }

    #[test]
    fn recognizes_vendors_by_address() {
        assert_eq!(
            balance_api(&relay("https://api.deepseek.com/")),
            Some(BalanceApi::DeepSeek)
        );
        assert_eq!(
            balance_api(&relay("https://api.moonshot.cn/v1")),
            Some(BalanceApi::Moonshot { global: false })
        );
        assert_eq!(
            balance_api(&relay("https://openrouter.ai/api/v1")),
            Some(BalanceApi::OpenRouter)
        );
        assert_eq!(balance_api(&relay("https://relay.example/v1")), None);
    }

    #[test]
    fn parses_each_vendor_response() {
        let deepseek = json!({ "is_available": true, "balance_infos": [
            { "currency": "USD", "total_balance": "1.00" },
            { "currency": "CNY", "total_balance": "86.40" }
        ]});
        assert_eq!(
            BalanceApi::DeepSeek.parse(&deepseek),
            Some((86.4, "CNY".to_string()))
        );
        let moonshot = json!({ "data": { "available_balance": 142.1 } });
        assert_eq!(
            BalanceApi::Moonshot { global: false }.parse(&moonshot),
            Some((142.1, "CNY".to_string()))
        );
        let openrouter = json!({ "data": { "total_credits": 30.0, "total_usage": 6.25 } });
        assert_eq!(
            BalanceApi::OpenRouter.parse(&openrouter),
            Some((23.75, "USD".to_string()))
        );
        let silicon = json!({ "data": { "totalBalance": "12.5" } });
        assert_eq!(
            BalanceApi::SiliconFlow { global: false }.parse(&silicon),
            Some((12.5, "CNY".to_string()))
        );
        assert_eq!(BalanceApi::OpenRouter.parse(&json!({})), None);
    }
}
