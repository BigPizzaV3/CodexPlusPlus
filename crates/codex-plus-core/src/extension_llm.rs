//! 拓展自己调模型（页面 `ctx.llm`、后台 / 工具里的 `llm`）：经 Codex++ 的协议代理，用当前的
//! 供应商与模型路由发一次 Responses 请求，把回复整理成 `{ text, model, usage }`。
//!
//! 这一跳不跑拓展的网关中间件（免得拓展自己的请求再被拓展改写、甚至递归）。
//! 上游可能是 Responses 也可能是 Chat Completions，流式与非流式都认。

use anyhow::Context;
use serde_json::{Value, json};

/// 单次调用的最长等待。
const LLM_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);
const MAX_INPUT_BYTES: usize = 1024 * 1024;

/// 拓展给的输入：字符串当一条用户消息；数组里是 `{ role, content | text }` 或现成的 Responses 条目。
fn input_items(input: &Value) -> anyhow::Result<Vec<Value>> {
    let message = |role: &str, text: &str| {
        let kind = if role == "assistant" { "output_text" } else { "input_text" };
        json!({ "type": "message", "role": role, "content": [{ "type": kind, "text": text }] })
    };
    Ok(match input {
        Value::String(text) => vec![message("user", text)],
        Value::Array(items) => items
            .iter()
            .map(|item| {
                if let Some(text) = item.as_str() {
                    return Ok(message("user", text));
                }
                let object = item.as_object().context("input 的每一项是字符串或 { role, content }")?;
                if object.get("type").and_then(Value::as_str).is_some() && object.get("role").is_none() {
                    return Ok(item.clone());
                }
                let role = match object.get("role").and_then(Value::as_str).unwrap_or("user") {
                    role @ ("user" | "assistant" | "developer" | "system") => role,
                    other => anyhow::bail!("不支持的角色 {other}"),
                };
                let role = if role == "system" { "developer" } else { role };
                match object.get("content").or_else(|| object.get("text")) {
                    Some(Value::String(text)) => Ok(message(role, text)),
                    Some(Value::Array(_)) => Ok(json!({ "type": "message", "role": role, "content": object["content"] })),
                    _ => anyhow::bail!("消息缺少 content"),
                }
            })
            .collect::<anyhow::Result<Vec<_>>>()?,
        _ => anyhow::bail!("input 需要是字符串或消息数组"),
    })
}

/// 当前 Codex 配置里的默认模型。
fn default_model() -> Option<String> {
    let text = std::fs::read_to_string(crate::codex_home::default_codex_home_dir().join("config.toml")).ok()?;
    let document = text.parse::<toml_edit::DocumentMut>().ok()?;
    document.get("model").and_then(|item| item.as_str()).map(str::to_string)
}

/// 拼请求体：`{ model?, input, instructions?, reasoningEffort?, maxOutputTokens?, temperature? }`。
pub fn request_body(params: &Value, fallback_model: Option<String>) -> anyhow::Result<Value> {
    let model = params
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(str::to_string)
        .or(fallback_model)
        .context("没有指定 model，Codex 配置里也没有默认模型")?;
    let input = input_items(params.get("input").context("缺少 input")?)?;
    let mut body = json!({ "model": model, "input": input, "stream": true, "store": false });
    if let Some(instructions) = params.get("instructions").and_then(Value::as_str).filter(|text| !text.is_empty()) {
        body["instructions"] = json!(instructions);
    }
    if let Some(effort) = params.get("reasoningEffort").and_then(Value::as_str) {
        body["reasoning"] = json!({ "effort": effort });
    }
    if let Some(limit) = params.get("maxOutputTokens").and_then(Value::as_u64) {
        body["max_output_tokens"] = json!(limit);
    }
    if let Some(temperature) = params.get("temperature").and_then(Value::as_f64) {
        body["temperature"] = json!(temperature);
    }
    // 函数工具：`[{ name, description, parameters }]` 或现成的 Responses 工具项；模型要调用时回复里给出 toolCalls。
    if let Some(tools) = params.get("tools").and_then(Value::as_array).filter(|tools| !tools.is_empty()) {
        let tools = tools
            .iter()
            .map(|tool| {
                let name = tool.get("name").and_then(Value::as_str).context("tools 的每一项要有 name")?;
                if tool.get("type").is_some() {
                    return Ok(tool.clone());
                }
                Ok(json!({
                    "type": "function",
                    "name": name,
                    "description": tool.get("description").cloned().unwrap_or(json!("")),
                    "parameters": tool.get("parameters").cloned().unwrap_or(json!({ "type": "object", "properties": {} })),
                }))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        body["tools"] = json!(tools);
        if let Some(choice) = params.get("toolChoice").filter(|choice| !choice.is_null()) {
            body["tool_choice"] = choice.clone();
        }
    }
    if body.to_string().len() > MAX_INPUT_BYTES {
        anyhow::bail!("请求超过 1 MiB");
    }
    Ok(body)
}

/// 从上游回复里取文字：Responses / Chat Completions，流式（SSE）或整段 JSON。
pub fn parse_reply(bytes: &[u8]) -> Value {
    let text = String::from_utf8_lossy(bytes);
    let mut out = String::new();
    let mut completed: Option<String> = None;
    let mut usage = Value::Null;
    let mut model = Value::Null;
    let mut error = Value::Null;
    // 工具调用：Responses 的 function_call 条目，或 Chat 的 tool_calls（流式按 index 拼参数）。
    let mut calls: Vec<Value> = Vec::new();
    let mut chat_calls: std::collections::BTreeMap<u64, (String, String, String)> = std::collections::BTreeMap::new();
    let mut reasoning = String::new();
    let mut absorb = |event: &Value, out: &mut String| {
        let mut push_call = |item: &Value| {
            if item.get("type").and_then(Value::as_str) == Some("function_call") {
                let call_id = item.get("call_id").or_else(|| item.get("id")).and_then(Value::as_str).unwrap_or_default().to_string();
                if !calls.iter().any(|call| call["callId"] == call_id.as_str()) {
                    calls.push(json!({ "callId": call_id, "name": item["name"], "arguments": item.get("arguments").cloned().unwrap_or(json!("")) }));
                }
            }
        };
        if event.get("type").and_then(Value::as_str) == Some("response.output_item.done") {
            push_call(&event["item"]);
        }
        if let Some(Some(delta)) = (event.get("type").and_then(Value::as_str) == Some("response.reasoning_summary_text.delta")).then(|| event["delta"].as_str()) {
            reasoning.push_str(delta);
        }
        for item in event.pointer("/response/output").and_then(Value::as_array).into_iter().flatten() {
            push_call(item);
        }
        if event.get("object").and_then(Value::as_str) == Some("response") {
            for item in event["output"].as_array().into_iter().flatten() {
                push_call(item);
            }
        }
        for choice in event.get("choices").and_then(Value::as_array).into_iter().flatten() {
            let tool_calls = choice.pointer("/delta/tool_calls").or_else(|| choice.pointer("/message/tool_calls"));
            for (position, call) in tool_calls.and_then(Value::as_array).into_iter().flatten().enumerate() {
                let index = call.get("index").and_then(Value::as_u64).unwrap_or(position as u64);
                let entry = chat_calls.entry(index).or_default();
                if let Some(id) = call.get("id").and_then(Value::as_str) {
                    entry.0 = id.to_string();
                }
                if let Some(name) = call.pointer("/function/name").and_then(Value::as_str) {
                    entry.1.push_str(name);
                }
                if let Some(arguments) = call.pointer("/function/arguments").and_then(Value::as_str) {
                    entry.2.push_str(arguments);
                }
            }
        }
        if let Some(found) = event.get("model").filter(|value| value.is_string()) {
            model = found.clone();
        }
        if let Some(found) = event.get("usage").filter(|value| !value.is_null()) {
            usage = found.clone();
        }
        match event.get("type").and_then(Value::as_str) {
            Some("response.output_text.delta") => out.push_str(event["delta"].as_str().unwrap_or_default()),
            Some("response.completed" | "response.done") => {
                let response = &event["response"];
                if let Some(found) = response.get("usage").filter(|value| !value.is_null()) {
                    usage = found.clone();
                }
                if let Some(found) = response.get("model").filter(|value| value.is_string()) {
                    model = found.clone();
                }
                completed = Some(output_text(response));
            }
            Some("response.failed" | "error") => {
                error = event.get("response").and_then(|response| response.get("error")).or_else(|| event.get("error")).cloned().unwrap_or_else(|| event.clone());
            }
            _ => {}
        }
        // Chat Completions：流式 delta 或整段 message。
        for choice in event.get("choices").and_then(Value::as_array).into_iter().flatten() {
            if let Some(delta) = choice.pointer("/delta/content").and_then(Value::as_str) {
                out.push_str(delta);
            } else if let Some(content) = choice.pointer("/message/content").and_then(Value::as_str) {
                out.push_str(content);
            }
        }
        // 整段的错误体：`{ error: {...} }`。
        if event.get("type").is_none() {
            if let Some(found) = event.get("error").filter(|value| !value.is_null()) {
                error = found.clone();
            }
        }
        // 非流式的 Responses 对象。
        if event.get("object").and_then(Value::as_str) == Some("response") {
            completed = Some(output_text(event));
        }
    };
    if let Ok(whole) = serde_json::from_str::<Value>(text.trim()) {
        absorb(&whole, &mut out);
    } else {
        for line in text.lines() {
            let Some(data) = line.strip_prefix("data:").map(str::trim) else { continue };
            if data == "[DONE]" {
                continue;
            }
            if let Ok(event) = serde_json::from_str::<Value>(data) {
                absorb(&event, &mut out);
            }
        }
    }
    let text = completed.filter(|text| !text.is_empty()).unwrap_or(out);
    calls.extend(chat_calls.into_values().map(|(id, name, arguments)| json!({ "callId": id, "name": name, "arguments": arguments })));
    let mut reply = json!({ "text": text, "model": model, "usage": usage, "error": error });
    if !calls.is_empty() {
        reply["toolCalls"] = json!(calls);
    }
    if !reasoning.is_empty() {
        reply["reasoning"] = json!(reasoning);
    }
    reply
}

fn output_text(response: &Value) -> String {
    response["output"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("message"))
        .flat_map(|item| item["content"].as_array().cloned().unwrap_or_default())
        .filter_map(|part| part.get("text").and_then(Value::as_str).map(str::to_string))
        .collect::<Vec<_>>()
        .join("")
}

/// 流式回复的增量解析：按行切 SSE（字节可能在任意处断开，攒到换行再解码），
/// 每个 `data:` 事件里的文字 / 推理增量交出来（Responses 与 Chat Completions 都认）。
#[derive(Debug, Default)]
pub struct SseDeltas {
    buffer: Vec<u8>,
}

impl SseDeltas {
    /// 喂一段字节，返回这段里完整事件的增量：`{ "type": "text" | "reasoning", "text": ... }`。
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Value> {
        self.buffer.extend_from_slice(bytes);
        let mut deltas = Vec::new();
        while let Some(end) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=end).collect();
            let line = String::from_utf8_lossy(&line);
            let Some(data) = line.trim().strip_prefix("data:").map(str::trim) else { continue };
            if data == "[DONE]" {
                continue;
            }
            if let Ok(event) = serde_json::from_str::<Value>(data) {
                deltas.extend(event_deltas(&event));
            }
        }
        deltas
    }
}

/// 一个流式事件里的文字 / 推理增量。
fn event_deltas(event: &Value) -> Vec<Value> {
    let mut deltas = Vec::new();
    let mut push = |kind: &str, text: Option<&str>| {
        if let Some(text) = text.filter(|text| !text.is_empty()) {
            deltas.push(json!({ "type": kind, "text": text }));
        }
    };
    match event.get("type").and_then(Value::as_str) {
        Some("response.output_text.delta") => push("text", event["delta"].as_str()),
        Some("response.reasoning_summary_text.delta" | "response.reasoning_text.delta") => push("reasoning", event["delta"].as_str()),
        _ => {}
    }
    for choice in event.get("choices").and_then(Value::as_array).into_iter().flatten() {
        push("reasoning", choice.pointer("/delta/reasoning_content").and_then(Value::as_str));
        push("text", choice.pointer("/delta/content").and_then(Value::as_str));
    }
    deltas
}

/// 发一次请求并整理回复。
pub async fn complete(params: &Value) -> anyhow::Result<Value> {
    stream(params, |_| {}).await
}

/// 发一次请求，边收边把文字 / 推理增量交给 `on_delta`，结束后整理成与 [`complete`] 相同的回复。
pub async fn stream(params: &Value, mut on_delta: impl FnMut(Value) + Send) -> anyhow::Result<Value> {
    let body = request_body(params, default_model())?;
    let run = async {
        let mut response = crate::protocol_proxy::open_responses_proxy_request(&body.to_string(), Some("CodexPlusPlus/extension-llm")).await?;
        let status = response.status_code;
        let mut deltas = SseDeltas::default();
        // 代理预读的流开头要先处理，再接着读剩下的。
        let prefix = std::mem::take(&mut response.stream_prefix);
        let mut bytes = Vec::new();
        let mut feed = |chunk: &[u8], bytes: &mut Vec<u8>| {
            bytes.extend_from_slice(chunk);
            if (200..300).contains(&status) {
                for delta in deltas.push(chunk) {
                    on_delta(delta);
                }
            }
        };
        feed(&prefix, &mut bytes);
        if let Some(buffered) = response.buffered_body.take() {
            feed(&buffered, &mut bytes);
        } else if let Some(mut upstream) = response.response.take() {
            while let Some(chunk) = upstream.chunk().await? {
                feed(&chunk, &mut bytes);
            }
        }
        anyhow::Ok((status, bytes))
    };
    let (status, bytes) = crate::extension_gateway::EXTENSIONS_ALREADY_APPLIED
        .scope(true, tokio::time::timeout(LLM_TIMEOUT, run))
        .await
        .context("模型调用超时")??;
    let reply = parse_reply(&bytes);
    if !(200..300).contains(&status) || !reply["error"].is_null() {
        let detail = match &reply["error"] {
            Value::Null => String::from_utf8_lossy(&bytes[..bytes.len().min(400)]).to_string(),
            Value::Object(error) => error.get("message").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| reply["error"].to_string()),
            other => other.to_string(),
        };
        anyhow::bail!("模型调用失败（HTTP {status}）：{detail}");
    }
    let mut reply = reply;
    if reply["model"].is_null() {
        reply["model"] = body["model"].clone();
    }
    reply.as_object_mut().map(|object| object.remove("error"));
    Ok(reply)
}

/// 可用模型：Codex 当前模型目录（config.toml 的 `model_catalog_json`）里的模型，加上默认模型。
pub fn list_models() -> anyhow::Result<Value> {
    let home = crate::codex_home::default_codex_home_dir();
    let text = std::fs::read_to_string(home.join("config.toml")).unwrap_or_default();
    let document = text.parse::<toml_edit::DocumentMut>().unwrap_or_default();
    let mut models = Vec::new();
    if let Some(catalog) = document.get("model_catalog_json").and_then(|item| item.as_str()) {
        let path = std::path::Path::new(catalog);
        let path = if path.is_absolute() { path.to_path_buf() } else { home.join(path) };
        if let Ok(raw) = std::fs::read_to_string(&path) {
            if let Ok(parsed) = serde_json::from_str::<Value>(&raw) {
                for model in parsed["models"].as_array().into_iter().flatten() {
                    let Some(slug) = model.get("slug").or_else(|| model.get("id")).and_then(Value::as_str) else { continue };
                    models.push(json!({
                        "id": slug,
                        "name": model.get("display_name").cloned().unwrap_or(Value::Null),
                        "contextWindow": model.get("context_window").cloned().unwrap_or(Value::Null),
                    }));
                }
            }
        }
    }
    let current = document.get("model").and_then(|item| item.as_str()).map(str::to_string);
    if let Some(current) = &current {
        if !models.iter().any(|model| model["id"] == current.as_str()) {
            models.insert(0, json!({ "id": current, "name": Value::Null, "contextWindow": Value::Null }));
        }
    }
    Ok(json!({ "models": models, "default": current }))
}

/// 桥接路由 `/extensions/llm/complete`。
pub async fn route_complete(payload: &Value) -> anyhow::Result<Value> {
    Ok(json!({ "status": "ok", "result": complete(payload).await? }))
}

/// 桥接路由 `/extensions/llm/models`。
pub fn route_models() -> anyhow::Result<Value> {
    Ok(json!({ "status": "ok", "result": list_models()? }))
}
