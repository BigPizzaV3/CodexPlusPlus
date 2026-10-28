//! 网关模式端到端：真实 helper + 模拟上游，覆盖 Claude Code / Grok 经网关的完整链路。
//!
//! 常驻 helper 是进程级单例、状态目录靠进程级环境变量覆盖，所以全部场景放在一个测试里顺序执行。

use std::sync::{Arc, Mutex};

use codex_plus_core::gateway::store::{
    AgentBinding, ClaudeRoleModels, GatewayGroup, GatewayProvider, GatewaySettings, GatewayStore,
    GroupMember, GroupMode,
};
use codex_plus_core::settings::{RelayApiKey, RelayProfile, RelayProtocol};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

type Captured = Arc<Mutex<Vec<(String, Value)>>>;

/// 起一个模拟上游：按顺序回放 `responses`（用完后重复最后一个），记录每个请求的 (请求头, 请求体)。
async fn mock_upstream(responses: Vec<(u16, &'static str, String)>) -> (String, Captured) {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let captured: Captured = Arc::default();
    let sink = captured.clone();
    tokio::spawn(async move {
        let mut index = 0;
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let (head, body) = read_request(&mut stream).await;
            sink.lock().unwrap().push((head, body));
            let (status, content_type, body) = responses[index.min(responses.len() - 1)].clone();
            index += 1;
            let response = format!(
                "HTTP/1.1 {status} X\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.shutdown().await;
        }
    });
    (format!("http://{address}/v1"), captured)
}

async fn read_request(stream: &mut tokio::net::TcpStream) -> (String, Value) {
    let mut buffer = Vec::new();
    let mut chunk = [0; 8192];
    let (header_end, length) = loop {
        let read = stream.read(&mut chunk).await.unwrap();
        assert!(read > 0);
        buffer.extend_from_slice(&chunk[..read]);
        let Some(end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        let head = String::from_utf8_lossy(&buffer[..end]).to_string();
        let length = head
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .unwrap_or(0);
        break (end + 4, length);
    };
    while buffer.len() < header_end + length {
        let read = stream.read(&mut chunk).await.unwrap();
        assert!(read > 0);
        buffer.extend_from_slice(&chunk[..read]);
    }
    let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let body =
        serde_json::from_slice(&buffer[header_end..header_end + length]).unwrap_or(Value::Null);
    (head, body)
}

fn relay(
    id: &str,
    base_url: &str,
    key: &str,
    protocol: RelayProtocol,
    models: &str,
) -> RelayProfile {
    let mut profile = codex_plus_core::provider_import::pool_relay_profile(
        id,
        base_url,
        key,
        protocol,
        models.lines().next().unwrap_or_default(),
        models,
        &[],
    )
    .unwrap();
    profile.id = id.to_string();
    profile
}

fn chat_sse(text: &str) -> String {
    [
        json!({"id":"c1","object":"chat.completion.chunk","model":"up","choices":[{"index":0,"delta":{"role":"assistant","content":text}}]}),
        json!({"id":"c1","object":"chat.completion.chunk","model":"up","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}),
        json!({"id":"c1","object":"chat.completion.chunk","model":"up","choices":[],"usage":{"prompt_tokens":20,"completion_tokens":5,"total_tokens":25}}),
    ]
    .iter()
    .map(|chunk| format!("data: {chunk}\n\n"))
    .collect::<String>()
        + "data: [DONE]\n\n"
}

fn chat_tool_sse() -> String {
    [
        json!({"id":"c2","object":"chat.completion.chunk","model":"up","choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{"index":0,"id":"call_7","type":"function","function":{"name":"Bash","arguments":""}}]}}]}),
        json!({"id":"c2","object":"chat.completion.chunk","model":"up","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"command\":\"ls\"}"}}]}}]}),
        json!({"id":"c2","object":"chat.completion.chunk","model":"up","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}),
    ]
    .iter()
    .map(|chunk| format!("data: {chunk}\n\n"))
    .collect::<String>()
        + "data: [DONE]\n\n"
}

fn binding(model: &str, token: &str) -> AgentBinding {
    AgentBinding {
        enabled: true,
        model: model.to_string(),
        token: token.to_string(),
        claude_roles: ClaudeRoleModels::default(),
        ..AgentBinding::default()
    }
}

/// 网关的供应商池只存在 gateway.json，不碰 settings.json。
fn providers(relays: Vec<RelayProfile>) -> Vec<GatewayProvider> {
    relays
        .into_iter()
        .map(|relay| GatewayProvider {
            enabled: true,
            source: String::new(),
            price: None,
            model_prices: Default::default(),
            model_windows: Default::default(),
            context_window: None,
            proxy: String::new(),
            upstream_new: Vec::new(),
            upstream_gone: Vec::new(),
            models_checked_at_ms: 0,
            relay,
            extra: Default::default(),
        })
        .collect()
}

fn save(gateway: &GatewaySettings) {
    GatewayStore::default().save(gateway).unwrap();
}

async fn post(port: u16, path: &str, token: &str, body: &Value) -> (u16, String) {
    let response = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}{path}"))
        .header("x-api-key", token)
        .header("anthropic-beta", "interleaved-thinking-2025-05-14")
        .header("x-claude-code-session-id", "session-e2e")
        .json(body)
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    (status, response.text().await.unwrap())
}

fn sse_events(text: &str) -> Vec<Value> {
    text.split("\n\n")
        .filter_map(|block| block.lines().find_map(|line| line.strip_prefix("data: ")))
        .filter(|data| *data != "[DONE]")
        .map(|data| serde_json::from_str(data).unwrap())
        .collect()
}

fn claude_request(stream: bool) -> Value {
    json!({
        "model": "claude-sonnet-5",
        "max_tokens": 1024,
        "stream": stream,
        "system": [{ "type": "text", "text": "You are Claude Code." }],
        "messages": [{ "role": "user", "content": "你好" }],
        "tools": [{ "name": "Bash", "description": "run", "input_schema": { "type": "object", "properties": { "command": { "type": "string" } } } }]
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gateway_end_to_end() {
    let state_dir = tempfile::tempdir().unwrap();
    unsafe {
        std::env::set_var(
            codex_plus_core::paths::STATE_DIR_ENV,
            state_dir.path().to_str().unwrap(),
        );
    }
    let port = {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.local_addr().unwrap().port()
    };
    codex_plus_core::resident_helper::ensure_resident_helper(port).unwrap();
    let token = "cpx-gw-claude-e2e";

    // —— 1. Claude → Chat 上游，流式文本；未知模型名改写成主模型 ——
    let (chat_base, chat_seen) =
        mock_upstream(vec![(200, "text/event-stream", chat_sse("你好呀"))]).await;
    let mut gateway = GatewaySettings {
        providers: providers(vec![relay(
            "chat",
            &chat_base,
            "sk-chat",
            RelayProtocol::ChatCompletions,
            "deepseek-v4-pro",
        )]),
        ..GatewaySettings::default()
    };
    gateway
        .bindings
        .insert("claude".into(), binding("deepseek-v4-pro", token));
    save(&gateway);
    let loaded = GatewayStore::default().load().unwrap();
    assert_eq!(
        loaded.providers[0].relay.base_url, chat_base,
        "供应商地址落盘后应保留"
    );
    assert!(
        !state_dir.path().join("settings.json").exists(),
        "网关模式不写直连模式的 settings.json"
    );

    let (status, text) = post(port, "/gw/claude/v1/messages", token, &claude_request(true)).await;
    assert_eq!(status, 200, "{text}");
    let events = sse_events(&text);
    assert_eq!(events.first().unwrap()["type"], "message_start");
    assert_eq!(
        events.first().unwrap()["message"]["model"],
        "claude-sonnet-5"
    );
    assert!(
        events
            .iter()
            .any(|event| event["delta"]["text"] == "你好呀"),
        "{text}"
    );
    assert_eq!(events.last().unwrap()["type"], "message_stop");
    let stop = events
        .iter()
        .find(|event| event["type"] == "message_delta")
        .unwrap();
    assert_eq!(stop["delta"]["stop_reason"], "end_turn");
    {
        let seen = chat_seen.lock().unwrap();
        let (head, body) = &seen[0];
        assert!(head.starts_with("POST /v1/chat/completions"), "{head}");
        assert!(
            head.to_ascii_lowercase()
                .contains("authorization: bearer sk-chat"),
            "{head}"
        );
        assert_eq!(body["model"], "deepseek-v4-pro");
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][0]["content"], "You are Claude Code.");
        assert_eq!(body["tools"][0]["function"]["name"], "Bash");
    }
    // 请求日志在响应转发完后异步落盘，稍等一下。
    let mut log = String::new();
    for _ in 0..40 {
        log =
            std::fs::read_to_string(state_dir.path().join("request-log.jsonl")).unwrap_or_default();
        if log.contains("\"agent\":\"claude\"") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(
        log.contains("\"agent\":\"claude\""),
        "请求日志应记录 Agent：{log}"
    );
    // 用量按天汇总单独存，不受请求日志裁剪影响（紧跟日志之后写，同样稍等）。
    let mut usage = String::new();
    for _ in 0..40 {
        usage = std::fs::read_to_string(state_dir.path().join("gateway-usage.json"))
            .unwrap_or_default();
        if usage.contains("\"agent\":\"claude\"") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(usage.contains("\"agent\":\"claude\""), "{usage}");
    assert!(usage.contains("\"inputTokens\":20"), "{usage}");

    // —— 2. Chat 上游返回工具调用 → tool_use 块 ——
    let (tool_base, _) = mock_upstream(vec![(200, "text/event-stream", chat_tool_sse())]).await;
    gateway.providers = providers(vec![relay(
        "chat",
        &tool_base,
        "sk-chat",
        RelayProtocol::ChatCompletions,
        "deepseek-v4-pro",
    )]);
    save(&gateway);
    let (status, text) = post(port, "/gw/claude/v1/messages", token, &claude_request(true)).await;
    assert_eq!(status, 200, "{text}");
    let events = sse_events(&text);
    let start = events
        .iter()
        .find(|event| event["content_block"]["type"] == "tool_use")
        .unwrap_or_else(|| panic!("{text}"));
    assert_eq!(start["content_block"]["id"], "call_7");
    assert_eq!(start["content_block"]["name"], "Bash");
    let arguments = events
        .iter()
        .filter_map(|event| event["delta"]["partial_json"].as_str())
        .collect::<String>();
    assert_eq!(arguments, "{\"command\":\"ls\"}");
    let stop = events
        .iter()
        .find(|event| event["type"] == "message_delta")
        .unwrap();
    assert_eq!(stop["delta"]["stop_reason"], "tool_use");

    // —— 2b. 上游无视 stream:true 回了整段 JSON：仍然给 Claude Code 完整事件流 ——
    let json_completion = json!({
        "id": "c3", "object": "chat.completion", "model": "up",
        "choices": [{ "index": 0, "message": { "role": "assistant", "content": "整段回复" }, "finish_reason": "stop" }],
        "usage": { "prompt_tokens": 9, "completion_tokens": 2, "total_tokens": 11 }
    })
    .to_string();
    let (json_base, _) = mock_upstream(vec![(200, "application/json", json_completion)]).await;
    gateway.providers = providers(vec![relay(
        "chat",
        &json_base,
        "sk-chat",
        RelayProtocol::ChatCompletions,
        "deepseek-v4-pro",
    )]);
    save(&gateway);
    let (status, text) = post(port, "/gw/claude/v1/messages", token, &claude_request(true)).await;
    assert_eq!(status, 200, "{text}");
    let events = sse_events(&text);
    assert!(
        events
            .iter()
            .any(|event| event["delta"]["text"] == "整段回复"),
        "{text}"
    );
    assert_eq!(events.last().unwrap()["type"], "message_stop", "{text}");

    // —— 3. 非流式 → 原生 Responses 上游 ——
    let responses_body = json!({
        "id": "resp_1", "object": "response", "status": "completed", "model": "gpt-up",
        "output": [{ "id": "msg_1", "type": "message", "role": "assistant", "status": "completed",
                     "content": [{ "type": "output_text", "text": "来自 Responses", "annotations": [] }] }],
        "usage": { "input_tokens": 30, "input_tokens_details": { "cached_tokens": 10 }, "output_tokens": 4 }
    })
    .to_string();
    let (resp_base, resp_seen) =
        mock_upstream(vec![(200, "application/json", responses_body)]).await;
    gateway.providers = providers(vec![relay(
        "resp",
        &resp_base,
        "sk-resp",
        RelayProtocol::Responses,
        "gpt-up",
    )]);
    gateway
        .bindings
        .insert("claude".into(), binding("gpt-up", token));
    save(&gateway);
    let (status, text) = post(
        port,
        "/gw/claude/v1/messages",
        token,
        &claude_request(false),
    )
    .await;
    assert_eq!(status, 200, "{text}");
    let message: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(message["type"], "message");
    assert_eq!(message["content"][0]["text"], "来自 Responses");
    assert_eq!(message["usage"]["input_tokens"], 20);
    assert_eq!(message["usage"]["cache_read_input_tokens"], 10);
    {
        let seen = resp_seen.lock().unwrap();
        let (head, body) = &seen[0];
        assert!(head.starts_with("POST /v1/responses"), "{head}");
        assert_eq!(body["model"], "gpt-up");
        assert_eq!(body["instructions"], "You are Claude Code.");
        assert_eq!(body["store"], false);
    }

    // —— 4. Anthropic 上游原样转发 ——
    let anthropic_sse = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_a\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-up\",\"content\":[],\"usage\":{\"input_tokens\":7,\"output_tokens\":1}}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n".to_string();
    let (anth_base, anth_seen) =
        mock_upstream(vec![(200, "text/event-stream", anthropic_sse.clone())]).await;
    gateway.providers = providers(vec![relay(
        "anth",
        &anth_base,
        "sk-anth",
        RelayProtocol::AnthropicMessages,
        "claude-up",
    )]);
    gateway
        .bindings
        .insert("claude".into(), binding("claude-up", token));
    save(&gateway);
    let request = claude_request(true);
    let (status, text) = post(port, "/gw/claude/v1/messages", token, &request).await;
    assert_eq!(status, 200, "{text}");
    assert_eq!(text, anthropic_sse);
    {
        let seen = anth_seen.lock().unwrap();
        let (head, body) = &seen[0];
        let lower = head.to_ascii_lowercase();
        assert!(head.starts_with("POST /v1/messages"), "{head}");
        assert!(lower.contains("x-api-key: sk-anth"), "{head}");
        assert!(
            !lower.contains("cpx-gw-claude"),
            "本地 token 不能发往上游：{head}"
        );
        assert!(
            lower.contains("anthropic-beta: interleaved-thinking-2025-05-14"),
            "{head}"
        );
        let mut expected = request.clone();
        expected["model"] = json!("claude-up");
        assert_eq!(body, &expected, "除模型名外请求体应原样转发");
    }

    // —— 5. 多 Key：第一个 Key 429 → 换第二个，之后的请求直接跳过冷却中的 Key ——
    let (multi_base, multi_seen) = mock_upstream(vec![
        (
            429,
            "application/json",
            "{\"error\":{\"message\":\"slow down\"}}".to_string(),
        ),
        (200, "text/event-stream", chat_sse("第二个 Key")),
        (200, "text/event-stream", chat_sse("还是第二个 Key")),
    ])
    .await;
    let mut multi = relay(
        "multi",
        &multi_base,
        "sk-one",
        RelayProtocol::ChatCompletions,
        "deepseek-v4-pro",
    );
    multi.api_keys = vec![
        RelayApiKey {
            id: "one".into(),
            name: "one".into(),
            api_key: "sk-one".into(),
        },
        RelayApiKey {
            id: "two".into(),
            name: "two".into(),
            api_key: "sk-two".into(),
        },
    ];
    multi.active_api_key_id = "one".into();
    gateway.providers = providers(vec![multi]);
    gateway
        .bindings
        .insert("claude".into(), binding("deepseek-v4-pro", token));
    save(&gateway);
    let (status, text) = post(port, "/gw/claude/v1/messages", token, &claude_request(true)).await;
    assert_eq!(status, 200, "{text}");
    assert!(text.contains("第二个 Key"), "{text}");
    let (status, text) = post(port, "/gw/claude/v1/messages", token, &claude_request(true)).await;
    assert_eq!(status, 200, "{text}");
    {
        let seen = multi_seen.lock().unwrap();
        let keys = seen
            .iter()
            .map(|(head, _)| {
                head.lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("authorization: bearer ")
                            .map(str::to_string)
                    })
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>();
        assert_eq!(keys, vec!["sk-one", "sk-two", "sk-two"]);
    }

    // —— 6. 路由组：400 不切换；5xx 切到下一个成员 ——
    let (bad_base, _) = mock_upstream(vec![
        (
            400,
            "application/json",
            "{\"error\":{\"message\":\"context_length_exceeded\"}}".to_string(),
        ),
        (
            502,
            "application/json",
            "{\"error\":{\"message\":\"bad gateway\"}}".to_string(),
        ),
    ])
    .await;
    let (good_base, good_seen) =
        mock_upstream(vec![(200, "text/event-stream", chat_sse("备用成员"))]).await;
    gateway.providers = providers(vec![
        relay(
            "bad",
            &bad_base,
            "sk-bad",
            RelayProtocol::ChatCompletions,
            "deepseek-v4-pro",
        ),
        relay(
            "good",
            &good_base,
            "sk-good",
            RelayProtocol::ChatCompletions,
            "glm-5.3",
        ),
    ]);
    // 网关自己的路由组：默认模型就是这个组，Claude 请求的未知模型改写成它。
    gateway.groups = vec![GatewayGroup {
        id: "group".into(),
        name: "group".into(),
        mode: GroupMode::InOrder,
        sticky: Default::default(),
        members: vec![
            GroupMember {
                model: "bad/deepseek-v4-pro".into(),
                weight: 1,
            },
            GroupMember {
                model: "good/glm-5.3".into(),
                weight: 1,
            },
        ],
        extra: Default::default(),
        ..GatewayGroup::default()
    }];
    gateway
        .bindings
        .insert("claude".into(), binding("group/group", token));
    save(&gateway);
    let (status, text) = post(port, "/gw/claude/v1/messages", token, &claude_request(true)).await;
    assert_eq!(status, 400, "{text}");
    let error: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(error["type"], "error");
    assert_eq!(error["error"]["type"], "invalid_request_error");
    assert!(good_seen.lock().unwrap().is_empty(), "400 不应切到其它成员");
    let (status, text) = post(port, "/gw/claude/v1/messages", token, &claude_request(true)).await;
    assert_eq!(status, 200, "{text}");
    assert!(text.contains("备用成员"), "{text}");
    assert_eq!(
        good_seen.lock().unwrap()[0].1["model"],
        "glm-5.3",
        "成员不认识的模型换成成员默认模型"
    );
    // 路由动态：最近一次请求先试 bad（502）再由 good 回答，计数里有换过上游和 Agent 收到的错误。
    {
        let activity = codex_plus_core::gateway::activity::snapshot();
        let claude = activity
            .agents
            .iter()
            .find(|agent| agent.agent == "claude")
            .expect("claude 有路由动态");
        let last = claude.last.as_ref().expect("有最近一次记录");
        assert_eq!(last.result, "answered");
        assert_eq!(last.group_id.as_deref(), Some("group"));
        let planned = last
            .planned
            .iter()
            .map(|member| member.relay_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(planned, vec!["bad", "good"]);
        let steps = last
            .steps
            .iter()
            .map(|step| (step.relay_id.as_str(), step.outcome.as_str(), step.status))
            .collect::<Vec<_>>();
        assert_eq!(
            steps,
            vec![
                ("bad", "failed", Some(502)),
                ("good", "answered", Some(200))
            ]
        );
        assert!(claude.rerouted >= 2, "429 换 Key 与 502 换成员都算换过上游");
        assert!(claude.errors >= 1, "400 原样交给了 Agent");
        assert!(claude.in_flight.is_empty(), "响应转发完后不再显示正在回答");
    }

    // —— 7. 鉴权与开关 ——
    let (status, text) = post(
        port,
        "/gw/claude/v1/messages",
        "cpx-gw-claude-wrong",
        &claude_request(true),
    )
    .await;
    assert_eq!(status, 401, "{text}");
    assert_eq!(
        serde_json::from_str::<Value>(&text).unwrap()["error"]["type"],
        "authentication_error"
    );
    let (status, text) = post(
        port,
        "/gw/claude/v1/messages/count_tokens",
        token,
        &claude_request(false),
    )
    .await;
    assert_eq!(status, 200, "{text}");
    assert!(
        serde_json::from_str::<Value>(&text).unwrap()["input_tokens"]
            .as_u64()
            .unwrap()
            > 0
    );
    let models = reqwest::Client::new()
        .get(format!("http://127.0.0.1:{port}/gw/claude/v1/models"))
        .header("x-api-key", token)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    // 默认模型（这里是路由组）在前，然后是池里所有模型和路由组。
    let ids = models["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|model| model["id"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(ids, vec!["group/group", "deepseek-v4-pro", "glm-5.3"]);

    // —— 8. Grok：Responses 客户端 → Chat 上游，输出 Responses 事件流 ——
    let (grok_base, grok_seen) =
        mock_upstream(vec![(200, "text/event-stream", chat_sse("Grok 你好"))]).await;
    gateway.providers = providers(vec![relay(
        "grokup",
        &grok_base,
        "sk-grok",
        RelayProtocol::ChatCompletions,
        "glm-5.3",
    )]);
    gateway
        .bindings
        .insert("grok".into(), binding("glm-5.3", "cpx-gw-grok-e2e"));
    save(&gateway);
    let response = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}/gw/grok/v1/responses"))
        .bearer_auth("cpx-gw-grok-e2e")
        .json(&json!({
            "model": "glm-5.3",
            "stream": true,
            "input": [{ "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "hi" }] }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    let text = response.text().await.unwrap();
    assert!(text.contains("response.output_text.delta"), "{text}");
    assert!(text.contains("Grok 你好"), "{text}");
    assert!(text.contains("response.completed"), "{text}");
    assert_eq!(grok_seen.lock().unwrap()[0].1["model"], "glm-5.3");

    // —— 8b. ChatGPT 订阅账号：请求整理成 Codex CLI 的样子（system → developer、store:false、只流式），
    //        客户端要非流式时从 response.completed 拼回完整响应 ——
    let completed = json!({ "type": "response.completed", "response": {
        "id": "resp_1", "object": "response", "status": "completed", "model": "gpt-6.1-sol",
        "output": [{ "type": "message", "role": "assistant", "content": [{ "type": "output_text", "text": "订阅回答" }] }],
        "usage": { "input_tokens": 12, "output_tokens": 3, "total_tokens": 15 }
    }});
    let delta = json!({ "type": "response.output_text.delta", "delta": "订阅回答" });
    let (official_base, official_seen) = mock_upstream(vec![
        (
            200,
            "text/event-stream",
            format!("event: response.completed\ndata: {completed}\n\n"),
        ),
        // 订阅后端在回复前后夹带私有事件：Codex 认得，Grok 不认得。
        (
            200,
            "text/event-stream",
            format!(
                "event: codex.rate_limits\ndata: {{\"type\":\"codex.rate_limits\",\"rate_limits\":{{}}}}\n\n\
                 event: response.metadata\ndata: {{\"type\":\"response.metadata\",\"metadata\":{{}}}}\n\n\
                 event: response.output_text.delta\ndata: {delta}\n\n\
                 event: response.completed\ndata: {completed}\n\n"
            ),
        ),
    ])
    .await;
    unsafe {
        std::env::set_var(
            "CODEX_PLUS_OFFICIAL_BASE_URL",
            official_base.trim_end_matches("/v1"),
        );
    }
    let mut chatgpt = codex_plus_core::settings::RelayProfile {
        id: "chatgpt".into(),
        name: "ChatGPT Pro".into(),
        relay_mode: codex_plus_core::settings::RelayMode::Official,
        protocol: RelayProtocol::Responses,
        auth_contents: json!({
            "auth_mode": "chatgpt",
            "codex_plus_gateway": true,
            "tokens": { "access_token": "at-official", "refresh_token": "rt", "account_id": "acc-1" }
        })
        .to_string(),
        model_list: "gpt-6.1-sol".into(),
        ..codex_plus_core::settings::RelayProfile::default()
    };
    chatgpt.model = "gpt-6.1-sol".into();
    gateway.providers = providers(vec![chatgpt]);
    gateway
        .bindings
        .insert("grok".into(), binding("gpt-6.1-sol", "cpx-gw-grok-e2e"));
    save(&gateway);
    let response = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}/gw/grok/v1/responses"))
        .bearer_auth("cpx-gw-grok-e2e")
        .json(&json!({
            "model": "gpt-6.1-sol",
            "stream": false,
            "temperature": 0.3,
            "instructions": "You are Grok.",
            "input": [
                { "type": "message", "role": "system", "content": [{ "type": "input_text", "text": "系统规则" }] },
                { "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "hi" }] }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    let text = response.text().await.unwrap();
    assert!(text.contains("订阅回答"), "{text}");
    {
        let seen = official_seen.lock().unwrap();
        let (head, body) = &seen[0];
        let head_lower = head.to_ascii_lowercase();
        assert!(
            head_lower.contains("authorization: bearer at-official"),
            "{head}"
        );
        assert!(head_lower.contains("chatgpt-account-id: acc-1"), "{head}");
        assert!(head_lower.contains("originator: codex_cli_rs"), "{head}");
        assert!(head_lower.contains("user-agent: codex_cli_rs/"), "{head}");
        assert!(head_lower.contains("session-id: "), "{head}");
        assert_eq!(body["store"], false);
        assert_eq!(body["stream"], true);
        assert!(body.get("temperature").is_none());
        let roles = body["input"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["role"].as_str().unwrap_or_default().to_string())
            .collect::<Vec<_>>();
        assert!(!roles.contains(&"system".to_string()), "{roles:?}");
        assert!(roles.contains(&"developer".to_string()), "{roles:?}");
    }
    // 流式转给 Grok 时去掉后端私有事件，否则 Grok 解析失败。
    let response = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}/gw/grok/v1/responses"))
        .bearer_auth("cpx-gw-grok-e2e")
        .json(&json!({
            "model": "gpt-6.1-sol",
            "stream": true,
            "input": [{ "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "hi" }] }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    let text = response.text().await.unwrap();
    assert!(text.contains("response.output_text.delta"), "{text}");
    assert!(text.contains("response.completed"), "{text}");
    assert!(!text.contains("response.metadata"), "{text}");
    assert!(!text.contains("codex.rate_limits"), "{text}");
    unsafe {
        std::env::remove_var("CODEX_PLUS_OFFICIAL_BASE_URL");
    }

    // —— 8c. ChatGPT API（用 ChatGPT 登录）：直接发 api.openai.com，保留 Agent 自己的指令 ——
    let (api_base, api_seen) = mock_upstream(vec![(
        200,
        "text/event-stream",
        format!("event: response.completed\ndata: {completed}\n\n"),
    )])
    .await;
    let far = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
        + 3_600_000;
    let siwc = RelayProfile {
        id: "chatgpt-api".into(),
        name: "ChatGPT API Plus".into(),
        protocol: RelayProtocol::Responses,
        auth_contents: json!({
            "codex_plus_account": "chatgpt-api",
            "client_id": "app_1",
            "access_token": "siwc-at",
            "refresh_token": "siwc-rt",
            "expires_at_ms": far,
            "api_base": api_base,
        })
        .to_string(),
        model_list: "gpt-6.1-sol".into(),
        model: "gpt-6.1-sol".into(),
        ..RelayProfile::default()
    };
    gateway.providers = providers(vec![siwc]);
    save(&gateway);
    let response = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}/gw/grok/v1/responses"))
        .bearer_auth("cpx-gw-grok-e2e")
        .json(&json!({
            "model": "gpt-6.1-sol",
            "stream": false,
            "temperature": 0.3,
            "instructions": "You are Grok.",
            "tools": [{ "type": "code_interpreter" }, { "type": "function", "name": "run" }],
            "input": [
                { "type": "message", "role": "system", "content": [{ "type": "input_text", "text": "系统规则" }] },
                { "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "hi" }] }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    let text = response.text().await.unwrap();
    assert!(text.contains("订阅回答"), "{text}");
    {
        let seen = api_seen.lock().unwrap();
        let (head, body) = &seen[0];
        let head_lower = head.to_ascii_lowercase();
        assert!(head_lower.starts_with("post /v1/responses"), "{head}");
        assert!(
            head_lower.contains("authorization: bearer siwc-at"),
            "{head}"
        );
        assert!(!head_lower.contains("originator: codex_cli_rs"), "{head}");
        assert_eq!(body["instructions"], "You are Grok.");
        assert_eq!(body["store"], false);
        assert_eq!(body["stream"], true);
        assert!(body.get("temperature").is_none());
        assert_eq!(body["input"][0]["role"], "developer");
        assert_eq!(body["tools"].as_array().unwrap().len(), 1);
    }

    // —— 8d. Codex 以 ChatGPT 官方身份接入：请求带着 Codex 自己的登录先到本地代理（openai_base_url），
    //        经托管的「Codex++ 网关」供应商转进网关，再按池路由 ——
    let (codex_base, codex_seen) =
        mock_upstream(vec![(200, "text/event-stream", chat_sse("经网关回答"))]).await;
    gateway.providers = providers(vec![relay(
        "glm",
        &codex_base,
        "sk-glm",
        RelayProtocol::ChatCompletions,
        "glm-5.3",
    )]);
    gateway
        .bindings
        .insert("codex".into(), binding("glm-5.3", "cpx-gw-codex-e2e"));
    save(&gateway);
    let managed = codex_plus_core::gateway::apply::codex_gateway_profile(
        &gateway,
        gateway
            .binding(&codex_plus_core::tools::ToolId::Codex)
            .unwrap(),
        port,
        true,
    )
    .unwrap();
    let mut direct = codex_plus_core::settings::BackendSettings::default();
    direct.relay_profiles_enabled = true;
    direct.relay_profiles = vec![managed];
    direct.active_relay_id = codex_plus_core::gateway::apply::CODEX_GATEWAY_RELAY_ID.into();
    codex_plus_core::settings::SettingsStore::default()
        .save(&direct)
        .unwrap();
    let response = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}/v1/responses"))
        .bearer_auth("chatgpt-access-token")
        .header("chatgpt-account-id", "acc-codex")
        .header("session-id", "codex-session-1")
        .json(&json!({
            "model": "glm-5.3",
            "stream": true,
            "instructions": "You are Codex.",
            "input": [{ "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "hi" }] }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    let text = response.text().await.unwrap();
    assert!(text.contains("经网关回答"), "{text}");
    assert!(text.contains("response.completed"), "{text}");
    {
        let seen = codex_seen.lock().unwrap();
        let (head, body) = &seen[0];
        assert!(
            head.to_ascii_lowercase()
                .contains("authorization: bearer sk-glm"),
            "{head}"
        );
        assert!(
            !head.contains("chatgpt-access-token"),
            "Codex 的登录凭据不外传给池里的供应商：{head}"
        );
        assert_eq!(body["model"], "glm-5.3");
    }
    // 本地代理这一跳不另记日志：只有网关那一条（带 agent）。
    let mut codex_entries = Vec::new();
    for _ in 0..50 {
        codex_entries = codex_plus_core::request_log::read_request_logs(100)
            .unwrap_or_default()
            .into_iter()
            .filter(|entry| entry.thread_id.as_deref() == Some("codex-session-1"))
            .collect::<Vec<_>>();
        if !codex_entries.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let codex_entries = codex_plus_core::request_log::read_request_logs(100)
        .unwrap_or_default()
        .into_iter()
        .filter(|entry| entry.thread_id.as_deref() == Some("codex-session-1"))
        .collect::<Vec<_>>();
    assert_eq!(codex_entries.len(), 1, "{codex_entries:?}");
    assert_eq!(codex_entries[0].agent.as_deref(), Some("codex"));

    // —— 8e. 生图：自动选池里的生图模型，第一家 5xx 换下一家，请求里的模型名换成那一家的 ——
    let (bad_base, _) = mock_upstream(vec![(
        500,
        "application/json",
        "{\"error\":\"boom\"}".to_string(),
    )])
    .await;
    let (image_base, image_seen) = mock_upstream(vec![(
        200,
        "application/json",
        json!({ "data": [{ "b64_json": "AAAA" }] }).to_string(),
    )])
    .await;
    gateway.providers = providers(vec![
        relay(
            "chatonly",
            &grok_base,
            "sk-grok",
            RelayProtocol::ChatCompletions,
            "glm-5.3",
        ),
        relay(
            "bad",
            &bad_base,
            "sk-bad",
            RelayProtocol::Responses,
            "gpt-image-1",
        ),
        relay(
            "painter",
            &image_base,
            "sk-paint",
            RelayProtocol::Responses,
            "flux-1-schnell",
        ),
    ]);
    save(&gateway);
    let response = reqwest::Client::new()
        .post(format!(
            "http://127.0.0.1:{port}/gw/grok/v1/images/generations"
        ))
        .bearer_auth("cpx-gw-grok-e2e")
        .json(&json!({ "model": "gpt-image-2", "prompt": "a cat" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    assert!(response.text().await.unwrap().contains("AAAA"));
    {
        let seen = image_seen.lock().unwrap();
        let (head, body) = &seen[0];
        assert!(head.starts_with("POST /v1/images/generations"), "{head}");
        assert!(
            head.to_ascii_lowercase()
                .contains("authorization: bearer sk-paint"),
            "{head}"
        );
        assert_eq!(body["model"], "flux-1-schnell");
        assert_eq!(body["prompt"], "a cat");
    }
    // 生图也记进请求日志：类型、Agent、最后回答的那家、试了几家。
    let mut image_entry = None;
    for _ in 0..40 {
        let log =
            std::fs::read_to_string(state_dir.path().join("request-log.jsonl")).unwrap_or_default();
        image_entry = log
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .find(|entry| {
                entry["wire_api"] == "imageGenerations" || entry["wireApi"] == "imageGenerations"
            });
        if image_entry.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let image_entry = image_entry.expect("生图请求要记进请求日志");
    let field = |snake: &str, camel: &str| {
        image_entry
            .get(snake)
            .or_else(|| image_entry.get(camel))
            .cloned()
            .unwrap_or(Value::Null)
    };
    assert_eq!(field("agent", "agent"), "grok");
    assert_eq!(field("relay_id", "relayId"), "painter");
    assert_eq!(field("attempts", "attempts"), 2);
    assert_eq!(field("success", "success"), true);
    // 关掉生图：直接报错，不发给上游。
    gateway.image_models.insert("grok".into(), "off".into());
    save(&gateway);
    let response = reqwest::Client::new()
        .post(format!(
            "http://127.0.0.1:{port}/gw/grok/v1/images/generations"
        ))
        .bearer_auth("cpx-gw-grok-e2e")
        .json(&json!({ "prompt": "a cat" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 400);
    assert!(response.text().await.unwrap().contains("生图已关闭"));
    gateway.image_models.clear();

    // Codex 以 ChatGPT 身份接入（8d 里 Codex 的供应商已是网关托管的那个）：它的生图请求打到本地代理的
    // `/v1/images/generations`，也要按 Codex 的生图模型走网关，不能按直连转回本地代理自己。
    let (codex_image_base, codex_image_seen) = mock_upstream(vec![(
        200,
        "application/json",
        json!({ "data": [{ "b64_json": "BBBB" }] }).to_string(),
    )])
    .await;
    gateway.providers = providers(vec![
        relay(
            "chatonly",
            &grok_base,
            "sk-grok",
            RelayProtocol::ChatCompletions,
            "glm-5.3",
        ),
        relay(
            "painter",
            &codex_image_base,
            "sk-paint",
            RelayProtocol::Responses,
            "gpt-image-2",
        ),
    ]);
    save(&gateway);
    let response = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}/v1/images/generations"))
        .bearer_auth("chatgpt-access-token")
        .json(&json!({ "model": "gpt-image-2", "prompt": "a cat" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    assert!(response.text().await.unwrap().contains("BBBB"));
    {
        let seen = codex_image_seen.lock().unwrap();
        let (head, body) = &seen[0];
        assert!(head.starts_with("POST /v1/images/generations"), "{head}");
        assert!(
            head.to_ascii_lowercase()
                .contains("authorization: bearer sk-paint"),
            "{head}"
        );
        assert_eq!(body["model"], "gpt-image-2");
    }

    // —— 9. 关闭绑定后返回 503，旧的 Codex 代理路径不受影响 ——
    gateway.bindings.get_mut("claude").unwrap().enabled = false;
    save(&gateway);
    let (status, text) = post(port, "/gw/claude/v1/messages", token, &claude_request(true)).await;
    assert_eq!(status, 503, "{text}");
    assert_eq!(
        serde_json::from_str::<Value>(&text).unwrap()["type"],
        "error"
    );

    codex_plus_core::resident_helper::stop_resident_helper();
}
