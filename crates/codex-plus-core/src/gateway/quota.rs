//! 订阅账号的额度：ChatGPT 账号的 5 小时 / 每周用量百分比，Copilot 的本月高级请求，
//! SuperGrok 的本周期额度。
//!
//! 两个来源：ChatGPT 后端每次响应都带的 `x-codex-*` 响应头，以及后台定期查询额度接口
//! （ChatGPT `wham/usage`、Copilot `copilot_internal/user`、SuperGrok `billing`）。
//! 只存在内存，重启后重新查；同一官方账号的不同供应商别名按身份作用域共享。
//! 模型组「额度优先」按剩余额度排先后，供应商池在账号行上显示。

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use reqwest::header::HeaderMap;
use serde::Serialize;
use serde_json::Value;

use crate::gateway::store::{
    GatewaySettings, GatewayStore, SubscriptionLimitAction, SubscriptionPolicy,
    SubscriptionUnknownUsage,
};
use crate::settings::RelayProfile;

const CHATGPT_USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
/// 后台多久查一次额度。
const REFRESH_EVERY: Duration = Duration::from_secs(60);
const RETRY_AFTER_MS: u64 = 30_000;

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaWindow {
    /// 窗口时长（分钟）：300 是 5 小时、10080 是每周；按月重置的额度（Copilot）为 None。
    pub window_minutes: Option<u64>,
    pub used_percent: f64,
    /// 重置时间（毫秒时间戳）。
    pub resets_at_ms: Option<u64>,
    /// 合并部分响应头时保留每个窗口原来的观测时间。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderQuota {
    pub windows: Vec<QuotaWindow>,
    pub updated_at_ms: u64,
    /// 完整 usage 库存；部分响应头不能证明未返回的窗口不适用。
    pub windows_complete: bool,
    pub official_allowed: Option<bool>,
    pub official_limit_reached: Option<bool>,
    /// 积分独立于订阅窗口，不授予额外积分消费权限。
    pub credits_available: Option<bool>,
}

#[derive(Default)]
struct QuotaCache {
    readings: HashMap<String, ProviderQuota>,
    accounts: HashMap<String, String>,
}

fn cache() -> &'static Mutex<QuotaCache> {
    static CACHE: OnceLock<Mutex<QuotaCache>> = OnceLock::new();
    CACHE.get_or_init(Mutex::default)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

pub fn record(relay_id: &str, quota: ProviderQuota) {
    if quota.windows.is_empty() && !quota.windows_complete {
        return;
    }
    let mut cache = cache().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let key = cache.accounts.get(relay_id).cloned().unwrap_or_else(|| relay_id.to_string());
    insert_reading(&mut cache, key, quota);
}

fn insert_reading(cache: &mut QuotaCache, key: String, mut quota: ProviderQuota) {
    for window in &mut quota.windows {
        window.observed_at_ms.get_or_insert(quota.updated_at_ms);
    }
    if let Some(old) = cache.readings.get(&key) {
        if old.windows_complete && old.updated_at_ms > quota.updated_at_ms {
            return;
        }
        if quota.windows_complete {
            // 较晚才返回的完整查询不能覆写查询期间已到达的新响应头。
            for window in &old.windows {
                if window.observed_at_ms.unwrap_or(old.updated_at_ms) > quota.updated_at_ms {
                    quota.windows.retain(|item| item.window_minutes != window.window_minutes);
                    quota.windows.push(window.clone());
                }
            }
        } else {
            let mut merged = old.clone();
            for window in quota.windows {
                if let Some(previous) = merged.windows.iter_mut().find(|item| item.window_minutes == window.window_minutes) {
                    if previous.observed_at_ms.unwrap_or(old.updated_at_ms) <= quota.updated_at_ms {
                        *previous = window;
                    }
                } else {
                    merged.windows.push(window);
                }
            }
            // 完整库存和官方标记仍保留原时间，不能被单个窗口的头部刷新。
            if !merged.windows_complete {
                merged.updated_at_ms = merged.updated_at_ms.max(quota.updated_at_ms);
            }
            quota = merged;
        }
    }
    quota.windows.sort_by_key(|window| window.window_minutes);
    cache.readings.insert(key, quota);
}

/// Share the vendor's allowance, never rotating OAuth tokens, across aliases and login sessions.
pub fn record_for_relay(relay: &RelayProfile, quota: ProviderQuota) {
    if quota.windows.is_empty() && !quota.windows_complete {
        return;
    }
    if !crate::official_login::member_uses_official_login_transport(relay) {
        record(&relay.id, quota);
        return;
    }
    let Some(scope) = crate::native_auth::account_scope(&relay.auth_contents) else {
        // An incomplete identity cannot safely inherit a previous occupant's reading.
        forget(&relay.id);
        return;
    };
    let key = format!("chatgpt:{scope}");
    let mut cache = cache().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    cache.accounts.insert(relay.id.clone(), key.clone());
    cache.readings.remove(&relay.id);
    insert_reading(&mut cache, key, quota);
}

pub fn quota_for_relay(relay: &RelayProfile) -> Option<ProviderQuota> {
    if !crate::official_login::member_uses_official_login_transport(relay) {
        return get(&relay.id);
    }
    let scope = crate::native_auth::account_scope(&relay.auth_contents)?;
    let key = format!("chatgpt:{scope}");
    let mut cache = cache().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    cache.accounts.insert(relay.id.clone(), key.clone());
    cache.readings.get(&key).cloned()
}

/// 忘掉一个供应商的额度（删除供应商时用：`login:chatgpt` 的 id 会被新登录的账号复用，
/// 不清的话新账号会沿用旧账号的额度）。
pub fn forget(relay_id: &str) {
    let mut cache = cache().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    cache.accounts.remove(relay_id);
    cache.readings.remove(relay_id);
}

/// 秒（可能是小数、负数或很大的数）→ 毫秒，越界时取边界值，不溢出。
fn seconds_to_ms(seconds: f64) -> u64 {
    // f64 → u64 的 `as` 本身是饱和转换（负数、NaN 得 0，过大得 u64::MAX）。
    (seconds * 1000.0) as u64
}

pub fn get(relay_id: &str) -> Option<ProviderQuota> {
    let cache = cache().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let key = cache.accounts.get(relay_id).map(String::as_str).unwrap_or(relay_id);
    cache.readings.get(key).cloned()
}

#[derive(Debug, Clone)]
pub struct SubscriptionBlocked {
    pub action: SubscriptionLimitAction,
    pub status: u16,
    pub message: String,
}

impl std::fmt::Display for SubscriptionBlocked {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for SubscriptionBlocked {}

#[derive(Default)]
pub struct DispatchGate {
    subscription_only: bool,
}

impl DispatchGate {
    pub fn check(
        &mut self,
        gateway: &GatewaySettings,
        relay: &RelayProfile,
    ) -> Result<(), SubscriptionBlocked> {
        if self.subscription_only
            && !crate::official_login::member_uses_official_login_transport(relay)
        {
            return Err(SubscriptionBlocked {
                action: SubscriptionLimitAction::NextPermitted,
                status: 403,
                message: "订阅账号的回退策略仅允许下一个订阅账号，未允许 API 回退".to_string(),
            });
        }
        let result = dispatch_decision(gateway, relay);
        if result.as_ref().is_err_and(|blocked| {
            matches!(&blocked.action, &SubscriptionLimitAction::NextPermitted)
        }) {
            self.subscription_only = true;
        }
        result
    }
}

fn action_rank(action: &SubscriptionLimitAction) -> u8 {
    match action {
        SubscriptionLimitAction::Stop => 0,
        SubscriptionLimitAction::NextPermitted => 1,
        SubscriptionLimitAction::ApiFallback => 2,
    }
}

fn shared_policy(gateway: &GatewaySettings, relay: &RelayProfile) -> Option<SubscriptionPolicy> {
    let mut policy = gateway.provider(&relay.id)?.subscription_policy.clone()?;
    let Some(scope) = crate::native_auth::account_scope(&relay.auth_contents) else {
        return Some(policy);
    };
    for alias in &gateway.providers {
        if !alias.enabled || !crate::official_login::member_uses_official_login_transport(&alias.relay)
            || crate::native_auth::account_scope(&alias.relay.auth_contents).as_ref() != Some(&scope)
        {
            continue;
        }
        let Some(other) = alias.subscription_policy.as_ref() else {
            continue;
        };
        policy.max_age_seconds = policy.max_age_seconds.min(other.max_age_seconds);
        if matches!(
            &other.unknown_usage,
            &SubscriptionUnknownUsage::Block
        ) {
            policy.unknown_usage = SubscriptionUnknownUsage::Block;
        }
        if action_rank(&other.on_limit) < action_rank(&policy.on_limit) {
            policy.on_limit = other.on_limit.clone();
        }
        // Keep effective per-window minima before reducing the common default.
        let mut windows = policy.window_used_percent_limits.clone();
        for key in other.window_used_percent_limits.keys() {
            windows.entry(key.clone()).or_insert_with(|| policy.default_used_percent_limit.unwrap_or(100));
        }
        for (key, cap) in &mut windows {
            let other_cap = other.window_used_percent_limits.get(key).copied()
                .or(other.default_used_percent_limit).unwrap_or(100);
            *cap = (*cap).min(other_cap);
        }
        policy.window_used_percent_limits = windows;
        policy.default_used_percent_limit = Some(
            policy.default_used_percent_limit.unwrap_or(100)
                .min(other.default_used_percent_limit.unwrap_or(100)),
        );
    }
    Some(policy)
}

/// A local admission decision only: this neither reserves official quota nor changes native auth.
pub fn dispatch_decision(
    gateway: &GatewaySettings,
    relay: &RelayProfile,
) -> Result<(), SubscriptionBlocked> {
    dispatch_decision_at(gateway, relay, now_ms())
}

fn dispatch_decision_at(
    gateway: &GatewaySettings,
    relay: &RelayProfile,
    now: u64,
) -> Result<(), SubscriptionBlocked> {
    if !crate::official_login::member_uses_official_login_transport(relay) {
        return Ok(());
    }
    let denied = |action, status, reason: &str| SubscriptionBlocked {
        action,
        status,
        message: format!("ChatGPT 订阅账号「{}」：{reason}", relay.name),
    };
    let Some(policy) = shared_policy(gateway, relay) else {
        return Ok(());
    };
    let unknown = |reason: &str| {
        if matches!(
            &policy.unknown_usage,
            &SubscriptionUnknownUsage::Allow
        ) {
            Ok(())
        } else {
            Err(denied(policy.on_limit.clone(), 503, reason))
        }
    };
    if policy.default_used_percent_limit == Some(0) {
        return Err(denied(policy.on_limit.clone(), 429, "订阅额度已用上限设为 0%，已停止新请求"));
    }
    let Some(quota) = quota_for_relay(relay) else {
        return unknown("官方额度未知，请刷新额度或明确允许未知额度");
    };
    let fresh = |at: u64| at <= now && now - at <= policy.max_age_seconds.saturating_mul(1000);
    let inventory_fresh = fresh(quota.updated_at_ms);
    if inventory_fresh && (quota.official_allowed == Some(false)
        || (quota.official_limit_reached == Some(true) && quota.official_allowed != Some(true)))
    {
        return Err(denied(policy.on_limit.clone(), 429, "官方订阅额度受限，请等待重置或刷新额度"));
    }
    // 有效完整库存内没有的窗口不适用；部分头部的缺失仍然未知。
    let missing_window = !quota.windows_complete && policy.window_used_percent_limits.keys().any(|key| {
        !quota.windows.iter().any(|window| window.window_minutes.map(|minutes| minutes.to_string()).as_ref() == Some(key))
    });
    let mut uncertain = !inventory_fresh || missing_window
        || (quota.windows.is_empty() && !(quota.windows_complete && quota.official_allowed == Some(true)));
    for window in &quota.windows {
        if !fresh(window.observed_at_ms.unwrap_or(quota.updated_at_ms))
            || !window.used_percent.is_finite() || !(0.0..=100.0).contains(&window.used_percent) {
            uncertain = true;
            continue;
        }
        if window.resets_at_ms.is_some_and(|reset| reset <= now) {
            uncertain = true;
            continue;
        }
        let cap = window.window_minutes
            .and_then(|minutes| policy.window_used_percent_limits.get(&minutes.to_string()).copied())
            .or(policy.default_used_percent_limit).unwrap_or(100).min(100);
        if window.used_percent >= f64::from(cap) {
            let label = window.window_minutes.map(|minutes| format!("{minutes} 分钟窗口"))
                .unwrap_or_else(|| "官方窗口".to_string());
            return Err(denied(policy.on_limit.clone(), 429,
                &format!("{label}已用 {:.1}%，达到 {cap}% 上限；已停止该账号的新请求", window.used_percent)));
        }
    }
    if uncertain {
        return unknown("官方窗口已重置或读数不完整，请刷新额度");
    }
    Ok(())
}

/// 剩余比例（0–1），保持旧版排序：已过重置时间的窗口按满额算，不代替派发准入检查。
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

/// 响应头是部分观测，窗口时长以字段为准，不能由 primary/secondary 推断。
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
        if !used.is_finite() || !(0.0..=100.0).contains(&used) {
            continue;
        }
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
            observed_at_ms: Some(now_ms),
        });
    }
    (!windows.is_empty()).then_some(ProviderQuota {
        windows,
        updated_at_ms: now_ms,
        ..ProviderQuota::default()
    })
}

/// `wham/usage` 的返回：`rate_limit.primary_window` / `secondary_window`。
pub fn parse_chatgpt_usage(value: &Value, now_ms: u64) -> Option<ProviderQuota> {
    let limit = value.get("rate_limit")?.as_object()?;
    let mut windows = Vec::new();
    for slot in ["primary_window", "secondary_window"] {
        let window = limit.get(slot)?;
        if window.is_null() {
            continue;
        }
        let window = window.as_object()?;
        let used = window.get("used_percent")?.as_f64()?;
        let seconds = window.get("limit_window_seconds")?.as_u64()?;
        if !used.is_finite() || !(0.0..=100.0).contains(&used) || seconds < 60 || seconds % 60 != 0 {
            return None;
        }
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
            window_minutes: Some(seconds / 60),
            used_percent: used,
            resets_at_ms,
            observed_at_ms: Some(now_ms),
        });
    }
    let allowed = limit.get("allowed").and_then(Value::as_bool);
    let reached = limit.get("limit_reached").and_then(Value::as_bool);
    if ["allowed", "limit_reached"].iter().any(|key| limit.get(*key).is_some_and(|value| !value.is_boolean())) {
        return None;
    }
    Some(ProviderQuota {
        windows,
        updated_at_ms: now_ms,
        windows_complete: true,
        official_allowed: allowed,
        official_limit_reached: reached,
        credits_available: value.get("credits").and_then(|credits| {
            let unlimited = credits.get("unlimited").and_then(Value::as_bool);
            let has = credits.get("has_credits").and_then(Value::as_bool);
            if unlimited == Some(true) || has == Some(true) { Some(true) } else { has.or(unlimited) }
        }),
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
            observed_at_ms: Some(now_ms),
        }],
        updated_at_ms: now_ms,
        ..ProviderQuota::default()
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
            observed_at_ms: Some(now_ms),
        }],
        updated_at_ms: now_ms,
        ..ProviderQuota::default()
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

#[derive(Default)]
struct RefreshFlight {
    lock: tokio::sync::Mutex<()>,
    outcome: Mutex<Option<(Instant, Result<Option<ProviderQuota>, String>)>>,
}

fn refresh_key(relay: &RelayProfile) -> String {
    if !relay.official_account_ref.trim().is_empty() {
        return format!("credential:{}", relay.official_account_ref.trim());
    }
    format!("credential:{}", relay.id)
}

fn refresh_flight(relay: &RelayProfile) -> Arc<RefreshFlight> {
    static FLIGHTS: OnceLock<Mutex<HashMap<String, Arc<RefreshFlight>>>> = OnceLock::new();
    FLIGHTS.get_or_init(Mutex::default).lock().unwrap_or_else(|error| error.into_inner())
        .entry(refresh_key(relay)).or_default().clone()
}

fn reading_fresh(quota: &ProviderQuota, now: u64, max_age_ms: u64, official: bool) -> bool {
    let fresh = |at| at <= now && now - at < max_age_ms;
    (!official || quota.windows_complete)
        && fresh(quota.updated_at_ms)
        && quota.windows.iter().all(|window| {
            fresh(window.observed_at_ms.unwrap_or(quota.updated_at_ms))
                && window.resets_at_ms.is_none_or(|reset| reset > now)
        })
}

async fn refresh_coordinated<F, Fut>(
    relay: &RelayProfile, max_age_ms: Option<u64>, fetch: F,
) -> anyhow::Result<Option<ProviderQuota>>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<Option<ProviderQuota>>>,
{
    let started = Instant::now();
    let flight = refresh_flight(relay);
    // 取消查询时异步锁自动释放，下一次刷新不会永久等待。
    let _guard = flight.lock.lock().await;
    let previous = flight.outcome.lock().unwrap_or_else(|error| error.into_inner()).clone();
    if let Some((finished, result)) = previous {
        if finished >= started || (max_age_ms.is_some() && result.is_err()
            && finished.elapsed().as_millis() < u128::from(RETRY_AFTER_MS))
            || (matches!(&result, Ok(None)) && max_age_ms.is_some_and(|age|
                finished.elapsed().as_millis() < u128::from(age)))
        {
            return result.map_err(anyhow::Error::msg);
        }
    }
    if let Some(max_age) = max_age_ms
        && let Some(quota) = quota_for_relay(relay)
        && reading_fresh(&quota, now_ms(), max_age,
            crate::official_login::member_uses_official_login_transport(relay))
    {
        return Ok(Some(quota));
    }
    let result = fetch().await;
    if let Ok(Some(quota)) = &result {
        record_for_relay(relay, quota.clone());
    }
    *flight.outcome.lock().unwrap_or_else(|error| error.into_inner()) =
        Some((Instant::now(), result.as_ref().map(Clone::clone).map_err(|error| format!("{error:#}"))));
    result
}

/// 手动刷新可绕过缓存，与同一凭证的在途查询共享结果。
pub async fn refresh(relay: &RelayProfile) -> anyhow::Result<Option<ProviderQuota>> {
    refresh_coordinated(relay, None, || fetch_reading(relay)).await
}

async fn fetch_reading(relay: &RelayProfile) -> anyhow::Result<Option<ProviderQuota>> {
    let now = now_ms();
    let quota = if crate::official_login::member_uses_official_login_transport(relay) {
        let credential = crate::official_login::resolve_official_credential_for_quota(relay).await?;
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
        Some(parse_chatgpt_usage(&response.json::<Value>().await?, now)
            .ok_or_else(|| anyhow::anyhow!("官方额度响应不完整，保留上次读数"))?)
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
    Ok(quota)
}

/// 包含尚未启用的已导入账号，按凭证引用去重。有效缓存与失败退避限制轮询请求。
fn prefetch_targets(gateway: &GatewaySettings) -> Vec<(&RelayProfile, u64)> {
    let mut targets: Vec<(&RelayProfile, u64)> = Vec::new();
    let mut positions = HashMap::new();
    for provider in &gateway.providers {
        let relay = &provider.relay;
        if provider.source == "native:external"
            || crate::official_login::is_native_owned_login(relay)
            || !supports(relay)
        {
            continue;
        }
        let max_age_ms = provider.subscription_policy.as_ref()
            .map(|policy| policy.max_age_seconds.saturating_mul(500))
            .unwrap_or(300_000).clamp(1_000, 300_000);
        let next_position = targets.len();
        let position = *positions.entry(refresh_key(relay)).or_insert(next_position);
        if position == next_position {
            targets.push((relay, max_age_ms));
        } else {
            targets[position].1 = targets[position].1.min(max_age_ms);
        }
    }
    targets
}

pub async fn prefetch(gateway: &GatewaySettings) {
    for (relay, max_age_ms) in prefetch_targets(gateway) {
        let _ = refresh_coordinated(relay, Some(max_age_ms), || fetch_reading(relay)).await;
    }
}

/// 启动立即预取；之后按缓存年龄和重置时间更新。
pub async fn watch(store: GatewayStore) {
    loop {
        if let Ok(gateway) = store.load() {
            prefetch(&gateway).await;
        }
        tokio::time::sleep(REFRESH_EVERY).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn canonical_gateway_fixture(
        relay: &RelayProfile,
    ) -> (tempfile::TempDir, super::super::store::GatewayPathTestScope) {
        let directory = tempfile::tempdir().unwrap();
        let scope = super::super::store::GatewayPathTestScope::new(directory.path());
        let mut gateway = GatewaySettings::default();
        add_subscription(&mut gateway, relay, None);
        GatewayStore::default().save(&gateway).unwrap();
        (directory, scope)
    }

    fn subscription_relay(id: &str, subject: &str) -> RelayProfile {
        use base64::Engine;
        let claims = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&json!({
                "iss": "https://auth.openai.com",
                "sub": subject,
            }))
            .unwrap(),
        );
        RelayProfile {
            id: id.to_string(),
            name: id.to_string(),
            relay_mode: crate::settings::RelayMode::Official,
            protocol: crate::settings::RelayProtocol::Responses,
            base_url: String::new(),
            api_key: String::new(),
            auth_contents: json!({
                "auth_mode": "chatgpt",
                "tokens": {
                    "id_token": format!("header.{claims}.signature"),
                    "access_token": format!("access-{id}"),
                    "refresh_token": format!("refresh-{id}"),
                    "account_id": "quota-test-workspace",
                },
            })
            .to_string(),
            ..RelayProfile::default()
        }
    }

    fn add_subscription(
        gateway: &mut GatewaySettings,
        relay: &RelayProfile,
        policy: Option<SubscriptionPolicy>,
    ) {
        gateway.providers.push(
            serde_json::from_value(json!({
                "relay": relay,
                "subscriptionPolicy": policy,
            }))
            .unwrap(),
        );
    }

    fn reading(used_percent: f64, updated_at_ms: u64, resets_at_ms: u64) -> ProviderQuota {
        ProviderQuota {
            windows: vec![QuotaWindow {
                window_minutes: Some(300),
                used_percent,
                resets_at_ms: Some(resets_at_ms),
                ..QuotaWindow::default()
            }],
            updated_at_ms,
            ..ProviderQuota::default()
        }
    }

    #[test]
    fn legacy_subscription_is_usable_without_a_policy_or_usage_reading() {
        let relay = subscription_relay("quota-legacy", "quota-legacy");
        let mut gateway = GatewaySettings::default();
        add_subscription(&mut gateway, &relay, None);
        assert!(dispatch_decision_at(&gateway, &relay, 1_000).is_ok());
        record_for_relay(&relay, reading(0.0, 1_000, 2_000));
        assert!(dispatch_decision_at(&gateway, &relay, 1_000).is_ok());
        let mut exhausted = reading(100.0, 1_000, 2_000);
        exhausted.official_allowed = Some(false);
        record_for_relay(&relay, exhausted);
        assert!(dispatch_decision_at(&gateway, &relay, 1_000).is_ok());
        let alias = subscription_relay("quota-legacy-alias", "quota-legacy");
        add_subscription(&mut gateway, &alias, Some(SubscriptionPolicy {
            default_used_percent_limit: Some(0),
            ..SubscriptionPolicy::default()
        }));
        assert!(dispatch_decision_at(&gateway, &relay, 1_000).is_ok());
        assert_eq!(dispatch_decision_at(&gateway, &alias, 1_000).unwrap_err().status, 429);
        let mut available = reading(0.0, 1_001, 2_000);
        available.windows_complete = true;
        available.official_allowed = Some(true);
        record_for_relay(&relay, available);
        gateway.providers.pop();
        gateway.providers[0].subscription_policy = Some(SubscriptionPolicy::default());
        assert!(dispatch_decision_at(&gateway, &relay, 1_001).is_ok());
        let legacy: SubscriptionPolicy = serde_json::from_value(json!({"permitted": false})).unwrap();
        assert_eq!(legacy, SubscriptionPolicy::default());
        assert!(serde_json::to_value(&legacy).unwrap().get("permitted").is_none());
    }

    #[test]
    fn zero_cap_and_reached_cap_block_even_a_fresh_official_reading() {
        let relay = subscription_relay("quota-zero", "quota-zero");
        let mut gateway = GatewaySettings::default();
        add_subscription(
            &mut gateway,
            &relay,
            Some(SubscriptionPolicy {
                default_used_percent_limit: Some(0),
                ..SubscriptionPolicy::default()
            }),
        );
        record_for_relay(&relay, reading(0.0, 1_000, 2_000));
        assert_eq!(
            dispatch_decision_at(&gateway, &relay, 1_000)
                .unwrap_err()
                .status,
            429
        );
        gateway.providers[0]
            .subscription_policy
            .as_mut()
            .unwrap()
            .default_used_percent_limit = Some(80);
        record_for_relay(&relay, reading(79.9, 1_001, 2_000));
        assert!(dispatch_decision_at(&gateway, &relay, 1_001).is_ok());
        record_for_relay(&relay, reading(80.0, 1_002, 2_000));
        assert_eq!(
            dispatch_decision_at(&gateway, &relay, 1_002)
                .unwrap_err()
                .status,
            429
        );
    }

    #[test]
    fn unknown_stale_future_and_reset_windows_require_explicit_allow() {
        let relay = subscription_relay("quota-uncertain", "quota-uncertain");
        let mut gateway = GatewaySettings::default();
        add_subscription(
            &mut gateway,
            &relay,
            Some(SubscriptionPolicy {
                unknown_usage: SubscriptionUnknownUsage::Block,
                ..SubscriptionPolicy::default()
            }),
        );
        assert_eq!(
            dispatch_decision_at(&gateway, &relay, 1_000)
                .unwrap_err()
                .status,
            503
        );
        record_for_relay(&relay, reading(20.0, 1_000, 1_000_000));
        assert!(dispatch_decision_at(&gateway, &relay, 1_000).is_ok());
        for now in [999, 601_001] {
            assert_eq!(
                dispatch_decision_at(&gateway, &relay, now)
                    .unwrap_err()
                    .status,
                503
            );
        }
        record_for_relay(&relay, reading(20.0, 1_001, 1_002));
        assert_eq!(
            dispatch_decision_at(&gateway, &relay, 1_002)
                .unwrap_err()
                .status,
            503
        );
        let policy = gateway.providers[0].subscription_policy.as_mut().unwrap();
        policy.unknown_usage = SubscriptionUnknownUsage::Allow;
        assert!(dispatch_decision_at(&gateway, &relay, 1_002).is_ok());
        record_for_relay(&relay, reading(0.0, 1_003, 2_000));
        assert!(dispatch_decision_at(&gateway, &relay, 1_003).is_ok());
        gateway.providers[0]
            .subscription_policy
            .as_mut()
            .unwrap()
            .unknown_usage = SubscriptionUnknownUsage::Block;
        gateway.providers[0]
            .subscription_policy
            .as_mut()
            .unwrap()
            .window_used_percent_limits
            .insert("10080".to_string(), 90);
        assert_eq!(
            dispatch_decision_at(&gateway, &relay, 1_003)
                .unwrap_err()
                .status,
            503
        );
    }

    #[test]
    fn separate_sessions_for_one_account_share_readings_and_strictest_cap() {
        let first = subscription_relay("quota-shared-a", "quota-shared");
        let alias = subscription_relay("quota-shared-b", "quota-shared");
        let other_member = subscription_relay("quota-other-member", "quota-other-member");
        let mut gateway = GatewaySettings::default();
        for (relay, cap) in [(&first, 90), (&alias, 80), (&other_member, 90)] {
            add_subscription(
                &mut gateway,
                relay,
                Some(SubscriptionPolicy {
                    default_used_percent_limit: Some(cap),
                    unknown_usage: SubscriptionUnknownUsage::Block,
                    ..SubscriptionPolicy::default()
                }),
            );
        }
        record_for_relay(&first, reading(81.0, 1_000, 2_000));
        assert_eq!(quota_for_relay(&first), quota_for_relay(&alias));
        for relay in [&first, &alias] {
            assert_eq!(
                dispatch_decision_at(&gateway, relay, 1_000)
                    .unwrap_err()
                    .status,
                429
            );
        }
        assert_eq!(
            dispatch_decision_at(&gateway, &other_member, 1_000)
                .unwrap_err()
                .status,
            503
        );
    }

    #[test]
    fn next_permitted_rejects_api_candidates_after_a_dispatch_time_cap_change() {
        let relay = subscription_relay("quota-next", "quota-next");
        let mut gateway = GatewaySettings::default();
        add_subscription(
            &mut gateway,
            &relay,
            Some(SubscriptionPolicy {
                default_used_percent_limit: Some(80),
                on_limit: SubscriptionLimitAction::NextPermitted,
                ..SubscriptionPolicy::default()
            }),
        );
        let api = RelayProfile {
            id: "quota-api".to_string(),
            relay_mode: crate::settings::RelayMode::PureApi,
            base_url: "https://example.test/v1".to_string(),
            api_key: "test-key".to_string(),
            ..RelayProfile::default()
        };
        let now = now_ms();
        record_for_relay(&relay, reading(20.0, now, u64::MAX));
        let mut planning = DispatchGate::default();
        assert!(planning.check(&gateway, &relay).is_ok());
        assert!(planning.check(&gateway, &api).is_ok());
        record_for_relay(&relay, reading(80.0, now, u64::MAX));
        let mut dispatch = DispatchGate::default();
        assert_eq!(
            dispatch.check(&gateway, &relay).unwrap_err().action,
            SubscriptionLimitAction::NextPermitted
        );
        assert_eq!(
            dispatch.check(&GatewaySettings::default(), &api).unwrap_err().status,
            403
        );
    }

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
                    observed_at_ms: Some(7),
                }],
                updated_at_ms: 7,
                ..ProviderQuota::default()
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
            "已重置5h按满额排序，仍受未重置7d限制"
        );
        assert_eq!(remaining_at(&reading(100.0, 1_000, 2_000), 2_000), 1.0);
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
                "primary_window": { "used_percent": 1, "limit_window_seconds": 18000, "reset_at": u64::MAX },
                "secondary_window": { "used_percent": 1, "limit_window_seconds": 604800, "reset_after_seconds": u64::MAX }
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
                    ..QuotaWindow::default()
                }],
                updated_at_ms: 0,
                ..ProviderQuota::default()
            },
        );
        assert!(get("quota-forget").is_some());
        forget("quota-forget");
        assert!(get("quota-forget").is_none());
    }

    #[test]
    fn stale_observation_cannot_replace_newer_shared_reading() {
        let key = "quota-ordering";
        record(
            key,
            ProviderQuota {
                windows: vec![QuotaWindow {
                    window_minutes: Some(300),
                    used_percent: 82.0,
                    resets_at_ms: Some(u64::MAX),
                    ..QuotaWindow::default()
                }],
                updated_at_ms: 20,
                ..ProviderQuota::default()
            },
        );
        record(
            key,
            ProviderQuota {
                windows: vec![QuotaWindow {
                    window_minutes: Some(300),
                    used_percent: 12.0,
                    resets_at_ms: Some(u64::MAX),
                    ..QuotaWindow::default()
                }],
                updated_at_ms: 10,
                ..ProviderQuota::default()
            },
        );
        assert_eq!(get(key).unwrap().updated_at_ms, 20);
        forget(key);
    }

    #[test]
    fn subscription_limit_actions_are_ordered_from_stop_to_api_fallback() {
        assert!(action_rank(&SubscriptionLimitAction::Stop)
            < action_rank(&SubscriptionLimitAction::NextPermitted));
        assert!(action_rank(&SubscriptionLimitAction::NextPermitted)
            < action_rank(&SubscriptionLimitAction::ApiFallback));
    }

    fn weekly_usage(plan: &str, now: u64) -> ProviderQuota {
        parse_chatgpt_usage(&json!({
            "plan_type": plan,
            "rate_limit": {
                "allowed": true, "limit_reached": false,
                "primary_window": { "used_percent": 20, "limit_window_seconds": 604800, "reset_at": 2000000 },
                "secondary_window": null
            }
        }), now).unwrap()
    }

    #[test]
    fn actual_windows_apply_to_rare_plus_pro_and_team_without_a_five_hour_limit() {
        for plan in ["plus", "pro", "team"] {
            let relay = subscription_relay(&format!("quota-week-{plan}"), &format!("quota-week-{plan}"));
            let mut gateway = GatewaySettings::default();
            let mut policy = SubscriptionPolicy::default();
            policy.window_used_percent_limits.insert("300".into(), 0);
            policy.window_used_percent_limits.insert("10080".into(), 80);
            add_subscription(&mut gateway, &relay, Some(policy));
            record_for_relay(&relay, reading(100.0, 999, u64::MAX));
            record_for_relay(&relay, weekly_usage(plan, 1000));
            let quota = quota_for_relay(&relay).unwrap();
            assert_eq!(quota.windows.len(), 1, "完整新库存应移除旧5h");
            assert_eq!(quota.windows[0].window_minutes, Some(10080));
            assert!(dispatch_decision_at(&gateway, &relay, 1000).is_ok(), "{plan}");
            let mut exhausted = weekly_usage(plan, 1001);
            exhausted.windows[0].used_percent = 100.0;
            record_for_relay(&relay, exhausted);
            assert_eq!(dispatch_decision_at(&gateway, &relay, 1001).unwrap_err().status, 429);
        }
    }

    #[test]
    fn empty_complete_inventory_requires_official_allowance_and_credits_do_not_grant_it() {
        let relay = subscription_relay("quota-empty-flags", "quota-empty-flags");
        let mut gateway = GatewaySettings::default();
        add_subscription(&mut gateway, &relay, Some(SubscriptionPolicy {
            unknown_usage: SubscriptionUnknownUsage::Block, ..SubscriptionPolicy::default()
        }));
        for (time, flags, status) in [
            (1000, json!({"allowed": true, "limit_reached": false}), None),
            (1001, json!({"allowed": false, "limit_reached": true}), Some(429)),
            (1002, json!({}), Some(503)),
        ] {
            let mut limit = flags;
            limit["primary_window"] = Value::Null;
            limit["secondary_window"] = Value::Null;
            let quota = parse_chatgpt_usage(&json!({"rate_limit": limit, "credits": {"unlimited": true}}), time).unwrap();
            assert_eq!(quota.credits_available, Some(true));
            record_for_relay(&relay, quota);
            assert_eq!(dispatch_decision_at(&gateway, &relay, time).err().map(|error| error.status), status);
        }
        gateway.providers[0].subscription_policy.as_mut().unwrap().unknown_usage = SubscriptionUnknownUsage::Allow;
        let mut limited = weekly_usage("pro", 1003);
        limited.official_allowed = Some(false);
        limited.credits_available = Some(true);
        record_for_relay(&relay, limited);
        assert_eq!(dispatch_decision_at(&gateway, &relay, 1003).unwrap_err().status, 429);
    }

    #[test]
    fn partial_headers_preserve_other_windows_inventory_age_and_official_flags() {
        let relay = subscription_relay("quota-merge", "quota-merge");
        let alias = subscription_relay("quota-merge-alias", "quota-merge");
        let mut full = weekly_usage("plus", 1000);
        full.windows.push(reading(30.0, 1000, u64::MAX).windows.remove(0));
        full.official_allowed = Some(false);
        record_for_relay(&relay, full);
        record_for_relay(&alias, reading(50.0, 700000, u64::MAX));
        let quota = quota_for_relay(&relay).unwrap();
        assert!(quota.windows_complete);
        assert_eq!(quota.updated_at_ms, 1000);
        assert_eq!(quota.official_allowed, Some(false));
        assert_eq!(quota.windows[0].observed_at_ms, Some(700000));
        assert_eq!(quota.windows[1].observed_at_ms, Some(1000));
        assert!(!reading_fresh(&quota, 700000, 600000, true));

        // 较早启动的完整查询不能把期间更晚的头部读数回退。
        record_for_relay(&relay, weekly_usage("plus", 699999));
        let quota = quota_for_relay(&relay).unwrap();
        assert_eq!(quota.windows[0].used_percent, 50.0);
        assert_eq!(quota.windows[0].observed_at_ms, Some(700000));
        record_for_relay(&relay, weekly_usage("plus", 700001));
        assert_eq!(quota_for_relay(&alias).unwrap().windows.len(), 1);
    }

    #[test]
    fn fresh_exhausted_window_blocks_even_when_another_observation_is_stale() {
        let relay = subscription_relay("quota-mixed-age", "quota-mixed-age");
        let mut gateway = GatewaySettings::default();
        add_subscription(&mut gateway, &relay, Some(SubscriptionPolicy {
            unknown_usage: SubscriptionUnknownUsage::Allow, ..SubscriptionPolicy::default()
        }));
        record_for_relay(&relay, weekly_usage("plus", 1000));
        record_for_relay(&relay, reading(100.0, 700000, u64::MAX));
        assert_eq!(dispatch_decision_at(&gateway, &relay, 700000).unwrap_err().status, 429);
    }

    #[test]
    fn malformed_usage_cannot_claim_a_complete_empty_inventory() {
        for limit in [
            json!({}),
            json!({"primary_window": null}),
            json!({"primary_window": [], "secondary_window": null}),
            json!({"primary_window": {"used_percent": 12}, "secondary_window": null}),
            json!({"primary_window": {"used_percent": -1, "limit_window_seconds": 18000}, "secondary_window": null}),
            json!({"primary_window": null, "secondary_window": null, "allowed": "true"}),
        ] {
            assert!(parse_chatgpt_usage(&json!({"rate_limit": limit}), 1000).is_none());
        }
    }

    #[tokio::test]
    async fn refresh_deduplicates_aliases_and_reuses_usage_without_consumption_permission() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let a = subscription_relay("quota-prefetch-a", "quota-prefetch");
        let mut b = subscription_relay("quota-prefetch-b", "quota-prefetch");
        b.official_account_ref = a.id.clone();
        let _canonical = canonical_gateway_fixture(&a);
        assert!(crate::official_login::member_uses_official_login_transport(&b));
        let calls = AtomicUsize::new(0);
        let fetch = || async {
            calls.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(20)).await;
            let now = now_ms();
            let mut quota = weekly_usage("plus", now);
            quota.windows[0].resets_at_ms = Some(now + 3_600_000);
            Ok(Some(quota))
        };
        let (first, second) = tokio::join!(
            refresh_coordinated(&a, None, fetch),
            refresh_coordinated(&b, None, fetch)
        );
        assert!(first.is_ok() && second.is_ok());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(refresh_coordinated(&b, Some(600000), fetch).await.is_ok());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(quota_for_relay(&b).unwrap().windows_complete);
        let other = subscription_relay("quota-prefetch-other", "quota-prefetch-other");
        assert!(refresh_coordinated(&other, None, fetch).await.is_ok());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn failed_refresh_backs_off_without_refreshing_old_data_and_manual_retry_works() {
        let relay = subscription_relay("quota-refresh-failed", "quota-refresh-failed");
        record_for_relay(&relay, weekly_usage("plus", 1000));
        let failure = || async { Err(anyhow::anyhow!("synthetic failure")) };
        assert!(refresh_coordinated(&relay, Some(600000), failure).await.is_err());
        assert!(refresh_coordinated(&relay, Some(600000), || async { panic!("must back off") }).await.is_err());
        assert_eq!(quota_for_relay(&relay).unwrap().updated_at_ms, 1000);
        assert!(refresh_coordinated(&relay, None, || async {
            Ok(Some(weekly_usage("plus", now_ms())))
        }).await.is_ok());
    }

    #[tokio::test]
    async fn failure_of_one_login_does_not_back_off_an_independent_session_of_the_same_account() {
        let first = subscription_relay("quota-session-failed", "quota-session-owner");
        let second = subscription_relay("quota-session-ok", "quota-session-owner");
        assert!(refresh_coordinated(&first, Some(600000), || async {
            Err(anyhow::anyhow!("expired native session"))
        }).await.is_err());
        assert!(refresh_coordinated(&second, Some(600000), || async {
            Ok(Some(weekly_usage("plus", now_ms())))
        }).await.is_ok());
        assert_eq!(quota_for_relay(&first), quota_for_relay(&second));
    }

    #[test]
    fn preserved_native_backups_are_not_prefetch_or_lending_targets() {
        let relay = subscription_relay("native-backup-prefetch", "native-backup-owner");
        let mut gateway = GatewaySettings::default();
        add_subscription(&mut gateway, &relay, None);
        gateway.providers[0].source = "native:external".into();
        assert!(prefetch_targets(&gateway).is_empty());
        gateway.providers[0].source.clear();
        let mut auth: Value = serde_json::from_str(&gateway.providers[0].relay.auth_contents).unwrap();
        auth[crate::official_login::GATEWAY_OWNED_MARKER] = json!(true);
        auth[crate::official_login::NATIVE_OWNED_MARKER] = json!(true);
        gateway.providers[0].relay.auth_contents = auth.to_string();
        assert!(prefetch_targets(&gateway).is_empty());
        assert!(!crate::official_login::is_gateway_owned_login(&gateway.providers[0].relay));
    }

    #[tokio::test]
    async fn cancelled_refresh_releases_the_account_slot() {
        let relay = subscription_relay("quota-refresh-cancel", "quota-refresh-cancel");
        let notified = Arc::new(tokio::sync::Notify::new());
        let started = notified.clone();
        let task_relay = relay.clone();
        let task = tokio::spawn(async move {
            refresh_coordinated(&task_relay, None, || async {
                started.notify_one();
                std::future::pending::<anyhow::Result<Option<ProviderQuota>>>().await
            }).await
        });
        notified.notified().await;
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        tokio::time::timeout(Duration::from_secs(1), refresh_coordinated(&relay, None, || async {
            Ok(Some(weekly_usage("plus", now_ms())))
        })).await.unwrap().unwrap();
    }

    #[test]
    fn prefetch_uses_strictest_alias_freshness_regardless_of_order_or_permission() {
        let a = subscription_relay("quota-ttl-a", "quota-ttl");
        let mut b = subscription_relay("quota-ttl-b", "quota-ttl");
        b.official_account_ref = a.id.clone();
        let _canonical = canonical_gateway_fixture(&a);
        assert!(crate::official_login::member_uses_official_login_transport(&b));
        let mut gateway = GatewaySettings::default();
        for (relay, seconds) in [(&a, 600), (&b, 30)] {
            add_subscription(&mut gateway, relay, Some(SubscriptionPolicy {
                max_age_seconds: seconds, ..SubscriptionPolicy::default()
            }));
        }
        for _ in 0..2 {
            let targets = prefetch_targets(&gateway);
            assert_eq!(targets.len(), 1);
            assert_eq!(targets[0].1, 15000);
            gateway.providers.reverse();
        }
    }

    #[tokio::test]
    async fn successful_no_window_provider_refresh_also_has_an_automatic_ttl() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let relay = RelayProfile { id: "quota-none-ttl".into(), ..RelayProfile::default() };
        let calls = AtomicUsize::new(0);
        let fetch = || async { calls.fetch_add(1, Ordering::SeqCst); Ok(None) };
        refresh_coordinated(&relay, Some(600000), fetch).await.unwrap();
        refresh_coordinated(&relay, Some(600000), fetch).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        refresh_coordinated(&relay, None, fetch).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
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
