//! Google Code Assist（Gemini CLI 与 Antigravity 共用的私有接口）的请求 / 响应转换。
//!
//! ```text
//! Chat Completions 请求 ──► { model, project, request: GenerateContentRequest }
//!                             request = contents / systemInstruction / tools / toolConfig / generationConfig
//! streamGenerateContent?alt=sse 的 { response: { candidates[].content.parts, usageMetadata } }
//!                         ──► Chat SSE（文本 / 推理 / 工具调用 / 结束 / 用量）
//! ```
//!
//! 两家只在细节上不同（[`Flavor`]）：Antigravity 的函数结果放在 `result` 下、工具调用 id 只能用
//! 字母数字和 `_-`、它的 Claude 模型要求工具结果和用户的话在同一个 turn 里，信封里还多几个字段。

use std::collections::{HashMap, HashSet, VecDeque};
use std::pin::Pin;
use std::sync::{Mutex, OnceLock};

use futures_util::StreamExt;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::gateway::subscriptions::chat::{ChatSse, SseReader, Usage, message_text};
use crate::gateway::subscriptions::{ChatStream, fail};

/// 走哪家的 Code Assist。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavor {
    GeminiCli,
    Antigravity,
}

/// 历史里的函数调用没有思维签名时用的占位值：Google 文档里给「不是它签的历史」用的，
/// Gemini 3 看到它就不校验签名。
pub const SKIP_SIGNATURE: &str = "skip_thought_signature_validator";

/// 调用没有对应结果（上一轮被打断）时补的结果。
const INTERRUPTED_RESULT: &str = "[No result: this tool call was interrupted before it returned.]";

/// 历史以模型的 turn 开头时（压缩后常见）补在前面的用户 turn，和 Gemini CLI 的做法一致。
const LEADING_USER_TEXT: &str = "[Continuing from previous AI thoughts...]";

/// 历史以模型的 turn 结尾时补的用户 turn（Google 不接受以模型 turn 结尾的请求）。
const TRAILING_USER_TEXT: &str = "Continue.";

// ---------------------------------------------------------------------------
// 思维签名：Gemini 3 在函数调用上给签名，下一轮要原样带回。Chat 客户端不认识这个字段，
// 只会把调用 id 原样带回来，所以在本进程里按调用 id 记住签名；进程重启后取不到，就用占位值。
// ---------------------------------------------------------------------------

const SIGNATURE_CAPACITY: usize = 4096;

#[derive(Default)]
struct Signatures {
    by_id: HashMap<String, String>,
    order: VecDeque<String>,
}

fn signatures() -> &'static Mutex<Signatures> {
    static CACHE: OnceLock<Mutex<Signatures>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// 记住某个调用 id 的思维签名（最多留 [`SIGNATURE_CAPACITY`] 个，先进先出）。
pub fn remember_signature(call_id: &str, signature: &str) {
    if call_id.is_empty() || signature.is_empty() {
        return;
    }
    let mut cache = signatures()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if cache
        .by_id
        .insert(call_id.to_string(), signature.to_string())
        .is_none()
    {
        cache.order.push_back(call_id.to_string());
    }
    while cache.order.len() > SIGNATURE_CAPACITY {
        if let Some(old) = cache.order.pop_front() {
            cache.by_id.remove(&old);
        }
    }
}

fn signature_of(call_id: &str) -> Option<String> {
    signatures()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .by_id
        .get(call_id)
        .cloned()
}

// ---------------------------------------------------------------------------
// 请求：Chat Completions → GenerateContentRequest
// ---------------------------------------------------------------------------

/// 一个 Gemini turn；`responses` 标记它装的是函数结果（Gemini 要求函数结果和用户的话分开）。
struct Turn {
    role: &'static str,
    parts: Vec<Value>,
    responses: bool,
}

fn is_claude(model: &str) -> bool {
    model.to_ascii_lowercase().contains("claude")
}

/// Antigravity 的调用 id 只能有字母、数字、`_`、`-`。
fn wire_call_id(flavor: Flavor, id: &str) -> String {
    match flavor {
        Flavor::GeminiCli => id.to_string(),
        Flavor::Antigravity => id
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                    ch
                } else {
                    '_'
                }
            })
            .collect(),
    }
}

/// `data:image/png;base64,xxxx` → (mime, data)。
fn parse_data_url(url: &str) -> Option<(String, String)> {
    let rest = url.strip_prefix("data:")?;
    let (meta, data) = rest.split_once(',')?;
    let mime = meta.split(';').next().unwrap_or_default();
    if !meta.contains(";base64") {
        return None;
    }
    Some((
        if mime.is_empty() {
            "application/octet-stream".to_string()
        } else {
            mime.to_string()
        },
        data.to_string(),
    ))
}

/// 按扩展名猜远程图片的类型。
fn guess_image_mime(url: &str) -> &'static str {
    let path = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .to_ascii_lowercase();
    if path.ends_with(".png") {
        "image/png"
    } else if path.ends_with(".gif") {
        "image/gif"
    } else if path.ends_with(".webp") {
        "image/webp"
    } else if path.ends_with(".heic") {
        "image/heic"
    } else {
        "image/jpeg"
    }
}

/// 一张图片（`image_url` 部件）→ `inlineData` / `fileData`。
fn image_part(url: &str) -> Option<Value> {
    if url.is_empty() {
        return None;
    }
    if let Some((mime, data)) = parse_data_url(url) {
        return Some(json!({ "inlineData": { "mimeType": mime, "data": data } }));
    }
    Some(json!({ "fileData": { "mimeType": guess_image_mime(url), "fileUri": url } }))
}

/// 用户 / 工具消息的内容 → (文本部件, 图片部件)。
fn content_parts(content: Option<&Value>) -> (Vec<Value>, Vec<Value>) {
    let mut texts = Vec::new();
    let mut media = Vec::new();
    match content {
        Some(Value::String(text)) if !text.is_empty() => texts.push(json!({ "text": text })),
        Some(Value::Array(parts)) => {
            for part in parts {
                match part.get("type").and_then(Value::as_str).unwrap_or("text") {
                    "text" | "input_text" => {
                        if let Some(text) = part.get("text").and_then(Value::as_str)
                            && !text.is_empty()
                        {
                            texts.push(json!({ "text": text }));
                        }
                    }
                    "image_url" | "input_image" => {
                        let url = part
                            .pointer("/image_url/url")
                            .or_else(|| part.get("image_url"))
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        if let Some(image) = image_part(url) {
                            media.push(image);
                        }
                    }
                    "file" => {
                        let data = part
                            .pointer("/file/file_data")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        if let Some((mime, data)) = parse_data_url(data) {
                            media.push(json!({ "inlineData": { "mimeType": mime, "data": data } }));
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
    (texts, media)
}

/// 工具调用参数（JSON 字符串）→ Gemini 的 `args`（必须是对象）。
fn call_args(arguments: Option<&Value>) -> Value {
    match arguments {
        Some(Value::String(text)) if text.trim().is_empty() => json!({}),
        Some(Value::String(text)) => match serde_json::from_str::<Value>(text) {
            Ok(Value::Object(map)) => Value::Object(map),
            Ok(Value::Null) => json!({}),
            Ok(other) => json!({ "value": other }),
            Err(_) => json!({ "arguments": text }),
        },
        Some(Value::Object(map)) => Value::Object(map.clone()),
        _ => json!({}),
    }
}

/// 加一个 turn：和上一个同角色时合并。用户的话和函数结果分开放（Gemini 不接受混在一起的 turn），
/// Antigravity 的 Claude 例外：它要的是 Anthropic 的 tool_result turn，结果和用户的话在一起。
fn push_turn(
    turns: &mut Vec<Turn>,
    role: &'static str,
    parts: Vec<Value>,
    responses: bool,
    merge_all: bool,
) {
    if parts.is_empty() {
        return;
    }
    if let Some(last) = turns.last_mut()
        && last.role == role
        && (role == "model" || merge_all || last.responses == responses)
    {
        last.parts.extend(parts);
        last.responses |= responses;
        return;
    }
    turns.push(Turn {
        role,
        parts,
        responses,
    });
}

/// 每个模型 turn 里的函数调用都要在紧跟着的用户 turn 里有结果；没有的（被打断的调用）补一个。
fn pair_function_turns(turns: Vec<Turn>, flavor: Flavor, merge_all: bool) -> Vec<Turn> {
    let key = response_key(flavor);
    let mut answered_ids = HashSet::new();
    let mut answered_names = HashSet::new();
    for turn in &turns {
        for part in &turn.parts {
            if let Some(response) = part.get("functionResponse") {
                match response.get("id").and_then(Value::as_str) {
                    Some(id) if !id.is_empty() => {
                        answered_ids.insert(id.to_string());
                    }
                    _ => {
                        answered_names.insert(
                            response
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string(),
                        );
                    }
                }
            }
        }
    }
    let mut out: Vec<Turn> = Vec::with_capacity(turns.len() + 1);
    let mut iter = turns.into_iter().peekable();
    while let Some(turn) = iter.next() {
        let missing: Vec<Value> = if turn.role == "model" {
            turn.parts
                .iter()
                .filter_map(|part| part.get("functionCall"))
                .filter(|call| {
                    let id = call.get("id").and_then(Value::as_str).unwrap_or_default();
                    let name = call.get("name").and_then(Value::as_str).unwrap_or_default();
                    if id.is_empty() {
                        !answered_names.contains(name)
                    } else {
                        !answered_ids.contains(id)
                    }
                })
                .map(|call| {
                    let mut response = json!({
                        "name": call.get("name").cloned().unwrap_or(json!("")),
                        "response": { key: INTERRUPTED_RESULT },
                    });
                    if let Some(id) = call
                        .get("id")
                        .and_then(Value::as_str)
                        .filter(|id| !id.is_empty())
                    {
                        response["id"] = json!(id);
                    }
                    json!({ "functionResponse": response })
                })
                .collect()
        } else {
            Vec::new()
        };
        out.push(turn);
        if missing.is_empty() {
            continue;
        }
        match iter.peek_mut() {
            Some(next) if next.role == "user" && (next.responses || merge_all) => {
                let mut parts = missing;
                parts.append(&mut next.parts);
                next.parts = parts;
                next.responses = true;
            }
            _ => out.push(Turn {
                role: "user",
                parts: missing,
                responses: true,
            }),
        }
    }
    out
}

fn response_key(flavor: Flavor) -> &'static str {
    match flavor {
        Flavor::GeminiCli => "output",
        Flavor::Antigravity => "result",
    }
}

/// Chat 请求的推理强度（`reasoning_effort` 或 `reasoning.effort`）。
fn reasoning_effort(chat: &Value) -> Option<String> {
    chat.get("reasoning_effort")
        .or_else(|| chat.pointer("/reasoning/effort"))
        .and_then(Value::as_str)
        .map(|effort| effort.trim().to_ascii_lowercase())
        .filter(|effort| !effort.is_empty())
}

/// 模型名里的 Gemini 大版本（`gemini-3.1-pro` → 3）；不是 Gemini 时为 0。
fn gemini_generation(model: &str) -> u32 {
    let lower = model.to_ascii_lowercase();
    let Some(rest) = lower
        .rsplit('/')
        .next()
        .and_then(|name| name.strip_prefix("gemini-"))
    else {
        return 0;
    };
    rest.chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .unwrap_or(0)
}

/// Antigravity 的模型 id 自己带着思考档位（`gemini-3.8-flash-low`）。
fn level_in_id(model: &str) -> Option<&'static str> {
    let lower = model.to_ascii_lowercase();
    if lower.ends_with("-extra-low") || lower.ends_with("-minimal") {
        Some(if lower.contains("flash") {
            "minimal"
        } else {
            "low"
        })
    } else if lower.ends_with("-low") {
        Some("low")
    } else if lower.ends_with("-medium") {
        Some("medium")
    } else if lower.ends_with("-high") {
        Some("high")
    } else {
        None
    }
}

/// 思考配置：Gemini 3 起用档位（`thinkingLevel`），2.x 和 Claude 用预算（`thinkingBudget`）。
fn thinking_config(
    model: &str,
    effort: Option<&str>,
    flavor: Flavor,
    max_tokens: Option<u64>,
) -> Option<Value> {
    let lower = model.to_ascii_lowercase();
    if lower.starts_with("gpt-oss") {
        return None;
    }
    if is_claude(model) {
        if !lower.contains("thinking") && effort.is_none() {
            return None;
        }
        let mut budget: u64 = match effort {
            Some("minimal" | "low") => 4096,
            Some("high" | "xhigh") => 32_000,
            Some("none") => return None,
            _ => 16_000,
        };
        if let Some(max) = max_tokens
            && budget >= max
        {
            budget = max.saturating_sub(1);
            if budget < 1024 {
                return None;
            }
        }
        return Some(json!({ "includeThoughts": true, "thinkingBudget": budget }));
    }
    let generation = gemini_generation(model);
    let agent_pro = lower.starts_with("gemini-pro-agent");
    if generation == 0 && !agent_pro {
        return None;
    }
    let mut config = json!({ "includeThoughts": true });
    if generation >= 3 || agent_pro {
        let pinned = if flavor == Flavor::Antigravity {
            level_in_id(model)
        } else {
            None
        };
        let level = pinned.or_else(|| {
            effort.map(|effort| match effort {
                "none" | "minimal" | "low" => "low",
                "medium" if lower.contains("flash") || generation > 3 => "medium",
                _ => "high",
            })
        });
        if let Some(level) = level {
            config["thinkingLevel"] = json!(level);
        }
    } else if let Some(effort) = effort {
        let budget = match effort {
            "none" | "minimal" => 512,
            "low" => 1024,
            "medium" => 8192,
            _ => 24_576,
        };
        config["thinkingBudget"] = json!(budget);
    }
    Some(config)
}

/// 函数工具 → `functionDeclarations`（参数 schema 转成 Gemini 能收的子集）。
fn function_declarations(chat: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    let tools = chat
        .get("tools")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let legacy = chat
        .get("functions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let functions = tools
        .iter()
        .filter(|tool| {
            tool.get("type")
                .and_then(Value::as_str)
                .unwrap_or("function")
                == "function"
        })
        .filter_map(|tool| tool.get("function"))
        .chain(legacy.iter());
    for function in functions {
        let Some(name) = function
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
        else {
            continue;
        };
        let mut declaration = json!({ "name": name });
        if let Some(description) = function
            .get("description")
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
        {
            declaration["description"] = json!(description);
        }
        let parameters = function
            .get("parameters")
            .filter(|value| value.is_object())
            .map(gemini_schema)
            .unwrap_or_else(|| json!({ "type": "object", "properties": {} }));
        declaration["parameters"] = parameters;
        out.push(declaration);
    }
    out
}

/// `tool_choice` → `functionCallingConfig`。
fn function_calling_config(chat: &Value, antigravity_claude: bool) -> Value {
    match chat.get("tool_choice") {
        Some(Value::String(choice)) if choice == "none" => json!({ "mode": "NONE" }),
        Some(Value::String(choice)) if choice == "required" || choice == "any" => {
            json!({ "mode": "ANY" })
        }
        Some(Value::Object(choice)) => {
            match choice
                .get("function")
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str)
            {
                Some(name) => json!({ "mode": "ANY", "allowedFunctionNames": [name] }),
                None => json!({ "mode": "AUTO" }),
            }
        }
        // Antigravity 的 Claude 要求按 schema 校验调用。
        _ if antigravity_claude => json!({ "mode": "VALIDATED" }),
        _ => json!({ "mode": "AUTO" }),
    }
}

/// Chat Completions 请求 → Code Assist 信封里的 `request`（GenerateContentRequest）。
pub fn build_request(chat: &Value, flavor: Flavor, model: &str) -> Value {
    let claude = is_claude(model);
    let antigravity_claude = flavor == Flavor::Antigravity && claude;
    let empty = Vec::new();
    let messages = chat
        .get("messages")
        .and_then(Value::as_array)
        .unwrap_or(&empty);

    let mut system = Vec::new();
    let mut turns: Vec<Turn> = Vec::new();
    // 调用 id → 工具名（tool 消息按 id 找回名字）；已经出现过的调用；已经回答过的调用。
    let mut names: HashMap<String, String> = HashMap::new();
    let mut answered: HashSet<String> = HashSet::new();

    for message in messages {
        let role = message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("user");
        match role {
            "system" | "developer" => {
                let text = message_text(message);
                if !text.trim().is_empty() {
                    system.push(text);
                }
            }
            "assistant" => {
                let mut parts = Vec::new();
                let text = message_text(message);
                if !text.is_empty() {
                    parts.push(json!({ "text": text }));
                }
                let calls = message
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let cached: Vec<Option<String>> = calls
                    .iter()
                    .map(|call| {
                        call.get("id")
                            .and_then(Value::as_str)
                            .and_then(signature_of)
                    })
                    .collect();
                let any_signed = cached.iter().any(Option::is_some);
                for (call, signature) in calls.iter().zip(cached) {
                    let id = call.get("id").and_then(Value::as_str).unwrap_or_default();
                    let name = call
                        .pointer("/function/name")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if name.is_empty() {
                        continue;
                    }
                    if !id.is_empty() {
                        names.insert(id.to_string(), name.to_string());
                        // 同一个 id 再次被调用：之后的结果算新的回答。
                        answered.remove(id);
                    }
                    let mut function_call = json!({
                        "name": name,
                        "args": call_args(call.pointer("/function/arguments")),
                    });
                    if !id.is_empty() {
                        function_call["id"] = json!(wire_call_id(flavor, id));
                    }
                    let mut part = json!({ "functionCall": function_call });
                    // Gemini 3 校验调用上的思维签名：有自己记下的原样带回，没有的用占位值；
                    // 同一步里只有第一个调用带签名，后面的不用再补。
                    let signature = if claude {
                        Some(SKIP_SIGNATURE.to_string())
                    } else if signature.is_some() {
                        signature
                    } else if !any_signed {
                        Some(SKIP_SIGNATURE.to_string())
                    } else {
                        None
                    };
                    if let Some(signature) = signature {
                        part["thoughtSignature"] = json!(signature);
                    }
                    parts.push(part);
                }
                push_turn(&mut turns, "model", parts, false, antigravity_claude);
            }
            "tool" | "function" => {
                let id = message
                    .get("tool_call_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let name = names
                    .get(id)
                    .cloned()
                    .or_else(|| {
                        message
                            .get("name")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    })
                    .unwrap_or_default();
                let (texts, media) = content_parts(message.get("content"));
                let output = texts
                    .iter()
                    .filter_map(|part| part.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("");
                let known = if id.is_empty() {
                    !name.is_empty() && names.values().any(|called| *called == name)
                } else {
                    names.contains_key(id)
                };
                if !known {
                    // 找不到对应调用的结果（历史被裁剪过）：Google 不接受孤立的 functionResponse，
                    // 当成用户的话放进去。
                    let label = if name.is_empty() {
                        "tool".to_string()
                    } else {
                        name.clone()
                    };
                    let mut parts =
                        vec![json!({ "text": format!("[Result of {label}]\n{output}") })];
                    parts.extend(media);
                    push_turn(&mut turns, "user", parts, false, antigravity_claude);
                    continue;
                }
                if !id.is_empty() && !answered.insert(id.to_string()) {
                    // 同一个调用回答了两次：只留第一个，否则 Google 因为结果数和调用数不一致拒绝。
                    continue;
                }
                let mut response = json!({
                    "name": name,
                    "response": { response_key(flavor): output },
                });
                if !id.is_empty() {
                    response["id"] = json!(wire_call_id(flavor, id));
                }
                let mut parts = vec![json!({ "functionResponse": response })];
                parts.extend(media);
                push_turn(&mut turns, "user", parts, true, antigravity_claude);
            }
            _ => {
                let (mut texts, media) = content_parts(message.get("content"));
                texts.extend(media);
                push_turn(&mut turns, "user", texts, false, antigravity_claude);
            }
        }
    }

    let mut turns = pair_function_turns(turns, flavor, antigravity_claude);
    if antigravity_claude {
        // Anthropic 的 tool_result 要排在 turn 最前面：函数结果先，图片和用户的话在后。
        for turn in turns.iter_mut().filter(|turn| turn.responses) {
            let (mut lead, rest): (Vec<Value>, Vec<Value>) = std::mem::take(&mut turn.parts)
                .into_iter()
                .partition(|part| part.get("functionResponse").is_some());
            lead.extend(rest);
            turn.parts = lead;
        }
    }
    if turns.first().is_some_and(|turn| turn.role == "model") {
        turns.insert(
            0,
            Turn {
                role: "user",
                parts: vec![json!({ "text": LEADING_USER_TEXT })],
                responses: false,
            },
        );
    }
    if turns.last().is_none_or(|turn| turn.role == "model") {
        turns.push(Turn {
            role: "user",
            parts: vec![json!({ "text": TRAILING_USER_TEXT })],
            responses: false,
        });
    }
    let contents: Vec<Value> = turns
        .into_iter()
        .map(|turn| json!({ "role": turn.role, "parts": turn.parts }))
        .collect();

    let mut request = json!({ "contents": contents });
    if !system.is_empty() {
        request["systemInstruction"] = json!({
            "role": "user",
            "parts": [{ "text": system.join("\n\n") }],
        });
    }
    let declarations = function_declarations(chat);
    if !declarations.is_empty() {
        request["tools"] = json!([{ "functionDeclarations": declarations }]);
        request["toolConfig"] = json!({
            "functionCallingConfig": function_calling_config(chat, antigravity_claude),
        });
    }

    let mut generation = Map::new();
    let max_tokens = chat
        .get("max_completion_tokens")
        .or_else(|| chat.get("max_tokens"))
        .and_then(Value::as_u64)
        .filter(|max| *max > 0);
    // Antigravity 的 Gemini 自己定输出上限，传了反而会被截短；Claude 要传。
    let send_max = flavor == Flavor::GeminiCli || claude;
    if let Some(max) = max_tokens.filter(|_| send_max) {
        generation.insert("maxOutputTokens".into(), json!(max));
    }
    if let Some(temperature) = chat.get("temperature").filter(|value| value.is_number()) {
        generation.insert("temperature".into(), temperature.clone());
    }
    if let Some(top_p) = chat.get("top_p").filter(|value| value.is_number()) {
        generation.insert("topP".into(), top_p.clone());
    }
    match chat.get("stop") {
        Some(Value::String(stop)) if !stop.is_empty() => {
            generation.insert("stopSequences".into(), json!([stop]));
        }
        Some(Value::Array(stops)) if !stops.is_empty() => {
            generation.insert("stopSequences".into(), Value::Array(stops.clone()));
        }
        _ => {}
    }
    match chat
        .pointer("/response_format/type")
        .and_then(Value::as_str)
    {
        Some("json_object") => {
            generation.insert("responseMimeType".into(), json!("application/json"));
        }
        Some("json_schema") => {
            generation.insert("responseMimeType".into(), json!("application/json"));
            if let Some(schema) = chat.pointer("/response_format/json_schema/schema") {
                generation.insert("responseSchema".into(), gemini_schema(schema));
            }
        }
        _ => {}
    }
    let effort = reasoning_effort(chat);
    if let Some(config) = thinking_config(model, effort.as_deref(), flavor, max_tokens) {
        // Claude 的答复要在思考预算之外还有空间。
        if claude
            && !generation.contains_key("maxOutputTokens")
            && let Some(budget) = config.get("thinkingBudget").and_then(Value::as_u64)
        {
            generation.insert("maxOutputTokens".into(), json!(budget + 32_000));
        }
        generation.insert("thinkingConfig".into(), config);
    }
    if !generation.is_empty() {
        request["generationConfig"] = Value::Object(generation);
    }
    request
}

/// 第一条用户消息的文字（会话 id 用它算，同一个对话每一轮都一样）。
fn first_user_text(request: &Value) -> String {
    request
        .get("contents")
        .and_then(Value::as_array)
        .and_then(|contents| {
            contents
                .iter()
                .find(|content| content.get("role").and_then(Value::as_str) == Some("user"))
        })
        .and_then(|content| content.get("parts").and_then(Value::as_array))
        .and_then(|parts| {
            parts.iter().find_map(|part| {
                part.get("text")
                    .and_then(Value::as_str)
                    .filter(|text| !text.is_empty())
            })
        })
        .unwrap_or_default()
        .to_string()
}

/// 由第一条用户消息算出的会话 id（`-` 加 63 位正整数，和 Antigravity 自己的形式一致）。
pub fn session_id(request: &Value) -> String {
    let mut first = first_user_text(request);
    if first.is_empty() {
        first = uuid::Uuid::new_v4().to_string();
    }
    let digest = Sha256::digest(first.as_bytes());
    let mut head = [0_u8; 8];
    head.copy_from_slice(&digest[..8]);
    let number = u64::from_be_bytes(head) & 0x7fff_ffff_ffff_ffff;
    format!("-{number}")
}

/// 包成 Code Assist 的信封：`{model, project, request, ...}`。
pub fn envelope(flavor: Flavor, model: &str, project: &str, mut request: Value) -> Value {
    let id = uuid::Uuid::new_v4().simple().to_string();
    match flavor {
        Flavor::GeminiCli => json!({
            "model": model,
            "project": project,
            "user_prompt_id": id,
            "request": request,
        }),
        Flavor::Antigravity => {
            request["sessionId"] = json!(session_id(&request));
            json!({
                "model": model,
                "project": project,
                "request": request,
                "userAgent": "antigravity",
                "requestType": "agent",
                "requestId": format!("agent-{id}"),
            })
        }
    }
}

// ---------------------------------------------------------------------------
// JSON Schema → Gemini Schema（OpenAPI 子集）
// ---------------------------------------------------------------------------

/// Gemini Schema 认识的字段（其余的如 `$schema`、`additionalProperties`、`default`、`format`、
/// `title`、`const`、`examples` 都去掉，Google 遇到不认识的字段会拒绝整个请求）。
const KEPT_KEYWORDS: &[&str] = &[
    "description",
    "nullable",
    "pattern",
    "minimum",
    "maximum",
    "minLength",
    "maxLength",
    "minItems",
    "maxItems",
    "minProperties",
    "maxProperties",
];

const MAX_SCHEMA_DEPTH: usize = 32;

/// 把 JSON Schema 转成 Gemini 的 Schema：展开本地 `$ref`、合并 `allOf`、`anyOf`/`oneOf` 取第一个
/// 非 null 分支（带 null 时标 `nullable`）、`type` 数组取一个、`const` 转 `enum`、非字符串的 `enum`
/// 去掉、数组补 `items`、`required` 只留存在的属性，不认识的字段去掉。
pub fn gemini_schema(schema: &Value) -> Value {
    let mut definitions = Map::new();
    for key in ["$defs", "definitions"] {
        if let Some(Value::Object(map)) = schema.get(key) {
            for (name, value) in map {
                definitions.insert(name.clone(), value.clone());
            }
        }
    }
    convert_schema(schema, &definitions, 0)
        .unwrap_or_else(|| json!({ "type": "object", "properties": {} }))
}

fn resolve_ref<'a>(reference: &str, definitions: &'a Map<String, Value>) -> Option<&'a Value> {
    let name = reference.rsplit('/').next().unwrap_or(reference);
    definitions.get(name)
}

fn required_names(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|names| {
            names
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// 返回 None 表示这个 schema 是 `false`（不允许任何值），持有它的地方应该去掉它。
fn convert_schema(schema: &Value, definitions: &Map<String, Value>, depth: usize) -> Option<Value> {
    let object = match schema {
        Value::Bool(true) => return Some(json!({})),
        Value::Bool(false) => return None,
        Value::Object(object) => object,
        _ => return Some(json!({})),
    };
    if depth > MAX_SCHEMA_DEPTH {
        return Some(json!({ "type": "object" }));
    }
    if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
        return match resolve_ref(reference, definitions) {
            Some(target) => convert_schema(&target.clone(), definitions, depth + 1),
            None => Some(json!({ "type": "object" })),
        };
    }
    if let Some(Value::Array(members)) = object.get("allOf") {
        let mut merged = object.clone();
        merged.remove("allOf");
        let mut required = required_names(object.get("required"));
        let mut properties = merged
            .get("properties")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for member in members {
            let member = match member.get("$ref").and_then(Value::as_str) {
                Some(reference) => resolve_ref(reference, definitions)
                    .cloned()
                    .unwrap_or(Value::Null),
                None => member.clone(),
            };
            let Value::Object(member) = member else {
                continue;
            };
            required.extend(required_names(member.get("required")));
            for (key, value) in member {
                match key.as_str() {
                    "required" => {}
                    "properties" => {
                        if let Value::Object(more) = value {
                            properties.extend(more);
                        }
                    }
                    _ => {
                        merged.entry(key).or_insert(value);
                    }
                }
            }
        }
        if !properties.is_empty() {
            merged.insert("properties".into(), Value::Object(properties));
        }
        if !required.is_empty() {
            let mut seen = HashSet::new();
            required.retain(|name| seen.insert(name.clone()));
            merged.insert("required".into(), json!(required));
        }
        return convert_schema(&Value::Object(merged), definitions, depth + 1);
    }
    for key in ["anyOf", "oneOf"] {
        let Some(Value::Array(branches)) = object.get(key) else {
            continue;
        };
        let nullable = branches
            .iter()
            .any(|branch| branch.get("type").and_then(Value::as_str) == Some("null"));
        let own_shape = object.contains_key("type") || object.contains_key("properties");
        let mut base = if own_shape {
            // 自己有类型 / 属性的 schema，union 只是在说哪些字段一起出现：保留自身，
            // 把各分支的属性并进来（都不算必填）。
            let mut own = object.clone();
            own.remove("anyOf");
            own.remove("oneOf");
            let mut properties = own
                .get("properties")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            for branch in branches {
                let branch = match branch.get("$ref").and_then(Value::as_str) {
                    Some(reference) => resolve_ref(reference, definitions)
                        .cloned()
                        .unwrap_or(Value::Null),
                    None => branch.clone(),
                };
                if let Some(Value::Object(more)) = branch.get("properties") {
                    for (name, value) in more {
                        properties.entry(name.clone()).or_insert(value.clone());
                    }
                }
            }
            if !properties.is_empty() {
                own.insert("properties".into(), Value::Object(properties));
            }
            convert_schema(&Value::Object(own), definitions, depth + 1).unwrap_or_else(|| json!({}))
        } else {
            branches
                .iter()
                .find(|branch| branch.get("type").and_then(Value::as_str) != Some("null"))
                .and_then(|branch| convert_schema(branch, definitions, depth + 1))
                .unwrap_or_else(|| json!({}))
        };
        if let Some(description) = object.get("description").and_then(Value::as_str)
            && base.get("description").is_none()
        {
            base["description"] = json!(description);
        }
        if nullable {
            base["nullable"] = json!(true);
        }
        if base.get("type").is_none() {
            base["type"] = json!("string");
        }
        return Some(base);
    }

    let mut out = Map::new();
    for (key, value) in object {
        match key.as_str() {
            keyword if KEPT_KEYWORDS.contains(&keyword) => {
                out.insert(key.clone(), value.clone());
            }
            "exclusiveMinimum" | "exclusiveMaximum" => {
                // Gemini 只认闭区间：数字形式的开区间退成闭区间（已有闭区间时不覆盖）。
                let inclusive = if key == "exclusiveMinimum" {
                    "minimum"
                } else {
                    "maximum"
                };
                if value.is_number() && !object.contains_key(inclusive) {
                    out.insert(inclusive.into(), value.clone());
                }
            }
            "type" => match value {
                Value::Array(types) => {
                    let names: Vec<&str> = types.iter().filter_map(Value::as_str).collect();
                    if names.contains(&"null") {
                        out.insert("nullable".into(), json!(true));
                    }
                    // Gemini 每个节点只能有一个类型：优先取和 items / properties 对得上的。
                    let pick = names
                        .iter()
                        .find(|name| {
                            (**name == "array" && object.contains_key("items"))
                                || (**name == "object" && object.contains_key("properties"))
                        })
                        .or_else(|| names.iter().find(|name| **name != "null"))
                        .copied();
                    if let Some(pick) = pick {
                        out.insert("type".into(), json!(pick));
                    }
                }
                Value::String(name) if name != "null" => {
                    out.insert("type".into(), json!(name));
                }
                Value::String(_) => {
                    out.insert("nullable".into(), json!(true));
                }
                _ => {}
            },
            "properties" => {
                let mut properties = Map::new();
                if let Value::Object(map) = value {
                    for (name, property) in map {
                        if let Some(converted) = convert_schema(property, definitions, depth + 1) {
                            properties.insert(name.clone(), converted);
                        }
                    }
                }
                out.insert("properties".into(), Value::Object(properties));
            }
            "items" => match value {
                Value::Array(tuple) => {
                    // 元组形式：取第一个。
                    let first = tuple
                        .first()
                        .and_then(|item| convert_schema(item, definitions, depth + 1))
                        .unwrap_or_else(|| json!({}));
                    out.insert("items".into(), first);
                }
                other => match convert_schema(other, definitions, depth + 1) {
                    Some(items) => {
                        out.insert("items".into(), items);
                    }
                    None => {
                        out.insert("items".into(), json!({}));
                        out.insert("maxItems".into(), json!(0));
                    }
                },
            },
            "enum" => {
                if let Value::Array(values) = value {
                    let strings: Vec<Value> = values
                        .iter()
                        .filter(|value| value.is_string())
                        .cloned()
                        .collect();
                    if !strings.is_empty() && strings.len() == values.len() {
                        out.insert("enum".into(), Value::Array(strings));
                    }
                }
            }
            "required" => {
                out.insert("required".into(), value.clone());
            }
            _ => {}
        }
    }
    if let Some(Value::String(constant)) = object.get("const") {
        out.insert("enum".into(), json!([constant]));
        out.entry("type").or_insert(json!("string"));
    }
    if out.contains_key("enum") {
        // Gemini 的 enum 只能用在字符串上。
        out.insert("type".into(), json!("string"));
    }
    if !out.contains_key("type") {
        if out.contains_key("properties") {
            out.insert("type".into(), json!("object"));
        } else if out.contains_key("items") {
            out.insert("type".into(), json!("array"));
        }
    }
    let kind = out.get("type").and_then(Value::as_str).map(str::to_string);
    if kind.as_deref() == Some("array") && !out.contains_key("items") {
        out.insert("items".into(), json!({}));
    }
    if let Some(kind) = kind.as_deref() {
        if kind != "array" {
            for key in ["items", "minItems", "maxItems"] {
                out.remove(key);
            }
        }
        if kind != "object" {
            for key in ["properties", "required", "minProperties", "maxProperties"] {
                out.remove(key);
            }
        }
    }
    if let Some(required) = out.get("required").cloned() {
        let properties = out.get("properties").and_then(Value::as_object);
        let keep: Vec<Value> = required
            .as_array()
            .into_iter()
            .flatten()
            .filter(|name| {
                name.as_str().is_some_and(|name| {
                    properties.is_some_and(|properties| properties.contains_key(name))
                })
            })
            .cloned()
            .collect();
        if keep.is_empty() {
            out.remove("required");
        } else {
            out.insert("required".into(), Value::Array(keep));
        }
    }
    Some(Value::Object(out))
}

// ---------------------------------------------------------------------------
// 响应：Code Assist SSE → Chat SSE
// ---------------------------------------------------------------------------

/// Gemini 的 `finishReason` → Chat 的 `finish_reason`；None 表示按有没有工具调用定（stop / tool_calls）。
fn finish_reason(reason: &str) -> Option<&'static str> {
    match reason {
        "MAX_TOKENS" => Some("length"),
        "SAFETY" | "RECITATION" | "BLOCKLIST" | "PROHIBITED_CONTENT" | "SPII" | "IMAGE_SAFETY"
        | "LANGUAGE" => Some("content_filter"),
        _ => None,
    }
}

/// Gemini 偶尔给工具名带上自己的命名空间（`default_api:shell`）。
fn tool_name(name: &str) -> &str {
    for prefix in ["default_api:", "default_api."] {
        if let Some(rest) = name.strip_prefix(prefix)
            && !rest.is_empty()
        {
            return rest;
        }
    }
    name
}

/// 读 Code Assist 流的状态机：每个 SSE 事件的 data 喂进 [`Decoder::feed`]，流结束调 [`Decoder::end`]。
pub struct Decoder {
    sse: ChatSse,
    response_id: String,
    calls: usize,
    finish: Option<Option<&'static str>>,
    usage: Option<Usage>,
    emitted: bool,
    /// 还没输出任何内容时上游就报错：交给调用方按状态码失败（不发 200 再报错）。
    early_error: Option<(u16, String)>,
    failed: bool,
}

impl Decoder {
    pub fn new(model: &str) -> Self {
        Self {
            sse: ChatSse::new(model),
            response_id: String::new(),
            calls: 0,
            finish: None,
            usage: None,
            emitted: false,
            early_error: None,
            failed: false,
        }
    }

    /// 给没有 id 的函数调用生成一个稳定的 id：同一个答复里按顺序、名字和参数算出来。
    fn call_id(&mut self, name: &str, args: &str) -> String {
        self.calls += 1;
        let digest = Sha256::digest(
            format!(
                "{}\u{0}{}\u{0}{name}\u{0}{args}",
                self.response_id, self.calls
            )
            .as_bytes(),
        );
        let hex: String = digest[..12]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        format!("call_{hex}")
    }

    pub fn is_finished(&self) -> bool {
        self.sse.is_finished() || self.failed
    }

    /// 还没输出就失败时的 (状态码, 信息)。
    pub fn take_early_error(&mut self) -> Option<(u16, String)> {
        self.early_error.take()
    }

    fn out(&mut self, bytes: Vec<u8>) -> Vec<u8> {
        if !bytes.is_empty() {
            self.emitted = true;
        }
        bytes
    }

    fn error(&mut self, status: u16, message: String) -> Vec<u8> {
        if self.emitted {
            let bytes = self.sse.error(&message);
            self.out(bytes)
        } else {
            self.early_error = Some((status, message));
            self.failed = true;
            Vec::new()
        }
    }

    /// 一个 SSE 事件的 data。
    pub fn feed(&mut self, data: &str) -> Vec<u8> {
        if self.is_finished() {
            return Vec::new();
        }
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            return Vec::new();
        }
        let value: Value = match serde_json::from_str(data) {
            Ok(value) => value,
            Err(_) => return Vec::new(),
        };
        // 流里也可能是数组（非 alt=sse 的形式）。
        if let Value::Array(items) = &value {
            let mut out = Vec::new();
            for item in items {
                out.extend(self.feed(&item.to_string()));
            }
            return out;
        }
        if let Some(error) = value.get("error") {
            let status = error
                .get("code")
                .and_then(Value::as_u64)
                .filter(|code| (400..600).contains(code))
                .unwrap_or(502) as u16;
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("Code Assist 返回错误")
                .to_string();
            return self.error(status, message);
        }
        let chunk = value.get("response").unwrap_or(&value);
        if self.response_id.is_empty()
            && let Some(id) = chunk.get("responseId").and_then(Value::as_str)
        {
            self.response_id = id.to_string();
        }
        if let Some(metadata) = chunk.get("usageMetadata") {
            let number = |key: &str| metadata.get(key).and_then(Value::as_u64).unwrap_or(0);
            let thoughts = number("thoughtsTokenCount");
            self.usage = Some(Usage {
                prompt_tokens: number("promptTokenCount") + number("toolUsePromptTokenCount"),
                completion_tokens: number("candidatesTokenCount") + thoughts,
                cached_tokens: number("cachedContentTokenCount"),
                reasoning_tokens: thoughts,
            });
        }
        let mut out = Vec::new();
        if let Some(reason) = chunk
            .pointer("/promptFeedback/blockReason")
            .and_then(Value::as_str)
            .filter(|reason| !reason.is_empty() && *reason != "BLOCK_REASON_UNSPECIFIED")
        {
            out.extend(self.sse.text(&format!("[Google 拒绝了这个请求：{reason}]")));
            self.finish = Some(Some("content_filter"));
        }
        let candidates = chunk
            .get("candidates")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if let Some(candidate) = candidates.first() {
            let parts = candidate
                .pointer("/content/parts")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for part in &parts {
                if let Some(call) = part.get("functionCall") {
                    let name =
                        tool_name(call.get("name").and_then(Value::as_str).unwrap_or_default())
                            .to_string();
                    let args = match call.get("args") {
                        Some(Value::Null) | None => "{}".to_string(),
                        Some(args) => args.to_string(),
                    };
                    let id = match call
                        .get("id")
                        .and_then(Value::as_str)
                        .filter(|id| !id.is_empty())
                    {
                        Some(id) => id.to_string(),
                        None => self.call_id(&name, &args),
                    };
                    if let Some(signature) = part.get("thoughtSignature").and_then(Value::as_str) {
                        remember_signature(&id, signature);
                    }
                    out.extend(self.sse.tool_call(&id, &name, &args));
                    continue;
                }
                let text = part.get("text").and_then(Value::as_str).unwrap_or_default();
                if part
                    .get("thought")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    out.extend(self.sse.reasoning(text));
                } else {
                    out.extend(self.sse.text(text));
                }
            }
            if let Some(reason) = candidate
                .get("finishReason")
                .and_then(Value::as_str)
                .filter(|reason| !reason.is_empty())
            {
                self.finish = Some(finish_reason(reason));
            }
        }
        self.out(out)
    }

    /// 流结束：发结束块（带用量）和 `[DONE]`。
    pub fn end(&mut self) -> Vec<u8> {
        if self.is_finished() {
            return Vec::new();
        }
        let reason = self.finish.flatten();
        let usage = self.usage;
        let bytes = self.sse.finish(reason, usage);
        self.out(bytes)
    }
}

type ByteStream =
    Pin<Box<dyn futures_util::Stream<Item = Result<Vec<u8>, reqwest::Error>> + Send + 'static>>;

/// 把 Code Assist 的 SSE 答复转成 Chat SSE 流。还没输出内容就报错时，流的第一项是带状态码的错误。
pub fn chat_stream(response: reqwest::Response, model: &str) -> ChatStream {
    let body: ByteStream = Box::pin(
        response
            .bytes_stream()
            .map(|chunk| chunk.map(|bytes| bytes.to_vec())),
    );
    let state = (body, SseReader::default(), Decoder::new(model), false);
    Box::pin(futures_util::stream::unfold(
        state,
        |(mut body, mut reader, mut decoder, done)| async move {
            if done {
                return None;
            }
            loop {
                match body.next().await {
                    Some(Ok(bytes)) => {
                        let mut out = Vec::new();
                        for (_, data) in reader.push(&bytes) {
                            out.extend(decoder.feed(&data));
                        }
                        if let Some((status, message)) = decoder.take_early_error() {
                            return Some((
                                Err(fail(status, message)),
                                (body, reader, decoder, true),
                            ));
                        }
                        let finished = decoder.is_finished();
                        if finished || !out.is_empty() {
                            return Some((Ok(out), (body, reader, decoder, finished)));
                        }
                    }
                    Some(Err(error)) => {
                        let error = anyhow::Error::from(error).context("读取 Code Assist 答复失败");
                        return Some((Err(error), (body, reader, decoder, true)));
                    }
                    None => {
                        let mut out = Vec::new();
                        if let Some((_, data)) = reader.finish() {
                            out.extend(decoder.feed(&data));
                        }
                        if let Some((status, message)) = decoder.take_early_error() {
                            return Some((
                                Err(fail(status, message)),
                                (body, reader, decoder, true),
                            ));
                        }
                        out.extend(decoder.end());
                        return Some((Ok(out), (body, reader, decoder, true)));
                    }
                }
            }
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::subscriptions::chat::aggregate;

    fn tool(name: &str, parameters: Value) -> Value {
        json!({ "type": "function", "function": { "name": name, "description": "d", "parameters": parameters } })
    }

    #[test]
    fn builds_system_user_and_image_parts() {
        let chat = json!({
            "model": "gemini-2.5-pro",
            "messages": [
                { "role": "system", "content": "你是助手" },
                { "role": "developer", "content": [{ "type": "text", "text": "用中文" }] },
                { "role": "user", "content": [
                    { "type": "text", "text": "看图" },
                    { "type": "image_url", "image_url": { "url": "data:image/png;base64,QUJD" } },
                    { "type": "image_url", "image_url": { "url": "https://x.test/a.webp?s=1" } },
                ] },
            ],
            "max_tokens": 100,
            "temperature": 0.2,
            "stop": "END",
        });
        let request = build_request(&chat, Flavor::GeminiCli, "gemini-2.5-pro");
        assert_eq!(
            request["systemInstruction"]["parts"][0]["text"],
            "你是助手\n\n用中文"
        );
        let parts = &request["contents"][0]["parts"];
        assert_eq!(request["contents"][0]["role"], "user");
        assert_eq!(parts[0]["text"], "看图");
        assert_eq!(parts[1]["inlineData"]["mimeType"], "image/png");
        assert_eq!(parts[1]["inlineData"]["data"], "QUJD");
        assert_eq!(parts[2]["fileData"]["mimeType"], "image/webp");
        let generation = &request["generationConfig"];
        assert_eq!(generation["maxOutputTokens"], 100);
        assert_eq!(generation["temperature"], 0.2);
        assert_eq!(generation["stopSequences"], json!(["END"]));
        assert_eq!(generation["thinkingConfig"]["includeThoughts"], true);
        assert!(request.get("tools").is_none());
    }

    #[test]
    fn multi_turn_tool_calls_pair_with_results_by_id() {
        let chat = json!({
            "messages": [
                { "role": "user", "content": "列目录再读文件" },
                { "role": "assistant", "content": "好的", "tool_calls": [
                    { "id": "call_a", "type": "function", "function": { "name": "shell", "arguments": "{\"cmd\":\"ls\"}" } },
                    { "id": "call_b", "type": "function", "function": { "name": "read", "arguments": "" } },
                ] },
                { "role": "tool", "tool_call_id": "call_b", "content": "文件内容" },
                { "role": "tool", "tool_call_id": "call_a", "content": [{ "type": "text", "text": "a.txt" }] },
                { "role": "tool", "tool_call_id": "call_a", "content": "重复的结果" },
                { "role": "user", "content": "继续" },
            ],
            "tools": [tool("shell", json!({ "type": "object", "properties": { "cmd": { "type": "string" } } })), tool("read", json!({}))],
        });
        let request = build_request(&chat, Flavor::GeminiCli, "gemini-3-pro-preview");
        let contents = request["contents"].as_array().unwrap();
        assert_eq!(contents.len(), 4, "{contents:#?}");
        assert_eq!(contents[1]["role"], "model");
        let calls = contents[1]["parts"].as_array().unwrap();
        assert_eq!(calls[0]["text"], "好的");
        assert_eq!(calls[1]["functionCall"]["name"], "shell");
        assert_eq!(calls[1]["functionCall"]["args"]["cmd"], "ls");
        assert_eq!(calls[1]["functionCall"]["id"], "call_a");
        assert_eq!(calls[1]["thoughtSignature"], SKIP_SIGNATURE);
        assert_eq!(calls[2]["functionCall"]["args"], json!({}));
        // 结果按 id 配回名字；重复的结果只留第一个；用户的话单独一个 turn。
        let responses = contents[2]["parts"].as_array().unwrap();
        assert_eq!(contents[2]["role"], "user");
        assert_eq!(responses.len(), 2);
        assert_eq!(responses[0]["functionResponse"]["name"], "read");
        assert_eq!(responses[0]["functionResponse"]["id"], "call_b");
        assert_eq!(
            responses[0]["functionResponse"]["response"]["output"],
            "文件内容"
        );
        assert_eq!(responses[1]["functionResponse"]["name"], "shell");
        assert_eq!(
            responses[1]["functionResponse"]["response"]["output"],
            "a.txt"
        );
        assert_eq!(contents[3]["parts"][0]["text"], "继续");
        assert_eq!(
            request["tools"][0]["functionDeclarations"][0]["parameters"]["properties"]["cmd"]["type"],
            "string"
        );
        assert_eq!(
            request["toolConfig"]["functionCallingConfig"]["mode"],
            "AUTO"
        );
        assert_eq!(
            request["generationConfig"]["thinkingConfig"]["includeThoughts"],
            true
        );
    }

    #[test]
    fn interrupted_calls_get_synthetic_results_and_trailing_model_turn_is_closed() {
        let chat = json!({
            "messages": [
                { "role": "assistant", "content": "", "tool_calls": [
                    { "id": "c1", "type": "function", "function": { "name": "shell", "arguments": "{}" } },
                ] },
                { "role": "user", "content": "算了" },
                { "role": "assistant", "content": "好" },
            ],
        });
        let request = build_request(&chat, Flavor::Antigravity, "gemini-3-flash");
        let contents = request["contents"].as_array().unwrap();
        assert_eq!(contents[0]["parts"][0]["text"], LEADING_USER_TEXT);
        assert_eq!(contents[1]["role"], "model");
        assert_eq!(contents[2]["parts"][0]["functionResponse"]["id"], "c1");
        assert_eq!(
            contents[2]["parts"][0]["functionResponse"]["response"]["result"],
            INTERRUPTED_RESULT
        );
        assert_eq!(contents[3]["parts"][0]["text"], "算了");
        assert_eq!(contents[4]["role"], "model");
        assert_eq!(contents[5]["parts"][0]["text"], TRAILING_USER_TEXT);
        // Antigravity 的 Gemini 不传输出上限。
        assert!(request["generationConfig"].get("maxOutputTokens").is_none());
    }

    #[test]
    fn orphan_tool_results_become_text() {
        let chat = json!({
            "messages": [
                { "role": "user", "content": "hi" },
                { "role": "tool", "tool_call_id": "gone", "name": "shell", "content": "out" },
            ],
        });
        let request = build_request(&chat, Flavor::GeminiCli, "gemini-2.5-flash");
        let contents = request["contents"].as_array().unwrap();
        assert_eq!(contents.len(), 1);
        assert_eq!(contents[0]["parts"][1]["text"], "[Result of shell]\nout");
    }

    #[test]
    fn antigravity_claude_keeps_results_and_text_together_and_sanitizes_ids() {
        let chat = json!({
            "messages": [
                { "role": "user", "content": "go" },
                { "role": "assistant", "tool_calls": [
                    { "id": "toolu.01:x", "type": "function", "function": { "name": "shell", "arguments": "{}" } },
                ] },
                { "role": "tool", "tool_call_id": "toolu.01:x", "content": [
                    { "type": "text", "text": "截图如下" },
                    { "type": "image_url", "image_url": { "url": "data:image/jpeg;base64,AAA" } },
                ] },
                { "role": "user", "content": "看看" },
            ],
            "tools": [tool("shell", json!({ "type": "object" }))],
            "reasoning_effort": "low",
        });
        let request = build_request(&chat, Flavor::Antigravity, "claude-opus-4-6-thinking");
        let contents = request["contents"].as_array().unwrap();
        assert_eq!(contents.len(), 3);
        assert_eq!(contents[1]["parts"][0]["functionCall"]["id"], "toolu_01_x");
        assert_eq!(contents[1]["parts"][0]["thoughtSignature"], SKIP_SIGNATURE);
        let parts = contents[2]["parts"].as_array().unwrap();
        assert_eq!(parts[0]["functionResponse"]["id"], "toolu_01_x");
        assert_eq!(
            parts[0]["functionResponse"]["response"]["result"],
            "截图如下"
        );
        assert_eq!(parts[1]["inlineData"]["mimeType"], "image/jpeg");
        assert_eq!(parts[2]["text"], "看看");
        assert_eq!(
            request["toolConfig"]["functionCallingConfig"]["mode"],
            "VALIDATED"
        );
        let generation = &request["generationConfig"];
        assert_eq!(generation["thinkingConfig"]["thinkingBudget"], 4096);
        assert_eq!(generation["maxOutputTokens"], 4096 + 32_000);
    }

    #[test]
    fn cached_signature_is_sent_back_on_first_call_only() {
        remember_signature("sig_call_1", "SIG");
        let chat = json!({
            "messages": [
                { "role": "user", "content": "x" },
                { "role": "assistant", "tool_calls": [
                    { "id": "sig_call_1", "type": "function", "function": { "name": "a", "arguments": "{}" } },
                    { "id": "sig_call_2", "type": "function", "function": { "name": "b", "arguments": "{}" } },
                ] },
                { "role": "tool", "tool_call_id": "sig_call_1", "content": "1" },
                { "role": "tool", "tool_call_id": "sig_call_2", "content": "2" },
            ],
        });
        let request = build_request(&chat, Flavor::GeminiCli, "gemini-3-pro-preview");
        let parts = &request["contents"][1]["parts"];
        assert_eq!(parts[0]["thoughtSignature"], "SIG");
        assert!(parts[1].get("thoughtSignature").is_none());
    }

    #[test]
    fn tool_choice_and_effort_map() {
        let mut chat = json!({
            "messages": [{ "role": "user", "content": "x" }],
            "tools": [tool("a", json!({ "type": "object" }))],
            "tool_choice": { "type": "function", "function": { "name": "a" } },
            "reasoning_effort": "medium",
        });
        let request = build_request(&chat, Flavor::GeminiCli, "gemini-3-flash-preview");
        assert_eq!(
            request["toolConfig"]["functionCallingConfig"]["mode"],
            "ANY"
        );
        assert_eq!(
            request["toolConfig"]["functionCallingConfig"]["allowedFunctionNames"],
            json!(["a"])
        );
        assert_eq!(
            request["generationConfig"]["thinkingConfig"]["thinkingLevel"],
            "medium"
        );
        chat["tool_choice"] = json!("none");
        let request = build_request(&chat, Flavor::GeminiCli, "gemini-3-pro-preview");
        assert_eq!(
            request["toolConfig"]["functionCallingConfig"]["mode"],
            "NONE"
        );
        assert_eq!(
            request["generationConfig"]["thinkingConfig"]["thinkingLevel"],
            "high"
        );
        let request = build_request(&chat, Flavor::GeminiCli, "gemini-2.5-pro");
        assert_eq!(
            request["generationConfig"]["thinkingConfig"]["thinkingBudget"],
            8192
        );
        // Antigravity 的档位写在 id 里。
        let request = build_request(&chat, Flavor::Antigravity, "gemini-3.8-flash-low");
        assert_eq!(
            request["generationConfig"]["thinkingConfig"]["thinkingLevel"],
            "low"
        );
        // gpt-oss 不发思考配置。
        let request = build_request(&chat, Flavor::Antigravity, "gpt-oss-120b-medium");
        assert!(request.get("generationConfig").is_none());
    }

    #[test]
    fn schema_is_cut_down_to_gemini_subset() {
        let schema = json!({
            "$schema": "http://json-schema.org/draft-07/schema#",
            "type": "object",
            "additionalProperties": false,
            "title": "Args",
            "$defs": { "Mode": { "type": "string", "enum": ["a", "b"], "default": "a" } },
            "properties": {
                "path": { "type": "string", "format": "uri", "description": "路径" },
                "mode": { "$ref": "#/$defs/Mode" },
                "count": { "type": ["integer", "null"], "exclusiveMinimum": 0 },
                "tags": { "type": "array" },
                "pair": { "type": "array", "items": [{ "type": "number" }] },
                "choice": { "anyOf": [{ "type": "null" }, { "type": "object", "properties": { "x": { "type": "boolean" } } }] },
                "never": false,
                "fixed": { "const": "v" },
                "level": { "enum": [1, 2] },
                "both": { "allOf": [
                    { "type": "object", "properties": { "a": { "type": "string" } }, "required": ["a"] },
                    { "properties": { "b": { "type": "string" } }, "required": ["b"] },
                ] },
                "odd": { "type": "string", "items": { "type": "string" }, "properties": {} },
            },
            "required": ["path", "never", "missing"],
        });
        let out = gemini_schema(&schema);
        assert_eq!(out["type"], "object");
        for key in ["$schema", "additionalProperties", "title", "$defs"] {
            assert!(out.get(key).is_none(), "{key} 应该去掉");
        }
        let properties = &out["properties"];
        assert_eq!(
            properties["path"],
            json!({ "type": "string", "description": "路径" })
        );
        assert_eq!(
            properties["mode"],
            json!({ "type": "string", "enum": ["a", "b"] })
        );
        assert_eq!(
            properties["count"],
            json!({ "type": "integer", "nullable": true, "minimum": 0 })
        );
        assert_eq!(properties["tags"], json!({ "type": "array", "items": {} }));
        assert_eq!(properties["pair"]["items"]["type"], "number");
        assert_eq!(properties["choice"]["type"], "object");
        assert_eq!(properties["choice"]["nullable"], true);
        assert_eq!(properties["choice"]["properties"]["x"]["type"], "boolean");
        assert!(properties.get("never").is_none());
        assert_eq!(
            properties["fixed"],
            json!({ "type": "string", "enum": ["v"] })
        );
        assert_eq!(properties["level"], json!({}));
        assert_eq!(properties["both"]["required"], json!(["a", "b"]));
        assert_eq!(properties["both"]["properties"]["b"]["type"], "string");
        assert_eq!(properties["odd"], json!({ "type": "string" }));
        assert_eq!(out["required"], json!(["path"]));
    }

    #[test]
    fn schema_handles_union_with_own_shape_and_recursion() {
        let schema = json!({
            "type": "object",
            "properties": { "a": { "type": "string" } },
            "oneOf": [{ "properties": { "b": { "type": "integer" } }, "required": ["b"] }],
        });
        let out = gemini_schema(&schema);
        assert_eq!(out["type"], "object");
        assert_eq!(out["properties"]["b"]["type"], "integer");
        let recursive = json!({
            "$defs": { "Node": { "type": "object", "properties": { "next": { "$ref": "#/$defs/Node" } } } },
            "$ref": "#/$defs/Node",
        });
        let out = gemini_schema(&recursive);
        assert_eq!(out["type"], "object");
    }

    fn sse_text(events: &[Value]) -> String {
        let mut decoder = Decoder::new("gemini-3-pro-preview");
        let mut out = Vec::new();
        for event in events {
            out.extend(decoder.feed(&event.to_string()));
        }
        out.extend(decoder.end());
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn decodes_text_thought_and_usage() {
        let text = sse_text(&[
            json!({ "response": { "responseId": "r1", "candidates": [{ "content": { "role": "model", "parts": [
                { "text": "先想想", "thought": true },
            ] } }], "usageMetadata": { "promptTokenCount": 10 } } }),
            json!({ "response": { "candidates": [{ "content": { "parts": [{ "text": "你好" }] } }] } }),
            json!({ "response": { "candidates": [{ "content": { "parts": [{ "text": "！" }] }, "finishReason": "STOP" }],
                "usageMetadata": { "promptTokenCount": 10, "candidatesTokenCount": 3, "thoughtsTokenCount": 5, "cachedContentTokenCount": 4 } } }),
        ]);
        assert!(text.ends_with("data: [DONE]\n\n"));
        let completion = aggregate(&text, "m").unwrap();
        let message = &completion["choices"][0]["message"];
        assert_eq!(message["content"], "你好！");
        assert_eq!(message["reasoning_content"], "先想想");
        assert_eq!(completion["choices"][0]["finish_reason"], "stop");
        assert_eq!(completion["usage"]["prompt_tokens"], 10);
        assert_eq!(completion["usage"]["completion_tokens"], 8);
        assert_eq!(
            completion["usage"]["prompt_tokens_details"]["cached_tokens"],
            4
        );
        assert_eq!(
            completion["usage"]["completion_tokens_details"]["reasoning_tokens"],
            5
        );
    }

    #[test]
    fn decodes_function_calls_with_generated_ids_and_signatures() {
        let events = [
            json!({ "response": { "responseId": "r2", "candidates": [{ "content": { "parts": [
            { "functionCall": { "name": "default_api:shell", "args": { "cmd": "ls" } }, "thoughtSignature": "S1" },
            { "functionCall": { "name": "read", "args": null, "id": "given" } },
        ] }, "finishReason": "STOP" }] } }),
        ];
        let text = sse_text(&events);
        let completion = aggregate(&text, "m").unwrap();
        let calls = completion["choices"][0]["message"]["tool_calls"]
            .as_array()
            .unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0]["function"]["name"], "shell");
        assert_eq!(calls[0]["function"]["arguments"], "{\"cmd\":\"ls\"}");
        let id = calls[0]["id"].as_str().unwrap();
        assert!(id.starts_with("call_"));
        assert_eq!(calls[1]["id"], "given");
        assert_eq!(calls[1]["function"]["arguments"], "{}");
        assert_eq!(completion["choices"][0]["finish_reason"], "tool_calls");
        // 同样的答复生成同样的 id；签名按 id 记住了。
        let again = aggregate(&sse_text(&events), "m").unwrap();
        assert_eq!(again["choices"][0]["message"]["tool_calls"][0]["id"], id);
        assert_eq!(signature_of(id).as_deref(), Some("S1"));
    }

    #[test]
    fn maps_finish_reasons_and_errors() {
        let text = sse_text(&[
            json!({ "response": { "candidates": [{ "content": { "parts": [{ "text": "半" }] }, "finishReason": "MAX_TOKENS" }] } }),
        ]);
        assert_eq!(
            aggregate(&text, "m").unwrap()["choices"][0]["finish_reason"],
            "length"
        );
        let text =
            sse_text(&[json!({ "response": { "candidates": [{ "finishReason": "SAFETY" }] } })]);
        assert_eq!(
            aggregate(&text, "m").unwrap()["choices"][0]["finish_reason"],
            "content_filter"
        );

        // 开头就报错：交给调用方按状态码失败。
        let mut decoder = Decoder::new("m");
        let out = decoder.feed(
            &json!({ "error": { "code": 429, "message": "Resource has been exhausted" } })
                .to_string(),
        );
        assert!(out.is_empty());
        assert_eq!(
            decoder.take_early_error(),
            Some((429, "Resource has been exhausted".to_string()))
        );
        assert!(decoder.end().is_empty());

        // 输出过内容后报错：发错误事件。
        let mut decoder = Decoder::new("m");
        let mut out = decoder.feed(&json!({ "response": { "candidates": [{ "content": { "parts": [{ "text": "a" }] } }] } }).to_string());
        out.extend(
            decoder.feed(&json!({ "error": { "code": 500, "message": "boom" } }).to_string()),
        );
        let text = String::from_utf8(out).unwrap();
        assert!(
            aggregate(&text, "m")
                .unwrap_err()
                .to_string()
                .contains("boom")
        );
    }

    #[test]
    fn envelopes_differ_by_flavor() {
        let request = build_request(
            &json!({ "messages": [{ "role": "user", "content": "同一个对话" }] }),
            Flavor::Antigravity,
            "gemini-3-flash",
        );
        let wrapped = envelope(
            Flavor::Antigravity,
            "gemini-3-flash",
            "proj-1",
            request.clone(),
        );
        assert_eq!(wrapped["project"], "proj-1");
        assert_eq!(wrapped["userAgent"], "antigravity");
        assert_eq!(wrapped["requestType"], "agent");
        assert!(wrapped["requestId"].as_str().unwrap().starts_with("agent-"));
        let session = wrapped["request"]["sessionId"].as_str().unwrap();
        assert!(session.starts_with('-'));
        assert_eq!(session, session_id(&request));
        let wrapped = envelope(Flavor::GeminiCli, "gemini-2.5-pro", "proj-2", request);
        assert_eq!(wrapped["model"], "gemini-2.5-pro");
        assert!(wrapped.get("userAgent").is_none());
        assert!(wrapped["user_prompt_id"].is_string());
    }

    #[test]
    fn gemini_generation_parses_versions() {
        assert_eq!(gemini_generation("gemini-3.1-pro-preview"), 3);
        assert_eq!(gemini_generation("models/gemini-2.5-flash"), 2);
        assert_eq!(gemini_generation("gemini-flash-latest"), 0);
        assert_eq!(gemini_generation("claude-sonnet-4-6"), 0);
    }

    async fn stream_from(body: String) -> Vec<anyhow::Result<Vec<u8>>> {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;
        let response = reqwest::Client::new()
            .post(server.uri())
            .send()
            .await
            .unwrap();
        chat_stream(response, "gemini-2.5-pro").collect().await
    }

    #[tokio::test]
    async fn chat_stream_converts_sse_body() {
        let body = format!(
            "data: {}\r\n\r\ndata: {}\n\n",
            json!({ "response": { "candidates": [{ "content": { "parts": [{ "text": "你" }] } }] } }),
            json!({ "response": { "candidates": [{ "content": { "parts": [{ "text": "好" }] }, "finishReason": "STOP" }],
                "usageMetadata": { "promptTokenCount": 2, "candidatesTokenCount": 1 } } }),
        );
        let items = stream_from(body).await;
        let mut text = Vec::new();
        for item in items {
            text.extend(item.unwrap());
        }
        let completion = aggregate(&String::from_utf8(text).unwrap(), "gemini-2.5-pro").unwrap();
        assert_eq!(completion["choices"][0]["message"]["content"], "你好");
        assert_eq!(completion["usage"]["total_tokens"], 3);
    }

    #[tokio::test]
    async fn chat_stream_fails_with_status_when_first_event_is_error() {
        let body = format!(
            "data: {}\n\n",
            json!({ "error": { "code": 429, "message": "quota", "status": "RESOURCE_EXHAUSTED" } })
        );
        let items = stream_from(body).await;
        assert_eq!(items.len(), 1);
        let error = items.into_iter().next().unwrap().unwrap_err();
        let status = error
            .downcast_ref::<crate::gateway::subscriptions::SubscriptionError>()
            .map(|error| error.status);
        assert_eq!(status, Some(429));
    }
}
