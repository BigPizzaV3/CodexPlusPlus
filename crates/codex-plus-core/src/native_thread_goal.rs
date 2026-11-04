use serde::Deserialize;
use serde_json::{Value, json};

const NATIVE_RUNTIME_VERSION: u32 = 1;
const NATIVE_RUNTIME_REVISION: u32 = 2;
const NATIVE_ERROR_CODES: &[&str] = &[
    "goal-native-getter-ambiguous",
    "goal-owner-changed",
    "goal-request-id-conflict",
    "goal-request-limit",
    "goal-runtime-reload-required",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GoalForwardError {
    InvalidRequest,
    EnhancementsDisabled,
    ProbeFailed,
    OwnerUnavailable,
    NativeGetterAmbiguous,
    OwnerChanged,
    RequestIdConflict,
    RequestLimit,
    RuntimeReloadRequired,
    ForwardFailed,
}

impl GoalForwardError {
    fn from_native_code(code: Option<&str>) -> Self {
        match code {
            Some("goal-native-getter-ambiguous") => Self::NativeGetterAmbiguous,
            Some("goal-owner-changed") => Self::OwnerChanged,
            Some("goal-request-id-conflict") => Self::RequestIdConflict,
            Some("goal-request-limit") => Self::RequestLimit,
            Some("goal-runtime-reload-required") => Self::RuntimeReloadRequired,
            _ => Self::ForwardFailed,
        }
    }

    fn code(self) -> &'static str {
        match self {
            Self::InvalidRequest => "goal-invalid-request",
            Self::EnhancementsDisabled => "goal-enhancements-disabled",
            Self::ProbeFailed => "goal-probe-failed",
            Self::OwnerUnavailable => "goal-owner-unavailable",
            Self::NativeGetterAmbiguous => "goal-native-getter-ambiguous",
            Self::OwnerChanged => "goal-owner-changed",
            Self::RequestIdConflict => "goal-request-id-conflict",
            Self::RequestLimit => "goal-request-limit",
            Self::RuntimeReloadRequired => "goal-runtime-reload-required",
            Self::ForwardFailed => "goal-forward-failed",
        }
    }

    fn message(self) -> &'static str {
        match self {
            Self::InvalidRequest => "会话目标请求无效，请重新设置目标",
            Self::EnhancementsDisabled => "Codex++ 增强已关闭",
            Self::RuntimeReloadRequired => "会话目标组件已更新，请刷新或重新打开此窗口后重试",
            _ => "会话所在窗口暂不可用，请稍后重试设置目标",
        }
    }

    pub(crate) fn response(self) -> Value {
        json!({ "status": "failed", "code": self.code(), "message": self.message() })
    }
}

impl std::fmt::Display for GoalForwardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for GoalForwardError {}

pub(crate) fn failed_response(error: &anyhow::Error) -> Value {
    error.downcast_ref::<GoalForwardError>().copied()
        .unwrap_or(GoalForwardError::ForwardFailed).response()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GoalRequest {
    host_id: String,
    thread_id: String,
    request_id: String,
    params: Value,
}

fn validate_request(payload: &Value) -> anyhow::Result<()> {
    let request: GoalRequest = serde_json::from_value(payload.clone())
        .map_err(|_| GoalForwardError::InvalidRequest)?;
    for value in [&request.host_id, &request.thread_id, &request.request_id] {
        anyhow::ensure!(!value.trim().is_empty() && value.len() <= 256, GoalForwardError::InvalidRequest);
    }
    // 参数由原生接口解释，避免拦截清空目标或未来新增字段。
    anyhow::ensure!(request.params.is_object(), GoalForwardError::InvalidRequest);
    Ok(())
}

fn is_app_window(target: &crate::cdp::CdpTarget) -> bool {
    target.target_type == "page"
        && target.web_socket_debugger_url.is_some()
        && reqwest::Url::parse(&target.url).is_ok_and(|url| {
            url.scheme() == "app" && url.host_str() == Some("-") && url.path() == "/index.html"
        })
}

async fn evaluate(websocket: &str, method: &str, payload: &Value) -> anyhow::Result<Value> {
    let timeout = std::time::Duration::from_secs(if method == "execute" { 20 } else { 3 });
    evaluate_with_timeout(websocket, method, payload, timeout).await
}

async fn evaluate_with_timeout(
    websocket: &str,
    method: &str,
    payload: &Value,
    timeout: std::time::Duration,
) -> anyhow::Result<Value> {
    let script = evaluation_script(method, payload)?;
    // 底层 CDP 错误可能包含 URL、调用栈及异常原文，不能送到原生界面。
    let response = crate::bridge::evaluate_script_with_timeout(websocket, &script, true, timeout)
        .await.map_err(|_| GoalForwardError::ForwardFailed)?;
    let text = response.pointer("/result/result/value").and_then(Value::as_str)
        .ok_or(GoalForwardError::ForwardFailed)?;
    decode_evaluation(text)
}

fn evaluation_script(method: &str, payload: &Value) -> anyhow::Result<String> {
    // 原始 Runtime.evaluate 默认不序列化对象；异步完成后显式编码，不能把 Promise 当成功。
    let argument = serde_json::to_string(&payload.to_string())
        .map_err(|_| GoalForwardError::ForwardFailed)?;
    let codes = serde_json::to_string(NATIVE_ERROR_CODES)
        .map_err(|_| GoalForwardError::ForwardFailed)?;
    Ok(format!(
        "{}\n;(async()=>{{try{{const runtime=window.__codexPlusNativeThreadGoal;if(runtime?.version!=={}||runtime?.revision!=={})return JSON.stringify({{ok:false,code:'goal-runtime-reload-required'}});return JSON.stringify({{ok:true,value:await runtime.{}(JSON.parse({}))}})}}catch(error){{const code=error?.code;return JSON.stringify({{ok:false,code:{}.includes(code)?code:null}})}}}})()",
        crate::assets::NATIVE_THREAD_GOAL_SCRIPT, NATIVE_RUNTIME_VERSION,
        NATIVE_RUNTIME_REVISION, method, argument, codes,
    ))
}

fn decode_evaluation(text: &str) -> anyhow::Result<Value> {
    let envelope: Value = serde_json::from_str(text).map_err(|_| GoalForwardError::ForwardFailed)?;
    match envelope.get("ok").and_then(Value::as_bool) {
        Some(true) => envelope.get("value").cloned().ok_or_else(|| GoalForwardError::ForwardFailed.into()),
        Some(false) => Err(GoalForwardError::from_native_code(
            envelope.get("code").and_then(Value::as_str),
        ).into()),
        None => Err(GoalForwardError::ForwardFailed.into()),
    }
}

/// 只在指定 Codex 实例的唯一原生 owner 窗口中设置目标。
pub async fn set_in_owner(debug_port: u16, payload: Value) -> anyhow::Result<Value> {
    validate_request(&payload)?;
    let probe = json!({ "hostId": payload["hostId"], "threadId": payload["threadId"] });
    let targets = crate::cdp::list_targets(debug_port).await
        .map_err(|_| GoalForwardError::ProbeFailed)?;
    let mut owners = Vec::new();
    let candidates: Vec<_> = targets.iter().filter(|target| is_app_window(target))
        .filter_map(|target| target.web_socket_debugger_url.as_deref()).collect();
    let results = futures_util::future::join_all(candidates.iter().map(|websocket| {
        evaluate(websocket, "probe", &probe)
    })).await;
    for (websocket, result) in candidates.into_iter().zip(results) {
        match result {
            Ok(result) if result["status"] == "owner" => owners.push(websocket),
            Ok(_) => {}
            Err(error) => {
                let kind = error.downcast_ref::<GoalForwardError>().copied();
                return Err(match kind {
                    Some(GoalForwardError::NativeGetterAmbiguous) => GoalForwardError::NativeGetterAmbiguous,
                    Some(GoalForwardError::RuntimeReloadRequired) => GoalForwardError::RuntimeReloadRequired,
                    _ => GoalForwardError::ProbeFailed,
                }.into());
            }
        }
    }
    anyhow::ensure!(owners.len() == 1, GoalForwardError::OwnerUnavailable);
    // CDP 超时不会取消已经执行的 JS；写入只发给这个 owner 一次，禁止改投其他窗口。
    let result = evaluate(owners[0], "execute", &payload).await?;
    match result.get("status").and_then(Value::as_str) {
        Some("ok") => Ok(result),
        Some("failed") => Err(GoalForwardError::from_native_code(
            result.get("code").and_then(Value::as_str),
        ).into()),
        _ => Err(GoalForwardError::ForwardFailed.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio_tungstenite::tungstenite::{
        Message, handshake::derive_accept_key, protocol::Role,
    };

    #[derive(Clone)]
    enum FixtureReply {
        Json(Value),
        Rejected(Option<String>),
        Disconnect,
        Exception,
        Hang,
    }

    struct FixtureWindow {
        id: String,
        url: &'static str,
        probe: FixtureReply,
        execute: FixtureReply,
    }

    fn window(id: &str, status: &str) -> FixtureWindow {
        FixtureWindow {
            id: id.to_string(),
            url: "app://-/index.html",
            probe: FixtureReply::Json(json!({ "status": status })),
            execute: FixtureReply::Json(json!({ "status": "ok" })),
        }
    }

    #[derive(Clone, Debug)]
    struct ObservedCommand {
        target: String,
        operation: String,
        payload: Value,
        params: Value,
    }

    struct FakeCdp {
        port: u16,
        commands: Arc<Mutex<Vec<ObservedCommand>>>,
        task: Option<tokio::task::JoinHandle<()>>,
    }

    impl FakeCdp {
        async fn start(windows: Vec<FixtureWindow>) -> Self {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let targets = Value::Array(windows.iter().map(|window| json!({
                "id": window.id,
                "type": "page",
                "title": "",
                "url": window.url,
                "webSocketDebuggerUrl": format!(
                    "ws://127.0.0.1:{port}/devtools/page/{}", window.id
                ),
            })).collect()).to_string();
            let commands = Arc::new(Mutex::new(Vec::new()));
            let observed = Arc::clone(&commands);
            let task = tokio::spawn(async move {
                loop {
                    let (mut stream, _) = listener.accept().await.unwrap();
                    let mut header = Vec::new();
                    while !header.ends_with(b"\r\n\r\n") {
                        header.push(stream.read_u8().await.unwrap());
                        assert!(header.len() <= 16_384);
                    }
                    let header = String::from_utf8(header).unwrap();
                    let path = header.lines().next().unwrap()
                        .split_whitespace().nth(1).unwrap();
                    if path == "/json" {
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{targets}",
                            targets.len(),
                        );
                        stream.write_all(response.as_bytes()).await.unwrap();
                        continue;
                    }
                    let target = path.strip_prefix("/devtools/page/").unwrap();
                    let window = windows.iter().find(|window| window.id == target).unwrap();
                    let key = header.lines().find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("sec-websocket-key").then(|| value.trim())
                    }).unwrap();
                    let response = format!(
                        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
                        derive_accept_key(key.as_bytes()),
                    );
                    stream.write_all(response.as_bytes()).await.unwrap();
                    let mut socket = tokio_tungstenite::WebSocketStream::from_raw_socket(
                        stream, Role::Server, None,
                    ).await;
                    let message = socket.next().await.unwrap().unwrap();
                    let command: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
                    assert_eq!(command["method"], "Runtime.evaluate");
                    let expression = command["params"]["expression"].as_str().unwrap();
                    let call = expression.rsplit_once("await runtime.").unwrap().1;
                    let (operation, arguments) = call.split_once('(').unwrap();
                    let encoded = arguments.strip_prefix("JSON.parse(").unwrap();
                    let json_text = serde_json::Deserializer::from_str(encoded)
                        .into_iter::<String>().next().unwrap().unwrap();
                    let payload = serde_json::from_str(&json_text).unwrap();
                    observed.lock().unwrap().push(ObservedCommand {
                        target: target.to_string(),
                        operation: operation.to_string(),
                        payload,
                        params: command["params"].clone(),
                    });
                    let reply = match operation {
                        "probe" => window.probe.clone(),
                        "execute" => window.execute.clone(),
                        _ => panic!("unexpected operation: {operation}"),
                    };
                    let result = match reply {
                        FixtureReply::Json(value) => json!({
                            "result": {
                                "type": "string",
                                "value": serde_json::to_string(&json!({"ok":true,"value":value})).unwrap(),
                            }
                        }),
                        FixtureReply::Rejected(code) => json!({
                            "result": {
                                "type": "string",
                                "value": serde_json::to_string(&json!({"ok":false,"code":code})).unwrap(),
                            }
                        }),
                        FixtureReply::Disconnect => continue,
                        FixtureReply::Exception => json!({
                            "result": { "type": "undefined" },
                            "exceptionDetails": {
                                "text": "secret-token: native setter rejected",
                                "exception": {"description": "Error at app://-/secret/path.js:123\nsecret-stack"},
                            },
                        }),
                        FixtureReply::Hang => {
                            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                            continue;
                        }
                    };
                    let response = json!({ "id": command["id"], "result": result });
                    socket.send(Message::Text(response.to_string().into())).await.unwrap();
                }
            });
            Self { port, commands, task: Some(task) }
        }

        fn commands(&self) -> Vec<ObservedCommand> {
            self.commands.lock().unwrap().clone()
        }

        fn websocket(&self, target: &str) -> String {
            format!("ws://127.0.0.1:{}/devtools/page/{target}", self.port)
        }

        async fn stop(mut self) {
            let task = self.task.take().unwrap();
            task.abort();
            let error = task.await.unwrap_err();
            assert!(error.is_cancelled(), "fixture failed: {error}");
        }
    }

    impl Drop for FakeCdp {
        fn drop(&mut self) {
            if let Some(task) = &self.task {
                task.abort();
            }
        }
    }

    fn goal_payload() -> Value {
        json!({
            "hostId": "local",
            "threadId": "thread",
            "requestId": "request",
            "params": {
                "objective": "test \"goal\"\nsecond line",
                "appendTranscriptItem": false,
                "threadSettings": { "model": "fixture-model" },
            },
        })
    }

    #[tokio::test]
    async fn no_owner_never_executes() {
        let fixture = FakeCdp::start(vec![
            window("first", "follower"),
            window("second", "missing"),
        ]).await;
        let error = set_in_owner(fixture.port, goal_payload()).await.unwrap_err();
        assert_eq!(failed_response(&error)["code"], "goal-owner-unavailable");
        let commands = fixture.commands();
        assert_eq!(commands.len(), 2);
        assert!(commands.iter().all(|command| command.operation == "probe"));
        fixture.stop().await;
    }

    #[tokio::test]
    async fn multiple_owners_never_execute() {
        let fixture = FakeCdp::start(vec![
            window("first", "owner"),
            window("second", "owner"),
        ]).await;
        let error = set_in_owner(fixture.port, goal_payload()).await.unwrap_err();
        assert_eq!(failed_response(&error)["code"], "goal-owner-unavailable");
        let commands = fixture.commands();
        assert_eq!(commands.len(), 2);
        assert!(commands.iter().all(|command| command.operation == "probe"));
        fixture.stop().await;
    }

    #[tokio::test]
    async fn unique_hidden_spotlight_owner_executes_once_with_original_payload() {
        let mut spotlight = window("spotlight", "owner");
        spotlight.url = "app://-/index.html?initialRoute=%2Fspotlight";
        let mut external = window("external", "owner");
        external.url = "https://codex.example/index.html";
        let fixture = FakeCdp::start(vec![
            window("main", "follower"), spotlight, external,
        ]).await;
        let mut payload = goal_payload();
        payload["params"]["objective"] = json!("");
        payload["params"]["futureOptions"] = json!({"nested": [true, null, "value"]});
        payload["params"]["__proto__"] = json!({"keep": "own property"});
        assert_eq!(
            set_in_owner(fixture.port, payload.clone()).await.unwrap(),
            json!({ "status": "ok" }),
        );
        let commands = fixture.commands();
        assert_eq!(commands.len(), 3);
        assert!(commands.iter().all(|command| command.target != "external"));
        for command in commands.iter().filter(|command| command.operation == "probe") {
            assert_eq!(command.payload, json!({ "hostId": "local", "threadId": "thread" }));
        }
        let executed: Vec<_> = commands.iter()
            .filter(|command| command.operation == "execute").collect();
        assert_eq!(executed.len(), 1);
        assert_eq!(executed[0].target, "spotlight");
        assert_eq!(executed[0].payload, payload);
        assert!(commands.iter().all(|command| command.params["awaitPromise"] == true));
        fixture.stop().await;
    }

    #[tokio::test]
    async fn probe_disconnect_prevents_all_execution() {
        let mut unreachable = window("unreachable", "follower");
        unreachable.probe = FixtureReply::Disconnect;
        let fixture = FakeCdp::start(vec![
            window("owner", "owner"), unreachable,
        ]).await;
        let error = set_in_owner(fixture.port, goal_payload()).await.unwrap_err();
        assert_eq!(failed_response(&error)["code"], "goal-probe-failed");
        let commands = fixture.commands();
        assert_eq!(commands.len(), 2);
        assert!(commands.iter().all(|command| command.operation == "probe"));
        fixture.stop().await;
    }

    #[tokio::test]
    async fn awaited_json_string_results_preserve_primitives_and_structured_values() {
        let values = [
            Value::Null,
            json!(true),
            json!(42),
            json!("quoted \"result\"\nsecond line"),
            json!([1, null, "result"]),
            json!({ "status": "owner", "nested": { "ok": true } }),
        ];
        let windows = values.iter().enumerate().map(|(index, value)| {
            let mut window = window(&index.to_string(), "owner");
            window.execute = FixtureReply::Json(value.clone());
            window
        }).collect();
        let fixture = FakeCdp::start(windows).await;
        for (index, expected) in values.iter().enumerate() {
            let actual = evaluate(
                &fixture.websocket(&index.to_string()), "execute", &goal_payload(),
            ).await.unwrap();
            assert_eq!(&actual, expected);
        }
        let commands = fixture.commands();
        assert_eq!(commands.len(), values.len());
        assert!(commands.iter().all(|command| command.params["awaitPromise"] == true));
        fixture.stop().await;
    }

    #[tokio::test]
    async fn failed_execution_never_retries_or_moves_to_another_window() {
        for failure in [FixtureReply::Disconnect, FixtureReply::Exception] {
            let mut owner = window("owner", "owner");
            owner.execute = failure;
            let fixture = FakeCdp::start(vec![
                window("follower", "follower"), owner,
            ]).await;
            let error = set_in_owner(fixture.port, goal_payload()).await.unwrap_err();
            assert_eq!(failed_response(&error), json!({
                "status": "failed", "code": "goal-forward-failed",
                "message": "会话所在窗口暂不可用，请稍后重试设置目标"
            }));
            assert!(!error.to_string().contains("secret"));
            let commands = fixture.commands();
            assert_eq!(commands.len(), 3);
            let executed: Vec<_> = commands.iter()
                .filter(|command| command.operation == "execute").collect();
            assert_eq!(executed.len(), 1);
            assert_eq!(executed[0].target, "owner");
            fixture.stop().await;
        }
    }

    #[tokio::test]
    async fn known_native_error_codes_are_preserved_and_unknown_codes_are_redacted() {
        for (native_code, expected) in [
            (Some("goal-owner-changed"), "goal-owner-changed"),
            (Some("goal-request-limit"), "goal-request-limit"),
            (Some("goal-request-id-conflict"), "goal-request-id-conflict"),
            (Some("secret-token app://-/private.js"), "goal-forward-failed"),
            (None, "goal-forward-failed"),
        ] {
            let mut owner = window("owner", "owner");
            owner.execute = FixtureReply::Rejected(native_code.map(str::to_string));
            let fixture = FakeCdp::start(vec![window("follower", "follower"), owner]).await;
            let error = set_in_owner(fixture.port, goal_payload()).await.unwrap_err();
            let response = failed_response(&error);
            assert_eq!(response["code"], expected);
            assert_eq!(response["message"], "会话所在窗口暂不可用，请稍后重试设置目标");
            assert!(!response.to_string().contains("secret"));
            let commands = fixture.commands();
            assert_eq!(commands.iter().filter(|command| command.operation == "execute").count(), 1);
            fixture.stop().await;
        }
    }

    #[tokio::test]
    async fn ambiguous_getter_probe_preserves_its_code_without_executing() {
        let mut ambiguous = window("ambiguous", "follower");
        ambiguous.probe = FixtureReply::Rejected(Some("goal-native-getter-ambiguous".to_string()));
        let fixture = FakeCdp::start(vec![window("owner", "owner"), ambiguous]).await;
        let error = set_in_owner(fixture.port, goal_payload()).await.unwrap_err();
        assert_eq!(failed_response(&error)["code"], "goal-native-getter-ambiguous");
        assert!(fixture.commands().iter().all(|command| command.operation == "probe"));
        fixture.stop().await;
    }

    #[tokio::test]
    async fn outdated_runtime_probe_reports_that_a_refresh_is_required() {
        let mut outdated = window("outdated", "follower");
        outdated.probe = FixtureReply::Rejected(Some("goal-runtime-reload-required".to_string()));
        let fixture = FakeCdp::start(vec![window("owner", "owner"), outdated]).await;
        let error = set_in_owner(fixture.port, goal_payload()).await.unwrap_err();
        assert_eq!(failed_response(&error), json!({
            "status": "failed", "code": "goal-runtime-reload-required",
            "message": "会话目标组件已更新，请刷新或重新打开此窗口后重试"
        }));
        assert!(fixture.commands().iter().all(|command| command.operation == "probe"));
        fixture.stop().await;
    }

    #[tokio::test]
    async fn cdp_timeout_is_a_generic_failure_without_transport_details() {
        let mut owner = window("owner", "owner");
        owner.execute = FixtureReply::Hang;
        let fixture = FakeCdp::start(vec![owner]).await;
        let error = evaluate_with_timeout(
            &fixture.websocket("owner"), "execute", &goal_payload(),
            std::time::Duration::from_millis(100),
        ).await.unwrap_err();
        assert_eq!(failed_response(&error), json!({
            "status": "failed", "code": "goal-forward-failed",
            "message": "会话所在窗口暂不可用，请稍后重试设置目标"
        }));
        assert!(!error.to_string().contains("127.0.0.1"));
        assert_eq!(fixture.commands().len(), 1);
        fixture.stop().await;
    }

    #[test]
    fn actual_javascript_rejects_unknown_runtime_without_invoking_it() {
        let harness = r#"
const vm = require('node:vm');
const [expression, version, revision] = process.argv.slice(1);
const calls = {probe:0, execute:0};
const runtime = {
  probe() { calls.probe++; return {status:'owner'}; },
  execute() { calls.execute++; return {status:'ok', goal:{objective:'test'}}; },
};
if (version !== 'missing') runtime.version = JSON.parse(version);
if (revision !== 'missing') runtime.revision = JSON.parse(revision);
Object.freeze(runtime);
const context = vm.createContext({window:{__codexPlusNativeThreadGoal:runtime}});
(async () => {
  const result = await vm.runInContext(expression, context);
  process.stdout.write(JSON.stringify({result, calls}));
})().catch(error => { process.stderr.write(String(error.stack || error)); process.exitCode = 1; });
"#;
        let current_version = NATIVE_RUNTIME_VERSION.to_string();
        let current_revision = NATIVE_RUNTIME_REVISION.to_string();
        for method in ["probe", "execute"] {
            for (version, revision, accepted) in [
                ("1", "missing", false),
                ("1", "null", false),
                ("1", "1", false),
                ("1", "3", false),
                ("2", "2", false),
                ("missing", "2", false),
                (current_version.as_str(), current_revision.as_str(), true),
            ] {
                let script = evaluation_script(method, &goal_payload()).unwrap();
                let output = std::process::Command::new("node")
                    .arg("-e").arg(harness).arg(script).arg(version).arg(revision)
                    .output().unwrap();
                assert!(output.status.success(), "{method} {version}/{revision}: {}", String::from_utf8_lossy(&output.stderr));
                let outcome: Value = serde_json::from_slice(&output.stdout).unwrap();
                let result = decode_evaluation(outcome["result"].as_str().unwrap());
                if accepted {
                    assert!(result.is_ok(), "{method} {version}/{revision}");
                    assert_eq!(outcome["calls"][method], 1);
                } else {
                    assert_eq!(failed_response(&result.unwrap_err())["code"], "goal-runtime-reload-required");
                    assert_eq!(outcome["calls"]["probe"], 0);
                    assert_eq!(outcome["calls"]["execute"], 0);
                }
            }
        }
    }

    #[test]
    fn actual_javascript_errors_cross_the_rust_result_contract() {
        // 与前端同样使用 Node，执行 Rust 真正发送的表达式与完整原生脚本。
        let harness = r#"
const vm = require('node:vm');
const expression = process.argv[1], mode = process.argv[2];
let role = 'owner';
class Manager {
  getHostId() { return 'local'; }
  getConversation(id) { return id === 'thread' ? {id} : null; }
  getStreamRole() { return {role}; }
  setThreadGoal() {
    if (mode === 'native-error') throw new Error('secret-token at app://-/private.js\nsecret-stack');
    if (mode === 'limit' || mode === 'conflict') return new Promise(() => {});
    return {objective:'success'};
  }
}
const manager = new Manager(), identity = {};
const signal = {kind:'signal-family', scope:identity}, list = {};
const scope = {scope:identity, get(key) { return key === signal ? manager : [manager]; }};
function getter(e, t) {
  if (!t) return null;
  const m = e.get(signal);
  return m.getConversation(t) ? m : e.get(list).find(x => x.getConversation(t));
}
function otherGetter(e, t) {
  if (!t) return null;
  const m = e.get(signal);
  return m.getConversation(t) ? m : e.get(list).find(x => x.getConversation(t));
}
const module = {signal, getter, ...(mode === 'ambiguous' ? {otherGetter} : {})};
const fiber = {memoizedState:{memoizedState:{current:scope}, next:null}};
const root = {__reactContainer$fixture:{stateNode:{current:fiber}}};
const context = vm.createContext({
  window:{location:new URL('app://-/index.html')},
  document:{getElementById:() => root, body:{}, documentElement:{}, querySelectorAll:() => []},
  performance:{getEntriesByType:() => []}, URL,
});
const marker = '\n;(async()=>{try{';
const split = expression.lastIndexOf(marker);
if (split < 0) throw new Error('missing evaluation wrapper');
vm.runInContext(expression.slice(0, split), context);
(async () => {
  const api = context.window.__codexPlusNativeThreadGoal;
  try {
    await api.install('thread', {hostId:'local', loadModule:async () => module,
      enabled:() => true, forward:async () => ({}), report:() => {}});
  } catch (error) { if (mode !== 'ambiguous') throw error; }
  if (mode === 'owner-changed') role = 'follower';
  if (mode === 'limit' || mode === 'conflict') {
    const count = mode === 'limit' ? 128 : 1;
    for (let i = 0; i < count; i++) void api.execute({hostId:'local', threadId:'thread',
      requestId: mode === 'conflict' ? 'request' : `pending-${i}`, params:{objective:'pending'}});
    await new Promise(setImmediate);
  }
  process.stdout.write(await vm.runInContext(expression, context));
})().catch(error => { process.stderr.write(String(error.stack || error)); process.exitCode = 1; });
"#;
        for (mode, method, expected_code) in [
            ("owner-changed", "execute", "goal-owner-changed"),
            ("limit", "execute", "goal-request-limit"),
            ("conflict", "execute", "goal-request-id-conflict"),
            ("ambiguous", "probe", "goal-native-getter-ambiguous"),
            ("native-error", "execute", "goal-forward-failed"),
        ] {
            let script = evaluation_script(method, &goal_payload()).unwrap();
            let output = std::process::Command::new("node")
                .arg("-e").arg(harness).arg(script).arg(mode).output().unwrap();
            assert!(output.status.success(), "{mode}: {}", String::from_utf8_lossy(&output.stderr));
            let text = String::from_utf8(output.stdout).unwrap();
            let error = decode_evaluation(&text).unwrap_err();
            let response = failed_response(&error);
            assert_eq!(response["code"], expected_code, "{mode}");
            assert!(!response.to_string().contains("secret"));
            assert!(!response.to_string().contains("app://"));
        }
    }

    #[tokio::test]
    async fn returned_failures_only_preserve_known_codes() {
        for (code, expected) in [
            ("goal-owner-changed", "goal-owner-changed"),
            ("private error and stack", "goal-forward-failed"),
        ] {
            let mut owner = window("owner", "owner");
            owner.execute = FixtureReply::Json(json!({
                "status": "failed", "code": code, "message": "secret-token app://-/private.js"
            }));
            let fixture = FakeCdp::start(vec![owner]).await;
            let error = set_in_owner(fixture.port, goal_payload()).await.unwrap_err();
            let response = failed_response(&error);
            assert_eq!(response["code"], expected);
            assert!(!response.to_string().contains("secret"));
            assert!(!response.to_string().contains("private"));
            fixture.stop().await;
        }
    }

    #[test]
    fn accepts_extensible_goal_parameters_but_validates_the_envelope() {
        let payload = json!({"hostId":"local","threadId":"thread","requestId":"request",
            "params":{"objective":"test","appendTranscriptItem":false,"threadSettings":null}});
        assert!(validate_request(&payload).is_ok());
        for params in [
            json!({"objective":""}),
            json!({"objective":null,"newField":{"nested":[1, true, null]}}),
            json!({"futureField":42,"threadSettings":[]}),
            json!({}),
        ] {
            let mut extended = payload.clone();
            extended["params"] = params;
            assert!(validate_request(&extended).is_ok());
        }
        for invalid in [
            json!({"params":{"objective":"test"}}),
            json!({"hostId":"local","threadId":"thread","requestId":"request","params":[]}),
            json!({"hostId":"local","threadId":"thread","requestId":"request","params":null}),
            json!({"hostId":"  ","threadId":"thread","requestId":"request","params":{}}),
        ] {
            assert!(validate_request(&invalid).is_err());
        }
        let mut arbitrary = payload.clone();
        arbitrary["expression"] = json!("anything");
        assert!(validate_request(&arbitrary).is_err());
    }

    #[test]
    fn filters_actual_app_windows_including_spotlight() {
        for (url, expected) in [
            ("app://-/index.html", true),
            ("app://-/index.html?initialRoute=%2Fspotlight", true),
            ("https://codex.example/index.html", false),
            ("app://other/index.html", false),
            ("app://-/browser.html", false),
        ] {
            let target: crate::cdp::CdpTarget = serde_json::from_value(json!({
                "id":"test","type":"page","title":"Codex","url":url,
                "webSocketDebuggerUrl":"ws://127.0.0.1/test"
            })).unwrap();
            assert_eq!(is_app_window(&target), expected, "{url}");
        }
    }
}
