//! Anthropic Messages 请求 → Responses 请求（网关的中间格式）。
//!
//! Claude Code 说 Anthropic Messages；网关把它转成 Responses 后交给现有代理链路，
//! 由那边再转成 Chat / Anthropic / 原生 Responses 发往上游。
//!
//! 思考块的签名：上游是 OpenAI 系时，Anthropic 的 `signature` 没有意义，
//! 网关把 reasoning 的 `encrypted_content` 编码进签名（`cpxgw1.` 前缀），下一轮
//! Claude Code 原样带回来时再解出来，推理上下文就能跨轮保留。

use base64::Engine;
use serde_json::{Map, Value, json};

/// 网关签发的思考块签名前缀。
pub const SIGNATURE_PREFIX: &str = "cpxgw1.";

/// Claude Code 在 system 里放的计费标记行，每次请求都变，会打散上游的 prompt cache。
const BILLING_HEADER_PREFIX: &str = "x-anthropic-billing-header:";

/// 把 reasoning 的不透明内容编码成思考块签名。
pub fn encode_signature(encrypted_content: Option<&str>) -> String {
    let payload = json!({ "e": encrypted_content.unwrap_or_default() });
    format!(
        "{SIGNATURE_PREFIX}{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload.to_string())
    )
}

/// 解出网关签发的签名；不是网关签的（真 Anthropic 签名）返回 `None`。
pub fn decode_signature(signature: &str) -> Option<Option<String>> {
    let encoded = signature.strip_prefix(SIGNATURE_PREFIX)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded)
        .ok()?;
    let payload: Value = serde_json::from_slice(&bytes).ok()?;
    let encrypted = payload
        .get("e")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    Some(encrypted)
}

pub fn is_gateway_signature(signature: &str) -> bool {
    signature.starts_with(SIGNATURE_PREFIX)
}

/// 请求是否开启了思考（`thinking.type` 为 enabled / adaptive）。
pub fn thinking_enabled(request: &Value) -> bool {
    matches!(
        request.pointer("/thinking/type").and_then(Value::as_str),
        Some("enabled" | "adaptive")
    )
}

/// 推理强度：优先 `output_config.effort`，其次按 `thinking.budget_tokens` 估算。
fn reasoning_effort(request: &Value) -> Option<&'static str> {
    if let Some(effort) = request
        .pointer("/output_config/effort")
        .and_then(Value::as_str)
    {
        return Some(match effort.trim() {
            "low" => "low",
            "medium" => "medium",
            "high" | "max" | "xhigh" => "high",
            _ => "medium",
        });
    }
    if !thinking_enabled(request) {
        return None;
    }
    let budget = request
        .pointer("/thinking/budget_tokens")
        .and_then(Value::as_u64);
    Some(match budget {
        Some(budget) if budget < 4_096 => "low",
        Some(budget) if budget < 16_384 => "medium",
        Some(_) => "high",
        None => "medium",
    })
}

fn system_text(system: &Value) -> String {
    match system {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .filter(|text| !text.trim_start().starts_with(BILLING_HEADER_PREFIX))
            .collect::<Vec<_>>()
            .join("\n\n"),
        _ => String::new(),
    }
}

fn image_url(source: &Value) -> Option<String> {
    match source.get("type").and_then(Value::as_str) {
        Some("base64") => {
            let media = source
                .get("media_type")
                .and_then(Value::as_str)
                .unwrap_or("image/png");
            let data = source.get("data").and_then(Value::as_str)?;
            Some(format!("data:{media};base64,{data}"))
        }
        Some("url") => source
            .get("url")
            .and_then(Value::as_str)
            .map(str::to_string),
        _ => None,
    }
}

/// user 侧的普通内容块 → Responses 的 input 内容部件。
fn user_part(block: &Value) -> Option<Value> {
    match block.get("type").and_then(Value::as_str) {
        Some("text") => Some(json!({
            "type": "input_text",
            "text": block.get("text").and_then(Value::as_str).unwrap_or_default()
        })),
        Some("image") => {
            let url = image_url(block.get("source")?)?;
            Some(json!({ "type": "input_image", "image_url": url }))
        }
        Some("document") => {
            let source = block.get("source")?;
            match source.get("type").and_then(Value::as_str) {
                Some("base64") => {
                    let media = source
                        .get("media_type")
                        .and_then(Value::as_str)
                        .unwrap_or("application/pdf");
                    let data = source.get("data").and_then(Value::as_str)?;
                    Some(json!({
                        "type": "input_file",
                        "filename": block.get("title").and_then(Value::as_str).unwrap_or("document.pdf"),
                        "file_data": format!("data:{media};base64,{data}")
                    }))
                }
                Some("text") => Some(json!({
                    "type": "input_text",
                    "text": source.get("data").and_then(Value::as_str).unwrap_or_default()
                })),
                _ => None,
            }
        }
        _ => None,
    }
}

/// tool_result 的内容 → function_call_output 的 output（纯文本时给字符串，带图时给部件数组）。
fn tool_result_output(block: &Value) -> Value {
    let is_error = block.get("is_error").and_then(Value::as_bool) == Some(true);
    let prefix = |text: String| {
        if is_error && !text.trim_start().to_ascii_lowercase().starts_with("error") {
            format!("Error: {text}")
        } else {
            text
        }
    };
    match block.get("content") {
        Some(Value::String(text)) => Value::String(prefix(text.clone())),
        Some(Value::Array(parts)) => {
            let converted = parts.iter().filter_map(user_part).collect::<Vec<_>>();
            if converted
                .iter()
                .all(|part| part.get("type").and_then(Value::as_str) == Some("input_text"))
            {
                let text = converted
                    .iter()
                    .filter_map(|part| part.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n");
                Value::String(prefix(text))
            } else {
                Value::Array(converted)
            }
        }
        _ => Value::String(prefix(String::new())),
    }
}

fn convert_user_message(content: &Value, input: &mut Vec<Value>) {
    let blocks = match content {
        Value::String(text) => {
            input.push(json!({
                "type": "message",
                "role": "user",
                "content": [{ "type": "input_text", "text": text }]
            }));
            return;
        }
        Value::Array(blocks) => blocks,
        _ => return,
    };
    // tool_result 是对上一轮工具调用的回答，必须紧跟在调用之后，排在本轮其它内容前面。
    let mut parts = Vec::new();
    for block in blocks {
        if block.get("type").and_then(Value::as_str) == Some("tool_result") {
            input.push(json!({
                "type": "function_call_output",
                "call_id": block.get("tool_use_id").and_then(Value::as_str).unwrap_or_default(),
                "output": tool_result_output(block)
            }));
        } else if let Some(part) = user_part(block) {
            parts.push(part);
        }
    }
    if !parts.is_empty() {
        input.push(json!({ "type": "message", "role": "user", "content": parts }));
    }
}

fn flush_assistant_text(parts: &mut Vec<Value>, input: &mut Vec<Value>) {
    if parts.is_empty() {
        return;
    }
    input.push(json!({
        "type": "message",
        "role": "assistant",
        "content": std::mem::take(parts)
    }));
}

fn convert_assistant_message(content: &Value, input: &mut Vec<Value>) {
    let blocks = match content {
        Value::String(text) => {
            input.push(json!({
                "type": "message",
                "role": "assistant",
                "content": [{ "type": "output_text", "text": text }]
            }));
            return;
        }
        Value::Array(blocks) => blocks,
        _ => return,
    };
    let mut parts = Vec::new();
    for block in blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                let text = block
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if !text.is_empty() {
                    parts.push(json!({ "type": "output_text", "text": text }));
                }
            }
            Some("thinking") => {
                flush_assistant_text(&mut parts, input);
                let thinking = block
                    .get("thinking")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let signature = block
                    .get("signature")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let encrypted = decode_signature(signature).flatten();
                if thinking.trim().is_empty() && encrypted.is_none() {
                    continue;
                }
                let mut item = json!({
                    "type": "reasoning",
                    "summary": if thinking.is_empty() {
                        json!([])
                    } else {
                        json!([{ "type": "summary_text", "text": thinking }])
                    }
                });
                if let Some(encrypted) = encrypted {
                    item["encrypted_content"] = json!(encrypted);
                }
                input.push(item);
            }
            Some("tool_use") => {
                flush_assistant_text(&mut parts, input);
                let arguments = block
                    .get("input")
                    .map(Value::to_string)
                    .unwrap_or_else(|| "{}".to_string());
                input.push(json!({
                    "type": "function_call",
                    "call_id": block.get("id").and_then(Value::as_str).unwrap_or_default(),
                    "name": block.get("name").and_then(Value::as_str).unwrap_or_default(),
                    "arguments": arguments
                }));
            }
            // redacted_thinking、服务端工具结果等没有对应物，丢弃。
            _ => {}
        }
    }
    flush_assistant_text(&mut parts, input);
}

fn convert_tools(tools: &[Value]) -> Vec<Value> {
    tools
        .iter()
        .filter_map(|tool| {
            let kind = tool.get("type").and_then(Value::as_str).unwrap_or("custom");
            if kind.starts_with("web_search") {
                // 服务端搜索：映射成 Responses 原生 web_search；Chat 上游会在代理里被剔除。
                return Some(json!({ "type": "web_search" }));
            }
            let name = tool.get("name").and_then(Value::as_str)?;
            if kind != "custom" && tool.get("input_schema").is_none() {
                // 其它 Anthropic 服务端工具（bash_*/text_editor_* 等）没有 schema，上游无法执行。
                return None;
            }
            let mut converted = json!({
                "type": "function",
                "name": name,
                "parameters": tool
                    .get("input_schema")
                    .cloned()
                    .unwrap_or_else(|| json!({ "type": "object", "properties": {} })),
                "strict": false
            });
            if let Some(description) = tool.get("description").and_then(Value::as_str) {
                converted["description"] = json!(description);
            }
            Some(converted)
        })
        .collect()
}

fn convert_tool_choice(choice: &Value) -> Option<Value> {
    match choice.get("type").and_then(Value::as_str)? {
        "auto" => Some(json!("auto")),
        "any" => Some(json!("required")),
        "none" => Some(json!("none")),
        "tool" => Some(json!({
            "type": "function",
            "name": choice.get("name").and_then(Value::as_str).unwrap_or_default()
        })),
        _ => None,
    }
}

/// Anthropic Messages 请求体 → Responses 请求体。
pub fn anthropic_to_responses(request: &Value) -> anyhow::Result<Value> {
    let object = request
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("请求体不是 JSON 对象"))?;
    let mut body = Map::new();
    body.insert(
        "model".to_string(),
        object.get("model").cloned().unwrap_or(json!("")),
    );
    if let Some(system) = object.get("system") {
        let text = system_text(system);
        if !text.trim().is_empty() {
            body.insert("instructions".to_string(), json!(text));
        }
    }
    let mut input = Vec::new();
    for message in object
        .get("messages")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        let content = message.get("content").unwrap_or(&Value::Null);
        match message.get("role").and_then(Value::as_str) {
            Some("assistant") => convert_assistant_message(content, &mut input),
            _ => convert_user_message(content, &mut input),
        }
    }
    body.insert("input".to_string(), Value::Array(input));

    if let Some(tools) = object.get("tools").and_then(Value::as_array) {
        let tools = convert_tools(tools);
        if !tools.is_empty() {
            body.insert("tools".to_string(), Value::Array(tools));
            if let Some(choice) = object.get("tool_choice").and_then(convert_tool_choice) {
                body.insert("tool_choice".to_string(), choice);
            }
            if object
                .get("tool_choice")
                .and_then(|choice| choice.get("disable_parallel_tool_use"))
                .and_then(Value::as_bool)
                == Some(true)
            {
                body.insert("parallel_tool_calls".to_string(), json!(false));
            }
        }
    }
    if let Some(max_tokens) = object.get("max_tokens").and_then(Value::as_u64) {
        body.insert("max_output_tokens".to_string(), json!(max_tokens));
    }
    let effort = reasoning_effort(request);
    if let Some(effort) = effort {
        body.insert(
            "reasoning".to_string(),
            json!({ "effort": effort, "summary": "auto" }),
        );
        body.insert(
            "include".to_string(),
            json!(["reasoning.encrypted_content"]),
        );
    } else {
        // 推理模型会拒绝 temperature/top_p；只在没开思考时透传。
        for key in ["temperature", "top_p"] {
            if let Some(value) = object.get(key) {
                body.insert(key.to_string(), value.clone());
            }
        }
    }
    body.insert("store".to_string(), json!(false));
    body.insert(
        "stream".to_string(),
        json!(
            object
                .get("stream")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        ),
    );
    Ok(Value::Object(body))
}

/// 发往 Responses 上游前：没有不透明推理内容的 reasoning 项会被原生上游拒绝（缺 id），去掉它。
/// Chat 上游需要保留摘要文本（DeepSeek 等要回传 reasoning_content）。
pub fn strip_unreplayable_reasoning(body: &mut Value) {
    if let Some(items) = body.get_mut("input").and_then(Value::as_array_mut) {
        items.retain(|item| {
            item.get("type").and_then(Value::as_str) != Some("reasoning")
                || item
                    .get("encrypted_content")
                    .and_then(Value::as_str)
                    .is_some_and(|value| !value.trim().is_empty())
        });
    }
}

/// 原样转发给 Anthropic 上游前：去掉网关签发的思考块（真 Anthropic 不认这个签名）。
///
/// 最后一条 assistant 消息里的思考块被去掉时，Anthropic 会要求工具调用前必须有思考块，
/// 这时把本轮的 `thinking` 参数一并关掉。
pub fn sanitize_for_native_anthropic(request: &mut Value) {
    let Some(messages) = request.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };
    let last_assistant = messages
        .iter()
        .rposition(|message| message.get("role").and_then(Value::as_str) == Some("assistant"));
    let mut stripped_in_last = false;
    for (index, message) in messages.iter_mut().enumerate() {
        let Some(blocks) = message.get_mut("content").and_then(Value::as_array_mut) else {
            continue;
        };
        let before = blocks.len();
        blocks.retain(|block| {
            block.get("type").and_then(Value::as_str) != Some("thinking")
                || !block
                    .get("signature")
                    .and_then(Value::as_str)
                    .is_some_and(is_gateway_signature)
        });
        if blocks.len() != before && Some(index) == last_assistant {
            stripped_in_last = true;
        }
    }
    if stripped_in_last && let Some(object) = request.as_object_mut() {
        object.remove("thinking");
    }
}

/// count_tokens 的本地估算（上游不是 Anthropic 时用）。
pub fn estimate_input_tokens(request: &Value) -> u64 {
    let mut chars = 0_usize;
    if let Some(system) = request.get("system") {
        chars += system_text(system).chars().count();
    }
    for key in ["messages", "tools"] {
        if let Some(value) = request.get(key) {
            chars += value.to_string().chars().count();
        }
    }
    crate::protocol_proxy::estimated_tokens_from_chars(chars)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_blocks_join_and_drop_billing_header() {
        let request = json!({
            "model": "claude-sonnet-5",
            "system": [
                { "type": "text", "text": "x-anthropic-billing-header: cc_version=2.1; cch=abc;" },
                { "type": "text", "text": "You are Claude Code." },
                { "type": "text", "text": "Be concise.", "cache_control": { "type": "ephemeral" } }
            ],
            "messages": [{ "role": "user", "content": "hi" }]
        });
        let body = anthropic_to_responses(&request).unwrap();
        assert_eq!(body["instructions"], "You are Claude Code.\n\nBe concise.");
        assert_eq!(body["input"][0]["content"][0]["text"], "hi");
        assert_eq!(body["store"], false);
    }

    #[test]
    fn tool_round_trip_keeps_order_and_pairing() {
        let request = json!({
            "model": "m",
            "max_tokens": 32000,
            "messages": [
                { "role": "user", "content": [{ "type": "text", "text": "列一下目录" }] },
                { "role": "assistant", "content": [
                    { "type": "text", "text": "好的" },
                    { "type": "tool_use", "id": "toolu_1", "name": "Bash", "input": { "command": "ls" } }
                ]},
                { "role": "user", "content": [
                    { "type": "tool_result", "tool_use_id": "toolu_1", "content": [{ "type": "text", "text": "a.txt" }] },
                    { "type": "text", "text": "继续" }
                ]}
            ],
            "tools": [{ "name": "Bash", "description": "run", "input_schema": { "type": "object", "properties": { "command": { "type": "string" } } } }],
            "tool_choice": { "type": "auto", "disable_parallel_tool_use": true }
        });
        let body = anthropic_to_responses(&request).unwrap();
        let input = body["input"].as_array().unwrap();
        let kinds = input
            .iter()
            .map(|item| {
                format!(
                    "{}:{}",
                    item["type"].as_str().unwrap(),
                    item.get("role").and_then(Value::as_str).unwrap_or("")
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            vec![
                "message:user",
                "message:assistant",
                "function_call:",
                "function_call_output:",
                "message:user"
            ]
        );
        assert_eq!(input[2]["call_id"], "toolu_1");
        assert_eq!(input[2]["arguments"], "{\"command\":\"ls\"}");
        assert_eq!(input[3]["output"], "a.txt");
        assert_eq!(body["tools"][0]["name"], "Bash");
        assert_eq!(body["tools"][0]["strict"], false);
        assert_eq!(body["tool_choice"], "auto");
        assert_eq!(body["parallel_tool_calls"], false);
        assert_eq!(body["max_output_tokens"], 32000);
    }

    #[test]
    fn tool_choice_variants() {
        for (choice, expected) in [
            (json!({ "type": "any" }), json!("required")),
            (json!({ "type": "none" }), json!("none")),
            (
                json!({ "type": "tool", "name": "Read" }),
                json!({ "type": "function", "name": "Read" }),
            ),
        ] {
            let request = json!({
                "model": "m",
                "messages": [],
                "tools": [{ "name": "Read", "input_schema": { "type": "object" } }],
                "tool_choice": choice
            });
            assert_eq!(
                anthropic_to_responses(&request).unwrap()["tool_choice"],
                expected
            );
        }
    }

    #[test]
    fn error_tool_results_and_images() {
        let request = json!({
            "model": "m",
            "messages": [{ "role": "user", "content": [
                { "type": "tool_result", "tool_use_id": "t1", "is_error": true, "content": "file not found" },
                { "type": "tool_result", "tool_use_id": "t2", "content": [
                    { "type": "text", "text": "截图" },
                    { "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": "AAAA" } }
                ]},
                { "type": "image", "source": { "type": "url", "url": "https://example.test/a.png" } }
            ]}]
        });
        let body = anthropic_to_responses(&request).unwrap();
        let input = body["input"].as_array().unwrap();
        assert_eq!(input[0]["output"], "Error: file not found");
        assert_eq!(input[1]["output"][1]["type"], "input_image");
        assert_eq!(
            input[1]["output"][1]["image_url"],
            "data:image/png;base64,AAAA"
        );
        assert_eq!(
            input[2]["content"][0]["image_url"],
            "https://example.test/a.png"
        );
    }

    #[test]
    fn thinking_maps_to_reasoning_and_signature_tunnel_round_trips() {
        let signature = encode_signature(Some("gAAAA-opaque"));
        assert!(is_gateway_signature(&signature));
        assert_eq!(
            decode_signature(&signature),
            Some(Some("gAAAA-opaque".to_string()))
        );
        assert_eq!(decode_signature("EqQBCkgIBxABGAIi-real-anthropic"), None);

        let request = json!({
            "model": "m",
            "thinking": { "type": "enabled", "budget_tokens": 10000 },
            "temperature": 1,
            "messages": [
                { "role": "user", "content": "q" },
                { "role": "assistant", "content": [
                    { "type": "thinking", "thinking": "先想想", "signature": signature },
                    { "type": "thinking", "thinking": "真签名", "signature": "EqQBreal" },
                    { "type": "redacted_thinking", "data": "xxx" },
                    { "type": "text", "text": "答" }
                ]}
            ]
        });
        let body = anthropic_to_responses(&request).unwrap();
        assert_eq!(body["reasoning"]["effort"], "medium");
        assert_eq!(body["include"][0], "reasoning.encrypted_content");
        assert!(body.get("temperature").is_none());
        let input = body["input"].as_array().unwrap();
        assert_eq!(input[1]["type"], "reasoning");
        assert_eq!(input[1]["encrypted_content"], "gAAAA-opaque");
        assert_eq!(input[1]["summary"][0]["text"], "先想想");
        assert_eq!(input[2]["type"], "reasoning");
        assert!(input[2].get("encrypted_content").is_none());
        assert_eq!(input[3]["role"], "assistant");

        let mut stripped = body.clone();
        strip_unreplayable_reasoning(&mut stripped);
        let kinds = stripped["input"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["type"].as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        assert_eq!(kinds, vec!["message", "reasoning", "message"]);
    }

    #[test]
    fn effort_mapping() {
        for (request, expected) in [
            (
                json!({ "thinking": { "type": "enabled", "budget_tokens": 1024 } }),
                Some("low"),
            ),
            (
                json!({ "thinking": { "type": "enabled", "budget_tokens": 31999 } }),
                Some("high"),
            ),
            (
                json!({ "thinking": { "type": "adaptive" } }),
                Some("medium"),
            ),
            (
                json!({ "output_config": { "effort": "max" } }),
                Some("high"),
            ),
            (json!({ "thinking": { "type": "disabled" } }), None),
            (json!({}), None),
        ] {
            assert_eq!(reasoning_effort(&request), expected, "{request}");
        }
    }

    #[test]
    fn web_search_server_tool_maps_and_schemaless_server_tools_drop() {
        let request = json!({
            "model": "m",
            "messages": [],
            "tools": [
                { "type": "web_search_20250305", "name": "web_search", "max_uses": 5 },
                { "type": "bash_20250124", "name": "bash" },
                { "name": "Read", "input_schema": { "type": "object" } }
            ]
        });
        let body = anthropic_to_responses(&request).unwrap();
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0]["type"], "web_search");
        assert_eq!(tools[1]["name"], "Read");
    }

    #[test]
    fn native_sanitize_drops_gateway_thinking_and_disables_thinking_for_last_turn() {
        let mut request = json!({
            "thinking": { "type": "enabled", "budget_tokens": 2048 },
            "messages": [
                { "role": "user", "content": "q" },
                { "role": "assistant", "content": [
                    { "type": "thinking", "thinking": "x", "signature": encode_signature(None) },
                    { "type": "tool_use", "id": "t", "name": "Read", "input": {} }
                ]},
                { "role": "user", "content": [{ "type": "tool_result", "tool_use_id": "t", "content": "ok" }] }
            ]
        });
        sanitize_for_native_anthropic(&mut request);
        assert_eq!(
            request["messages"][1]["content"].as_array().unwrap().len(),
            1
        );
        assert!(request.get("thinking").is_none());

        let mut untouched = json!({
            "thinking": { "type": "enabled", "budget_tokens": 2048 },
            "messages": [{ "role": "assistant", "content": [
                { "type": "thinking", "thinking": "x", "signature": "EqQBreal" }
            ]}]
        });
        sanitize_for_native_anthropic(&mut untouched);
        assert!(untouched.get("thinking").is_some());
    }
}
