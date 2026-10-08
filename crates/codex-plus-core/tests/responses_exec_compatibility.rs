use codex_plus_core::protocol_proxy::{
    ProxyHttpResponse, handle_responses_proxy_request,
    open_responses_proxy_request_with_settings_for_path,
};
use codex_plus_core::settings::{
    BackendSettings, RelayMode, RelayProfile, RelayProtocol, SettingsStore,
};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const EXEC_REJECTION: &str = "Unsupported custom tool: 'exec'. Only 'apply_patch' is supported.";
const PROGRAM: &str = "const result = await tools.lookup({ query: \"a\\\\b\\n\" });\ntext(result);";

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
    fn new(server: &MockServer) -> Self {
        // wiremock reuses servers; each fixture needs its own relay/cache identity.
        static FIXTURE_ID: AtomicUsize = AtomicUsize::new(0);
        let relay_id = format!(
            "exec-compatibility-{}",
            FIXTURE_ID.fetch_add(1, Ordering::Relaxed)
        );
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings.json");
        let settings = BackendSettings {
            active_relay_id: relay_id.clone(),
            relay_profiles: vec![RelayProfile {
                id: relay_id,
                name: "local exec fixture".to_string(),
                relay_mode: RelayMode::PureApi,
                protocol: RelayProtocol::Responses,
                base_url: format!("{}/v1", server.uri()),
                model: "gpt-5.6-luna".to_string(),
                model_list: "gpt-5.6-luna,gpt-5.6-sol".to_string(),
                no_auth: false,
                api_key: "local-fixture-key".to_string(),
                ..RelayProfile::default()
            }],
            ..BackendSettings::default()
        };
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

fn request(model: &str) -> Value {
    json!({
        "model": model,
        "stream": false,
        "store": false,
        "input": [{"type": "message", "role": "user", "content": "inspect the workspace"}],
        "tools": [
            {"type": "custom", "name": "exec", "description": "Execute Code Mode JavaScript",
             "format": {"type": "text"}},
            {"type": "custom", "name": "apply_patch", "description": "Apply a patch",
             "format": {"type": "text"}},
            {"type": "function", "name": "lookup", "description": "Search",
             "parameters": {"type": "object", "properties": {"query": {"type": "string"}},
                            "required": ["query"], "additionalProperties": false}}
        ],
        "tool_choice": {"type": "custom", "name": "exec"}
    })
}

fn rejection(status: u16, error_type: &str, message: &str, param: Value) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_json(json!({
        "error": {"type": error_type, "message": message, "param": param, "code": null}
    }))
}

fn exec_rejection() -> ResponseTemplate {
    rejection(
        400,
        "invalid_request_error",
        EXEC_REJECTION,
        json!("tools[0]"),
    )
}

fn response_with_item(item: Value) -> Value {
    json!({
        "id": "resp_exec_fixture",
        "object": "response",
        "status": "completed",
        "output": [
            {"type": "reasoning", "id": "rs_fixture", "summary": [],
             "encrypted_content": "opaque-reasoning"},
            item
        ],
        "usage": {"input_tokens": 1, "output_tokens": 1, "total_tokens": 2}
    })
}

fn function_response(arguments: &str, call_id: &str) -> Value {
    response_with_item(json!({
        "type": "function_call", "id": format!("fc_{call_id}"),
        "call_id": call_id, "name": "exec", "arguments": arguments, "status": "completed"
    }))
}

fn custom_response(call_id: &str) -> Value {
    response_with_item(json!({
        "type": "custom_tool_call", "id": format!("ctc_{call_id}"),
        "call_id": call_id, "name": "exec", "input": PROGRAM, "status": "completed"
    }))
}

fn is_custom_exec(body: &Value) -> bool {
    body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tool| tool["type"] == "custom" && tool["name"] == "exec")
}

fn assert_function_bridge(body: &Value, original: &Value) {
    let exec = &body["tools"][0];
    assert_eq!(exec["type"], "function");
    assert_eq!(exec["name"], "exec");
    assert!(
        exec.get("function").is_none(),
        "Responses tools must be flat"
    );
    assert_eq!(exec["parameters"]["type"], "object");
    assert_eq!(
        exec["parameters"]["properties"],
        json!({"input": {"type": "string"}})
    );
    assert_eq!(exec["parameters"]["required"], json!(["input"]));
    assert_eq!(exec["parameters"]["additionalProperties"], false);
    assert_eq!(body["tools"][1], original["tools"][1]);
    assert_eq!(body["tools"][2], original["tools"][2]);
    assert_eq!(
        body["tool_choice"],
        json!({"type": "function", "name": "exec"})
    );
}

async fn client_response(body: &Value) -> (ProxyHttpResponse, Value) {
    let response = handle_responses_proxy_request(&body.to_string())
        .await
        .unwrap();
    let json = serde_json::from_slice(&response.body).unwrap();
    (response, json)
}

fn assert_client_exec(response: &Value, program: &str, call_id: &str) {
    let exec = &response["output"][1];
    assert_eq!(exec["type"], "custom_tool_call");
    assert_eq!(exec["name"], "exec");
    assert_eq!(exec["call_id"], call_id);
    assert_eq!(exec["input"], program);
    assert!(exec.get("arguments").is_none());
    assert_eq!(
        response["output"][0]["encrypted_content"],
        "opaque-reasoning"
    );
}

#[tokio::test]
async fn explicit_custom_exec_rejection_retries_once_and_restores_raw_json_input() {
    let _lock = settings_path_test_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let server = MockServer::start().await;
    let _settings = TestSettings::new(&server);
    let attempts = Arc::new(AtomicUsize::new(0));
    let count = attempts.clone();
    Mock::given(method("POST"))
        .respond_with(move |_: &Request| {
            if count.fetch_add(1, Ordering::SeqCst) == 0 {
                exec_rejection()
            } else {
                ResponseTemplate::new(200).set_body_json(function_response(
                    &json!({"input": PROGRAM}).to_string(),
                    "call_first",
                ))
            }
        })
        .mount(&server)
        .await;

    let original = request("gpt-5.6-luna");
    let (response, json) = client_response(&original).await;
    assert_eq!(
        response.status, "200 OK",
        "the exact custom exec validation rejection must negotiate a function retry"
    );
    assert_client_exec(&json, PROGRAM, "call_first");
    let received = server.received_requests().await.unwrap();
    assert_eq!(received.len(), 2, "exactly one physical negotiation retry");
    assert_eq!(received[0].body_json::<Value>().unwrap(), original);
    assert_function_bridge(&received[1].body_json::<Value>().unwrap(), &original);
    assert_eq!(received[1].url.path(), "/v1/responses");
}

#[tokio::test]
async fn repeated_exact_validation_rejection_never_retries_more_than_once() {
    let _lock = settings_path_test_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let server = MockServer::start().await;
    let _settings = TestSettings::new(&server);
    Mock::given(method("POST"))
        .respond_with(exec_rejection())
        .mount(&server)
        .await;

    let original = request("other-model");
    let upstream = open_responses_proxy_request_with_settings_for_path(
        &original.to_string(),
        _settings.settings.clone(),
        "/v1/responses",
    )
    .await
    .unwrap();
    assert_eq!(upstream.status_code, 400);
    let error: Value = serde_json::from_slice(&upstream.response.bytes().await.unwrap()).unwrap();
    assert_eq!(error["error"]["message"], EXEC_REJECTION);
    let received = server.received_requests().await.unwrap();
    assert_eq!(
        received.len(),
        2,
        "negotiation has a whole-request retry budget"
    );
    assert_function_bridge(&received[1].body_json::<Value>().unwrap(), &original);
}

#[tokio::test]
async fn nonmatching_errors_do_not_trigger_exec_negotiation() {
    let _lock = settings_path_test_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    for (status, error_type, message, param) in [
        (
            400,
            "invalid_request_error",
            "Unsupported tools",
            Value::Null,
        ),
        (400, "server_error", EXEC_REJECTION, Value::Null),
        (
            400,
            "invalid_request_error",
            EXEC_REJECTION,
            json!("tools[1]"),
        ),
        (
            400,
            "invalid_request_error",
            "Unsupported custom tool: 'execute'. Only 'apply_patch' is supported.",
            Value::Null,
        ),
        (401, "invalid_request_error", EXEC_REJECTION, Value::Null),
        (429, "invalid_request_error", EXEC_REJECTION, Value::Null),
    ] {
        let server = MockServer::start().await;
        let settings = TestSettings::new(&server);
        Mock::given(method("POST"))
            .respond_with(rejection(status, error_type, message, param))
            .mount(&server)
            .await;
        let original = request("other-model");
        let upstream = open_responses_proxy_request_with_settings_for_path(
            &original.to_string(),
            settings.settings.clone(),
            "/v1/responses",
        )
        .await
        .unwrap();
        assert_eq!(
            upstream.status_code, status,
            "{status} {error_type}: {message}"
        );
        let error: Value =
            serde_json::from_slice(&upstream.response.bytes().await.unwrap()).unwrap();
        assert_eq!(error["error"]["message"], message);
        let received = server.received_requests().await.unwrap();
        assert_eq!(received.len(), 1, "{status} {error_type}: {message}");
        assert_eq!(received[0].body_json::<Value>().unwrap(), original);
    }
}

#[tokio::test]
async fn two_round_history_and_gpt_model_switch_keep_compatibility_isolated() {
    let _lock = settings_path_test_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let server = MockServer::start().await;
    let _settings = TestSettings::new(&server);
    let adapted_round = Arc::new(AtomicUsize::new(0));
    let round = adapted_round.clone();
    Mock::given(method("POST"))
        .respond_with(move |incoming: &Request| {
            let body = incoming.body_json::<Value>().unwrap();
            if body["model"] == "gpt-5.6-sol" {
                ResponseTemplate::new(200).set_body_json(custom_response("call_native"))
            } else if is_custom_exec(&body) {
                exec_rejection()
            } else {
                let call_id = format!("call_round_{}", round.fetch_add(1, Ordering::SeqCst));
                ResponseTemplate::new(200).set_body_json(function_response(
                    &json!({"input": PROGRAM}).to_string(),
                    &call_id,
                ))
            }
        })
        .mount(&server)
        .await;

    let native = request("gpt-5.6-sol");
    let (native_response, native_json) = client_response(&native).await;
    assert_eq!(native_response.status, "200 OK");
    assert_eq!(native_json, custom_response("call_native"));

    let first = request("gpt-5.6-luna");
    let (first_response, first_json) = client_response(&first).await;
    assert_eq!(first_response.status, "200 OK");
    assert_client_exec(&first_json, PROGRAM, "call_round_0");
    let patch_call = json!({
        "type": "custom_tool_call", "id": "ctc_patch", "call_id": "call_patch",
        "name": "apply_patch", "input": "*** Begin Patch\n*** End Patch"
    });
    let patch_output = json!({
        "type": "custom_tool_call_output", "id": "ctco_patch",
        "call_id": "call_patch", "output": "patch applied"
    });
    let mut second = first.clone();
    second["input"].as_array_mut().unwrap().extend([
        first_json["output"][0].clone(),
        first_json["output"][1].clone(),
        json!({"type": "custom_tool_call_output", "id": "ctco_round_0",
               "call_id": "call_round_0", "output": "lookup result"}),
        patch_call.clone(),
        patch_output.clone(),
        json!({"type": "message", "role": "user", "content": "continue"}),
    ]);
    let canonical = second.clone();
    let (second_response, second_json) = client_response(&second).await;
    assert_eq!(second_response.status, "200 OK");
    assert_client_exec(&second_json, PROGRAM, "call_round_1");
    assert_eq!(
        second, canonical,
        "canonical custom history remains untouched"
    );
    let (_, final_native_json) = client_response(&native).await;
    assert_eq!(final_native_json, custom_response("call_native"));

    let received = server.received_requests().await.unwrap();
    assert_eq!(
        received.len(),
        5,
        "second adapted round must use validated cache evidence"
    );
    assert_eq!(received[0].body_json::<Value>().unwrap(), native);
    assert_eq!(received[1].body_json::<Value>().unwrap(), first);
    assert_function_bridge(&received[2].body_json::<Value>().unwrap(), &first);
    let replay = received[3].body_json::<Value>().unwrap();
    assert_function_bridge(&replay, &second);
    let history = replay["input"].as_array().unwrap();
    assert_eq!(history[1], first_json["output"][0]);
    assert_eq!(history[2]["type"], "function_call");
    assert_eq!(history[2]["name"], "exec");
    assert_eq!(history[2]["call_id"], "call_round_0");
    assert_eq!(
        serde_json::from_str::<Value>(history[2]["arguments"].as_str().unwrap()).unwrap(),
        json!({"input": PROGRAM})
    );
    assert_eq!(history[3]["type"], "function_call_output");
    assert_eq!(history[3]["call_id"], "call_round_0");
    assert_eq!(history[3]["output"], "lookup result");
    assert_eq!(history[4], patch_call);
    assert_eq!(history[5], patch_output);
    assert_eq!(received[4].body_json::<Value>().unwrap(), native);
}

#[tokio::test]
async fn stateful_and_compaction_requests_bypass_negotiation_and_a_warm_cache() {
    let _lock = settings_path_test_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let server = MockServer::start().await;
    let settings = TestSettings::new(&server);
    Mock::given(method("POST"))
        .respond_with(|incoming: &Request| {
            let body = incoming.body_json::<Value>().unwrap();
            if is_custom_exec(&body) {
                exec_rejection()
            } else {
                ResponseTemplate::new(200).set_body_json(function_response(
                    &json!({"input": PROGRAM}).to_string(),
                    "call_cached",
                ))
            }
        })
        .mount(&server)
        .await;
    let base = request("gpt-5.6-luna");
    let (seed, _) = client_response(&base).await;
    assert_eq!(seed.status, "200 OK");

    let mut previous = base.clone();
    previous["previous_response_id"] = json!("resp_server_history");
    let mut conversation = base.clone();
    conversation["conversation"] = json!({"id": "conv_server_history"});
    let mut trigger = base.clone();
    trigger["input"]
        .as_array_mut()
        .unwrap()
        .push(json!({"type": "compaction_trigger"}));
    let mut compact_history = base.clone();
    compact_history["input"]
        .as_array_mut()
        .unwrap()
        .push(json!({"type": "compaction", "id": "cmp_history",
                     "encrypted_content": "opaque-summary"}));
    let cases = [
        (previous, "/v1/responses"),
        (conversation, "/v1/responses"),
        (trigger, "/v1/responses"),
        (compact_history, "/v1/responses"),
        (base.clone(), "/v1/responses/compact"),
    ];
    for (body, path) in &cases {
        let upstream = open_responses_proxy_request_with_settings_for_path(
            &body.to_string(),
            settings.settings.clone(),
            path,
        )
        .await
        .unwrap();
        assert_eq!(upstream.status_code, 400, "{path}: {body}");
        let error: Value =
            serde_json::from_slice(&upstream.response.bytes().await.unwrap()).unwrap();
        assert_eq!(error["error"]["message"], EXEC_REJECTION);
    }
    let received = server.received_requests().await.unwrap();
    assert_eq!(received.len(), 2 + cases.len());
    for (incoming, (expected, _)) in received[2..].iter().zip(cases) {
        assert_eq!(incoming.body_json::<Value>().unwrap(), expected);
    }
    assert_eq!(received.last().unwrap().url.path(), "/v1/responses/compact");
}

#[tokio::test]
async fn malformed_adapted_arguments_never_become_executable_custom_input() {
    let _lock = settings_path_test_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    for arguments in [
        r#"{"input":"safe","input":"injected"}"#,
        r#"{"input":"program","extra":true}"#,
        r#"{"input":42}"#,
        r#"{"input":"truncated"#,
    ] {
        let server = MockServer::start().await;
        let _settings = TestSettings::new(&server);
        let arguments = arguments.to_string();
        Mock::given(method("POST"))
            .respond_with(move |incoming: &Request| {
                let body = incoming.body_json::<Value>().unwrap();
                if is_custom_exec(&body) {
                    exec_rejection()
                } else {
                    ResponseTemplate::new(200)
                        .set_body_json(function_response(&arguments, "call_invalid"))
                }
            })
            .mount(&server)
            .await;
        let response = handle_responses_proxy_request(&request("gpt-5.6-luna").to_string())
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&response.body).unwrap();
        assert!(
            response.status != "200 OK" || body["status"] == "failed",
            "invalid arguments must fail the adapted response: {body}"
        );
        assert!(
            body["output"].as_array().is_none_or(|items| {
                items.iter().all(|item| item["type"] != "custom_tool_call")
            }),
            "unvalidated arguments must never be exposed as executable raw input"
        );
        assert_eq!(
            server.received_requests().await.unwrap().len(),
            2,
            "a parser failure must not retry or produce compatibility cache evidence"
        );
    }
}
