//! 网关供应商的账户余额：有公开余额接口的几家（DeepSeek、Kimi / Moonshot、OpenRouter、SiliconFlow）
//! 用该供应商的第一个 Key 查；识别为中转站（sub2api / new-api）的，和供应商池卡片走同一条识别链路。
//! 其它供应商不列出。

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
    /// 没有金额时的说明（如 new-api 的 Key 不限额度、没配访问令牌），界面放在悬停提示里。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
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

/// 供应商池卡片据此显示余额（#2793）：有余额接口的供应商（DeepSeek 等）不当中转站识别，
/// 余额和「用量」页、托盘面板同一个来源（[`fetch_balances`]），界面共用一份查询结果。
pub fn has_balance_api(relay: &RelayProfile) -> bool {
    balance_api(relay).is_some()
}

/// 查池里所有打开的供应商的余额：有余额接口的厂商和识别出的中转站（托盘、「用量」页、卡片共用）。
/// 并发请求，每个最多等 10 秒；不是中转站、也没有余额接口的不列出。
pub async fn fetch_balances(gateway: &GatewaySettings) -> Vec<ProviderBalance> {
    let tasks = gateway
        .providers
        .iter()
        .filter(|provider| provider.enabled)
        .map(|provider| {
            let relay = provider.relay.clone();
            let access = provider.newapi_access.clone();
            tokio::spawn(async move {
                match balance_api(&relay) {
                    Some(api) => Some(vendor_balance(&relay, api).await),
                    None => {
                        let probe = crate::relay_billing::probe_relay_billing_with_access(
                            &relay,
                            access.as_ref(),
                        )
                        .await;
                        relay_balance(&relay, &probe)
                    }
                }
            })
        })
        .collect::<Vec<_>>();
    let mut balances = Vec::new();
    for task in tasks {
        if let Ok(Some(balance)) = task.await {
            balances.push(balance);
        }
    }
    balances
}

async fn vendor_balance(relay: &RelayProfile, api: BalanceApi) -> ProviderBalance {
    let result = fetch_one(relay, api).await;
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
        note: None,
    }
}

/// 中转站的余额，取自识别结果（和供应商池卡片同一来源）；没识别出中转站时不列出。
fn relay_balance(
    relay: &RelayProfile,
    probe: &crate::relay_billing::RelayBillingProbe,
) -> Option<ProviderBalance> {
    probe.platform.as_ref()?;
    let usage = probe.usage.as_ref()?;
    let amount = usage.remaining.filter(|_| usage.kind != "unlimited");
    // 访问令牌查账户余额失败时给出原因；Key 不限额度又没配令牌时只是查不到，不算失败。
    let error = amount
        .is_none()
        .then(|| probe.newapi_access.as_ref().and_then(|status| status.error.clone()))
        .flatten();
    let note = (amount.is_none() && error.is_none() && usage.kind == "unlimited")
        .then(|| "Key 不限额度，在供应商详情里配置访问令牌后可显示账户余额".to_string());
    Some(ProviderBalance {
        provider_id: relay.id.clone(),
        provider: relay.name.clone(),
        base_url: relay.base_url.clone(),
        amount,
        currency: usage.unit.clone(),
        error,
        note,
    })
}

async fn fetch_one(relay: &RelayProfile, api: BalanceApi) -> Result<(f64, String), String> {
    fetch_one_at(relay, api, api.url()).await
}

/// 同 [`fetch_one`]，地址由调用方给（测试里换成本机的模拟上游）。
async fn fetch_one_at(
    relay: &RelayProfile,
    api: BalanceApi,
    url: &str,
) -> Result<(f64, String), String> {
    let key = relay.api_key.trim();
    if key.is_empty() {
        return Err("没有 Key".to_string());
    }
    // 余额查询属于这个供应商：按它自己的代理设置发（#2775）。
    let client = crate::http_client::proxied_client_for_relay("CodexPlusPlus", relay)
        .map_err(|error| format!("{error:#}"))?;
    let response = client
        .get(url)
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

    #[test]
    fn pool_cards_show_balance_only_for_vendors_with_balance_api() {
        // 供应商池卡片的余额来源（#2793）：DeepSeek 这类走余额接口，不当中转站识别；中转站不走。
        assert!(has_balance_api(&relay("https://api.deepseek.com")));
        assert!(has_balance_api(&relay("https://api.siliconflow.cn/v1")));
        assert!(!has_balance_api(&relay("https://jojocode.com/v1")));
    }

    fn relay_probe(kind: &str, remaining: Option<f64>) -> crate::relay_billing::RelayBillingProbe {
        crate::relay_billing::RelayBillingProbe {
            platform: Some("newapi".to_string()),
            rate: None,
            usage: Some(crate::relay_billing::RelayUsageInfo {
                endpoint: String::new(),
                kind: kind.to_string(),
                is_valid: true,
                plan_name: None,
                remaining,
                quota_limit: None,
                quota_used: None,
                unit: "USD".to_string(),
                rate_limits: Vec::new(),
            }),
            detail: String::new(),
            outcome: crate::relay_billing::RelayBillingOutcome::Recognized,
            newapi_access: None,
        }
    }

    #[test]
    fn relay_balances_come_from_the_billing_probe() {
        // 托盘「余额」和卡片同一来源：中转站也要列出，不能说「没有可以查询余额的供应商」。
        let provider = RelayProfile {
            id: "jojo".to_string(),
            name: "jojo".to_string(),
            ..relay("https://relay.example/v1")
        };
        let wallet = relay_balance(&provider, &relay_probe("wallet", Some(12.5))).unwrap();
        assert_eq!(wallet.amount, Some(12.5));
        assert_eq!(wallet.currency, "USD");
        assert_eq!(wallet.error, None);

        // Key 不限额度、没配访问令牌：列出但没有金额，提示去配令牌。
        let unlimited = relay_balance(&provider, &relay_probe("unlimited", None)).unwrap();
        assert_eq!(unlimited.amount, None);
        assert_eq!(unlimited.error, None);
        assert!(unlimited.note.unwrap().contains("访问令牌"));

        // 配了令牌但查询失败：给出原因。
        let mut failed = relay_probe("unlimited", None);
        failed.newapi_access = Some(crate::relay_billing::NewApiAccessStatus {
            configured: true,
            error: Some("HTTP 401：令牌无效".to_string()),
        });
        let failed = relay_balance(&provider, &failed).unwrap();
        assert_eq!(failed.error.as_deref(), Some("HTTP 401：令牌无效"));
        assert_eq!(failed.note, None);

        // 没识别出中转站：不列出。
        let unrecognized = crate::relay_billing::RelayBillingProbe::unrecognized(
            crate::relay_billing::RelayBillingOutcome::NotRelay,
            String::new(),
        );
        assert!(relay_balance(&provider, &unrecognized).is_none());
    }

    #[tokio::test]
    async fn pool_balances_include_recognized_relays() {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/usage"))
            .and(header("authorization", "Bearer sk-relay"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"mode":"unrestricted","isValid":true,"planName":"钱包余额","remaining":8,"unit":"USD","balance":8}"#,
            ))
            .mount(&server)
            .await;
        let provider = crate::gateway::store::GatewayProvider {
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
            models_checked_at_ms: 0,
            newapi_access: None,
            upstream_meta: Default::default(),
            relay: RelayProfile {
                id: "relay-a".to_string(),
                name: "中转 A".to_string(),
                api_key: "sk-relay".to_string(),
                relay_mode: crate::settings::RelayMode::PureApi,
                ..relay(&server.uri())
            },
            extra: Default::default(),
        };
        let mut disabled = provider.clone();
        disabled.enabled = false;
        disabled.relay.id = "relay-off".to_string();
        let gateway = GatewaySettings {
            providers: vec![provider, disabled],
            ..GatewaySettings::default()
        };

        let balances = fetch_balances(&gateway).await;

        assert_eq!(balances.len(), 1, "{balances:?}");
        assert_eq!(balances[0].provider_id, "relay-a");
        assert_eq!(balances[0].amount, Some(8.0));
        assert_eq!(balances[0].currency, "USD");
    }

    #[tokio::test]
    async fn deepseek_balance_comes_from_its_balance_api_with_provider_key() {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user/balance"))
            .and(header("authorization", "Bearer sk-deepseek"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "is_available": true,
                "balance_infos": [{ "currency": "CNY", "total_balance": "14.60" }]
            })))
            .mount(&server)
            .await;
        let provider = RelayProfile {
            api_key: "sk-deepseek".to_string(),
            ..relay("https://api.deepseek.com")
        };
        let url = format!("{}/user/balance", server.uri());

        let result = fetch_one_at(&provider, BalanceApi::DeepSeek, &url).await;

        assert_eq!(result, Ok((14.6, "CNY".to_string())));

        // 上游拒绝时给出原因，不带 Key。
        let failing = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&failing)
            .await;
        let error = fetch_one_at(
            &provider,
            BalanceApi::DeepSeek,
            &format!("{}/user/balance", failing.uri()),
        )
        .await
        .unwrap_err();
        assert_eq!(error, "查询余额失败：HTTP 401");
    }
}
