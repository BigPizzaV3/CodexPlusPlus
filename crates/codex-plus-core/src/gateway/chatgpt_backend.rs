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
    adapt_request_for_transport(body, codex_prompt, false);
}

/// WS 请求带 `previous_response_id` 时是增量续接：上游按同一连接上的上一响应接前文，
/// 本次 input 只有新增项（常见的只有工具结果）。这类请求必须保留续接 ID，
/// 也不能把「调用在上一响应里」的工具输出当成孤立输出改写（#3151 / #3155）。
fn websocket_continuation(object: &Map<String, Value>, websocket: bool) -> bool {
    websocket
        && object
            .get("previous_response_id")
            .and_then(Value::as_str)
            .is_some_and(|id| !id.trim().is_empty())
}

/// `websocket` 为 true 时按 WS 传输整理：增量续接保留续接 ID 与工具输出原样。
/// HTTP 每轮都是完整历史，仍按原规则整理。
pub fn adapt_request_for_transport(body: &mut Value, codex_prompt: Option<&str>, websocket: bool) {
    let Some(object) = body.as_object_mut() else {
        return;
    };
    let continuation = websocket_continuation(object, websocket);
    for key in REJECTED_FIELDS {
        if continuation && *key == "previous_response_id" {
            continue;
        }
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
    input = adapt_input(input, continuation);

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
    // 增量续接的前文里已经有首轮插入的那条 developer 消息，不再重复插入。
    if let Some(prompt) = codex_prompt.filter(|prompt| !prompt.trim().is_empty()) {
        if !own.trim().is_empty() && first_line(&own) != first_line(prompt) {
            if !continuation {
                input.insert(
                    0,
                    json!({
                        "type": "message",
                        "role": "developer",
                        "content": [{ "type": "input_text", "text": own }],
                    }),
                );
            }
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
/// `continuation`（WS 增量续接）时调用可能在上一响应里，工具输出保持原样交给上游配对。
fn adapt_input(input: Vec<Value>, continuation: bool) -> Vec<Value> {
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
            if !paired && !continuation {
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

/// 官方请求用的客户端版本（`client_version` 参数、`version` 头、UA）。
///
/// 取 `models_cache.json` 的 `client_version` 与本机实际安装的 Codex CLI / Desktop 内置 CLI 版本中较新者，
/// 且不低于内置基线：旧缓存不能把官方目录限制在旧客户端能看到的模型上（#3178）。
/// 等待冷缓存探测；只有探测确认未找到安装版本时才只按文件缓存与基线取值。
pub async fn codex_version(codex_home: &Path) -> String {
    let cached = std::fs::read_to_string(codex_home.join("models_cache.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|value| {
            value
                .get("client_version")
                .and_then(Value::as_str)
                .map(str::to_string)
        });
    select_codex_version(
        cached.as_deref(),
        crate::codex_cli_version::installed_codex_version().await.as_deref(),
    )
}

/// 版本选择：缓存版本、安装版本、内置基线三者按 semver 取最新，非法值忽略。
/// 结果只保留 `X.Y.Z`（预发布版的 Codex CLI 请求官方目录时也这样发）。
pub fn select_codex_version(cached: Option<&str>, installed: Option<&str>) -> String {
    use crate::codex_cli_version::CodexVersion;
    let builtin = crate::model_catalog::OFFICIAL_MODELS_CLIENT_VERSION;
    [Some(builtin), cached, installed]
        .into_iter()
        .flatten()
        .filter_map(CodexVersion::parse)
        .max()
        .map(|version| version.whole())
        .unwrap_or_else(|| builtin.to_string())
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
    adapt_api_request_for_transport(body, false);
}

/// 同 [`adapt_api_request`]；WS 增量续接保留 `previous_response_id`（同一连接上的前文由上游保留）。
pub fn adapt_api_request_for_transport(body: &mut Value, websocket: bool) {
    let Some(object) = body.as_object_mut() else {
        return;
    };
    let continuation = websocket_continuation(object, websocket);
    for key in API_REJECTED_FIELDS {
        if continuation && *key == "previous_response_id" {
            continue;
        }
        object.remove(*key);
    }
    object.insert("store".to_string(), Value::Bool(false));
    object.insert("stream".to_string(), Value::Bool(true));
    let mut pending_reasoning: Option<Map<String, Value>> = None;
    match object.get_mut("input") {
        Some(Value::String(text)) => {
            let text = std::mem::take(text);
            object.insert(
                "input".to_string(),
                json!([{ "type": "message", "role": "user", "content": [{ "type": "input_text", "text": text }] }]),
            );
        }
        Some(Value::Array(items)) => {
            // 中途改思考强度时，新版 Codex 不改请求级 reasoning，而是插一条位置型的
            // configuration_update 输入项（保住提示词缓存）。用 ChatGPT 登录的共享额度不接受
            // 这种输入项，会直接 400（subscription_sharing_unsupported_capability，#2834）。
            // 去掉它们，把最后一条的设置写回请求级 reasoning：强度照样生效，只是这次切换不命中缓存。
            let mut configured: Option<Map<String, Value>> = None;
            items.retain(|item| {
                if item.get("type").and_then(Value::as_str) != Some("configuration_update") {
                    return true;
                }
                if let Some(Value::Object(reasoning)) = item.get("reasoning") {
                    configured.get_or_insert_with(Map::new).extend(
                        reasoning
                            .iter()
                            .map(|(key, value)| (key.clone(), value.clone())),
                    );
                }
                false
            });
            if let Some(configured) = configured {
                pending_reasoning = Some(configured);
            }
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
    if let Some(configured) = pending_reasoning {
        let reasoning = object
            .entry("reasoning")
            .or_insert_with(|| Value::Object(Map::new()));
        if !reasoning.is_object() {
            *reasoning = Value::Object(Map::new());
        }
        if let Some(reasoning) = reasoning.as_object_mut() {
            reasoning.extend(configured);
        }
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
    fn api_requests_fold_configuration_updates_into_request_reasoning() {
        // #2834：中途切换思考强度时 Codex 插入的 configuration_update 会被共享额度拒绝。
        let mut body = json!({
            "model": "gpt-6-luna",
            "reasoning": { "effort": "low", "summary": "auto" },
            "input": [
                { "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "test1" }] },
                { "type": "configuration_update", "reasoning": { "effort": "medium" } },
                { "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "test2" }] },
                { "type": "configuration_update", "reasoning": { "effort": "high" } },
                { "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "test3" }] }
            ]
        });
        adapt_api_request(&mut body);
        let types: Vec<&str> = body["input"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["type"].as_str().unwrap())
            .collect();
        assert_eq!(types, vec!["message", "message", "message"]);
        assert_eq!(
            body["reasoning"],
            json!({ "effort": "high", "summary": "auto" }),
            "最后一次切换生效，其余字段保留"
        );

        // 请求里原本没有 reasoning 时补上；没有 configuration_update 时不动 reasoning。
        let mut bare = json!({
            "input": [{ "type": "configuration_update", "reasoning": { "effort": "xhigh" } }]
        });
        adapt_api_request(&mut bare);
        assert_eq!(bare["reasoning"], json!({ "effort": "xhigh" }));
        assert_eq!(bare["input"], json!([]));

        let mut untouched = json!({ "reasoning": { "effort": "low" }, "input": "hi" });
        adapt_api_request(&mut untouched);
        assert_eq!(untouched["reasoning"], json!({ "effort": "low" }));
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
    fn websocket_continuations_keep_previous_response_and_tool_outputs() {
        // WS 增量续接：只带续接 ID 和上一响应里那次调用的结果（#3151 / #3155）。
        let continuation = || {
            json!({
                "model": "gpt-6.1-sol",
                "instructions": "You are Grok.",
                "previous_response_id": "resp_with_task",
                "input": [
                    { "type": "function_call_output", "call_id": "call_read", "output": "listing" },
                    { "type": "custom_tool_call_output", "call_id": "call_patch", "output": "done" }
                ],
                "temperature": 0.2
            })
        };
        let mut body = continuation();
        adapt_request_for_transport(&mut body, Some("You are Codex.\nOfficial."), true);
        assert_eq!(body["previous_response_id"], "resp_with_task");
        assert!(body.get("temperature").is_none(), "其它被拒字段照样去掉");
        let input = body["input"].as_array().unwrap();
        assert_eq!(input.len(), 2, "前文已有 developer 指令，续接不再插入");
        assert_eq!(input[0]["type"], "function_call_output");
        assert_eq!(input[0]["call_id"], "call_read");
        assert_eq!(input[1]["type"], "custom_tool_call_output");
        assert_eq!(input[1]["call_id"], "call_patch");
        assert_eq!(body["instructions"], "You are Codex.\nOfficial.");

        // HTTP 每轮是完整历史：仍去掉续接 ID，孤立输出照旧改写。
        let mut http = continuation();
        adapt_request_for_transport(&mut http, Some("You are Codex.\nOfficial."), false);
        assert!(http.get("previous_response_id").is_none());
        assert!(http["input"].as_array().unwrap().iter().all(|item| item["type"] == "message"));

        // WS 但没有续接 ID（首轮 / 完整上下文）：与 HTTP 相同。
        let mut fresh = continuation();
        fresh.as_object_mut().unwrap().remove("previous_response_id");
        adapt_request_for_transport(&mut fresh, Some("You are Codex.\nOfficial."), true);
        assert_eq!(fresh["input"][0]["role"], "developer");
        assert_eq!(fresh["input"][1]["type"], "message");

        // ChatGPT API 订阅通道同理。
        let mut api = continuation();
        adapt_api_request_for_transport(&mut api, true);
        assert_eq!(api["previous_response_id"], "resp_with_task");
        let mut api_http = continuation();
        adapt_api_request_for_transport(&mut api_http, false);
        assert!(api_http.get("previous_response_id").is_none());
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

    #[tokio::test]
    async fn prompts_and_versions_come_from_codex_models_cache() {
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
        // 安装版本由环境决定，这里只断言缓存版本是下限。
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("models_cache.json"), cache.to_string()).unwrap();
        assert_ne!(
            crate::codex_cli_version::compare(&codex_version(temp.path()).await, "0.170.2"),
            Some(std::cmp::Ordering::Less)
        );
        assert_eq!(
            select_codex_version(None, None),
            crate::model_catalog::OFFICIAL_MODELS_CLIENT_VERSION
        );
        assert!(codex_user_agent("0.170.2").starts_with("codex_cli_rs/0.170.2 ("));
    }

    #[tokio::test]
    async fn obsolete_or_invalid_cache_cannot_downgrade_official_requests() {
        let builtin = crate::model_catalog::OFFICIAL_MODELS_CLIENT_VERSION;
        for version in [
            "0.124.0", "0.99.9", "0", "1.2", "1.2.3.4", "9.bad.0", "9.0.0\n", "",
            "18446744073709551616.0.0",
        ] {
            assert_eq!(select_codex_version(Some(version), None), builtin, "{version}");
        }
        for version in [builtin, "0.170.2", "1.0.0"] {
            assert_eq!(select_codex_version(Some(version), None), version);
        }
        assert_eq!(select_codex_version(None, None), builtin);
        // 读不到缓存时 codex_version 也不改写任何文件。
        let temp = tempfile::tempdir().unwrap();
        let cache = json!({"client_version": "0.124.0"}).to_string();
        std::fs::write(temp.path().join("models_cache.json"), &cache).unwrap();
        let _ = codex_version(temp.path()).await;
        assert_eq!(
            std::fs::read_to_string(temp.path().join("models_cache.json")).unwrap(),
            cache,
            "版本选择不改写客户端缓存"
        );
    }

    #[test]
    fn newer_installed_codex_beats_an_old_models_cache() {
        // #3178：缓存停在 0.164.0，本机已装 0.169.1 → 用 0.169.1。
        assert_eq!(select_codex_version(Some("0.164.0"), Some("0.169.1")), "0.169.1");
        // Desktop 内置的预发布 CLI 也比旧缓存新；发出去只带 X.Y.Z。
        assert_eq!(
            select_codex_version(Some("0.164.0"), Some("0.169.0-alpha.11.1")),
            "0.169.0"
        );
        // 缓存更新时仍用缓存；安装版本是同号预发布时不降级。
        assert_eq!(select_codex_version(Some("0.170.2"), Some("0.169.1")), "0.170.2");
        assert_eq!(
            select_codex_version(Some("0.169.0"), Some("0.169.0-alpha.11.1")),
            "0.169.0"
        );
        // 安装版本低于基线或不合法时不起作用。
        let builtin = crate::model_catalog::OFFICIAL_MODELS_CLIENT_VERSION;
        assert_eq!(select_codex_version(None, Some("0.100.0")), builtin);
        assert_eq!(select_codex_version(Some("0.124.0"), Some("garbage")), builtin);
        // 没有缓存、只有安装版本。
        assert_eq!(select_codex_version(None, Some("0.169.1")), "0.169.1");
    }

    #[test]
    fn completed_response_is_taken_from_sse() {
        let sse = b"event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"r\"}}\n\nevent: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"r\",\"output\":[1]}}\n\n";
        assert_eq!(responses_json_from_sse(sse).unwrap()["output"], json!([1]));
        assert!(responses_json_from_sse(b"{\"id\":\"x\"}").is_none());
    }
}
