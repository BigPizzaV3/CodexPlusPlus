//! Request-scoped negotiation evidence and bounded error-body inspection.

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::relay_rotation::{RelayRotationSnapshot, RotationEvent};

type CacheKey = [u8; 32];
const CACHE_CAPACITY: usize = 256;
const CACHE_TTL: Duration = Duration::from_secs(30 * 60);
const INSPECTION_TIMEOUT: Duration = Duration::from_secs(5);

fn cache() -> &'static Mutex<VecDeque<(CacheKey, Instant)>> {
    static CACHE: OnceLock<Mutex<VecDeque<(CacheKey, Instant)>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(VecDeque::new()))
}

pub(crate) fn cache_key(
    relay_id: &str,
    request: &reqwest::Request,
    body: &Value,
    user_agent: &str,
) -> CacheKey {
    let mut hash = Sha256::new();
    let mut field = |bytes: &[u8]| {
        hash.update((bytes.len() as u64).to_be_bytes());
        hash.update(bytes);
    };
    field(relay_id.as_bytes());
    field(request.url().as_str().as_bytes());
    field(
        body.get("model")
            .and_then(Value::as_str)
            .unwrap_or("")
            .as_bytes(),
    );
    field(
        serde_json::to_string(&body.get("tools"))
            .unwrap_or_default()
            .as_bytes(),
    );
    field(user_agent.as_bytes());
    let mut headers = request.headers().iter().collect::<Vec<_>>();
    headers.sort_by(|left, right| left.0.as_str().cmp(right.0.as_str()));
    for (name, value) in headers {
        field(name.as_str().as_bytes());
        field(value.as_bytes());
    }
    hash.finalize().into()
}

pub(crate) fn cached(key: &CacheKey) -> bool {
    let mut entries = cache().lock().unwrap_or_else(|error| error.into_inner());
    entries.retain(|(_, created)| created.elapsed() < CACHE_TTL);
    entries.iter().any(|(entry, _)| entry == key)
}

pub(crate) fn evict(key: &CacheKey) {
    cache()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .retain(|(entry, _)| entry != key);
}

pub(crate) struct ExecOutcome {
    key: CacheKey,
    snapshot: Option<RelayRotationSnapshot>,
    finished: bool,
}

impl ExecOutcome {
    pub(crate) fn new(key: CacheKey, snapshot: Option<RelayRotationSnapshot>) -> Self {
        Self {
            key,
            snapshot,
            finished: false,
        }
    }

    // Dropping an unfinished outcome is cancellation, not capability evidence.
    pub(crate) fn finish(&mut self, success: bool) {
        if self.finished {
            return;
        }
        self.finished = true;
        evict(&self.key);
        if success {
            let mut entries = cache().lock().unwrap_or_else(|error| error.into_inner());
            entries.retain(|(_, created)| created.elapsed() < CACHE_TTL);
            entries.push_back((self.key, Instant::now()));
            while entries.len() > CACHE_CAPACITY {
                entries.pop_front();
            }
        }
        if let Some(snapshot) = &self.snapshot {
            crate::relay_rotation::record_relay_request_event_for_snapshot(
                snapshot,
                if success {
                    RotationEvent::Success
                } else {
                    RotationEvent::Failure
                },
            );
        }
    }
}

/// Rebuilds the response with every inspected byte followed by its unread body.
/// A timeout or size overflow therefore retains ordinary upstream error handling.
pub(crate) async fn inspect_rejection(
    mut response: reqwest::Response,
    request: &Value,
) -> anyhow::Result<(reqwest::Response, bool)> {
    let status = response.status();
    let version = response.version();
    let headers = response.headers().clone();
    let content_type = headers
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let deadline = tokio::time::Instant::now() + INSPECTION_TIMEOUT;
    let mut chunks = Vec::new();
    let mut inspected = Vec::new();
    let mut complete = false;
    loop {
        match tokio::time::timeout_at(deadline, response.chunk()).await {
            Ok(Ok(Some(bytes))) => {
                let fits = inspected.len().saturating_add(bytes.len())
                    <= crate::responses_exec::MAX_REJECTION_BODY_BYTES;
                if fits {
                    inspected.extend_from_slice(&bytes);
                }
                chunks.push(Ok::<_, reqwest::Error>(bytes));
                if !fits {
                    break;
                }
            }
            Ok(Ok(None)) => {
                complete = true;
                break;
            }
            Ok(Err(error)) => {
                chunks.push(Err(error));
                break;
            }
            Err(_) => break,
        }
    }
    let matches = complete
        && crate::responses_exec::is_explicit_exec_rejection(
            status.as_u16(),
            content_type,
            &inspected,
            request,
        );
    let body = reqwest::Body::wrap_stream(
        futures_util::stream::iter(chunks).chain(response.bytes_stream()),
    );
    let mut rebuilt = http::Response::builder()
        .status(status)
        .version(version)
        .body(body)?;
    *rebuilt.headers_mut() = headers;
    Ok((reqwest::Response::from(rebuilt), matches))
}

pub(crate) fn adapted_json(bytes: &[u8]) -> (Vec<u8>, bool) {
    let result = (|| -> anyhow::Result<Value> {
        let value: Value = serde_json::from_slice(bytes)?;
        anyhow::ensure!(
            value.get("status").and_then(Value::as_str) == Some("completed")
                && value.get("output").is_some_and(Value::is_array),
            "adapted response is not complete"
        );
        crate::responses_exec::adapt_response(value)
    })();
    match result {
        Ok(value) => (
            serde_json::to_vec(&value).expect("JSON value serializes"),
            true,
        ),
        Err(_) => (
            serde_json::to_vec(&serde_json::json!({
                "object": "response", "status": "failed", "output": [],
                "error": {"code": "responses_exec_adaptation_failed",
                          "message": "Invalid or incomplete adapted exec response"}
            }))
            .expect("JSON value serializes"),
            false,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_uses_effective_headers_endpoint_model_and_tools() {
        let body = serde_json::json!({"model":"one","tools":[{"type":"custom","name":"exec"}]});
        let client = reqwest::Client::new();
        let first = client
            .post("http://localhost/v1/responses")
            .header("authorization", "Bearer first")
            .header("x-tenant", "one")
            .build()
            .unwrap();
        let key = cache_key("relay", &first, &body, "ua");
        let second = client
            .post("http://localhost/v1/responses")
            .header("authorization", "Bearer first")
            .header("x-tenant", "two")
            .build()
            .unwrap();
        assert_ne!(key, cache_key("relay", &second, &body, "ua"));
        assert_ne!(key, cache_key("other-relay", &first, &body, "ua"));
        let mut changed = body.clone();
        changed["model"] = serde_json::json!("two");
        assert_ne!(key, cache_key("relay", &first, &changed, "ua"));
        changed["tools"] = serde_json::json!([]);
        assert_ne!(key, cache_key("relay", &first, &changed, "ua"));
    }

    #[test]
    fn identity_includes_client_default_user_agent() {
        let body = serde_json::json!({"model":"one","tools":[{"type":"custom","name":"exec"}]});
        let first_client = reqwest::Client::builder()
            .user_agent("ua-one")
            .build()
            .unwrap();
        let second_client = reqwest::Client::builder()
            .user_agent("ua-two")
            .build()
            .unwrap();
        let first = first_client
            .post("http://localhost/v1/responses")
            .header("authorization", "Bearer same")
            .build()
            .unwrap();
        let second = second_client
            .post("http://localhost/v1/responses")
            .header("authorization", "Bearer same")
            .build()
            .unwrap();
        assert_ne!(
            cache_key("relay", &first, &body, "ua-one"),
            cache_key("relay", &second, &body, "ua-two")
        );
    }

    #[test]
    fn invalid_json_produces_no_executable_output() {
        for bytes in [b"{}".as_slice(), b"not JSON".as_slice()] {
            let (output, valid) = adapted_json(bytes);
            assert!(!valid);
            let output: Value = serde_json::from_slice(&output).unwrap();
            assert_eq!(output["status"], "failed");
            assert_eq!(output["output"], serde_json::json!([]));
        }
    }

    #[tokio::test]
    async fn inspection_preserves_unmatched_and_oversized_bodies() {
        let request = serde_json::json!({"tools":[{"type":"custom","name":"exec"}]});
        for bytes in [
            br#"{"error":{"type":"invalid_request_error","message":"different"}}"#.to_vec(),
            vec![b'x'; crate::responses_exec::MAX_REJECTION_BODY_BYTES + 1],
        ] {
            let response = http::Response::builder()
                .status(400)
                .header("content-type", "application/json")
                .body(reqwest::Body::from(bytes.clone()))
                .unwrap();
            let (response, matched) = inspect_rejection(response.into(), &request).await.unwrap();
            assert!(!matched);
            assert_eq!(response.status().as_u16(), 400);
            assert_eq!(response.bytes().await.unwrap().as_ref(), bytes.as_slice());
        }
    }

    #[tokio::test]
    async fn inspection_deadline_retains_the_unread_stream() {
        let prefix = b"{\"error\":".to_vec();
        let tail = b"null}".to_vec();
        let chunks = futures_util::stream::iter(vec![Ok::<_, std::io::Error>(prefix.clone())])
            .chain(futures_util::stream::once(async move {
                tokio::time::sleep(INSPECTION_TIMEOUT + Duration::from_millis(100)).await;
                Ok::<_, std::io::Error>(tail)
            }));
        let response = http::Response::builder()
            .status(400)
            .header("content-type", "application/json")
            .body(reqwest::Body::wrap_stream(chunks))
            .unwrap();
        let request = serde_json::json!({"tools":[{"type":"custom","name":"exec"}]});
        let started = Instant::now();
        let (response, matched) = inspect_rejection(response.into(), &request).await.unwrap();
        assert!(!matched);
        assert!(started.elapsed() < INSPECTION_TIMEOUT + Duration::from_millis(500));
        assert_eq!(
            response.bytes().await.unwrap().as_ref(),
            b"{\"error\":null}"
        );
    }

    #[test]
    fn only_completed_validated_outcomes_create_bounded_cache_evidence() {
        let canceled_key = Sha256::digest(b"canceled-exec-outcome").into();
        drop(ExecOutcome::new(canceled_key, None));
        assert!(!cached(&canceled_key));
        let good_key = Sha256::digest(b"successful-exec-outcome").into();
        let mut valid = ExecOutcome::new(good_key, None);
        valid.finish(true);
        assert!(cached(&good_key));
        valid.finish(false);
        assert!(cached(&good_key), "outcomes are accounted exactly once");
        let mut invalid = ExecOutcome::new(good_key, None);
        invalid.finish(false);
        assert!(!cached(&good_key));
        for index in 0..=CACHE_CAPACITY {
            let key = Sha256::digest(index.to_be_bytes()).into();
            ExecOutcome::new(key, None).finish(true);
        }
        let mut entries = cache().lock().unwrap_or_else(|error| error.into_inner());
        assert_eq!(entries.len(), CACHE_CAPACITY);
        entries.push_back((canceled_key, Instant::now() - CACHE_TTL));
        drop(entries);
        assert!(!cached(&canceled_key));
    }

    fn failover_settings(pool_id: &str) -> crate::settings::BackendSettings {
        use crate::settings::{
            AggregateRelayMember, AggregateRelayProfile, AggregateRelayStrategy, BackendSettings,
            RelayMode, RelayProfile,
        };
        BackendSettings {
            active_relay_id: pool_id.into(),
            active_aggregate_relay_id: pool_id.into(),
            relay_profiles: vec![
                RelayProfile {
                    id: pool_id.into(),
                    relay_mode: RelayMode::Aggregate,
                    ..Default::default()
                },
                RelayProfile {
                    id: "exec-a".into(),
                    relay_mode: RelayMode::PureApi,
                    base_url: "http://127.0.0.1:1/v1".into(),
                    api_key: "fixture-a".into(),
                    ..Default::default()
                },
                RelayProfile {
                    id: "exec-b".into(),
                    relay_mode: RelayMode::PureApi,
                    base_url: "http://127.0.0.1:2/v1".into(),
                    api_key: "fixture-b".into(),
                    ..Default::default()
                },
            ],
            aggregate_relay_profiles: vec![AggregateRelayProfile {
                id: pool_id.into(),
                name: "exec pool fixture".into(),
                session_provider: crate::settings::RelaySessionProvider::Custom,
                strategy: AggregateRelayStrategy::Failover,
                model_aggregation_enabled: false,
                members: vec![
                    AggregateRelayMember {
                        relay_id: "exec-a".into(),
                        weight: 1,
                    },
                    AggregateRelayMember {
                        relay_id: "exec-b".into(),
                        weight: 1,
                    },
                ],
                routes: Vec::new(),
                default_relay_id: String::new(),
                sync_default_member_login: false,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn old_exec_outcome_cannot_advance_a_new_generation() {
        use crate::relay_rotation::{RotationContext, select_relay_for_request_with_revision};
        let _guard = crate::paths::settings_path_test_guard();
        let _rotation_guard = crate::relay_rotation::rotation_test_guard();
        let mut settings = failover_settings("old-exec-pool");
        let (old_relay, snapshot) =
            select_relay_for_request_with_revision(&settings, RotationContext::default(), "")
                .unwrap();
        assert_eq!(old_relay.id, "exec-a");
        let mut old = ExecOutcome::new(
            Sha256::digest(b"old-exec-generation").into(),
            Some(snapshot),
        );
        // 成员凭据改变而池定义不变，必须建立新 generation。
        settings.relay_profiles[1].api_key = "fixture-new-credential".into();
        let (current, _) =
            select_relay_for_request_with_revision(&settings, RotationContext::default(), "")
                .unwrap();
        assert_eq!(current.id, "exec-a");
        old.finish(false);
        let (next, snapshot) =
            select_relay_for_request_with_revision(&settings, RotationContext::default(), "")
                .unwrap();
        assert_eq!(next.id, "exec-a");
        // 正向控制：新 generation 的真实失败仍能推进。
        ExecOutcome::new(
            Sha256::digest(b"current-exec-generation").into(),
            Some(snapshot),
        )
        .finish(false);
        let (next, _) =
            select_relay_for_request_with_revision(&settings, RotationContext::default(), "")
                .unwrap();
        assert_eq!(next.id, "exec-b");
    }

    #[test]
    fn strict_exec_without_snapshot_never_changes_the_active_pool() {
        use crate::relay_rotation::{RotationContext, select_relay_for_request_with_revision};
        let _guard = crate::paths::settings_path_test_guard();
        let _rotation_guard = crate::relay_rotation::rotation_test_guard();
        let settings = failover_settings("strict-exec-pool");
        let (_, snapshot) =
            select_relay_for_request_with_revision(&settings, RotationContext::default(), "")
                .unwrap();
        crate::relay_rotation::record_relay_request_event_for_snapshot(
            &snapshot,
            RotationEvent::Failure,
        );
        let (before, _) =
            select_relay_for_request_with_revision(&settings, RotationContext::default(), "")
                .unwrap();
        assert_eq!(before.id, "exec-b");
        for succeeded in [false, true] {
            ExecOutcome::new(Sha256::digest([succeeded as u8]).into(), None).finish(succeeded);
            let (after, _) =
                select_relay_for_request_with_revision(&settings, RotationContext::default(), "")
                    .unwrap();
            assert_eq!(after.id, before.id);
        }
    }
}
