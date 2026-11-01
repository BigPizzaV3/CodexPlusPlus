//! Gemini generateContent 请求 → Responses 请求（网关的中间格式）。
//!
//! Gemini CLI 等说 Gemini API 的 Agent 把请求发到 `/gw/<agent>/v1beta/models/<模型>:<动作>`；
//! 这里负责拆路径、把请求体转成 Responses，交给现有代理链路。
//!
//! 函数调用的配对：Gemini 的 `functionCall` / `functionResponse` 的 `id` 是可选的。网关回给客户端的
//! `functionCall` 总是带 `id`（见 [`super::gemini_outbound`]），客户端一般会原样带回；没带时按
//! 「名字 + 序号」生成稳定的 call_id（同一段历史每次生成的结果一样），`functionResponse` 按名字
//! 先进先出配给还没回答的调用。
//!
//! 推理：历史里 `thought: true` 的文本转成只有摘要的 reasoning 项（与 Chat 入站同理：Chat 上游需要，
//! 原生 Responses 上游会在管道里去掉）；`thoughtSignature` 对 OpenAI 系上游没有意义，丢弃。

use std::collections::{HashMap, VecDeque};

use anyhow::bail;
use base64::Engine;
use serde_json::{Map, Value, json};

/// Gemini 路径里的动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeminiAction {
    Generate,
    StreamGenerate,
    CountTokens,
}

/// 把 `/v1beta/models/gemini-2.5-pro:streamGenerateContent?alt=sse` 一类的路径拆成（模型名，动作）。
pub fn parse_gemini_path(tail: &str) -> Option<(String, GeminiAction)> {
    let path = tail.split(['?', '#']).next().unwrap_or_default();
    let path = path.trim_end_matches('/');
    // 网关的模型名本身带 `/`（`<供应商>/<模型>`、`group/<组>`），取第一个 `/models/` 之后的全部；
    // 客户端可能把 `/` 编码成 `%2F`。
    let rest = match path.find("/models/") {
        Some(index) => &path[index + "/models/".len()..],
        None => path.strip_prefix("models/")?,
    };
    let (model, action) = rest.rsplit_once(':')?;
    let model = model.replace("%2F", "/").replace("%2f", "/");
    if model.is_empty() {
        return None;
    }
    let action = match action {
        "generateContent" => GeminiAction::Generate,
        "streamGenerateContent" => GeminiAction::StreamGenerate,
        "countTokens" => GeminiAction::CountTokens,
        _ => return None,
    };
    Some((model.to_string(), action))
}

/// 同时兼容 camelCase 与 snake_case 字段名（REST 两种都收）。
fn field<'a>(value: &'a Value, camel: &str, snake: &str) -> Option<&'a Value> {
    value.get(camel).or_else(|| value.get(snake))
}

fn parts_of(content: &Value) -> &[Value] {
    content
        .get("parts")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn system_text(system: &Value) -> String {
    match system {
        Value::String(text) => text.clone(),
        _ => parts_of(system)
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .filter(|text| !text.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n\n"),
    }
}

/// `inlineData` / `fileData` → Responses 的内容部件。
fn media_part(part: &Value) -> anyhow::Result<Option<Value>> {
    if let Some(inline) = field(part, "inlineData", "inline_data") {
        let mime = field(inline, "mimeType", "mime_type")
            .and_then(Value::as_str)
            .unwrap_or("application/octet-stream");
        let data = inline
            .get("data")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let data_url = format!("data:{mime};base64,{data}");
        return Ok(Some(if mime.starts_with("image/") {
            json!({ "type": "input_image", "image_url": data_url })
        } else if mime == "application/pdf" {
            json!({ "type": "input_file", "filename": "document.pdf", "file_data": data_url })
        } else if mime.starts_with("text/") || mime == "application/json" {
            // 文本类附件直接解码成文字，所有上游都能读。
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(data.trim())
                .map_err(|error| {
                    anyhow::anyhow!("inlineData（{mime}）不是合法的 base64：{error}")
                })?;
            json!({ "type": "input_text", "text": String::from_utf8_lossy(&bytes) })
        } else {
            bail!(
                "暂不支持 {mime} 类型的 inlineData：网关的 Responses 管道只能转发图片、PDF 与文本"
            )
        }));
    }
    if let Some(file) = field(part, "fileData", "file_data") {
        let mime = field(file, "mimeType", "mime_type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let Some(uri) = field(file, "fileUri", "file_uri").and_then(Value::as_str) else {
            bail!("fileData 缺少 fileUri");
        };
        return Ok(Some(if mime.starts_with("image/") {
            json!({ "type": "input_image", "image_url": uri })
        } else {
            json!({ "type": "input_file", "file_url": uri })
        }));
    }
    Ok(None)
}

/// functionResponse 的 response → function_call_output 的 output。
///
/// Gemini CLI 习惯包成 `{"output": "..."}`；只有这一个字符串字段时直接给字符串，模型读起来更自然。
fn function_output(response: Option<&Value>) -> String {
    let Some(response) = response else {
        return String::new();
    };
    if let Some(object) = response.as_object()
        && object.len() == 1
    {
        let (key, value) = object.iter().next().expect("只有一个字段");
        if let Some(text) = value.as_str() {
            match key.as_str() {
                "output" | "result" | "content" => return text.to_string(),
                "error" => return format!("Error: {text}"),
                _ => {}
            }
        }
    }
    match response {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// 函数调用与回答的配对状态（见模块文档）。
#[derive(Default)]
struct CallPairing {
    /// 每个函数名已出现的调用次数，用来生成稳定的 call_id。
    counters: HashMap<String, usize>,
    /// 还没回答的调用，按名字先进先出。
    pending: HashMap<String, VecDeque<String>>,
}

impl CallPairing {
    fn call(&mut self, name: &str, id: Option<&str>) -> String {
        let counter = self.counters.entry(name.to_string()).or_default();
        let call_id = match id.filter(|id| !id.trim().is_empty()) {
            Some(id) => id.to_string(),
            None => format!("call_{}_{counter}", sanitize_name(name)),
        };
        *counter += 1;
        self.pending
            .entry(name.to_string())
            .or_default()
            .push_back(call_id.clone());
        call_id
    }

    fn response(&mut self, name: &str, id: Option<&str>) -> String {
        let id = id.filter(|id| !id.trim().is_empty());
        let queue = self.pending.entry(name.to_string()).or_default();
        // 带了 id 且确实对应一个未回答的调用：按 id 配。
        if let Some(id) = id
            && let Some(position) = queue.iter().position(|pending| pending == id)
        {
            return queue.remove(position).unwrap_or_else(|| id.to_string());
        }
        // 否则配给同名的最早一个未回答调用（客户端自己生成的 id 对不上时也走这里）。
        if let Some(call_id) = queue.pop_front() {
            return call_id;
        }
        match id {
            Some(id) => id.to_string(),
            None => {
                let counter = self.counters.entry(name.to_string()).or_default();
                let call_id = format!("call_{}_{counter}", sanitize_name(name));
                *counter += 1;
                call_id
            }
        }
    }
}

fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|char| {
            if char.is_ascii_alphanumeric() || char == '_' || char == '-' {
                char
            } else {
                '_'
            }
        })
        .collect()
}

fn flush_message(role: &str, parts: &mut Vec<Value>, input: &mut Vec<Value>) {
    if parts.is_empty() {
        return;
    }
    input.push(json!({
        "type": "message",
        "role": role,
        "content": std::mem::take(parts)
    }));
}

fn convert_content(
    content: &Value,
    pairing: &mut CallPairing,
    input: &mut Vec<Value>,
) -> anyhow::Result<()> {
    let is_model = content.get("role").and_then(Value::as_str) == Some("model");
    let role = if is_model { "assistant" } else { "user" };
    let text_type = if is_model {
        "output_text"
    } else {
        "input_text"
    };
    let mut parts = Vec::new();
    // 同一条 user 内容里的 functionResponse 是对上一轮调用的回答，要紧跟调用、排在其它内容前面。
    let mut outputs = Vec::new();
    for part in parts_of(content) {
        if let Some(call) = field(part, "functionCall", "function_call") {
            flush_message(role, &mut parts, input);
            let name = call.get("name").and_then(Value::as_str).unwrap_or_default();
            let call_id = pairing.call(name, call.get("id").and_then(Value::as_str));
            let arguments = call
                .get("args")
                .filter(|args| args.is_object())
                .map(Value::to_string)
                .unwrap_or_else(|| "{}".to_string());
            input.push(json!({
                "type": "function_call",
                "call_id": call_id,
                "name": name,
                "arguments": arguments
            }));
            continue;
        }
        if let Some(response) = field(part, "functionResponse", "function_response") {
            let name = response
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let call_id = pairing.response(name, response.get("id").and_then(Value::as_str));
            outputs.push(json!({
                "type": "function_call_output",
                "call_id": call_id,
                "output": function_output(response.get("response"))
            }));
            continue;
        }
        if let Some(text) = part.get("text").and_then(Value::as_str) {
            if part.get("thought").and_then(Value::as_bool) == Some(true) {
                if is_model && !text.trim().is_empty() {
                    flush_message(role, &mut parts, input);
                    input.push(json!({
                        "type": "reasoning",
                        "summary": [{ "type": "summary_text", "text": text }]
                    }));
                }
                continue;
            }
            if !text.is_empty() {
                parts.push(json!({ "type": text_type, "text": text }));
            }
            continue;
        }
        if let Some(media) = media_part(part)? {
            if is_model {
                // assistant 消息里放不了图片 / 文件，文字说明一下，免得上下文断掉。
                parts.push(json!({ "type": "output_text", "text": "[附件]" }));
            } else {
                parts.push(media);
            }
        }
        // executableCode / codeExecutionResult 等没有对应物，丢弃。
    }
    input.extend(outputs);
    flush_message(role, &mut parts, input);
    Ok(())
}

/// Gemini Schema（OpenAPI 子集，类型大写）→ JSON Schema。
fn gemini_schema_to_json_schema(schema: &Value) -> Value {
    let Value::Object(map) = schema else {
        return schema.clone();
    };
    let mut converted = Map::new();
    for (key, value) in map {
        let value = match key.as_str() {
            "type" => match value {
                Value::String(kind) if kind == "TYPE_UNSPECIFIED" => continue,
                Value::String(kind) => json!(kind.to_ascii_lowercase()),
                Value::Array(kinds) => Value::Array(
                    kinds
                        .iter()
                        .map(|kind| match kind.as_str() {
                            Some(kind) => json!(kind.to_ascii_lowercase()),
                            None => kind.clone(),
                        })
                        .collect(),
                ),
                other => other.clone(),
            },
            // Gemini 专有字段，JSON Schema 里没有。
            "nullable" | "propertyOrdering" => continue,
            "properties" | "$defs" | "definitions" | "patternProperties" => match value {
                Value::Object(properties) => Value::Object(
                    properties
                        .iter()
                        .map(|(name, schema)| (name.clone(), gemini_schema_to_json_schema(schema)))
                        .collect(),
                ),
                other => other.clone(),
            },
            "items" | "additionalProperties" | "not" => gemini_schema_to_json_schema(value),
            "anyOf" | "oneOf" | "allOf" | "prefixItems" => match value {
                Value::Array(schemas) => {
                    Value::Array(schemas.iter().map(gemini_schema_to_json_schema).collect())
                }
                other => other.clone(),
            },
            // int64 在 proto JSON 里是字符串，JSON Schema 要数字。
            "minItems" | "maxItems" | "minLength" | "maxLength" | "minProperties"
            | "maxProperties" => match value.as_str().and_then(|text| text.parse::<u64>().ok()) {
                Some(number) => json!(number),
                None => value.clone(),
            },
            _ => value.clone(),
        };
        converted.insert(key.clone(), value);
    }
    if map.get("nullable").and_then(Value::as_bool) == Some(true)
        && let Some(Value::String(kind)) = converted.get("type").cloned()
    {
        converted.insert("type".to_string(), json!([kind, "null"]));
    }
    Value::Object(converted)
}

fn convert_tools(tools: &[Value]) -> Vec<Value> {
    let mut converted = Vec::new();
    for tool in tools {
        for declaration in field(tool, "functionDeclarations", "function_declarations")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            let Some(name) = declaration.get("name").and_then(Value::as_str) else {
                continue;
            };
            let parameters = field(
                declaration,
                "parametersJsonSchema",
                "parameters_json_schema",
            )
            .cloned()
            .or_else(|| {
                declaration
                    .get("parameters")
                    .map(gemini_schema_to_json_schema)
            })
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({ "type": "object", "properties": {} }));
            let mut function = json!({
                "type": "function",
                "name": name,
                "parameters": parameters,
                "strict": false
            });
            if let Some(description) = declaration.get("description").and_then(Value::as_str) {
                function["description"] = json!(description);
            }
            converted.push(function);
        }
        if field(tool, "googleSearch", "google_search").is_some() {
            // 服务端搜索：映射成 Responses 原生 web_search；Chat 上游会在代理里被剔除。
            converted.push(json!({ "type": "web_search" }));
        }
        // codeExecution、urlContext 等服务端工具没有对应物，丢弃。
    }
    converted
}

fn convert_tool_choice(config: &Value) -> Option<Value> {
    let calling = field(config, "functionCallingConfig", "function_calling_config")?;
    let allowed = field(calling, "allowedFunctionNames", "allowed_function_names")
        .and_then(Value::as_array)
        .map(|names| names.iter().filter_map(Value::as_str).collect::<Vec<_>>())
        .unwrap_or_default();
    match calling
        .get("mode")
        .and_then(Value::as_str)?
        .to_ascii_uppercase()
        .as_str()
    {
        "AUTO" | "VALIDATED" => Some(json!("auto")),
        "NONE" => Some(json!("none")),
        // 只允许一个函数时可以精确指定；多个时 Responses 的 Chat 兼容链路没有对应物，退化为 required。
        "ANY" if allowed.len() == 1 => Some(json!({ "type": "function", "name": allowed[0] })),
        "ANY" => Some(json!("required")),
        _ => None,
    }
}

/// thinkingConfig → reasoning.effort。`thinkingBudget: 0` 表示关闭思考，不带 reasoning。
fn reasoning_effort(thinking: &Value) -> Option<String> {
    if let Some(level) = field(thinking, "thinkingLevel", "thinking_level").and_then(Value::as_str)
    {
        let level = level.trim().to_ascii_lowercase();
        return Some(match level.as_str() {
            "minimal" | "low" | "medium" | "high" => level,
            _ => "medium".to_string(),
        });
    }
    let budget = field(thinking, "thinkingBudget", "thinking_budget").and_then(Value::as_i64)?;
    Some(
        match budget {
            0 => return None,
            // -1 是「动态思考」。
            budget if budget < 0 => "medium",
            budget if budget < 4_096 => "low",
            budget if budget < 16_384 => "medium",
            _ => "high",
        }
        .to_string(),
    )
}

fn convert_text_format(config: &Value) -> Option<Value> {
    let mime = field(config, "responseMimeType", "response_mime_type").and_then(Value::as_str)?;
    if mime != "application/json" {
        return None;
    }
    let schema = field(config, "responseJsonSchema", "response_json_schema")
        .cloned()
        .or_else(|| {
            field(config, "responseSchema", "response_schema").map(gemini_schema_to_json_schema)
        });
    Some(match schema {
        Some(schema) => json!({
            "type": "json_schema",
            "name": "response",
            "schema": schema,
            "strict": false
        }),
        None => json!({ "type": "json_object" }),
    })
}

/// Gemini generateContent 请求体 → Responses 请求体。`model` 取自路径，`stream` 取自动作。
pub fn gemini_to_responses(model: &str, request: &Value, stream: bool) -> anyhow::Result<Value> {
    let object = request
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("请求体不是 JSON 对象"))?;
    let mut body = Map::new();
    body.insert(
        "model".to_string(),
        json!(model.strip_prefix("models/").unwrap_or(model)),
    );
    if let Some(system) = field(request, "systemInstruction", "system_instruction") {
        let text = system_text(system);
        if !text.trim().is_empty() {
            body.insert("instructions".to_string(), json!(text));
        }
    }
    let mut input = Vec::new();
    let mut pairing = CallPairing::default();
    for content in object
        .get("contents")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        convert_content(content, &mut pairing, &mut input)?;
    }
    body.insert("input".to_string(), Value::Array(input));

    if let Some(tools) = object.get("tools").and_then(Value::as_array) {
        let tools = convert_tools(tools);
        if !tools.is_empty() {
            body.insert("tools".to_string(), Value::Array(tools));
            if let Some(choice) =
                field(request, "toolConfig", "tool_config").and_then(convert_tool_choice)
            {
                body.insert("tool_choice".to_string(), choice);
            }
        }
    }

    let config = field(request, "generationConfig", "generation_config")
        .cloned()
        .unwrap_or(Value::Null);
    if let Some(count) = field(&config, "candidateCount", "candidate_count").and_then(Value::as_u64)
        && count > 1
    {
        bail!("网关不支持 candidateCount>1（一次生成多个候选），请改为 1");
    }
    if let Some(max_tokens) =
        field(&config, "maxOutputTokens", "max_output_tokens").and_then(Value::as_u64)
    {
        body.insert("max_output_tokens".to_string(), json!(max_tokens));
    }
    for (camel, snake, key) in [
        ("temperature", "temperature", "temperature"),
        ("topP", "top_p", "top_p"),
    ] {
        if let Some(value) = field(&config, camel, snake).filter(|value| value.is_number()) {
            body.insert(key.to_string(), value.clone());
        }
    }
    if let Some(thinking) = field(&config, "thinkingConfig", "thinking_config")
        && let Some(effort) = reasoning_effort(thinking)
    {
        let mut reasoning = json!({ "effort": effort });
        // 客户端要看思考过程时才让上游回摘要（Gemini CLI 会开 includeThoughts）。
        if field(thinking, "includeThoughts", "include_thoughts").and_then(Value::as_bool)
            == Some(true)
        {
            reasoning["summary"] = json!("auto");
        }
        body.insert("reasoning".to_string(), reasoning);
    }
    if let Some(format) = convert_text_format(&config) {
        body.insert("text".to_string(), json!({ "format": format }));
    }
    body.insert("store".to_string(), json!(false));
    body.insert("stream".to_string(), json!(stream));
    Ok(Value::Object(body))
}

/// 输入 token 的本地粗估（countTokens 与上游没回 usage 时用）。
///
/// countTokens 的请求体可能把真正的请求包在 `generateContentRequest` 里。
pub fn estimate_input_tokens(request: &Value) -> u64 {
    if let Some(inner) = field(
        request,
        "generateContentRequest",
        "generate_content_request",
    ) {
        return estimate_input_tokens(inner);
    }
    let mut chars = 0_usize;
    if let Some(system) = field(request, "systemInstruction", "system_instruction") {
        chars += system_text(system).chars().count();
    }
    for key in ["contents", "tools"] {
        if let Some(value) = request.get(key) {
            chars += value.to_string().chars().count();
        }
    }
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
    fn path_parsing() {
        assert_eq!(
            parse_gemini_path("/models/gemini-2.5-pro:streamGenerateContent"),
            Some(("gemini-2.5-pro".to_string(), GeminiAction::StreamGenerate))
        );
        assert_eq!(
            parse_gemini_path("/v1beta/models/x:generateContent"),
            Some(("x".to_string(), GeminiAction::Generate))
        );
        assert_eq!(
            parse_gemini_path("/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse"),
            Some(("gemini-2.5-flash".to_string(), GeminiAction::StreamGenerate))
        );
        assert_eq!(
            parse_gemini_path("/v1/models/m-1.5:countTokens"),
            Some(("m-1.5".to_string(), GeminiAction::CountTokens))
        );
        assert_eq!(
            parse_gemini_path("models/x:generateContent"),
            Some(("x".to_string(), GeminiAction::Generate))
        );
        assert_eq!(
            parse_gemini_path("/v1beta/models/deepseek/deepseek-v4-pro:streamGenerateContent"),
            Some((
                "deepseek/deepseek-v4-pro".to_string(),
                GeminiAction::StreamGenerate
            ))
        );
        assert_eq!(
            parse_gemini_path("/v1beta/models/group%2Fdaily:generateContent"),
            Some(("group/daily".to_string(), GeminiAction::Generate))
        );
        assert_eq!(parse_gemini_path("/v1beta/models/x:embedContent"), None);
        assert_eq!(parse_gemini_path("/v1beta/models/x"), None);
        assert_eq!(parse_gemini_path("/v1beta/models/:generateContent"), None);
        assert_eq!(parse_gemini_path("/v1beta/files/x:generateContent"), None);
    }

    #[test]
    fn text_system_and_generation_config() {
        let request = json!({
            "systemInstruction": { "parts": [{ "text": "You are Gemini CLI." }, { "text": "Be brief." }] },
            "contents": [
                { "role": "user", "parts": [{ "text": "hi" }] },
                { "role": "model", "parts": [{ "text": "hello" }] },
                { "parts": [{ "text": "again" }] }
            ],
            "generationConfig": {
                "maxOutputTokens": 512,
                "temperature": 0.3,
                "topP": 0.95,
                "topK": 40,
                "stopSequences": ["END"]
            }
        });
        let body = gemini_to_responses("models/gemini-2.5-pro", &request, true).unwrap();
        assert_eq!(body["model"], "gemini-2.5-pro");
        assert_eq!(body["instructions"], "You are Gemini CLI.\n\nBe brief.");
        assert_eq!(
            kinds(&body),
            vec!["message:user", "message:assistant", "message:user"]
        );
        assert_eq!(body["input"][0]["content"][0]["type"], "input_text");
        assert_eq!(body["input"][1]["content"][0]["type"], "output_text");
        assert_eq!(body["max_output_tokens"], 512);
        assert_eq!(body["temperature"], 0.3);
        assert_eq!(body["top_p"], 0.95);
        assert_eq!(body["stream"], true);
        assert_eq!(body["store"], false);
        assert!(body.get("reasoning").is_none());

        let snake = gemini_to_responses(
            "x",
            &json!({
                "system_instruction": { "parts": [{ "text": "sys" }] },
                "contents": [{ "role": "user", "parts": [{ "text": "q" }] }],
                "generation_config": { "max_output_tokens": 9 }
            }),
            false,
        )
        .unwrap();
        assert_eq!(snake["instructions"], "sys");
        assert_eq!(snake["max_output_tokens"], 9);
        assert_eq!(snake["stream"], false);
    }

    #[test]
    fn inline_media_parts() {
        let request = json!({
            "contents": [{ "role": "user", "parts": [
                { "text": "看图" },
                { "inlineData": { "mimeType": "image/png", "data": "AAAA" } },
                { "inlineData": { "mimeType": "application/pdf", "data": "JVBE" } },
                { "inlineData": { "mimeType": "text/plain", "data": "5L2g5aW9" } },
                { "fileData": { "mimeType": "image/jpeg", "fileUri": "https://example.test/a.jpg" } }
            ]}]
        });
        let body = gemini_to_responses("m", &request, false).unwrap();
        let parts = body["input"][0]["content"].as_array().unwrap();
        assert_eq!(parts[1]["type"], "input_image");
        assert_eq!(parts[1]["image_url"], "data:image/png;base64,AAAA");
        assert_eq!(parts[2]["type"], "input_file");
        assert_eq!(parts[2]["file_data"], "data:application/pdf;base64,JVBE");
        assert_eq!(parts[3]["text"], "你好");
        assert_eq!(parts[4]["image_url"], "https://example.test/a.jpg");

        let error = gemini_to_responses(
            "m",
            &json!({ "contents": [{ "role": "user", "parts": [
                { "inlineData": { "mimeType": "audio/wav", "data": "AAAA" } }
            ]}]}),
            false,
        )
        .unwrap_err();
        assert!(error.to_string().contains("audio/wav"), "{error}");

        let error = gemini_to_responses(
            "m",
            &json!({ "contents": [], "generationConfig": { "candidateCount": 2 } }),
            false,
        )
        .unwrap_err();
        assert!(error.to_string().contains("candidateCount"), "{error}");
    }

    #[test]
    fn function_calls_without_ids_get_stable_paired_ids() {
        let request = json!({
            "contents": [
                { "role": "user", "parts": [{ "text": "读两个文件" }] },
                { "role": "model", "parts": [
                    { "text": "好的" },
                    { "functionCall": { "name": "read_file", "args": { "path": "a" } } },
                    { "functionCall": { "name": "read_file", "args": { "path": "b" } } }
                ]},
                { "role": "user", "parts": [
                    { "functionResponse": { "name": "read_file", "response": { "output": "A" } } },
                    { "functionResponse": { "name": "read_file", "response": { "output": "B" } } }
                ]},
                { "role": "model", "parts": [
                    { "functionCall": { "id": "call_x", "name": "ls", "args": {} } }
                ]},
                { "role": "user", "parts": [
                    // 客户端自己生成的 id 对不上时，按名字配给未回答的调用。
                    { "functionResponse": { "id": "ls-123-random", "name": "ls", "response": { "files": ["a", "b"] } } },
                    { "text": "继续" }
                ]}
            ]
        });
        let body = gemini_to_responses("m", &request, false).unwrap();
        assert_eq!(
            kinds(&body),
            vec![
                "message:user",
                "message:assistant",
                "function_call:",
                "function_call:",
                "function_call_output:",
                "function_call_output:",
                "function_call:",
                "function_call_output:",
                "message:user"
            ]
        );
        let input = body["input"].as_array().unwrap();
        assert_eq!(input[2]["call_id"], "call_read_file_0");
        assert_eq!(input[3]["call_id"], "call_read_file_1");
        assert_eq!(input[2]["arguments"], "{\"path\":\"a\"}");
        assert_eq!(input[4]["call_id"], "call_read_file_0");
        assert_eq!(input[4]["output"], "A");
        assert_eq!(input[5]["call_id"], "call_read_file_1");
        assert_eq!(input[5]["output"], "B");
        assert_eq!(input[6]["call_id"], "call_x");
        assert_eq!(input[7]["call_id"], "call_x");
        assert_eq!(input[7]["output"], "{\"files\":[\"a\",\"b\"]}");

        // 同一段历史再转一次，生成的 id 不变。
        assert_eq!(gemini_to_responses("m", &request, false).unwrap(), body);
    }

    #[test]
    fn function_response_with_matching_id_pairs_out_of_order() {
        let request = json!({
            "contents": [
                { "role": "model", "parts": [
                    { "functionCall": { "id": "c1", "name": "run", "args": {} } },
                    { "functionCall": { "id": "c2", "name": "run", "args": {} } }
                ]},
                { "role": "user", "parts": [
                    { "functionResponse": { "id": "c2", "name": "run", "response": { "error": "boom" } } },
                    { "functionResponse": { "id": "c1", "name": "run", "response": { "output": "ok" } } }
                ]}
            ]
        });
        let body = gemini_to_responses("m", &request, false).unwrap();
        let input = body["input"].as_array().unwrap();
        assert_eq!(input[2]["call_id"], "c2");
        assert_eq!(input[2]["output"], "Error: boom");
        assert_eq!(input[3]["call_id"], "c1");
        assert_eq!(input[3]["output"], "ok");
    }

    #[test]
    fn thoughts_become_reasoning_items() {
        let request = json!({
            "contents": [
                { "role": "user", "parts": [{ "text": "q" }] },
                { "role": "model", "parts": [
                    { "text": "先想想", "thought": true, "thoughtSignature": "sig" },
                    { "text": "答案" }
                ]}
            ],
            "generationConfig": { "thinkingConfig": { "thinkingBudget": 8192, "includeThoughts": true } }
        });
        let body = gemini_to_responses("m", &request, true).unwrap();
        assert_eq!(
            kinds(&body),
            vec!["message:user", "reasoning:", "message:assistant"]
        );
        assert_eq!(body["input"][1]["summary"][0]["text"], "先想想");
        assert_eq!(body["reasoning"]["effort"], "medium");
        assert_eq!(body["reasoning"]["summary"], "auto");
    }

    #[test]
    fn thinking_config_mapping() {
        for (thinking, expected) in [
            (json!({ "thinkingBudget": 1024 }), Some("low")),
            (json!({ "thinkingBudget": 24576 }), Some("high")),
            (json!({ "thinkingBudget": -1 }), Some("medium")),
            (json!({ "thinkingBudget": 0 }), None),
            (json!({ "thinkingLevel": "HIGH" }), Some("high")),
            (json!({ "thinking_level": "low" }), Some("low")),
            (json!({ "includeThoughts": true }), None),
        ] {
            assert_eq!(
                reasoning_effort(&thinking).as_deref(),
                expected,
                "{thinking}"
            );
        }
        let body = gemini_to_responses(
            "m",
            &json!({ "contents": [], "generationConfig": { "thinkingConfig": { "thinkingLevel": "low" } } }),
            false,
        )
        .unwrap();
        assert_eq!(body["reasoning"], json!({ "effort": "low" }));
    }

    #[test]
    fn tools_schema_and_tool_config() {
        let request = json!({
            "contents": [],
            "tools": [
                { "functionDeclarations": [
                    {
                        "name": "read_file",
                        "description": "读文件",
                        "parameters": {
                            "type": "OBJECT",
                            "properties": {
                                "path": { "type": "STRING", "description": "路径" },
                                "type": { "type": "STRING", "enum": ["A", "B"], "nullable": true },
                                "lines": { "type": "ARRAY", "items": { "type": "INTEGER" }, "maxItems": "5" }
                            },
                            "required": ["path"],
                            "propertyOrdering": ["path", "type", "lines"]
                        }
                    },
                    { "name": "ls", "parametersJsonSchema": { "type": "object", "properties": { "dir": { "type": "string" } } } },
                    { "name": "now" }
                ]},
                { "googleSearch": {} },
                { "codeExecution": {} }
            ],
            "toolConfig": { "functionCallingConfig": { "mode": "ANY", "allowedFunctionNames": ["ls"] } }
        });
        let body = gemini_to_responses("m", &request, false).unwrap();
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 4);
        assert_eq!(tools[0]["type"], "function");
        assert_eq!(tools[0]["name"], "read_file");
        assert_eq!(tools[0]["description"], "读文件");
        assert_eq!(tools[0]["strict"], false);
        let parameters = &tools[0]["parameters"];
        assert_eq!(parameters["type"], "object");
        assert_eq!(parameters["properties"]["path"]["type"], "string");
        // 属性名叫 type 也不会被当成关键字改写。
        assert_eq!(
            parameters["properties"]["type"]["type"],
            json!(["string", "null"])
        );
        assert_eq!(parameters["properties"]["type"]["enum"], json!(["A", "B"]));
        assert!(parameters["properties"]["type"].get("nullable").is_none());
        assert_eq!(
            parameters["properties"]["lines"]["items"]["type"],
            "integer"
        );
        assert_eq!(parameters["properties"]["lines"]["maxItems"], 5);
        assert!(parameters.get("propertyOrdering").is_none());
        assert_eq!(parameters["required"], json!(["path"]));
        assert_eq!(
            tools[1]["parameters"]["properties"]["dir"]["type"],
            "string"
        );
        assert_eq!(
            tools[2]["parameters"],
            json!({ "type": "object", "properties": {} })
        );
        assert_eq!(tools[3]["type"], "web_search");
        assert_eq!(
            body["tool_choice"],
            json!({ "type": "function", "name": "ls" })
        );

        for (mode, expected) in [
            ("AUTO", json!("auto")),
            ("ANY", json!("required")),
            ("NONE", json!("none")),
        ] {
            let request = json!({
                "contents": [],
                "tools": [{ "functionDeclarations": [{ "name": "ls" }] }],
                "toolConfig": { "functionCallingConfig": { "mode": mode } }
            });
            assert_eq!(
                gemini_to_responses("m", &request, false).unwrap()["tool_choice"],
                expected
            );
        }
    }

    #[test]
    fn response_format_mapping() {
        let body = gemini_to_responses(
            "m",
            &json!({
                "contents": [],
                "generationConfig": {
                    "responseMimeType": "application/json",
                    "responseSchema": { "type": "OBJECT", "properties": { "a": { "type": "NUMBER" } } }
                }
            }),
            false,
        )
        .unwrap();
        assert_eq!(body["text"]["format"]["type"], "json_schema");
        assert_eq!(body["text"]["format"]["schema"]["type"], "object");
        assert_eq!(
            body["text"]["format"]["schema"]["properties"]["a"]["type"],
            "number"
        );

        let body = gemini_to_responses(
            "m",
            &json!({ "contents": [], "generationConfig": { "responseMimeType": "application/json" } }),
            false,
        )
        .unwrap();
        assert_eq!(body["text"]["format"], json!({ "type": "json_object" }));

        let body = gemini_to_responses(
            "m",
            &json!({ "contents": [], "generationConfig": { "responseMimeType": "text/plain" } }),
            false,
        )
        .unwrap();
        assert!(body.get("text").is_none());
    }

    #[test]
    fn estimate_handles_count_tokens_wrapper() {
        let inner = json!({
            "systemInstruction": { "parts": [{ "text": "s".repeat(400) }] },
            "contents": [{ "role": "user", "parts": [{ "text": "hi" }] }]
        });
        let direct = estimate_input_tokens(&inner);
        assert!(direct > 100);
        assert_eq!(
            estimate_input_tokens(&json!({ "generateContentRequest": inner })),
            direct
        );
        assert!(estimate_input_tokens(&json!({ "contents": [] })) < direct);
    }
}
