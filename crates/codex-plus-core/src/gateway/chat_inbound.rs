//! OpenAI Chat Completions 请求 → Responses 请求（网关的中间格式）。
//!
//! 说 Chat Completions 的 Agent 把请求发到网关后，这里转成 Responses，交给现有代理链路，
//! 由那边再转成 Chat / Anthropic / 原生 Responses 发往上游。
//!
//! 推理内容：DeepSeek 等上游要求带工具调用的 assistant 消息回传 `reasoning_content`，
//! 所以历史里的 `reasoning_content` / `reasoning` 转成只有摘要文本的 reasoning 项，而不是丢弃：
//! - 上游是 Chat 时，代理会把它还原成 `reasoning_content`；
//! - 上游是原生 Responses 时，没有 `encrypted_content` 的 reasoning 项会在管道里被
//!   [`super::anthropic_inbound::strip_unreplayable_reasoning`] 去掉，不会因缺 id 被拒。

use std::collections::{HashMap, VecDeque};

use anyhow::bail;
use serde_json::{Map, Value, json};

/// 消息内容里的纯文本（字符串，或只取文本部件）。
fn plain_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| match part {
                Value::String(text) => Some(text.as_str()),
                _ => part.get("text").and_then(Value::as_str),
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// user / tool 侧的内容部件 → Responses 的 input 内容部件。
fn user_part(part: &Value) -> anyhow::Result<Option<Value>> {
    if let Value::String(text) = part {
        return Ok(Some(json!({ "type": "input_text", "text": text })));
    }
    let kind = part.get("type").and_then(Value::as_str).unwrap_or("text");
    Ok(match kind {
        "text" | "input_text" => Some(json!({
            "type": "input_text",
            "text": part.get("text").and_then(Value::as_str).unwrap_or_default()
        })),
        "image_url" | "input_image" => {
            let image = part.get("image_url").unwrap_or(&Value::Null);
            let (url, detail) = match image {
                Value::String(url) => (Some(url.as_str()), part.get("detail")),
                _ => (
                    image.get("url").and_then(Value::as_str),
                    image.get("detail"),
                ),
            };
            let Some(url) = url.filter(|url| !url.trim().is_empty()) else {
                bail!("image_url 部件缺少 url");
            };
            let mut converted = json!({ "type": "input_image", "image_url": url });
            if let Some(detail) = detail.and_then(Value::as_str) {
                converted["detail"] = json!(detail);
            }
            Some(converted)
        }
        "file" | "input_file" => {
            let file = part.get("file").unwrap_or(part);
            let mut converted = Map::new();
            converted.insert("type".to_string(), json!("input_file"));
            for key in ["file_data", "file_id", "filename", "file_url"] {
                if let Some(value) = file.get(key).and_then(Value::as_str) {
                    converted.insert(key.to_string(), json!(value));
                }
            }
            if !["file_data", "file_id", "file_url"]
                .iter()
                .any(|key| converted.contains_key(*key))
            {
                bail!("file 部件缺少 file_data / file_id");
            }
            if converted.contains_key("file_data") && !converted.contains_key("filename") {
                converted.insert("filename".to_string(), json!("file"));
            }
            Some(Value::Object(converted))
        }
        "input_audio" => bail!(
            "暂不支持音频输入（input_audio）：网关的 Responses 管道无法转发音频，请先转写成文字"
        ),
        other => bail!("不支持的消息内容类型：{other}"),
    })
}

fn convert_user_message(content: &Value, input: &mut Vec<Value>) -> anyhow::Result<()> {
    let parts = match content {
        Value::String(text) => vec![json!({ "type": "input_text", "text": text })],
        Value::Array(parts) => {
            let mut converted = Vec::new();
            for part in parts {
                if let Some(part) = user_part(part)? {
                    converted.push(part);
                }
            }
            converted
        }
        Value::Null => Vec::new(),
        _ => bail!("user 消息的 content 必须是字符串或数组"),
    };
    if !parts.is_empty() {
        input.push(json!({ "type": "message", "role": "user", "content": parts }));
    }
    Ok(())
}

/// assistant 消息里的推理文本（DeepSeek 的 `reasoning_content`，OpenRouter 等的 `reasoning`）。
fn assistant_reasoning(message: &Value) -> Option<&str> {
    ["reasoning_content", "reasoning"]
        .iter()
        .filter_map(|key| message.get(*key).and_then(Value::as_str))
        .find(|text| !text.trim().is_empty())
}

/// assistant 历史：推理 → 文字 → 工具调用，与上游原本的输出顺序一致。
fn convert_assistant_message(
    message: &Value,
    legacy_calls: &mut LegacyCalls,
    input: &mut Vec<Value>,
) -> anyhow::Result<()> {
    if let Some(reasoning) = assistant_reasoning(message) {
        input.push(json!({
            "type": "reasoning",
            "summary": [{ "type": "summary_text", "text": reasoning }]
        }));
    }
    let mut parts = Vec::new();
    match message.get("content").unwrap_or(&Value::Null) {
        Value::String(text) if !text.is_empty() => {
            parts.push(json!({ "type": "output_text", "text": text }));
        }
        Value::Array(blocks) => {
            for block in blocks {
                let field = |key: &str| {
                    block
                        .get(key)
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string()
                };
                match block.get("type").and_then(Value::as_str) {
                    Some("refusal") => {
                        parts.push(json!({ "type": "refusal", "refusal": field("refusal") }));
                    }
                    _ => {
                        let text = match block {
                            Value::String(text) => text.clone(),
                            _ => field("text"),
                        };
                        if !text.is_empty() {
                            parts.push(json!({ "type": "output_text", "text": text }));
                        }
                    }
                }
            }
        }
        _ => {}
    }
    if let Some(refusal) = message.get("refusal").and_then(Value::as_str)
        && !refusal.is_empty()
    {
        parts.push(json!({ "type": "refusal", "refusal": refusal }));
    }
    if !parts.is_empty() {
        input.push(json!({ "type": "message", "role": "assistant", "content": parts }));
    }
    for call in message
        .get("tool_calls")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        let kind = call
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("function");
        if kind != "function" {
            bail!("不支持的工具调用类型：{kind}");
        }
        let function = call.get("function").unwrap_or(&Value::Null);
        input.push(json!({
            "type": "function_call",
            "call_id": call.get("id").and_then(Value::as_str).unwrap_or_default(),
            "name": function.get("name").and_then(Value::as_str).unwrap_or_default(),
            "arguments": arguments_string(function.get("arguments"))
        }));
    }
    // 旧版 function_call（没有 id）：生成 call_id，并记下来给后面的 function 角色消息配对。
    if let Some(function) = message
        .get("function_call")
        .filter(|value| value.is_object())
    {
        let name = function
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let call_id = legacy_calls.open(name);
        input.push(json!({
            "type": "function_call",
            "call_id": call_id,
            "name": name,
            "arguments": arguments_string(function.get("arguments"))
        }));
    }
    Ok(())
}

fn arguments_string(arguments: Option<&Value>) -> String {
    match arguments {
        Some(Value::String(text)) if !text.trim().is_empty() => text.clone(),
        Some(Value::Object(_)) => arguments.map(Value::to_string).unwrap_or_default(),
        _ => "{}".to_string(),
    }
}

/// tool 消息的内容 → function_call_output 的 output（纯文本给字符串，带图给部件数组）。
fn tool_output(content: &Value) -> anyhow::Result<Value> {
    let Value::Array(parts) = content else {
        return Ok(json!(plain_text(content)));
    };
    let mut converted = Vec::new();
    for part in parts {
        if let Some(part) = user_part(part)? {
            converted.push(part);
        }
    }
    if converted
        .iter()
        .all(|part| part.get("type").and_then(Value::as_str) == Some("input_text"))
    {
        Ok(json!(
            converted
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        ))
    } else {
        Ok(Value::Array(converted))
    }
}

/// 旧版 `function_call` / `function` 角色的配对：按名字先进先出。
#[derive(Default)]
struct LegacyCalls {
    pending: HashMap<String, VecDeque<String>>,
    counter: usize,
}

impl LegacyCalls {
    fn open(&mut self, name: &str) -> String {
        let call_id = format!("call_legacy_{}", self.counter);
        self.counter += 1;
        self.pending
            .entry(name.to_string())
            .or_default()
            .push_back(call_id.clone());
        call_id
    }

    fn close(&mut self, name: &str) -> String {
        self.pending
            .get_mut(name)
            .and_then(VecDeque::pop_front)
            .unwrap_or_else(|| {
                let call_id = format!("call_legacy_{}", self.counter);
                self.counter += 1;
                call_id
            })
    }
}

fn convert_function_tool(function: &Value, strict_default: Option<&Value>) -> Option<Value> {
    let name = function.get("name").and_then(Value::as_str)?;
    let mut converted = json!({
        "type": "function",
        "name": name,
        "parameters": function
            .get("parameters")
            .filter(|value| value.is_object())
            .cloned()
            .unwrap_or_else(|| json!({ "type": "object", "properties": {} })),
        // Responses 的 function 工具默认 strict=true，会要求 schema 满足严格模式；Chat 默认是 false。
        "strict": function
            .get("strict")
            .or(strict_default)
            .and_then(Value::as_bool)
            .unwrap_or(false)
    });
    if let Some(description) = function.get("description").and_then(Value::as_str) {
        converted["description"] = json!(description);
    }
    Some(converted)
}

/// Chat 的 tools → Responses 的 function 工具；非 function 工具（custom 等）上游没有对应执行方，丢弃。
fn convert_tools(tools: &[Value]) -> Vec<Value> {
    tools
        .iter()
        .filter(|tool| {
            tool.get("type")
                .and_then(Value::as_str)
                .unwrap_or("function")
                == "function"
        })
        .filter_map(|tool| convert_function_tool(tool.get("function")?, tool.get("strict")))
        .collect()
}

fn convert_tool_choice(choice: &Value) -> Option<Value> {
    match choice {
        Value::String(mode) => match mode.as_str() {
            "auto" | "none" | "required" => Some(json!(mode)),
            _ => None,
        },
        Value::Object(_) => {
            let name = choice
                .pointer("/function/name")
                .or_else(|| choice.get("name"))
                .and_then(Value::as_str)?;
            Some(json!({ "type": "function", "name": name }))
        }
        _ => None,
    }
}

/// 推理强度：Chat 的 `reasoning_effort`，或 OpenRouter 风格的 `reasoning.effort`。
fn reasoning_effort(object: &Map<String, Value>) -> Option<String> {
    let effort = object
        .get("reasoning_effort")
        .or_else(|| {
            object
                .get("reasoning")
                .and_then(|value| value.get("effort"))
        })
        .and_then(Value::as_str)?
        .trim()
        .to_ascii_lowercase();
    Some(match effort.as_str() {
        "none" | "minimal" | "low" | "medium" | "high" | "xhigh" => effort,
        "max" => "xhigh".to_string(),
        _ => "medium".to_string(),
    })
}

fn convert_response_format(format: &Value) -> Option<Value> {
    match format.get("type").and_then(Value::as_str)? {
        "json_schema" => {
            let schema = format.get("json_schema")?;
            let mut converted = json!({
                "type": "json_schema",
                "name": schema.get("name").and_then(Value::as_str).unwrap_or("response"),
                "schema": schema.get("schema").cloned().unwrap_or_else(|| json!({}))
            });
            for key in ["strict", "description"] {
                if let Some(value) = schema.get(key) {
                    converted[key] = value.clone();
                }
            }
            Some(converted)
        }
        "json_object" => Some(json!({ "type": "json_object" })),
        "text" => Some(json!({ "type": "text" })),
        _ => None,
    }
}

/// Chat Completions 请求体 → Responses 请求体。
pub fn chat_to_responses(request: &Value) -> anyhow::Result<Value> {
    let object = request
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("请求体不是 JSON 对象"))?;
    if let Some(n) = object.get("n").and_then(Value::as_u64)
        && n > 1
    {
        bail!("网关不支持 n>1（一次生成多个候选），请改为 n=1");
    }
    let mut body = Map::new();
    body.insert(
        "model".to_string(),
        object.get("model").cloned().unwrap_or(json!("")),
    );

    // 开头连续的 system / developer 消息拼成 instructions；对话中途插入的保留为 developer 消息，位置不变。
    let mut instructions = Vec::new();
    let mut input = Vec::new();
    let mut legacy_calls = LegacyCalls::default();
    for message in object
        .get("messages")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        let content = message.get("content").unwrap_or(&Value::Null);
        match message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("user")
        {
            "system" | "developer" => {
                let text = plain_text(content);
                if text.trim().is_empty() {
                    continue;
                }
                if input.is_empty() {
                    instructions.push(text);
                } else {
                    input.push(json!({
                        "type": "message",
                        "role": "developer",
                        "content": [{ "type": "input_text", "text": text }]
                    }));
                }
            }
            "user" => convert_user_message(content, &mut input)?,
            "assistant" => convert_assistant_message(message, &mut legacy_calls, &mut input)?,
            "tool" => input.push(json!({
                "type": "function_call_output",
                "call_id": message.get("tool_call_id").and_then(Value::as_str).unwrap_or_default(),
                "output": tool_output(content)?
            })),
            "function" => {
                let name = message
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                input.push(json!({
                    "type": "function_call_output",
                    "call_id": legacy_calls.close(name),
                    "output": plain_text(content)
                }));
            }
            other => bail!("不支持的消息角色：{other}"),
        }
    }
    if !instructions.is_empty() {
        body.insert("instructions".to_string(), json!(instructions.join("\n\n")));
    }
    body.insert("input".to_string(), Value::Array(input));

    // 工具：新版 tools / tool_choice，兼容旧版 functions / function_call。
    let tools = match object.get("tools").and_then(Value::as_array) {
        Some(tools) => convert_tools(tools),
        None => object
            .get("functions")
            .and_then(Value::as_array)
            .map(|functions| {
                functions
                    .iter()
                    .filter_map(|function| convert_function_tool(function, None))
                    .collect()
            })
            .unwrap_or_default(),
    };
    if !tools.is_empty() {
        body.insert("tools".to_string(), Value::Array(tools));
        if let Some(choice) = object
            .get("tool_choice")
            .or_else(|| object.get("function_call"))
            .and_then(convert_tool_choice)
        {
            body.insert("tool_choice".to_string(), choice);
        }
        if let Some(parallel) = object.get("parallel_tool_calls").and_then(Value::as_bool) {
            body.insert("parallel_tool_calls".to_string(), json!(parallel));
        }
    }

    if let Some(max_tokens) = object
        .get("max_completion_tokens")
        .and_then(Value::as_u64)
        .or_else(|| object.get("max_tokens").and_then(Value::as_u64))
    {
        body.insert("max_output_tokens".to_string(), json!(max_tokens));
    }
    for key in ["temperature", "top_p"] {
        if let Some(value) = object.get(key).filter(|value| value.is_number()) {
            body.insert(key.to_string(), value.clone());
        }
    }
    if let Some(effort) = reasoning_effort(object) {
        // summary=auto：上游才会回推理摘要，转回 Chat 时放进 reasoning_content。
        body.insert(
            "reasoning".to_string(),
            json!({ "effort": effort, "summary": "auto" }),
        );
    }
    let mut text = Map::new();
    if let Some(format) = object
        .get("response_format")
        .and_then(convert_response_format)
    {
        text.insert("format".to_string(), format);
    }
    if let Some(verbosity) = object.get("verbosity").and_then(Value::as_str) {
        text.insert("verbosity".to_string(), json!(verbosity));
    }
    if !text.is_empty() {
        body.insert("text".to_string(), Value::Object(text));
    }
    // Responses 也认的字段原样带上；stop / seed / penalty / logprobs 等 Responses 没有，丢弃。
    for key in [
        "user",
        "prompt_cache_key",
        "safety_identifier",
        "service_tier",
    ] {
        if let Some(value) = object.get(key).filter(|value| value.is_string()) {
            body.insert(key.to_string(), value.clone());
        }
    }
    if let Some(metadata) = object.get("metadata").filter(|value| value.is_object()) {
        body.insert("metadata".to_string(), metadata.clone());
    }
    body.insert("store".to_string(), json!(false));
    // stream_options 是 Chat 专有的，include_usage 由出站转换器自己处理。
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

/// 输入 token 的本地粗估（上游没回 usage 时用）。
pub fn estimate_input_tokens(request: &Value) -> u64 {
    let chars = ["messages", "tools", "functions"]
        .iter()
        .filter_map(|key| request.get(*key))
        .map(|value| value.to_string().chars().count())
        .sum();
    crate::protocol_proxy::estimated_tokens_from_chars(chars)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(body: &Value) -> Vec<String> {
        body["input"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| {
                format!(
                    "{}:{}",
                    item["type"].as_str().unwrap(),
                    item.get("role").and_then(Value::as_str).unwrap_or("")
                )
            })
            .collect()
    }

    #[test]
    fn system_messages_become_instructions_and_text_round_trips() {
        let request = json!({
            "model": "gpt-x",
            "messages": [
                { "role": "system", "content": "You are helpful." },
                { "role": "developer", "content": [{ "type": "text", "text": "Be concise." }] },
                { "role": "user", "content": "hi" },
                { "role": "assistant", "content": "hello" },
                { "role": "system", "content": "中途提醒" },
                { "role": "user", "content": [{ "type": "text", "text": "再说一遍" }] }
            ],
            "stream": true,
            "stream_options": { "include_usage": true },
            "temperature": 0.2,
            "top_p": 0.9,
            "max_tokens": 100,
            "user": "u-1",
            "stop": ["\n"],
            "seed": 7
        });
        let body = chat_to_responses(&request).unwrap();
        assert_eq!(body["model"], "gpt-x");
        assert_eq!(body["instructions"], "You are helpful.\n\nBe concise.");
        assert_eq!(
            kinds(&body),
            vec![
                "message:user",
                "message:assistant",
                "message:developer",
                "message:user"
            ]
        );
        assert_eq!(body["input"][0]["content"][0]["type"], "input_text");
        assert_eq!(body["input"][1]["content"][0]["type"], "output_text");
        assert_eq!(body["input"][2]["content"][0]["text"], "中途提醒");
        assert_eq!(body["stream"], true);
        assert_eq!(body["store"], false);
        assert_eq!(body["temperature"], 0.2);
        assert_eq!(body["top_p"], 0.9);
        assert_eq!(body["max_output_tokens"], 100);
        assert_eq!(body["user"], "u-1");
        for dropped in ["stream_options", "stop", "seed", "messages"] {
            assert!(body.get(dropped).is_none(), "{dropped} 应该被丢弃");
        }
    }

    #[test]
    fn max_completion_tokens_wins_over_max_tokens() {
        let body = chat_to_responses(&json!({
            "model": "m", "messages": [], "max_tokens": 10, "max_completion_tokens": 20
        }))
        .unwrap();
        assert_eq!(body["max_output_tokens"], 20);
    }

    #[test]
    fn multimodal_user_parts() {
        let request = json!({
            "model": "m",
            "messages": [{ "role": "user", "content": [
                { "type": "text", "text": "看图" },
                { "type": "image_url", "image_url": { "url": "data:image/png;base64,AAAA", "detail": "high" } },
                { "type": "image_url", "image_url": "https://example.test/a.png" },
                { "type": "file", "file": { "filename": "a.pdf", "file_data": "data:application/pdf;base64,JVBE" } }
            ]}]
        });
        let body = chat_to_responses(&request).unwrap();
        let parts = body["input"][0]["content"].as_array().unwrap();
        assert_eq!(parts[0]["text"], "看图");
        assert_eq!(parts[1]["type"], "input_image");
        assert_eq!(parts[1]["image_url"], "data:image/png;base64,AAAA");
        assert_eq!(parts[1]["detail"], "high");
        assert_eq!(parts[2]["image_url"], "https://example.test/a.png");
        assert_eq!(parts[3]["type"], "input_file");
        assert_eq!(parts[3]["filename"], "a.pdf");
        assert_eq!(parts[3]["file_data"], "data:application/pdf;base64,JVBE");
    }

    #[test]
    fn unsupported_inputs_report_clear_errors() {
        let audio = chat_to_responses(&json!({
            "model": "m",
            "messages": [{ "role": "user", "content": [
                { "type": "input_audio", "input_audio": { "data": "AAAA", "format": "wav" } }
            ]}]
        }))
        .unwrap_err();
        assert!(audio.to_string().contains("input_audio"), "{audio}");

        let n = chat_to_responses(&json!({ "model": "m", "messages": [], "n": 2 })).unwrap_err();
        assert!(n.to_string().contains("n>1"), "{n}");
        assert!(chat_to_responses(&json!({ "model": "m", "messages": [], "n": 1 })).is_ok());

        let role = chat_to_responses(&json!({
            "model": "m", "messages": [{ "role": "robot", "content": "x" }]
        }))
        .unwrap_err();
        assert!(role.to_string().contains("robot"), "{role}");

        assert!(chat_to_responses(&json!([])).is_err());
    }

    #[test]
    fn multi_turn_tool_calls_round_trip() {
        let request = json!({
            "model": "m",
            "messages": [
                { "role": "user", "content": "列一下目录再读 a.txt" },
                { "role": "assistant", "content": null, "reasoning_content": "先列目录",
                  "tool_calls": [
                    { "id": "call_1", "type": "function", "function": { "name": "Bash", "arguments": "{\"command\":\"ls\"}" } },
                    { "id": "call_2", "type": "function", "function": { "name": "Read", "arguments": "{\"file\":\"a.txt\"}" } }
                ]},
                { "role": "tool", "tool_call_id": "call_1", "content": "a.txt" },
                { "role": "tool", "tool_call_id": "call_2", "content": [{ "type": "text", "text": "内容" }] },
                { "role": "assistant", "content": "读完了" },
                { "role": "user", "content": "好" }
            ],
            "tools": [
                { "type": "function", "function": { "name": "Bash", "description": "run", "parameters": { "type": "object", "properties": { "command": { "type": "string" } } } } },
                { "type": "function", "function": { "name": "Read", "parameters": { "type": "object" }, "strict": true } },
                { "type": "custom", "custom": { "name": "patch" } }
            ],
            "tool_choice": "required",
            "parallel_tool_calls": false
        });
        let body = chat_to_responses(&request).unwrap();
        assert_eq!(
            kinds(&body),
            vec![
                "message:user",
                "reasoning:",
                "function_call:",
                "function_call:",
                "function_call_output:",
                "function_call_output:",
                "message:assistant",
                "message:user"
            ]
        );
        let input = body["input"].as_array().unwrap();
        assert_eq!(input[1]["summary"][0]["text"], "先列目录");
        assert!(input[1].get("encrypted_content").is_none());
        assert_eq!(input[2]["call_id"], "call_1");
        assert_eq!(input[2]["name"], "Bash");
        assert_eq!(input[2]["arguments"], "{\"command\":\"ls\"}");
        assert_eq!(input[3]["call_id"], "call_2");
        assert_eq!(input[4]["call_id"], "call_1");
        assert_eq!(input[4]["output"], "a.txt");
        assert_eq!(input[5]["output"], "内容");
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 2, "custom 工具被丢弃");
        assert_eq!(tools[0]["type"], "function");
        assert_eq!(tools[0]["name"], "Bash");
        assert_eq!(tools[0]["description"], "run");
        assert_eq!(tools[0]["strict"], false);
        assert_eq!(tools[1]["strict"], true);
        assert_eq!(body["tool_choice"], "required");
        assert_eq!(body["parallel_tool_calls"], false);

        // 没有 encrypted_content 的 reasoning 项发往原生 Responses 前会被去掉。
        let mut stripped = body.clone();
        crate::gateway::anthropic_inbound::strip_unreplayable_reasoning(&mut stripped);
        assert!(
            stripped["input"]
                .as_array()
                .unwrap()
                .iter()
                .all(|item| item["type"] != "reasoning")
        );
    }

    #[test]
    fn tool_choice_variants() {
        for (choice, expected) in [
            (json!("auto"), json!("auto")),
            (json!("none"), json!("none")),
            (json!("required"), json!("required")),
            (
                json!({ "type": "function", "function": { "name": "Read" } }),
                json!({ "type": "function", "name": "Read" }),
            ),
        ] {
            let request = json!({
                "model": "m",
                "messages": [],
                "tools": [{ "type": "function", "function": { "name": "Read" } }],
                "tool_choice": choice
            });
            assert_eq!(
                chat_to_responses(&request).unwrap()["tool_choice"],
                expected
            );
        }
        // 没有工具时不带 tool_choice。
        let body =
            chat_to_responses(&json!({ "model": "m", "messages": [], "tool_choice": "auto" }))
                .unwrap();
        assert!(body.get("tool_choice").is_none());
        assert!(body.get("tools").is_none());
    }

    #[test]
    fn legacy_functions_pair_by_name() {
        let request = json!({
            "model": "m",
            "functions": [{ "name": "lookup", "parameters": { "type": "object" } }],
            "function_call": { "name": "lookup" },
            "messages": [
                { "role": "user", "content": "q" },
                { "role": "assistant", "content": null, "function_call": { "name": "lookup", "arguments": "{\"k\":1}" } },
                { "role": "function", "name": "lookup", "content": "v" }
            ]
        });
        let body = chat_to_responses(&request).unwrap();
        let input = body["input"].as_array().unwrap();
        assert_eq!(input[1]["type"], "function_call");
        assert_eq!(input[2]["type"], "function_call_output");
        assert_eq!(input[1]["call_id"], input[2]["call_id"]);
        assert_eq!(body["tools"][0]["name"], "lookup");
        assert_eq!(
            body["tool_choice"],
            json!({ "type": "function", "name": "lookup" })
        );
    }

    #[test]
    fn reasoning_effort_and_response_format() {
        let body = chat_to_responses(&json!({
            "model": "m",
            "messages": [],
            "reasoning_effort": "high",
            "response_format": {
                "type": "json_schema",
                "json_schema": { "name": "answer", "strict": true, "schema": { "type": "object" } }
            }
        }))
        .unwrap();
        assert_eq!(body["reasoning"]["effort"], "high");
        assert_eq!(body["reasoning"]["summary"], "auto");
        assert_eq!(body["text"]["format"]["type"], "json_schema");
        assert_eq!(body["text"]["format"]["name"], "answer");
        assert_eq!(body["text"]["format"]["strict"], true);
        assert_eq!(body["text"]["format"]["schema"]["type"], "object");

        let body = chat_to_responses(&json!({
            "model": "m",
            "messages": [],
            "reasoning": { "effort": "minimal" },
            "response_format": { "type": "json_object" }
        }))
        .unwrap();
        assert_eq!(body["reasoning"]["effort"], "minimal");
        assert_eq!(body["text"]["format"]["type"], "json_object");

        let plain = chat_to_responses(&json!({ "model": "m", "messages": [] })).unwrap();
        assert!(plain.get("reasoning").is_none());
        assert!(plain.get("text").is_none());
    }

    #[test]
    fn assistant_reasoning_field_variants() {
        let body = chat_to_responses(&json!({
            "model": "m",
            "messages": [
                { "role": "user", "content": "q" },
                { "role": "assistant", "content": "a", "reasoning": "OpenRouter 风格" },
                { "role": "assistant", "content": "b", "reasoning_content": "" }
            ]
        }))
        .unwrap();
        assert_eq!(
            kinds(&body),
            vec![
                "message:user",
                "reasoning:",
                "message:assistant",
                "message:assistant"
            ]
        );
        assert_eq!(body["input"][1]["summary"][0]["text"], "OpenRouter 风格");
    }

    #[test]
    fn estimate_counts_messages_and_tools() {
        let small =
            estimate_input_tokens(&json!({ "messages": [{ "role": "user", "content": "hi" }] }));
        let large = estimate_input_tokens(&json!({
            "messages": [{ "role": "user", "content": "hi".repeat(400) }],
            "tools": [{ "type": "function", "function": { "name": "x" } }]
        }));
        assert!(small > 0);
        assert!(large > small + 150);
    }
}
