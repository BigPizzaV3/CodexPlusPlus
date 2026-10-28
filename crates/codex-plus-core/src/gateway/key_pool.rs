//! 网关的多 Key 轮换与冷却。
//!
//! 一个供应商配了多个 Key（`RelayProfile.api_keys`）时，网关把每个 Key 展开成一个候选：
//! 某个 Key 被限流（429）或额度耗尽就冷却它、换下一个；Key 失效（401）停用到 Key 本身被改掉；
//! 上游连续 5xx 时熔断整个供应商一小段时间。
//!
//! 状态只放内存：网关跑在 Codex++ 管理器进程里，重启后从头探测即可，不值得落盘。
//! 默认「会话粘性」：同一会话固定用一个 Key，出错才换，避免按请求轮换打散上游 prompt cache。

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use serde::Serialize;

use crate::gateway::store::KeyRotationMode;
use crate::settings::RelayProfile;

/// 限流没给 Retry-After 时的首个冷却时长，之后按连续次数翻倍。
const RATE_LIMIT_BASE_COOLDOWN: Duration = Duration::from_secs(30);
const RATE_LIMIT_MAX_COOLDOWN: Duration = Duration::from_secs(10 * 60);
/// 额度耗尽且上游没说何时恢复时的冷却时长。
const QUOTA_DEFAULT_COOLDOWN: Duration = Duration::from_secs(60 * 60);
/// 连续多少次 5xx/连接失败后熔断供应商，以及熔断时长。
const BREAKER_THRESHOLD: u32 = 3;
const BREAKER_OPEN: Duration = Duration::from_secs(60);
/// 粘性映射的上限，防止长时间运行后无界增长。
const STICKY_LIMIT: usize = 4096;

pub const PRIMARY_KEY_ID: &str = "primary";
pub const OFFICIAL_KEY_ID: &str = "official";

/// 一个候选 Key 的身份。`fingerprint` 是 Key 原文的哈希，Key 被用户改掉后旧的停用状态自动失效。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyRef {
    pub relay_id: String,
    pub key_id: String,
    pub fingerprint: u64,
}

/// 展开后的一个 Key 候选：`relay.api_key` 已换成这个 Key。
#[derive(Debug, Clone)]
pub struct KeyCandidate {
    pub relay: RelayProfile,
    pub key: KeyRef,
}

/// 上游失败的分类，决定是否换候选、是否冷却。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailureClass {
    /// 429 限流：冷却这个 Key。
    RateLimited { retry_after: Option<Duration> },
    /// 额度/余额耗尽：冷却更久。
    QuotaExhausted { reset_after: Option<Duration> },
    /// Key 无效：停用到 Key 被改掉。
    AuthInvalid,
    /// 5xx、超时、连接失败：计入供应商熔断。
    ServerError,
    /// 其它 4xx（403/404 等）：换下一个候选，但不冷却。
    Failover,
    /// 请求本身的问题（400/413/422）：换谁都一样，直接返回，避免把所有 Key 都烧一遍。
    ClientError,
}

impl FailureClass {
    pub fn should_failover(&self) -> bool {
        !matches!(self, FailureClass::ClientError)
    }

    pub fn label(&self) -> &'static str {
        match self {
            FailureClass::RateLimited { .. } => "rate_limited",
            FailureClass::QuotaExhausted { .. } => "quota_exhausted",
            FailureClass::AuthInvalid => "auth_invalid",
            FailureClass::ServerError => "server_error",
            FailureClass::Failover => "failover",
            FailureClass::ClientError => "client_error",
        }
    }
}

/// 按状态码、响应头和错误体给失败分类。
pub fn classify_failure(
    status: u16,
    headers: &reqwest::header::HeaderMap,
    body: &[u8],
) -> FailureClass {
    let text = String::from_utf8_lossy(&body[..body.len().min(16 * 1024)]).to_ascii_lowercase();
    let quota_hint = [
        "insufficient_quota",
        "usage_limit_reached",
        "subscription_sharing_usage_limit_exceeded",
        "exceeded your current quota",
        "quota exceeded",
        "quota_exceeded",
        "billing_hard_limit",
        "insufficient balance",
        "insufficient_balance",
        "credit balance is too low",
    ]
    .iter()
    .any(|hint| text.contains(hint));
    match status {
        402 => FailureClass::QuotaExhausted {
            reset_after: reset_after_from_body(&text).or_else(|| retry_after(headers)),
        },
        429 if quota_hint => FailureClass::QuotaExhausted {
            reset_after: reset_after_from_body(&text).or_else(|| retry_after(headers)),
        },
        429 => FailureClass::RateLimited {
            retry_after: retry_after(headers),
        },
        401 => FailureClass::AuthInvalid,
        403 if text.contains("invalid_api_key")
            || text.contains("invalid api key")
            || text.contains("incorrect api key") =>
        {
            FailureClass::AuthInvalid
        }
        403 if quota_hint => FailureClass::QuotaExhausted {
            reset_after: reset_after_from_body(&text),
        },
        400 | 413 | 422 => FailureClass::ClientError,
        408 | 500..=599 => FailureClass::ServerError,
        _ => FailureClass::Failover,
    }
}

/// `Retry-After`（秒数或 HTTP 日期）与 `retry-after-ms`。
fn retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    if let Some(ms) = headers
        .get("retry-after-ms")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value >= 0.0)
    {
        return Some(Duration::from_millis(ms as u64));
    }
    crate::protocol_proxy::exec_retry_after_duration(headers, SystemTime::now())
}

/// ChatGPT 订阅额度耗尽时错误体里带 `resets_in_seconds`。
fn reset_after_from_body(text: &str) -> Option<Duration> {
    let index = text.find("resets_in_seconds")?;
    let digits: String = text[index + "resets_in_seconds".len()..]
        .chars()
        .skip_while(|ch| !ch.is_ascii_digit())
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse::<u64>().ok().map(Duration::from_secs)
}

#[derive(Debug, Clone, Default)]
struct KeyHealth {
    cooldown_until: Option<Instant>,
    /// 停用时 Key 的指纹；指纹变化（用户改了 Key）即解除。
    disabled_fingerprint: Option<u64>,
    consecutive_rate_limits: u32,
    last_failure: Option<String>,
    last_failure_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Default)]
struct ProviderHealth {
    consecutive_errors: u32,
    open_until: Option<Instant>,
}

#[derive(Default)]
struct PoolState {
    keys: HashMap<(String, String), KeyHealth>,
    providers: HashMap<String, ProviderHealth>,
    /// (relay_id, 会话) → key_id
    sticky: HashMap<(String, String), String>,
    round_robin: HashMap<String, usize>,
}

fn state() -> &'static Mutex<PoolState> {
    static STATE: OnceLock<Mutex<PoolState>> = OnceLock::new();
    STATE.get_or_init(Mutex::default)
}

fn lock() -> std::sync::MutexGuard<'static, PoolState> {
    state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn fingerprint(value: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.trim().hash(&mut hasher);
    hasher.finish()
}

/// 供应商的全部 Key（还没按健康状态过滤）。
///
/// 两个及以上命名 Key 时逐个展开；否则只有 `relay.api_key` 一个。官方登录成员不走 Key。
pub fn expand_keys(relay: &RelayProfile) -> Vec<KeyCandidate> {
    if crate::official_login::member_uses_official_login_transport(relay) {
        return vec![KeyCandidate {
            key: KeyRef {
                relay_id: relay.id.clone(),
                key_id: OFFICIAL_KEY_ID.to_string(),
                fingerprint: 0,
            },
            relay: relay.clone(),
        }];
    }
    let named = relay
        .api_keys
        .iter()
        .filter(|entry| !entry.api_key.trim().is_empty())
        .collect::<Vec<_>>();
    if named.len() < 2 {
        return vec![KeyCandidate {
            key: KeyRef {
                relay_id: relay.id.clone(),
                key_id: PRIMARY_KEY_ID.to_string(),
                fingerprint: fingerprint(&relay.api_key),
            },
            relay: relay.clone(),
        }];
    }
    // 当前选中的 Key 排第一，其余保持用户书写顺序。
    let mut ordered = named;
    if let Some(position) = ordered
        .iter()
        .position(|entry| entry.id == relay.active_api_key_id)
    {
        let active = ordered.remove(position);
        ordered.insert(0, active);
    }
    ordered
        .into_iter()
        .map(|entry| {
            let mut relay = relay.clone();
            relay.api_key = entry.api_key.trim().to_string();
            KeyCandidate {
                key: KeyRef {
                    relay_id: relay.id.clone(),
                    key_id: entry.id.clone(),
                    fingerprint: fingerprint(&entry.api_key),
                },
                relay,
            }
        })
        .collect()
}

/// 一个供应商可用的 Key 候选，按策略排好序。
///
/// 返回 (可用候选, 最早恢复时间)。可用为空时第二项给出最近一个 Key 何时恢复，供调用方回 429。
pub fn order_keys(
    relay: &RelayProfile,
    mode: KeyRotationMode,
    conversation: Option<&str>,
    now: Instant,
) -> (Vec<KeyCandidate>, Option<Duration>) {
    order_keys_with(relay, mode, conversation, now, true)
}

/// 同 [`order_keys`]；`advance` 为 false 时只按当前轮询进度排、不往前推（预览用）。
pub fn order_keys_with(
    relay: &RelayProfile,
    mode: KeyRotationMode,
    conversation: Option<&str>,
    now: Instant,
    advance: bool,
) -> (Vec<KeyCandidate>, Option<Duration>) {
    let all = expand_keys(relay);
    let mut state = lock();
    if let Some(provider) = state.providers.get(&relay.id)
        && let Some(open_until) = provider.open_until
        && open_until > now
    {
        return (Vec::new(), Some(open_until - now));
    }
    let mut available = Vec::new();
    let mut soonest: Option<Duration> = None;
    for candidate in all {
        let health = state
            .keys
            .get(&(candidate.key.relay_id.clone(), candidate.key.key_id.clone()));
        if let Some(health) = health {
            if health.disabled_fingerprint == Some(candidate.key.fingerprint) {
                continue;
            }
            if let Some(until) = health.cooldown_until
                && until > now
            {
                let remaining = until - now;
                soonest = Some(soonest.map_or(remaining, |current| current.min(remaining)));
                continue;
            }
        }
        available.push(candidate);
    }
    if available.len() > 1 {
        let sticky = conversation
            .filter(|value| !value.trim().is_empty())
            .and_then(|conversation| {
                state
                    .sticky
                    .get(&(relay.id.clone(), conversation.to_string()))
                    .cloned()
            });
        if let Some(key_id) = sticky
            && let Some(position) = available
                .iter()
                .position(|candidate| candidate.key.key_id == key_id)
        {
            let pinned = available.remove(position);
            available.insert(0, pinned);
        } else if mode == KeyRotationMode::RoundRobin {
            let offset = if advance {
                let counter = state.round_robin.entry(relay.id.clone()).or_default();
                let current = *counter;
                *counter = counter.wrapping_add(1);
                current
            } else {
                state
                    .round_robin
                    .get(&relay.id)
                    .copied()
                    .unwrap_or_default()
            } % available.len();
            available.rotate_left(offset);
        }
    }
    (available, soonest)
}

/// 记一次失败。返回是否应该换下一个候选。
pub fn record_failure(key: &KeyRef, class: &FailureClass, now: Instant) -> bool {
    let mut state = lock();
    let entry = state
        .keys
        .entry((key.relay_id.clone(), key.key_id.clone()))
        .or_default();
    entry.last_failure = Some(class.label().to_string());
    entry.last_failure_at_ms = Some(now_unix_ms());
    match class {
        FailureClass::RateLimited { retry_after } => {
            entry.consecutive_rate_limits = entry.consecutive_rate_limits.saturating_add(1);
            let backoff = retry_after.unwrap_or_else(|| {
                let exponent = entry.consecutive_rate_limits.saturating_sub(1).min(10);
                RATE_LIMIT_BASE_COOLDOWN
                    .saturating_mul(1 << exponent)
                    .min(RATE_LIMIT_MAX_COOLDOWN)
            });
            // 上游明确给了 Retry-After 就照办；没给时按指数退避，上限 10 分钟。
            entry.cooldown_until = Some(now + backoff);
        }
        FailureClass::QuotaExhausted { reset_after } => {
            entry.cooldown_until = Some(now + reset_after.unwrap_or(QUOTA_DEFAULT_COOLDOWN));
        }
        FailureClass::AuthInvalid => {
            entry.disabled_fingerprint = Some(key.fingerprint);
        }
        FailureClass::ServerError => {
            let provider = state.providers.entry(key.relay_id.clone()).or_default();
            provider.consecutive_errors = provider.consecutive_errors.saturating_add(1);
            if provider.consecutive_errors >= BREAKER_THRESHOLD {
                provider.open_until = Some(now + BREAKER_OPEN);
                provider.consecutive_errors = 0;
            }
        }
        FailureClass::Failover | FailureClass::ClientError => {}
    }
    class.should_failover()
}

/// 记一次成功：清掉限流计数、关闭熔断，并把会话粘到这个 Key 上。
pub fn record_success(key: &KeyRef, conversation: Option<&str>) {
    let mut state = lock();
    if let Some(entry) = state
        .keys
        .get_mut(&(key.relay_id.clone(), key.key_id.clone()))
    {
        entry.consecutive_rate_limits = 0;
        entry.cooldown_until = None;
    }
    if let Some(provider) = state.providers.get_mut(&key.relay_id) {
        provider.consecutive_errors = 0;
        provider.open_until = None;
    }
    if let Some(conversation) = conversation.filter(|value| !value.trim().is_empty()) {
        if state.sticky.len() >= STICKY_LIMIT {
            state.sticky.clear();
        }
        state.sticky.insert(
            (key.relay_id.clone(), conversation.to_string()),
            key.key_id.clone(),
        );
    }
}

/// 清掉某个供应商的全部健康状态（UI 上的「重置」）。
pub fn reset(relay_id: &str) {
    let mut state = lock();
    state.keys.retain(|(relay, _), _| relay != relay_id);
    state.providers.remove(relay_id);
    state.sticky.retain(|(relay, _), _| relay != relay_id);
    state.round_robin.remove(relay_id);
}

/// UI 展示用的健康快照，不含 Key 原文。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyHealthView {
    pub relay_id: String,
    pub key_id: String,
    /// `ok` / `cooling` / `disabled`
    pub state: String,
    pub remaining_ms: Option<u64>,
    pub last_failure: Option<String>,
    pub last_failure_at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderHealthView {
    pub relay_id: String,
    pub breaker_open: bool,
    pub remaining_ms: Option<u64>,
    pub consecutive_errors: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PoolSnapshot {
    pub keys: Vec<KeyHealthView>,
    pub providers: Vec<ProviderHealthView>,
}

pub fn snapshot(now: Instant) -> PoolSnapshot {
    let state = lock();
    let mut keys = state
        .keys
        .iter()
        .map(|((relay_id, key_id), health)| {
            let remaining = health
                .cooldown_until
                .filter(|until| *until > now)
                .map(|until| (until - now).as_millis() as u64);
            KeyHealthView {
                relay_id: relay_id.clone(),
                key_id: key_id.clone(),
                state: if health.disabled_fingerprint.is_some() {
                    "disabled"
                } else if remaining.is_some() {
                    "cooling"
                } else {
                    "ok"
                }
                .to_string(),
                remaining_ms: remaining,
                last_failure: health.last_failure.clone(),
                last_failure_at_ms: health.last_failure_at_ms,
            }
        })
        .collect::<Vec<_>>();
    keys.sort_by(|left, right| {
        (left.relay_id.as_str(), left.key_id.as_str())
            .cmp(&(right.relay_id.as_str(), right.key_id.as_str()))
    });
    let mut providers = state
        .providers
        .iter()
        .map(|(relay_id, health)| {
            let remaining = health
                .open_until
                .filter(|until| *until > now)
                .map(|until| (until - now).as_millis() as u64);
            ProviderHealthView {
                relay_id: relay_id.clone(),
                breaker_open: remaining.is_some(),
                remaining_ms: remaining,
                consecutive_errors: health.consecutive_errors,
            }
        })
        .collect::<Vec<_>>();
    providers.sort_by(|left, right| left.relay_id.cmp(&right.relay_id));
    PoolSnapshot { keys, providers }
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{RelayApiKey, RelayMode};
    use reqwest::header::{HeaderMap, HeaderValue};

    fn relay(id: &str, keys: &[(&str, &str)]) -> RelayProfile {
        RelayProfile {
            id: id.to_string(),
            name: id.to_string(),
            relay_mode: RelayMode::PureApi,
            base_url: "https://example.test/v1".to_string(),
            api_key: keys
                .first()
                .map(|(_, key)| key.to_string())
                .unwrap_or_default(),
            api_keys: keys
                .iter()
                .map(|(key_id, key)| RelayApiKey {
                    id: key_id.to_string(),
                    name: key_id.to_string(),
                    api_key: key.to_string(),
                })
                .collect(),
            active_api_key_id: keys
                .first()
                .map(|(id, _)| id.to_string())
                .unwrap_or_default(),
            ..RelayProfile::default()
        }
    }

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(*name, HeaderValue::from_str(value).unwrap());
        }
        map
    }

    #[test]
    fn classify_failure_table() {
        let none = HeaderMap::new();
        let cases: Vec<(u16, HeaderMap, &str, FailureClass)> = vec![
            (
                429,
                headers(&[("retry-after", "12")]),
                "{}",
                FailureClass::RateLimited {
                    retry_after: Some(Duration::from_secs(12)),
                },
            ),
            (
                429,
                headers(&[("retry-after-ms", "1500")]),
                "{}",
                FailureClass::RateLimited {
                    retry_after: Some(Duration::from_millis(1500)),
                },
            ),
            (
                429,
                none.clone(),
                "{}",
                FailureClass::RateLimited { retry_after: None },
            ),
            (
                429,
                none.clone(),
                r#"{"error":{"type":"insufficient_quota"}}"#,
                FailureClass::QuotaExhausted { reset_after: None },
            ),
            (
                429,
                none.clone(),
                r#"{"error":{"type":"usage_limit_reached","resets_in_seconds":3600}}"#,
                FailureClass::QuotaExhausted {
                    reset_after: Some(Duration::from_secs(3600)),
                },
            ),
            (
                402,
                none.clone(),
                "{}",
                FailureClass::QuotaExhausted { reset_after: None },
            ),
            (401, none.clone(), "{}", FailureClass::AuthInvalid),
            (
                403,
                none.clone(),
                r#"{"error":{"code":"invalid_api_key"}}"#,
                FailureClass::AuthInvalid,
            ),
            (
                403,
                none.clone(),
                r#"{"error":"forbidden region"}"#,
                FailureClass::Failover,
            ),
            (404, none.clone(), "{}", FailureClass::Failover),
            (
                400,
                none.clone(),
                "context_length_exceeded",
                FailureClass::ClientError,
            ),
            (413, none.clone(), "{}", FailureClass::ClientError),
            (422, none.clone(), "{}", FailureClass::ClientError),
            (500, none.clone(), "{}", FailureClass::ServerError),
            (529, none.clone(), "{}", FailureClass::ServerError),
            (408, none.clone(), "{}", FailureClass::ServerError),
        ];
        for (status, headers, body, expected) in cases {
            assert_eq!(
                classify_failure(status, &headers, body.as_bytes()),
                expected,
                "status {status} body {body}"
            );
        }
    }

    #[test]
    fn single_key_relay_expands_to_primary() {
        let relay = relay("kp-single", &[("a", "sk-a")]);
        let keys = expand_keys(&relay);
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].key.key_id, PRIMARY_KEY_ID);
        assert_eq!(keys[0].relay.api_key, "sk-a");
    }

    #[test]
    fn active_key_goes_first_and_each_candidate_carries_its_key() {
        let mut relay = relay("kp-order", &[("a", "sk-a"), ("b", "sk-b"), ("c", "sk-c")]);
        relay.active_api_key_id = "b".to_string();
        let keys = expand_keys(&relay);
        let ids = keys
            .iter()
            .map(|key| key.key.key_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["b", "a", "c"]);
        assert_eq!(keys[1].relay.api_key, "sk-a");
    }

    #[test]
    fn rate_limited_key_cools_down_and_recovers() {
        let relay = relay("kp-cool", &[("a", "sk-a"), ("b", "sk-b")]);
        let now = Instant::now();
        let (keys, _) = order_keys(&relay, KeyRotationMode::Sticky, None, now);
        assert_eq!(keys[0].key.key_id, "a");
        assert!(record_failure(
            &keys[0].key,
            &FailureClass::RateLimited {
                retry_after: Some(Duration::from_secs(20))
            },
            now
        ));
        let (keys, _) = order_keys(&relay, KeyRotationMode::Sticky, None, now);
        assert_eq!(
            keys.iter()
                .map(|key| key.key.key_id.as_str())
                .collect::<Vec<_>>(),
            vec!["b"]
        );
        let later = now + Duration::from_secs(21);
        let (keys, _) = order_keys(&relay, KeyRotationMode::Sticky, None, later);
        assert_eq!(keys.len(), 2);
    }

    #[test]
    fn all_keys_cooling_reports_soonest_recovery() {
        let relay = relay("kp-all", &[("a", "sk-a"), ("b", "sk-b")]);
        let now = Instant::now();
        for (key_id, seconds) in [("a", 50), ("b", 10)] {
            let key = expand_keys(&relay)
                .into_iter()
                .find(|key| key.key.key_id == key_id)
                .unwrap()
                .key;
            record_failure(
                &key,
                &FailureClass::RateLimited {
                    retry_after: Some(Duration::from_secs(seconds)),
                },
                now,
            );
        }
        let (keys, soonest) = order_keys(&relay, KeyRotationMode::Sticky, None, now);
        assert!(keys.is_empty());
        assert_eq!(soonest, Some(Duration::from_secs(10)));
    }

    #[test]
    fn auth_invalid_disables_until_key_changes() {
        let mut relay = relay("kp-auth", &[("a", "sk-a"), ("b", "sk-b")]);
        let now = Instant::now();
        let key_a = expand_keys(&relay)[0].key.clone();
        record_failure(&key_a, &FailureClass::AuthInvalid, now);
        let (keys, _) = order_keys(&relay, KeyRotationMode::Sticky, None, now);
        assert_eq!(keys.len(), 1);
        relay.api_keys[0].api_key = "sk-a-new".to_string();
        let (keys, _) = order_keys(&relay, KeyRotationMode::Sticky, None, now);
        assert_eq!(keys.len(), 2);
    }

    #[test]
    fn sticky_conversation_keeps_its_key() {
        let relay = relay("kp-sticky", &[("a", "sk-a"), ("b", "sk-b")]);
        let now = Instant::now();
        let key_b = expand_keys(&relay)[1].key.clone();
        record_success(&key_b, Some("conv-1"));
        let (keys, _) = order_keys(&relay, KeyRotationMode::Sticky, Some("conv-1"), now);
        assert_eq!(keys[0].key.key_id, "b");
        let (keys, _) = order_keys(&relay, KeyRotationMode::Sticky, Some("conv-2"), now);
        assert_eq!(keys[0].key.key_id, "a");
    }

    #[test]
    fn round_robin_rotates_per_request() {
        let relay = relay("kp-rr", &[("a", "sk-a"), ("b", "sk-b")]);
        let now = Instant::now();
        let first = order_keys(&relay, KeyRotationMode::RoundRobin, None, now).0[0]
            .key
            .key_id
            .clone();
        let second = order_keys(&relay, KeyRotationMode::RoundRobin, None, now).0[0]
            .key
            .key_id
            .clone();
        assert_ne!(first, second);
    }

    #[test]
    fn preview_ordering_does_not_advance_round_robin() {
        let relay = relay("kp-rr-preview", &[("a", "sk-a"), ("b", "sk-b")]);
        let now = Instant::now();
        let first = |advance| {
            order_keys_with(&relay, KeyRotationMode::RoundRobin, None, now, advance).0[0]
                .key
                .key_id
                .clone()
        };
        let preview = first(false);
        assert_eq!(first(false), preview, "预览不推进轮询");
        assert_eq!(first(true), preview, "预览看到的就是下一个真实请求的 Key");
        assert_ne!(first(true), preview);
    }

    #[test]
    fn repeated_server_errors_open_the_breaker() {
        let relay = relay("kp-breaker", &[("a", "sk-a")]);
        let now = Instant::now();
        let key = expand_keys(&relay)[0].key.clone();
        for _ in 0..BREAKER_THRESHOLD {
            record_failure(&key, &FailureClass::ServerError, now);
        }
        let (keys, soonest) = order_keys(&relay, KeyRotationMode::Sticky, None, now);
        assert!(keys.is_empty());
        assert_eq!(soonest, Some(BREAKER_OPEN));
        record_success(&key, None);
        let (keys, _) = order_keys(&relay, KeyRotationMode::Sticky, None, now);
        assert_eq!(keys.len(), 1);
    }

    #[test]
    fn client_errors_do_not_failover_or_cool_down() {
        let relay = relay("kp-client", &[("a", "sk-a"), ("b", "sk-b")]);
        let now = Instant::now();
        let key = expand_keys(&relay)[0].key.clone();
        assert!(!record_failure(&key, &FailureClass::ClientError, now));
        let (keys, _) = order_keys(&relay, KeyRotationMode::Sticky, None, now);
        assert_eq!(keys.len(), 2);
    }
}
