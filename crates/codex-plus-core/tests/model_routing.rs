//! End-to-end coverage for “按模型自动路由供应商” (per-model provider routing).
//!
//! These tests drive the real `/v1/responses` entry point against loopback
//! upstreams so the assertions cover what actually leaves the process: which
//! provider a model lands on, which credential that request carries, and which
//! official path is preserved.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use codex_plus_core::protocol_proxy::open_responses_proxy_request_with_settings_and_client_context;
use codex_plus_core::settings::{BackendSettings, RelayMode, RelayProfile, RelayProtocol};

const JSON_RESPONSE: &str = "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 35\r\nconnection: close\r\n\r\n{\"id\":\"resp_1\",\"object\":\"response\"}";
/// Discard port: any request routed here fails fast instead of silently passing.
const UNREACHABLE_BASE_URL: &str = "http://127.0.0.1:9/v1";
const OFFICIAL_BASE_URL_ENV: &str = "CODEX_PLUS_OFFICIAL_BASE_URL";

#[tokio::test]
async fn matched_model_goes_to_its_provider_with_that_providers_api_key() {
    let _lock = routing_test_lock();
    let temp = tempfile::tempdir().unwrap();
    let _settings_path = SettingsPathGuard::set(temp.path().join("settings.json"));
    let upstream = MockUpstream::start();
    let settings = routing_settings(vec![
        third_party_profile("alpha", UNREACHABLE_BASE_URL, "sk-alpha", "alpha-model"),
        third_party_profile(
            "beta",
            &format!("{}/v1", upstream.base_url()),
            "sk-beta",
            "beta-model\nbeta-mini",
        ),
    ]);

    let result = open_responses_proxy_request_with_settings_and_client_context(
        r#"{"model":"beta-model","input":"hi","stream":false}"#,
        settings,
        None,
        "/v1/responses",
        &[header("authorization", "Bearer official-token")],
    )
    .await
    .unwrap();

    assert_eq!(result.status_code, 200);
    let request = upstream.finish();
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/v1/responses");
    // The provider's own key, not the ChatGPT authorization the caller sent.
    assert_eq!(request.header("authorization"), Some("Bearer sk-beta"));
    assert_eq!(request.json()["model"], "beta-model");
}

#[tokio::test]
async fn model_declared_with_a_context_suffix_still_routes_to_its_provider() {
    let _lock = routing_test_lock();
    let temp = tempfile::tempdir().unwrap();
    let _settings_path = SettingsPathGuard::set(temp.path().join("settings.json"));
    let upstream = MockUpstream::start();
    let settings = routing_settings(vec![third_party_profile(
        "mimo",
        &format!("{}/v1", upstream.base_url()),
        "sk-mimo",
        "mimo-v2.5-pro[1M]",
    )]);

    let result = open_responses_proxy_request_with_settings_and_client_context(
        r#"{"model":"mimo-v2.5-pro","input":"hi","stream":false}"#,
        settings,
        None,
        "/v1/responses",
        &[header("authorization", "Bearer official-token")],
    )
    .await
    .unwrap();

    assert_eq!(result.status_code, 200);
    assert_eq!(
        upstream.finish().header("authorization"),
        Some("Bearer sk-mimo")
    );
}

#[tokio::test]
async fn unmatched_model_forwards_the_official_authorization() {
    let _lock = routing_test_lock();
    let temp = tempfile::tempdir().unwrap();
    let _settings_path = SettingsPathGuard::set(temp.path().join("settings.json"));
    let upstream = MockUpstream::start();
    let _official = EnvGuard::set(
        OFFICIAL_BASE_URL_ENV,
        &format!("{}/backend-api/codex", upstream.base_url()),
    );
    let settings = routing_settings(vec![third_party_profile(
        "beta",
        UNREACHABLE_BASE_URL,
        "sk-beta",
        "beta-model",
    )]);

    let result = open_responses_proxy_request_with_settings_and_client_context(
        r#"{"model":"gpt-5.6-codex","input":"hi","stream":false}"#,
        settings,
        Some("codex_cli_rs/1.0"),
        "/v1/responses",
        &[
            header("authorization", "Bearer official-token"),
            header("chatgpt-account-id", "acct_1"),
            header("originator", "codex_cli_rs"),
            header("x-api-key", "sk-must-not-leak"),
        ],
    )
    .await
    .unwrap();

    assert_eq!(result.status_code, 200);
    let request = upstream.finish();
    assert_eq!(request.path, "/backend-api/codex/responses");
    assert_eq!(
        request.header("authorization"),
        Some("Bearer official-token")
    );
    assert_eq!(request.header("chatgpt-account-id"), Some("acct_1"));
    assert_eq!(request.header("originator"), Some("codex_cli_rs"));
    // A third-party key must never ride along to the official backend.
    assert_eq!(request.header("x-api-key"), None);
    assert_eq!(request.json()["model"], "gpt-5.6-codex");
}

#[tokio::test]
async fn unmatched_model_keeps_the_compact_endpoint() {
    let _lock = routing_test_lock();
    let temp = tempfile::tempdir().unwrap();
    let _settings_path = SettingsPathGuard::set(temp.path().join("settings.json"));
    let upstream = MockUpstream::start();
    let _official = EnvGuard::set(
        OFFICIAL_BASE_URL_ENV,
        &format!("{}/backend-api/codex", upstream.base_url()),
    );
    let settings = routing_settings(vec![third_party_profile(
        "beta",
        UNREACHABLE_BASE_URL,
        "sk-beta",
        "beta-model",
    )]);

    let result = open_responses_proxy_request_with_settings_and_client_context(
        r#"{"model":"gpt-5.6-codex","input":"hi","stream":false}"#,
        settings,
        None,
        "/v1/responses/compact",
        &[header("authorization", "Bearer official-token")],
    )
    .await
    .unwrap();

    assert_eq!(result.status_code, 200);
    assert_eq!(
        upstream.finish().path,
        "/backend-api/codex/responses/compact"
    );
}

#[tokio::test]
async fn model_bound_to_two_providers_is_rejected_with_both_names() {
    let _lock = routing_test_lock();
    let temp = tempfile::tempdir().unwrap();
    let _settings_path = SettingsPathGuard::set(temp.path().join("settings.json"));
    let settings = routing_settings(vec![
        third_party_profile("provider-a", UNREACHABLE_BASE_URL, "sk-a", "shared-model"),
        third_party_profile("provider-b", UNREACHABLE_BASE_URL, "sk-b", "shared-model"),
    ]);

    let error = open_responses_proxy_request_with_settings_and_client_context(
        r#"{"model":"shared-model","input":"hi","stream":false}"#,
        settings,
        None,
        "/v1/responses",
        &[header("authorization", "Bearer official-token")],
    )
    .await
    .err()
    .expect("a model bound to two providers must not be routed")
    .to_string();

    assert!(error.contains("同时存在于多个供应商"), "{error}");
    assert!(error.contains("provider-a"), "{error}");
    assert!(error.contains("provider-b"), "{error}");
}

#[tokio::test]
async fn official_route_reports_a_missing_or_placeholder_chatgpt_login() {
    let _lock = routing_test_lock();
    let temp = tempfile::tempdir().unwrap();
    let _settings_path = SettingsPathGuard::set(temp.path().join("settings.json"));
    let profiles = vec![third_party_profile(
        "beta",
        UNREACHABLE_BASE_URL,
        "sk-beta",
        "beta-model",
    )];

    let missing = open_responses_proxy_request_with_settings_and_client_context(
        r#"{"model":"gpt-5.6-codex","input":"hi","stream":false}"#,
        routing_settings(profiles.clone()),
        None,
        "/v1/responses",
        &[],
    )
    .await
    .err()
    .expect("the official route must refuse a request with no authorization")
    .to_string();
    assert!(
        missing.contains("请先在 Codex 中完成 ChatGPT 登录"),
        "{missing}"
    );

    let placeholder = open_responses_proxy_request_with_settings_and_client_context(
        r#"{"model":"gpt-5.6-codex","input":"hi","stream":false}"#,
        routing_settings(profiles),
        None,
        "/v1/responses",
        &[header("authorization", "Bearer PROXY_MANAGED")],
    )
    .await
    .err()
    .expect("the official route must refuse the proxy placeholder credential")
    .to_string();
    assert!(
        placeholder.contains("重新加载官方登录认证"),
        "{placeholder}"
    );
}

#[tokio::test]
async fn routing_disabled_keeps_the_active_profile_for_every_model() {
    let _lock = routing_test_lock();
    let temp = tempfile::tempdir().unwrap();
    let _settings_path = SettingsPathGuard::set(temp.path().join("settings.json"));
    let upstream = MockUpstream::start();
    let settings = BackendSettings {
        model_routing_enabled: false,
        active_relay_id: "alpha".to_string(),
        ..routing_settings(vec![
            third_party_profile(
                "alpha",
                &format!("{}/v1", upstream.base_url()),
                "sk-alpha",
                "alpha-model",
            ),
            third_party_profile("beta", UNREACHABLE_BASE_URL, "sk-beta", "beta-model"),
        ])
    };

    // "beta-model" belongs to the beta profile, but with routing off the active
    // profile still owns the request — the pre-existing per-profile behaviour.
    let result = open_responses_proxy_request_with_settings_and_client_context(
        r#"{"model":"beta-model","input":"hi","stream":false}"#,
        settings,
        None,
        "/v1/responses",
        &[header("authorization", "Bearer official-token")],
    )
    .await
    .unwrap();

    assert_eq!(result.status_code, 200);
    assert_eq!(
        upstream.finish().header("authorization"),
        Some("Bearer sk-alpha")
    );
}

fn routing_settings(relay_profiles: Vec<RelayProfile>) -> BackendSettings {
    BackendSettings {
        model_routing_enabled: true,
        relay_profiles_enabled: true,
        relay_profiles,
        ..BackendSettings::default()
    }
}

fn third_party_profile(id: &str, base_url: &str, api_key: &str, model_list: &str) -> RelayProfile {
    RelayProfile {
        id: id.to_string(),
        name: id.to_string(),
        base_url: base_url.to_string(),
        api_key: api_key.to_string(),
        protocol: RelayProtocol::Responses,
        relay_mode: RelayMode::PureApi,
        model_list: model_list.to_string(),
        ..RelayProfile::default()
    }
}

fn header(name: &str, value: &str) -> (String, String) {
    (name.to_string(), value.to_string())
}

/// Serializes the tests: they share the process-wide settings path and the
/// official base URL environment variable.
fn routing_test_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct SettingsPathGuard {
    previous: Option<PathBuf>,
}

impl SettingsPathGuard {
    fn set(path: PathBuf) -> Self {
        Self {
            previous: codex_plus_core::paths::set_settings_path_for_tests(Some(path)),
        }
    }
}

impl Drop for SettingsPathGuard {
    fn drop(&mut self) {
        codex_plus_core::paths::set_settings_path_for_tests(self.previous.take());
    }
}

struct EnvGuard {
    key: &'static str,
    previous: Option<String>,
}

impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let previous = std::env::var(key).ok();
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, previous }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => unsafe { std::env::set_var(self.key, value) },
            None => unsafe { std::env::remove_var(self.key) },
        }
    }
}

struct MockUpstream {
    address: SocketAddr,
    handle: thread::JoinHandle<RecordedRequest>,
}

impl MockUpstream {
    fn start() -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let handle = thread::spawn(move || {
            let started = Instant::now();
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            started.elapsed() < Duration::from_secs(10),
                            "test upstream did not receive a request"
                        );
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("failed to accept test request: {error}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let raw = read_request(&mut stream);
            stream.write_all(JSON_RESPONSE.as_bytes()).unwrap();
            stream.flush().unwrap();
            parse_request(&raw)
        });
        Self { address, handle }
    }

    fn base_url(&self) -> String {
        format!("http://{}", self.address)
    }

    fn finish(self) -> RecordedRequest {
        self.handle.join().unwrap()
    }
}

struct RecordedRequest {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: String,
}

impl RecordedRequest {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header_name, _)| header_name == name)
            .map(|(_, value)| value.as_str())
    }

    fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body).expect("test upstream received a non-JSON body")
    }
}

fn read_request(stream: &mut TcpStream) -> Vec<u8> {
    let mut raw = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let read = stream
            .read(&mut chunk)
            .expect("failed to read test request");
        if read == 0 {
            break;
        }
        raw.extend_from_slice(&chunk[..read]);
        let Some(header_end) = find_header_end(&raw) else {
            continue;
        };
        let head = String::from_utf8_lossy(&raw[..header_end]).to_string();
        let content_length = head
            .lines()
            .filter_map(|line| line.split_once(':'))
            .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
            .and_then(|(_, value)| value.trim().parse::<usize>().ok())
            .unwrap_or(0);
        if raw.len() >= header_end + 4 + content_length {
            break;
        }
    }
    raw
}

fn find_header_end(raw: &[u8]) -> Option<usize> {
    raw.windows(4).position(|window| window == b"\r\n\r\n")
}

fn parse_request(raw: &[u8]) -> RecordedRequest {
    let header_end = find_header_end(raw).expect("test upstream received a truncated request");
    let head = String::from_utf8_lossy(&raw[..header_end]).to_string();
    let body = String::from_utf8_lossy(&raw[header_end + 4..]).to_string();
    let mut lines = head.lines();
    let mut request_line = lines.next().unwrap_or_default().split_whitespace();
    let method = request_line.next().unwrap_or_default().to_string();
    let path = request_line.next().unwrap_or_default().to_string();
    let headers = lines
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_ascii_lowercase(), value.trim().to_string()))
        })
        .collect();
    RecordedRequest {
        method,
        path,
        headers,
        body,
    }
}
