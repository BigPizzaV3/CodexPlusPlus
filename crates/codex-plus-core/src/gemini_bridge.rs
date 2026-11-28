//! Codex Responses ↔ Google Gemini (`generateContent` / `streamGenerateContent`) 协议桥。
//!
//! 形态设计（参考 `anthropic_bridge`）：
//! - 请求方向：`Responses → Gemini generateContent`（直转以保留多模态、思考等级、工具与
//!   `thoughtSignature` 签名回放）；
//! - 响应方向：`Gemini → Chat Completions → Responses`（非流式 JSON 与 SSE 同构，复用
//!   `protocol_proxy` 既有测试覆盖的 `ChatSseToResponsesConverter` 与用量统计逻辑）。
//!
//! 关键兼容细节：
//! - Gemini 3+ 的工具调用要求下一轮原样回放 `thoughtSignature`；通过内存签名表 + 短标记
//!   附在 `call_id` 中穿越各类客户端，并在发回 Gemini 前解包回 `part.thoughtSignature`；
//!   无签名历史自动补 Google 官方文档指定的 `skip_thought_signature_validator`。
//! - `functionResponse` 单独成 user turn（不与普通用户文本混在同一个 turn），且首尾不以
//!   model turn 开始/结束，避免 Gemini / Vertex 400 拒绝。
//! - Gemini 3+ 发送 `thinkingLevel`（支持 xhigh 仅限 Gemini 4+），Gemini 2.5 等发送
//!   `thinkingBudget`。

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

use crate::protocol_proxy::ChatSseToResponsesConverter;

/// Google 官方文档指定的占位签名：用于非 Gemini 产生的工具调用或签名丢失的历史。
pub const SKIP_THOUGHT_SIGNATURE: &str = "skip_thought_signature_validator";

/// Gemini API Key 请求头。
pub const GEMINI_API_KEY_HEADER: &str = "x-goog-api-key";

/// 附在 `call_id` 里的签名引用前缀，长度固定极短，避免破坏客户端对 call_id 的长度限制。
const SIG_MARK: &str = "__gts";
const SIG_END: &str = "__";
const MAX_SIG_ENTRIES: usize = 4096;

struct SigStore {
    next_id: u64,
    order: VecDeque<String>,
    by_key: HashMap<String, String>,
    by_sig: HashMap<String, String>,
}

impl SigStore {
    fn new() -> Self {
        Self {
            next_id: 1,
            order: VecDeque::new(),
            by_key: HashMap::new(),
            by_sig: HashMap::new(),
        }
    }

    fn insert(&mut self, sig: &str) -> String {
        if let Some(existing) = self.by_sig.get(sig) {
            return existing.clone();
        }
        let key = format!("{:x}", self.next_id);
        self.next_id = self.next_id.wrapping_add(1).max(1);
        while self.order.len() >= MAX_SIG_ENTRIES {
            if let Some(old_key) = self.order.pop_front() {
                if let Some(old_sig) = self.by_key.remove(&old_key) {
                    self.by_sig.remove(&old_sig);
                }
            }
        }
        self.order.push_back(key.clone());
        self.by_key.insert(key.clone(), sig.to_string());
        self.by_sig.insert(sig.to_string(), key.clone());
        key
    }

    fn get(&self, key: &str) -> Option<String> {
        self.by_key.get(key).cloned()
    }
}

static SIG_STORE: LazyLock<Mutex<SigStore>> = LazyLock::new(|| Mutex::new(SigStore::new()));

/// 把 `thoughtSignature` 存入内存表，并返回带短引用的 `call_id`。
pub fn sign_call_id(id: &str, signature: &str) -> String {
    let sig = signature.trim();
    if sig.is_empty() {
        return id.to_string();
    }
    let key = SIG_STORE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(sig);
    format!("{id}{SIG_MARK}{}{SIG_END}{key}", key.len())
}

/// 从 `call_id` 中剥离签名引用，还原原始 `call_id` 与签名。
pub fn unsign_call_id(id: &str) -> (String, Option<String>) {
    let Some(pos) = id.find(SIG_MARK) else {
        return (id.to_string(), None);
    };
    let rest = &id[pos + SIG_MARK.len()..];
    let Some((len_str, key)) = rest.split_once(SIG_END) else {
        return (id.to_string(), None);
    };
    let Ok(expected_len) = len_str.parse::<usize>() else {
        return (id.to_string(), None);
    };
    if key.len() != expected_len || key.is_empty() {
        return (id.to_string(), None);
    }
    let sig = SIG_STORE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(key);
    (id[..pos].to_string(), sig)
}

/// 整理 Gemini 基础路径：
/// - 末尾带 `#` 时原样使用（去掉 `#`）；
/// - 剥掉误粘上的 `/models/...:generateContent` 等具体方法路径；
/// - 仅有域名（无路径）时补 `/v1beta`；带 `/v1` 或 `/v1beta` 等版本路径则保留。
pub fn normalize_gemini_base_url(base_url: &str) -> String {
    let trimmed = base_url.trim();
    if let Some(exact) = trimmed.strip_suffix('#') {
        return exact.trim_end_matches('/').to_string();
    }
    let mut base = trimmed.trim_end_matches('/').to_string();
    if let Some(idx) = base.find("/models/") {
        base.truncate(idx);
    } else if let Some(stripped) = base.strip_suffix("/models") {
        base = stripped.to_string();
    }
    let base = base.trim_end_matches('/');
    if base.is_empty() {
        return String::new();
    }
    let origin_only = base
        .split_once("://")
        .map_or(!base.contains('/'), |(_, rest)| !rest.contains('/'));
    if origin_only {
        format!("{base}/v1beta")
    } else {
        base.to_string()
    }
}

/// 构造 Gemini 上游生成端点：
/// - 流式：`{base}/models/{model}:streamGenerateContent?alt=sse`
/// - 非流式：`{base}/models/{model}:generateContent`
/// 若 `base_url` 末尾带 `#`，按原样地址发送（不做拼接）。
pub fn gemini_generate_url(base_url: &str, model: &str, is_stream: bool) -> String {
    let trimmed = base_url.trim();
    if let Some(exact) = trimmed.strip_suffix('#') {
        return exact.to_string();
    }
    let base = normalize_gemini_base_url(trimmed);
    let model = model.trim().trim_start_matches("models/");
    let action = if is_stream {
        "streamGenerateContent?alt=sse"
    } else {
        "generateContent"
    };
    format!("{base}/models/{model}:{action}")
}

/// 构造 Gemini 模型列表端点：`{base}/models?pageSize=1000`。
pub fn gemini_models_url(base_url: &str) -> String {
    let trimmed = base_url.trim();
    if let Some(exact) = trimmed.strip_suffix('#') {
        let exact = exact.trim_end_matches('/');
        return if exact.contains('?') {
            exact.to_string()
        } else {
            format!("{exact}?pageSize=1000")
        };
    }
    let base = normalize_gemini_base_url(trimmed);
    if base.is_empty() {
        return String::new();
    }
    format!("{base}/models?pageSize=1000")
}

/// 从模型名解析 Gemini 主版本号（如 `gemini-3.8-flash` → 3，`gemini-4-pro` → 4，`gemini-2.5-pro` → 2）。
pub fn gemini_generation(model: &str) -> u32 {
    let lower = model.to_ascii_lowercase();
    let slug = lower.rsplit('/').next().unwrap_or(&lower);
    let Some(rest) = slug.strip_prefix("gemini-") else {
        return 0;
    };
    let digits: String = rest.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    digits.parse::<u32>().unwrap_or(0)
}

/// 判断 Gemini 请求体里的 `thinkingConfig`：
/// - Gemini 3+ 使用 `thinkingLevel`；Gemini 4+ 支持 `xhigh`，Gemini 3 最高到 `high`，3 Pro 无 `medium` 升为 `high`；
/// - Gemini 2.5 等使用 `thinkingBudget`。
fn build_thinking_config(model: &str, effort: Option<&str>) -> Option<Value> {
    let effort = effort.map(str::trim).filter(|value| !value.is_empty())?;
    let lower_effort = effort.to_ascii_lowercase();
    let lower_model = model.to_ascii_lowercase();
    let generation = gemini_generation(&lower_model);

    if generation >= 3 || lower_model.contains("gemini-pro-agent") {
        let level = match lower_effort.as_str() {
            "none" | "off" => "low",
            "minimal" => {
                if lower_model.contains("flash") {
                    "minimal"
                } else {
                    "low"
                }
            }
            "low" => "low",
            "medium" => {
                if lower_model.contains("flash") || generation >= 4 {
                    "medium"
                } else {
                    "high"
                }
            }
            "xhigh" | "max" if generation >= 4 => "xhigh",
            _ => "high",
        };
        return Some(json!({
            "includeThoughts": true,
            "thinkingLevel": level,
        }));
    }

    let budget: i64 = match lower_effort.as_str() {
        "none" | "off" => {
            if lower_model.contains("flash") {
                0
            } else {
                128
            }
        }
        "minimal" => 1024,
        "low" => 4096,
        "medium" => 8192,
        "high" | "xhigh" | "max" => 16384,
        _ => 8192,
    };
    if budget == 0 {
        return Some(json!({ "thinkingBudget": 0 }));
    }
    Some(json!({
        "includeThoughts": true,
        "thinkingBudget": budget,
    }))
}

/// 拆分 `data:<mime>;base64,<data>` URL。
fn parse_data_url(url: &str) -> Option<(String, String)> {
    let rest = url.strip_prefix("data:")?;
    let (meta, data) = rest.split_once(',')?;
    let mime = meta.split(';').next().unwrap_or("application/octet-stream").trim();
    if !meta.to_ascii_lowercase().contains(";base64") {
        return None;
    }
    Some((mime.to_string(), data.trim().to_string()))
}

fn responses_content_part_to_gemini(part: &Value) -> Option<Value> {
    let kind = part.get("type").and_then(Value::as_str).unwrap_or("");
    match kind {
        "input_text" | "output_text" | "text" => {
            let text = part.get("text").and_then(Value::as_str).unwrap_or("");
            if text.is_empty() {
                None
            } else {
                Some(json!({ "text": text }))
            }
        }
        "input_image" => {
            let url = part
                .get("image_url")
                .and_then(|value| {
                    value
                        .as_str()
                        .or_else(|| value.get("url").and_then(Value::as_str))
                })
                .unwrap_or("")
                .trim();
            if url.is_empty() {
                return None;
            }
            if let Some((mime, data)) = parse_data_url(url) {
                Some(json!({ "inlineData": { "mimeType": mime, "data": data } }))
            } else {
                Some(json!({ "fileData": { "mimeType": "image/jpeg", "fileUri": url } }))
            }
        }
        "input_file" => {
            if let Some(file_data) = part.get("file_data").and_then(Value::as_str) {
                if let Some((mime, data)) = parse_data_url(file_data) {
                    return Some(json!({ "inlineData": { "mimeType": mime, "data": data } }));
                }
                return Some(json!({
                    "inlineData": { "mimeType": "application/pdf", "data": file_data.trim() }
                }));
            }
            if let Some(file_url) = part.get("file_url").and_then(Value::as_str) {
                return Some(json!({
                    "fileData": { "mimeType": "application/pdf", "fileUri": file_url.trim() }
                }));
            }
            None
        }
        _ => None,
    }
}

fn push_gemini_turn(contents: &mut Vec<Value>, role: &str, parts: Vec<Value>) {
    if parts.is_empty() {
        return;
    }
    let has_fn_resp = parts.iter().any(|part| part.get("functionResponse").is_some());
    if let Some(last) = contents.last_mut() {
        let same_role = last.get("role").and_then(Value::as_str) == Some(role);
        let last_has_fn_resp = last
            .get("parts")
            .and_then(Value::as_array)
            .is_some_and(|arr| arr.iter().any(|part| part.get("functionResponse").is_some()));
        // Gemini 不允许把 functionResponse 与普通 user 文本混在同一个 turn 里，
        // 同类型（都是 functionResponse 或都不是）才合并。
        if same_role && (role != "user" || last_has_fn_resp == has_fn_resp) {
            if let Some(arr) = last.get_mut("parts").and_then(Value::as_array_mut) {
                arr.extend(parts);
                return;
            }
        }
    }
    contents.push(json!({
        "role": role,
        "parts": parts,
    }));
}

fn parse_tool_arguments(raw: Option<&Value>) -> Value {
    match raw {
        Some(Value::Object(map)) => Value::Object(map.clone()),
        Some(Value::String(text)) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                json!({})
            } else {
                serde_json::from_str::<Value>(trimmed)
                    .ok()
                    .filter(Value::is_object)
                    .unwrap_or_else(|| json!({ "raw": text }))
            }
        }
        _ => json!({}),
    }
}

fn extract_tool_output(output: Option<&Value>) -> (String, Vec<Value>) {
    let mut texts = Vec::new();
    let mut media_parts = Vec::new();
    match output {
        Some(Value::String(text)) => texts.push(text.clone()),
        Some(Value::Array(items)) => {
            for item in items {
                if let Some(text) = item.as_str() {
                    texts.push(text.to_string());
                    continue;
                }
                let kind = item.get("type").and_then(Value::as_str).unwrap_or("");
                if matches!(kind, "input_text" | "output_text" | "text") {
                    if let Some(text) = item.get("text").and_then(Value::as_str) {
                        texts.push(text.to_string());
                    }
                } else if let Some(part) = responses_content_part_to_gemini(item) {
                    media_parts.push(part);
                } else {
                    texts.push(item.to_string());
                }
            }
        }
        Some(other) if !other.is_null() => texts.push(other.to_string()),
        _ => {}
    }
    (texts.join("\n"), media_parts)
}

/// 补齐历史中被中断（没有对应 `functionResponse`）的 `functionCall`，避免 Gemini 返回 400。
fn pair_function_turns(contents: Vec<Value>) -> Vec<Value> {
    let mut answered_ids = HashSet::new();
    let mut answered_names = HashSet::new();
    for content in &contents {
        if let Some(parts) = content.get("parts").and_then(Value::as_array) {
            for part in parts {
                if let Some(fr) = part.get("functionResponse") {
                    if let Some(id) = fr.get("id").and_then(Value::as_str).filter(|s| !s.is_empty())
                    {
                        answered_ids.insert(id.to_string());
                    } else if let Some(name) = fr.get("name").and_then(Value::as_str) {
                        answered_names.insert(name.to_string());
                    }
                }
            }
        }
    }

    let mut out = Vec::with_capacity(contents.len());
    let mut index = 0;
    while index < contents.len() {
        let content = contents[index].clone();
        let is_model = content.get("role").and_then(Value::as_str) == Some("model");
        let mut missing = Vec::new();
        if is_model {
            if let Some(parts) = content.get("parts").and_then(Value::as_array) {
                for part in parts {
                    let Some(fc) = part.get("functionCall") else {
                        continue;
                    };
                    let id = fc.get("id").and_then(Value::as_str).unwrap_or("");
                    let name = fc.get("name").and_then(Value::as_str).unwrap_or("tool");
                    let answered = (!id.is_empty() && answered_ids.contains(id))
                        || (id.is_empty() && answered_names.contains(name));
                    if !answered {
                        let mut fr = json!({
                            "name": name,
                            "response": { "output": "[Tool execution interrupted]" },
                        });
                        if !id.is_empty() {
                            fr["id"] = json!(id);
                        }
                        missing.push(json!({ "functionResponse": fr }));
                    }
                }
            }
        }
        out.push(content);
        if !missing.is_empty() {
            // 若紧随其后就是一个 functionResponse turn，合并到前面；否则插入独立 user turn。
            let next_is_fn_resp = contents.get(index + 1).is_some_and(|next| {
                next.get("role").and_then(Value::as_str) == Some("user")
                    && next
                        .get("parts")
                        .and_then(Value::as_array)
                        .is_some_and(|arr| arr.iter().any(|p| p.get("functionResponse").is_some()))
            });
            if next_is_fn_resp {
                let mut next = contents[index + 1].clone();
                if let Some(parts) = next.get_mut("parts").and_then(Value::as_array_mut) {
                    missing.append(parts);
                    *parts = missing;
                }
                out.push(next);
                index += 2;
                continue;
            }
            out.push(json!({ "role": "user", "parts": missing }));
        }
        index += 1;
    }
    out
}

/// 清理 JSON Schema 中 Gemini 不接受的布尔 schema 与 `x-` 扩展字段。
fn sanitize_gemini_schema(value: &Value) -> Value {
    match value {
        Value::Bool(true) => json!({}),
        Value::Bool(false) => json!({ "type": "string" }),
        Value::Array(items) => Value::Array(items.iter().map(sanitize_gemini_schema).collect()),
        Value::Object(map) => {
            let mut out = Map::new();
            for (key, val) in map {
                if key.starts_with("x-") {
                    continue;
                }
                if key == "additionalProperties" {
                    // additionalProperties: false 是标准 JSON Schema 写法，保留给 parametersJsonSchema；
                    // 若是子 schema 对象则递归清理。
                    if val.is_boolean() {
                        out.insert(key.clone(), val.clone());
                    } else {
                        out.insert(key.clone(), sanitize_gemini_schema(val));
                    }
                    continue;
                }
                if key == "properties" {
                    if let Some(props) = val.as_object() {
                        let mut clean_props = Map::new();
                        for (prop_name, prop_schema) in props {
                            if prop_schema.as_bool() == Some(false) {
                                continue;
                            }
                            clean_props.insert(prop_name.clone(), sanitize_gemini_schema(prop_schema));
                        }
                        out.insert(key.clone(), Value::Object(clean_props));
                        continue;
                    }
                }
                out.insert(key.clone(), sanitize_gemini_schema(val));
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

/// 将 Codex Responses API 请求体转换为 Google Gemini `generateContent` 请求体。
pub fn responses_to_gemini_request(request_json: Value) -> anyhow::Result<Value> {
    let model = request_json
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    let mut system_parts = Vec::new();
    if let Some(instructions) = request_json
        .get("instructions")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        system_parts.push(instructions.to_string());
    }

    let mut contents: Vec<Value> = Vec::new();
    let mut call_names: HashMap<String, String> = HashMap::new();
    let mut seen_responses: HashSet<String> = HashSet::new();

    // 预扫一遍 input，记下每个 function_call 的原始 call_id → 函数名。
    let input_items: Vec<Value> = match request_json.get("input") {
        Some(Value::String(text)) => vec![json!({
            "type": "message",
            "role": "user",
            "content": [{ "type": "input_text", "text": text }]
        })],
        Some(Value::Array(items)) => items.clone(),
        _ => Vec::new(),
    };

    for item in &input_items {
        if item.get("type").and_then(Value::as_str) == Some("function_call") {
            let raw_id = item
                .get("call_id")
                .or_else(|| item.get("id"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let (clean_id, _) = unsign_call_id(raw_id);
            let name = item.get("name").and_then(Value::as_str).unwrap_or("tool");
            if !clean_id.is_empty() {
                call_names.insert(clean_id, name.to_string());
            }
        }
    }

    for item in &input_items {
        let item_type = item.get("type").and_then(Value::as_str).unwrap_or("message");
        match item_type {
            "message" => {
                let role = item.get("role").and_then(Value::as_str).unwrap_or("user");
                if matches!(role, "system" | "developer") {
                    let text = match item.get("content") {
                        Some(Value::String(text)) => text.trim().to_string(),
                        Some(Value::Array(arr)) => arr
                            .iter()
                            .filter_map(|part| part.get("text").and_then(Value::as_str))
                            .filter(|text| !text.trim().is_empty())
                            .collect::<Vec<_>>()
                            .join("\n\n"),
                        _ => String::new(),
                    };
                    if !text.is_empty() {
                        system_parts.push(text);
                    }
                    continue;
                }
                let gemini_role = if role == "assistant" { "model" } else { "user" };
                let mut parts = Vec::new();
                match item.get("content") {
                    Some(Value::String(text)) if !text.is_empty() => {
                        parts.push(json!({ "text": text }));
                    }
                    Some(Value::Array(arr)) => {
                        for part in arr {
                            if let Some(converted) = responses_content_part_to_gemini(part) {
                                parts.push(converted);
                            }
                        }
                    }
                    _ => {}
                }
                push_gemini_turn(&mut contents, gemini_role, parts);
            }
            "function_call" => {
                let raw_id = item
                    .get("call_id")
                    .or_else(|| item.get("id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let (clean_id, sig) = unsign_call_id(raw_id);
                let name = item.get("name").and_then(Value::as_str).unwrap_or("tool");
                let args = parse_tool_arguments(item.get("arguments"));
                let mut fc = json!({
                    "name": name,
                    "args": args,
                });
                if !clean_id.is_empty() {
                    fc["id"] = json!(clean_id);
                }
                let mut part = json!({ "functionCall": fc });
                if let Some(signature) = sig {
                    part["thoughtSignature"] = json!(signature);
                } else {
                    // 同一 model turn 已有带签名的 call 时不再覆盖，否则补官方 skip 占位签名，
                    // 避免 Gemini 3 报 "Function call is missing a thought_signature"。
                    let turn_already_signed = contents
                        .last()
                        .filter(|turn| turn.get("role").and_then(Value::as_str) == Some("model"))
                        .and_then(|turn| turn.get("parts").and_then(Value::as_array))
                        .is_some_and(|parts| {
                            parts.iter().any(|existing| {
                                existing.get("functionCall").is_some()
                                    && existing
                                        .get("thoughtSignature")
                                        .and_then(Value::as_str)
                                        .is_some_and(|s| !s.is_empty() && s != SKIP_THOUGHT_SIGNATURE)
                            })
                        });
                    if !turn_already_signed {
                        part["thoughtSignature"] = json!(SKIP_THOUGHT_SIGNATURE);
                    }
                }
                push_gemini_turn(&mut contents, "model", vec![part]);
            }
            "function_call_output" => {
                let raw_id = item.get("call_id").and_then(Value::as_str).unwrap_or("");
                let (clean_id, _) = unsign_call_id(raw_id);
                if !clean_id.is_empty() && !seen_responses.insert(clean_id.clone()) {
                    // 同一个 call_id 的工具结果只保留第一份，重复的不再发给 Gemini。
                    continue;
                }
                let name = item
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .or_else(|| call_names.get(&clean_id).cloned())
                    .unwrap_or_else(|| "tool".to_string());
                let (text, media_parts) = extract_tool_output(item.get("output"));
                let mut fr = json!({
                    "name": name,
                    "response": { "output": text },
                });
                if !clean_id.is_empty() {
                    fr["id"] = json!(clean_id);
                }
                let mut parts = vec![json!({ "functionResponse": fr })];
                parts.extend(media_parts);
                push_gemini_turn(&mut contents, "user", parts);
            }
            // reasoning 项不原样回发（签名已随 functionCall 回放），避免发未知签名被拒。
            "reasoning" | "compaction" | "web_search_call" => {}
            _ => {}
        }
    }

    let mut contents = pair_function_turns(contents);
    if contents
        .first()
        .is_some_and(|turn| turn.get("role").and_then(Value::as_str) == Some("model"))
    {
        contents.insert(
            0,
            json!({
                "role": "user",
                "parts": [{ "text": "[Continuing from previous AI thoughts...]" }]
            }),
        );
    }
    if contents
        .last()
        .is_some_and(|turn| turn.get("role").and_then(Value::as_str) == Some("model"))
    {
        contents.push(json!({
            "role": "user",
            "parts": [{ "text": "Continue." }]
        }));
    }
    if contents.is_empty() {
        contents.push(json!({
            "role": "user",
            "parts": [{ "text": "hi" }]
        }));
    }

    let mut body = Map::new();
    body.insert("contents".to_string(), Value::Array(contents));

    if !system_parts.is_empty() {
        body.insert(
            "systemInstruction".to_string(),
            json!({
                "parts": [{ "text": system_parts.join("\n\n") }]
            }),
        );
    }

    // 工具与 tool_choice
    let tool_choice = request_json.get("tool_choice");
    let choice_none = tool_choice.and_then(Value::as_str) == Some("none");
    let mut function_decls = Vec::new();
    let mut wants_google_search = false;
    if let Some(tools) = request_json.get("tools").and_then(Value::as_array) {
        for tool in tools {
            let kind = tool.get("type").and_then(Value::as_str).unwrap_or("");
            match kind {
                "function" => {
                    let Some(name) = tool.get("name").and_then(Value::as_str).filter(|s| !s.is_empty())
                    else {
                        continue;
                    };
                    let mut decl = Map::new();
                    decl.insert("name".to_string(), json!(name));
                    if let Some(desc) = tool.get("description").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                        decl.insert("description".to_string(), json!(desc));
                    }
                    let schema = tool
                        .get("parameters")
                        .filter(|value| value.is_object())
                        .map(sanitize_gemini_schema)
                        .unwrap_or_else(|| json!({ "type": "object", "properties": {} }));
                    decl.insert("parametersJsonSchema".to_string(), schema);
                    function_decls.push(Value::Object(decl));
                }
                "web_search" | "web_search_preview" | "web_search_preview_2025_03_11" => {
                    wants_google_search = true;
                }
                _ => {}
            }
        }
    }

    if !function_decls.is_empty() && !choice_none {
        body.insert(
            "tools".to_string(),
            json!([{ "functionDeclarations": function_decls }]),
        );
        let mut fc = Map::new();
        match tool_choice {
            Some(Value::String(mode)) if mode == "required" => {
                fc.insert("mode".to_string(), json!("ANY"));
            }
            Some(Value::Object(obj)) => {
                if let Some(name) = obj.get("name").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                    fc.insert("mode".to_string(), json!("ANY"));
                    fc.insert("allowedFunctionNames".to_string(), json!([name]));
                } else {
                    fc.insert("mode".to_string(), json!("AUTO"));
                }
            }
            _ => {
                fc.insert("mode".to_string(), json!("AUTO"));
            }
        }
        body.insert(
            "toolConfig".to_string(),
            json!({ "functionCallingConfig": Value::Object(fc) }),
        );
    } else if choice_none && !function_decls.is_empty() {
        body.insert(
            "toolConfig".to_string(),
            json!({ "functionCallingConfig": { "mode": "NONE" } }),
        );
    } else if wants_google_search && !choice_none {
        body.insert("tools".to_string(), json!([{ "googleSearch": {} }]));
    }

    // generationConfig
    let mut gen_config = Map::new();
    if let Some(max_tokens) = request_json
        .get("max_output_tokens")
        .or_else(|| request_json.get("max_tokens"))
        .and_then(Value::as_u64)
        .filter(|&value| value > 0)
    {
        gen_config.insert("maxOutputTokens".to_string(), json!(max_tokens));
    }
    if let Some(temp) = request_json.get("temperature").and_then(Value::as_f64) {
        gen_config.insert("temperature".to_string(), json!(temp));
    }
    if let Some(top_p) = request_json.get("top_p").and_then(Value::as_f64) {
        gen_config.insert("topP".to_string(), json!(top_p));
    }
    if let Some(format) = request_json.pointer("/text/format") {
        let format_type = format.get("type").and_then(Value::as_str).unwrap_or("");
        if format_type == "json_object" {
            gen_config.insert("responseMimeType".to_string(), json!("application/json"));
        } else if format_type == "json_schema" {
            gen_config.insert("responseMimeType".to_string(), json!("application/json"));
            if let Some(schema) = format.get("schema").filter(|value| value.is_object()) {
                gen_config.insert("responseJsonSchema".to_string(), sanitize_gemini_schema(schema));
            }
        }
    }
    let effort = request_json
        .pointer("/reasoning/effort")
        .and_then(Value::as_str);
    if let Some(thinking) = build_thinking_config(&model, effort) {
        gen_config.insert("thinkingConfig".to_string(), thinking);
    }
    if !gen_config.is_empty() {
        body.insert("generationConfig".to_string(), Value::Object(gen_config));
    }

    Ok(Value::Object(body))
}

fn unwrap_gemini_envelope(payload: &Value) -> &Value {
    payload
        .get("response")
        .filter(|value| value.is_object())
        .unwrap_or(payload)
}

fn gemini_finish_reason_to_chat(finish_reason: Option<&str>, has_tool_calls: bool) -> Option<&'static str> {
    let reason = finish_reason?.trim();
    if reason.is_empty() {
        return None;
    }
    if has_tool_calls && reason == "STOP" {
        return Some("tool_calls");
    }
    match reason {
        "STOP" => Some("stop"),
        "MAX_TOKENS" => Some("length"),
        "SAFETY" | "RECITATION" | "BLOCKLIST" | "PROHIBITED_CONTENT" | "SPII" => {
            Some("content_filter")
        }
        _ => Some("stop"),
    }
}

fn gemini_usage_to_chat(usage: Option<&Value>) -> Option<Value> {
    let usage = usage?;
    let prompt = usage
        .get("promptTokenCount")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let candidates = usage
        .get("candidatesTokenCount")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let thoughts = usage
        .get("thoughtsTokenCount")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cached = usage
        .get("cachedContentTokenCount")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let completion = candidates.saturating_add(thoughts);
    let total = usage
        .get("totalTokenCount")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| prompt.saturating_add(completion));
    let mut out = json!({
        "prompt_tokens": prompt,
        "completion_tokens": completion,
        "total_tokens": total,
    });
    if cached > 0 {
        out["prompt_tokens_details"] = json!({ "cached_tokens": cached });
    }
    if thoughts > 0 {
        out["completion_tokens_details"] = json!({ "reasoning_tokens": thoughts });
    }
    Some(out)
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

/// 将非流式 Gemini `generateContent` JSON 响应转为 OpenAI Chat Completions JSON，
/// 再由外层复用既有 `chat_completion_to_response`。
pub fn gemini_response_to_chat_completion(response: Value) -> anyhow::Result<Value> {
    let body = unwrap_gemini_envelope(&response);
    if let Some(error) = body.get("error").filter(|value| !value.is_null()) {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("Gemini upstream returned an error");
        anyhow::bail!("{message}");
    }
    let id = body
        .get("responseId")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("chatcmpl-gemini-{}", now_unix_secs()));
    let model = body
        .get("modelVersion")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    let candidate = body
        .get("candidates")
        .and_then(Value::as_array)
        .and_then(|arr| arr.first());

    let mut content = String::new();
    let mut reasoning = String::new();
    let mut tool_calls = Vec::new();

    if let Some(parts) = candidate
        .and_then(|c| c.pointer("/content/parts"))
        .and_then(Value::as_array)
    {
        for (idx, part) in parts.iter().enumerate() {
            if let Some(fc) = part.get("functionCall") {
                let raw_id = fc
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("call_gemini_{idx}"));
                let sig = part
                    .get("thoughtSignature")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let signed = sign_call_id(&raw_id, sig);
                let name = fc.get("name").and_then(Value::as_str).unwrap_or("tool");
                let args = fc
                    .get("args")
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "{}".to_string());
                tool_calls.push(json!({
                    "id": signed,
                    "type": "function",
                    "function": {
                        "name": name,
                        "arguments": args,
                    }
                }));
            } else if part.get("thought").and_then(Value::as_bool) == Some(true) {
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    reasoning.push_str(text);
                }
            } else if let Some(text) = part.get("text").and_then(Value::as_str) {
                content.push_str(text);
            }
        }
    }

    let mut message = Map::new();
    message.insert("role".to_string(), json!("assistant"));
    message.insert(
        "content".to_string(),
        if content.is_empty() && !tool_calls.is_empty() {
            Value::Null
        } else {
            json!(content)
        },
    );
    if !reasoning.is_empty() {
        message.insert("reasoning_content".to_string(), json!(reasoning));
    }
    if !tool_calls.is_empty() {
        message.insert("tool_calls".to_string(), Value::Array(tool_calls.clone()));
    }

    let finish_reason = gemini_finish_reason_to_chat(
        candidate
            .and_then(|c| c.get("finishReason"))
            .and_then(Value::as_str),
        !tool_calls.is_empty(),
    )
    .unwrap_or("stop");

    let mut chat = json!({
        "id": id,
        "object": "chat.completion",
        "created": now_unix_secs(),
        "model": model,
        "choices": [{
            "index": 0,
            "message": Value::Object(message),
            "finish_reason": finish_reason,
        }],
    });
    if let Some(usage) = gemini_usage_to_chat(body.get("usageMetadata")) {
        chat["usage"] = usage;
    }
    Ok(chat)
}

/// 流式 Gemini SSE → Chat Completions SSE 增量转换器。
#[derive(Default)]
pub struct GeminiSseToChatSse {
    buffer: String,
    utf8_remainder: Vec<u8>,
    response_id: String,
    model: String,
    sent_role: bool,
    saw_tool_calls: bool,
    tool_index: usize,
    saw_finish: bool,
    finish_reason: Option<String>,
    last_usage: Option<Value>,
    stream_error: Option<String>,
}

impl GeminiSseToChatSse {
    pub fn push_bytes(&mut self, bytes: &[u8]) -> Vec<u8> {
        append_utf8_safe(&mut self.buffer, &mut self.utf8_remainder, bytes);
        let mut output = String::new();
        while let Some(block) = take_sse_block(&mut self.buffer) {
            self.process_block(&block, &mut output);
        }
        output.into_bytes()
    }

    pub fn finish(&mut self) -> Vec<u8> {
        let mut output = String::new();
        if !self.utf8_remainder.is_empty() {
            let tail = String::from_utf8_lossy(&self.utf8_remainder).to_string();
            self.utf8_remainder.clear();
            self.buffer.push_str(&tail);
        }
        if !self.buffer.trim().is_empty() {
            let remaining = std::mem::take(&mut self.buffer);
            self.process_block(&remaining, &mut output);
        }
        if self.stream_error.is_none() {
            if let Some(usage) = self.last_usage.take() {
                let chunk = self.chunk_envelope(json!([]), Some(usage));
                emit_chat_chunk(&mut output, &chunk);
            }
            if self.saw_finish {
                output.push_str("data: [DONE]\n\n");
            }
        }
        output.into_bytes()
    }

    pub fn has_terminal_event(&self) -> bool {
        self.saw_finish
    }

    pub fn has_failed(&self) -> bool {
        self.stream_error.is_some()
    }

    pub fn stream_error(&self) -> Option<&str> {
        self.stream_error.as_deref()
    }

    pub fn has_successful_compaction_completion(&self) -> bool {
        self.stream_error.is_none()
            && matches!(self.finish_reason.as_deref(), Some("STOP"))
    }

    fn chunk_envelope(&mut self, choices: Value, usage: Option<Value>) -> Value {
        if self.response_id.is_empty() {
            self.response_id = format!("chatcmpl-gemini-{}", now_unix_secs());
        }
        let mut obj = json!({
            "id": self.response_id,
            "object": "chat.completion.chunk",
            "created": now_unix_secs(),
            "model": self.model,
            "choices": choices,
        });
        if let Some(usage) = usage {
            obj["usage"] = usage;
        }
        obj
    }

    fn emit_delta(&mut self, output: &mut String, mut delta: Map<String, Value>, finish_reason: Option<&str>) {
        if !self.sent_role {
            delta.insert("role".to_string(), json!("assistant"));
            self.sent_role = true;
        }
        let chunk = self.chunk_envelope(
            json!([{
                "index": 0,
                "delta": Value::Object(delta),
                "finish_reason": finish_reason,
            }]),
            None,
        );
        emit_chat_chunk(output, &chunk);
    }

    fn process_block(&mut self, block: &str, output: &mut String) {
        let mut data_lines = Vec::new();
        for line in block.lines() {
            let line = line.trim_end_matches('\r');
            if let Some(data) = line.strip_prefix("data:") {
                data_lines.push(data.trim_start());
            }
        }
        let data = data_lines.join("\n");
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            return;
        }
        let Ok(payload) = serde_json::from_str::<Value>(data) else {
            return;
        };
        let body = unwrap_gemini_envelope(&payload);
        if let Some(error) = body.get("error").filter(|value| !value.is_null()) {
            let msg = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("Gemini upstream stream error")
                .to_string();
            self.stream_error = Some(msg);
            return;
        }
        if let Some(id) = body.get("responseId").and_then(Value::as_str).filter(|s| !s.is_empty()) {
            self.response_id = id.to_string();
        }
        if let Some(model) = body.get("modelVersion").and_then(Value::as_str).filter(|s| !s.is_empty()) {
            self.model = model.to_string();
        }
        if let Some(usage) = gemini_usage_to_chat(body.get("usageMetadata")) {
            self.last_usage = Some(usage);
        }

        let Some(candidates) = body.get("candidates").and_then(Value::as_array) else {
            return;
        };
        for cand in candidates {
            if let Some(parts) = cand.pointer("/content/parts").and_then(Value::as_array) {
                for part in parts {
                    if let Some(fc) = part.get("functionCall") {
                        self.saw_tool_calls = true;
                        let index = self.tool_index;
                        self.tool_index += 1;
                        let raw_id = fc
                            .get("id")
                            .and_then(Value::as_str)
                            .filter(|s| !s.is_empty())
                            .map(str::to_string)
                            .unwrap_or_else(|| format!("call_gemini_{index}"));
                        let sig = part
                            .get("thoughtSignature")
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        let signed = sign_call_id(&raw_id, sig);
                        let name = fc.get("name").and_then(Value::as_str).unwrap_or("tool");
                        let args = fc
                            .get("args")
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "{}".to_string());
                        let mut delta = Map::new();
                        delta.insert(
                            "tool_calls".to_string(),
                            json!([{
                                "index": index,
                                "id": signed,
                                "type": "function",
                                "function": {
                                    "name": name,
                                    "arguments": args,
                                }
                            }]),
                        );
                        self.emit_delta(output, delta, None);
                    } else if part.get("thought").and_then(Value::as_bool) == Some(true) {
                        if let Some(text) = part.get("text").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                            let mut delta = Map::new();
                            delta.insert("reasoning_content".to_string(), json!(text));
                            self.emit_delta(output, delta, None);
                        }
                    } else if let Some(text) = part.get("text").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                        let mut delta = Map::new();
                        delta.insert("content".to_string(), json!(text));
                        self.emit_delta(output, delta, None);
                    }
                }
            }
            if let Some(reason) = cand.get("finishReason").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                self.saw_finish = true;
                self.finish_reason = Some(reason.to_string());
                let chat_reason = gemini_finish_reason_to_chat(Some(reason), self.saw_tool_calls);
                self.emit_delta(output, Map::new(), chat_reason);
            }
        }
    }
}

/// 流式 Gemini SSE → Responses SSE 增量转换器（供 launcher 与 protocol_proxy 使用）。
#[derive(Default)]
pub struct GeminiSseToResponsesConverter {
    bridge: GeminiSseToChatSse,
    inner: ChatSseToResponsesConverter,
}

impl GeminiSseToResponsesConverter {
    pub fn with_request(request: &Value) -> Self {
        Self {
            bridge: GeminiSseToChatSse::default(),
            inner: ChatSseToResponsesConverter::with_request(request),
        }
    }

    pub fn push_bytes(&mut self, bytes: &[u8]) -> Vec<u8> {
        let chat_bytes = self.bridge.push_bytes(bytes);
        if let Some(error) = self.bridge.stream_error().map(str::to_string) {
            return self.inner.fail(error, Some("upstream_stream_error".to_string()));
        }
        if chat_bytes.is_empty() {
            return Vec::new();
        }
        self.inner.push_bytes(&chat_bytes)
    }

    pub fn finish(&mut self) -> Vec<u8> {
        let mut output = Vec::new();
        let chat_tail = self.bridge.finish();
        if let Some(error) = self.bridge.stream_error().map(str::to_string) {
            return self.inner.fail(error, Some("upstream_stream_error".to_string()));
        }
        if !chat_tail.is_empty() {
            output.extend(self.inner.push_bytes(&chat_tail));
        }
        output.extend(self.inner.finish());
        output
    }

    pub fn fail(&mut self, message: String, code: Option<String>) -> Vec<u8> {
        self.inner.fail(message, code)
    }

    pub fn has_terminal_event(&self) -> bool {
        self.bridge.has_terminal_event()
    }

    pub fn has_failed(&self) -> bool {
        self.bridge.has_failed() || self.inner.has_failed()
    }

    pub fn request_usage(&self) -> Option<crate::request_log::RequestUsage> {
        self.inner.request_usage()
    }
}

/// 一次性把整段 Gemini SSE 转成 Responses SSE（非 launcher 路径使用）。
pub fn gemini_sse_to_responses_sse(input: &str, request: &Value) -> Vec<u8> {
    let mut converter = GeminiSseToResponsesConverter::with_request(request);
    let mut output = converter.push_bytes(input.as_bytes());
    output.extend(converter.finish());
    output
}

/// 一次性把整段 Gemini SSE 转成 Chat SSE（压缩聚合器与测试使用）。
pub fn gemini_sse_to_chat_sse(input: &str) -> String {
    let mut converter = GeminiSseToChatSse::default();
    let mut output = converter.push_bytes(input.as_bytes());
    output.extend(converter.finish());
    String::from_utf8_lossy(&output).to_string()
}

fn emit_chat_chunk(output: &mut String, chunk: &Value) {
    output.push_str("data: ");
    output.push_str(&chunk.to_string());
    output.push_str("\n\n");
}

fn take_sse_block(buffer: &mut String) -> Option<String> {
    let lf = buffer.find("\n\n").map(|index| (index, 2));
    let crlf = buffer.find("\r\n\r\n").map(|index| (index, 4));
    let (index, delimiter_len) = match (lf, crlf) {
        (Some(left), Some(right)) => {
            if left.0 <= right.0 {
                left
            } else {
                right
            }
        }
        (Some(value), None) | (None, Some(value)) => value,
        (None, None) => return None,
    };
    let block = buffer[..index].to_string();
    buffer.drain(..index + delimiter_len);
    Some(block)
}

fn append_utf8_safe(buffer: &mut String, remainder: &mut Vec<u8>, bytes: &[u8]) {
    if bytes.is_empty() {
        return;
    }
    let mut combined = Vec::new();
    if !remainder.is_empty() {
        combined.extend_from_slice(remainder);
        remainder.clear();
    }
    combined.extend_from_slice(bytes);
    match std::str::from_utf8(&combined) {
        Ok(text) => buffer.push_str(text),
        Err(error) => {
            let valid = error.valid_up_to();
            if valid > 0 {
                buffer.push_str(std::str::from_utf8(&combined[..valid]).unwrap_or_default());
            }
            if error.error_len().is_none() {
                remainder.extend_from_slice(&combined[valid..]);
            } else {
                buffer.push_str(&String::from_utf8_lossy(&combined[valid..]));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gemini_urls_follow_v1beta_and_exact_hash_rules() {
        assert_eq!(
            gemini_generate_url("https://generativelanguage.googleapis.com", "gemini-2.5-pro", true),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-pro:streamGenerateContent?alt=sse"
        );
        assert_eq!(
            gemini_generate_url("https://generativelanguage.googleapis.com/v1beta/", "models/gemini-2.5-flash", false),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:generateContent"
        );
        assert_eq!(
            gemini_generate_url("https://relay.example.com/custom/endpoint#", "gemini-2.5-pro", true),
            "https://relay.example.com/custom/endpoint"
        );
        assert_eq!(
            gemini_models_url("https://generativelanguage.googleapis.com"),
            "https://generativelanguage.googleapis.com/v1beta/models?pageSize=1000"
        );
        assert_eq!(
            gemini_models_url("https://relay.example.com/google/v1beta/models/gemini-2.5-pro:generateContent"),
            "https://relay.example.com/google/v1beta/models?pageSize=1000"
        );
    }

    #[test]
    fn responses_to_gemini_maps_system_tools_thinking_and_signatures() {
        let signed = sign_call_id("call_1", "sig_abc_123");
        let req = json!({
            "model": "gemini-3.5-pro",
            "instructions": "Be concise.",
            "reasoning": { "effort": "medium" },
            "max_output_tokens": 1024,
            "tools": [{
                "type": "function",
                "name": "read_file",
                "description": "Read a file",
                "parameters": {
                    "type": "object",
                    "x-vendor": "drop-me",
                    "properties": {
                        "path": { "type": "string" }
                    },
                    "required": ["path"]
                }
            }],
            "input": [
                { "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "hello" }] },
                { "type": "function_call", "call_id": signed, "name": "read_file", "arguments": "{\"path\":\"a.txt\"}" },
                { "type": "function_call_output", "call_id": signed, "output": "file body" },
                { "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "summarize" }] }
            ]
        });
        let out = responses_to_gemini_request(req).unwrap();
        assert_eq!(out["systemInstruction"]["parts"][0]["text"], "Be concise.");
        // Gemini 3 Pro 没有 medium，提升为 high。
        assert_eq!(out["generationConfig"]["thinkingConfig"]["thinkingLevel"], "high");
        assert_eq!(out["generationConfig"]["thinkingConfig"]["includeThoughts"], true);
        assert_eq!(out["generationConfig"]["maxOutputTokens"], 1024);
        // x-vendor 已清理。
        assert!(out["tools"][0]["functionDeclarations"][0]["parametersJsonSchema"].get("x-vendor").is_none());
        // 4 个独立 turn：user("hello") -> model(functionCall + signature) -> user(functionResponse) -> user("summarize")
        let contents = out["contents"].as_array().unwrap();
        assert_eq!(contents.len(), 4);
        assert_eq!(contents[1]["parts"][0]["functionCall"]["id"], "call_1");
        assert_eq!(contents[1]["parts"][0]["thoughtSignature"], "sig_abc_123");
        assert_eq!(contents[2]["parts"][0]["functionResponse"]["name"], "read_file");
        assert_eq!(contents[2]["parts"][0]["functionResponse"]["id"], "call_1");
        assert_eq!(contents[3]["parts"][0]["text"], "summarize");
    }

    #[test]
    fn unsigned_function_call_gets_skip_signature_and_missing_response_is_paired() {
        let req = json!({
            "model": "gemini-2.5-flash",
            "input": [
                { "type": "message", "role": "user", "content": "run tool" },
                { "type": "function_call", "call_id": "call_raw", "name": "ping", "arguments": "{}" }
            ]
        });
        let out = responses_to_gemini_request(req).unwrap();
        let contents = out["contents"].as_array().unwrap();
        assert_eq!(contents[1]["parts"][0]["thoughtSignature"], SKIP_THOUGHT_SIGNATURE);
        // 自动补齐中断的 functionResponse，且末尾不是 model turn。
        assert_eq!(contents.last().unwrap()["role"], "user");
        assert_eq!(contents[2]["parts"][0]["functionResponse"]["id"], "call_raw");
    }

    #[test]
    fn gemini_sse_stream_converts_thought_text_tool_call_and_usage_to_responses() {
        let sse = concat!(
            "data: {\"responseId\":\"resp_1\",\"modelVersion\":\"gemini-2.5-pro\",\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"thinking...\",\"thought\":true}]}}]}\n\n",
            "data: {\"responseId\":\"resp_1\",\"modelVersion\":\"gemini-2.5-pro\",\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"Hello \"},{\"functionCall\":{\"id\":\"fc_1\",\"name\":\"lookup\",\"args\":{\"q\":\"hi\"}},\"thoughtSignature\":\"sig_xyz\"}]}}]}\n\n",
            "data: {\"responseId\":\"resp_1\",\"modelVersion\":\"gemini-2.5-pro\",\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[]},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":10,\"candidatesTokenCount\":5,\"thoughtsTokenCount\":3,\"cachedContentTokenCount\":4,\"totalTokenCount\":18}}\n\n"
        );
        let request = json!({ "model": "gemini-2.5-pro" });
        let responses_sse = String::from_utf8(gemini_sse_to_responses_sse(sse, &request)).unwrap();
        assert!(responses_sse.contains("response.reasoning_summary_text.delta") || responses_sse.contains("thinking..."));
        assert!(responses_sse.contains("Hello "));
        assert!(responses_sse.contains("lookup"));
        assert!(responses_sse.contains("response.completed"));
        // 确认 thoughtSignature 已编码进 call_id 且可以还原。
        let start = responses_sse.find("fc_1__gts").expect("signed call_id in stream");
        let end = responses_sse[start..].find('"').unwrap() + start;
        let (clean, sig) = unsign_call_id(&responses_sse[start..end]);
        assert_eq!(clean, "fc_1");
        assert_eq!(sig.as_deref(), Some("sig_xyz"));
    }

    #[test]
    fn gemini_non_stream_json_converts_to_chat_completion() {
        let raw = json!({
            "responseId": "r1",
            "modelVersion": "gemini-2.5-pro",
            "candidates": [{
                "content": {
                    "role": "model",
                    "parts": [
                        { "text": "plan", "thought": true },
                        { "text": "answer" }
                    ]
                },
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 12,
                "candidatesTokenCount": 4,
                "thoughtsTokenCount": 2,
                "totalTokenCount": 18
            }
        });
        let chat = gemini_response_to_chat_completion(raw).unwrap();
        assert_eq!(chat["choices"][0]["message"]["content"], "answer");
        assert_eq!(chat["choices"][0]["message"]["reasoning_content"], "plan");
        assert_eq!(chat["choices"][0]["finish_reason"], "stop");
        assert_eq!(chat["usage"]["prompt_tokens"], 12);
        assert_eq!(chat["usage"]["completion_tokens"], 6);
        assert_eq!(chat["usage"]["completion_tokens_details"]["reasoning_tokens"], 2);
    }
}
