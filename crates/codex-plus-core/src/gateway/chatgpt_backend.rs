//! 发往 ChatGPT 订阅后端（chatgpt.com/backend-api/codex）的请求，整理成 Codex CLI 会发的样子。
//!
//! 这个后端只服务 Codex 自己的客户端：不收 `system` 角色、不收采样参数和存储相关的字段、
//! 只支持流式、`store` 必须为 false，较新的模型还要求请求像是 Codex CLI 发出的（UA / version 头）。
//! 其它 Agent（Grok、Claude Code 经转换后）的请求在这里补齐这些差异。

use std::collections::HashMap;
use std::path::Path;

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

/// 后端拒绝的字段：采样参数、续接 / 存储相关、显式缓存选项。
const REJECTED_FIELDS: &[&str] = &[
    "max_output_tokens",
    "max_completion_tokens",
    "temperature",
    "top_p",
    "previous_response_id",
    "user",
    "safety_identifier",
    "prompt_cache_options",
];

/// 调用 id 的长度上限；超过的用 sha256 代替（同一个 id 每次得到同一个哈希，调用与输出仍能配对）。
const MAX_CALL_ID: usize = 64;

/// 整理请求体。`codex_prompt` 是该模型的官方指令（读不到时为 None，保留客户端自己的指令）。
pub fn adapt_request(body: &mut Value, codex_prompt: Option<&str>) {
    let Some(object) = body.as_object_mut() else {
        return;
    };
    for key in REJECTED_FIELDS {
        object.remove(*key);
    }
    // 只保留 Codex 快速档会发的服务等级。
    let tier = object.get("service_tier").and_then(Value::as_str);
    if !matches!(tier, Some("priority" | "ultrafast")) {
        object.remove("service_tier");
    }

    let mut input = match object.remove("input") {
        Some(Value::String(text)) => vec![json!({
            "type": "message",
            "role": "user",
            "content": [{ "type": "input_text", "text": text }],
        })],
        Some(Value::Array(items)) => items,
        _ => Vec::new(),
    };
    input = adapt_input(input);

    let own = object
        .get("instructions")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let has_cache_key = object
        .get("prompt_cache_key")
        .and_then(Value::as_str)
        .is_some_and(|key| !key.is_empty());
    if !has_cache_key && let Some(key) = conversation_key(&own, &input) {
        object.insert("prompt_cache_key".to_string(), Value::String(key));
    }

    // 指令用该模型的官方指令；客户端自己的指令作为第一条 developer 消息放进 input。
    // 客户端本来就是 Codex（指令第一行一致）时保持原样。
    if let Some(prompt) = codex_prompt.filter(|prompt| !prompt.trim().is_empty()) {
        if !own.trim().is_empty() && first_line(&own) != first_line(prompt) {
            input.insert(
                0,
                json!({
                    "type": "message",
                    "role": "developer",
                    "content": [{ "type": "input_text", "text": own }],
                }),
            );
            object.insert(
                "instructions".to_string(),
                Value::String(prompt.to_string()),
            );
        } else if own.trim().is_empty() {
            object.insert(
                "instructions".to_string(),
                Value::String(prompt.to_string()),
            );
        }
    }
    object.insert("input".to_string(), Value::Array(input));
    object.insert("store".to_string(), Value::Bool(false));
    object.insert("stream".to_string(), Value::Bool(true));
    object.insert("tool_choice".to_string(), Value::String("auto".to_string()));
    object
        .entry("parallel_tool_calls")
        .or_insert(Value::Bool(true));
    object
        .entry("text")
        .or_insert_with(|| json!({ "verbosity": "medium" }));

    // 推理：保留加密内容，跨轮次回放；关掉推理时不要。
    let effort = object
        .get("reasoning")
        .and_then(|reasoning| reasoning.get("effort"))
        .and_then(Value::as_str)
        .map(str::to_string);
    if let Some(reasoning) = object.get_mut("reasoning").and_then(Value::as_object_mut) {
        if effort.as_deref() == Some("ultra") {
            reasoning.insert("effort".to_string(), Value::String("max".to_string()));
        }
        if effort.as_deref() != Some("none") {
            reasoning
                .entry("summary")
                .or_insert_with(|| Value::String("auto".to_string()));
        }
    }
    if effort.as_deref() == Some("none") {
        object.remove("include");
    } else {
        let mut include = object
            .get("include")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if !include
            .iter()
            .any(|item| item.as_str() == Some("reasoning.encrypted_content"))
        {
            include.push(Value::String("reasoning.encrypted_content".to_string()));
        }
        object.insert("include".to_string(), Value::Array(include));
    }
}

/// 调用 id 超过 64 个字符时换成它的 sha256。
pub fn bound_call_id(id: &str) -> String {
    if id.len() <= MAX_CALL_ID {
        return id.to_string();
    }
    let digest = Sha256::digest(id.trim().as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// 整理输入项：不引用已存的项、不带 id（以及跟着 id 的 status）、调用 id 不超长、
/// 找不到调用的工具输出改成一条说明、`system` 角色改成 `developer`。
fn adapt_input(input: Vec<Value>) -> Vec<Value> {
    const CALL_TYPES: &[&str] = &[
        "function_call",
        "function_call_output",
        "local_shell_call",
        "local_shell_call_output",
        "custom_tool_call",
        "custom_tool_call_output",
        "tool_search_call",
        "tool_search_output",
    ];
    let mut input = input;
    let mut calls: HashMap<String, &'static str> = HashMap::new();
    for item in &mut input {
        let Some(object) = item.as_object_mut() else {
            continue;
        };
        let kind = object
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if !CALL_TYPES.contains(&kind.as_str()) {
            continue;
        }
        let id = object
            .get("call_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let bounded = bound_call_id(&id);
        if bounded != id {
            object.insert("call_id".to_string(), Value::String(bounded.clone()));
        }
        let output = match kind.as_str() {
            "function_call" => Some("function_call_output"),
            "local_shell_call" => Some("local_shell_call_output"),
            "custom_tool_call" => Some("custom_tool_call_output"),
            _ => None,
        };
        if let Some(output) = output {
            calls.insert(bounded.trim().to_string(), output);
        }
    }

    let mut out = Vec::with_capacity(input.len());
    for item in input {
        let Value::Object(mut object) = item else {
            out.push(item);
            continue;
        };
        let kind = object
            .get("type")
            .and_then(Value::as_str)
            .map(str::to_string);
        if kind.as_deref() == Some("item_reference") {
            continue;
        }
        let call_id = object
            .get("call_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        // 没有调用 id 的独立通知（Codex 自己会发）原样保留。
        if matches!(
            kind.as_deref(),
            Some("function_call_output" | "custom_tool_call_output")
        ) && call_id.is_empty()
        {
            out.push(Value::Object(object));
            continue;
        }
        object.remove("id");
        if matches!(kind.as_deref(), None | Some("message") | Some("reasoning")) {
            object.remove("status");
        }
        if let Some(kind) = kind.as_deref()
            && matches!(
                kind,
                "function_call_output" | "custom_tool_call_output" | "local_shell_call_output"
            )
        {
            let want = calls.get(&call_id).copied();
            let paired = want == Some(kind)
                || (kind == "function_call_output" && want == Some("local_shell_call_output"));
            if !paired {
                object = orphan_output(&object, &call_id);
            }
        }
        let is_message = matches!(kind.as_deref(), None | Some("message"));
        if is_message && object.get("role").and_then(Value::as_str) == Some("system") {
            object.insert("role".to_string(), Value::String("developer".to_string()));
        }
        out.push(Value::Object(object));
    }
    out
}

/// 找不到调用的工具输出：改成助手的一条说明，内容保留。
fn orphan_output(object: &Map<String, Value>, call_id: &str) -> Map<String, Value> {
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .unwrap_or("tool");
    let output = match object.get("output") {
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
        None => String::new(),
    };
    let call_id = if call_id.is_empty() {
        "unknown"
    } else {
        call_id
    };
    let mut message = Map::new();
    message.insert("type".to_string(), json!("message"));
    message.insert("role".to_string(), json!("assistant"));
    message.insert(
        "content".to_string(),
        json!([{ "type": "output_text", "text": format!("[{name} output for call {call_id}]\n{output}") }]),
    );
    message
}

/// 没有 `prompt_cache_key` 的客户端：按对话开头（指令 + 第一条用户消息为止的输入）算一个固定的键，
/// 每一轮都相同，上游的提示词缓存才能命中。还没有用户消息时不给。
pub fn conversation_key(instructions: &str, input: &[Value]) -> Option<String> {
    let mut hasher = Sha256::new();
    hasher.update(instructions.as_bytes());
    let mut seen_user = false;
    for item in input {
        hasher.update(serde_json::to_vec(item).unwrap_or_default());
        if item.get("role").and_then(Value::as_str) == Some("user") {
            seen_user = true;
            break;
        }
    }
    if !seen_user {
        return None;
    }
    let hex: String = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Some(format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    ))
}

fn first_line(text: &str) -> &str {
    text.trim().lines().next().unwrap_or_default().trim()
}

/// Codex CLI 的模型缓存（`~/.codex/models_cache.json`）里某个模型的官方指令。
pub fn codex_prompt_from_cache(codex_home: &Path, model: &str) -> Option<String> {
    let text = std::fs::read_to_string(codex_home.join("models_cache.json")).ok()?;
    codex_prompt_from_models(&serde_json::from_str(&text).ok()?, model)
}

pub fn codex_prompt_from_models(value: &Value, model: &str) -> Option<String> {
    let models = value.get("models")?.as_array()?;
    let entry = models
        .iter()
        .find(|entry| entry.get("slug").and_then(Value::as_str) == Some(model))?;
    entry
        .pointer("/model_messages/instructions_template")
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .or_else(|| {
            entry
                .get("base_instructions")
                .and_then(Value::as_str)
                .filter(|text| !text.trim().is_empty())
        })
        .map(str::to_string)
}

/// Codex CLI 的版本：读模型缓存里记录的，没有就用内置的。
pub fn codex_version(codex_home: &Path) -> String {
    std::fs::read_to_string(codex_home.join("models_cache.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|value| {
            value
                .get("client_version")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .filter(|version| {
            version
                .split('.')
                .all(|part| !part.is_empty() && part.chars().all(|ch| ch.is_ascii_digit()))
        })
        .unwrap_or_else(|| crate::model_catalog::OFFICIAL_MODELS_CLIENT_VERSION.to_string())
}

/// Codex CLI 的 User-Agent：`codex_cli_rs/0.159.0 (Mac OS 26.0.0; arm64) Apple_Terminal/455`。
pub fn codex_user_agent(version: &str) -> String {
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x86_64",
        other => other,
    };
    let (os, terminal) = match std::env::consts::OS {
        "macos" => ("Mac OS", "Apple_Terminal/455"),
        "windows" => ("Windows", "WindowsTerminal"),
        _ => ("Linux", "xterm-256color"),
    };
    format!("codex_cli_rs/{version} ({os}; {arch}) {terminal}")
}

/// 把一段 Responses SSE 里 `response.completed`（或 `response.incomplete`）携带的完整响应取出来；
/// 后端只给流式结果，客户端要非流式时用它拼回 JSON。
pub fn responses_json_from_sse(body: &[u8]) -> Option<Value> {
    let text = String::from_utf8_lossy(body);
    let mut last = None;
    for block in text.split("\n\n") {
        let data = block
            .lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .map(str::trim_start)
            .collect::<Vec<_>>()
            .join("\n");
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(&data) else {
            continue;
        };
        if matches!(
            value.get("type").and_then(Value::as_str),
            Some("response.completed" | "response.incomplete" | "response.done")
        ) && let Some(response) = value.get("response")
        {
            last = Some(response.clone());
        }
    }
    last
}

/// 「用 ChatGPT 登录」（ChatGPT API 订阅）不接受的 Responses 字段。`previous_response_id` 也去掉：
/// 不存储，每轮都发完整对话。
const API_REJECTED_FIELDS: &[&str] = &[
    "background",
    "conversation",
    "max_output_tokens",
    "max_tool_calls",
    "metadata",
    "moderation",
    "multi_agent",
    "prompt",
    "prompt_cache_retention",
    "safety_identifier",
    "temperature",
    "top_logprobs",
    "top_p",
    "truncation",
    "user",
    "previous_response_id",
];

/// OpenAI 自己执行的工具；ChatGPT 登录的 token 不能用，带上会整条请求被拒。
const API_HOSTED_TOOLS: &[&str] = &[
    "image_generation",
    "file_search",
    "code_interpreter",
    "computer_use",
    "computer_use_preview",
    "computer",
    "mcp",
    "tool_search",
];

/// 发往 api.openai.com 的 ChatGPT API 订阅请求：不存储、只流式，去掉不接受的字段和托管工具，
/// `system` 消息改成 `developer`。与 Codex 订阅不同，指令保留 Agent 自己的。
pub fn adapt_api_request(body: &mut Value) {
    let Some(object) = body.as_object_mut() else {
        return;
    };
    for key in API_REJECTED_FIELDS {
        object.remove(*key);
    }
    object.insert("store".to_string(), Value::Bool(false));
    object.insert("stream".to_string(), Value::Bool(true));
    match object.get_mut("input") {
        Some(Value::String(text)) => {
            let text = std::mem::take(text);
            object.insert(
                "input".to_string(),
                json!([{ "type": "message", "role": "user", "content": [{ "type": "input_text", "text": text }] }]),
            );
        }
        Some(Value::Array(items)) => {
            for item in items.iter_mut() {
                let Some(item) = item.as_object_mut() else {
                    continue;
                };
                if item.get("role").and_then(Value::as_str) == Some("system") {
                    item.insert("role".to_string(), json!("developer"));
                }
                // 工具以输入项的形式交过来（additional_tools）时也要去掉托管工具。
                if item.get("type").and_then(Value::as_str) == Some("additional_tools")
                    && let Some(Value::Array(tools)) = item.get_mut("tools")
                {
                    tools.retain(|tool| !is_hosted_tool(tool));
                }
            }
        }
        _ => {}
    }
    // 按对话命名缓存键：OpenAI 按它保留 ChatGPT 登录的提示词缓存。
    let has_cache_key = object
        .get("prompt_cache_key")
        .and_then(Value::as_str)
        .is_some_and(|key| !key.is_empty());
    if !has_cache_key {
        let own = object
            .get("instructions")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let input = object
            .get("input")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        if let Some(key) = conversation_key(own, input) {
            object.insert("prompt_cache_key".to_string(), Value::String(key));
        }
    }
    if let Some(Value::Array(tools)) = object.get_mut("tools") {
        tools.retain(|tool| !is_hosted_tool(tool));
        if tools.is_empty() {
            object.remove("tools");
            object.remove("tool_choice");
        }
    }
}

fn is_hosted_tool(tool: &Value) -> bool {
    tool.get("type")
        .and_then(Value::as_str)
        .is_some_and(|kind| API_HOSTED_TOOLS.contains(&kind))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_requests_keep_own_instructions_and_drop_hosted_tools() {
        let mut body = json!({
            "model": "gpt-6.1-sol",
            "instructions": "You are Grok.",
            "temperature": 0.2,
            "max_output_tokens": 100,
            "previous_response_id": "resp_1",
            "stream": false,
            "input": [
                { "type": "message", "role": "system", "content": [{ "type": "input_text", "text": "规则" }] },
                { "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "hi" }] }
            ],
            "tools": [{ "type": "web_search" }, { "type": "code_interpreter" }, { "type": "function", "name": "f" }],
            "tool_choice": "auto"
        });
        adapt_api_request(&mut body);
        assert_eq!(
            body["instructions"], "You are Grok.",
            "指令保留 Agent 自己的"
        );
        assert_eq!(body["store"], false);
        assert_eq!(body["stream"], true);
        for key in ["temperature", "max_output_tokens", "previous_response_id"] {
            assert!(body.get(key).is_none(), "{key}");
        }
        assert_eq!(body["input"][0]["role"], "developer");
        let tools: Vec<&str> = body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["type"].as_str().unwrap())
            .collect();
        assert_eq!(tools, vec!["web_search", "function"]);
        assert!(
            body["prompt_cache_key"]
                .as_str()
                .is_some_and(|key| key.len() == 36)
        );

        let mut only_hosted =
            json!({ "input": "hi", "tools": [{ "type": "mcp" }], "tool_choice": "auto" });
        adapt_api_request(&mut only_hosted);
        assert!(only_hosted.get("tools").is_none());
        assert!(only_hosted.get("tool_choice").is_none());
        assert_eq!(only_hosted["input"][0]["content"][0]["text"], "hi");
    }

    #[test]
    fn requests_are_reshaped_like_codex_cli_sends_them() {
        let mut body = json!({
            "model": "gpt-6.1-sol",
            "instructions": "You are Grok.\nBe brief.",
            "input": [
                { "type": "message", "role": "system", "content": [{ "type": "input_text", "text": "rules" }] },
                { "id": "msg_1", "status": "completed", "type": "message", "role": "user", "content": "hi" },
                { "type": "item_reference", "id": "rs_1" },
                { "type": "function_call", "id": "fc_1", "call_id": "c".repeat(80), "name": "bash", "arguments": "{}" },
                { "type": "function_call_output", "call_id": "c".repeat(80), "output": "ok" },
                { "type": "function_call_output", "call_id": "lost", "name": "read", "output": "data" }
            ],
            "temperature": 0.2,
            "max_output_tokens": 100,
            "previous_response_id": "resp_1",
            "service_tier": "flex",
            "store": true,
            "stream": false,
            "reasoning": { "effort": "high" }
        });
        adapt_request(&mut body, Some("You are Codex.\nOfficial."));
        for key in [
            "temperature",
            "max_output_tokens",
            "previous_response_id",
            "service_tier",
        ] {
            assert!(body.get(key).is_none(), "{key}");
        }
        assert_eq!(body["store"], false);
        assert_eq!(body["stream"], true);
        assert_eq!(body["instructions"], "You are Codex.\nOfficial.");
        assert_eq!(body["reasoning"]["summary"], "auto");
        assert_eq!(body["include"], json!(["reasoning.encrypted_content"]));
        assert!(body["prompt_cache_key"].as_str().unwrap().len() == 36);
        let input = body["input"].as_array().unwrap();
        // 客户端自己的指令排在最前，作为 developer 消息。
        assert_eq!(input[0]["role"], "developer");
        assert_eq!(input[0]["content"][0]["text"], "You are Grok.\nBe brief.");
        assert_eq!(input[1]["role"], "developer", "system → developer");
        assert!(input[2].get("id").is_none() && input[2].get("status").is_none());
        assert!(input.iter().all(|item| item["type"] != "item_reference"));
        let call = &input[3];
        assert!(call.get("id").is_none());
        assert_eq!(call["call_id"].as_str().unwrap().len(), 64);
        assert_eq!(input[4]["call_id"], call["call_id"], "调用与输出仍然配对");
        assert_eq!(input[5]["type"], "message", "找不到调用的输出改成说明");
        assert_eq!(input[5]["role"], "assistant");
        assert!(
            input[5]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("data")
        );
    }

    #[test]
    fn codex_requests_keep_their_own_instructions_and_text_input_becomes_a_message() {
        let mut body = json!({ "model": "m", "instructions": "You are Codex.\nlocal edits", "input": "hi", "reasoning": { "effort": "none" }, "include": ["x"] });
        adapt_request(&mut body, Some("You are Codex.\nOfficial."));
        assert_eq!(body["instructions"], "You are Codex.\nlocal edits");
        assert_eq!(body["input"][0]["role"], "user");
        assert_eq!(body["input"][0]["content"][0]["text"], "hi");
        assert!(body.get("include").is_none(), "关掉推理时不要加密内容");
        // 读不到官方指令：保留客户端的指令。
        let mut other = json!({ "instructions": "mine", "input": [] });
        adapt_request(&mut other, None);
        assert_eq!(other["instructions"], "mine");
        assert!(
            other.get("prompt_cache_key").is_none(),
            "还没有用户消息时不生成缓存键"
        );
    }

    #[test]
    fn prompts_and_versions_come_from_codex_models_cache() {
        let cache = json!({ "client_version": "0.170.2", "models": [
            { "slug": "gpt-6.1-sol", "model_messages": { "instructions_template": "T" }, "base_instructions": "B" },
            { "slug": "gpt-6-luna", "base_instructions": "B2" }
        ]});
        assert_eq!(
            codex_prompt_from_models(&cache, "gpt-6.1-sol").as_deref(),
            Some("T")
        );
        assert_eq!(
            codex_prompt_from_models(&cache, "gpt-6-luna").as_deref(),
            Some("B2")
        );
        assert_eq!(codex_prompt_from_models(&cache, "x"), None);
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("models_cache.json"), cache.to_string()).unwrap();
        assert_eq!(codex_version(temp.path()), "0.170.2");
        assert_eq!(
            codex_version(&temp.path().join("missing")),
            crate::model_catalog::OFFICIAL_MODELS_CLIENT_VERSION
        );
        assert!(codex_user_agent("0.170.2").starts_with("codex_cli_rs/0.170.2 ("));
    }

    #[test]
    fn completed_response_is_taken_from_sse() {
        let sse = b"event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"r\"}}\n\nevent: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"r\",\"output\":[1]}}\n\n";
        assert_eq!(responses_json_from_sse(sse).unwrap()["output"], json!([1]));
        assert!(responses_json_from_sse(b"{\"id\":\"x\"}").is_none());
    }
}
