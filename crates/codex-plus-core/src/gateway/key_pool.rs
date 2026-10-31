//! 网关的多 Key 轮换与冷却。
//!
//! 一个供应商配了多个 Key（`RelayProfile.api_keys`）时，网关把每个 Key 展开成一个候选：
//! 某个 Key 被限流（429）或额度耗尽就冷却它、换下一个；Key 失效（401）停用到 Key 本身被改掉；
//! 上游连续 5xx 时熔断整个供应商一小段时间。用户还可以按供应商给每个 Key 设 RPM / 并发上限
//! （主动限速，见文件后半部分），满了的 Key 先跳过、都满了排队，不等 429 再冷却。
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

/// 限流没给任何恢复时间（Retry-After / x-ratelimit-reset*）时的首个冷却时长，之后按连续次数翻倍。
///
/// 原来 30 秒起步（#2802）：只有一个 Key 时，一次 429 就让所有会话被网关本地拒绝半分钟，
/// 而 Codex 自己的重试几秒内就用完了，比直连还糟。偶发的 429 多半几秒就恢复，先短冷却、连续再翻倍。
const RATE_LIMIT_BASE_COOLDOWN: Duration = Duration::from_secs(5);
const RATE_LIMIT_MAX_COOLDOWN: Duration = Duration::from_secs(10 * 60);
/// 额度耗尽且上游没说何时恢复时的冷却时长。
const QUOTA_DEFAULT_COOLDOWN: Duration = Duration::from_secs(60 * 60);
/// 官方登录的 401：凭据由 Codex 的登录维护、指纹不会变，不能永久停用，冷却一会儿再试。
const OFFICIAL_AUTH_COOLDOWN: Duration = Duration::from_secs(60);
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
            retry_after: retry_after(headers).or_else(|| rate_limit_reset(headers)),
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

/// OpenAI 风格的限流重置头（#2802）：`x-ratelimit-reset-requests` / `x-ratelimit-reset-tokens`
/// （`1s`、`6m0s`、`250ms` 这类写法），以及不少中转站用的 `x-ratelimit-reset`（秒数或 Unix 时间戳）。
///
/// 对应的 `x-ratelimit-remaining-*` 为 0 的那一项才是这次超限的原因，取它们里最晚的；
/// 看不出是哪一项时取最早的，宁可早点再试也不多冷却。上限与指数退避的上限一致。
fn rate_limit_reset(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .filter(|value| !value.is_empty())
    };
    let mut exhausted: Vec<Duration> = Vec::new();
    let mut any: Vec<Duration> = Vec::new();
    for kind in ["requests", "tokens"] {
        let Some(reset) = header(&format!("x-ratelimit-reset-{kind}")).and_then(parse_reset_value)
        else {
            continue;
        };
        any.push(reset);
        if header(&format!("x-ratelimit-remaining-{kind}"))
            .and_then(|value| value.parse::<f64>().ok())
            .is_some_and(|remaining| remaining <= 0.0)
        {
            exhausted.push(reset);
        }
    }
    if let Some(reset) = header("x-ratelimit-reset").and_then(parse_reset_value) {
        any.push(reset);
    }
    exhausted
        .into_iter()
        .max()
        .or_else(|| any.into_iter().min())
        .map(|reset| reset.min(RATE_LIMIT_MAX_COOLDOWN))
}

/// 重置时间的几种写法：`1h2m3.5s` / `250ms` 这类时长、纯秒数，或 Unix 时间戳（秒或毫秒）。
fn parse_reset_value(value: &str) -> Option<Duration> {
    if let Ok(number) = value.parse::<f64>() {
        if !number.is_finite() || number < 0.0 {
            return None;
        }
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .ok()?
            .as_secs_f64();
        let seconds = if number >= 1e12 {
            // 毫秒时间戳。
            (number / 1000.0 - now).max(0.0)
        } else if number >= 1e9 {
            // 秒时间戳。
            (number - now).max(0.0)
        } else {
            number
        };
        return Some(Duration::from_secs_f64(seconds));
    }
    let mut total = 0.0_f64;
    let mut rest = value;
    let mut matched = false;
    while !rest.is_empty() {
        let digits = rest
            .find(|ch: char| !(ch.is_ascii_digit() || ch == '.'))
            .unwrap_or(rest.len());
        if digits == 0 {
            return None;
        }
        let amount = rest[..digits].parse::<f64>().ok()?;
        rest = &rest[digits..];
        let (unit, len) = if rest.starts_with("ms") {
            (0.001, 2)
        } else if rest.starts_with('h') {
            (3600.0, 1)
        } else if rest.starts_with('m') {
            (60.0, 1)
        } else if rest.starts_with('s') {
            (1.0, 1)
        } else {
            return None;
        };
        rest = &rest[len..];
        total += amount * unit;
        matched = true;
    }
    (matched && total.is_finite()).then(|| Duration::from_secs_f64(total))
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

/// 单 Key 供应商的指纹。账号类供应商（ChatGPT / Copilot / SuperGrok 登录）落盘时不存 `api_key`，
/// token 在 `auth_contents` 里：一起算进去，刷新或重新登录后指纹变了，401 的停用才会解除。
fn primary_fingerprint(relay: &RelayProfile) -> u64 {
    if relay.auth_contents.trim().is_empty() {
        fingerprint(&relay.api_key)
    } else {
        fingerprint(&format!("{}\n{}", relay.api_key.trim(), relay.auth_contents.trim()))
    }
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
                fingerprint: primary_fingerprint(relay),
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
    let order = order_keys_detailed(relay, mode, conversation, now, advance);
    (order.available, order.soonest)
}

/// 供应商是否正处于熔断中（只读，诊断与测试用）。
pub fn breaker_open(relay_id: &str, now: Instant) -> bool {
    lock()
        .providers
        .get(relay_id)
        .and_then(|provider| provider.open_until)
        .is_some_and(|open_until| open_until > now)
}

/// 暂时不可用的 Key 为什么不可用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitReason {
    /// 这个 Key 自己在冷却（429 / 额度耗尽 / 官方登录 401）。
    Cooling,
    /// Key 本身没事，是整个供应商在熔断（连续 5xx / 连接失败）。
    Breaker,
}

/// 冷却或熔断中的一个 Key：多久后恢复、为什么。停用（401）的 Key 不在这里。
#[derive(Debug, Clone)]
pub struct WaitingKey {
    pub candidate: KeyCandidate,
    pub remaining: Duration,
    pub reason: WaitReason,
}

/// [`order_keys_detailed`] 的结果。
#[derive(Debug, Clone, Default)]
pub struct KeyOrder {
    /// 现在可用的 Key，按策略排好序。
    pub available: Vec<KeyCandidate>,
    /// 冷却 / 熔断中的 Key 里最早多久后恢复。
    pub soonest: Option<Duration>,
    /// 冷却 / 熔断中的 Key（按书写顺序），供候选全空时决定是等一等还是放行（#2802）。
    pub waiting: Vec<WaitingKey>,
}

/// 同 [`order_keys_with`]，另外交回冷却 / 熔断中的 Key 及其原因。
pub fn order_keys_detailed(
    relay: &RelayProfile,
    mode: KeyRotationMode,
    conversation: Option<&str>,
    now: Instant,
    advance: bool,
) -> KeyOrder {
    let all = expand_keys(relay);
    let mut state = lock();
    let breaker = state
        .providers
        .get(&relay.id)
        .and_then(|provider| provider.open_until)
        .filter(|open_until| *open_until > now)
        .map(|open_until| open_until - now);
    let mut available = Vec::new();
    let mut waiting = Vec::new();
    let mut soonest: Option<Duration> = breaker;
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
                if breaker.is_none() {
                    soonest = Some(soonest.map_or(remaining, |current| current.min(remaining)));
                }
                waiting.push(WaitingKey {
                    candidate,
                    remaining,
                    reason: WaitReason::Cooling,
                });
                continue;
            }
        }
        if let Some(remaining) = breaker {
            waiting.push(WaitingKey {
                candidate,
                remaining,
                reason: WaitReason::Breaker,
            });
            continue;
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
    KeyOrder {
        available,
        soonest,
        waiting,
    }
}

/// 这个 Key 自己还要冷却多久；没在冷却返回 None（供应商熔断不算，见 [`WaitReason::Breaker`]）。
pub fn cooling_remaining(key: &KeyRef, now: Instant) -> Option<Duration> {
    lock()
        .keys
        .get(&(key.relay_id.clone(), key.key_id.clone()))
        .and_then(|health| health.cooldown_until)
        .filter(|until| *until > now)
        .map(|until| until - now)
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
        FailureClass::AuthInvalid if key.key_id == OFFICIAL_KEY_ID => {
            entry.cooldown_until = Some(now + OFFICIAL_AUTH_COOLDOWN);
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
        // 能成功就说明凭据是好的（例如刚刷新过）。
        entry.disabled_fingerprint = None;
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

/// 某个 Key 被删除或换成新值（#2782）：清掉它的冷却 / 停用记录、限速计数，以及固定到它的会话。
///
/// 会话固定指向已删的 Key 时，下次请求按策略重新选 Key；进行中的请求名额释放时找不到计数，自然无事。
/// 供应商级的熔断不动：那是上游地址的问题，与哪个 Key 无关。
pub fn forget_key(relay_id: &str, key_id: &str) {
    let mut state = lock();
    state.keys.remove(&(relay_id.to_string(), key_id.to_string()));
    state
        .sticky
        .retain(|(relay, _), pinned| !(relay == relay_id && pinned == key_id));
    drop(state);
    usage_lock().remove(&(relay_id.to_string(), key_id.to_string()));
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

// ---------------------------------------------------------------------------
// 主动限速（#1130 / #2726）
//
// 用户按供应商填「每个 Key 每分钟最多请求数」「每个 Key 同时进行的最多请求数」，网关发请求前先占名额：
// 名额满了的 Key 像冷却中一样先跳过（换同一家的下一个 Key 或下一家），但不算失败、不冷却、不熔断，
// 也不改会话固定；所有候选都满了由候选循环排队等最早空出的名额（见 `pipeline`）。
// 计数和冷却一样只放内存：按 (供应商 id, Key id) 记最近 60 秒发出的时间和进行中的请求数。
// 没填上限的 Key 完全不进这张表，行为和以前一样。
// ---------------------------------------------------------------------------

/// 所有候选都因限速满了时，最多排队等这么久；超时返回本地限速错误。
pub const LIMIT_QUEUE_TIMEOUT: Duration = Duration::from_secs(120);
/// RPM 的滑动窗口。
const RPM_WINDOW: Duration = Duration::from_secs(60);

/// 一个 Key 的主动限速上限；都为 None 表示不限。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KeyLimits {
    /// 每分钟最多发出的请求数（滑动 60 秒）。
    pub rpm: Option<u32>,
    /// 同时进行中的最多请求数。
    pub concurrency: Option<u32>,
}

impl KeyLimits {
    pub fn is_unlimited(&self) -> bool {
        self.rpm.is_none() && self.concurrency.is_none()
    }
}

/// 某个 Key 现在能不能再发一个请求。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyLimitState {
    Free,
    /// 满了。`retry_in` 是按 RPM 窗口算出的最早空出时间；只因并发满了时为 None（要等进行中的请求结束）。
    Full {
        retry_in: Option<Duration>,
    },
}

#[derive(Debug, Default)]
struct KeyUsage {
    /// 最近 60 秒内发出请求的时间，按先后排。
    sent: std::collections::VecDeque<Instant>,
    in_flight: u32,
}

impl KeyUsage {
    fn prune(&mut self, now: Instant) {
        while self
            .sent
            .front()
            .is_some_and(|at| now.saturating_duration_since(*at) >= RPM_WINDOW)
        {
            self.sent.pop_front();
        }
    }

    fn state(&self, limits: KeyLimits, now: Instant) -> KeyLimitState {
        let rpm_full = limits
            .rpm
            .is_some_and(|rpm| self.sent.len() >= rpm as usize);
        let busy = limits
            .concurrency
            .is_some_and(|concurrency| self.in_flight >= concurrency);
        if !rpm_full && !busy {
            return KeyLimitState::Free;
        }
        // 窗口里第 (len - rpm + 1) 早的那次滑出窗口后才有名额。
        let retry_in = rpm_full
            .then(|| {
                let rpm = limits.rpm.unwrap_or_default() as usize;
                self.sent
                    .len()
                    .checked_sub(rpm.max(1))
                    .and_then(|index| self.sent.get(index))
            })
            .flatten()
            .map(|at| (*at + RPM_WINDOW).saturating_duration_since(now));
        KeyLimitState::Full { retry_in }
    }
}

type UsageTable = HashMap<(String, String), KeyUsage>;

fn usage_lock() -> std::sync::MutexGuard<'static, UsageTable> {
    static USAGE: OnceLock<Mutex<UsageTable>> = OnceLock::new();
    USAGE
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn limit_notify() -> &'static tokio::sync::Notify {
    static NOTIFY: OnceLock<tokio::sync::Notify> = OnceLock::new();
    NOTIFY.get_or_init(tokio::sync::Notify::new)
}

/// 有名额被释放时会被唤醒的 future。排队方要先 `enable` 再检查名额，避免漏掉检查和等待之间的释放。
pub fn limit_released() -> tokio::sync::futures::Notified<'static> {
    limit_notify().notified()
}

/// 这个 Key 按上限看现在有没有名额（不占名额）。
pub fn limit_state(key: &KeyRef, limits: KeyLimits, now: Instant) -> KeyLimitState {
    if limits.is_unlimited() {
        return KeyLimitState::Free;
    }
    let mut table = usage_lock();
    match table.get_mut(&(key.relay_id.clone(), key.key_id.clone())) {
        Some(usage) => {
            usage.prune(now);
            usage.state(limits, now)
        }
        None => KeyLimitState::Free,
    }
}

/// 占一个名额：有名额时记一次发出（计入 RPM）并把进行中加一，返回的 [`KeySlot`] drop 时释放并发名额。
/// 满了返回 None。没设上限时不记任何东西，直接给一个空名额。
pub fn try_acquire(key: &KeyRef, limits: KeyLimits, now: Instant) -> Option<KeySlot> {
    if limits.is_unlimited() {
        return Some(KeySlot { held: None });
    }
    let id = (key.relay_id.clone(), key.key_id.clone());
    let mut table = usage_lock();
    let usage = table.entry(id.clone()).or_default();
    usage.prune(now);
    if usage.state(limits, now) != KeyLimitState::Free {
        return None;
    }
    usage.sent.push_back(now);
    usage.in_flight = usage.in_flight.saturating_add(1);
    Some(KeySlot {
        held: Some((id, now)),
    })
}

/// 占着的一个名额。drop 时（响应转发完、出错、客户端断开）释放并发名额并唤醒排队的请求；
/// RPM 的那一次留在窗口里，60 秒后自然滑出。
#[derive(Debug)]
#[must_use = "名额在 drop 时释放，要持有到请求真正结束"]
pub struct KeySlot {
    held: Option<((String, String), Instant)>,
}

impl KeySlot {
    /// 请求其实没发出去（例如发之前换 token 失败）：连 RPM 的这一次也退掉。
    pub fn cancel(mut self) {
        if let Some((id, at)) = self.held.take() {
            let mut table = usage_lock();
            if let Some(usage) = table.get_mut(&id) {
                if let Some(position) = usage.sent.iter().rposition(|sent| *sent == at) {
                    usage.sent.remove(position);
                }
                usage.in_flight = usage.in_flight.saturating_sub(1);
            }
            drop(table);
            limit_notify().notify_waiters();
        }
    }
}

impl Drop for KeySlot {
    fn drop(&mut self) {
        if let Some((id, _)) = self.held.take() {
            let mut table = usage_lock();
            if let Some(usage) = table.get_mut(&id) {
                usage.in_flight = usage.in_flight.saturating_sub(1);
            }
            drop(table);
            limit_notify().notify_waiters();
        }
    }
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

    /// 账号类供应商不存 `api_key`：401 后刷新或重新登录（auth_contents 变了）就恢复，成功一次也恢复。
    #[test]
    fn account_provider_recovers_after_token_changes() {
        let mut relay = relay("kp-account", &[]);
        relay.api_key = String::new();
        relay.auth_contents = r#"{"access_token":"old"}"#.to_string();
        let now = Instant::now();
        let key = expand_keys(&relay)[0].key.clone();
        record_failure(&key, &FailureClass::AuthInvalid, now);
        assert!(order_keys(&relay, KeyRotationMode::Sticky, None, now).0.is_empty());
        relay.auth_contents = r#"{"access_token":"new"}"#.to_string();
        assert_eq!(order_keys(&relay, KeyRotationMode::Sticky, None, now).0.len(), 1);

        relay.auth_contents = r#"{"access_token":"old"}"#.to_string();
        record_success(&key, None);
        assert_eq!(order_keys(&relay, KeyRotationMode::Sticky, None, now).0.len(), 1, "成功一次就解除");
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

    /// 429 没给 Retry-After 时看限流重置头（#2802）：超限的那一项（remaining 为 0）说了算。
    #[test]
    fn rate_limit_reset_headers_set_the_cooldown() {
        let class = |pairs: &[(&'static str, &str)]| classify_failure(429, &headers(pairs), b"{}");
        assert_eq!(
            class(&[
                ("x-ratelimit-remaining-requests", "0"),
                ("x-ratelimit-reset-requests", "1m2.5s"),
                ("x-ratelimit-remaining-tokens", "5000"),
                ("x-ratelimit-reset-tokens", "250ms"),
            ]),
            FailureClass::RateLimited {
                retry_after: Some(Duration::from_millis(62_500))
            }
        );
        assert_eq!(
            class(&[
                ("x-ratelimit-reset-requests", "6s"),
                ("x-ratelimit-reset-tokens", "250ms"),
            ]),
            FailureClass::RateLimited {
                retry_after: Some(Duration::from_millis(250))
            },
            "看不出是哪一项超限时取最早的"
        );
        assert_eq!(
            class(&[("x-ratelimit-reset", "3")]),
            FailureClass::RateLimited {
                retry_after: Some(Duration::from_secs(3))
            }
        );
        assert_eq!(
            class(&[("retry-after", "7"), ("x-ratelimit-reset-requests", "1s")]),
            FailureClass::RateLimited {
                retry_after: Some(Duration::from_secs(7))
            },
            "Retry-After 优先"
        );
        assert_eq!(
            class(&[("x-ratelimit-reset-requests", "5h")]),
            FailureClass::RateLimited {
                retry_after: Some(RATE_LIMIT_MAX_COOLDOWN)
            },
            "重置头给的时间同样封顶"
        );
        assert_eq!(
            class(&[("x-ratelimit-reset-requests", "soon")]),
            FailureClass::RateLimited { retry_after: None },
            "看不懂的写法当没给"
        );
        let epoch = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 30;
        let FailureClass::RateLimited {
            retry_after: Some(wait),
        } = class(&[("x-ratelimit-reset", &epoch.to_string())])
        else {
            panic!("Unix 时间戳也认");
        };
        assert!(
            (Duration::from_secs(28)..=Duration::from_secs(30)).contains(&wait),
            "{wait:?}"
        );
        // 额度耗尽不看这些头：它们说的是限流窗口，不是额度何时恢复。
        assert_eq!(
            classify_failure(
                429,
                &headers(&[("x-ratelimit-reset-requests", "1s")]),
                br#"{"error":{"code":"insufficient_quota"}}"#
            ),
            FailureClass::QuotaExhausted { reset_after: None }
        );
    }

    /// 无提示的 429：第一次只冷却 5 秒，连续再翻倍（#2802，原来 30 秒起步）。
    #[test]
    fn first_rate_limit_without_hint_cools_briefly() {
        let relay = relay("kp-short-cool", &[("a", "sk-a")]);
        let now = Instant::now();
        let key = expand_keys(&relay)[0].key.clone();
        let limited = FailureClass::RateLimited { retry_after: None };
        record_failure(&key, &limited, now);
        assert_eq!(cooling_remaining(&key, now), Some(Duration::from_secs(5)));
        record_failure(&key, &limited, now);
        assert_eq!(cooling_remaining(&key, now), Some(Duration::from_secs(10)));
        record_success(&key, None);
        assert_eq!(cooling_remaining(&key, now), None);
        record_failure(&key, &limited, now);
        assert_eq!(
            cooling_remaining(&key, now),
            Some(Duration::from_secs(5)),
            "成功一次后从头算"
        );
    }

    /// 熔断中的 Key 交回时标明是熔断，自己在冷却的标明是冷却（候选全空时据此决定放行还是等）。
    #[test]
    fn waiting_keys_carry_the_reason() {
        let relay = relay("kp-waiting", &[("a", "sk-a"), ("b", "sk-b")]);
        let now = Instant::now();
        let keys = expand_keys(&relay);
        record_failure(
            &keys[0].key,
            &FailureClass::RateLimited {
                retry_after: Some(Duration::from_secs(8)),
            },
            now,
        );
        let order = order_keys_detailed(&relay, KeyRotationMode::Sticky, None, now, false);
        assert_eq!(order.available.len(), 1);
        assert_eq!(order.waiting.len(), 1);
        assert_eq!(order.waiting[0].reason, WaitReason::Cooling);
        assert_eq!(order.waiting[0].remaining, Duration::from_secs(8));
        for _ in 0..BREAKER_THRESHOLD {
            record_failure(&keys[1].key, &FailureClass::ServerError, now);
        }
        let order = order_keys_detailed(&relay, KeyRotationMode::Sticky, None, now, false);
        assert!(order.available.is_empty());
        assert_eq!(order.soonest, Some(BREAKER_OPEN));
        let reasons = order
            .waiting
            .iter()
            .map(|item| (item.candidate.key.key_id.as_str(), item.reason))
            .collect::<Vec<_>>();
        assert_eq!(
            reasons,
            vec![("a", WaitReason::Cooling), ("b", WaitReason::Breaker)]
        );
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

    fn limits(rpm: Option<u32>, concurrency: Option<u32>) -> KeyLimits {
        KeyLimits { rpm, concurrency }
    }

    /// 选出第一个有名额的 Key（候选循环里跳过满了的 Key 就是这样）。
    fn first_free(relay: &RelayProfile, limits: KeyLimits, now: Instant) -> Option<String> {
        order_keys(relay, KeyRotationMode::Sticky, None, now)
            .0
            .into_iter()
            .find(|key| limit_state(&key.key, limits, now) == KeyLimitState::Free)
            .map(|key| key.key.key_id)
    }

    #[test]
    fn rpm_full_key_is_skipped_for_the_next_key() {
        let relay = relay("kp-rpm", &[("a", "sk-a"), ("b", "sk-b")]);
        let limits = limits(Some(2), None);
        let now = Instant::now();
        let key_a = expand_keys(&relay)[0].key.clone();
        for _ in 0..2 {
            drop(try_acquire(&key_a, limits, now).expect("有名额"));
        }
        assert!(
            try_acquire(&key_a, limits, now).is_none(),
            "一分钟内第 3 次超出上限"
        );
        assert_eq!(first_free(&relay, limits, now).as_deref(), Some("b"));
        assert_eq!(
            limit_state(&key_a, limits, now + Duration::from_secs(10)),
            KeyLimitState::Full {
                retry_in: Some(Duration::from_secs(50))
            },
            "最早那次滑出 60 秒窗口才有名额"
        );
        // 滑动窗口：60 秒后又有名额。
        let later = now + RPM_WINDOW;
        assert_eq!(limit_state(&key_a, limits, later), KeyLimitState::Free);
        assert_eq!(first_free(&relay, limits, later).as_deref(), Some("a"));
    }

    #[test]
    fn concurrency_full_key_recovers_when_the_slot_is_released() {
        let relay = relay("kp-conc", &[("a", "sk-a"), ("b", "sk-b")]);
        let limits = limits(None, Some(1));
        let now = Instant::now();
        let key_a = expand_keys(&relay)[0].key.clone();
        let slot = try_acquire(&key_a, limits, now).unwrap();
        assert_eq!(
            limit_state(&key_a, limits, now),
            KeyLimitState::Full { retry_in: None },
            "只因并发满了：要等进行中的请求结束"
        );
        assert!(try_acquire(&key_a, limits, now).is_none());
        assert_eq!(first_free(&relay, limits, now).as_deref(), Some("b"));
        drop(slot);
        assert_eq!(limit_state(&key_a, limits, now), KeyLimitState::Free);
        assert_eq!(first_free(&relay, limits, now).as_deref(), Some("a"));
    }

    #[test]
    fn cancelled_slot_refunds_the_rpm_count() {
        let relay = relay("kp-cancel", &[("a", "sk-a")]);
        let limits = limits(Some(1), Some(1));
        let now = Instant::now();
        let key = expand_keys(&relay)[0].key.clone();
        try_acquire(&key, limits, now).unwrap().cancel();
        assert_eq!(limit_state(&key, limits, now), KeyLimitState::Free);
        let slot = try_acquire(&key, limits, now).unwrap();
        drop(slot);
        assert!(
            matches!(
                limit_state(&key, limits, now),
                KeyLimitState::Full { retry_in: Some(_) }
            ),
            "真正发出去的那次留在 RPM 窗口里"
        );
    }

    #[test]
    fn unlimited_keys_are_never_tracked() {
        let relay = relay("kp-unlimited", &[("a", "sk-a")]);
        let key = expand_keys(&relay)[0].key.clone();
        let now = Instant::now();
        let slots: Vec<KeySlot> = (0..100)
            .map(|_| try_acquire(&key, KeyLimits::default(), now).unwrap())
            .collect();
        assert_eq!(
            limit_state(&key, KeyLimits::default(), now),
            KeyLimitState::Free
        );
        assert!(
            !usage_lock().contains_key(&(key.relay_id.clone(), key.key_id.clone())),
            "没设上限的 Key 不进计数表"
        );
        drop(slots);
    }

    #[test]
    fn saturated_keys_do_not_cool_down_or_trip_the_breaker() {
        let relay = relay("kp-limit-health", &[("a", "sk-a")]);
        let limits = limits(Some(1), Some(1));
        let now = Instant::now();
        let key = expand_keys(&relay)[0].key.clone();
        let _slot = try_acquire(&key, limits, now).unwrap();
        for _ in 0..5 {
            assert!(try_acquire(&key, limits, now).is_none());
        }
        let (keys, soonest) = order_keys(&relay, KeyRotationMode::Sticky, None, now);
        assert_eq!(keys.len(), 1, "限速满了不算冷却，Key 仍在候选里");
        assert_eq!(soonest, None);
        let snapshot = snapshot(now);
        assert!(
            snapshot
                .keys
                .iter()
                .all(|view| view.relay_id != "kp-limit-health")
        );
        assert!(
            snapshot
                .providers
                .iter()
                .all(|view| view.relay_id != "kp-limit-health")
        );
    }
}
