//! Anthropic 上游的真机验收（默认忽略，需要真实的 Anthropic 协议网关）。
//!
//! 这些用例会向真实上游发请求、消耗 token，因此默认 `#[ignore]`，不进常规回归。
//! 运行方式（凭据只走环境变量，不要写进文件）：
//!
//! ```text
//! LIVE_ANTHROPIC_BASE_URL=https://your-gateway.example.com \
//! LIVE_ANTHROPIC_API_KEY=sk-... \
//! LIVE_ANTHROPIC_MODEL=claude-sonnet-4-6 \
//! cargo test -p codex-plus-core --test all live_anthropic_upstream:: -- --ignored --nocapture --test-threads=1
//! ```
//!
//! 覆盖：认证/端点探针、非流式与流式转换、工具调用、**思考签名跨轮回放**、
//! 截断上报、1M beta 头、Claude Code 伪装、提示缓存命中。

use codex_plus_core::anthropic_bridge::{
    AnthropicSseToResponsesConverter, anthropic_message_to_chat_completion,
    attach_thinking_to_responses, thinking_blocks_from_anthropic_message,
};
use codex_plus_core::protocol_proxy::{
    chat_completion_to_response_with_request, open_responses_proxy_request_with_settings,
};
use codex_plus_core::relay_config::test_relay_profile;
use codex_plus_core::settings::{BackendSettings, RelayMode, RelayProfile, RelayProtocol};
use serde_json::{Value, json};

fn model() -> String {
    std::env::var("LIVE_ANTHROPIC_model().as_str()").unwrap_or_else(|_| "claude-sonnet-4-6".to_string())
}

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("缺少环境变量 {name}"))
}

fn settings(context_window: &str, windows: &str) -> BackendSettings {
    BackendSettings {
        active_relay_id: "live".to_string(),
        relay_profiles: vec![RelayProfile {
            id: "live".to_string(),
            name: "live".to_string(),
            protocol: RelayProtocol::AnthropicMessages,
            base_url: env("LIVE_ANTHROPIC_BASE_URL"),
            api_key: env("LIVE_ANTHROPIC_API_KEY"),
            relay_mode: RelayMode::PureApi,
            context_window: context_window.to_string(),
            model_windows: windows.to_string(),
            ..RelayProfile::default()
        }],
        ..BackendSettings::default()
    }
}

fn tools() -> Value {
    json!([{
        "type": "function",
        "name": "get_weather",
        "description": "查询城市天气",
        "parameters": {
            "type": "object",
            "properties": { "city": { "type": "string" } },
            "required": ["city"]
        }
    }])
}

/// 把上游原始响应体走一遍 launcher 的转换链，拿到 Codex 实际看到的 Responses JSON。
fn to_responses(raw: &[u8], request: &Value) -> Value {
    let message: Value = serde_json::from_slice(raw).expect("上游不是合法 JSON");
    let blocks = thinking_blocks_from_anthropic_message(&message);
    let chat = anthropic_message_to_chat_completion(message).expect("Anthropic → Chat 失败");
    let mut response =
        chat_completion_to_response_with_request(chat, request).expect("Chat → Responses 失败");
    if !blocks.is_empty() {
        attach_thinking_to_responses(&mut response, &blocks);
    }
    response
}

/// 真机请求带重试：网关按通道路由，偶发 5xx（无可用通道/限流）不该让验收误报。
async fn open(
    request: &Value,
    settings: BackendSettings,
) -> codex_plus_core::protocol_proxy::UpstreamProxyResponse {
    let mut last_error = String::from("未发起请求");
    for attempt in 1..=3u32 {
        match open_responses_proxy_request_with_settings(&request.to_string(), settings.clone())
            .await
        {
            Ok(response) if response.status_code < 500 => return response,
            Ok(response) => {
                last_error = format!("上游返回 {}（第 {attempt} 次）", response.status_code)
            }
            Err(error) => last_error = format!("{error:#}（第 {attempt} 次）"),
        }
        tokio::time::sleep(std::time::Duration::from_millis(600 * u64::from(attempt))).await;
    }
    panic!("上游连续失败：{last_error}");
}

#[tokio::test]
#[ignore = "真机用例：需要 LIVE_ANTHROPIC_* 环境变量与可用额度"]
async fn live_probe_auth_and_endpoint() {
    let profile = settings("", "").relay_profiles[0].clone();
    let result = test_relay_profile(&profile, model().as_str()).await.unwrap();
    println!("[probe] status={} endpoint={}", result.http_status, result.endpoint);
    println!("[probe] preview={}", &result.response_preview.chars().take(160).collect::<String>());
    assert_eq!(result.http_status, 200, "供应商测试探针失败");
}

#[tokio::test]
#[ignore = "真机用例：需要 LIVE_ANTHROPIC_* 环境变量与可用额度"]
async fn live_single_turn_non_stream() {
    let request = json!({
        "model": model().as_str(),
        "stream": false,
        "instructions": "你是简洁的中文助手。",
        "max_output_tokens": 128,
        "input": [{ "type": "message", "role": "user",
            "content": [{ "type": "input_text", "text": "回复两个字：收到" }] }]
    });
    let mut upstream = open(&request, settings("", "")).await;
    println!("[non-stream] status={} wire={:?}", upstream.status_code, upstream.wire_api);
    let raw = upstream.read_body().await.unwrap();
    println!("[non-stream] upstream={}", String::from_utf8_lossy(&raw).chars().take(200).collect::<String>());
    let response = to_responses(&raw, &request);
    let text = response["output"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["content"].as_array())
        .flatten()
        .filter_map(|part| part["text"].as_str())
        .collect::<String>();
    println!("[non-stream] codex_text={text:?} usage={}", response["usage"]);
    assert!(text.contains("收到"), "文本未透出：{response}");
    assert!(response["usage"]["input_tokens"].as_u64().unwrap() > 0);
}

#[tokio::test]
#[ignore = "真机用例：需要 LIVE_ANTHROPIC_* 环境变量与可用额度"]
async fn live_stream_events() {
    let request = json!({
        "model": model().as_str(),
        "stream": true,
        "instructions": "你是简洁的中文助手。",
        "max_output_tokens": 128,
        "input": [{ "type": "message", "role": "user",
            "content": [{ "type": "input_text", "text": "从 1 数到 5，用顿号分隔" }] }]
    });
    let mut upstream = open(&request, settings("", "")).await;
    assert!(upstream.is_stream);
    let raw = upstream.read_body().await.unwrap();
    let mut converter = AnthropicSseToResponsesConverter::with_request(&request);
    let mut out = converter.push_bytes(&raw);
    out.extend(converter.finish());
    let text = String::from_utf8_lossy(&out).to_string();
    let events: Vec<&str> = text
        .lines()
        .filter_map(|line| line.strip_prefix("event: "))
        .collect();
    println!("[stream] 事件序列={:?}", events);
    assert!(events.contains(&"response.created"));
    assert!(events.contains(&"response.output_text.delta"));
    assert!(events.contains(&"response.completed"));
    assert!(text.contains("1"), "流式文本缺失：{text}");
}

#[tokio::test]
#[ignore = "真机用例：需要 LIVE_ANTHROPIC_* 环境变量与可用额度"]
async fn live_tool_call_round_trip() {
    let request = json!({
        "model": model().as_str(),
        "stream": false,
        "instructions": "需要外部信息时必须调用工具，不要自己编。",
        "max_output_tokens": 512,
        "tools": tools(),
        "tool_choice": "auto",
        "input": [{ "type": "message", "role": "user",
            "content": [{ "type": "input_text", "text": "北京现在天气怎么样？用 get_weather 查。" }] }]
    });
    let mut upstream = open(&request, settings("", "")).await;
    let raw = upstream.read_body().await.unwrap();
    let response = to_responses(&raw, &request);
    let items: Vec<String> = response["output"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["type"].as_str().unwrap_or("?").to_string())
        .collect();
    println!("[tool] output items={items:?}");
    println!("[tool] response={}", response.to_string().chars().take(400).collect::<String>());
    assert!(items.iter().any(|kind| kind == "function_call"), "未产出 function_call");
    let call = response["output"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["type"] == "function_call")
        .unwrap();
    assert_eq!(call["name"], "get_weather");
    println!("[tool] call_id={} arguments={}", call["call_id"], call["arguments"]);
}

/// 关键用例：思考 + 工具循环 + 签名回放（第二轮把 reasoning item 原样带回）。
#[tokio::test]
#[ignore = "真机用例：需要 LIVE_ANTHROPIC_* 环境变量与可用额度"]
async fn live_thinking_signature_replay_multi_turn() {
    let base_settings = settings("", "");
    // 第一轮：开思考，要求调用工具。
    let first = json!({
        "model": model().as_str(),
        "stream": false,
        "instructions": "需要外部信息时必须调用工具，不要自己编。",
        "max_output_tokens": 2048,
        "reasoning": { "effort": "medium" },
        "tools": tools(),
        "tool_choice": "auto",
        "input": [{ "type": "message", "role": "user",
            "content": [{ "type": "input_text", "text": "上海现在天气怎么样？用 get_weather 查。" }] }]
    });
    let mut upstream =
        open(&first, base_settings.clone()).await;
    println!("[think:1] status={}", upstream.status_code);
    let raw = upstream.read_body().await.unwrap();
    let response = to_responses(&raw, &first);
    let reasoning = response["output"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["type"] == "reasoning")
        .cloned();
    let call = response["output"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["type"] == "function_call")
        .cloned();
    println!(
        "[think:1] reasoning={} function_call={}",
        reasoning.is_some(),
        call.is_some()
    );
    if let Some(item) = &reasoning {
        let encrypted = item["encrypted_content"].as_str().unwrap_or("");
        println!("[think:1] 签名信封长度={} 前缀OK={}", encrypted.len(), encrypted.starts_with(codex_plus_core::anthropic_bridge::ANTHROPIC_THINKING_PREFIX));
    }
    let (Some(reasoning), Some(call)) = (reasoning, call) else {
        panic!("第一轮没有产出 reasoning + function_call，无法验证签名回放：{response}");
    };

    // 第二轮：reasoning item + function_call + 工具结果一起回传。
    let second = json!({
        "model": model().as_str(),
        "stream": false,
        "instructions": "需要外部信息时必须调用工具，不要自己编。",
        "max_output_tokens": 2048,
        "reasoning": { "effort": "medium" },
        "tools": tools(),
        "tool_choice": "auto",
        "input": [
            reasoning.clone(),
            {
                "type": "function_call",
                "call_id": call["call_id"],
                "name": call["name"],
                "arguments": call["arguments"]
            },
            {
                "type": "function_call_output",
                "call_id": call["call_id"],
                "output": "{\"city\":\"上海\",\"temp_c\":19,\"weather\":\"多云\"}"
            }
        ]
    });
    // 先看我们自己发出去的 Anthropic 请求里有没有把签名思考块放回去（本地可验证）。
    let converted = codex_plus_core::anthropic_bridge::responses_to_anthropic_messages(
        serde_json::from_str::<Value>(&second.to_string()).unwrap(),
        &codex_plus_core::anthropic_bridge::request_options(&base_settings.relay_profiles[0]),
    )
    .unwrap();
    let assistant = converted["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|message| message["role"] == "assistant")
        .cloned();
    if let Some(assistant) = &assistant {
        println!(
            "[think:2] 发出给上游的助手首块={:?} 带签名={}",
            assistant["content"][0]["type"],
            assistant["content"][0]["signature"].is_string()
        );
    }
    println!("[think:2] thinking={}", converted["thinking"]);

    let mut upstream = open(&second, base_settings).await;
    let status = upstream.status_code;
    let raw = upstream.read_body().await.unwrap();
    println!("[think:2] status={status}");
    println!(
        "[think:2] upstream={}",
        String::from_utf8_lossy(&raw).chars().take(300).collect::<String>()
    );
    assert_eq!(status, 200, "第二轮被上游拒绝（多半是签名回放问题）");
    let response = to_responses(&raw, &second);
    let text = response["output"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["content"].as_array())
        .flatten()
        .filter_map(|part| part["text"].as_str())
        .collect::<String>();
    println!("[think:2] 最终回答={text:?}");
    assert!(!text.trim().is_empty(), "第二轮没有文本回答：{response}");
}

#[tokio::test]
#[ignore = "真机用例：需要 LIVE_ANTHROPIC_* 环境变量与可用额度"]
async fn live_truncation_is_reported_as_incomplete() {
    let request = json!({
        "model": model().as_str(),
        "stream": false,
        "max_output_tokens": 16,
        "input": [{ "type": "message", "role": "user",
            "content": [{ "type": "input_text", "text": "写一篇 800 字关于海洋的散文" }] }]
    });
    let mut upstream = open(&request, settings("", "")).await;
    let raw = upstream.read_body().await.unwrap();
    let response = to_responses(&raw, &request);
    println!(
        "[truncate] status={} incomplete={}",
        response["status"], response["incomplete_details"]
    );
    assert_eq!(response["status"], "incomplete", "截断必须如实上报：{response}");
    assert_eq!(response["incomplete_details"]["reason"], "max_output_tokens");
}

/// 1M beta 头：网关是否接受 context-1m-2025-08-07（不接受属网关限制，只记录）。
#[tokio::test]
#[ignore = "真机用例：需要 LIVE_ANTHROPIC_* 环境变量与可用额度"]
async fn live_context_1m_beta_probe() {
    let request = json!({
        "model": model().as_str(),
        "stream": false,
        "max_output_tokens": 64,
        "input": [{ "type": "message", "role": "user",
            "content": [{ "type": "input_text", "text": "说 ok" }] }]
    });
    let mut upstream = open(&request, settings("", r#"{"claude-sonnet-4-6":"1M"}"#)).await;
    let raw = upstream.read_body().await.unwrap();
    println!(
        "[1m] status={} body={}",
        upstream.status_code,
        String::from_utf8_lossy(&raw).chars().take(240).collect::<String>()
    );
}

/// 伪装 Claude Code：网关是否接受（不接受属网关策略，只记录）。
#[tokio::test]
#[ignore = "真机用例：需要 LIVE_ANTHROPIC_* 环境变量与可用额度"]
async fn live_impersonation_probe() {
    let mut base = settings("", "");
    base.relay_profiles[0].anthropic_impersonate_claude_code = true;
    let request = json!({
        "model": model().as_str(),
        "stream": false,
        "max_output_tokens": 64,
        "input": [{ "type": "message", "role": "user",
            "content": [{ "type": "input_text", "text": "说 ok" }] }]
    });
    let mut upstream = open(&request, base).await;
    let raw = upstream.read_body().await.unwrap();
    println!(
        "[impersonate] status={} body={}",
        upstream.status_code,
        String::from_utf8_lossy(&raw).chars().take(240).collect::<String>()
    );
}

/// 提示缓存：同一前缀连发两次，第二次应出现 cache_read（网关不支持时只记录）。
#[tokio::test]
#[ignore = "真机用例：需要 LIVE_ANTHROPIC_* 环境变量与可用额度"]
async fn live_prompt_cache_probe() {
    let long_system = "你是严谨的中文技术助手。".repeat(200);
    let mut cache_reads = Vec::new();
    for round in 0..2 {
        let request = json!({
            "model": model().as_str(),
            "stream": false,
            "instructions": long_system,
            "max_output_tokens": 32,
            "input": [{ "type": "message", "role": "user",
                "content": [{ "type": "input_text", "text": format!("第 {round} 次：说 ok") }] }]
        });
        let mut upstream = open(&request, settings("", "")).await;
        let raw = upstream.read_body().await.unwrap();
        let response = to_responses(&raw, &request);
        let read = response["usage"]["cache_read_input_tokens"]
            .as_u64()
            .unwrap_or(0);
        println!("[cache] round={round} usage={}", response["usage"]);
        cache_reads.push(read);
    }
    println!("[cache] cache_read 序列={cache_reads:?}");
}
