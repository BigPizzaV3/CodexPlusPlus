//! 订阅账号的额度：ChatGPT 账号的 5 小时 / 每周用量百分比，Copilot 的本月高级请求，
//! SuperGrok 的本周期额度。
//!
//! 两个来源：ChatGPT 后端每次响应都带的 `x-codex-*` 响应头，以及后台定期查询额度接口
//! （ChatGPT `wham/usage`、Copilot `copilot_internal/user`、SuperGrok `billing`）。
//! 只存在内存，重启后重新查。
//! 模型组「额度优先」按剩余额度排先后，供应商池在账号行上显示。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use reqwest::header::HeaderMap;
use serde::Serialize;
use serde_json::Value;

use crate::gateway::store::GatewayStore;
use crate::settings::RelayProfile;

const CHATGPT_USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
/// 后台多久查一次额度。
const REFRESH_EVERY: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaWindow {
    /// 窗口时长（分钟）：300 是 5 小时、10080 是每周；按月重置的额度（Copilot）为 None。
    pub window_minutes: Option<u64>,
    pub used_percent: f64,
    /// 重置时间（毫秒时间戳）。
    pub resets_at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderQuota {
    pub windows: Vec<QuotaWindow>,
    pub updated_at_ms: u64,
}

fn cache() -> &'static Mutex<HashMap<String, ProviderQuota>> {
    static CACHE: OnceLock<Mutex<HashMap<String, ProviderQuota>>> = OnceLock::new();
    CACHE.get_or_init(Mutex::default)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

pub fn record(relay_id: &str, quota: ProviderQuota) {
    if quota.windows.is_empty() {
        return;
    }
    cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(relay_id.to_string(), quota);
}

/// 忘掉一个供应商的额度（删除供应商时用：`login:chatgpt` 的 id 会被新登录的账号复用，
/// 不清的话新账号会沿用旧账号的额度）。
pub fn forget(relay_id: &str) {
    cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(relay_id);
}

/// 秒（可能是小数、负数或很大的数）→ 毫秒，越界时取边界值，不溢出。
fn seconds_to_ms(seconds: f64) -> u64 {
    // f64 → u64 的 `as` 本身是饱和转换（负数、NaN 得 0，过大得 u64::MAX）。
    (seconds * 1000.0) as u64
}

pub fn get(relay_id: &str) -> Option<ProviderQuota> {
    cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(relay_id)
        .cloned()
}

/// 剩余比例（0–1），取最紧的那个窗口；已经过了重置时间的窗口按满额算。没有额度数据返回 None。
pub fn remaining(relay_id: &str) -> Option<f64> {
    get(relay_id).map(|quota| remaining_at(&quota, now_ms()))
}

pub fn remaining_at(quota: &ProviderQuota, now_ms: u64) -> f64 {
    quota
        .windows
        .iter()
        .map(|window| {
            if window.resets_at_ms.is_some_and(|reset| reset <= now_ms) {
                1.0
            } else {
                (1.0 - window.used_percent / 100.0).clamp(0.0, 1.0)
            }
        })
        .fold(1.0, f64::min)
}

/// ChatGPT 后端响应头里的额度：`x-codex-primary-*`（5 小时）与 `x-codex-secondary-*`（每周）。
pub fn from_codex_headers(headers: &HeaderMap, now_ms: u64) -> Option<ProviderQuota> {
    let read = |name: String| {
        headers
            .get(name.as_str())
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<f64>().ok())
    };
    let mut windows = Vec::new();
    for slot in ["primary", "secondary"] {
        let Some(used) = read(format!("x-codex-{slot}-used-percent")) else {
            continue;
        };
        let resets_at_ms = read(format!("x-codex-{slot}-reset-at"))
            .map(seconds_to_ms)
            .or_else(|| {
                read(format!("x-codex-{slot}-reset-after-seconds"))
                    .map(|seconds| now_ms.saturating_add(seconds_to_ms(seconds)))
            });
        windows.push(QuotaWindow {
            window_minutes: read(format!("x-codex-{slot}-window-minutes"))
                .map(|value| value as u64),
            used_percent: used,
            resets_at_ms,
        });
    }
    (!windows.is_empty()).then_some(ProviderQuota {
        windows,
        updated_at_ms: now_ms,
    })
}

/// `wham/usage` 的返回：`rate_limit.primary_window` / `secondary_window`。
pub fn parse_chatgpt_usage(value: &Value, now_ms: u64) -> Option<ProviderQuota> {
    let limit = value.get("rate_limit")?;
    let mut windows = Vec::new();
    for slot in ["primary_window", "secondary_window"] {
        let Some(window) = limit.get(slot).filter(|window| window.is_object()) else {
            continue;
        };
        let Some(used) = window.get("used_percent").and_then(Value::as_f64) else {
            continue;
        };
        let resets_at_ms = window
            .get("reset_at")
            .and_then(Value::as_u64)
            .map(|seconds| seconds.saturating_mul(1000))
            .or_else(|| {
                window
                    .get("reset_after_seconds")
                    .and_then(Value::as_u64)
                    .map(|seconds| now_ms.saturating_add(seconds.saturating_mul(1000)))
            });
        windows.push(QuotaWindow {
            window_minutes: window
                .get("limit_window_seconds")
                .and_then(Value::as_u64)
                .map(|seconds| seconds / 60),
            used_percent: used,
            resets_at_ms,
        });
    }
    (!windows.is_empty()).then_some(ProviderQuota {
        windows,
        updated_at_ms: now_ms,
    })
}

/// `copilot_internal/user` 的返回：本月高级请求（`quota_snapshots.premium_interactions`），无限额度不算。
pub fn parse_copilot_user(value: &Value, now_ms: u64) -> Option<ProviderQuota> {
    let premium = value.pointer("/quota_snapshots/premium_interactions")?;
    if premium.get("unlimited").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let remaining = premium.get("percent_remaining").and_then(Value::as_f64)?;
    let resets_at_ms = value
        .get("quota_reset_date_utc")
        .or_else(|| value.get("quota_reset_date"))
        .and_then(Value::as_str)
        .and_then(parse_reset_date);
    Some(ProviderQuota {
        windows: vec![QuotaWindow {
            window_minutes: None,
            used_percent: (100.0 - remaining).clamp(0.0, 100.0),
            resets_at_ms,
        }],
        updated_at_ms: now_ms,
    })
}

/// SuperGrok 订阅代理 `billing?format=credits` 的返回：`config.creditUsagePercent` 是本周期已用百分比，
/// 周期看 `config.currentPeriod`（SuperGrok 是每周），没有时按账单周期（`billingPeriodEnd`）。
/// 按量付费（`onDemandUsed` / `onDemandCap`）不是时间窗口，不计入。
pub fn parse_grok_billing(value: &Value, now_ms: u64) -> Option<ProviderQuota> {
    let config = value.get("config")?.as_object()?;
    let used = config
        .get("creditUsagePercent")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let (window_minutes, end) = match config
        .get("currentPeriod")
        .filter(|period| period.is_object())
    {
        Some(period) => {
            let kind = period
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim_start_matches("USAGE_PERIOD_TYPE_");
            let minutes = match kind {
                "DAILY" => Some(24 * 60),
                "WEEKLY" => Some(7 * 24 * 60),
                _ => None,
            };
            (minutes, period.get("end"))
        }
        None => (None, config.get("billingPeriodEnd")),
    };
    Some(ProviderQuota {
        windows: vec![QuotaWindow {
            window_minutes,
            used_percent: used.clamp(0.0, 100.0),
            resets_at_ms: end.and_then(Value::as_str).and_then(parse_rfc3339_ms),
        }],
        updated_at_ms: now_ms,
    })
}

/// RFC 3339 时间（`2026-10-12T02:17:59.504011+00:00`、`…Z`）→ 毫秒时间戳。
fn parse_rfc3339_ms(text: &str) -> Option<u64> {
    let day_ms = i64::try_from(parse_reset_date(text)?).ok()?;
    let time = text.get(10..)?.strip_prefix(['T', 't', ' '])?;
    let number = |range: std::ops::Range<usize>| -> Option<i64> {
        let digits = time.get(range)?;
        digits
            .bytes()
            .all(|byte| byte.is_ascii_digit())
            .then(|| digits.parse().ok())
            .flatten()
    };
    let (hour, minute, second) = (number(0..2)?, number(3..5)?, number(6..8)?);
    if time.get(2..3)? != ":" || time.get(5..6)? != ":" || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let mut rest = time.get(8..)?;
    let mut millis = 0;
    if let Some(fraction) = rest.strip_prefix('.') {
        let digits = fraction.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return None;
        }
        millis = format!("{:0<3}", &fraction[..digits.min(3)]).parse().ok()?;
        rest = &fraction[digits..];
    }
    let offset_minutes = match rest {
        "Z" | "z" => 0,
        _ => {
            let sign = match rest.get(..1)? {
                "+" => 1,
                "-" => -1,
                _ => return None,
            };
            let (hours, minutes) = rest.get(1..)?.split_once(':')?;
            if hours.len() != 2 || minutes.len() != 2 {
                return None;
            }
            sign * (hours.parse::<i64>().ok()? * 60 + minutes.parse::<i64>().ok()?)
        }
    };
    let ms =
        day_ms + ((hour * 60 + minute) * 60 + second) * 1000 + millis - offset_minutes * 60_000;
    u64::try_from(ms).ok()
}

/// `2026-11-01`（或以它开头的时间）→ 当天 0 点（UTC）的毫秒时间戳。
fn parse_reset_date(text: &str) -> Option<u64> {
    let date = text.get(..10)?;
    let mut parts = date.split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: i64 = parts.next()?.parse().ok()?;
    let day: i64 = parts.next()?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    // 公历日期 → 1970-01-01 起的天数（Howard Hinnant 的 days_from_civil）。
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    u64::try_from(days * 86_400_000).ok()
}

/// 这个供应商有没有额度可查：ChatGPT 订阅账号、Copilot 账号或 SuperGrok 账号。
pub fn supports(relay: &RelayProfile) -> bool {
    crate::official_login::member_uses_official_login_transport(relay)
        || matches!(
            crate::gateway::accounts::relay_account_kind(relay).as_deref(),
            Some("copilot" | "grok")
        )
}

/// 查一次额度并记下来。不支持的供应商返回 Ok(None)。
pub async fn refresh(relay: &RelayProfile) -> anyhow::Result<Option<ProviderQuota>> {
    let now = now_ms();
    let quota = if crate::official_login::member_uses_official_login_transport(relay) {
        let credential = crate::official_login::resolve_official_credential(relay, false).await?;
        // 额度查询按这个供应商自己的代理设置发（#2775）。
        let mut request = crate::http_client::build_for_relay(
            reqwest::Client::builder().timeout(Duration::from_secs(20)),
            relay,
        )?
        .get(CHATGPT_USAGE_URL)
        .bearer_auth(&credential.access_token)
        .header(reqwest::header::ACCEPT, "application/json");
        if !credential.account_id.is_empty() {
            request = request.header("chatgpt-account-id", credential.account_id.as_str());
        }
        let response = request.send().await?;
        if !response.status().is_success() {
            anyhow::bail!("查询额度失败：HTTP {}", response.status().as_u16());
        }
        parse_chatgpt_usage(&response.json::<Value>().await?, now)
    } else if crate::gateway::accounts::relay_account_kind(relay).as_deref() == Some("copilot") {
        let value = crate::gateway::accounts::copilot_user(relay).await?;
        parse_copilot_user(&value, now)
    } else if crate::gateway::accounts::relay_account_kind(relay).as_deref() == Some("grok") {
        // 先过 prepare：access token 快过期时换新，请求头同步到当前版本。
        let prepared = crate::gateway::accounts::prepare_relay(relay).await?;
        let value = crate::gateway::accounts::grok_billing(&prepared).await?;
        parse_grok_billing(&value, now)
    } else {
        return Ok(None);
    };
    if let Some(quota) = &quota {
        record(&relay.id, quota.clone());
    }
    Ok(quota)
}

/// 后台循环：每 10 分钟查一遍池里打开的订阅账号。失败只记日志。
pub async fn watch(store: GatewayStore) {
    loop {
        if let Ok(gateway) = store.load() {
            for provider in gateway.providers.iter().filter(|provider| provider.enabled) {
                if !supports(&provider.relay) {
                    continue;
                }
                if let Err(error) = refresh(&provider.relay).await {
                    let _ = crate::diagnostic_log::append_diagnostic_log(
                        "gateway.quota_failed",
                        serde_json::json!({
                            "relayId": provider.relay.id,
                            "error": crate::protocol_proxy::error_chain(&error),
                        }),
                    );
                }
            }
        }
        tokio::time::sleep(REFRESH_EVERY).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_grok_billing() {
        let midnight = parse_reset_date("2026-10-12").unwrap();
        let weekly = parse_grok_billing(
            &json!({"config": {
                "creditUsagePercent": 37.5,
                "currentPeriod": {"type": "USAGE_PERIOD_TYPE_WEEKLY", "end": "2026-10-12T02:17:59.504011+00:00"},
                "billingPeriodEnd": "2026-11-01T00:00:00Z",
                "onDemandCap": {"val": 10}, "onDemandUsed": {"val": 5}
            }}),
            7,
        )
        .unwrap();
        assert_eq!(
            weekly,
            ProviderQuota {
                windows: vec![QuotaWindow {
                    window_minutes: Some(10080),
                    used_percent: 37.5,
                    resets_at_ms: Some(midnight + (2 * 3600 + 17 * 60 + 59) * 1000 + 504),
                }],
                updated_at_ms: 7,
            },
            "按量付费不算时间窗口"
        );

        let billing_period = parse_grok_billing(
            &json!({"config": {"creditUsagePercent": 120, "billingPeriodEnd": "2026-10-05T08:00:00+08:00"}}),
            0,
        )
        .unwrap();
        assert_eq!(billing_period.windows[0].window_minutes, None);
        assert_eq!(billing_period.windows[0].used_percent, 100.0);
        assert_eq!(
            billing_period.windows[0].resets_at_ms,
            parse_reset_date("2026-10-05"),
            "带时区偏移的时间换成 UTC"
        );

        let daily = parse_grok_billing(
            &json!({"config": {"currentPeriod": {"type": "USAGE_PERIOD_TYPE_DAILY", "end": "bad"}}}),
            0,
        )
        .unwrap();
        assert_eq!(daily.windows[0].window_minutes, Some(1440));
        assert_eq!(daily.windows[0].used_percent, 0.0);
        assert_eq!(daily.windows[0].resets_at_ms, None);

        assert!(parse_grok_billing(&json!({"error": "unauthorized"}), 0).is_none());
        for bad in [
            "2026-10-12",
            "2026-10-12T25:00:00Z",
            "2026-10-12T02:17:59",
            "2026-10-12T02:17:59.Z",
        ] {
            assert_eq!(parse_rfc3339_ms(bad), None, "{bad}");
        }
    }

    #[test]
    fn reads_codex_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("x-codex-primary-used-percent", "32".parse().unwrap());
        headers.insert("x-codex-primary-window-minutes", "300".parse().unwrap());
        headers.insert("x-codex-primary-reset-after-seconds", "60".parse().unwrap());
        headers.insert("x-codex-secondary-used-percent", "80.5".parse().unwrap());
        headers.insert("x-codex-secondary-window-minutes", "10080".parse().unwrap());
        let quota = from_codex_headers(&headers, 1_000).unwrap();
        assert_eq!(quota.windows.len(), 2);
        assert_eq!(quota.windows[0].window_minutes, Some(300));
        assert_eq!(quota.windows[0].resets_at_ms, Some(61_000));
        assert!(
            (remaining_at(&quota, 1_000) - 0.195).abs() < 1e-9,
            "取最紧的窗口"
        );
        assert!(
            (remaining_at(&quota, 70_000) - 0.195).abs() < 1e-9,
            "5 小时窗口过了重置时间算满额，每周的仍然紧"
        );
        assert!(from_codex_headers(&HeaderMap::new(), 0).is_none());
    }

    #[test]
    fn huge_reset_values_saturate_instead_of_overflowing() {
        let mut headers = HeaderMap::new();
        headers.insert("x-codex-primary-used-percent", "5".parse().unwrap());
        headers.insert(
            "x-codex-primary-reset-after-seconds",
            "1e300".parse().unwrap(),
        );
        let quota = from_codex_headers(&headers, u64::MAX - 1).unwrap();
        assert_eq!(quota.windows[0].resets_at_ms, Some(u64::MAX));

        let usage = json!({
            "rate_limit": {
                "primary_window": { "used_percent": 1, "reset_at": u64::MAX },
                "secondary_window": { "used_percent": 1, "reset_after_seconds": u64::MAX }
            }
        });
        let quota = parse_chatgpt_usage(&usage, 1_000).unwrap();
        assert_eq!(quota.windows[0].resets_at_ms, Some(u64::MAX));
        assert_eq!(quota.windows[1].resets_at_ms, Some(u64::MAX));
    }

    #[test]
    fn forget_drops_a_deleted_providers_quota() {
        record(
            "quota-forget",
            ProviderQuota {
                windows: vec![QuotaWindow {
                    window_minutes: Some(300),
                    used_percent: 90.0,
                    resets_at_ms: None,
                }],
                updated_at_ms: 0,
            },
        );
        assert!(get("quota-forget").is_some());
        forget("quota-forget");
        assert!(get("quota-forget").is_none());
    }

    #[test]
    fn parses_chatgpt_usage_and_copilot_user() {
        let usage = json!({
            "plan_type": "pro",
            "rate_limit": {
                "primary_window": { "used_percent": 12, "limit_window_seconds": 18000, "reset_at": 1_700_000_000 },
                "secondary_window": null
            }
        });
        let quota = parse_chatgpt_usage(&usage, 0).unwrap();
        assert_eq!(quota.windows.len(), 1);
        assert_eq!(quota.windows[0].window_minutes, Some(300));
        assert_eq!(quota.windows[0].resets_at_ms, Some(1_700_000_000_000));

        let user = json!({
            "quota_reset_date": "2026-11-01",
            "quota_snapshots": { "premium_interactions": { "percent_remaining": 75.0, "unlimited": false } }
        });
        let quota = parse_copilot_user(&user, 0).unwrap();
        assert_eq!(quota.windows[0].used_percent, 25.0);
        assert_eq!(quota.windows[0].window_minutes, None);
        assert_eq!(
            quota.windows[0].resets_at_ms,
            Some(1_793_491_200_000),
            "2026-11-01T00:00:00Z"
        );
        let unlimited =
            json!({ "quota_snapshots": { "premium_interactions": { "unlimited": true } } });
        assert!(parse_copilot_user(&unlimited, 0).is_none());
    }
}
