//! new-api（one-api 的分支）的余额与分组倍率，只用供应商自己的 Key：
//! - `GET /api/usage/token/`：Key 的额度（`object = token_usage`），Key 允许时还带账户余额
//! - `GET /api/log/token`：Key 最近的消费记录，`other.group_ratio` 是实际扣费用的分组倍率
//! - `GET /api/status`（公开）：额度点数和货币的换算（`quota_per_unit` 等）
//!
//! Key 不限额度时，Key 查不到账户余额；用户配置了访问令牌（#2793）就再查
//! `GET /api/user/self`（`data.quota` 是账户剩余额度）。按 new-api 文档
//! （https://docs.newapi.pro/zh/docs/api/management/auth ）带 `Authorization: Bearer <访问令牌>`
//! 和 `New-Api-User: <用户 ID>`：旧版必须带用户 ID 且要和令牌的主人一致，新版的分权限令牌
//! 需要勾选「个人资料 · 查看」（`profile:read`）。
//!
//! 额度都是点数，除以 `quota_per_unit`（默认 500000）才是美元。

use anyhow::Context;
use serde::Deserialize;
use serde_json::Value;

use crate::relay_billing::{
    BillingFetchError, BillingTarget, NewApiAccess, NewApiAccessStatus, RelayBillingOutcome,
    RelayBillingProbe, RelayRateInfo, RelayUsageInfo, get_json, get_json_with_headers, site_root,
};

const DEFAULT_QUOTA_PER_UNIT: f64 = 500_000.0;
/// new-api 消费日志的类型值（`LogTypeConsume`）。
const LOG_TYPE_CONSUME: i64 = 2;

#[derive(Debug, Deserialize)]
struct TokenUsageResponse {
    #[serde(default)]
    data: Option<TokenUsage>,
}

#[derive(Debug, Deserialize)]
struct TokenUsage {
    object: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    total_granted: f64,
    #[serde(default)]
    total_used: f64,
    #[serde(default)]
    total_available: f64,
    #[serde(default)]
    unlimited_quota: bool,
    /// 秒级时间戳，0 表示永不过期。
    #[serde(default)]
    expires_at: i64,
    #[serde(default)]
    user_quota: Option<UserQuota>,
}

#[derive(Debug, Deserialize)]
struct UserQuota {
    total_available: f64,
}

/// 点数 → 展示金额的换算。站点按人民币展示时跟着换成人民币。
#[derive(Debug, Clone, PartialEq)]
struct QuotaUnit {
    per_unit: f64,
    rate: f64,
    unit: &'static str,
}

impl Default for QuotaUnit {
    fn default() -> Self {
        Self {
            per_unit: DEFAULT_QUOTA_PER_UNIT,
            rate: 1.0,
            unit: "USD",
        }
    }
}

impl QuotaUnit {
    fn amount(&self, quota: f64) -> f64 {
        quota / self.per_unit * self.rate
    }
}

pub(crate) async fn probe(
    target: &BillingTarget,
    access: Option<&NewApiAccess>,
) -> RelayBillingProbe {
    let root = site_root(&target.base_url);
    let usage_endpoint = format!("{root}/api/usage/token/");
    let log_endpoint = format!("{root}/api/log/token");
    let status_endpoint = format!("{root}/api/status");
    let (usage, logs, status) = tokio::join!(
        get_json(&target.client, &usage_endpoint, Some(&target.api_key)),
        get_json(&target.client, &log_endpoint, Some(&target.api_key)),
        get_json(&target.client, &status_endpoint, None),
    );
    let usage = usage.and_then(|body| {
        parse_token_usage(&body).map_err(|error| BillingFetchError::not_relay(error.to_string()))
    });
    let token = match usage {
        Ok(token) => token,
        Err(error) => {
            // 请求失败（断网、超时、429 / 5xx、401/403）只是这次没拿到，不等于不是 new-api。
            return RelayBillingProbe::unrecognized(error.outcome(), format!("余额：{error}"));
        }
    };
    let unit = status
        .ok()
        .and_then(|body| quota_unit(&body))
        .unwrap_or_default();
    let rate = logs.ok().and_then(|body| latest_group_ratio(&body, &log_endpoint));
    let mut usage = usage_info(usage_endpoint, token, &unit, now_secs());
    let access = access.filter(|access| !access.token.trim().is_empty());
    let access_status = match access {
        None => NewApiAccessStatus {
            configured: false,
            error: None,
        },
        Some(access) => {
            // Key 自己有额度（或站点已经给了账户余额）时那就是要看的数，不再多发请求。
            let error = if usage.kind == "unlimited" {
                let endpoint = format!("{root}/api/user/self");
                match fetch_account_quota(&target.client, &endpoint, access).await {
                    Ok(quota) => {
                        usage = account_wallet(usage, endpoint, quota, &unit);
                        None
                    }
                    Err(error) => Some(error.to_string()),
                }
            } else {
                None
            };
            NewApiAccessStatus {
                configured: true,
                error,
            }
        }
    };
    RelayBillingProbe {
        platform: Some("newapi".to_string()),
        detail: if rate.is_none() {
            "这个 Key 还没有消费记录，拿不到分组倍率".to_string()
        } else {
            String::new()
        },
        rate,
        usage: Some(usage),
        outcome: RelayBillingOutcome::Recognized,
        newapi_access: Some(access_status),
    }
}

/// 用访问令牌查账户剩余额度（点数）。令牌只放在请求头里，出错信息不含令牌。
async fn fetch_account_quota(
    client: &reqwest::Client,
    endpoint: &str,
    access: &NewApiAccess,
) -> Result<f64, BillingFetchError> {
    let authorization = format!("Bearer {}", access.token.trim());
    let user_id = access.user_id.trim();
    let mut headers = vec![("Authorization", authorization.as_str())];
    if !user_id.is_empty() {
        headers.push(("New-Api-User", user_id));
    }
    let body = get_json_with_headers(client, endpoint, &headers).await?;
    parse_user_self_quota(&body).map_err(BillingFetchError::not_relay)
}

/// `/api/user/self` 的返回：`{"success": true, "data": {"quota": 点数, ...}}`。
/// 旧版 new-api 鉴权失败也回 HTTP 200，只是 `success = false`，原因在 `message`。
fn parse_user_self_quota(body: &str) -> Result<f64, String> {
    let payload: Value =
        serde_json::from_str(body).map_err(|_| "账户信息接口返回的不是 JSON".to_string())?;
    if payload.get("success").and_then(Value::as_bool) == Some(false) {
        let message = payload
            .get("message")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|message| !message.is_empty())
            .unwrap_or("上游拒绝请求");
        return Err(message.to_string());
    }
    payload
        .pointer("/data/quota")
        .and_then(Value::as_f64)
        .filter(|quota| quota.is_finite())
        .ok_or_else(|| "账户信息里没有余额（quota）".to_string())
}

/// Key 不限额度时，真正的上限是账户余额：换成「余额」显示，Key 是否有效等信息沿用。
fn account_wallet(
    usage: RelayUsageInfo,
    endpoint: String,
    quota: f64,
    unit: &QuotaUnit,
) -> RelayUsageInfo {
    RelayUsageInfo {
        endpoint,
        kind: "wallet".to_string(),
        remaining: Some(unit.amount(quota)),
        ..usage
    }
}

fn parse_token_usage(body: &str) -> anyhow::Result<TokenUsage> {
    let parsed: TokenUsageResponse =
        serde_json::from_str(body).context("余额接口返回结构不是 new-api 令牌用量")?;
    parsed
        .data
        .filter(|data| data.object == "token_usage")
        .ok_or_else(|| anyhow::anyhow!("余额接口返回结构不是 new-api 令牌用量"))
}

fn usage_info(endpoint: String, token: TokenUsage, unit: &QuotaUnit, now: i64) -> RelayUsageInfo {
    let expired = token.expires_at > 0 && token.expires_at <= now;
    let plan_name = token
        .name
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty());
    let base = RelayUsageInfo {
        endpoint,
        kind: String::new(),
        is_valid: !expired,
        plan_name,
        remaining: None,
        quota_limit: None,
        quota_used: None,
        unit: unit.unit.to_string(),
        rate_limits: Vec::new(),
    };
    if !token.unlimited_quota {
        return RelayUsageInfo {
            kind: "keyQuota".to_string(),
            remaining: Some(unit.amount(token.total_available)),
            quota_limit: Some(unit.amount(token.total_granted)),
            quota_used: Some(unit.amount(token.total_used)),
            ..base
        };
    }
    // Key 不限额度时真正的上限是账户余额；站点没让查就只能显示「不限额度」。
    match token.user_quota {
        Some(user) => RelayUsageInfo {
            kind: "wallet".to_string(),
            remaining: Some(unit.amount(user.total_available)),
            ..base
        },
        None => RelayUsageInfo {
            kind: "unlimited".to_string(),
            ..base
        },
    }
}

fn quota_unit(body: &str) -> Option<QuotaUnit> {
    let payload: Value = serde_json::from_str(body).ok()?;
    let data = payload.get("data")?;
    let per_unit = data
        .get("quota_per_unit")
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(DEFAULT_QUOTA_PER_UNIT);
    let cny_rate = data
        .get("usd_exchange_rate")
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && *value > 0.0);
    let display_cny = match data.get("quota_display_type").and_then(Value::as_str) {
        Some(kind) => kind.eq_ignore_ascii_case("CNY"),
        None => false,
    };
    Some(match (display_cny, cny_rate) {
        (true, Some(rate)) => QuotaUnit {
            per_unit,
            rate,
            unit: "CNY",
        },
        _ => QuotaUnit {
            per_unit,
            ..QuotaUnit::default()
        },
    })
}

/// 最近一条消费记录里的分组倍率（已含用户专属倍率，就是实际扣费用的那个）。
fn latest_group_ratio(body: &str, endpoint: &str) -> Option<RelayRateInfo> {
    let payload: Value = serde_json::from_str(body).ok()?;
    let logs = payload.get("data")?.as_array()?;
    logs.iter()
        .filter(|log| log.get("type").and_then(Value::as_i64) == Some(LOG_TYPE_CONSUME))
        .find_map(|log| {
            // `other` 是 JSON 字符串；兼容直接给对象的情况。
            let other = match log.get("other")? {
                Value::String(text) => serde_json::from_str::<Value>(text).ok()?,
                value => value.clone(),
            };
            let ratio = other
                .get("group_ratio")
                .and_then(Value::as_f64)
                .filter(|value| value.is_finite() && *value >= 0.0)?;
            Some(RelayRateInfo {
                endpoint: endpoint.to_string(),
                multiplier: ratio,
                applied_peak_multiplier: None,
                group: log
                    .get("group")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|group| !group.is_empty())
                    .map(str::to_string),
            })
        })
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(body: &str) -> TokenUsage {
        parse_token_usage(body).unwrap()
    }

    #[test]
    fn rejects_non_newapi_usage_shapes() {
        assert!(parse_token_usage(r#"{"mode":"unrestricted","isValid":true}"#).is_err());
        assert!(parse_token_usage(r#"{"code":true,"data":{"object":"list"}}"#).is_err());
    }

    #[test]
    fn limited_key_shows_key_quota_in_dollars() {
        let info = usage_info(
            String::new(),
            token(r#"{"code":true,"data":{"object":"token_usage","name":" codex ","total_granted":5000000,"total_used":1000000,"total_available":4000000,"unlimited_quota":false,"expires_at":0}}"#),
            &QuotaUnit::default(),
            100,
        );
        assert_eq!(info.kind, "keyQuota");
        assert_eq!(info.remaining, Some(8.0));
        assert_eq!(info.quota_limit, Some(10.0));
        assert_eq!(info.plan_name.as_deref(), Some("codex"));
        assert!(info.is_valid);
    }

    #[test]
    fn unlimited_key_falls_back_to_user_balance_or_unlimited() {
        let with_user = usage_info(
            String::new(),
            token(r#"{"code":true,"data":{"object":"token_usage","unlimited_quota":true,"total_available":0,"user_quota":{"total_available":250000,"total_used":10}}}"#),
            &QuotaUnit::default(),
            100,
        );
        assert_eq!(with_user.kind, "wallet");
        assert_eq!(with_user.remaining, Some(0.5));

        let without_user = usage_info(
            String::new(),
            token(r#"{"code":true,"data":{"object":"token_usage","unlimited_quota":true}}"#),
            &QuotaUnit::default(),
            100,
        );
        assert_eq!(without_user.kind, "unlimited");
        assert_eq!(without_user.remaining, None);
    }

    #[test]
    fn expired_key_is_marked_invalid() {
        let info = usage_info(
            String::new(),
            token(r#"{"code":true,"data":{"object":"token_usage","unlimited_quota":false,"expires_at":50}}"#),
            &QuotaUnit::default(),
            100,
        );
        assert!(!info.is_valid);
    }

    #[test]
    fn status_controls_quota_unit_and_currency() {
        let cny = quota_unit(r#"{"success":true,"data":{"quota_per_unit":1000,"quota_display_type":"CNY","usd_exchange_rate":7.2}}"#).unwrap();
        assert_eq!(cny.unit, "CNY");
        assert_eq!(cny.amount(1000.0), 7.2);
        let usd = quota_unit(r#"{"success":true,"data":{"quota_per_unit":1000,"quota_display_type":"USD","usd_exchange_rate":7.2}}"#).unwrap();
        assert_eq!(usd.unit, "USD");
        assert_eq!(usd.amount(1000.0), 1.0);
        assert!(quota_unit("not json").is_none());
    }

    #[test]
    fn group_ratio_comes_from_latest_consume_log() {
        let body = r#"{"success":true,"data":[
            {"type":5,"group":"vip","other":"{\"group_ratio\":9}"},
            {"type":2,"group":"default","other":"{\"model_ratio\":2.5,\"group_ratio\":0.8,\"user_group_ratio\":-1}"},
            {"type":2,"group":"old","other":"{\"group_ratio\":1}"}
        ]}"#;
        let rate = latest_group_ratio(body, "e").unwrap();
        assert_eq!(rate.multiplier, 0.8);
        assert_eq!(rate.group.as_deref(), Some("default"));
        assert!(latest_group_ratio(r#"{"success":true,"data":[]}"#, "e").is_none());
    }
}
