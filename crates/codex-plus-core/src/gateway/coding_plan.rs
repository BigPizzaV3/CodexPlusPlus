//! API Key 型订阅套餐的额度（#3084）：OpenCode Go、Command Code。
//!
//! 两家返回的是 5 小时 / 每周 / 每月窗口的使用率，不是账户金额，所以映射成 [`ProviderQuota`]，
//! 和 ChatGPT、Copilot 的额度共用缓存、后台预取、在途去重与界面展示（见 [`crate::gateway::quota`]）。
//!
//! 按 Base URL 识别，用供应商自己的 `relay.api_key`、按供应商代理设置发请求；
//! 只带 Bearer，不带供应商的自定义请求头（控制面和推理面不是一个地址）。Key 不进日志、不回界面。
//!
//! 这两个用量接口都不是公开稳定文档化的接口，协议参照 CC Switch v4.0.29 的实现与测试
//! （farion1231/cc-switch@efbec38 `src-tauri/src/services/coding_plan.rs`），上游随时可能变形：
//! 解析不了时明确报「响应格式不认识」，不渲染空额度。

use std::time::Duration;

use anyhow::anyhow;
use serde_json::Value;

use crate::gateway::quota::{ProviderQuota, QuotaWindow};
use crate::settings::RelayProfile;

/// OpenCode Go 的用量接口：固定地址，不从推理 Base URL 派生。
pub const OPENCODE_GO_USAGE_URL: &str = "https://opencode.ai/zen/go/v1/usage";
/// Command Code 的控制面根地址；供应商的 `/provider` 是推理数据面，不能拿来查额度。
pub const COMMAND_CODE_API_BASE: &str = "https://api.commandcode.ai";
const TIMEOUT: Duration = Duration::from_secs(15);
const FIVE_HOURS: u64 = 300;
const WEEK: u64 = 7 * 24 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodingPlan {
    OpencodeGo,
    CommandCode,
}

impl CodingPlan {
    fn name(self) -> &'static str {
        match self {
            CodingPlan::OpencodeGo => "OpenCode Go",
            CodingPlan::CommandCode => "Command Code",
        }
    }
}

/// 按 Base URL 识别。OpenCode 只认 Go 的两个入口（`/zen/go`、`/inference/go`）：
/// 按量的 Zen（`/zen/v1`）没有用量接口。Command Code 只认 `api.commandcode.ai/provider`。
pub fn plan_of(base_url: &str) -> Option<CodingPlan> {
    if crate::gateway::opencode::plan_of(base_url) == Some(crate::gateway::opencode::Plan::Go) {
        return Some(CodingPlan::OpencodeGo);
    }
    let url = reqwest::Url::parse(base_url.trim()).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();
    let first = url.path_segments()?.find(|segment| !segment.is_empty());
    (host == "api.commandcode.ai" && first == Some("provider")).then_some(CodingPlan::CommandCode)
}

/// 这个供应商能不能按套餐查额度：普通 API Key 供应商、地址认得、有 Key。
pub fn plan_for_relay(relay: &RelayProfile) -> Option<CodingPlan> {
    if crate::official_login::member_uses_official_login_transport(relay)
        || crate::gateway::accounts::relay_account_kind(relay).is_some()
        || relay.uses_no_auth()
    {
        return None;
    }
    plan_of(&relay.base_url)
}

/// 查这个供应商的套餐额度；地址认不出时返回 None（不发任何请求）。
pub async fn fetch(relay: &RelayProfile, now_ms: u64) -> anyhow::Result<Option<ProviderQuota>> {
    match plan_for_relay(relay) {
        Some(CodingPlan::OpencodeGo) => {
            fetch_opencode_go_at(relay, OPENCODE_GO_USAGE_URL, now_ms).await.map(Some)
        }
        Some(CodingPlan::CommandCode) => {
            fetch_command_code_at(relay, COMMAND_CODE_API_BASE, now_ms).await.map(Some)
        }
        None => Ok(None),
    }
}

fn api_key(relay: &RelayProfile) -> anyhow::Result<&str> {
    let key = relay.api_key.trim();
    if key.is_empty() {
        anyhow::bail!("没有 API Key，查不了额度");
    }
    Ok(key)
}

/// GET 一个 JSON 接口：Bearer + Accept，按供应商代理设置发；非 2xx 按状态码给出可区分的错误。
async fn get_json(
    plan: CodingPlan,
    relay: &RelayProfile,
    url: &str,
    query: &[(&str, &str)],
) -> anyhow::Result<Value> {
    let key = api_key(relay)?;
    let client = crate::http_client::proxied_client_for_relay("CodexPlusPlus", relay)?;
    let response = client
        .get(url)
        .query(query)
        .bearer_auth(key)
        .header(reqwest::header::ACCEPT, "application/json")
        .timeout(TIMEOUT)
        .send()
        .await
        .map_err(|error| anyhow!("连不上 {} 的用量接口：{}", plan.name(), error.without_url()))?;
    let status = response.status().as_u16();
    let body = response
        .text()
        .await
        .map_err(|error| anyhow!("读取 {} 的用量响应失败：{}", plan.name(), error.without_url()))?;
    if !(200..300).contains(&status) {
        return Err(status_error(plan, status, &body));
    }
    serde_json::from_str(&body).map_err(|_| unrecognized(plan))
}

fn unrecognized(plan: CodingPlan) -> anyhow::Error {
    anyhow!("{} 的用量响应格式不认识（上游接口可能已变更）", plan.name())
}

/// 401 / 403 / 429 分开提示；其余只带状态码和脱敏后的错误代码，响应正文不进界面。
fn status_error(plan: CodingPlan, status: u16, body: &str) -> anyhow::Error {
    let code = error_code(body).map(|code| format!("，{code}")).unwrap_or_default();
    match (plan, status) {
        (_, 401) => anyhow!("{} 的 API Key 无效或已过期（HTTP 401）", plan.name()),
        (CodingPlan::OpencodeGo, 403) => {
            anyhow!("Key 有效，但所在 workspace 没有 OpenCode Go 订阅（HTTP 403）")
        }
        (_, 403) => anyhow!("{} 的 Key 没有对应的订阅或权限（HTTP 403{code}）", plan.name()),
        (_, 429) => anyhow!("{} 的用量接口限流（HTTP 429），稍后再试", plan.name()),
        _ => anyhow!("{} 的用量接口返回 HTTP {status}{code}", plan.name()),
    }
}

/// 响应里的错误代码（如 `upgrade_required`）：只认短的标识符，其余内容一律不回显。
fn error_code(body: &str) -> Option<String> {
    let value: Value = serde_json::from_str(body).ok()?;
    let code = value
        .pointer("/error/code")
        .or_else(|| value.get("code"))
        .or_else(|| value.pointer("/error/type"))
        .or_else(|| value.get("error").filter(|error| error.is_string()))?
        .as_str()?
        .trim();
    let plain = !code.is_empty()
        && code.len() <= 48
        && code
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'));
    plain.then(|| code.to_string())
}

/// 数字或数字字符串。
fn number(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()))
        .filter(|number| number.is_finite())
}

/// 重置时间：RFC 3339 字符串，或秒 / 毫秒时间戳；`0`、`null`、非法值都当作没有（免得显示 1970 年）。
fn reset_ms(value: Option<&Value>) -> Option<u64> {
    match value? {
        Value::String(text) => crate::gateway::quota::parse_rfc3339_ms(text.trim()).filter(|ms| *ms > 0),
        Value::Number(number) => {
            let raw = number.as_i64().filter(|raw| *raw > 0)?;
            let ms = if raw < 1_000_000_000_000 { raw.saturating_mul(1000) } else { raw };
            u64::try_from(ms).ok()
        }
        _ => None,
    }
}

fn window(minutes: Option<u64>, used: f64, resets_at_ms: Option<u64>, now_ms: u64) -> QuotaWindow {
    QuotaWindow {
        window_minutes: minutes,
        used_percent: used.clamp(0.0, 100.0),
        resets_at_ms,
        observed_at_ms: Some(now_ms),
    }
}

fn reading(windows: Vec<QuotaWindow>, now_ms: u64) -> ProviderQuota {
    ProviderQuota {
        windows,
        updated_at_ms: now_ms,
        windows_complete: true,
        ..ProviderQuota::default()
    }
}

/// OpenCode Go：`usage.rolling / weekly / monthly`，`percent` 是已用百分比。
/// 单个窗口缺失或 `percent` 不是数字时跳过；三个都解析不了返回 None。
/// `percent == 0` 时上游的 `resetsAt` 是「现在 + 窗口时长」的占位值，不展示。
pub fn parse_opencode_go(value: &Value, now_ms: u64) -> Option<ProviderQuota> {
    let usage = value.get("usage")?.as_object()?;
    let windows: Vec<QuotaWindow> = [("rolling", Some(FIVE_HOURS)), ("weekly", Some(WEEK)), ("monthly", None)]
        .into_iter()
        .filter_map(|(name, minutes)| {
            let item = usage.get(name)?;
            let used = number(item.get("percent")?)?;
            let resets = if used > 0.0 { reset_ms(item.get("resetsAt")) } else { None };
            Some(window(minutes, used, resets, now_ms))
        })
        .collect();
    (!windows.is_empty()).then(|| reading(windows, now_ms))
}

/// 同 [`fetch`] 的 OpenCode Go 分支，地址由调用方给（测试里换成本机的模拟上游）。
pub async fn fetch_opencode_go_at(
    relay: &RelayProfile,
    url: &str,
    now_ms: u64,
) -> anyhow::Result<ProviderQuota> {
    let plan = CodingPlan::OpencodeGo;
    let value = get_json(plan, relay, url, &[]).await?;
    parse_opencode_go(&value, now_ms).ok_or_else(|| unrecognized(plan))
}

/// Command Code：`credits` 的剩余额度 + `windowLimits`（可能在顶层，也可能在 `credits` 下），
/// 订阅周期（`data.currentPeriodEnd`）和本周期花费（`totalCost`）。
///
/// - `windowLimits.limited == true` 时才有 5 小时 / 每周窗口，利用率 = used / cap，`cap <= 0` 跳过；
/// - 每月：已用 = totalCost / (totalCost + monthlyCredits + purchasedCredits + freeCredits)，
///   各项至少按 0 算，总额为 0 时是 0%；重置时间取订阅的 `currentPeriodEnd`。
pub fn parse_command_code(
    credits: &Value,
    subscription: Option<&Value>,
    summary: &Value,
    now_ms: u64,
) -> Option<ProviderQuota> {
    let pool = credits.get("credits")?.as_object()?;
    let total_cost = number(summary.get("totalCost")?)?.max(0.0);
    let remaining: f64 = ["monthlyCredits", "purchasedCredits", "freeCredits"]
        .iter()
        .map(|key| pool.get(*key).and_then(number).unwrap_or(0.0).max(0.0))
        .sum();
    let mut windows = Vec::new();
    let limits = credits
        .get("windowLimits")
        .or_else(|| pool.get("windowLimits"))
        .filter(|limits| limits.get("limited").and_then(Value::as_bool) == Some(true));
    if let Some(limits) = limits {
        for (name, minutes) in [("fiveHour", FIVE_HOURS), ("weekly", WEEK)] {
            let Some(item) = limits.get(name) else {
                continue;
            };
            let (Some(used), Some(cap)) = (item.get("used").and_then(number), item.get("cap").and_then(number)) else {
                continue;
            };
            if cap <= 0.0 {
                continue;
            }
            windows.push(window(Some(minutes), used / cap * 100.0, reset_ms(item.get("resetAt")), now_ms));
        }
    }
    let pool_total = total_cost + remaining;
    let used = if pool_total > 0.0 { total_cost / pool_total * 100.0 } else { 0.0 };
    let reset = subscription.and_then(|value| reset_ms(value.pointer("/data/currentPeriodEnd")));
    windows.push(window(None, used, reset, now_ms));
    Some(reading(windows, now_ms))
}

/// 同 [`fetch`] 的 Command Code 分支，控制面根地址由调用方给（测试里换成本机的模拟上游）。
/// 依次请求 whoami → credits → subscriptions → usage summary，任何一步失败整体失败（不降级成 0%）。
pub async fn fetch_command_code_at(
    relay: &RelayProfile,
    base: &str,
    now_ms: u64,
) -> anyhow::Result<ProviderQuota> {
    let plan = CodingPlan::CommandCode;
    let base = base.trim_end_matches('/');
    let whoami = get_json(plan, relay, &format!("{base}/alpha/whoami"), &[("limits", "1")]).await?;
    let org = whoami.pointer("/org/id").and_then(Value::as_str).map(str::to_string);
    let org_query: Vec<(&str, &str)> = org.as_deref().map(|org| ("orgId", org)).into_iter().collect();
    let credits = get_json(plan, relay, &format!("{base}/alpha/billing/credits"), &org_query).await?;
    let subscription =
        get_json(plan, relay, &format!("{base}/alpha/billing/subscriptions"), &org_query).await?;
    let since = subscription
        .pointer("/data/currentPeriodStart")
        .and_then(Value::as_str)
        .map(str::to_string);
    let mut summary_query: Vec<(&str, &str)> = since.as_deref().map(|since| ("since", since)).into_iter().collect();
    summary_query.extend(org_query.iter().copied());
    let summary = get_json(plan, relay, &format!("{base}/alpha/usage/summary"), &summary_query).await?;
    parse_command_code(&credits, Some(&subscription), &summary, now_ms).ok_or_else(|| unrecognized(plan))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NOW: u64 = 1_790_000_000_000;

    #[test]
    fn detects_only_the_plans_with_a_usage_api() {
        for url in [
            "https://opencode.ai/zen/go",
            "https://opencode.ai/zen/go/v1",
            "https://opencode.ai/inference/go/v1",
            "https://OpenCode.ai/inference/go/anthropic",
        ] {
            assert_eq!(plan_of(url), Some(CodingPlan::OpencodeGo), "{url}");
        }
        for url in [
            "https://api.commandcode.ai/provider",
            "https://api.commandcode.ai/provider/v1",
            "https://API.commandcode.ai/provider/v1/",
        ] {
            assert_eq!(plan_of(url), Some(CodingPlan::CommandCode), "{url}");
        }
        for url in [
            "https://opencode.ai/zen/v1",
            "https://opencode.ai/inference/v1",
            "https://commandcode.ai/provider",
            "https://api.commandcode.ai/providers",
            "https://api.commandcode.ai/alpha/whoami",
            "https://evil.test/api.commandcode.ai/provider",
            "https://api.deepseek.com/v1",
            "",
        ] {
            assert_eq!(plan_of(url), None, "{url}");
        }
    }

    #[test]
    fn opencode_go_maps_three_windows_and_skips_bad_ones() {
        let quota = parse_opencode_go(
            &json!({"usage": {
                "rolling": {"status": "ok", "percent": 37, "resetsAt": "2026-08-26T14:12:03.000Z"},
                "weekly": {"status": "ok", "percent": 62, "resetsAt": "2026-08-31T00:00:00.000Z"},
                "monthly": {"status": "rate-limited", "percent": 100, "resetsAt": "2026-09-11T00:00:00.000Z"}
            }}),
            NOW,
        )
        .unwrap();
        let summary: Vec<_> = quota.windows.iter().map(|w| (w.window_minutes, w.used_percent)).collect();
        assert_eq!(summary, vec![(Some(300), 37.0), (Some(10080), 62.0), (None, 100.0)]);
        assert_eq!(quota.windows[0].resets_at_ms, Some(1_787_753_523_000));
        assert!(quota.windows_complete);

        // percent 为 0：重置时间是占位值，不展示。
        let zero = parse_opencode_go(&json!({"usage": {"rolling": {"percent": 0, "resetsAt": "2026-08-26T15:00:00.000Z"}}}), NOW).unwrap();
        assert_eq!(zero.windows[0].resets_at_ms, None);

        // 坏窗口跳过，数字字符串照收。
        let partial = parse_opencode_go(&json!({"usage": {"rolling": {"status": "ok"}, "weekly": {"percent": "12"}, "monthly": null}}), NOW).unwrap();
        assert_eq!(partial.windows.len(), 1);
        assert_eq!(partial.windows[0].window_minutes, Some(10080));
        assert_eq!(partial.windows[0].used_percent, 12.0);

        // 旧的扁平形态、三个窗口都不认识：不渲染空额度。
        assert!(parse_opencode_go(&json!({"rollingUsage": {"usagePercent": 37, "resetInSec": 3600}}), NOW).is_none());
        assert!(parse_opencode_go(&json!({"usage": {"rolling": {"percent": "n/a"}}}), NOW).is_none());
    }

    #[test]
    fn command_code_reads_nested_or_top_level_window_limits() {
        let nested = json!({"credits": {"planId": "provider", "monthlyCredits": 12.5, "purchasedCredits": 3.0, "freeCredits": 0.5,
            "windowLimits": {"limited": true,
                "fiveHour": {"used": 4.0, "cap": 14.0, "resetAt": "2026-09-12T18:00:00.000Z"},
                "weekly": {"used": 12.0, "cap": 40.0, "resetAt": "2026-09-16T00:00:00.000Z"}}}});
        let subscription = json!({"data": {"planId": "goat", "currentPeriodStart": "2026-09-01T00:00:00.000Z", "currentPeriodEnd": "2026-10-01T00:00:00.000Z"}});
        let quota = parse_command_code(&nested, Some(&subscription), &json!({"totalCost": 7.0}), NOW).unwrap();
        assert_eq!(quota.windows.len(), 3);
        assert!((quota.windows[0].used_percent - 4.0 / 14.0 * 100.0).abs() < 1e-9);
        assert_eq!(quota.windows[0].window_minutes, Some(300));
        assert_eq!(quota.windows[1].used_percent, 30.0);
        assert_eq!(quota.windows[1].window_minutes, Some(10080));
        assert!((quota.windows[2].used_percent - 7.0 / 23.0 * 100.0).abs() < 1e-9);
        assert_eq!(quota.windows[2].window_minutes, None);
        assert_eq!(quota.windows[2].resets_at_ms, crate::gateway::quota::parse_rfc3339_ms("2026-10-01T00:00:00.000Z"));

        // 真实 goat 形态：windowLimits 在顶层，resetAt 为 0 视为没有。
        let top = json!({"credits": {"monthlyCredits": 70, "purchasedCredits": 0, "freeCredits": 0},
            "windowLimits": {"limited": true, "exceeded": null,
                "fiveHour": {"used": 0, "cap": 14, "resetAt": 0},
                "weekly": {"used": 0, "cap": 35, "resetAt": 0}}});
        let quota = parse_command_code(&top, None, &json!({"totalCost": 0, "periodBasis": "billing-period"}), NOW).unwrap();
        assert_eq!(quota.windows.len(), 3);
        assert!(quota.windows.iter().all(|w| w.used_percent == 0.0 && w.resets_at_ms.is_none()));
    }

    #[test]
    fn command_code_skips_unlimited_and_broken_windows() {
        // 按量付费：limited=false，只剩每月。
        let payg = json!({"credits": {"monthlyCredits": 0, "purchasedCredits": 18.25, "windowLimits": {"limited": false,
            "fiveHour": {"used": 1, "cap": 2}}}});
        let quota = parse_command_code(&payg, None, &json!({"totalCost": 0}), NOW).unwrap();
        assert_eq!(quota.windows.len(), 1);
        assert_eq!(quota.windows[0].window_minutes, None);
        // cap 为 0 的窗口跳过；负数余额按 0 算。
        let broken = json!({"credits": {"monthlyCredits": -5, "windowLimits": {"limited": true,
            "fiveHour": {"used": 1, "cap": 0}, "weekly": {"used": 1, "cap": 10, "resetAt": 1_790_100_000}}}});
        let quota = parse_command_code(&broken, None, &json!({"totalCost": 1}), NOW).unwrap();
        assert_eq!(quota.windows.iter().map(|w| w.window_minutes).collect::<Vec<_>>(), vec![Some(10080), None]);
        assert_eq!(quota.windows[0].resets_at_ms, Some(1_790_100_000_000));
        assert_eq!(quota.windows[1].used_percent, 100.0);
        // 缺字段：认不出。
        assert!(parse_command_code(&json!({}), None, &json!({"totalCost": 0}), NOW).is_none());
        assert!(parse_command_code(&json!({"credits": {}}), None, &json!({}), NOW).is_none());
    }

    #[test]
    fn error_codes_are_short_identifiers_only() {
        assert_eq!(error_code(r#"{"error":{"code":"upgrade_required"}}"#).as_deref(), Some("upgrade_required"));
        assert_eq!(error_code(r#"{"error":"sk-live-abc 被拒绝"}"#), None);
        assert_eq!(error_code("slow down"), None);
        let message = status_error(CodingPlan::CommandCode, 500, r#"{"message":"key sk-secret leaked"}"#).to_string();
        assert!(message.contains("HTTP 500") && !message.contains("sk-secret"), "{message}");
    }
}
