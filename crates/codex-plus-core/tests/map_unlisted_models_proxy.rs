// 目录外模型映射（#1183）的端到端行为：经本地协议代理 Responses 路径后，
// 断言上游实际收到的 model 字段。
//
// 重点守住两条：开关默认关闭时原样转发；codex-auto-review 在配置了
// autoReviewModel（#1446）时按其改写，不被目录外映射抢先改成默认模型。
use codex_plus_core::protocol_proxy::open_responses_proxy_request_with_settings_for_path;
use codex_plus_core::settings::{
    BackendSettings, RelayMode, RelayProfile, RelayProtocol, SettingsStore,
};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn settings_path_test_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct TestSettings {
    previous: Option<PathBuf>,
    settings: BackendSettings,
    _temp: tempfile::TempDir,
}

impl TestSettings {
    fn new(server: &MockServer, map_unlisted: Option<bool>, auto_review_model: &str) -> Self {
        static FIXTURE_ID: AtomicUsize = AtomicUsize::new(0);
        let relay_id = format!(
            "map-unlisted-{}",
            FIXTURE_ID.fetch_add(1, Ordering::Relaxed)
        );
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings.json");
        let mut settings = BackendSettings {
            active_relay_id: relay_id.clone(),
            relay_profiles: vec![RelayProfile {
                id: relay_id,
                name: "map unlisted fixture".to_string(),
                relay_mode: RelayMode::PureApi,
                protocol: RelayProtocol::Responses,
                base_url: format!("{}/v1", server.uri()),
                model: "minimax-m2.5".to_string(),
                model_list: "minimax-m2.5\ndeepseek-v4-flash".to_string(),
                api_key: "local-fixture-key".to_string(),
                auto_review_model: auto_review_model.to_string(),
                ..RelayProfile::default()
            }],
            protocol_proxy_auto_retry: false,
            ..BackendSettings::default()
        };
        // None 表示沿用默认值，用来守住「默认关闭」。
        if let Some(enabled) = map_unlisted {
            settings.protocol_proxy_map_unlisted_models = enabled;
        }
        SettingsStore::new(path.clone()).save(&settings).unwrap();
        let previous = codex_plus_core::paths::set_settings_path_for_tests(Some(path));
        Self {
            previous,
            settings,
            _temp: temp,
        }
    }
}

impl Drop for TestSettings {
    fn drop(&mut self) {
        codex_plus_core::paths::set_settings_path_for_tests(self.previous.take());
    }
}

/// 发一条请求，返回上游收到的 model 字段。
async fn upstream_model_for(map_unlisted: Option<bool>, auto_review_model: &str, model: &str) -> String {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "resp_fixture",
            "object": "response",
            "status": "completed",
            "output": [],
            "usage": {"input_tokens": 1, "output_tokens": 1, "total_tokens": 2}
        })))
        .mount(&server)
        .await;
    let settings = TestSettings::new(&server, map_unlisted, auto_review_model);
    let body = json!({
        "model": model,
        "stream": false,
        "store": false,
        "input": [{"type": "message", "role": "user", "content": "hi"}]
    });
    let response = open_responses_proxy_request_with_settings_for_path(
        &body.to_string(),
        settings.settings.clone(),
        "/v1/responses",
    )
    .await
    .unwrap();
    assert_eq!(response.status_code, 200);
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let sent: Value = serde_json::from_slice(&requests[0].body).unwrap();
    sent["model"].as_str().unwrap_or_default().to_string()
}

#[tokio::test]
async fn unlisted_model_passes_through_when_switch_is_default_off() {
    let _guard = settings_path_test_lock().lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(upstream_model_for(None, "", "gpt-5.4").await, "gpt-5.4");
    assert_eq!(
        upstream_model_for(Some(false), "", "codex-auto-review").await,
        "codex-auto-review"
    );
}

#[tokio::test]
async fn unlisted_model_maps_to_default_when_switch_is_on() {
    let _guard = settings_path_test_lock().lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(upstream_model_for(Some(true), "", "gpt-5.4").await, "minimax-m2.5");
    // 列表内模型照原样转发。
    assert_eq!(
        upstream_model_for(Some(true), "", "deepseek-v4-flash").await,
        "deepseek-v4-flash"
    );
    // 未配置审批模型时 codex-auto-review 也属于目录外模型，映射到默认模型。
    assert_eq!(
        upstream_model_for(Some(true), "", "codex-auto-review").await,
        "minimax-m2.5"
    );
}

#[tokio::test]
async fn auto_review_model_wins_over_unlisted_mapping() {
    let _guard = settings_path_test_lock().lock().unwrap_or_else(|e| e.into_inner());
    // 两者同时开启：codex-auto-review 按 autoReviewModel 改写，不被映射成默认模型。
    assert_eq!(
        upstream_model_for(Some(true), "deepseek-v4-flash", "codex-auto-review").await,
        "deepseek-v4-flash"
    );
    // 映射关闭时审批模型改写照常生效（#1446 原有行为）。
    assert_eq!(
        upstream_model_for(Some(false), "deepseek-v4-flash", "codex-auto-review").await,
        "deepseek-v4-flash"
    );
    // 其他目录外模型仍按默认模型映射。
    assert_eq!(
        upstream_model_for(Some(true), "deepseek-v4-flash", "gpt-5.4").await,
        "minimax-m2.5"
    );
}
