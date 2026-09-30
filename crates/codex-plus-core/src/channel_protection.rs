use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use reqwest::header::{HeaderMap, RETRY_AFTER};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

use crate::settings::{
    RelayProfile, normalize_channel_cooldown_statuses,
    normalize_channel_requests_per_minute,
};

const DEFAULT_COOLDOWN: Duration = Duration::from_secs(30);
const REQUEST_WINDOW: Duration = Duration::from_secs(60);

struct ChannelState {
    queue: Arc<AsyncMutex<()>>,
    runtime: AsyncMutex<ChannelRuntime>,
}

#[derive(Default)]
struct ChannelRuntime {
    cooldown_until: Option<Instant>,
    request_times: VecDeque<Instant>,
}

pub struct ChannelPermit {
    _queue_guard: Option<OwnedMutexGuard<()>>,
}

static CHANNELS: OnceLock<Mutex<HashMap<String, Arc<ChannelState>>>> = OnceLock::new();

fn channel_state(key: &str) -> Arc<ChannelState> {
    let channels = CHANNELS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut channels = channels
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    channels
        .entry(key.to_string())
        .or_insert_with(|| {
            Arc::new(ChannelState {
                queue: Arc::new(AsyncMutex::new(())),
                runtime: AsyncMutex::new(ChannelRuntime::default()),
            })
        })
        .clone()
}

pub fn key_for_relay(relay: &RelayProfile) -> String {
    if !relay.id.trim().is_empty() {
        return relay.id.trim().to_string();
    }
    relay.base_url.trim().trim_end_matches('/').to_ascii_lowercase()
}

pub async fn acquire(key: &str, profile: &RelayProfile) -> ChannelPermit {
    let state = channel_state(key);
    if profile.rate_limit_cooldown_enabled {
        wait_for_cooldown(&state).await;
    }
    let queue_guard = if profile.channel_queue_enabled {
        Some(state.queue.clone().lock_owned().await)
    } else {
        None
    };

    if profile.rate_limit_cooldown_enabled {
        wait_for_cooldown(&state).await;
    }
    if profile.channel_queue_enabled {
        reserve_request(
            &state,
            normalize_channel_requests_per_minute(profile.channel_requests_per_minute),
        )
        .await;
    }

    ChannelPermit {
        _queue_guard: queue_guard,
    }
}

async fn wait_for_cooldown(state: &ChannelState) {
    loop {
        let remaining = {
            let mut runtime = state.runtime.lock().await;
            let Some(until) = runtime.cooldown_until else {
                return;
            };
            let now = Instant::now();
            if until <= now {
                runtime.cooldown_until = None;
                return;
            }
            until.saturating_duration_since(now)
        };
        tokio::time::sleep(remaining).await;
    }
}

async fn reserve_request(state: &ChannelState, request_limit: u32) {
    loop {
        let wait = {
            let mut runtime = state.runtime.lock().await;
            let now = Instant::now();
            runtime
                .request_times
                .retain(|timestamp| now.duration_since(*timestamp) < REQUEST_WINDOW);
            if runtime.request_times.len() < request_limit as usize {
                runtime.request_times.push_back(now);
                return;
            }
            runtime
                .request_times
                .front()
                .map(|oldest| {
                    oldest
                        .saturating_duration_since(now)
                        .max(REQUEST_WINDOW.saturating_sub(now.duration_since(*oldest)))
                })
                .unwrap_or(REQUEST_WINDOW)
        };
        tokio::time::sleep(wait.max(Duration::from_millis(1))).await;
    }
}

pub async fn mark_failure(
    key: &str,
    profile: &RelayProfile,
    status_code: u16,
    retry_after: Option<Duration>,
) -> bool {
    if !profile.rate_limit_cooldown_enabled
        || !normalize_channel_cooldown_statuses(&profile.cooldown_error_statuses)
            .contains(&status_code)
    {
        return false;
    }

    let state = channel_state(key);
    let delay = retry_after
        .filter(|duration| !duration.is_zero())
        .map(|duration| duration.max(DEFAULT_COOLDOWN))
        .unwrap_or(DEFAULT_COOLDOWN);
    let until = Instant::now() + delay;
    let mut runtime = state.runtime.lock().await;
    runtime.cooldown_until = Some(
        runtime
            .cooldown_until
            .map(|existing| existing.max(until))
            .unwrap_or(until),
    );
    true
}

pub fn retry_after_duration(headers: &HeaderMap) -> Option<Duration> {
    let value = headers.get(RETRY_AFTER)?.to_str().ok()?.trim();
    let seconds = value.parse::<u64>().ok()?;
    Some(Duration::from_secs(seconds))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::RelayProfile;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_KEY_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn test_key() -> String {
        format!(
            "channel-protection-test-{}",
            TEST_KEY_COUNTER.fetch_add(1, Ordering::Relaxed)
        )
    }

    fn enabled_profile() -> RelayProfile {
        RelayProfile {
            rate_limit_cooldown_enabled: true,
            channel_queue_enabled: true,
            ..RelayProfile::default()
        }
    }

    #[test]
    fn relay_key_is_shared_by_profile_id() {
        let relay = RelayProfile {
            id: "provider-a".to_string(),
            base_url: "https://one.example/v1".to_string(),
            ..RelayProfile::default()
        };
        assert_eq!(key_for_relay(&relay), "provider-a");
    }

    #[test]
    fn retry_after_accepts_delta_seconds() {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, "90".parse().unwrap());
        assert_eq!(
            retry_after_duration(&headers),
            Some(Duration::from_secs(90))
        );
    }

    #[test]
    fn retry_after_ignores_invalid_values() {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, "tomorrow".parse().unwrap());
        assert_eq!(retry_after_duration(&headers), None);
    }

    #[tokio::test]
    async fn failure_uses_at_least_the_default_cooldown() {
        let key = test_key();
        let profile = enabled_profile();
        assert!(mark_failure(&key, &profile, 429, Some(Duration::from_secs(1))).await);

        let state = channel_state(&key);
        let runtime = state.runtime.lock().await;
        let remaining = runtime
            .cooldown_until
            .expect("failure should set a cooldown")
            .saturating_duration_since(Instant::now());
        assert!(remaining >= DEFAULT_COOLDOWN.saturating_sub(Duration::from_secs(1)));
    }

    #[test]
    fn default_cooldown_is_thirty_seconds() {
        assert_eq!(DEFAULT_COOLDOWN, Duration::from_secs(30));
    }

    #[tokio::test]
    async fn failure_prefers_a_longer_retry_after() {
        let key = test_key();
        let profile = enabled_profile();
        let retry_after = Duration::from_secs(120);
        assert!(mark_failure(&key, &profile, 500, Some(retry_after)).await);

        let state = channel_state(&key);
        let runtime = state.runtime.lock().await;
        let remaining = runtime
            .cooldown_until
            .expect("failure should set a cooldown")
            .saturating_duration_since(Instant::now());
        assert!(remaining >= retry_after.saturating_sub(Duration::from_secs(1)));
    }

    #[tokio::test]
    async fn unconfigured_status_does_not_set_cooldown() {
        let key = test_key();
        let profile = RelayProfile {
            rate_limit_cooldown_enabled: true,
            cooldown_error_statuses: vec![429],
            ..RelayProfile::default()
        };
        assert!(!mark_failure(&key, &profile, 500, None).await);

        let state = channel_state(&key);
        assert!(state.runtime.lock().await.cooldown_until.is_none());
    }

    #[tokio::test]
    async fn request_reservation_is_isolated_by_channel() {
        let first_key = test_key();
        let second_key = test_key();
        let first = channel_state(&first_key);
        let second = channel_state(&second_key);

        reserve_request(&first, 2).await;
        reserve_request(&first, 2).await;
        reserve_request(&second, 2).await;

        assert_eq!(first.runtime.lock().await.request_times.len(), 2);
        assert_eq!(second.runtime.lock().await.request_times.len(), 1);
    }

    #[tokio::test]
    async fn queue_serializes_requests_for_the_same_channel() {
        let key = test_key();
        let profile = enabled_profile();
        let first_permit = acquire(&key, &profile).await;
        let (sender, mut receiver) = tokio::sync::oneshot::channel();
        let waiter_key = key.clone();
        let waiter_profile = profile.clone();
        let waiter = tokio::spawn(async move {
            let _permit = acquire(&waiter_key, &waiter_profile).await;
            let _ = sender.send(());
        });

        tokio::task::yield_now().await;
        assert!(receiver.try_recv().is_err());
        drop(first_permit);
        receiver.await.unwrap();
        waiter.await.unwrap();
    }
}
