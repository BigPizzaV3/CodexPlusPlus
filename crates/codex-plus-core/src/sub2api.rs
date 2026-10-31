use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::relay_billing::{
    BillingFetchError, BillingTarget, RelayBillingOutcome, RelayBillingProbe, RelayRateInfo,
    RelayRateLimitWindow, RelayUsageInfo, get_json,
};
use crate::settings::RelayProfile;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sub2ApiBillingInfo {
    pub endpoint: String,
    pub group_rate_multiplier: f64,
    pub user_rate_multiplier: Option<f64>,
    pub resolved_rate_multiplier: f64,
    pub peak_rate_enabled: bool,
    pub peak_rate_multiplier: Option<f64>,
    pub applied_peak_multiplier: Option<f64>,
    pub effective_rate_multiplier: f64,
    pub observed_at: String,
}

#[derive(Debug, Deserialize)]
struct Sub2ApiBillingResponse {
    object: String,
    schema_version: u8,
    billing_scope: String,
    group_rate_multiplier: f64,
    #[serde(default)]
    user_rate_multiplier: Option<f64>,
    resolved_rate_multiplier: f64,
    peak_rate_enabled: bool,
    #[serde(default)]
    peak_rate_multiplier: Option<f64>,
    #[serde(default)]
    applied_peak_multiplier: Option<f64>,
    effective_rate_multiplier: f64,
    observed_at: String,
}

pub async fn fetch_sub2api_billing_info(
    profile: &RelayProfile,
) -> anyhow::Result<Sub2ApiBillingInfo> {
    let base_url = if profile.upstream_base_url.trim().is_empty() {
        profile.base_url.trim()
    } else {
        profile.upstream_base_url.trim()
    };
    if base_url.is_empty() {
        anyhow::bail!("Base URL 不能为空");
    }
    let api_key = crate::relay_config::relay_profile_api_key(profile);
    if api_key.trim().is_empty() {
        anyhow::bail!("API Key 不能为空");
    }
    // 与识别共用 client：只跟随同站重定向，Key 不会被带到别的主机。
    let client = crate::relay_billing::billing_client(profile)?;
    Ok(fetch_billing_with_client(&client, base_url, api_key.trim()).await?)
}

#[derive(Debug, Deserialize)]
struct Sub2ApiUsageResponse {
    mode: String,
    #[serde(rename = "isValid")]
    is_valid: bool,
    #[serde(default, rename = "planName")]
    plan_name: Option<String>,
    #[serde(default)]
    remaining: Option<f64>,
    #[serde(default)]
    balance: Option<f64>,
    #[serde(default)]
    unit: Option<String>,
    #[serde(default)]
    quota: Option<Sub2ApiUsageQuota>,
    #[serde(default)]
    rate_limits: Vec<RelayRateLimitWindow>,
    #[serde(default)]
    subscription: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct Sub2ApiUsageQuota {
    limit: f64,
    used: f64,
    remaining: f64,
}

/// 同时请求倍率和余额接口，任一按 sub2api 的结构返回就认定是 sub2api。
/// 新版 sub2api 两个接口都有；老版本没有 `/sub2api/billing`，只能拿到余额。
pub(crate) async fn probe(target: &BillingTarget) -> RelayBillingProbe {
    let (billing, usage) = tokio::join!(
        fetch_billing_with_client(&target.client, &target.base_url, &target.api_key),
        fetch_usage_with_client(&target.client, &target.base_url, &target.api_key),
    );
    let detail = [
        billing.as_ref().err().map(|error| format!("倍率：{error}")),
        usage.as_ref().err().map(|error| format!("余额：{error}")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join("；");
    // 两个接口都没拿到：只要有一个是「这次请求没成功」，就不能断定不是 sub2api。
    let any_unavailable = [billing.as_ref().err(), usage.as_ref().err()]
        .into_iter()
        .flatten()
        .any(|error| error.unavailable);
    let rate = billing.ok().map(|billing| RelayRateInfo {
        endpoint: billing.endpoint,
        multiplier: billing.effective_rate_multiplier,
        applied_peak_multiplier: billing.applied_peak_multiplier,
        group: None,
    });
    let usage = usage.ok();
    let platform = (rate.is_some() || usage.is_some()).then(|| "sub2api".to_string());
    let outcome = if platform.is_some() {
        RelayBillingOutcome::Recognized
    } else if any_unavailable {
        RelayBillingOutcome::Unavailable
    } else {
        RelayBillingOutcome::NotRelay
    };
    RelayBillingProbe {
        platform,
        rate,
        usage,
        detail,
        outcome,
        newapi_access: None,
    }
}

async fn fetch_billing_with_client(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
) -> Result<Sub2ApiBillingInfo, BillingFetchError> {
    let endpoint = sub2api_billing_endpoint(base_url);
    let body = get_json(client, &endpoint, Some(api_key)).await?;
    let parsed: Sub2ApiBillingResponse = serde_json::from_str(&body)
        .map_err(|_| BillingFetchError::not_relay("倍率接口没有返回有效 JSON"))?;
    validate_sub2api_billing_response(&parsed)
        .map_err(|error| BillingFetchError::not_relay(error.to_string()))?;
    Ok(Sub2ApiBillingInfo {
        endpoint,
        group_rate_multiplier: parsed.group_rate_multiplier,
        user_rate_multiplier: parsed.user_rate_multiplier,
        resolved_rate_multiplier: parsed.resolved_rate_multiplier,
        peak_rate_enabled: parsed.peak_rate_enabled,
        peak_rate_multiplier: parsed.peak_rate_multiplier,
        applied_peak_multiplier: parsed.applied_peak_multiplier,
        effective_rate_multiplier: parsed.effective_rate_multiplier,
        observed_at: parsed.observed_at,
    })
}

async fn fetch_usage_with_client(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
) -> Result<RelayUsageInfo, BillingFetchError> {
    let endpoint = sub2api_usage_endpoint(base_url);
    let body = get_json(client, &endpoint, Some(api_key)).await?;
    let parsed: Sub2ApiUsageResponse = serde_json::from_str(&body)
        .map_err(|_| BillingFetchError::not_relay("余额接口返回结构不是 sub2api 用量"))?;
    sub2api_usage_info(endpoint, parsed)
        .map_err(|error| BillingFetchError::not_relay(error.to_string()))
}

fn sub2api_usage_info(
    endpoint: String,
    parsed: Sub2ApiUsageResponse,
) -> anyhow::Result<RelayUsageInfo> {
    // `mode` 只有这两种取值是 sub2api 独有的形状，其它中转的 /v1/usage 不认。
    let kind = match parsed.mode.as_str() {
        "quota_limited" => "keyQuota",
        "unrestricted" if parsed.subscription.is_some() => "subscription",
        "unrestricted" if parsed.balance.is_some() => "wallet",
        // 订阅模式下 sub2api 可能拿不到订阅详情，只有套餐名
        "unrestricted" => "subscription",
        _ => anyhow::bail!("余额接口返回结构不是 sub2api 用量"),
    };
    let finite = |value: Option<f64>| value.filter(|value| value.is_finite());
    let remaining = finite(parsed.remaining)
        .or_else(|| finite(parsed.balance))
        .or_else(|| parsed.quota.as_ref().and_then(|quota| finite(Some(quota.remaining))));
    Ok(RelayUsageInfo {
        endpoint,
        kind: kind.to_string(),
        is_valid: parsed.is_valid,
        plan_name: parsed
            .plan_name
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty()),
        remaining,
        quota_limit: parsed.quota.as_ref().map(|quota| quota.limit),
        quota_used: parsed.quota.as_ref().map(|quota| quota.used),
        unit: parsed
            .unit
            .map(|unit| unit.trim().to_string())
            .filter(|unit| !unit.is_empty())
            .unwrap_or_else(|| "USD".to_string()),
        rate_limits: parsed
            .rate_limits
            .into_iter()
            .filter(|window| window.limit.is_finite() && window.remaining.is_finite())
            .collect(),
    })
}

pub fn sub2api_usage_endpoint(base_url: &str) -> String {
    let cleaned = base_url.trim().trim_end_matches('/');
    if cleaned.is_empty() {
        return String::new();
    }
    if crate::protocol_proxy::has_version_suffix(cleaned) {
        return format!("{cleaned}/usage");
    }
    format!("{cleaned}/v1/usage")
}

pub fn sub2api_billing_endpoint(base_url: &str) -> String {
    let cleaned = base_url.trim().trim_end_matches('/');
    if cleaned.is_empty() {
        return String::new();
    }
    if cleaned.ends_with("/sub2api/billing") {
        return cleaned.to_string();
    }
    if crate::protocol_proxy::has_version_suffix(cleaned) {
        return format!("{cleaned}/sub2api/billing");
    }
    format!("{cleaned}/v1/sub2api/billing")
}

fn validate_sub2api_billing_response(response: &Sub2ApiBillingResponse) -> anyhow::Result<()> {
    if response.object != "sub2api.key_billing"
        || response.schema_version != 1
        || response.billing_scope != "token"
    {
        anyhow::bail!("倍率接口返回结构不是 sub2api.key_billing");
    }
    for value in [
        response.group_rate_multiplier,
        response.resolved_rate_multiplier,
        response.effective_rate_multiplier,
    ] {
        if !value.is_finite() || value < 0.0 {
            anyhow::bail!("倍率接口返回了无效倍率");
        }
    }
    if let Some(value) = response.user_rate_multiplier {
        if !value.is_finite() || value < 0.0 {
            anyhow::bail!("倍率接口返回了无效用户倍率");
        }
    }
    if response.observed_at.trim().is_empty() {
        anyhow::bail!("倍率接口缺少观测时间");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn builds_sub2api_billing_endpoint_from_common_base_urls() {
        assert_eq!(
            sub2api_billing_endpoint("https://example.com"),
            "https://example.com/v1/sub2api/billing"
        );
        assert_eq!(
            sub2api_billing_endpoint("https://example.com/v1"),
            "https://example.com/v1/sub2api/billing"
        );
        assert_eq!(
            sub2api_billing_endpoint("https://example.com/v1/sub2api/billing"),
            "https://example.com/v1/sub2api/billing"
        );
    }

    #[tokio::test]
    async fn fetch_sub2api_billing_info_reads_effective_multiplier() {
        let server = spawn_billing_server(
            200,
            r#"{
                "object":"sub2api.key_billing",
                "schema_version":1,
                "billing_scope":"token",
                "group_rate_multiplier":0.8,
                "user_rate_multiplier":0.6,
                "resolved_rate_multiplier":0.6,
                "peak_rate_enabled":true,
                "peak_rate_multiplier":1.5,
                "applied_peak_multiplier":1.5,
                "effective_rate_multiplier":0.9,
                "observed_at":"2026-07-30T10:00:00Z"
            }"#,
        );
        let profile = RelayProfile {
            base_url: server.base_url.clone(),
            api_key: "sk-test".to_string(),
            ..RelayProfile::default()
        };

        let info = fetch_sub2api_billing_info(&profile).await.unwrap();
        let request = server.finish();

        assert_eq!(request.path, "/v1/sub2api/billing");
        assert_eq!(request.authorization, "Bearer sk-test");
        assert_eq!(request.x_api_key, "sk-test");
        assert_eq!(info.group_rate_multiplier, 0.8);
        assert_eq!(info.user_rate_multiplier, Some(0.6));
        assert_eq!(info.effective_rate_multiplier, 0.9);
    }

    #[test]
    fn builds_sub2api_usage_endpoint_from_common_base_urls() {
        assert_eq!(
            sub2api_usage_endpoint("https://example.com/"),
            "https://example.com/v1/usage"
        );
        assert_eq!(
            sub2api_usage_endpoint("https://example.com/v1"),
            "https://example.com/v1/usage"
        );
    }

    fn parse_usage(body: &str) -> anyhow::Result<RelayUsageInfo> {
        sub2api_usage_info(String::new(), serde_json::from_str(body)?)
    }

    #[test]
    fn parses_sub2api_wallet_balance() {
        let info = parse_usage(
            r#"{"mode":"unrestricted","isValid":true,"planName":"钱包余额","remaining":12.5,"unit":"USD","balance":12.5}"#,
        )
        .unwrap();
        assert_eq!(info.kind, "wallet");
        assert_eq!(info.remaining, Some(12.5));
        assert_eq!(info.unit, "USD");
    }

    #[test]
    fn parses_sub2api_subscription_remaining() {
        let info = parse_usage(
            r#"{"mode":"unrestricted","isValid":true,"planName":"Pro 月卡","unit":"USD","remaining":3.2,
                "subscription":{"daily_usage_usd":1.8,"daily_limit_usd":5}}"#,
        )
        .unwrap();
        assert_eq!(info.kind, "subscription");
        assert_eq!(info.plan_name.as_deref(), Some("Pro 月卡"));
        assert_eq!(info.remaining, Some(3.2));
    }

    #[test]
    fn parses_sub2api_key_quota_and_rate_limits() {
        let info = parse_usage(
            r#"{"mode":"quota_limited","isValid":true,"status":"active",
                "quota":{"limit":50,"used":20,"remaining":30,"unit":"USD"},"remaining":30,"unit":"USD",
                "rate_limits":[{"window":"5h","limit":10,"used":4,"remaining":6,"window_start":null}]}"#,
        )
        .unwrap();
        assert_eq!(info.kind, "keyQuota");
        assert_eq!(info.remaining, Some(30.0));
        assert_eq!(info.quota_limit, Some(50.0));
        assert_eq!(info.rate_limits.len(), 1);
        assert_eq!(info.rate_limits[0].window, "5h");
        assert_eq!(info.rate_limits[0].remaining, 6.0);
    }

    #[test]
    fn rejects_usage_shapes_from_other_relays() {
        assert!(parse_usage(r#"{"object":"list","data":[]}"#).is_err());
        assert!(parse_usage(r#"{"mode":"daily","isValid":true}"#).is_err());
    }

    struct BillingServer {
        base_url: String,
        handle: thread::JoinHandle<BillingRequest>,
    }

    impl BillingServer {
        fn finish(self) -> BillingRequest {
            self.handle.join().unwrap()
        }
    }

    struct BillingRequest {
        path: String,
        authorization: String,
        x_api_key: String,
    }

    fn spawn_billing_server(status: u16, body: &'static str) -> BillingServer {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let base_url = format!("http://{address}");
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0u8; 4096];
            let read = stream.read(&mut buffer).unwrap();
            let request = String::from_utf8_lossy(&buffer[..read]).to_string();
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or_default()
                .to_string();
            let authorization = header_value(&request, "authorization");
            let x_api_key = header_value(&request, "x-api-key");
            let response = format!(
                "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            BillingRequest {
                path,
                authorization,
                x_api_key,
            }
        });
        BillingServer { base_url, handle }
    }

    fn header_value(request: &str, name: &str) -> String {
        request
            .lines()
            .find_map(|line| {
                let (key, value) = line.split_once(':')?;
                if key.eq_ignore_ascii_case(name) {
                    Some(value.trim().to_string())
                } else {
                    None
                }
            })
            .unwrap_or_default()
    }
}
