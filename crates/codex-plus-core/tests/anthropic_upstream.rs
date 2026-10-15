//! Codex（Responses）→ Anthropic Messages 上游的端到端测试。
//!
//! 覆盖：请求体/URL/认证头、非流式与流式响应转回 Responses、
//! 供应商测试端点，以及 profile 落盘后 Codex 侧指向本机代理。

use codex_plus_core::anthropic_bridge::{
    AnthropicSseToResponsesConverter, anthropic_message_to_chat_completion,
};
use codex_plus_core::protocol_proxy::{
    UpstreamWireApi, chat_completion_to_response_with_request,
    open_responses_proxy_request_with_settings,
};
use codex_plus_core::relay_config::{
    apply_relay_profile_to_home_with_switch_rules, normalize_relay_profile_for_storage,
    test_relay_profile,
};
use codex_plus_core::settings::{BackendSettings, RelayMode, RelayProfile, RelayProtocol};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn anthropic_profile(base_url: String) -> RelayProfile {
    RelayProfile {
        id: "anthropic".to_string(),
        name: "Anthropic".to_string(),
        protocol: RelayProtocol::AnthropicMessages,
        base_url,
        api_key: "sk-anthropic".to_string(),
        relay_mode: RelayMode::PureApi,
        ..RelayProfile::default()
    }
}

fn anthropic_settings(base_url: String) -> BackendSettings {
    BackendSettings {
        active_relay_id: "anthropic".to_string(),
        relay_profiles: vec![anthropic_profile(base_url)],
        ..BackendSettings::default()
    }
}

/// 收下一次请求，回固定响应，返回 (请求头文本, 请求体 JSON)。
async fn capture_request(
    listener: tokio::net::TcpListener,
    content_type: &str,
    response_body: String,
) -> (String, Value) {
    let (mut stream, _) = listener.accept().await.unwrap();
    let mut buffer = Vec::new();
    let mut chunk = [0; 4096];
    let (header_end, content_length) = loop {
        let read = stream.read(&mut chunk).await.unwrap();
        assert!(read > 0, "请求在请求头结束前断开");
        buffer.extend_from_slice(&chunk[..read]);
        let Some(header_end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        let headers = String::from_utf8_lossy(&buffer[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.split_once(':').and_then(|(name, value)| {
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
            })
            .unwrap_or(0);
        break (header_end + 4, content_length);
    };
    while buffer.len() < header_end + content_length {
        let read = stream.read(&mut chunk).await.unwrap();
        assert!(read > 0, "请求在请求体结束前断开");
        buffer.extend_from_slice(&chunk[..read]);
    }
    let headers = String::from_utf8_lossy(&buffer[..header_end - 4]).to_string();
    let body = serde_json::from_slice(&buffer[header_end..header_end + content_length]).unwrap();
    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-length: {}\r\ncontent-type: {content_type}\r\n\r\n{}",
        response_body.len(),
        response_body
    );
    stream.write_all(response.as_bytes()).await.unwrap();
    (headers, body)
}

fn codex_request(stream: bool) -> Value {
    json!({
        "model": "claude-sonnet-5",
        "stream": stream,
        "instructions": "你是助手",
        "max_output_tokens": 256,
        "input": [
            { "type": "message", "role": "user",
              "content": [{ "type": "input_text", "text": "你好" }] }
        ],
        "tools": [{
            "type": "function",
            "name": "lookup",
            "description": "查一下",
            "parameters": { "type": "object", "properties": { "q": { "type": "string" } } }
        }]
    })
}

#[tokio::test]
async fn requests_go_to_messages_endpoint_with_anthropic_body_and_headers() {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let upstream_body = json!({
        "id": "msg_01",
        "type": "message",
        "role": "assistant",
        "model": "claude-sonnet-5",
        "stop_reason": "end_turn",
        "content": [{ "type": "text", "text": "你好，我是 Claude" }],
        "usage": { "input_tokens": 12, "output_tokens": 6 }
    })
    .to_string();
    let server = tokio::spawn(capture_request(
        listener,
        "application/json",
        upstream_body.clone(),
    ));

    let request = codex_request(false);
    let mut upstream = open_responses_proxy_request_with_settings(
        &request.to_string(),
        anthropic_settings(format!("http://{address}")),
    )
    .await
    .unwrap();

    assert_eq!(upstream.wire_api, UpstreamWireApi::AnthropicMessages);
    assert!(!upstream.is_stream);

    let (headers, sent) = server.await.unwrap();
    // 请求打到 /v1/messages，且 body 已是 Anthropic 形态。
    assert!(
        headers.starts_with("POST /v1/messages "),
        "未命中 Messages 端点：{headers}"
    );
    assert!(
        headers.to_ascii_lowercase().contains("x-api-key: sk-anthropic"),
        "缺少 x-api-key：{headers}"
    );
    assert!(
        headers
            .to_ascii_lowercase()
            .contains("anthropic-version: 2023-06-01"),
        "缺少 anthropic-version：{headers}"
    );
    assert_eq!(sent["model"], "claude-sonnet-5");
    assert_eq!(sent["system"], "你是助手");
    assert_eq!(sent["max_tokens"], 256);
    assert_eq!(sent["messages"][0]["role"], "user");
    assert_eq!(sent["messages"][0]["content"][0]["text"], "你好");
    assert_eq!(sent["tools"][0]["name"], "lookup");
    assert_eq!(sent["tools"][0]["input_schema"]["type"], "object");
    assert!(sent.get("input").is_none(), "不该把 Responses 的 input 原样发出");

    // 上游返回体保持原样，转换由 launcher 负责（这里复现同一条链路）。
    let raw = upstream.read_body().await.unwrap();
    assert_eq!(String::from_utf8_lossy(&raw), upstream_body);
    let message: Value = serde_json::from_slice(&raw).unwrap();
    let chat = anthropic_message_to_chat_completion(message).unwrap();
    let response = chat_completion_to_response_with_request(chat, &request).unwrap();
    assert_eq!(response["output"][0]["content"][0]["text"], "你好，我是 Claude");
    assert_eq!(response["usage"]["input_tokens"], 12);
    assert_eq!(response["usage"]["output_tokens"], 6);
}

#[tokio::test]
async fn bearer_mode_switches_authentication_header() {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(capture_request(
        listener,
        "application/json",
        r#"{"id":"msg_02","content":[{"type":"text","text":"ok"}],"usage":{"input_tokens":1,"output_tokens":1}}"#.to_string(),
    ));

    let mut settings = anthropic_settings(format!("http://{address}"));
    settings.relay_profiles[0].anthropic_auth_header = "authorization".to_string();
    let _ = open_responses_proxy_request_with_settings(
        &codex_request(false).to_string(),
        settings,
    )
    .await
    .unwrap();

    let (headers, _) = server.await.unwrap();
    let lowered = headers.to_ascii_lowercase();
    assert!(
        lowered.contains("authorization: bearer sk-anthropic"),
        "Bearer 模式未注入 Authorization：{headers}"
    );
    assert!(
        !lowered.contains("x-api-key"),
        "Bearer 模式不该再发 x-api-key：{headers}"
    );
    // 版本头与认证字段无关，始终要有。
    assert!(lowered.contains("anthropic-version: 2023-06-01"));
}

#[tokio::test]
async fn streaming_upstream_sse_becomes_responses_events() {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let upstream_sse = concat!(
        "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_stream\",\"model\":\"claude-sonnet-5\",\"usage\":{\"input_tokens\":9}}}\n\n",
        "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\"}}\n\n",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"你\"}}\n\n",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"好\"}}\n\n",
        "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":2}}\n\n",
        "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"
    );
    let server = tokio::spawn(capture_request(
        listener,
        "text/event-stream",
        upstream_sse.to_string(),
    ));

    let request = codex_request(true);
    let mut upstream = open_responses_proxy_request_with_settings(
        &request.to_string(),
        anthropic_settings(format!("http://{address}")),
    )
    .await
    .unwrap();
    assert!(upstream.is_stream);
    assert_eq!(upstream.wire_api, UpstreamWireApi::AnthropicMessages);
    let (headers, sent) = server.await.unwrap();
    assert!(headers.starts_with("POST /v1/messages "), "{headers}");
    assert_eq!(sent["stream"], true);

    let raw = upstream.read_body().await.unwrap();
    // 故意切成两半：SSE 事件跨 chunk 时必须自己缓冲。
    let split = raw.len() / 2;
    let mut converter = AnthropicSseToResponsesConverter::with_request(&request);
    let mut output = converter.push_bytes(&raw[..split]);
    output.extend(converter.push_bytes(&raw[split..]));
    assert!(converter.has_terminal_event(), "message_stop 应被识别为终止事件");
    output.extend(converter.finish());
    let text = String::from_utf8_lossy(&output).to_string();
    assert!(text.contains("response.created"), "{text}");
    assert!(text.contains("response.output_text.delta"), "{text}");
    assert!(text.contains("\"你\""), "{text}");
    assert!(text.contains("\"好\""), "{text}");
    assert!(text.contains("response.completed"), "{text}");
}

#[tokio::test]
async fn provider_test_probe_hits_messages_endpoint() {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(capture_request(
        listener,
        "application/json",
        r#"{"id":"msg_probe","content":[{"type":"text","text":"hi"}],"usage":{"input_tokens":1,"output_tokens":1}}"#.to_string(),
    ));

    let profile = anthropic_profile(format!("http://{address}"));
    let result = test_relay_profile(&profile, "claude-sonnet-5").await.unwrap();
    assert_eq!(result.http_status, 200);
    assert_eq!(result.endpoint, format!("http://{address}/v1/messages"));

    let (headers, sent) = server.await.unwrap();
    assert!(headers.starts_with("POST /v1/messages "), "{headers}");
    assert!(
        headers.to_ascii_lowercase().contains("x-api-key: sk-anthropic"),
        "{headers}"
    );
    assert_eq!(sent["model"], "claude-sonnet-5");
    assert_eq!(sent["max_tokens"], 16);
    assert_eq!(sent["messages"][0]["content"], "hi");
}

#[tokio::test]
async fn anthropic_profile_points_codex_at_local_proxy() {
    let temp = tempfile::tempdir().unwrap();
    let mut profile = RelayProfile {
        id: "relay-anthropic".to_string(),
        model: "claude-sonnet-5".to_string(),
        upstream_base_url: "https://api.anthropic.com".to_string(),
        api_key: "sk-test-redacted".to_string(),
        protocol: RelayProtocol::AnthropicMessages,
        relay_mode: RelayMode::PureApi,
        config_contents: r#"model = "claude-sonnet-5"
model_provider = "custom"

[model_providers.custom]
name = "custom"
wire_api = "responses"
requires_openai_auth = true
base_url = "https://api.anthropic.com"
"#
        .to_string(),
        auth_contents: r#"{"OPENAI_API_KEY":"sk-test-redacted"}"#.to_string(),
        ..RelayProfile::default()
    };

    normalize_relay_profile_for_storage(&mut profile).unwrap();
    // 真实上游地址保存在本地，不写进 Codex 的 live 配置。
    assert_eq!(profile.upstream_base_url, "https://api.anthropic.com");

    apply_relay_profile_to_home_with_switch_rules(temp.path(), &profile, "").unwrap();
    let live = std::fs::read_to_string(temp.path().join("config.toml")).unwrap();
    assert!(
        live.contains(r#"base_url = "http://127.0.0.1:57321/v1""#),
        "Codex 应指向本机协议代理：{live}"
    );
    assert!(
        !live.contains("https://api.anthropic.com"),
        "真实上游不能进 live 配置：{live}"
    );
}

/// 协议变体与新增字段的序列化 / 默认值契约。
///
/// 只做纯数据验证，不读写真实 settings.json，避免测试碰用户环境。
#[test]
fn protocol_variant_and_field_defaults_round_trip() {
    let profile = anthropic_profile("https://api.anthropic.com".to_string());
    let serialized = serde_json::to_value(&profile).unwrap();
    assert_eq!(serialized["protocol"], "anthropicMessages");
    let parsed: RelayProfile = serde_json::from_value(serialized).unwrap();
    assert_eq!(parsed.protocol, RelayProtocol::AnthropicMessages);
    assert_eq!(parsed.anthropic_auth_header(), "x-api-key");
    assert_eq!(
        parsed.anthropic_max_tokens(),
        codex_plus_core::anthropic_bridge::ANTHROPIC_DEFAULT_MAX_TOKENS
    );

    // 旧版本设置文件没有这两个字段，必须按默认值加载。
    let legacy: RelayProfile = serde_json::from_value(json!({
        "id": "legacy",
        "name": "legacy",
        "protocol": "anthropicMessages"
    }))
    .unwrap();
    assert_eq!(legacy.anthropic_max_tokens, 0);
    assert_eq!(legacy.anthropic_max_tokens(), 8192);
    assert_eq!(legacy.anthropic_auth_header.trim(), "");

    // 别名写法在归一化后收敛到两种取值。
    let mut aliased = anthropic_profile("https://api.anthropic.com".to_string());
    aliased.anthropic_auth_header = "Bearer".to_string();
    normalize_relay_profile_for_storage(&mut aliased).unwrap();
    assert_eq!(aliased.anthropic_auth_header, "authorization");
}

/// Anthropic 协议必须被判定为「需要本地协议代理」，否则 Codex 会直连 `/responses` 拿 404。
#[test]
fn anthropic_protocol_requires_protocol_proxy_transport() {
    let settings = anthropic_settings("https://api.anthropic.com".to_string());
    assert!(settings.active_relay_transport_uses_protocol_proxy());
    assert!(settings.relay_profiles[0].requires_protocol_proxy_conversion());

    // 回归保护：Chat Completions 专有开关不能因为新增协议被顺手放开。
    let mut profile = anthropic_profile("https://api.anthropic.com".to_string());
    profile.standard_openai_protocol = true;
    assert!(
        !profile.web_search_history_compat_enabled(),
        "web_search_history_compat 只对 Responses 生效"
    );
}
