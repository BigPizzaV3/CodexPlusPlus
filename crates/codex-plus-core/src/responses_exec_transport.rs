//! Request-scoped negotiation evidence and bounded error-body inspection.

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::relay_rotation::RotationEvent;
use crate::settings::BackendSettings;

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
    // Codex 透传的轮次级身份头每轮都变，算进键里协商结果就永远命不中（#2783）。
    let mut headers = request
        .headers()
        .iter()
        .filter(|(name, _)| {
            !crate::protocol_proxy::is_volatile_client_identity_header(name.as_str())
        })
        .collect::<Vec<_>>();
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
    /// 协商得来的适配才有缓存键；按上游类型直接适配的没有，也不进协商缓存。
    key: Option<CacheKey>,
    adaptation: crate::responses_exec::ToolAdaptation,
    settings: BackendSettings,
    finished: bool,
}

impl ExecOutcome {
    pub(crate) fn new(
        key: Option<CacheKey>,
        adaptation: crate::responses_exec::ToolAdaptation,
        settings: &BackendSettings,
    ) -> Self {
        Self {
            key,
            adaptation,
            settings: settings.clone(),
            finished: false,
        }
    }

    /// 响应要按哪些工具改写还原。
    pub(crate) fn adaptation(&self) -> &crate::responses_exec::ToolAdaptation {
        &self.adaptation
    }

    // Dropping an unfinished outcome is cancellation, not capability evidence.
    pub(crate) fn finish(&mut self, success: bool) {
        if self.finished {
            return;
        }
        self.finished = true;
        if let Some(key) = self.key {
            evict(&key);
            if success {
                let mut entries = cache().lock().unwrap_or_else(|error| error.into_inner());
                entries.retain(|(_, created)| created.elapsed() < CACHE_TTL);
                entries.push_back((key, Instant::now()));
                while entries.len() > CACHE_CAPACITY {
                    entries.pop_front();
                }
            }
        }
        crate::relay_rotation::record_relay_request_event(
            &self.settings,
            if success {
                RotationEvent::Success
            } else {
                RotationEvent::Failure
            },
        );
    }
}

/// Rebuilds the response with every inspected byte followed by its unread body.
/// A timeout or size overflow therefore retains ordinary upstream error handling.
pub(crate) async fn inspect_rejection(
    response: reqwest::Response,
    request: &Value,
) -> anyhow::Result<(reqwest::Response, bool)> {
    inspect_error_body(response, |status, content_type, body| {
        crate::responses_exec::is_explicit_exec_rejection(status, content_type, body, request)
    })
    .await
}

/// 有上限地预读错误响应体交给 `matcher` 判断，再把读过的字节接回去重建响应；
/// 没读完整（超时、超长）时一律判为不匹配，原响应照常交给后续处理。
pub(crate) async fn inspect_error_body(
    mut response: reqwest::Response,
    matcher: impl FnOnce(u16, &str, &[u8]) -> bool,
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
    let matches = complete && matcher(status.as_u16(), content_type, &inspected);
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

pub(crate) fn adapted_json(
    bytes: &[u8],
    adaptation: &crate::responses_exec::ToolAdaptation,
) -> (Vec<u8>, bool) {
    let result = (|| -> anyhow::Result<Value> {
        let value: Value = serde_json::from_slice(bytes)?;
        anyhow::ensure!(
            value.get("status").and_then(Value::as_str) == Some("completed")
                && value.get("output").is_some_and(Value::is_array),
            "adapted response is not complete"
        );
        crate::responses_exec::adapt_response_for(value, adaptation)
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
    fn identity_ignores_per_turn_codex_headers_but_keeps_originator() {
        // #2783：轮次级的身份头每轮都变，不能让协商缓存每轮失效；originator 这类稳定头照常算进键。
        let body = serde_json::json!({"model":"one","tools":[{"type":"custom","name":"exec"}]});
        let client = reqwest::Client::new();
        let request = |turn: &str, originator: &str| {
            client
                .post("http://localhost/v1/responses")
                .header("authorization", "Bearer same")
                .header("originator", originator)
                .header(
                    "x-codex-turn-metadata",
                    format!("{{\"turn_id\":\"{turn}\"}}"),
                )
                .header("x-codex-window-id", format!("win:{turn}"))
                .header("x-client-request-id", turn)
                .build()
                .unwrap()
        };
        assert_eq!(
            cache_key("relay", &request("t1", "codex_cli_rs"), &body, "ua"),
            cache_key("relay", &request("t2", "codex_cli_rs"), &body, "ua")
        );
        assert_ne!(
            cache_key("relay", &request("t1", "codex_cli_rs"), &body, "ua"),
            cache_key("relay", &request("t1", "codex_exec"), &body, "ua")
        );
    }

    #[test]
    fn invalid_json_produces_no_executable_output() {
        for bytes in [b"{}".as_slice(), b"not JSON".as_slice()] {
            let (output, valid) =
                adapted_json(bytes, &crate::responses_exec::ToolAdaptation::exec());
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
        let settings = BackendSettings::default();
        let canceled_key = Sha256::digest(b"canceled-exec-outcome").into();
        drop(ExecOutcome::new(
            Some(canceled_key),
            crate::responses_exec::ToolAdaptation::exec(),
            &settings,
        ));
        assert!(!cached(&canceled_key));
        let good_key = Sha256::digest(b"successful-exec-outcome").into();
        let mut valid = ExecOutcome::new(
            Some(good_key),
            crate::responses_exec::ToolAdaptation::exec(),
            &settings,
        );
        valid.finish(true);
        assert!(cached(&good_key));
        valid.finish(false);
        assert!(cached(&good_key), "outcomes are accounted exactly once");
        let mut invalid = ExecOutcome::new(
            Some(good_key),
            crate::responses_exec::ToolAdaptation::exec(),
            &settings,
        );
        invalid.finish(false);
        assert!(!cached(&good_key));
        for index in 0..=CACHE_CAPACITY {
            let key = Sha256::digest(index.to_be_bytes()).into();
            ExecOutcome::new(
                Some(key),
                crate::responses_exec::ToolAdaptation::exec(),
                &settings,
            )
            .finish(true);
        }
        let mut entries = cache().lock().unwrap_or_else(|error| error.into_inner());
        assert_eq!(entries.len(), CACHE_CAPACITY);
        entries.push_back((canceled_key, Instant::now() - CACHE_TTL));
        drop(entries);
        assert!(!cached(&canceled_key));
    }
}
